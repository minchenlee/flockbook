import Foundation
import Observation

@MainActor @Observable
public final class HerdrStore {
    public let session: HerdrSession
    public let client: HerdrClient
    public private(set) var workspaces: [Workspace] = []
    /// True while at least one backend answers.
    public private(set) var connected = false
    public private(set) var tuiConnected = false
    public private(set) var hcConnected = false
    /// herdr-core client; nil when the store was made without an hc session (tests, previews).
    public let hcClient: HCClient?
    /// The panes there are now, from the last snapshot herdr gave (none: the last tab closed). Nil: no snapshot yet.
    public var knownPanes: Set<String>? {
        snapshotted ? Set(workspaces.flatMap(\.tabs).flatMap(\.panes).map(\.id)) : nil
    }
    /// Whether the pane is gone, judged only by its own backend: a TUI pane after a herdr snapshot without it, an hc
    /// pane after an hc snapshot without it. One backend's snapshot never stands for the other's (a herdr server that
    /// is down or slower than hc must not make its panes look gone).
    public func paneGone(_ id: String) -> Bool {
        guard HCMapper.address(fromPaneId: id) != nil ? hcSnapshotted : snapshotted else { return false }
        return !workspaces.lazy.flatMap(\.tabs).flatMap(\.panes).contains { $0.id == id }
    }
    /// A herdr (TUI) snapshot was applied (`workspaces` is herdr's, not the empty start).
    @ObservationIgnored private var snapshotted = false
    /// An hc snapshot was applied.
    @ObservationIgnored private var hcSnapshotted = false
    /// Why the last snapshot failed, with the next step (`ConnectionCheck`). Nil while connected, or when the socket
    /// accepts but the request fails.
    public var problem: ConnectionProblem? { tuiProblem ?? hcProblem }
    /// The TUI's problem (`ConnectionCheck`), and hc's (`hcNotRunning`) when the plan reports it.
    private var tuiProblem: ConnectionProblem?
    private var hcProblem: ConnectionProblem?
    /// Connected, but no `herdr` program where the app looks (`ConnectionCheck.programWarning`): terminals cannot start.
    /// Looked up once, on the first snapshot.
    public private(set) var programWarning: String?
    @ObservationIgnored var isPreview = false
    @ObservationIgnored var fuzzState: UInt64 = 0
    @ObservationIgnored var fuzzCounter = 0

    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private var refreshTask: Task<Void, Never>?
    @ObservationIgnored private var logs: [String: SessionLog] = [:]
    private var logItems: [String: SessionLog.Snapshot] = [:]
    @ObservationIgnored private var logTasks: [String: Task<Void, Never>] = [:]
    /// Tailed sessions, most recently read first. Beyond `maxLiveLogs` the oldest idle ones are released.
    @ObservationIgnored private var recentLogs: [String] = []
    /// Upper bound of tailed logs. Blocked agents are never released (the inbox reads their permission request).
    public var maxLiveLogs = 8
    /// Sessions whose pane is on screen; they keep the full 2 MB tail, every other log keeps about 256 KB.
    /// The app may set it. While it is nil, the session read last counts as visible.
    @ObservationIgnored private var visibleOverride: Set<String>?
    @ObservationIgnored private var reliefPending = false

    public func setVisibleSessions(_ ids: Set<String>?) { visibleOverride = ids; applyDepth() }
    /// What `setVisibleSessions` set last (nil: never set).
    public var visibleSessions: Set<String>? { visibleOverride }

    /// Without `setVisibleSessions`, a log read in the last 2 s counts as visible. (Only "the last one read" made two
    /// panes read in turn swap full/short on every call, and each swap re-parsed 2 MB.)
    @ObservationIgnored private var lastRead: [String: Date] = [:]
    @ObservationIgnored private var demotePending = false
    private func applyDepth() {
        let now = Date()
        let vis = visibleOverride ?? Set(lastRead.filter { now.timeIntervalSince($0.value) < 2 }.keys)
        for (id, log) in logs { log.setFull(vis.contains(id)) }
        scheduleRelief()
        if visibleOverride == nil, !vis.isEmpty, !demotePending {
            demotePending = true
            Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(2100))
                self?.demotePending = false
                if self?.visibleOverride == nil { self?.applyDepth() }
            }
        }
    }

    /// Hands freed pages back to the system once, shortly after logs were released or trimmed.
    private func scheduleRelief() {
        guard !reliefPending else { return }
        reliefPending = true
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(1))
            self?.reliefPending = false
            malloc_zone_pressure_relief(nil, 0)
        }
    }

    /// `hc`: also follow herdr-core at that socket (missing socket = normal, retried). Nil: TUI only.
    public init(session: HerdrSession, hc: HCSession? = nil, plan: BackendPlan = .both) {
        self.session = session
        var plan = plan
        if hc == nil { plan.hc = false }   // nothing to follow
        if !plan.tui && !plan.hc { plan.tui = true }
        self.plan = plan
        self.client = HerdrClient(session: session)
        self.hcClient = hc.map { HCClient(session: $0) }
        self.bridge = RemoteBridge(session: session)
        self.resolver = SessionResolver(socketPath: session.socketPath, remote: session.remote)
    }

    /// Which loops run and when a silent backend is reported (Settings > General > Backend).
    public private(set) var plan: BackendPlan
    /// TUI diagnoses run so far, and TUI snapshot tries (checks).
    @ObservationIgnored var diagnoses = 0
    @ObservationIgnored var tuiTries = 0

    /// Follows a new plan without a relaunch. False: it needs hc and this store has no hc client (relaunch).
    @discardableResult
    public func use(_ next: BackendPlan) -> Bool {
        guard !next.hc || hcClient != nil else { return false }
        guard next != plan else { return true }
        let running = loop != nil || hcLoop != nil
        stop()
        plan = next
        // The side that no longer runs leaves at once, with its problem.
        if !next.tui { lastRaw = []; snapshotted = false; tuiProblem = nil }
        if !next.hc { lastHCRaw = []; hcSnapshotted = false; hcProblem = nil }
        compose()
        if running { start() }
        return true
    }

    /// Layout writes for that backend (nil: hc not configured).
    public func layout(_ kind: BackendKind) -> (any LayoutBackend)? { kind == .hc ? hcClient : client }

    /// The backend that owns this pane id (`hc:` prefix = herdr-core).
    public func backend(forPane paneId: String) -> (any HerdrBackend)? {
        if HCMapper.address(fromPaneId: paneId) != nil { return hcClient }
        return client
    }

    private func updateConnected() {
        let c = tuiConnected || hcConnected
        if c != connected { connected = c }
    }

    /// An hc pane of a terminal on another machine (`hc:box/t7`).
    public static func isRemotePane(_ paneId: String) -> Bool {
        HCMapper.address(fromPaneId: paneId).map { !HCMapper.machinePrefix($0).isEmpty } ?? false
    }

    // MARK: - Log times

    /// Session id → when that agent last wrote its log (`LogTimeCache`). Filled by `refreshLogTimes`.
    public private(set) var logTimes: [String: Date] = [:]
    @ObservationIgnored private let logTimeCache = LogTimeCache()
    @ObservationIgnored private var logTimesRunning = false

    /// Looks at the logs of the agents shown now, off the main thread; each log at most once per 30 s. The paths are
    /// the sessions herdr names or `SessionResolver` found (`agent.sessionId`).
    public func refreshLogTimes(now: Date = Date()) async {
        guard !isPreview, !logTimesRunning else { return }
        let sessions = workspaces.flatMap(\.tabs).flatMap(\.panes).compactMap { p in p.agent.flatMap { a in a.sessionId.map { ($0, a.kind) } } }
        logTimesRunning = true
        let cache = logTimeCache
        let times = await Task.detached { cache.refresh(sessions, now: now); return cache.times }.value
        logTimesRunning = false
        if times != logTimes { logTimes = times }
    }

    /// When the tab's agents last wrote their logs (the newest); nil when none has a log found (a shell tab).
    public func lastActivity(of tab: Tab, now: Date = Date()) -> Date? {
        if isPreview { return PreviewData.logAges[tab.id].map { now - $0 } }
        return LogTimeCache.newest(of: tab, times: logTimes)
    }

    // MARK: - Session fallback

    /// Finds logs for agent panes that herdr reports without a session (read-only; see `SessionResolver`).
    @ObservationIgnored private let resolver: SessionResolver
    /// paneId → log path found by the resolver.
    @ObservationIgnored private var resolved: [String: String] = [:]
    @ObservationIgnored private var resolveTask: Task<Void, Never>?

    /// paneId → the log to show in place of a teammate's or undecided log that herdr names (see `SessionResolver.leads`).
    @ObservationIgnored private var swaps: [String: LeadSwap] = [:]
    /// Claude sessions the resolver judged to be a lead's log. Other Claude sessions are held back until judged.
    @ObservationIgnored private var leads: Set<String> = []

    /// Fills `agent.sessionId` from `resolved` where herdr gave none, and puts the lead's log (or none) in place of
    /// a teammate's or undecided log while herdr still names it. Either way the session is not herdr's
    /// (`sessionResolved`), and the pane says which (`team`). With `leads`, a Claude session that is in neither `leads`
    /// nor `swaps` is not judged yet and shows no log, with no note (judging takes one pass); nil `leads` (no resolver,
    /// as in previews) holds nothing back.
    static func withFallbacks(_ ws: [Workspace], resolved: [String: String], swaps: [String: LeadSwap], leads: Set<String>?) -> [Workspace] {
        guard !resolved.isEmpty || !swaps.isEmpty || leads != nil else { return ws }
        var ws = ws
        for w in ws.indices { for t in ws[w].tabs.indices { for p in ws[w].tabs[t].panes.indices {
            let pane = ws[w].tabs[t].panes[p]
            // The log to show in place of `sid` and the pane's note, or nil to show `sid`.
            func instead(_ sid: String) -> (String?, TeamNote?)? {
                if let s = swaps[pane.id], s.teammate == sid { return (s.lead, s.lead == nil ? .waitingForLead : .lastLead) }
                if let leads, pane.agent?.kind == "claude", !leads.contains(sid) { return (nil, nil) }
                return nil
            }
            guard let sid = pane.agent?.sessionId else {
                if pane.agent != nil, let path = resolved[pane.id] {
                    let shown = instead(path)
                    ws[w].tabs[t].panes[p].agent?.sessionId = shown.map(\.0) ?? path
                    ws[w].tabs[t].panes[p].agent?.team = shown?.1
                    ws[w].tabs[t].panes[p].agent?.sessionResolved = true
                }
                continue
            }
            if let shown = instead(sid) {
                ws[w].tabs[t].panes[p].agent?.sessionId = shown.0
                ws[w].tabs[t].panes[p].agent?.team = shown.1
                ws[w].tabs[t].panes[p].agent?.sessionResolved = true
            }
        } } }
        return ws
    }

    /// The logs panes already show: herdr's sessions and the logs the resolver found.
    static func claims(_ ws: [Workspace], resolved: [String: String]) -> Set<String> {
        Set(ws.flatMap(\.tabs).flatMap(\.panes).compactMap { $0.agent?.sessionId }).union(resolved.values)
    }

    /// The Claude panes to judge: herdr's session, else the log the resolver found.
    static func leadAsks(_ raw: [Workspace], found: [String: String]) -> [SessionResolver.LeadAsk] {
        raw.flatMap(\.tabs).flatMap(\.panes).compactMap { p in
            guard p.agent?.kind == "claude", let sid = p.agent?.sessionId ?? found[p.id] else { return nil }
            return SessionResolver.LeadAsk(paneId: p.id, sessionId: sid)
        }
    }

    /// Starts one resolver pass for the panes that still have no session, and for Claude panes whose session may be
    /// a teammate's log. The snapshot never waits for it.
    private func resolveMissing(_ raw: [Workspace]) {
        guard !isPreview, resolveTask == nil else { return }
        let panes = raw.flatMap(\.tabs).flatMap(\.panes)
        let needs = panes.compactMap { p in
            p.agent.flatMap { $0.sessionId == nil ? SessionResolver.Need(paneId: p.id, kind: $0.kind, cwd: p.cwd, pids: p.processIds,
                                                              process: $0.process) : nil }
        }
        if needs.isEmpty, resolved.isEmpty, Self.leadAsks(raw, found: [:]).isEmpty, swaps.isEmpty, leads.isEmpty { return }
        let claimed = Set(panes.compactMap { $0.agent?.sessionId })
        // The claimant's cwd per session; two claimants in different folders: unknown (the claim always counts).
        var claimedCwd: [String: String] = [:], mixed = Set<String>()
        for p in panes { if let sid = p.agent?.sessionId { if let c = claimedCwd[sid], c != p.cwd { mixed.insert(sid) }; claimedCwd[sid] = p.cwd } }
        for sid in mixed { claimedCwd[sid] = nil }
        // Agents on another machine (`hc:box/t7`): found there and mirrored (`HCClient.resolveRemote`); asked every
        // pass while hc is connected, so mirrors of panes gone stop.
        let remote = needs.filter { Self.isRemotePane($0.paneId) }
        let local = needs.filter { !Self.isRemotePane($0.paneId) }
        resolveTask = Task { [weak self, resolver, hcClient] in
            var found = await resolver.resolve(local, claimed: claimed, claimedCwd: claimedCwd)
            if let hcClient { found.merge(await hcClient.resolveRemote(remote, claimed: claimed)) { a, _ in a } }
            let pass = await resolver.leads(Self.leadAsks(raw, found: found), claimed: claimed.union(found.values))
            guard let self else { return }
            self.resolveTask = nil
            if found != self.resolved || pass.swaps != self.swaps || pass.leads != self.leads {
                self.resolved = found
                self.swaps = pass.swaps
                self.leads = pass.leads
                self.compose()
            }
            // Sessions herdr named while this pass ran are judged by the next one.
            let judged = Set(pass.swaps.values.map(\.teammate)).union(pass.leads)
            let all = self.lastRaw + self.lastHCRaw
            let named = all.flatMap(\.tabs).flatMap(\.panes).compactMap { $0.agent?.kind == "claude" ? $0.agent?.sessionId : nil }
            if named.contains(where: { !judged.contains($0) }) { self.resolveMissing(all) }
        }
    }

    /// The last snapshots as each backend sent them, before `withFallbacks`. TUI workspaces come first.
    @ObservationIgnored private var lastRaw: [Workspace] = []
    @ObservationIgnored private var lastHCRaw: [Workspace] = []

    private func applySnapshot(_ raw: [Workspace]) {
        lastRaw = raw
        compose()
    }

    private func compose() {
        let ws = Self.withFallbacks(lastRaw + lastHCRaw, resolved: resolved, swaps: swaps, leads: isPreview ? nil : leads)
        if ws != workspaces { workspaces = ws }
        syncLogs()
    }

    // MARK: - Remote host

    /// The local relay socket for a session on another host (nil: herdr on this Mac). Runs from `start` to `stop`.
    @ObservationIgnored private let bridge: RemoteBridge?
    /// A remote diagnosis runs a command on the host: one at a time.
    @ObservationIgnored private var diagnosing = false

    public func start() {
        guard loop == nil, hcLoop == nil else { return }
        if plan.tui, let bridge, !isPreview {
            do { try bridge.start() } catch {
                if let host = session.remote { tuiProblem = .remote(.other("cannot open the relay socket: \(error)"), host: host, session: session.name) }
            }
        }
        if plan.tui { loop = Task { [weak self] in await self?.run() } }
        if plan.hc, let hc = hcClient { hcLoop = Task { [weak self] in await self?.runHC(hc) } }
    }
    @ObservationIgnored private var hcLoop: Task<Void, Never>?
    /// hc answered or failed at least once since `start` (Automatic diagnoses a silent TUI only after that).
    @ObservationIgnored private var hcTried = false

    private func restartTUI() {
        loop?.cancel()
        loop = Task { [weak self] in await self?.run() }
    }

    private func restartHC() {
        guard let hc = hcClient else { return }
        hcLoop?.cancel()
        hcLoop = Task { [weak self] in await self?.runHC(hc) }
    }

    public func stop() {
        bridge?.stop()
        loop?.cancel(); loop = nil
        hcLoop?.cancel(); hcLoop = nil
        hcTried = false
        refreshTask?.cancel(); hcRefreshTask?.cancel()
        for t in logTasks.values { t.cancel() }
        logTasks = [:]
        connected = false; tuiConnected = false; hcConnected = false
    }

    /// Successful snapshots taken so far.
    public private(set) var snapshots = 0
    /// A snapshot herdr gave.
    func took(_ raw: [Workspace]) {
        tuiConnected = true
        updateConnected()
        if !snapshotted, !isPreview {
            Task { [weak self] in
                let session = self?.session ?? .default()
                let w = await Task.detached { ConnectionCheck.programWarning(session: session) }.value
                self?.programWarning = w
            }
        }
        snapshotted = true
        snapshots += 1
        if tuiProblem != nil { tuiProblem = nil }
        applySnapshot(raw)
    }

    /// Fresh snapshots from every backend.
    public func refresh() async {
        if plan.tui { await refreshTUI() }
        if plan.hc, hcClient != nil { await refreshHC() }
    }

    func refreshTUI() async {
        guard plan.tui else { return }
        tuiTries += 1
        do {
            let raw = try await client.snapshot()
            took(raw)
            resolveMissing(lastRaw + lastHCRaw)
        } catch {
            tuiConnected = false
            updateConnected()
            // Automatic, hc answers: a silent TUI shows nothing and is not diagnosed. Nor before hc's first try: it may
            // answer (at launch the TUI loop is first).
            guard plan.tuiDown(hcConnected: hcConnected) == .report, !(plan.quiet && plan.hc && !hcTried) else {
                if tuiProblem != nil { tuiProblem = nil }
                return
            }
            let session = session
            let found: ConnectionProblem?
            diagnoses += 1
            if session.remote != nil {
                guard !diagnosing else { return }
                diagnosing = true
                found = await ConnectionCheck.diagnose(remote: session, bridge: bridge)
                diagnosing = false
            } else {
                found = await Task.detached { ConnectionCheck.diagnose(session: session) }.value
            }
            if found != tuiProblem, !tuiConnected, plan.tuiDown(hcConnected: hcConnected) == .report { tuiProblem = found }
        }
    }

    func refreshHC() async {
        guard plan.hc, let hc = hcClient else { return }
        do {
            let raw = try await hc.snapshot()
            hcConnected = true
            hcTried = true
            hcDownSince = nil
            if hcProblem != nil { hcProblem = nil }
            // hc answers again: a TUI problem kept from before (Automatic) goes.
            if plan.tuiDown(hcConnected: true) == .hide, tuiProblem != nil { tuiProblem = nil }
            hcSnapshotted = true
            lastHCRaw = raw
            compose()
            resolveMissing(lastRaw + lastHCRaw)
        } catch {
            hcDown()
        }
        updateConnected()
    }

    /// How long an unreachable hc's panes stay (a reconnect within it keeps them); then they leave.
    public var hcStaleAfter: Duration = .seconds(10)
    @ObservationIgnored private var hcDownSince: ContinuousClock.Instant?
    /// hc did not answer: not connected; past `hcStaleAfter` its panes leave the store.
    private func hcDown() {
        hcTried = true
        let was = hcConnected
        hcConnected = false
        let p: ConnectionProblem? = plan.showsHCProblem && plan.hcDown(tuiConnected: tuiConnected) == .report
            ? .hcNotRunning(socket: hcClient?.session.socketPath ?? "") : nil
        if p != hcProblem { hcProblem = p }
        // A TUI hidden while hc answered sleeps up to 30 s: now it is the only one left, try it at once.
        if was, plan.tui, !tuiConnected, loop != nil { restartTUI() }
        let since = hcDownSince ?? .now
        hcDownSince = since
        if ContinuousClock.now - since >= hcStaleAfter, !lastHCRaw.isEmpty { lastHCRaw = []; compose() }
    }

    /// Latest parsed conversation for a Claude Code session id. Starts tailing on first call.
    public func conversation(for sessionId: String?) -> [ConversationItem] {
        if isPreview { return PreviewData.conversation }
        guard let sessionId else { return [] }
        track(sessionId)
        touch(sessionId)
        guard let snap = logItems[sessionId] else { return [] }
        return snap.items
    }

    /// The log file `conversation(for:)` reads (picture refs point into it); nil before it is found.
    public func logURL(for sessionId: String?) -> URL? {
        guard !isPreview, let sessionId else { return nil }
        return logs[sessionId]?.url
    }

    /// Whether `conversation(for:)` is still loading, ready (possibly empty), has no log file, or failed.
    /// Starts tailing on first call, like `conversation(for:)`. A nil session id is `.missing`.
    public func conversationState(for sessionId: String?) -> ConversationLoadState {
        if isPreview { return .ready }
        guard let sessionId else { return .missing }
        track(sessionId)
        touch(sessionId)
        return logItems[sessionId]?.load ?? .loading
    }

    /// Claude Code's queue as that session's log has it (`ClaudeQueueLog`). Read with the screen (`QueuedInput.read`).
    public func queuedInLog(for sessionId: String?) -> ClaudeQueueLog {
        guard !isPreview, let sessionId else { return [] }
        return logItems[sessionId]?.queued ?? []
    }

    /// Loads an older segment, distinguishing the start of the log from a failed read. `conversation(for:)` grows within one poll.
    @discardableResult
    public func loadEarlier(for sessionId: String?, bytes: Int = 2_000_000) async -> EarlierLoadResult {
        guard !isPreview, let sessionId, let log = logs[sessionId] else { return .end }
        return await log.loadEarlier(bytes: bytes)
    }

    /// Same as `loadEarlier(for:)`, addressed by pane. End when the pane is unknown or has no agent session.
    @discardableResult
    public func loadEarlier(paneId: String, bytes: Int = 2_000_000) async -> EarlierLoadResult {
        await loadEarlier(for: Self.sessionId(forPane: paneId, in: workspaces), bytes: bytes)
    }

    public static func sessionId(forPane paneId: String, in workspaces: [Workspace]) -> String? {
        workspaces.lazy.flatMap(\.tabs).flatMap(\.panes).first { $0.id == paneId }?.agent?.sessionId
    }

    func setPreview(workspaces: [Workspace], connected: Bool) {
        self.workspaces = workspaces
        snapshotted = true
        self.connected = connected
        self.isPreview = true
    }

    // MARK: - Live loop

    /// Reconnect: first retry after 100 ms, then doubling up to 1 s. A local Unix socket connect costs microseconds,
    /// so a short cap keeps the gap after a herdr restart under about a second. A remote host caps at 5 s: each try
    /// starts an ssh. While the plan hides a silent TUI (Automatic, hc answers): from 2 s, backing off to 30 s.
    private func run() async {
        var delay: Duration = .milliseconds(100)
        let cap: Duration = session.remote == nil ? .seconds(1) : .seconds(5)
        while !Task.isCancelled {
            // The relay socket could not open in `start` (e.g. the runtime folder): try again before each connect.
            if let bridge, !isPreview, !bridge.isRunning { try? bridge.start() }
            await refreshTUI()
            let poll = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    await self?.refreshTUI()
                }
            }
            defer { poll.cancel() }
            do {
                for try await _ in client.events() {
                    delay = .milliseconds(100)
                    scheduleRefresh()
                }
            } catch {}
            let was = tuiConnected
            tuiConnected = false
            updateConnected()
            // An hc hidden while the TUI answered sleeps up to 30 s: now it is the only one left, try it at once.
            if was, plan.hc, !hcConnected, hcLoop != nil { restartHC() }
            delay = await backOff(delay, hidden: plan.tuiDown(hcConnected: hcConnected) == .hide, cap: cap)
        }
    }

    /// Sleeps before the next connect and returns the wait after that one (`BackendPlan.retry`). Hidden: at least
    /// 2 s, backing off to 30 s; otherwise `delay` (at most `cap`), then doubled.
    private func backOff(_ delay: Duration, hidden: Bool, cap: Duration) async -> Duration {
        if hidden {
            let wait = BackendPlan.retry(after: delay, hidden: true)
            try? await Task.sleep(for: wait)
            return wait
        }
        let wait = min(delay, cap)
        try? await Task.sleep(for: wait)
        return BackendPlan.retry(after: wait, hidden: false, cap: cap)
    }

    /// Same loop for herdr-core. No socket file is normal (hc not installed or not running): check again every 2 s
    /// (backing off to 30 s while the plan hides a silent hc).
    private func runHC(_ hc: HCClient) async {
        var delay: Duration = .milliseconds(100)
        var missing: Duration = .seconds(1)
        while !Task.isCancelled {
            guard hc.session.isAvailable else {
                hcDown(); updateConnected()
                let hidden = plan.hcDown(tuiConnected: tuiConnected) == .hide
                missing = hidden ? BackendPlan.retry(after: missing, hidden: true) : .seconds(2)
                try? await Task.sleep(for: missing)
                continue
            }
            missing = .seconds(1)
            await refreshHC()
            let poll = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(nanoseconds: 2_000_000_000)
                    await self?.refreshHC()
                }
            }
            defer { poll.cancel() }
            do {
                for try await _ in hc.events() {
                    delay = .milliseconds(100)
                    scheduleHCRefresh()
                }
            } catch {}
            poll.cancel()
            hcDown()
            updateConnected()
            delay = await backOff(delay, hidden: plan.hcDown(tuiConnected: tuiConnected) == .hide, cap: .seconds(1))
        }
    }

    @ObservationIgnored private var hcRefreshTask: Task<Void, Never>?
    @ObservationIgnored private var hcRefreshAgain = false
    private func scheduleHCRefresh() {
        guard hcRefreshTask == nil else { hcRefreshAgain = true; return }
        hcRefreshTask = Task { [weak self] in
            await self?.refreshHC()
            while let self, self.hcRefreshAgain, !Task.isCancelled {
                self.hcRefreshAgain = false
                try? await Task.sleep(nanoseconds: 50_000_000)
                await self.refreshHC()
            }
            self?.hcRefreshTask = nil
        }
    }

    /// The first event refreshes at once; events that arrive while that refresh runs (or within 50 ms after it)
    /// are coalesced into one more refresh. A burst costs at most two snapshots, a single event costs no delay.
    @ObservationIgnored private var refreshAgain = false
    private func scheduleRefresh() {
        guard refreshTask == nil else { refreshAgain = true; return }
        refreshTask = Task { [weak self] in
            await self?.refreshTUI()
            while let self, self.refreshAgain, !Task.isCancelled {
                self.refreshAgain = false
                try? await Task.sleep(nanoseconds: 50_000_000)
                await self.refreshTUI()
            }
            self?.refreshTask = nil
        }
    }

    // MARK: - Session logs

    /// Stop tailing logs of sessions that left the tree.
    private func syncLogs() {
        let live = Set(workspaces.flatMap(\.tabs).flatMap(\.panes).compactMap { $0.agent?.sessionId })
        for (id, t) in logTasks where !live.contains(id) {
            release(id)
        }
        recentLogs.removeAll { !live.contains($0) }
    }

    private func release(_ id: String) {
        logTasks[id]?.cancel(); logTasks[id] = nil; logItems[id] = nil; logs[id] = nil; lastRead[id] = nil
        scheduleRelief()
    }

    private func touch(_ id: String) {
        let wasVisible = lastRead[id].map { Date().timeIntervalSince($0) < 2 } ?? false
        lastRead[id] = Date()
        if recentLogs.first != id {
            recentLogs.removeAll { $0 == id }
            recentLogs.insert(id, at: 0)
        }
        if visibleOverride == nil, !wasVisible { applyDepth() }
        guard recentLogs.count > maxLiveLogs else { return }
        let blocked = Set(workspaces.flatMap(\.tabs).flatMap(\.panes).compactMap(\.agent).filter { $0.status == .blocked }.compactMap(\.sessionId))
        var i = recentLogs.count - 1
        while recentLogs.count > maxLiveLogs, i >= maxLiveLogs {
            let old = recentLogs[i]
            if blocked.contains(old) { i -= 1; continue }
            recentLogs.remove(at: i); release(old); i -= 1
        }
    }

    private func track(_ sessionId: String) {
        guard logTasks[sessionId] == nil else { return }
        let kind = workspaces.lazy.flatMap(\.tabs).flatMap(\.panes).compactMap(\.agent).first { $0.sessionId == sessionId }?.kind
        let log = SessionLog(sessionId: sessionId, agent: kind ?? "claude", remote: session.remote)
        log.setFull(visibleOverride?.contains(sessionId) ?? true)   // touch() trims it when it is not the visible one
        logs[sessionId] = log
        logTasks[sessionId] = Task { [weak self] in
            for await snap in log.snapshots() {
                self?.logItems[sessionId] = snap
            }
        }
    }
}
