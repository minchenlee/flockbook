import AppKit
import HerdrKit
import SwiftUI

/// The chat's "Since you left" card and Summaries, as in the Herdr app mod (since.js, glance.js, settings.js): what
/// came into a conversation while it was not on screen, and with Summaries on, two lines by a model about the agent's
/// last turn. The words, the 5-minute threshold and the counts are HerdrKit's (`SinceLeft`, `Summary`, `Glance`).
/// The marks, summaries and the count of model runs are the app's own files (`SinceStore`): Application Support, a
/// temp folder in checks and snapshots.
@MainActor @Observable
final class SinceModel {
    static let onKey = "summaries", byKey = "summaryBy", modelKey = "summaryModel"

    @ObservationIgnored let store: SinceStore
    /// Mark key -> the time away, while the card shows.
    var away: [String: SinceLeft.Away] = [:]
    /// The app is on screen: not hidden, not minimized, not under other windows.
    var appVisible = true
    /// Moves when a summary is kept: views that show one read it again.
    private(set) var kept = 0
    /// Runs the model for a pane; nil: no summaries are asked (checks, snapshots).
    @ObservationIgnored var runner: ((Pane) -> (any SummaryRunner)?)?
    /// Another path for the program (snapshots and checks use a stub).
    @ObservationIgnored var program: String?
    @ObservationIgnored let tails = LogTails()
    /// Snapshots: the log of an agent (nil: none), never one found on this Mac.
    @ObservationIgnored var logFor: ((Agent, String) -> URL?)?
    @ObservationIgnored private var located: [String: URL] = [:]
    @ObservationIgnored private var busy = false
    /// The round asking now.
    @ObservationIgnored private var batch: Task<Void, Never>?
    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private var observers: [NSObjectProtocol] = []

    init(dir: URL) {
        store = SinceStore(dir: dir)
    }

    /// A check or snapshot run: never this Mac's marks or logs.
    static let isCheck = AppDefaults.isTest || CommandLine.arguments.contains(where: { $0.hasPrefix("--snapshot") || $0.hasPrefix("--test") || $0.hasPrefix("--hittest") })

    /// The app's folder; a fresh temp folder for every check and snapshot run, so they never see the owner's marks.
    static func defaultDir() -> URL {
        if isCheck {
            return URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("flockbook-since-\(getpid())")
        }
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        return support.appendingPathComponent("Flockbook/since")
    }

    var settings: Summary.Settings {
        let d = AppDefaults.store
        return Summary.Settings(on: d.bool(forKey: Self.onKey), by: d.string(forKey: Self.byKey) ?? "own", model: d.string(forKey: Self.modelKey) ?? "")
    }

    func key(_ pane: Pane, session: HerdrSession) -> String { SinceStore.key(session: session.name, terminal: pane.terminalId) }

    /// The two lines for the turn `id` of the conversation `key` now, if any.
    func summary(_ key: String, id: String?) -> Summary.Kept? {
        _ = kept
        return Summary.shown(store.summaries[key], id: id, settings: settings)
    }

    /// The usage line under Summaries: today and the last 7 days.
    func usageText(now: Date = Date()) -> String {
        let ms = { (d: Date) in Int64(d.timeIntervalSince1970 * 1000) }
        let u = store.usage(since: [ms(Calendar.current.startOfDay(for: now)), ms(now) - 7 * 86_400_000], now: ms(now))
        return Summary.usageText(today: u[0], week: u[1])
    }

    // MARK: on screen

    /// Follows the app's own visibility (hidden, minimized, covered): coming back is an arrival too.
    func watchApp() {
        guard observers.isEmpty else { return }
        let c = NotificationCenter.default
        let update: @Sendable (Notification) -> Void = { _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                let v = !NSApp.isHidden && NSApp.occlusionState.contains(.visible) && NSApp.windows.contains { $0.isVisible && !$0.isMiniaturized }
                if v != self.appVisible { self.appVisible = v }
            }
        }
        for n in [NSApplication.didHideNotification, NSApplication.didUnhideNotification, NSApplication.didChangeOcclusionStateNotification,
                  NSWindow.didMiniaturizeNotification, NSWindow.didDeminiaturizeNotification] {
            observers.append(c.addObserver(forName: n, object: nil, queue: .main, using: update))
        }
    }

    /// The pane's conversation is on screen (`onScreen`): the card for the time away, decided on a read of the log made
    /// now (`SinceLeft.Arrival`, as the mod waits for a glance from after it came on screen), then a mark every 2 s of
    /// what it shows. Off screen: nothing (the mark stays at the last time it was seen).
    func watch(_ pane: Pane, onScreen: Bool, model: AppModel) async {
        guard let agent = pane.agent, let sid = agent.sessionId else { return }
        let key = key(pane, session: model.store.session)
        guard onScreen else { return }
        let now = { Int64(Date().timeIntervalSince1970 * 1000) }
        var arrival = SinceLeft.Arrival()
        while !Task.isCancelled {
            let state = model.store.conversationState(for: sid)
            let shown = state == .ready ? Glance.facts(model.store.conversation(for: sid), agent: agent.kind).id : nil
            if arrival.decided == nil, state != .loading {
                let tails = tails
                let url = logURL(agent, sid: sid, store: model.store)
                let read = await Task.detached { url.flatMap { tails.items($0) } }.value
                guard !Task.isCancelled else { return }
                let r = arrival.first(read.flatMap { Glance.facts($0, agent: agent.kind).id }, shown: shown, seen: store.seen[key], now: now())
                if r.decided { away[key] = r.away }
            }
            if let id = arrival.mark(shown) { store.markSeen(key, id: id, now: now()) }
            try? await Task.sleep(for: arrival.decided == nil ? .milliseconds(500) : .seconds(2))
        }
    }

    // MARK: summaries

    /// Every 5 s while a tab shows and the app is on screen: the stopped agents on this Mac get a summary (Summaries on).
    /// A round that is still asking does not hold the loop: the next one starts when it ends.
    func start(_ model: AppModel) {
        guard loop == nil else { return }
        watchApp()
        observers.append(NotificationCenter.default.addObserver(forName: NSApplication.willTerminateNotification, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.stop() }
        })
        loop = Task { @MainActor [weak self, weak model] in
            while !Task.isCancelled {
                if let self, let model {
                    if !self.settings.on { self.batch?.cancel() }
                    if model.screen == .tab, self.appVisible { await self.tick(model) }
                }
                try? await Task.sleep(for: .seconds(5))
            }
        }
    }

    /// Ends the runs of a model now (the app quits): their programs stop with them.
    func stop() {
        batch?.cancel()
    }

    /// Waits for the round that is asking now (checks and snapshots).
    func settle() async { await batch?.value }

    /// The agent's status now, in the store (snapshots: another).
    @ObservationIgnored var statusNow: ((Pane, AppModel) -> AgentStatus?)?
    private func status(_ pane: Pane, _ model: AppModel) -> AgentStatus? {
        if let statusNow { return statusNow(pane, model) }
        return model.store.workspaces.flatMap(\.tabs).flatMap(\.panes).first { $0.id == pane.id }?.agent?.status
    }

    /// One round: the agents to ask about (`Summary.next`), at most three at a time, each turn once. Returns once the
    /// runs started; the round ends when the last answers (`settle`). Each run starts only if Summaries is still on
    /// with the same writer, and the agent still stopped, when its turn comes (reading the logs takes time).
    func tick(_ model: AppModel) async {
        let s = settings
        guard s.on, !busy, let runner else { return }
        busy = true
        let session = model.store.session
        var cands: [(Summary.Candidate, Pane, [ConversationItem])] = []
        var keys = Set<String>()
        for pane in model.store.workspaces.flatMap(\.tabs).flatMap(\.panes) {
            // Only agents on this Mac: the turn of an agent on another machine stays there.
            guard let a = pane.agent, let sid = a.sessionId, pane.composerRemote(session.remote) == nil, !pane.terminalId.contains("/") else { continue }
            let key = key(pane, session: session)
            keys.insert(key)
            guard a.status != .working, a.status != .blocked, let url = logURL(a, sid: sid, store: model.store) else { continue }
            let tails = tails
            guard let items = await Task.detached(operation: { tails.items(url) }).value else { continue }
            cands.append((Summary.Candidate(key: key, kind: a.kind, status: a.status, facts: Glance.facts(items, agent: a.kind)), pane, items))
        }
        let prefix = SinceStore.key(session: session.name, terminal: "")
        store.prune(keeping: keys.union(store.summaries.keys.filter { !$0.hasPrefix(prefix) }))
        let picks = Summary.next(cands.map(\.0), have: store.summaries, settings: s)
        let program = program
        let runs: [(Summary.Candidate, Pane, [ConversationItem], String, any SummaryRunner)] = picks.compactMap { p in
            guard let (c, pane, items) = cands.first(where: { $0.0.key == p.key }), let id = c.facts.id, let run = runner(pane) else { return nil }
            return (c, pane, items, id, run)
        }
        guard !runs.isEmpty else { busy = false; return }
        // At most `Summary.batch` side by side; calling the round off calls each run off.
        batch = Task { @MainActor [weak self, weak model] in
            let tasks = runs.map { (c, pane, items, id, run) in
                Task { @MainActor [weak self, weak model] in
                    guard let self, let model, !Task.isCancelled else { return }
                    // Checked again when the run starts: no await between here and the program's start.
                    guard self.settings == s, let st = self.status(pane, model), st != .working, st != .blocked else { return }
                    let r = await Summary.ask(items: items, kind: c.kind, settings: s, runner: run, store: self.store, program: program)
                    guard !Task.isCancelled else { return }   // called off: the turn is asked again later
                    switch r {
                    case .success(let a): self.store.keep(c.key, .init(id: a.id, by: s.writer, state: a.state, needs: a.needs, model: a.model))
                    case .failure: self.store.keep(c.key, .init(id: id, by: s.writer, failed: true))   // not asked again
                    }
                    self.kept += 1
                }
            }
            await withTaskCancellationHandler {
                for t in tasks { await t.value }
            } onCancel: {
                for t in tasks { t.cancel() }
            }
            guard let self else { return }
            self.busy = false
            self.batch = nil
            // The next agents now, not after the next 5 s.
            if let model, !Task.isCancelled, self.loop != nil, model.screen == .tab, self.appVisible { await self.tick(model) }
        }
    }

    /// The agent's log: the one the store reads, else found once on disk (it is not on screen).
    private func logURL(_ a: Agent, sid: String, store: HerdrStore) -> URL? {
        if let logFor { return logFor(a, sid) }
        // Checks and snapshots read only the logs they wrote (`logFor`), never one found on this Mac.
        if Self.isCheck { return nil }
        if let u = store.logURL(for: sid) { return u }
        if let u = located[sid], FileManager.default.fileExists(atPath: u.path) { return u }
        let u = SessionLog.locate(agent: a.kind, sessionId: sid, dir: SessionLog.defaultDir(agent: a.kind))
        located[sid] = u
        return u
    }

    /// The app's runner: through herdr-core on this Mac, as the Herdr app's helper runs it (`Summary.route`); no
    /// herdr-core: no summary.
    static func liveRunner(_ store: HerdrStore) -> (Pane) -> (any SummaryRunner)? {
        { pane in
            guard Summary.route(backend: pane.backend, core: store.hcClient != nil) == .core, let hc = store.hcClient else { return nil }
            return HCExecRunner(client: hc)
        }
    }
}

/// Snapshots: where the card and the reply box of a real pane stand (global frames), while `on`.
@MainActor enum SinceProbe {
    static var on = false
    static var card = CGRect.zero, composer = CGRect.zero
}

/// A press of the card's Jump: a new token each time, so the same message can be jumped to again.
struct SinceJump: Equatable { let id: String; let token: Int }

/// The card, over the reply box. As the mod's sinceCard: the title and ×, the counts, the summary (marked as a
/// model's), the line from the log, and the owner's last message with Jump.
struct SinceCard: View {
    let card: SinceLeft.Card
    var summary: Summary.Kept?
    var onJump: (String) -> Void = { _ in }
    var onHide: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Text(SinceLeft.Texts.title).font(.system(size: 13.5, weight: .semibold)).frame(maxWidth: .infinity, alignment: .leading)
                Button(action: onHide) { Image(systemName: "xmark").font(.system(size: 10, weight: .semibold)).frame(width: 22, height: 22).contentShape(Rectangle()) }
                    .buttonStyle(.plain).foregroundStyle(.secondary).help("Hide").accessibilityLabel("Hide")
            }
            small(SinceLeft.countLine(card))
            if let state = summary?.state, !state.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text(SinceLeft.Texts.summaryMark).font(.system(size: 11.5)).foregroundStyle(.tertiary)
                    Text(state).font(.system(size: 13)).textSelection(.enabled)
                    if let needs = summary?.needs { Text(SinceLeft.Texts.needsPrefix + needs).font(.system(size: 13)).textSelection(.enabled) }
                }
            }
            let facts = SinceLeft.factsLine(card, summary: summary?.state?.isEmpty == false)
            if !facts.isEmpty { small(facts) }
            if let said = card.said {
                HStack(spacing: 10) {
                    small(SinceLeft.saidLine(said)).lineLimit(1).truncationMode(.tail).frame(maxWidth: .infinity, alignment: .leading)
                    Button(SinceLeft.Texts.jump) { onJump(said.id) }.controlSize(.small)
                }
            }
        }
        .padding(14)
        .frame(maxWidth: 760, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(.background.secondary))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(.separator, lineWidth: 1))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(SinceLeft.Texts.title)
    }

    private func small(_ s: String) -> some View { Text(s).font(.system(size: 12.5)).foregroundStyle(.secondary) }
}

/// A program's models for the Model row: Claude Code's list kept in HerdrKit, Codex's own file, pi's own list.
@MainActor @Observable
final class SummaryModels {
    struct Got: Equatable { var models: [(id: String, note: String)]; var source: String?; var failed: Bool
        static func == (a: Got, b: Got) -> Bool { a.models.map(\.id) == b.models.map(\.id) && a.source == b.source && a.failed == b.failed }
    }
    private(set) var got: [String: Got] = [:]
    /// Snapshots: the lists, never read from this Mac.
    @ObservationIgnored var fixed: [String: Got]?

    init(fixed: [String: Got]? = nil) { self.fixed = fixed; if let fixed { got = fixed } }

    func load(_ by: String) async {
        if fixed != nil || got[by] != nil { return }
        switch by {
        case "claude": got[by] = Got(models: Summary.claudeModels, source: "built-in", failed: false)
        case "codex":
            let env = ProcessInfo.processInfo.environment
            let home = env["CODEX_HOME"].flatMap { $0.isEmpty ? nil : $0 } ?? NSHomeDirectory() + "/.codex"
            let text = try? String(contentsOfFile: home + "/models_cache.json", encoding: .utf8)
            let ms = text.map(Summary.codexModels) ?? []
            got[by] = Got(models: ms, source: ms.isEmpty ? nil : "file", failed: ms.isEmpty)
        case "pi":
            let r = await ProcessRunner().run(argv: ["pi", "--offline", "--list-models"], input: "", cwd: NSTemporaryDirectory(), timeout: 20)
            let ms = r.code == 0 ? Summary.piModels(r.stdout) : []
            got[by] = Got(models: ms, source: ms.isEmpty ? nil : "program", failed: ms.isEmpty)
        default: break
        }
    }
}

/// Settings > General > What agents did: Summaries, Written by and Model, with the mod's words.
struct SummariesGroup: View {
    @AppStorage(SinceModel.onKey, store: AppDefaults.store) private var on = false
    @AppStorage(SinceModel.byKey, store: AppDefaults.store) private var by = "own"
    @AppStorage(SinceModel.modelKey, store: AppDefaults.store) private var saved = ""
    var usage: String = ""
    var models: SummaryModels
    @State private var field = Summary.ModelField(saved: "")
    @FocusState private var typing: Bool

    var body: some View {
        SettingsGroup("What agents did", content: {
            SettingsRow(Summary.Texts.summaries) {
                Picker(Summary.Texts.summaries, selection: $on) { Text("Off").tag(false); Text("On").tag(true) }
                    .pickerStyle(.segmented).labelsHidden().fixedSize()
            }
            .settingsControl(.summaries)
            if on {
                // Stacked, as in the mod: the four choices are wide, the name goes over them.
                VStack(alignment: .leading, spacing: 8) {
                    Text(Summary.Texts.by)
                    Picker(Summary.Texts.by, selection: Binding(get: { by }, set: { if $0 != by { by = $0; saved = ""; field = .init(saved: "") } })) {
                        ForEach(Summary.Texts.byChoices, id: \.0) { Text($0.1).tag($0.0) }
                    }
                    .pickerStyle(.segmented).labelsHidden()
                    Text(Summary.Texts.byNotes[by] ?? "").font(.system(size: 11.5)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal, 14).padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .settingsControl(.summaryBy)
                if by != "own" { modelRow.settingsControl(.summaryModel) }
            }
        }, footer: {
            VStack(alignment: .leading, spacing: 4) {
                Text(Summary.Texts.summariesAbout)
                if on {
                    Text(Summary.Texts.sent)
                    if !usage.isEmpty { Text(usage) }
                    Text("\(Summary.Texts.by): \(Summary.Texts.byAbout)")
                }
            }
        })
        .task(id: on ? by : "") { if on, by != "own" { await models.load(by) } }
        .onAppear { field = .init(saved: saved) }
    }

    @ViewBuilder private var modelRow: some View {
        let got = models.got[by]
        let hits = Summary.matches(got?.models ?? [], typed: field.typed, saved: saved)
        VStack(alignment: .leading, spacing: 6) {
            SettingsRow(Summary.Texts.model) {
                // As the mod: typing narrows the list; the name is kept on Return, when the field loses the keys, or on a click.
                TextField(Summary.Texts.modelEmpty[by] ?? "", text: Binding(get: { field.typed }, set: { _ = field.type($0) }))
                    .textFieldStyle(.roundedBorder).frame(width: 260)
                    .focused($typing)
                    .onSubmit { if let v = field.commit() { saved = v } }
                    .onChange(of: typing) { _, now in if !now, let v = field.commit() { saved = v } }
                    .onDisappear { if let v = field.commit() { saved = v } }
            }
            VStack(alignment: .leading, spacing: 2) {
                ForEach(Array(hits.prefix(Summary.Texts.modelsShown)), id: \.id) { m in
                    Button { if let v = field.pick(m.id) { saved = v } } label: {
                        HStack(spacing: 8) {
                            Image(systemName: m.id == saved ? "checkmark" : "").frame(width: 12)
                            Text(m.id).font(.system(size: 12, design: .monospaced))
                            if !m.note.isEmpty { Text(m.note).font(.system(size: 11.5)).foregroundStyle(.secondary).lineLimit(1) }
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                }
                Group {
                    if let got, !got.models.isEmpty {
                        if hits.count > Summary.Texts.modelsShown { Text(Summary.Texts.more(hits.count - Summary.Texts.modelsShown)) }
                        if hits.isEmpty { Text(Summary.Texts.noMatch) }
                        Text(got.source.flatMap { Summary.Texts.modelSource[$0] } ?? "")
                    } else if got != nil {
                        Text(Summary.Texts.empty(by, failed: got?.failed == true))
                    }
                    Text(Summary.Texts.modelAbout)
                }
                .font(.system(size: 11.5)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, 14).padding(.bottom, 8)
        }
    }
}
