import HerdrKit
import SwiftUI
import Observation
import os

/// Notification button decisions: `log show --predicate 'subsystem == "dev.mclee.flockbook" && category == "notice"' --info`.
let noticeLog = Logger(subsystem: "dev.mclee.flockbook", category: "notice")

enum Screen: Hashable { case tab, inbox, overview, search, settings }

@MainActor @Observable
final class AppModel {
    let store: HerdrStore
    /// Remote access; nil in checks and previews (they never listen).
    var remote: RemoteHub?
    var screen: Screen = .tab { didSet { if screen != .tab { showTerminal = false }; updateVisibleSessions() } }
    var workspaceID: String? { didSet { updateVisibleSessions() } }
    var tabID: String? { didSet { updateVisibleSessions() } }
    var showTerminal = false
    /// The Settings page covers the screen under it (`underSettings` in its views): nothing there sends to a pane.
    var coveredBySettings: Bool { screen == .settings }
    /// Settings is a page of the main window (sidebar foot, ⌘,).
    /// While it shows, Settings has the sidebar (Back and its tabs) and the toolbar has no tab row.
    func showSettings() {
        if screen != .settings { screenBeforeSettings = screen }
        screen = .settings
    }
    /// ⌘,: opens Settings, or leaves it when it is open.
    func toggleSettings() { screen == .settings ? closeSettings() : showSettings() }
    /// Back and Esc: the screen from before Settings, its workspace and tab as they were (Settings never changes them).
    func closeSettings() {
        guard screen == .settings else { return }
        screen = screenBeforeSettings
    }
    /// The screen Settings was opened from: it stays in the window under Settings (hidden), so Back finds it as it was.
    private(set) var screenBeforeSettings: Screen = .tab
    var splitDirection: String = "down"
    var searchText = ""
    /// Width of the tab row toolbar item. Follows the detail column, never the tab count (see `setDetailWidth`).
    var tabRowWidth: CGFloat = 420
    @ObservationIgnored var toolbarNudge: (() -> Void)?

    /// Free width of the detail column (window minus the docked sidebar, from SwiftUI geometry) -> room for the tab row.
    /// The tab island is centered (principal), so the room is symmetric: the wider side reserve counts twice.
    /// Left reserve: sidebar toggle (~40) and, with the sidebar hidden, the traffic lights (~80); right: system padding.
    func setDetailWidth(_ detail: CGFloat) {
        let left: CGFloat = 40 + (sidebarVisibility == .detailOnly ? 80 : 0), right: CGFloat = 25
        let width = max(240, detail - 2 * max(left, right) - 20)
        guard abs(tabRowWidth - width) >= 1 else { return }
        tabRowWidth = width
        toolbarNudge?()
    }
    /// A terminal shows its history view: Esc leaves that first, it must not close the terminal sheet.
    var terminalHistoryOpen = false
    var sidebarVisibility: NavigationSplitViewVisibility = .all
    /// Per-tab pane focus and zoom. App-side only; herdr's own zoom is never touched.
    var focusedPane: [String: String] = [:]
    var zoomedPane: [String: String] = [:] { didSet { updateVisibleSessions() } }
    var renamingTabID: String?
    /// Hover reveal: the sidebar floats over the content while the pointer rests on the left edge (only when the owner hid it).
    var sidebarPeek = false
    @ObservationIgnored private var peekStrip = false
    @ObservationIgnored private var peekPanel = false
    @ObservationIgnored private var peekTask: Task<Void, Never>?
    /// Workspace management UI state: the New workspace sheet, the workspace being renamed inline, the one awaiting close confirmation.
    var newWorkspaceOpen = false
    var renamingWorkspaceID: String?
    var closingWorkspace: Workspace?
    /// Tests only: replaces the calls to herdr for workspace writes (name of the call, workspace id or label).
    var workspaceHook: ((String, String) -> Void)?
    /// Last time the workspace tree changed (for the "Reconnecting…" footer).
    var lastUpdate: Date?

    func rename(_ tab: Tab, to label: String) {
        renamingTabID = nil
        let t = label.trimmingCharacters(in: .whitespacesAndNewlines)
        guard writesEnabled, layoutOK(tab), !t.isEmpty, t != tab.label else { return }
        runLayout(tab.backend, "Rename tab") { try await $0.renameTab(tab.id, to: t) }
    }

    init(store: HerdrStore) {
        self.store = store
        workspaceID = store.workspaces.first?.id
        tabID = store.workspaces.first?.tabs.first?.id
    }

    var workspace: Workspace? { store.workspaces.first { $0.id == workspaceID } ?? store.workspaces.first }
    var tab: Tab? { workspace?.tabs.first { $0.id == tabID } ?? workspace?.tabs.first }
    var allBlocked: [(Workspace, Tab)] {
        store.workspaces.flatMap { ws in ws.tabs.filter { $0.status == .blocked }.map { (ws, $0) } }
    }

    /// Writes (input, tab/pane changes) are on by default for the user. Off with `--read-only` (always use it
    /// when testing against the default session) or with the "Allow sending input" setting.
    var writesEnabled: Bool {
        if CommandLine.arguments.contains("--read-only") { return false }
        return AppDefaults.store.object(forKey: AppSettings.allowInputKey) as? Bool ?? true
    }

    /// Layout writes (new/close/rename/move tab, split, workspace changes) for the current workspace. On hc they go
    /// to the official app's document (`HCLayout.swift`); the "Other terminals" group has no layout.
    var layoutEnabled: Bool { writesEnabled && (workspace.map(layoutOK) ?? true) }

    /// Whether layout writes apply: TUI items, and hc spaces/tabs/panes from the official app's labels.
    func layoutOK(_ ws: Workspace) -> Bool { ws.backend == .tui || HCClient.layoutWritable(id: ws.id) }
    func layoutOK(_ t: Tab) -> Bool { t.backend == .tui || HCClient.layoutWritable(id: t.id) }
    func layoutOK(_ p: Pane, in t: Tab?) -> Bool { p.backend == .tui || (t.map(layoutOK) ?? false) }

    /// Runs a layout write on `backend`, then refreshes. A failure shows as a notice (on hc it says why the
    /// official app's layout was left alone).
    func runLayout(_ backend: BackendKind, _ what: String, then: (@MainActor () -> Void)? = nil,
                   _ op: @escaping @Sendable (any LayoutBackend) async throws -> Void) {
        guard let l = store.layout(backend) else { return }
        Task {
            do { try await op(l) } catch { notice = "\(what) failed: \(error)" }
            await store.refresh()
            then?()
        }
    }

    /// Bumped on every send: the conversation of that pane scrolls to the bottom (wherever the reader was).
    /// Panes whose right clicks go to the program (herdr's setting is write-only, so the app remembers it).
    var rightClickToPane: Set<String> = []
    var renamingPane: Pane?
    var paneRenameDraft = ""
    var closingPane: Pane?
    var sendTokens: [String: Int] = [:]
    func noteSent(_ paneID: String) { sendTokens[paneID, default: 0] += 1 }

    func send(_ text: String, to pane: Pane) {
        guard writesEnabled else { return }
        let id = pane.id, kind = pane.agent?.kind
        Task { await deliver(text, pane: id, kind: kind) }
    }
    /// Test hook: when set, replaces the call to herdr (tests must never create a real tab).
    var newTabHook: ((Workspace) -> Void)?
    func newTab() {
        guard writesEnabled, let ws = workspace, layoutOK(ws) else { return }
        if let hook = newTabHook { hook(ws); return }
        let cwd = tab?.panes.first?.cwd, wsID = ws.id, before = Set(ws.tabs.map(\.id))
        runLayout(ws.backend, "New tab", then: { [weak self] in self?.selectCreatedTab(in: wsID, before: before) }) {
            try await $0.createTab(workspaceId: wsID, label: nil, cwd: cwd)
        }
    }
    /// Selects the tab herdr just created: the tab of `wsID` that was not in `before` (tab.create gives no id to the
    /// app). Called in the same main-actor turn as the refresh, so the old tab is not drawn with the new list.
    func selectCreatedTab(in wsID: String, before: Set<String>) {
        guard let ws = store.workspaces.first(where: { $0.id == wsID }),
              let id = Self.createdTabID(before: before, after: ws.tabs.map(\.id)) else { return }
        workspaceID = wsID
        tabID = id
    }
    /// The last id in `after` that is not in `before`; nil when no tab appeared.
    static func createdTabID(before: Set<String>, after: [String]) -> String? { after.last { !before.contains($0) } }
    func closeTab() { if let t = tab { closeTab(t) } }
    /// Test hook: when set, replaces the call to herdr (tests must never close a real tab).
    var closeTabHook: ((Tab) -> Void)?
    func closeTab(_ t: Tab) {
        guard writesEnabled, layoutOK(t) else { return }
        if let hook = closeTabHook { hook(t); return }
        if t.id == tab?.id, let ws = workspace, let i = ws.tabs.firstIndex(of: t), ws.tabs.count > 1 {
            tabID = ws.tabs[i + 1 < ws.tabs.count ? i + 1 : i - 1].id
        }
        let id = t.id
        runLayout(t.backend, "Close tab") { try await $0.closeTab(id) }
    }
    /// The toolbar's +: what Settings > General chose (`PlusAction`), on the focused pane; no pane: a new tab.
    func plus() {
        let action = PlusAction(stored: AppDefaults.store.string(forKey: PlusAction.key))
        let hasPane = tab.map { focusedPaneID(in: $0) != nil } ?? false
        if let dir = action.resolved(hasPane: hasPane).split { splitFocused(dir) } else { newTab() }
    }
    /// Test hook: when set, replaces the call to herdr (tests must never split a real pane).
    var splitHook: ((Pane, SplitDirection) -> Void)?
    func splitFocused(_ dir: SplitDirection) {
        guard writesEnabled, let t = tab, layoutOK(t), let fid = focusedPaneID(in: t), let p = t.panes.first(where: { $0.id == fid }) else { return }
        splitDirection = dir.rawValue
        if let hook = splitHook { hook(p, dir); return }
        let id = p.id
        runLayout(t.backend, "Split") { try await $0.split(paneId: id, direction: dir) }
    }

    /// terminal ids whose latest screen shows a Claude permission prompt (checked from observe frames, read-only).
    var promptVisible: Set<String> = []
    /// Panes whose card answer is on its way: their card stays off and presses are ignored until the re-check after it.
    let approvals = ApprovalSender()
    var notice: String?

    /// True when the log ends with a pending tool call for this pane's agent.
    func hasPending(_ pane: Pane) -> Bool {
        store.conversation(for: pane.agent?.sessionId).contains {
            if case .permissionRequest(let r) = $0.kind { return r.state == .pending }
            return false
        }
    }

    /// Screen reads and card keys for this pane's backend: `terminal.read` / `terminal.keys` on hc, herdr's pane calls on TUI.
    func screen(for pane: Pane) -> (any PaneScreen)? { screen(paneId: pane.id) }
    /// The same by pane id (the hub's calls name a pane by id): nil for an hc pane while hc is off.
    func screen(paneId id: String) -> (any PaneScreen)? {
        HCMapper.address(fromPaneId: id) != nil ? store.hcClient : store.client
    }

    /// Asks the kit whether the visible screen shows Claude's permission prompt.
    /// Runs only when `PermissionGate.checkApproval` says so (hc: blocked, not a question; TUI: blocked or a pending
    /// tool call); otherwise clears the flag. Also keeps the prompt read from the screen (`screenPrompts`) for a blocked
    /// Claude, Grok or agy. A prompt whose answer is on the way stays off (its buttons stay off).
    func checkPrompt(for pane: Pane) async {
        guard PermissionGate.checkApproval(pane.agent, pendingTool: hasPending(pane)), let reader = screen(for: pane) else {
            promptVisible.remove(pane.id); screenPrompts[pane.id] = nil; return
        }
        let screen = try? await reader.visibleText(paneId: pane.id)
        if let screen, PermissionGate.screenShowsPrompt(screen), !approvals.isAnswering(pane.id) { promptVisible.insert(pane.id) }
        else { promptVisible.remove(pane.id) }
        let a = pane.agent
        screenPrompts[pane.id] = a?.status == .blocked && (a?.kind == "claude" || a?.kind == "grok" || a?.kind == "agy")
            ? screen.flatMap { ApprovalNotice.screenRequest($0, agent: a?.kind ?? "") } : nil
    }

    /// While a tab's card can show: re-reads the screen of its blocked panes (and of panes whose card is up) every 2 s,
    /// so a prompt answered in the terminal or replaced by the next one does not leave a stale card. Read-only.
    func watchPrompts(tabId: String) async {
        while !Task.isCancelled {
            let panes = store.workspaces.flatMap(\.tabs).first { $0.id == tabId }?.panes ?? []
            for p in panes where p.agent?.status == .blocked || promptVisible.contains(p.id) { await checkPrompt(for: p) }
            try? await Task.sleep(for: .seconds(2))
        }
    }

    /// Permission prompts read from the screen, by pane id: the card's request when the log has none (a Claude without a
    /// transcript, or a log that lags the screen).
    var screenPrompts: [String: PermissionRequest] = [:]

    /// The request the approval card shows and binds its buttons to: the log's pending call when it is the prompt on
    /// screen, else the prompt on screen (`ApprovalNotice.cardRequest`).
    func approvalRequest(_ pane: Pane) -> PermissionRequest? {
        ApprovalNotice.cardRequest(logged: loggedRequest(pane), onScreen: pane.agent?.status == .blocked ? screenPrompts[pane.id] : nil)
    }

    /// The log's last permission request, when it is still pending.
    private func loggedRequest(_ pane: Pane) -> PermissionRequest? {
        let logged = store.conversation(for: pane.agent?.sessionId).reversed().lazy.compactMap { item -> PermissionRequest? in
            if case .permissionRequest(let r) = item.kind { return r }
            return nil
        }.first
        return logged?.state == .pending ? logged : nil
    }

    /// Where the pane's live approval card stands (`ApprovalNotice.cardPlace`): one card, bound to `approvalRequest`'s
    /// id, as the tab's card and the notification. Nothing while the prompt is not on screen.
    func cardPlace(_ pane: Pane) -> ApprovalNotice.CardPlace {
        guard promptVisible.contains(pane.id) else { return .none }
        return ApprovalNotice.cardPlace(pending: loggedRequest(pane), card: approvalRequest(pane))
    }
    /// The pending call in the conversation that gets Allow / Deny, if the card stands there.
    func liveCardInConversation(_ pane: Pane) -> String? {
        if case .conversation(let id) = cardPlace(pane) { return id }
        return nil
    }
    /// The card from the screen in the composer slot, if the card stands there.
    func cardInComposer(_ pane: Pane) -> PermissionRequest? {
        if case .composer(let r) = cardPlace(pane) { return r }
        return nil
    }

    /// Allow once = "1" (no Enter), Deny = Escape (verified in a named test session). Only when the prompt is on screen.
    /// `bound`: the request the card showed (nil: the card's request now). The keys go out only while that prompt is
    /// still open (`respondIfOpen` reads the pane right before them); else nothing is sent and the terminal opens.
    /// One answer per prompt across the card, the notification buttons and the hub (`ApprovalSender.answer`): a
    /// second press while the first is on its way does nothing.
    func approve(pane: Pane, allow: Bool, bound: PermissionRequest? = nil) {
        if approvals.isAnswering(pane.id) { return }
        guard writesEnabled, promptVisible.contains(pane.id), let agent = pane.agent, let screen = screen(for: pane),
              let prompt = (bound ?? approvalRequest(pane))?.toolUseId else { showTerminal = true; return }
        let id = pane.id
        let pending = pendingRequest(pane)
        promptVisible.remove(id)   // the card's buttons turn off at once (canRespond reads promptVisible)
        Task {
            let r = await approvals.answer(paneId: id, via: screen, prompt: prompt, allow: allow, agentKind: agent.kind,
                                           blocked: agent.status == .blocked, pending: pending)
            noticeLog.info("card \(allow ? "allow" : "deny", privacy: .public) pane \(id, privacy: .public) prompt \(prompt, privacy: .public): \(String(describing: r), privacy: .public)")
            switch r {
            case .busy: return
            case .failed: notice = "The answer was not sent. Open the terminal to check."; showTerminal = true
            case .notSent: notice = "The prompt changed. Nothing was sent."; showTerminal = true
            case .unconfirmed: notice = Self.unconfirmedAnswer
            case .sent:
                try? await Task.sleep(for: .seconds(1.5))
                await store.refresh()
                let still = store.workspaces.flatMap(\.tabs).flatMap(\.panes).first { $0.id == id }?.agent?.status == .blocked
                if still { notice = "Status did not change. Open the terminal to check."; showTerminal = true }
            }
            await checkPrompt(for: pane)
        }
    }

    /// The tool call the pane's log ends with, still without a result (the approval card's request).
    func pendingRequest(_ pane: Pane) -> PermissionRequest? {
        ApprovalNotice.pendingRequest(in: store.conversation(for: pane.agent?.sessionId))
    }
    func pendingToolUseId(_ pane: Pane) -> String? { pendingRequest(pane)?.toolUseId }
    /// hc held the answer's keys past their timeout: they may still land, so it is not sent again.
    static let unconfirmedAnswer = "Answer sent, not confirmed: no reply in time. Check the terminal before you answer again."

    /// Read-only: the blocked pane's state for the notification buttons (reads its visible screen).
    func approvalState(_ pane: Pane) async -> ApprovalNotice.PaneState? {
        guard let a = pane.agent else { return nil }
        return await screen(for: pane)?.approvalState(paneId: pane.id, agentKind: a.kind, blocked: a.status == .blocked, pending: pendingRequest(pane))
    }

    /// The pane and prompt a new "needs you" notification may answer with Allow / Deny. Nil: no buttons. Read-only.
    /// Tries a few times: the log's tool call and the prompt on screen can come a moment after the blocked status.
    func approvalPrompt(in tab: Tab) async -> (pane: String, prompt: String, request: PermissionRequest?)? {
        for attempt in 0..<3 {
            guard writesEnabled else { noticeLog.info("no buttons: input is off"); return nil }
            if attempt > 0 { try? await Task.sleep(for: .milliseconds(700)) }
            let t = store.workspaces.flatMap(\.tabs).first { $0.id == tab.id } ?? tab
            guard let pane = t.panes.first(where: { $0.agent?.status == .blocked }) else { return nil }
            if let s = await approvalState(pane), let id = ApprovalNotice.prompt(s, writes: writesEnabled) {
                promptVisible.insert(pane.id)
                noticeLog.info("buttons: pane \(pane.id, privacy: .public) prompt \(id, privacy: .public)")
                // The request the id names, for the notification's text: the log's call, else the box on screen.
                let logged = pendingRequest(pane)
                return (pane.id, id, logged?.toolUseId == id ? logged : ApprovalNotice.screenRequest(s.screen, agent: s.agentKind))
            }
            noticeLog.info("no buttons yet: pane \(pane.id, privacy: .public) try \(attempt) log call \(self.pendingToolUseId(pane) ?? "none", privacy: .public)")
        }
        noticeLog.info("no buttons: tab \(tab.id, privacy: .public)")
        return nil
    }

    /// Allow / Deny chosen on a notification. Re-reads the pane; only when it still shows the same prompt does it send
    /// the approval card's input (`approve`). Otherwise nothing is sent and the tab opens. Returns true when it sent.
    @discardableResult
    func answerNotice(action: String, paneID: String, prompt: String?) async -> Bool {
        await store.refresh()
        let pane = store.workspaces.flatMap(\.tabs).flatMap(\.panes).first { $0.id == paneID }
        let now: ApprovalNotice.PaneState? = if let pane { await approvalState(pane) } else { nil }
        let outcome = ApprovalNotice.outcome(actionIdentifier: action, boundPrompt: prompt, now: now, writes: writesEnabled)
        noticeLog.info("action \(action, privacy: .public) pane \(paneID, privacy: .public) bound \(prompt ?? "none", privacy: .public) now \(now.flatMap { ApprovalNotice.prompt($0, writes: self.writesEnabled) } ?? "none", privacy: .public) -> \(String(describing: outcome), privacy: .public)")
        switch outcome {
        case .send(let allow):
            guard let pane, let a = pane.agent, let prompt, let screen = screen(for: pane) else { return false }
            // Once more right before the keys, and one answer per prompt with the card and the hub (`ApprovalSender`).
            let r = await approvals.answer(paneId: pane.id, via: screen, prompt: prompt, allow: allow, agentKind: a.kind,
                                           blocked: a.status == .blocked, pending: pendingRequest(pane))
            noticeLog.info("action \(action, privacy: .public) pane \(paneID, privacy: .public): \(String(describing: r), privacy: .public)")
            if r == .unconfirmed { notice = Self.unconfirmedAnswer; return true }
            guard r == .sent else { return false }
            promptVisible.insert(pane.id)
            Task {
                try? await Task.sleep(for: .seconds(1.5))
                await store.refresh()
                let still = store.workspaces.flatMap(\.tabs).flatMap(\.panes).first { $0.id == pane.id }?.agent?.status == .blocked
                if still { notice = "Status did not change. Open the terminal to check."; showTerminal = true }
                await checkPrompt(for: pane)
            }
            return true
        case .open:
            return false
        }
    }

    func select(workspace id: String) {
        endPeek()
        workspaceID = id
        tabID = store.workspaces.first { $0.id == id }?.tabs.first?.id
        screen = .tab
    }
    func select(tab: Tab, in ws: Workspace) {
        workspaceID = ws.id; tabID = tab.id; screen = .tab
    }
    func selectTab(index: Int) {
        guard let ws = workspace, ws.tabs.indices.contains(index) else { return }
        select(tab: ws.tabs[index], in: ws)
    }
    func cycleTab(_ delta: Int) {
        guard let ws = workspace, let t = tab, let i = ws.tabs.firstIndex(of: t), !ws.tabs.isEmpty else { return }
        select(tab: ws.tabs[(i + delta + ws.tabs.count) % ws.tabs.count], in: ws)
    }
    func cycleWorkspace(_ delta: Int) {
        let all = store.workspaces
        guard let w = workspace, let i = all.firstIndex(of: w), !all.isEmpty else { return }
        select(workspace: all[(i + delta + all.count) % all.count].id)
    }

    /// Tells the store which logs are on screen (both panes of a split, only the zoomed one when zoomed) so they
    /// load deep (2 MB) while every other log stays short. Nothing is on screen outside the tab screen; under Settings the
    /// tab screen stays (hidden), so its logs keep their depth and the older messages it shows.
    func updateVisibleSessions() {
        let tabScreen = screen == .tab || (screen == .settings && screenBeforeSettings == .tab)
        guard tabScreen, let t = tab else { store.setVisibleSessions([]); return }
        let zoomed = zoomedPaneID(in: t)
        let panes = zoomed.map { id in t.panes.filter { $0.id == id } } ?? t.panes
        store.setVisibleSessions(Set(panes.compactMap { $0.agent?.sessionId }))
    }

    /// Drops per-tab state for tabs and panes that no longer exist.
    /// Unsent composer text per pane id. Memory only. Not observed: the composer owns the live text.
    @ObservationIgnored var drafts: [String: ComposerDraft] = [:]
    func setDraft(_ d: ComposerDraft, for paneID: String) { drafts[paneID] = d.isEmpty ? nil : d }
    /// Bumped when Flockbook puts text in a pane's composer (a pulled queue): the composer starts again from `drafts`.
    var composerResets: [String: Int] = [:]
    /// The agents' own queues by pane id (`checkQueue`), and the pulls in flight.
    var queued: [String: QueuedInput] = [:]
    /// Claude Code's status line per pane (`StatusLineSetup`), while "Show Claude Code status line" is on.
    var statusLines: [String: StatusLineRecord] = [:]
    /// Panes with a pull running, from the Mac or a phone: one at a time per pane, and no send meanwhile.
    var pulling: Set<String> = []
    /// Panes with a pull or a send running (Mac or phone): one at a time per pane, for the whole of it, so a send never
    /// types into an input box a pull is filling or clearing.
    @ObservationIgnored var busy: Set<String> = []
    /// Takes the pane's lock; false: a pull or a send of it runs.
    func claim(_ id: String) -> Bool { busy.insert(id).inserted }
    func release(_ id: String) { busy.remove(id) }

    static let answerBusy = "Another answer, send or pull of this pane is on its way. Nothing was sent."
    /// Runs `body` holding the pane's lock (the one sends and pulls take), so no other picker keys land in between
    /// (on hc each key may be held a while). Held already: throws, nothing runs.
    func withPane<T>(_ id: String, _ body: () async throws -> T) async throws -> T {
        guard claim(id) else { throw QuestionAdapter.Failure.unsupported(Self.answerBusy) }
        defer { release(id) }
        return try await body()
    }

    /// A question's answer keys, for the Mac card: under the pane's lock (`withPane`).
    func answerQuestion(pane id: String, request: QuestionRequest, answers: [QuestionAnswer], gate: KeyGate? = nil) async throws {
        try await withPane(id) { try await answerKeys(pane: id, request: request, answers: answers, gate: gate) }
    }

    /// Reads the questions not read yet (moves Claude's picker: Left/Right), for the Mac card: under the pane's lock.
    func loadAllQuestions(pane id: String, gate: KeyGate? = nil, expect: QuestionRequest? = nil) async throws -> QuestionRequest? {
        try await withPane(id) { try await loadKeys(pane: id, gate: gate, expect: expect) }
    }

    /// Without the lock: only inside `withPane` (the phone holds it from the first navigation key to the last answer).
    func answerKeys(pane id: String, request: QuestionRequest, answers: [QuestionAnswer], gate: KeyGate?) async throws {
        guard let screen = screen(paneId: id) else { throw QuestionAdapter.Failure.unsupported("this pane's backend is not connected") }
        try await screen.answerQuestion(paneId: id, request: request, answers: answers, gate: gate)
    }

    func loadKeys(pane id: String, gate: KeyGate?, expect: QuestionRequest?) async throws -> QuestionRequest? {
        guard let screen = screen(paneId: id) else { throw QuestionAdapter.Failure.unsupported("this pane's backend is not connected") }
        return try await screen.loadAllQuestions(paneId: id, gate: gate, expect: expect)
    }
    /// Test hook: when set, replaces MessageSender (offscreen checks must never type into a real pane).
    @ObservationIgnored var sendHook: ((String, String) async -> Bool)?
    /// The composer's send: `text` into the pane, holding the pane's lock. Sent: the pulled texts in that composer
    /// went with it (`composerSent`).
    func deliver(_ text: String, pane id: String, kind: String?) async -> Bool {
        await deliverResult(text, pane: id, kind: kind) != .failed
    }
    /// `deliver` with hc's third outcome: `unconfirmed` (`terminal.send` timed out; the core may still deliver it, so the
    /// composer keeps the text out and says so).
    func deliverResult(_ text: String, pane id: String, kind: String?) async -> DeliveryOutcome {
        guard writesEnabled else { return .failed }
        guard claim(id) else {
            notice = "A pull of this pane is running. Your message was not sent; it is still in the composer."
            return .failed
        }
        defer { release(id) }
        noteSent(id)
        let outcome: DeliveryOutcome
        if let sendHook { outcome = await sendHook(id, text) ? .sent : .failed }
        else if HCMapper.address(fromPaneId: id) != nil { outcome = await MessageSender.deliverHC(text, to: id, hc: store.hcClient) }
        else { outcome = await MessageSender.send(text, to: id, client: store.client, kind: kind) ? .sent : .failed }
        if outcome != .failed { composerSent(pane: id) }
        return outcome
    }
    /// Pulled texts kept until a composer sent them (`PulledText`): the Mac's own, and the phones' (the hub's).
    /// The app keeps them in a file (`HerdrDeskApp`); offscreen runs in memory.
    @ObservationIgnored var pulledTexts = MemoryPulledStore()
    /// The Mac's kept texts this run put in a composer (the drafts are memory only: a quit puts them there again).
    @ObservationIgnored var delivered: Set<String> = []

    /// Kept pulled texts of panes that are gone, each judged by its own backend's snapshot (`HerdrStore.paneGone`; an
    /// empty one too: the last tab closed), never the empty start. Called on every snapshot (`HerdrStore.snapshots`), not only on a change.
    func prunePulled() {
        pulledTexts.prune(gone: store.paneGone)
    }

    func prune() {
        defer { updateVisibleSessions() }
        prunePulled()
        let tabs = store.workspaces.flatMap(\.tabs)
        guard !tabs.isEmpty else { return }
        let panes = Set(tabs.flatMap(\.panes).map(\.id))
        for (k, d) in drafts where !panes.contains(k) { d.images.forEach(PasteImages.remove) }
        drafts = drafts.filter { panes.contains($0.key) }
        queued = queued.filter { panes.contains($0.key) }
        statusLines = statusLines.filter { panes.contains($0.key) }
        rightClickToPane = rightClickToPane.filter { panes.contains($0) }
        let live = Set(tabs.map(\.id))
        focusedPane = focusedPane.filter { live.contains($0.key) }
        zoomedPane = zoomedPane.filter { live.contains($0.key) }
        if let r = renamingTabID, !live.contains(r) { renamingTabID = nil }
        if let w = workspaceID, !store.workspaces.contains(where: { $0.id == w }) { workspaceID = store.workspaces.first?.id }
        if let t = tabID, !live.contains(t) { tabID = workspace?.tabs.first?.id }
    }

    func focus(_ pane: Pane, in tab: Tab) { focusedPane[tab.id] = pane.id }
    func focusedPaneID(in tab: Tab) -> String? {
        if let id = focusedPane[tab.id], tab.panes.contains(where: { $0.id == id }) { return id }
        return tab.panes.first?.id
    }
    /// The zoomed pane of a tab, if it still exists and the tab is split.
    func zoomedPaneID(in tab: Tab) -> String? {
        guard tab.isSplit, let id = zoomedPane[tab.id], tab.panes.contains(where: { $0.id == id }) else { return nil }
        return id
    }
    func toggleZoom(_ pane: Pane, in tab: Tab) {
        guard tab.isSplit else { return }
        if zoomedPane[tab.id] == pane.id { zoomedPane[tab.id] = nil } else { zoomedPane[tab.id] = pane.id; focusedPane[tab.id] = pane.id }
    }
    func toggleZoomFocused() {
        guard let tab, let id = focusedPaneID(in: tab), let pane = tab.panes.first(where: { $0.id == id }) else { return }
        toggleZoom(pane, in: tab)
    }

    /// Show-terminal sheet size (remembered across launches) and the main window's content size (for its maximum).
    var terminalSize = TerminalSheetSize.load()
    var windowSize: CGSize = .zero
    /// The Show-terminal sheet's window, and whether an edge drag is in progress (the model-to-window sync stands aside then).
    weak var sheetWindow: NSWindow?
    /// The main window (WindowWidthTracker.attach), for AppKit actions on its split view.
    weak var mainWindow: NSWindow?
    var terminalDragging = false
    /// Frame the sheet window must have during a drag (SwiftUI re-centres the sheet when its content size changes; the sizer puts it back).
    var sheetTargetFrame: NSRect?
    /// Full-screen state and the traffic lights' frames (close, minimize, zoom; window content space, top-left origin), kept by
    /// WindowChrome / the window observers. Measured windowed only (in full screen they live in the system's toolbar reveal).
    var fullScreen = false
    var lights: [CGRect] = []
    /// Where the system sidebar toggle sits windowed (window content space), for FullScreenChrome's own toggle: with the
    /// sidebar shown, at the panel's trailing edge (`trailing` = panel maxX - toggle maxX); hidden, beside the lights (`frame`,
    /// where the system's shows in the revealed full-screen band; ours sits at the left edge there, FullScreenChrome.restSpot).
    var toggleShown = ToggleSpot(trailing: 4, frame: CGRect(x: 215.5, y: 11.5, width: 38.5, height: 28.5))
    var toggleHidden = ToggleSpot(trailing: 0, frame: CGRect(x: 92, y: 11.5, width: 38.5, height: 28.5))
    struct ToggleSpot: Equatable { var trailing: CGFloat; var frame: CGRect }
    /// The docked sidebar panel's trailing edge (window space), 0 while hidden.
    var sidebarMaxX: CGFloat = 0
    /// Height of the windowed toolbar (the detail column's top safe area). Full screen keeps a band this high at the top
    /// (the system toolbar is hidden there), so sidebar rows, the tab island and content sit where they sit windowed.
    var toolbarBand: CGFloat = 52
    /// The system moved the toolbar into its full-screen window (real full screen): the content lost the toolbar's inset.
    var toolbarDetached = false
    /// How far the system's full-screen toolbar (revealed with the menu bar) reaches into the window from the top; 0 at rest.
    var toolbarReveal: CGFloat = 0
    /// While the system's toolbar is revealed, its own lights and toggle show: ours step aside.
    var systemChromeRevealed: Bool { fullScreen && toolbarReveal > 0.5 }
    /// How far the window content moves down while revealed: only what the reveal covers beyond the band (the menu bar, on
    /// a screen without a notch; on a notched screen the menu bar sits beside the notch, above the window, and this is 0).
    var revealShift: CGFloat { fullScreen ? max(0, toolbarReveal - toolbarBand) : 0 }
    /// Where the system toolbar lays out the tab island and its two buttons (top-left space of the window holding the
    /// toolbar: the window itself, or the full-screen toolbar window, whose top meets the window top when revealed). The
    /// full-screen content copy sits exactly there, so the revealed toolbar copy lands on it. Zero until measured.
    var toolbarIsland: CGRect = .zero
    var toolbarActions: CGRect = .zero
    /// Which copy of the tab row the pointer and the keyboard use: the toolbar's windowed and while revealed in full screen,
    /// the content copy (FullScreenTopRow) at rest in full screen.
    func islandActive(_ host: IslandHost) -> Bool { (host == .toolbar) == (!fullScreen || systemChromeRevealed) }
    /// Text of the rename in progress, shared by both copies so it survives a hand-over between them.
    var renameDraft = ""
    var renameDraftTab: String?
    var lightsMaxX: CGFloat { lights.last?.maxX ?? 0 }
    var lightsMidY: CGFloat { lights.last?.midY ?? 0 }

    /// Pointer entered or left the edge strip / the peeking panel (called from `onHover`, which AppKit drives with tracking areas).
    /// Show after resting 150 ms on the strip; hide 300 ms after the pointer left both.
    func setPeekHover(strip: Bool? = nil, panel: Bool? = nil) {
        if let strip { peekStrip = strip }
        if let panel { peekPanel = panel }
        peekTask?.cancel()
        guard sidebarVisibility == .detailOnly else { return }
        if !sidebarPeek, peekStrip {
            peekTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(150))
                guard !Task.isCancelled, let self, self.peekStrip, self.sidebarVisibility == .detailOnly else { return }
                self.sidebarPeek = true
            }
        } else if sidebarPeek, !peekStrip, !peekPanel {
            peekTask = Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(300))
                guard !Task.isCancelled, let self, !self.peekStrip, !self.peekPanel else { return }
                self.sidebarPeek = false
            }
        }
    }
    func endPeek() { peekTask?.cancel(); sidebarPeek = false; peekStrip = false; peekPanel = false }

    /// Show or hide the sidebar as the system's sidebar button does: AppKit's own slide of the split view (columnVisibility
    /// follows). Not a SwiftUI animation of the binding: that animates the split's size while AppKit keeps the detail
    /// column at full width, so the window content grows by the sidebar's width, is centred (shifting left by up to half
    /// of it) and jumps into place at the end (--test-root-width traces it).
    @ObservationIgnored private var sidebarToggleGeneration = 0
    /// Programmatic visibility requests supersede an unfinished native toggle; SwiftUI's binding updates do not.
    func setSidebarVisibility(_ visibility: NavigationSplitViewVisibility) {
        sidebarToggleGeneration += 1
        sidebarVisibility = visibility
    }
    func toggleSidebar() {
        endPeek()
        if let w = mainWindow, let c = WindowWidthTracker.splitView(in: w.contentView)?.delegate as? NSSplitViewController {
            sidebarToggleGeneration += 1
            let generation = sidebarToggleGeneration
            NSAnimationContext.runAnimationGroup { _ in
                c.toggleSidebar(nil)
            } completionHandler: { [weak self] in
                Task { @MainActor in
                    guard let self, self.sidebarToggleGeneration == generation,
                          let window = self.mainWindow,
                          let controller = WindowWidthTracker.splitView(in: window.contentView)?.delegate as? NSSplitViewController,
                          let sidebar = controller.splitViewItems.first(where: { $0.behavior == .sidebar }) else { return }
                    // SwiftUI can report the previous collapsed state after a native toggle. Read it after the slide,
                    // without changing the binding during the animation or accepting an older toggle's completion.
                    self.sidebarVisibility = sidebar.isCollapsed ? .detailOnly : .all
                }
            }
        } else {
            sidebarVisibility = sidebarVisibility == .detailOnly ? .all : .detailOnly
        }
    }
}

/// Size of the Show terminal sheet: 600x360 up to the main window minus a margin; remembered in UserDefaults.
enum TerminalSheetSize {
    static let min = CGSize(width: 600, height: 360)
    static let standard = CGSize(width: 880, height: 560)
    static let key = "terminalSheetSize"
    static func load() -> CGSize {
        guard let a = AppDefaults.store.array(forKey: key) as? [Double], a.count == 2 else { return standard }
        return CGSize(width: max(min.width, a[0]), height: max(min.height, a[1]))
    }
    static func save(_ s: CGSize) { AppDefaults.store.set([Double(s.width), Double(s.height)], forKey: key) }
    /// Clamp to [min, window content size minus margin]; an unknown window (zero) leaves the maximum open.
    static func clamp(_ s: CGSize, window: CGSize) -> CGSize {
        let maxW = window.width > 0 ? Swift.max(min.width, window.width - 40) : .infinity
        let maxH = window.height > 0 ? Swift.max(min.height, window.height - 60) : .infinity
        return CGSize(width: Swift.min(Swift.max(s.width, min.width), maxW), height: Swift.min(Swift.max(s.height, min.height), maxH))
    }
}

/// The app's settings store. The real app uses the standard defaults. Every headless or test run gets its own suite
/// (`herdr-desk.test.<pid>`, removed at exit), so checks never change the owner's settings and concurrent runs from
/// several agents never change each other's (the cause of flaky allowInput-dependent checks).
enum AppDefaults {
    nonisolated(unsafe) private(set) static var store: UserDefaults = .standard
    nonisolated(unsafe) private static var suite: String?
    nonisolated(unsafe) private static var signals: [DispatchSourceSignal] = []

    static var isTest: Bool { suite != nil }

    /// The app's bundle id before the rename to Flockbook (dev.mclee.flockbook). Its settings domain is read, never written.
    static let oldDomain = "dev.mclee.herdr-desk"
    static let migratedKey = "migratedFromHerdrDesk"

    /// First launch under the new bundle id: copy the old id's settings (window and split frames included) into the
    /// standard defaults once. Not for test runs (their own suite) or a run without a bundle id. Keys already in the new
    /// domain stay: AppKit writes split-view frames to the standard defaults even in test runs, so "empty" is not a test.
    static func migrateOldSettings() {
        guard !isTest, let id = Bundle.main.bundleIdentifier, id != oldDomain else { return }
        migrate(from: UserDefaults.standard.persistentDomain(forName: oldDomain), into: .standard, domain: id)
    }

    /// Once (until the flag is set): copies the keys of `old` that `d`'s own domain lacks; sets the flag. True when it
    /// copied any key.
    @discardableResult
    static func migrate(from old: [String: Any]?, into d: UserDefaults, domain: String) -> Bool {
        let current = d.persistentDomain(forName: domain) ?? [:]
        guard current[migratedKey] == nil else { return false }
        defer { d.set(true, forKey: migratedKey) }
        let missing = (old ?? [:]).filter { current[$0.key] == nil }
        for (k, v) in missing { d.set(v, forKey: k) }
        return !missing.isEmpty
    }

    static func isolateForTests() {
        let name = "herdr-desk.test.\(getpid())"
        guard suite == nil, let d = UserDefaults(suiteName: name) else { return }
        suite = name; store = d
        // No window restoration for this process: nothing saved at quit, nothing restored (argument domain: this process only).
        var args = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
        args["NSQuitAlwaysKeepsWindows"] = false
        args["ApplePersistenceIgnoreState"] = true
        UserDefaults.standard.setVolatileDomain(args, forName: UserDefaults.argumentDomain)
        sweepStale()
        atexit { AppDefaults.removeSuite() }
        // A check killed by its timeout (SIGTERM, SIGINT) still removes its suite.
        for sig in [SIGTERM, SIGINT] {
            signal(sig, SIG_IGN)
            let src = DispatchSource.makeSignalSource(signal: sig, queue: .main)
            src.setEventHandler { exit(128 + sig) }
            src.resume(); signals.append(src)
        }
    }
    private static func removeSuite() {
        guard let suite else { return }
        store.removePersistentDomain(forName: suite)
        store.synchronize()
        try? FileManager.default.removeItem(at: plist(suite))   // cfprefsd leaves the emptied file
    }
    private static func plist(_ name: String) -> URL {
        FileManager.default.homeDirectoryForCurrentUser.appending(path: "Library/Preferences/\(name).plist")
    }
    /// Suites of runs that died without cleanup (SIGKILL): the pid in the name no longer exists.
    private static func sweepStale() {
        let dir = plist("x").deletingLastPathComponent()
        for f in (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? [] where f.hasPrefix("herdr-desk.test.") && f.hasSuffix(".plist") {
            guard let pid = Int32(f.dropFirst("herdr-desk.test.".count).dropLast(".plist".count)), pid != getpid(),
                  kill(pid, 0) != 0, errno == ESRCH else { continue }
            UserDefaults(suiteName: String(f.dropLast(".plist".count)))?.removePersistentDomain(forName: String(f.dropLast(".plist".count)))
            try? FileManager.default.removeItem(at: dir.appending(path: f))
        }
    }
}
