import Foundation
import CryptoKit

// The hub's protocol logic for one connection (remote protocol v1, docs/design/remote-protocol.md), without a
// network: messages in, messages out. `HubServer` carries it over WebSocket. The app gives the data and the
// input paths through `HubBackend`, and the paired devices through `HubDeviceStore`.

/// What the hub needs from the app. Input goes through the app's own paths (composer send, approval card, question card).
@MainActor public protocol HubBackend: AnyObject {
    var hubName: String { get }
    var hubVersion: String { get }
    /// herdr session name; nil = the default session.
    var sessionName: String? { get }
    var workspaces: [Workspace] { get }
    /// The app-wide "Allow input" setting (and not a read-only launch).
    var writesEnabled: Bool { get }
    func conversation(session: String) -> [ConversationItem]
    /// The log may hold items older than `conversation(session:)` has.
    func hasEarlier(session: String) -> Bool
    /// Loads older items, distinguishing the log start from a failed read.
    func loadEarlier(session: String) async -> EarlierLoadResult
    /// The pane's visible screen text (read-only).
    func screen(pane: String) async -> String?
    /// Where re-encoded images from remote devices go (the composer's image folder). Nil: images are refused.
    var imageDirectory: URL? { get }
    /// Types the text, then the image files' paths, and submits, as the composer does.
    /// `agent`: the agent the checks saw (refuse when another one is in the pane now). `allowed`: asked again right
    /// before each key goes out (the device's level and Allow input now); false -> nothing more is sent.
    func send(pane: String, text: String, images: [URL], agent: Agent, allowed: @escaping @MainActor @Sendable () -> Bool) async -> HubSendResult
    /// The approval card's input (`HerdrClient.respond`). Called only after the prompt was checked.
    func approve(pane: String, prompt: String, allow: Bool, agent: Agent, allowed: @escaping @MainActor @Sendable () -> Bool) async -> Bool
    /// The question card's input. Called only after the question was checked.
    func answer(pane: String, question: QuestionRequest, answers: [QuestionAnswer], agent: Agent, allowed: @escaping @MainActor @Sendable () -> Bool) async throws
    /// The newest AskUserQuestion call the Claude hook saved for this agent session (`ClaudeQuestionHook`), if any.
    func hookedQuestion(session: String) -> QuestionRequest?
    /// Claude Code's queue as the session's log has it (`ClaudeQueueLog`), oldest first; [] when the log has none.
    func queuedLog(session: String) -> ClaudeQueueLog
    /// Takes the agent's own queue back (`QueuePull.run`): the text for the device's composer. Called only after the
    /// hub saw a queue it may pull. `allowed` as for `send`; `keep` saves the text before the first Ctrl-U (false: none).
    func pullQueued(pane: String, agent: Agent, log: ClaudeQueueLog, keep: @escaping @MainActor @Sendable (String) -> Bool,
                    allowed: @escaping @MainActor @Sendable () -> Bool) async -> QueuePull.Outcome
    /// Pulled texts kept until the device that pulled them has them (`PulledText`).
    var pulledTexts: MemoryPulledStore { get }
    /// The hub's VAPID public key (base64url) for the PWA; nil: no Web Push.
    var pushKey: String? { get }
    /// Keeps the device's push subscription (one per device). False: refused (invalid, or push is off).
    func pushSubscribe(_ s: PushSubscription) -> Bool
    func pushUnsubscribe(device: String, endpoint: String)
    /// The device has a subscription.
    func pushSubscribed(device: String) -> Bool
    /// "Show pictures on remote devices": items name their pictures, and a watching device may ask for them.
    var remotePictures: Bool { get }
    /// The log file of an agent session, where its pictures are read back (`LogImage.read`).
    func logURL(session: String) -> URL?
    /// The saved status line of this agent session (`StatusLineFolder`; Claude Code or Grok Build); nil when its switch is off or none.
    func statusLine(kind: String, session: String) -> StatusLineRecord?
}

/// `refused`: the backend held the message back before any key, and says why in words for the person (a screen
/// command alone, or the agent's own input box already has text: `SlashCommands.opensScreen`, `AgentBox`).
public enum HubSendResult: Sendable, Hashable { case sent, changed, failed, refused(String) }

public struct HubDevice: Codable, Hashable, Sendable {
    public var id: String
    public var name: String
    public var level: HubLevel
    public init(id: String, name: String, level: HubLevel) { self.id = id; self.name = name; self.level = level }
}

extension ApprovalNotice {
    /// The approval card's request from the log: the last permission request, when it is still pending.
    public static func pendingRequest(in items: [ConversationItem]) -> PermissionRequest? {
        for item in items.reversed() { if case .permissionRequest(let r) = item.kind { return r.state == .pending ? r : nil } }
        return nil
    }
}

extension Hub {
    /// The log holds this call with its result (answered or declined). Live (2.1.296 with the hook): the call
    /// itself may be in the log while the picker is open; pending, it still stands.
    static func answered(_ item: ConversationItem, id: String) -> Bool {
        if case .question(let q) = item.kind { return q.toolUseId == id && q.state != .pending }
        return false
    }

    /// The question a pane waits on, as the app's question card finds it: Claude's picker on screen (blocked only),
    /// else a pending question that ends the log while the agent is not working (Codex).
    /// `hooked`: the hook's newest call for the agent's session (`ClaudeQuestionHook`). It stands for the picker only
    /// when the switch is on, the log has no result for its id yet (an answered or declined call is stale), and the
    /// screen is its exact rendering (some question of it, focus anywhere); else the screen-read request (Mac only).
    public static func openQuestion(agent: Agent?, screen: String?, items: [ConversationItem], hooked: QuestionRequest? = nil) -> QuestionRequest? {
        guard let agent else { return nil }
        if agent.kind == "claude", agent.status == .blocked, let screen, let shown = QuestionAdapter.claudeScreen(screen) {
            if claudeHookQuestions, let h = hooked, h.agent == "claude", !screenRead(h), !items.contains(where: { Self.answered($0, id: h.toolUseId) }),
               h.questions.indices.contains(where: { QuestionAdapter.claudeExact(h, question: $0, screen: screen, other: nil, focus: .any, checks: nil) }) {
                return h
            }
            return QuestionAdapter.screenRequest(headers: shown.headers, questions: [shown.question])
        }
        guard agent.status != .working, let last = items.last, case .question(let q) = last.kind, q.state == .pending else { return nil }
        return q
    }
}

@MainActor public final class HubSession {
    /// Items sent on `watch` and per `loadEarlier`.
    public static let window = 100

    private let backend: HubBackend
    private let devices: HubDeviceStore
    public private(set) var closed = false
    private var deviceID: String?
    private var watched: [String] = []
    private var lastSnapshot: [HubWorkspace]?
    private var sent: [String: [HubItem]] = [:]
    private var lastScreen: [String: String] = [:]
    private var lastPrompt: [String: HubPrompt] = [:]
    private var lastQueued: [String: HubQueued] = [:]
    private var lastStatusLine: [String: HubStatusLine] = [:]

    private let pairing: HubPairing?

    /// `pairing` nil: `pair` is refused (the hub shows no offer).
    /// `source`: the peer's address; wrong pairing tries are counted per source.
    private let source: String

    /// Image uploads: the per-device limit (one limiter per hub), the open upload, and the finished unsent ones.
    private let uploads: HubUploadLimiter
    private struct OpenUpload { var name: String; var parts: Int; var next: Int; var data: Data }
    private var openUpload: OpenUpload?
    private var held: [(name: String, url: URL)] = []
    /// Counts `discard` calls, so an upload that finishes after one is deleted, not held.
    private var discards = 0
    /// Uploads held per connection.
    public static let maxHeld = 4

    /// Where pictures for devices are encoded (shared by the hub's connections).
    private let pictureSlots: HubWorkSlots

    public init(backend: HubBackend, devices: HubDeviceStore, pairing: HubPairing? = nil, source: String = "", uploads: HubUploadLimiter? = nil,
                pictureSlots: HubWorkSlots = .images) {
        self.backend = backend; self.devices = devices; self.pairing = pairing; self.source = source
        self.uploads = uploads ?? HubUploadLimiter()
        self.pictureSlots = pictureSlots
    }

    /// Test hook: runs after an image is decoded, before it is held.
    var afterDecode: (@MainActor () -> Void)?
    /// Test hook: runs after a picture for a device is encoded, before it is checked again and sent.
    var afterEncode: (@MainActor () async -> Void)?
    /// The level at hello, and the device whose connection slot this connection holds.
    private var helloLevel: HubLevel?
    private var slot: String?

    /// The device's level now differs from the one at hello (changed or revoked): the connection is closed, and
    /// the client says hello again to learn its level.
    public var levelChanged: Bool { deviceID != nil && device?.level != helloLevel }
    /// Input allowed right now: the device's level and the app's Allow input, looked up again on every call.
    /// Checked after every wait and again right before the keys (the backend asks `allowed`).
    private var inputAllowed: Bool { !closed && device?.level == .input && backend.writesEnabled }

    /// The connection ends for good: `discard`, and the device's connection slot is freed.
    public func end() {
        closed = true
        discard()
        if let d = slot { uploads.disconnect(device: d); slot = nil }
    }

    /// The connection ends: unsent uploads and their files go.
    public func discard() {
        discards += 1
        openUpload = nil
        for h in held { try? FileManager.default.removeItem(at: h.url) }
        if let d = deviceID, !held.isEmpty { uploads.release(device: d, count: held.count) }
        held.removeAll()
    }

    /// The device of this connection, looked up again each time (nil after a revoke).
    public var device: HubDevice? { deviceID.flatMap { devices.device(id: $0) } }
    /// Said hello once, and its device is gone now.
    public var revoked: Bool { deviceID != nil && device == nil }

    /// One client message: the pushes it causes, then its reply.
    public func receive(_ data: Data) async -> [HubMessage] {
        guard !closed else { return [] }
        let rid = Self.requestID(data)
        let m: ClientMessage
        do { m = try Hub.decodeClient(data) } catch Hub.DecodeError.unsupportedVersion(let v) {
            closed = true
            return [fail(rid, .unsupportedVersion, "This hub speaks protocol \(Hub.version), not \(v).")]
        } catch {
            return [fail(rid, .badRequest, "Not a protocol \(Hub.version) client message.")]
        }
        if case .pair(let secret, let name) = m.body {
            guard deviceID == nil else { return [fail(m.id, .badRequest, "This connection is already paired.")] }
            guard let (token, d) = pairing?.redeem(secret, name: name, source: source) else {
                closed = true
                return [fail(m.id, .unauthorized, "Wrong or expired pairing code. Start pairing again on the hub.")]
            }
            return [.paired(HubPaired(token: token, device: d)), ok(m.id)]
        }
        if case .hello(let token, _, _) = m.body {
            guard let d = devices.device(token: token) else { closed = true; return [fail(m.id, .unauthorized, "Unknown device. Pair it on the hub.")] }
            guard deviceID == nil || deviceID == d.id else { return [fail(m.id, .badRequest, "This connection belongs to another device.")] }
            if slot == nil {
                guard uploads.connect(device: d.id) else {
                    closed = true
                    return [fail(m.id, .rateLimited, "Too many connections from this device. Close another one.")]
                }
                slot = d.id
            }
            deviceID = d.id
            helloLevel = d.level
            lastSnapshot = snapshot()
            return [.welcome(HubWelcome(hub: backend.hubName, hubVersion: backend.hubVersion, level: d.level, session: backend.sessionName,
                                        pushKey: backend.pushKey, pushOn: backend.pushKey == nil ? nil : backend.pushSubscribed(device: d.id))),
                    .snapshot(lastSnapshot ?? [])]
                // Pulled texts this device's composer did not confirm yet (the connection dropped before), with the
                // warning they had (or `unfinished`, when the hub stopped mid-pull).
                + backend.pulledTexts.pending(device: d.id).map {
                    .pulled(HubPulled(id: $0.id, pane: $0.pane, text: $0.text, interrupted: $0.interrupted, note: $0.note))
                }
                + [ok(m.id)]
        }
        guard let device else { closed = true; return [fail(m.id, .unauthorized, deviceID == nil ? "Send hello first." : "This device was removed.")] }
        let canInput = inputAllowed
        let allowed: @MainActor @Sendable () -> Bool = { [weak self] in self?.inputAllowed ?? false }
        switch m.body {
        case .hello, .pair:
            return []   // handled above
        case .ping:
            return [.pong, ok(m.id)]
        case .pushSubscribe(let endpoint, let p256dh, let auth):
            // Any level may subscribe: a notification carries no conversation text. Bound to this device; the
            // allow list runs here and again in the store.
            let sub = PushSubscription(device: device.id, endpoint: endpoint, p256dh: p256dh, auth: auth)
            guard sub.isValid else { return [fail(m.id, .badRequest, "Not a push service this hub sends to.")] }
            guard backend.pushKey != nil, backend.pushSubscribe(sub) else { return [fail(m.id, .failed, "Notifications are off on this hub.")] }
            return [ok(m.id)]
        case .pushUnsubscribe(let endpoint):
            backend.pushUnsubscribe(device: device.id, endpoint: endpoint)
            return [ok(m.id)]
        case .watch(let id):
            guard let p = pane(id) else { return [fail(m.id, .notFound, "No pane \(id).")] }
            if !watched.contains(id) { watched.append(id) }
            return await pushes(p, force: true) + [ok(m.id)]
        case .unwatch(let id):
            watched.removeAll { $0 == id }
            sent[id] = nil; lastScreen[id] = nil; lastPrompt[id] = nil; lastQueued[id] = nil; lastStatusLine[id] = nil
            return [ok(m.id)]
        case .loadEarlier(let session, let before):
            return await loadEarlier(session: session, before: before, re: m.id)
        case .attach(let name, let part, let parts, let data):
            guard canInput else { return [readOnly(m.id)] }
            return await attach(m.id, device: device, name: name, part: part, parts: parts, base64: data)
        case .send(let id, let text, let attachments):
            guard canInput else { return [readOnly(m.id)] }
            guard let p = pane(id) else { return [fail(m.id, .notFound, "No pane \(id).")] }
            // A plain shell is never typed into from a remote device.
            guard p.agent != nil else { return [fail(m.id, .readOnly, "Remote input goes to agents only.")] }
            if let why = Hub.inputProblem(text, limit: Hub.maxSendText) { return [fail(m.id, .badRequest, why)] }
            guard attachments.count <= Hub.maxAttachments else { return [fail(m.id, .badRequest, "At most \(Hub.maxAttachments) images per message.")] }
            guard Set(attachments).count == attachments.count else { return [fail(m.id, .badRequest, "An image is named twice.")] }
            let images = attachments.compactMap { n in held.first { $0.name == n }?.url }
            guard images.count == attachments.count else { return [fail(m.id, .badRequest, "Unknown image. Attach it again. Nothing was sent.")] }
            guard !text.isEmpty || !images.isEmpty else { return [fail(m.id, .badRequest, "Nothing to send.")] }
            let st = await state(p)
            guard inputAllowed else { return [readOnly(m.id)] }
            guard let screen = st.screen else { return [unreadable(m.id)] }
            guard st.approval == nil, st.question == nil, !st.unreadablePrompt, p.agent?.status != .blocked, !Hub.screenTakesKeys(screen) else {
                return [fail(m.id, .promptOpen, "A prompt is open in this pane. Nothing was sent.")]
            }
            // The backend reads the pane once more right before the keys (`HerdrClient.readyForText`).
            let result = await backend.send(pane: id, text: text, images: images, agent: p.agent ?? Agent(name: "", kind: "", status: .idle, sessionId: nil, title: ""), allowed: allowed)
            if result == .sent {
                // Sent: the files stay for the agent (the folder's purge removes them later).
                let gone = held.filter { attachments.contains($0.name) }.count
                held.removeAll { attachments.contains($0.name) }
                uploads.release(device: device.id, count: gone)
            }
            if result != .sent, !inputAllowed { return [readOnly(m.id)] }
            switch result {
            case .sent: return [ok(m.id)]
            case .changed: return [fail(m.id, .promptOpen, "The pane changed before the text went out. Nothing was sent.")]
            case .failed: return [fail(m.id, .failed, "herdr did not take the message.")]
            // As the helper's server answers "opens_screen" and "box_not_empty": failed, with the note. The device
            // keeps its text and its images.
            case .refused(let why): return [fail(m.id, .failed, why)]
            }
        case .pullQueued(let id):
            guard canInput else { return [readOnly(m.id)] }
            guard let p = pane(id) else { return [fail(m.id, .notFound, "No pane \(id).")] }
            guard let agent = p.agent, QueuePull.pullKey(agent: agent.kind) != nil else {
                return [fail(m.id, .badRequest, "This agent has no queue Flockbook can pull back.")]
            }
            let st = await state(p)
            guard inputAllowed else { return [readOnly(m.id)] }
            guard let q = st.queued, !q.items.isEmpty else { return [fail(m.id, .failed, "No queued message. Nothing was sent.")] }
            guard q.pullable else { return [fail(m.id, .failed, "The agent's input box is not empty. Nothing was sent.")] }
            let log = agent.sessionId.map { backend.queuedLog(session: $0) } ?? []
            // The text is kept for this device before the agent's box is cleared, until its composer confirms it
            // (`pulledAck`); a lost connection gets it again after the next hello.
            let entry = UUID().uuidString, store = backend.pulledTexts, deviceID = device.id
            let saved = Flag()
            let keep: @MainActor @Sendable (String) -> Bool = { [weak self] text in
                // Removed while the pull ran: no composer of it will confirm the text, so it is not kept (and the
                // agent's box is not cleared: the text stays in the terminal).
                guard self?.device?.id == deviceID else { saved.on = false; return false }
                saved.on = store.keep(PulledText(id: entry, pane: id, device: deviceID, text: text, note: PulledText.unfinished))
                return saved.on
            }
            let r = await backend.pullQueued(pane: id, agent: agent, log: log, keep: keep, allowed: allowed)
            let kept = saved.on ? entry : nil
            if saved.on {
                switch r {
                case .partial(_, let why): store.finish(id: entry, note: why, interrupted: false)
                case .interrupted: store.finish(id: entry, note: nil, interrupted: true)
                default: store.finish(id: entry, note: nil, interrupted: false)
                }
            }
            switch r {
            case .pulled(let text): return [.pulled(HubPulled(id: kept, pane: id, text: text, interrupted: false)), ok(m.id)]
            case .interrupted(let text): return [.pulled(HubPulled(id: kept, pane: id, text: text, interrupted: true)), ok(m.id)]
            case .partial(let text, let why): return [.pulled(HubPulled(id: kept, pane: id, text: text, interrupted: false, note: why)), ok(m.id)]
            case .refused(let why), .failed(let why):
                if !inputAllowed { return [readOnly(m.id)] }
                return [fail(m.id, .failed, why)]
            }
        case .pulledAck(let pulled):
            _ = backend.pulledTexts.take(id: pulled, device: device.id)
            return [ok(m.id)]
        case .picture(let id, let item, let index, let full):
            // Any level: a picture is part of the conversation the device already reads.
            return await picture(m.id, device: device, pane: id, item: item, index: index, full: full)
        case .approve(let id, let prompt, let allow):
            guard canInput else { return [readOnly(m.id)] }
            guard let p = pane(id) else { return [fail(m.id, .notFound, "No pane \(id).")] }
            let st = await state(p)
            guard inputAllowed else { return [readOnly(m.id)] }
            guard st.screen != nil else { return [unreadable(m.id)] }
            let action = (allow ? ApprovalNotice.Action.allow : .deny).rawValue
            guard case .send(let a) = ApprovalNotice.outcome(actionIdentifier: action, boundPrompt: prompt, now: st.pane, writes: true) else {
                return [fail(m.id, .stale, "The prompt changed. Nothing was sent.")]
            }
            guard await backend.approve(pane: id, prompt: prompt, allow: a, agent: p.agent ?? Agent(name: "", kind: "", status: .idle, sessionId: nil, title: ""), allowed: allowed) else {
                if !inputAllowed { return [readOnly(m.id)] }
                return [fail(m.id, .stale, "The prompt changed before the keys went out. Nothing was sent.")]
            }
            return [ok(m.id)]
        case .answer(let id, let question, let answers):
            guard canInput else { return [readOnly(m.id)] }
            guard let p = pane(id) else { return [fail(m.id, .notFound, "No pane \(id).")] }
            for a in answers { if let o = a.other, let why = Hub.inputProblem(o, limit: Hub.maxOtherText) { return [fail(m.id, .badRequest, why)] } }
            let st = await state(p)
            guard inputAllowed else { return [readOnly(m.id)] }
            guard st.screen != nil else { return [unreadable(m.id)] }
            guard let q = st.question, Hub.questionID(q) == question else { return [fail(m.id, .stale, "The question changed. Nothing was sent.")] }
            guard q.delivery == .picker else { return [fail(m.id, .failed, "Answer this question in the terminal.")] }
            guard !Hub.answerOnMac(q, agent: p.agent) else { return [fail(m.id, .failed, "Answer this question on the Mac. Nothing was sent.")] }
            // A hooked request: before every key, still the session's newest hook record and not answered in the log
            // (another request with the same picker must not get this answer).
            var keysAllowed = allowed
            if q.agent == "claude", !Hub.screenRead(q) {
                guard let sid = p.agent?.sessionId else { return [fail(m.id, .stale, "The question changed. Nothing was sent.")] }
                keysAllowed = { [weak self] in
                    guard let self, allowed(), self.backend.hookedQuestion(session: sid)?.toolUseId == q.toolUseId else { return false }
                    return !self.backend.conversation(session: sid).contains { Hub.answered($0, id: q.toolUseId) }
                }
            }
            do {
                try await backend.answer(pane: id, question: q, answers: answers.map { QuestionAnswer(picks: $0.picks, other: $0.other) }, agent: p.agent ?? Agent(name: "", kind: "", status: .idle, sessionId: nil, title: ""), allowed: keysAllowed)
                return [ok(m.id)]
            } catch {
                if !inputAllowed { return [readOnly(m.id)] }
                return [fail(m.id, .failed, "\(error)")]
            }
        }
    }

    /// Pushes since the last call: a new snapshot, and for watched panes new items, screen and prompt changes.
    public func tick() async -> [HubMessage] {
        guard !closed, deviceID != nil else { return [] }
        guard device != nil, !levelChanged else { closed = true; return [] }
        var out: [HubMessage] = []
        let snap = snapshot()
        if snap != lastSnapshot { lastSnapshot = snap; out.append(.snapshot(snap)) }
        for id in watched {
            if let p = pane(id) { out += await pushes(p, force: false) }
        }
        return out
    }

    // MARK: - Images

    private static func uploadName(_ s: String) -> Bool {
        (1...32).contains(s.count) && s.unicodeScalars.allSatisfy { ("a"..."z").contains($0) || ("A"..."Z").contains($0) || ("0"..."9").contains($0) || $0 == "_" || $0 == "-" }
    }

    /// One `attach` part (remote-protocol.md 3.3). Any refused part drops the open upload.
    private func attach(_ re: String, device: HubDevice, name: String, part: Int, parts: Int, base64: String) async -> [HubMessage] {
        guard let dir = backend.imageDirectory else { return [fail(re, .failed, "This hub takes no images.")] }
        let open = openUpload
        openUpload = nil
        func bad(_ why: String) -> [HubMessage] { [fail(re, .badRequest, why)] }
        guard Self.uploadName(name) else { return bad("Upload names are 1-32 letters, digits, _ or -.") }
        guard (1...HubImage.maxParts).contains(parts), (0..<parts).contains(part) else { return bad("An image has 1-\(HubImage.maxParts) parts.") }
        if let open {
            guard open.name == name, open.parts == parts, open.next == part else { return bad("Parts of one image come in order, one image at a time.") }
        } else {
            guard part == 0 else { return bad("Parts of one image come in order, one image at a time.") }
            guard !held.contains(where: { $0.name == name }) else { return bad("Upload \(name) is already held.") }
            // The device holds its cap over all its connections (this one could only free a slot by dropping its own).
            guard held.count >= Self.maxHeld || uploads.held(device: device.id) < HubUploadLimiter.maxHeld else { return [tooMany(re)] }
        }
        // Base64 of 640 KiB is under 874 KiB; a longer string is refused before decoding.
        let size = base64.utf8.count
        guard size <= (HubImage.maxPartBytes + 2) / 3 * 4 else { return bad("A part is at most \(HubImage.maxPartBytes / 1024) KiB.") }
        guard (open?.data.count ?? 0) + size / 4 * 3 <= HubImage.maxBytes else { return bad("An image is at most \(HubImage.maxBytes / 1024 / 1024) MiB.") }
        // The limit is counted before any decoding, by the size the string decodes to.
        guard uploads.take(device: device.id, bytes: size / 4 * 3, image: part == 0) else { return [tooMany(re)] }
        guard let data = Data(base64Encoded: base64) else { return bad("Part data is not base64.") }
        guard data.count <= HubImage.maxPartBytes else { return bad("A part is at most \(HubImage.maxPartBytes / 1024) KiB.") }
        let all = (open?.data ?? Data()) + data
        guard part == parts - 1 else {
            openUpload = OpenUpload(name: name, parts: parts, next: part + 1, data: all)
            return [ok(re)]
        }
        // Decoding is off the main thread, at most 2 at once for the hub; a `discard` meanwhile deletes the result.
        // The folder's own files are capped right after (count, bytes, age).
        let round = discards
        let result = await HubWorkSlots.images.run { () -> Result<HubImage.Output, HubImage.Failure> in
            do {
                let out = try HubImage.reencode(all, into: dir)
                HubImage.purge(dir)
                return .success(out)
            } catch { return .failure(error as? HubImage.Failure ?? .undecodable) }
        }
        afterDecode?()
        switch result {
        case .failure(let f):
            switch f {
            case .notImage: return bad("Not a PNG, JPEG, HEIC or WebP image.")
            case .tooLarge: return bad("The image is too large (at most \(HubImage.maxSide) px a side, \(HubImage.maxPixels / 1_000_000) megapixels).")
            case .undecodable: return bad("The image could not be read.")
            case .writeFailed: return [fail(re, .failed, "The hub could not store the image.")]
            }
        case .success(let out):
            uploads.count(device: device.id, bytes: out.bytes)
            guard round == discards, !held.contains(where: { $0.name == name }) else {
                try? FileManager.default.removeItem(at: out.url)
                return bad("The upload was dropped.")
            }
            // Downgraded or revoked while decoding: the file goes.
            guard inputAllowed else { try? FileManager.default.removeItem(at: out.url); return [readOnly(re)] }
            if held.count >= Self.maxHeld {
                try? FileManager.default.removeItem(at: held.removeFirst().url)
                uploads.release(device: device.id)
            }
            guard uploads.hold(device: device.id) else { try? FileManager.default.removeItem(at: out.url); return [tooMany(re)] }
            held.append((name, out.url))
            return [.attached(HubAttached(upload: name, width: out.width, height: out.height, bytes: out.bytes)), ok(re)]
        }
    }

    private enum Picture: Sendable { case gone, undecodable, encoded(HubImage.Encoded) }

    /// One picture of a watched pane's conversation (remote-protocol.md 3.4), read back from the log (the ref's digest
    /// checked) and re-encoded off the main thread. At most `HubUploadLimiter.maxPictureJobs` per device at once; not
    /// encoded when no longer wanted once a slot is free. Checked again after encoding and before each part (`vet`).
    private func picture(_ re: String, device: HubDevice, pane id: String, item: String, index: Int, full: Bool) async -> [HubMessage] {
        let off = fail(re, .failed, "Pictures are off on this hub.")
        guard backend.remotePictures else { return [off] }
        guard watched.contains(id) else { return [fail(re, .badRequest, "Watch the pane first.")] }
        guard let session = pane(id)?.agent?.sessionId else { return [fail(re, .notFound, "No conversation in pane \(id).")] }
        guard let found = backend.conversation(session: session).first(where: { $0.id == item }) else { return [fail(re, .notFound, "No item \(item).")] }
        let refs = Hub.pictures(of: found)
        guard refs.indices.contains(index) else { return [fail(re, .notFound, "No picture \(index) in item \(item).")] }
        let gone = fail(re, .notFound, "The picture is no longer in the log.")
        guard let url = backend.logURL(session: session) else { return [gone] }
        let ref = refs[index]
        // Taken until the job ends, also when the connection ends meanwhile.
        guard uploads.beginPicture(device: device.id) else { return [fail(re, .rateLimited, "Too many pictures at once. Try again soon.")] }
        defer { uploads.endPicture(device: device.id) }
        let result = await pictureSlots.run(if: { !self.closed && self.pictureChanged(re, device: device.id, pane: id, session: session) == nil }) { () -> Picture in
            guard let data = LogImage.read(ref, from: url) else { return .gone }
            guard let e = full ? HubImage.full(data) : HubImage.preview(data) else { return .undecodable }
            return .encoded(e)
        }
        await afterEncode?()
        guard !closed else { return [] }
        if let why = pictureChanged(re, device: device.id, pane: id, session: session) { return [why] }
        guard let result else { return [fail(re, .stale, "The pane changed. No picture was sent.")] }
        switch result {
        case .gone: return [gone]
        case .undecodable: return [fail(re, .failed, "The picture could not be read.")]
        case .encoded(let e):
            let parts = HubImage.parts(e.data)
            sending[re] = (device.id, id, session)
            return parts.enumerated().map { i, d in
                .picturePart(HubPicturePart(re: re, pane: id, item: item, index: index, full: full, part: i, parts: parts.count, mime: "image/jpeg",
                                            width: e.width, height: e.height, data: d.base64EncodedString()))
            } + [ok(re)]
        }
    }

    /// The reply when a picture is no longer wanted: the device removed (and the connection closed), the setting off,
    /// the pane not watched or showing another session. Nil while it is.
    private func pictureChanged(_ re: String, device: String, pane id: String, session: String) -> HubMessage? {
        guard self.device?.id == device else { closed = true; return fail(re, .unauthorized, "This device was removed.") }
        guard backend.remotePictures else { return fail(re, .failed, "Pictures are off on this hub.") }
        guard watched.contains(id), pane(id)?.agent?.sessionId == session else { return fail(re, .stale, "The pane changed. No picture was sent.") }
        return nil
    }

    /// Pictures whose parts are on their way, by request (device, pane, session); requests stopped part way, by the
    /// reply that replaces their `ok`.
    private var sending: [String: (device: String, pane: String, session: String)] = [:]
    private var stopped: [String: HubMessage] = [:]

    /// A message about to go out, checked again (the server sends a step's messages one by one): a picture part no
    /// longer wanted is not sent, nor the rest of its picture, and its reply says why.
    public func vet(_ m: HubMessage) -> HubMessage? {
        switch m {
        case .picturePart(let p):
            if closed || stopped[p.re] != nil { return nil }
            guard let s = sending[p.re] else { return m }
            if let why = pictureChanged(p.re, device: s.device, pane: s.pane, session: s.session) {
                sending[p.re] = nil
                stopped[p.re] = why
                return nil
            }
            return m
        case .reply(let r):
            sending[r.re] = nil
            return stopped.removeValue(forKey: r.re) ?? m
        default: return m
        }
    }

    // MARK: - Helpers

    /// A conversation item as devices get it: its pictures named only while the setting is on (a change sends the items again).
    private func hubItem(_ item: ConversationItem) -> HubItem { HubItem(item, pictures: backend.remotePictures) }
    private func snapshot() -> [HubWorkspace] { backend.workspaces.map(HubWorkspace.init) }
    private func pane(_ id: String) -> Pane? { backend.workspaces.lazy.flatMap(\.tabs).flatMap(\.panes).first { $0.id == id } }
    private func ok(_ re: String) -> HubMessage { .reply(HubReply(re: re, ok: true)) }
    private func fail(_ re: String, _ code: HubError.Code, _ message: String) -> HubMessage {
        .reply(HubReply(re: re, ok: false, error: HubError(code: code, message: message)))
    }
    private func readOnly(_ re: String) -> HubMessage { fail(re, .readOnly, "Input is off for this device.") }
    private func tooMany(_ re: String) -> HubMessage { fail(re, .rateLimited, "Too many images from this device. Try again in a few minutes.") }
    /// Fail closed: without the screen the hub cannot tell what the keys would answer.
    private func unreadable(_ re: String) -> HubMessage { fail(re, .failed, "Could not read the pane. Nothing was sent.") }

    /// The id of a message that did not decode, so its error reply can still name it.
    static func requestID(_ data: Data) -> String {
        ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any])?["id"] as? String ?? ""
    }

    private struct PaneNow {
        var screen: String?
        var pane: ApprovalNotice.PaneState?
        var approval: HubApproval?
        var question: QuestionRequest?
        /// A permission prompt on screen that the parser cannot read: counted as open.
        var unreadablePrompt = false
        /// The agent's own queue (`QueuedInput`).
        var queued: QueuedInput?
    }

    /// The pane as it is now: one screen read, the log, and the open prompt by the app's rules.
    private func state(_ p: Pane) async -> PaneNow {
        let screen = await backend.screen(pane: p.id)
        let items = p.agent?.sessionId.map { backend.conversation(session: $0) } ?? []
        let logged = ApprovalNotice.pendingRequest(in: items)
        let ps = p.agent.map { ApprovalNotice.PaneState(agentKind: $0.kind, blocked: $0.status == .blocked, pendingToolUseId: logged?.toolUseId, screen: screen ?? "",
                                                        pendingTool: logged?.toolName, pendingCommand: logged?.command) }
        let approval = ps.flatMap { Hub.openApproval($0, logged: logged) }
        let hooked = p.agent.flatMap { $0.kind == "claude" && $0.status == .blocked ? $0.sessionId : nil }.flatMap { backend.hookedQuestion(session: $0) }
        let question = Hub.openQuestion(agent: p.agent, screen: screen, items: items, hooked: hooked)
        let unreadable = approval == nil && question == nil && p.agent?.kind == "claude"
            && Hub.unreadablePrompt(screen: screen ?? "", blocked: p.agent?.status == .blocked)
        let log = p.agent?.sessionId.map { backend.queuedLog(session: $0) } ?? []
        let queued = QueuedInput.read(agent: p.agent, screen: screen, log: log)
        return PaneNow(screen: screen, pane: ps, approval: approval, question: question, unreadablePrompt: unreadable, queued: queued)
    }

    private func pushes(_ p: Pane, force: Bool) async -> [HubMessage] {
        var out: [HubMessage] = []
        if let c = conversationPush(p, force: force) { out.append(c) }
        let st = await state(p)
        if let s = st.screen, force || s != lastScreen[p.id] { lastScreen[p.id] = s; out.append(.screen(pane: p.id, text: s)) }
        let prompt = HubPrompt(pane: p.id, approval: st.approval, question: st.question.map { HubQuestion($0, paneAgent: p.agent) })
        if force || prompt != lastPrompt[p.id] { lastPrompt[p.id] = prompt; out.append(.prompt(prompt)) }
        // The queue: on watch only when there is one; then each change, an empty one clearing it.
        let queued = HubQueued(pane: p.id, st.queued)
        if force ? !queued.items.isEmpty : queued != (lastQueued[p.id] ?? HubQueued(pane: p.id, nil)) { out.append(.queued(queued)) }
        lastQueued[p.id] = queued
        // The status line (Claude Code and Grok Build), the same way.
        let line = HubStatusLine(pane: p.id, record: p.agent.flatMap { a in
            a.kind == "claude" || a.kind == "grok" ? a.sessionId.flatMap { backend.statusLine(kind: a.kind, session: $0) } : nil
        })
        if force ? !line.isEmpty : line != (lastStatusLine[p.id] ?? HubStatusLine(pane: p.id, segments: [])) { out.append(.statusLine(line)) }
        lastStatusLine[p.id] = line
        return out
    }

    /// `append` when the client's items are still a prefix of the log (changed items are sent again by id), else `replace`.
    private func conversationPush(_ p: Pane, force: Bool) -> HubMessage? {
        guard let session = p.agent?.sessionId else { return nil }
        let items = backend.conversation(session: session)
        let old = force ? nil : sent[p.id]
        if let old, let first = old.first, let start = items.firstIndex(where: { $0.id == first.id }) {
            let cur = items[start...].map(hubItem)
            if cur.count >= old.count, zip(old, cur).allSatisfy({ $0.id == $1.id }) {
                sent[p.id] = cur
                let delta = zip(old, cur).filter { $0 != $1 }.map(\.1) + cur[old.count...]
                guard !delta.isEmpty else { return nil }
                return .conversation(HubConversation(pane: p.id, session: session, mode: .append, items: delta,
                                                     hasEarlier: start > 0 || backend.hasEarlier(session: session)))
            }
        }
        if let old, old.isEmpty, items.isEmpty { return nil }
        let tail = Array(items.suffix(Self.window).map(hubItem))
        if let old, old == tail { return nil }
        sent[p.id] = tail
        return .conversation(HubConversation(pane: p.id, session: session, mode: .replace, items: tail,
                                             hasEarlier: items.count > tail.count || backend.hasEarlier(session: session)))
    }

    private func loadEarlier(session: String, before: String, re: String) async -> [HubMessage] {
        guard let p = backend.workspaces.lazy.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.agent?.sessionId == session }) else {
            return [fail(re, .notFound, "No pane shows session \(session).")]
        }
        var items = backend.conversation(session: session)
        guard var idx = items.firstIndex(where: { $0.id == before }) else { return [fail(re, .notFound, "No item \(before).")] }
        var more = backend.hasEarlier(session: session)
        if idx < Self.window, more || idx == 0 {
            let result = await backend.loadEarlier(session: session)
            guard result != .failed else { return [fail(re, .failed, "Could not load earlier messages. Check the host connection and try again.")] }
            more = result == .more
            items = backend.conversation(session: session)
            idx = items.firstIndex { $0.id == before } ?? idx
        }
        let lo = max(0, idx - Self.window)
        let older = items[lo..<idx].map(hubItem)
        if let held = sent[p.id], held.first?.id == before { sent[p.id] = older + held }
        return [.conversation(HubConversation(pane: p.id, session: session, mode: .prepend, items: older, hasEarlier: lo > 0 || more)), ok(re)]
    }
}

/// A flag a `@Sendable` closure on the main actor sets.
@MainActor final class Flag { var on = false }
