import CryptoKit
import HerdrKit
import Foundation

/// The remote hub's view of the app: the same store, the same writes setting and the same input paths as the cards
/// and the composer. `HubSession` checks the prompt and the level before it calls a write here.
@MainActor final class AppHubBackend: HubBackend {
    let model: AppModel
    /// Web Push: the phones' subscriptions and the hub's VAPID key. No key: the hub offers no push.
    let pushes: MemoryPushStore?
    let vapid: P256.Signing.PrivateKey?
    init(model: AppModel, pushes: MemoryPushStore? = nil, vapid: P256.Signing.PrivateKey? = nil) {
        self.model = model; self.pushes = pushes; self.vapid = vapid
    }

    private var store: HerdrStore { model.store }
    private func pane(_ id: String) -> Pane? { store.workspaces.lazy.flatMap(\.tabs).flatMap(\.panes).first { $0.id == id } }

    var hubName: String { Host.current().localizedName ?? "Mac" }
    var hubVersion: String { Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "dev" }
    var sessionName: String? { store.session.name }
    var workspaces: [Workspace] { store.workspaces }
    var writesEnabled: Bool { model.writesEnabled }

    func conversation(session: String) -> [ConversationItem] { store.conversation(for: session) }
    /// The store does not say if the log has more; the client learns it from `loadEarlier`.
    func hasEarlier(session: String) -> Bool { true }
    func loadEarlier(session: String) async -> EarlierLoadResult { await store.loadEarlier(for: session) }
    func screen(pane: String) async -> String? { try? await model.screen(paneId: pane)?.visibleText(paneId: pane) }

    /// The composer's image folder: `MessageSender` waits for the agent's image marker on paths in it.
    var imageDirectory: URL? { PasteImages.directory }

    /// The composer's send: the text, then the image paths (quoted as the composer does).
    /// `allowed` (the device's level and Allow input now) is asked before the read and before every key step.
    func send(pane id: String, text: String, images: [URL], agent: Agent, allowed: @escaping @MainActor @Sendable () -> Bool) async -> HubSendResult {
        let model = model
        let may: @MainActor @Sendable () -> Bool = { model.writesEnabled && allowed() }
        guard may() else { return .failed }
        // The pane's lock for the whole send: a pull (Mac or phone) never runs in between.
        guard model.claim(id) else { return .changed }
        defer { model.release(id) }
        if let hook = model.sendHook { return await hook(id, text) ? .sent : .failed }
        guard let reader = model.screen(paneId: id) else { return .failed }
        // herdr has no compare-and-send: this read and the keys are two calls, so a small window stays.
        guard await reader.readyForText(paneId: id, agent: agent) else { return .changed }
        guard may() else { return .failed }
        // Every image must still be a file of ours; else nothing is sent (it would go as plain text).
        guard images.allSatisfy({ ImagePaste.ownFile($0.path) }) else { return .failed }
        model.noteSent(id)
        let message = ([text] + images.map(ComposerHandle.quoted)).filter { !$0.isEmpty }.joined(separator: " ")
        if let hc = reader as? HCClient {
            // hc types the whole message in one `terminal.send`: files go first (another machine: a call each), then
            // the gate once, right before the text. Unconfirmed (no answer in time) counts as sent: the core may still
            // type it, so a retry could send it twice.
            return await hc.sendFromDevice(text: message, to: id, files: HCClient.localFiles(in: message, under: PasteImages.directory.path),
                                           gate: KeyGate(agent: agent, allowed: may))
        }
        return await MessageSender.send(message, to: id, client: store.client, kind: agent.kind,
                                        gate: KeyGate(agent: agent, allowed: may)) ? .sent : .failed
    }

    /// The approval card's keys, without its UI follow-up (the Mac window does not change for a phone action).
    /// Re-reads the pane right before the keys (`respondIfOpen`): false, and nothing sent, when the prompt, the agent
    /// (kind and session from the hub's check) or the device's level changed.
    func approve(pane id: String, prompt: String, allow: Bool, agent: Agent, allowed: @escaping @MainActor @Sendable () -> Bool) async -> Bool {
        let model = model
        let gate = KeyGate(agent: agent, allowed: { model.writesEnabled && allowed() })
        guard model.writesEnabled, allowed(), let p = pane(id), let a = p.agent, let reader = model.screen(paneId: id) else { return false }
        // One answer per prompt with the card and the notification buttons (`ApprovalSender`). Unconfirmed (hc held the
        // keys past their timeout) counts as answered: they may still land, so the phone must not send them again.
        let r = await model.approvals.answer(paneId: id, via: reader, prompt: prompt, allow: allow, agentKind: a.kind,
                                             blocked: a.status == .blocked, pending: model.pendingRequest(p), gate: gate)
        guard r == .sent || r == .unconfirmed else { return false }
        try? await Task.sleep(for: .seconds(1))
        await store.refresh()
        if let p = pane(id) { await model.checkPrompt(for: p) }
        return true
    }

    func queuedLog(session: String) -> ClaudeQueueLog { store.queuedInLog(for: session) }
    var pulledTexts: MemoryPulledStore { model.pulledTexts }

    /// The agent's own queue back into the phone's composer (`QueuePull`): Up or Shift-Left, then Ctrl-U; never Escape.
    /// Holds the pane's lock for the whole pull: no other pull or send of it (Mac or phone) meanwhile.
    func pullQueued(pane id: String, agent: Agent, log: ClaudeQueueLog, keep: @escaping @MainActor @Sendable (String) -> Bool,
                    allowed: @escaping @MainActor @Sendable () -> Bool) async -> QueuePull.Outcome {
        let model = model
        guard HCMapper.address(fromPaneId: id) == nil else { return .refused(AppModel.hcPullNote) }
        guard model.claim(id) else { return .refused("A pull or a send of this pane is running. Nothing was sent.") }
        model.pulling.insert(id)
        defer { model.pulling.remove(id); model.release(id) }
        let io = HerdrQueuePullIO(client: store.client, paneId: id, allowed: { model.writesEnabled && allowed() })
        let r = await QueuePull.run(expect: agent, log: log, io: io, keep: keep)
        switch r {
        case .interrupted: remoteLog.error("pullQueued \(id, privacy: .public): the agent's turn stopped")
        case .failed(let why), .partial(_, let why): remoteLog.error("pullQueued \(id, privacy: .public): \(why, privacy: .public)")
        case .pulled, .refused: break
        }
        return r
    }

    /// The hook's newest AskUserQuestion call for this session (the files the hook wrote, read-only).
    func hookedQuestion(session: String) -> QuestionRequest? { ClaudeHookSetup.live.question(session: session) }
    var pushKey: String? { vapid.map { Base64URL.encode($0.publicKey.x963Representation) } }
    func pushSubscribe(_ s: PushSubscription) -> Bool { vapid != nil && pushes?.add(s) == true }
    func pushUnsubscribe(device: String, endpoint: String) { pushes?.remove(device: device, endpoint: endpoint) }
    func pushSubscribed(device: String) -> Bool { pushes?.all.contains { $0.device == device } == true }
    /// Settings > Remote: "Show pictures on remote devices" (`RemoteHub.pictures`).
    var remotePictures: Bool { AppDefaults.store.object(forKey: RemoteHub.picturesKey) as? Bool ?? true }
    func logURL(session: String) -> URL? { store.logURL(for: session) }
    /// Settings > Agents: "Show Claude Code status line" or "Show Grok Build status line" (the wrapper's saved line;
    /// nil while off).
    func statusLine(kind: String, session: String) -> StatusLineRecord? { StatusLineSetup.live(agent: kind)?.record(session: session) }

    /// The question card's path. Questions not read yet are read first (moves Claude's picker, navigation only);
    /// the answers must then cover every question, else nothing is sent. Every key is gated: the agent from the
    /// hub's check, this question's picker on screen, the device's level now (`KeyGate`).
    func answer(pane id: String, question: QuestionRequest, answers: [QuestionAnswer], agent: Agent, allowed: @escaping @MainActor @Sendable () -> Bool) async throws {
        let model = model
        let gate = KeyGate(agent: agent, allowed: { model.writesEnabled && allowed() })
        guard model.writesEnabled, allowed() else { throw QuestionAdapter.Failure.unsupported("input is off") }
        // One hold of the pane's lock from the first navigation key to the last answer key: no other picker keys
        // (the Mac card, a send) land in between.
        try await model.withPane(id) {
            var q = question
            if q.unread > 0 {
                // Bound to the request the client answered: another request on screen now (even with as many questions) is refused.
                guard let full = try await model.loadKeys(pane: id, gate: gate, expect: question),
                      QuestionAdapter.sameRequest(full, as: question) else { throw QuestionAdapter.Failure.notOnScreen }
                q = full
            }
            guard answers.count == q.questions.count else {
                throw QuestionAdapter.Failure.unsupported("\(q.questions.count) questions, \(answers.count) answers")
            }
            guard model.writesEnabled, allowed() else { throw QuestionAdapter.Failure.unsupported("input is off") }
            try await model.answerKeys(pane: id, request: q, answers: answers, gate: gate)
        }
    }
}
