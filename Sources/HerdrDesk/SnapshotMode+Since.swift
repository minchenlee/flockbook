import AppKit
import HerdrKit
import SwiftUI

/// `--snapshot-since <dir>`: the Since you left card over the reply box, offscreen, light and dark, at the size of the
/// window's content (1280 × 820 less the sidebar and the toolbar): the line from the log, a summary with what it needs,
/// nothing new, and a narrow pane; Settings > General > What agents did, off, on and with pi. Also the model side, with
/// synthetic data only: a temp folder for the app's files, a log written here, and a stub program for the model
/// (never a real model, never this Mac's logs or model lists). Reads and writes no settings file of the owner.
extension SnapshotRunner {
    static func runSince(dir: String) {
        Task { @MainActor in await since(dir: dir) }
    }

    /// The conversation the shots and the model checks read: the owner asks, the agent edits, runs pytest and answers.
    static func sinceLog(now: Date) -> [String] {
        let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        func at(_ min: Double) -> String { f.string(from: now.addingTimeInterval(-min * 60)) }
        func json(_ o: [String: Any]) -> String { String(decoding: try! JSONSerialization.data(withJSONObject: o, options: [.sortedKeys]), as: UTF8.self) }
        func user(_ id: String, _ m: Double, _ text: String) -> String { json(["type": "user", "uuid": id, "timestamp": at(m), "message": ["role": "user", "content": text]]) }
        func says(_ id: String, _ m: Double, _ text: String) -> String {
            json(["type": "assistant", "uuid": id, "timestamp": at(m), "message": ["role": "assistant", "content": [["type": "text", "text": text]]]])
        }
        func call(_ id: String, _ m: Double, _ tool: String, _ input: [String: Any], _ cid: String) -> String {
            json(["type": "assistant", "uuid": id, "timestamp": at(m), "message": ["role": "assistant", "content": [["type": "tool_use", "id": cid, "name": tool, "input": input]]]])
        }
        func result(_ id: String, _ m: Double, _ cid: String, _ text: String) -> String {
            json(["type": "user", "uuid": id, "timestamp": at(m), "message": ["role": "user", "content": [["type": "tool_result", "tool_use_id": cid, "content": text]]]])
        }
        return [
            user("u0", 95, "Read the importer and tell me why rows go missing."),
            says("a0", 94, "The importer drops a row when a field holds a newline: the CSV reader splits it in two."),
            user("u1", 60, "Fix the importer so quoted newlines stay in one row.\nAdd a test for it."),
            says("a1", 47, "I will make the reader keep quoted fields whole, then add a test with a newline inside quotes."),
            call("c1", 46, "Edit", ["file_path": "src/importer.py", "old_string": "split", "new_string": "reader"], "t1"),
            result("r1", 46, "t1", "ok"),
            call("c2", 45, "Edit", ["file_path": "tests/test_importer.py", "old_string": "", "new_string": "def test_quoted_newline"], "t2"),
            result("r2", 45, "t2", "ok"),
            call("c3", 44, "Bash", ["command": "pytest -q tests/test_importer.py"], "t3"),
            result("r3", 43, "t3", "12 passed in 0.41s"),
            says("a2", 42, "Quoted newlines now stay in one row. The new test and the other 11 pass. The changelog does not name this fix yet: say if I should add a line."),
        ]
    }

    @MainActor private static func since(dir: String) async {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("snapshot-since: \(n) \(ok ? "ok" : "FAIL")") }
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("flockbook-since-shot-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }
        let now = Date()
        let lines = sinceLog(now: now)
        let log = tmp.appendingPathComponent("agent.jsonl")
        try? Data((lines.joined(separator: "\n") + "\n").utf8).write(to: log)

        // The model side: the stopped agent's turn goes to the stub, once, and only with Summaries on.
        let store = HerdrStore.previewWithSessions
        let model = AppModel(store: store)
        let since = SinceModel(dir: tmp.appendingPathComponent("app"))
        let panes = store.workspaces.flatMap(\.tabs).flatMap(\.panes)
        guard let pane = panes.first(where: { ($0.agent.map { Summary.programs.contains($0.kind) && $0.status != .working && $0.status != .blocked }) ?? false }),
              let agent = pane.agent else {
            step("a stopped Claude Code, Codex or pi agent in the preview tree", false); exit(1)
        }
        let key = since.key(pane, session: store.session)
        since.logFor = { a, _ in a.name == agent.name ? log : nil }
        let stub = tmp.appendingPathComponent("fake-model"), asked = tmp.appendingPathComponent("asked")
        try? Data("#!/bin/sh\ncat > '\(asked.path)'\nprintf 'STATE: Quoted newlines stay in one row; the new test and 11 others pass.\\nNEEDS: Say whether to add a changelog line.\\n'\n".utf8).write(to: stub)
        try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: stub.path)
        since.runner = { _ in ProcessRunner() }
        since.program = stub.path
        AppDefaults.store.removeObject(forKey: SinceModel.onKey)
        AppDefaults.store.removeObject(forKey: SinceModel.byKey); AppDefaults.store.removeObject(forKey: SinceModel.modelKey)
        step("Summaries is off by default", since.settings == Summary.Settings())
        await since.tick(model); await since.settle()
        step("off: no model runs", since.store.summaries.isEmpty && !FileManager.default.fileExists(atPath: asked.path))
        AppDefaults.store.set(true, forKey: SinceModel.onKey)
        await since.tick(model); await since.settle()
        let kept = since.store.summaries[key]
        step("on: the stopped agent's last turn is asked (\(String(describing: kept)))", kept?.state?.hasPrefix("Quoted newlines") == true && kept?.needs == "Say whether to add a changelog line.")
        let sent = (try? String(contentsOf: asked, encoding: .utf8)) ?? ""
        step("the model gets the owner's message, the end of the turn and the line of facts", sent.hasPrefix("OWNER ASKED:\nFix the importer") && sent.contains("FROM THE LOG: edited 2 files; ran 1 commands; tests passed (pytest)"))
        step("only that agent: others have no log here", since.store.summaries.count == 1)
        try? FileManager.default.removeItem(at: asked)
        await since.tick(model); await since.settle()
        step("each turn once: not asked again", !FileManager.default.fileExists(atPath: asked.path))
        step("the usage line counts the run (\(since.usageText()))", since.usageText().hasPrefix("Today: 1 run of a model, about "))
        AppDefaults.store.set(false, forKey: SinceModel.onKey)
        let items = SessionLog.parse(lines: lines)
        let facts = Glance.facts(items, agent: "claude")
        step("off: a kept summary is not shown", since.summary(key, id: facts.id) == nil)
        AppDefaults.store.set(true, forKey: SinceModel.onKey)
        step("on: the kept summary is shown for its turn", since.summary(key, id: facts.id)?.state == kept?.state)

        await rounds(tmp: tmp, log: log, agent: agent, model: model, step: step)
        AppDefaults.store.set(true, forKey: SinceModel.onKey)

        // The marks and the arrival, in the app's own folder.
        let ms = { (d: Date) in Int64(d.timeIntervalSince1970 * 1000) }
        since.store.markSeen(key, id: "a1", now: ms(now) - 47 * 60_000)
        step("back after 47 minutes with a new turn: a card", SinceLeft.arrive(seen: since.store.seen[key], id: facts.id, now: ms(now)) != nil)
        step("the marks are in the temp folder", FileManager.default.fileExists(atPath: tmp.appendingPathComponent("app/since-marks.json").path))

        // The pane at the window's content size, and a narrow one.
        let away = SinceLeft.Away(from: ms(now) - 50 * 60_000, back: ms(now))
        let card = SinceLeft.card(items, away: away, facts: facts)
        step("the card's counts: \(SinceLeft.countLine(card))", card.messages == 2 && card.said?.text == "Fix the importer so quoted newlines stay in one row.")
        let quiet = SinceLeft.card(items, away: .init(from: ms(now) - 30 * 60_000, back: ms(now)), facts: facts)
        let unwritable = await shotPane(items, card: card, summary: nil, dark: false, size: NSSize(width: 600, height: 500), path: tmp.appendingPathComponent("no-such-folder/x.png").path)
        step("a picture that cannot be written fails the check", !unwritable.away)
        for dark in [false, true] {
            let r = await shotRealPane(model: AppModel(store: store), agent: agent, log: log, now: now, dark: dark, path: "\(dir)/since-pane-\(dark ? "dark" : "light").png")
            step("real pane \(dark ? "dark" : "light"): back after 47 minutes, the pane's own way in shows the card right over its reply box (gap \(Int(r.gap)) pt, \(Int(r.card.width)) × \(Int(r.card.height)) pt)",
                 r.away && r.shown && r.gap >= 0 && r.gap <= 32 && r.card.height > 60)
        }
        do {
            // The pane's own approval card in the conversation (the log's pending call, its prompt on screen): no Since card.
            let r = await shotRealPane(model: AppModel(store: store), agent: agent, log: log, now: now, dark: false, path: "\(dir)/since-pane-approval.png", approval: true)
            step("real pane: an approval card in the conversation holds the place: no Since card (approval card \(r.live), Since cards that came \(SinceProbe.appeared))", r.away && r.live && !r.shown && SinceProbe.appeared == 0)
        }
        do {
            // Claude Code's status row over the reply box: the card stands right over that row, the row stays on the box.
            let line = StatusLineRecord(lines: ANSIText.lines("Opus 5.5 · ⎇ main · ctx 42%"), info: nil, time: now)
            let r = await shotRealPane(model: AppModel(store: store), agent: agent, log: log, now: now, dark: false, path: "\(dir)/since-pane-status.png", status: line)
            let over = r.status.minY - r.card.maxY, under = r.composer.minY - r.status.maxY
            step("real pane with a status row: the card right over the row (gap \(Int(over)) pt), the row on the reply box (gap \(Int(under)) pt)",
                 r.away && r.shown && r.status.height > 0 && over >= 0 && over <= 32 && under >= 0 && under <= 8)
        }
        for dark in [false, true] {
            let mode = dark ? "dark" : "light"
            for (n, c, s) in [("log", card, nil), ("summary", card, kept), ("nothing-new", quiet, nil)] as [(String, SinceLeft.Card, Summary.Kept?)] {
                let r = await shotPane(items, card: c, summary: s, dark: dark, size: NSSize(width: 1020, height: 768), path: "\(dir)/since-\(n)-\(mode).png")
                step("\(n) \(mode): off every screen, the card right over the reply box (its own 8 pt and the box's top inset; gap \(Int(r.gap)) pt), \(Int(r.card.width)) × \(Int(r.card.height)) pt",
                     r.away && r.gap >= 0 && r.gap <= 32 && r.card.width <= 760.5 && r.card.height > 60)
            }
            let narrow = await shotPane(items, card: card, summary: kept, dark: dark, size: NSSize(width: 420, height: 700), path: "\(dir)/since-narrow-\(mode).png")
            step("narrow \(mode): inside the pane", narrow.away && narrow.card.width <= 420 && narrow.card.minX >= 0)
            // Settings > General: off, on with each agent's own program, and pi with its list.
            let models = SummaryModels(fixed: ["pi": .init(models: [(id: "opencode-go/glm-5.3-flash", note: "1M context"), (id: "openrouter/qwen-coder:free", note: "262K context")], source: "program", failed: false)])
            for (n, on, by, m) in [("off", false, "own", ""), ("on", true, "own", ""), ("pi", true, "pi", "opencode-go/glm-5.3-flash")] {
                AppDefaults.store.set(on, forKey: SinceModel.onKey); AppDefaults.store.set(by, forKey: SinceModel.byKey); AppDefaults.store.set(m, forKey: SinceModel.modelKey)
                // The rows --snapshot-settings leaves out (it runs with Summaries off): Written by only when on, Model only for a set
                // program. Read while the window is up: the last window's rows leave the probe when it closes.
                var rows: Set<SettingsControl> = []
                let ok = await shotSettings(SettingsView(tab: .general, since: since, summaryModels: models), dark: dark, path: "\(dir)/since-settings-\(n)-\(mode).png") {
                    rows = Set(SettingsProbe.controls.filter { $0.value.width > 0 }.keys).intersection([.summaries, .summaryBy, .summaryModel])                }
                let want: Set<SettingsControl> = !on ? [.summaries] : by == "own" ? [.summaries, .summaryBy] : [.summaries, .summaryBy, .summaryModel]
                step("settings \(n) \(mode): off every screen, rows [\(rows.map(\.rawValue).sorted().joined(separator: ","))]", ok && rows == want)
            }
        }
        AppDefaults.store.removeObject(forKey: SinceModel.onKey)
        print("snapshot-since: \(failures == 0 ? "PASS" : "FAIL x\(failures)"), written to \(dir)")
        exit(failures == 0 ? 0 : 1)
    }

    /// The rounds of Summaries, each with its own folder and stub: what is checked when a run starts, a round that does
    /// not hold the loop, and a run called off.
    @MainActor private static func rounds(tmp: URL, log: URL, agent: Agent, model: AppModel, step: (String, Bool) -> Void) async {
        func stub(_ name: String, _ body: String) -> URL {
            let u = tmp.appendingPathComponent(name)
            try? Data("#!/bin/sh\n\(body)\n".utf8).write(to: u)
            try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: u.path)
            return u
        }
        func fresh(_ n: String, program: URL, logFor: @escaping (Agent, String) -> URL?) -> SinceModel {
            let s = SinceModel(dir: tmp.appendingPathComponent(n))
            s.runner = { _ in ProcessRunner() }
            s.program = program.path
            s.logFor = logFor
            return s
        }
        let answer = "printf 'STATE: Done.\\nNEEDS: none\\n'"
        // Changed while the logs are read: the run does not start.
        for (n, change) in [("turned off", { AppDefaults.store.set(false, forKey: SinceModel.onKey) }),
                            ("another writer", { AppDefaults.store.set("pi", forKey: SinceModel.byKey) })] as [(String, () -> Void)] {
            AppDefaults.store.set(true, forKey: SinceModel.onKey); AppDefaults.store.set("own", forKey: SinceModel.byKey)
            let asked = tmp.appendingPathComponent("asked-\(n.count)")
            let s = fresh("app-\(n.count)", program: stub("m-\(n.count)", "cat > '\(asked.path)'\n\(answer)")) { a, _ in
                guard a.name == agent.name else { return nil }
                change(); return log
            }
            await s.tick(model); await s.settle()
            step("\(n) while the logs are read: no model runs", !FileManager.default.fileExists(atPath: asked.path) && s.store.summaries.isEmpty)
        }
        AppDefaults.store.set(true, forKey: SinceModel.onKey); AppDefaults.store.set("own", forKey: SinceModel.byKey)
        do {
            let asked = tmp.appendingPathComponent("asked-working")
            let s = fresh("app-working", program: stub("m-working", "cat > '\(asked.path)'\n\(answer)")) { a, _ in a.name == agent.name ? log : nil }
            s.statusNow = { _, _ in .working }
            await s.tick(model); await s.settle()
            step("the agent started working while the logs are read: no model runs", !FileManager.default.fileExists(atPath: asked.path))
        }
        do {
            let s = fresh("app-slow", program: stub("m-slow", "cat > /dev/null\nsleep 2\n\(answer)")) { a, _ in a.name == agent.name ? log : nil }
            let began = Date()
            await s.tick(model)
            let took = Date().timeIntervalSince(began)
            await s.settle()
            step("a round that asks does not hold the loop (\(String(format: "%.1f", took)) s); its answer comes after", took < 1.5 && s.store.summaries.first?.value.state == "Done.")
        }
        do {
            // Changed while the request is on its way (herdr-core: after the connection opened): nothing goes.
            let asked = tmp.appendingPathComponent("asked-late")
            let s = fresh("app-late", program: stub("m-late", "cat > '\(asked.path)'\n\(answer)")) { a, _ in a.name == agent.name ? log : nil }
            s.runner = { _ in LateChange(change: { AppDefaults.store.set("pi", forKey: SinceModel.byKey) }) }
            await s.tick(model); await s.settle()
            step("another writer while the request is on its way: nothing goes, nothing is marked", !FileManager.default.fileExists(atPath: asked.path) && s.store.summaries.isEmpty)
            AppDefaults.store.set("own", forKey: SinceModel.byKey)
        }
        do {
            // Changed while the model runs: the round is called off at once, its program stops.
            let pidFile = tmp.appendingPathComponent("pid-writer")
            let s = fresh("app-writer", program: stub("m-hang-w", "echo $$ > '\(pidFile.path)'\nexec sleep 30")) { a, _ in a.name == agent.name ? log : nil }
            s.watchSettings()
            await s.tick(model)
            var pid: Int32 = 0
            for _ in 0..<300 where pid == 0 {
                try? await Task.sleep(for: .milliseconds(50))
                pid = Int32(((try? String(contentsOf: pidFile, encoding: .utf8)) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
            }
            let began = Date()
            AppDefaults.store.set("pi", forKey: SinceModel.byKey)
            await s.settle()
            let took = Date().timeIntervalSince(began)
            step("another writer while the model runs: its program stops (\(String(format: "%.1f", took)) s, pid \(pid)), nothing is marked", pid > 0 && took < 5 && kill(pid, 0) != 0 && s.store.summaries.isEmpty)
            AppDefaults.store.set("own", forKey: SinceModel.byKey)
        }
        do {
            let pidFile = tmp.appendingPathComponent("pid")
            let s = fresh("app-quit", program: stub("m-hang", "echo $$ > '\(pidFile.path)'\nexec sleep 30")) { a, _ in a.name == agent.name ? log : nil }
            await s.tick(model)
            var pid: Int32 = 0
            for _ in 0..<300 where pid == 0 {   // up to 15 s: a busy Mac starts programs slowly
                try? await Task.sleep(for: .milliseconds(50))
                pid = Int32(((try? String(contentsOf: pidFile, encoding: .utf8)) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
            }
            let began = Date()
            s.stop(); await s.settle()
            let took = Date().timeIntervalSince(began)
            step("the app quits during a run: its program stops (\(String(format: "%.1f", took)) s, pid \(pid), alive \(pid > 0 && kill(pid, 0) == 0), kept \(s.store.summaries.count)) and the turn is not marked", pid > 0 && took < 5 && kill(pid, 0) != 0 && s.store.summaries.isEmpty)
        }
    }

    /// A runner whose request is on its way when the settings change (herdr-core: the connection is open), then asks
    /// `go` as `HCExecRunner` does right before the request is written.
    private struct LateChange: SummaryRunner {
        let change: @MainActor @Sendable () -> Void
        func run(argv: [String], input: String, cwd: String, timeout: Int) async -> ExecResult {
            await ProcessRunner().run(argv: argv, input: input, cwd: cwd, timeout: timeout)
        }
        func run(argv: [String], input: String, cwd: String, timeout: Int, go: @escaping @Sendable () async -> Bool) async -> ExecResult {
            await change()
            guard await go() else { return ExecResult(code: nil, sent: false) }
            return await run(argv: argv, input: input, cwd: cwd, timeout: timeout)
        }
    }

    /// A real pane of the preview tree, its conversation on screen: the pane's own way in decides the card from the log
    /// (`logFor`), the owner last saw the conversation 47 minutes ago. Returns the card's frame and gap over the
    /// pane's reply box (`SinceProbe`), whether the card showed, and whether the window touched no screen.
    /// `approval`: the prompt of the log's pending call is on screen, so its approval card stands in the conversation.
    /// `status`: Claude Code's status row over the reply box. `live`: the approval card stood in the conversation.
    @MainActor private static func shotRealPane(model: AppModel, agent: Agent, log: URL, now: Date, dark: Bool, path: String,
                                                approval: Bool = false, status: StatusLineRecord? = nil) async
        -> (card: CGRect, gap: CGFloat, shown: Bool, away: Bool, status: CGRect, composer: CGRect, live: Bool) {
        guard let pane = model.store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.agent?.name == agent.name }) else { return (.zero, 0, false, false, .zero, .zero, false) }
        let since = model.since
        since.logFor = { a, _ in a.name == agent.name ? log : nil }
        let key = since.key(pane, session: model.store.session)
        since.away[key] = nil
        since.store.markSeen(key, id: "a1", now: Int64(now.timeIntervalSince1970 * 1000) - 47 * 60_000)
        SinceProbe.on = true; SinceProbe.card = .zero; SinceProbe.composer = .zero; SinceProbe.status = .zero; SinceProbe.appeared = 0
        let savedLines = StatusLineProbe.injected
        if let status, let sid = pane.agent?.sessionId { StatusLineProbe.injected[sid] = status; model.checkStatusLine(for: pane) }
        defer { SinceProbe.on = false; StatusLineProbe.injected = savedLines }
        var live = false
        // The prompt stays on screen while the window renders (the pane's own screen check finds none in the preview).
        if approval { SinceProbe.prompt.insert(pane.id); model.promptVisible.insert(pane.id) }
        defer { SinceProbe.prompt.remove(pane.id) }
        let size = NSSize(width: 1020, height: 768)
        let view = ZStack { WindowSurface(glass: .off); PaneView(pane: pane, showLabel: false, focused: true) }
            .environment(model).environment(\.flatGlass, true).environment(\.colorScheme, dark ? .dark : .light)
            .frame(width: size.width, height: size.height)
        let away = await renderSince(view, size: size, dark: dark, path: path) {
            live = model.liveCardInConversation(pane) != nil
        }
        let shown = since.away[key] != nil && SinceProbe.appeared > 0 && SinceProbe.card.height > 0
        return (SinceProbe.card, SinceProbe.composer.minY - SinceProbe.card.maxY, shown, away, SinceProbe.status, SinceProbe.composer, live)
    }

    /// A pane's conversation, the card and the reply box, as in a tab. Returns the card's frame, its gap over the reply
    /// box, and whether the window touched no screen.
    @MainActor private static func shotPane(_ items: [ConversationItem], card: SinceLeft.Card, summary: Summary.Kept?, dark: Bool, size: NSSize, path: String) async
        -> (card: CGRect, gap: CGFloat, away: Bool) {
        var cardFrame = CGRect.zero, composer = CGRect.zero
        let view = ZStack {
            Color(nsColor: .textBackgroundColor)
            VStack(spacing: 0) {
                ConversationView(items: items, agentName: "claude", agentStatus: .done, loadState: .ready)
                    .frame(maxHeight: .infinity)
                SinceCard(card: card, summary: summary)
                    .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { cardFrame = $0 }
                    .inputColumn().padding(.bottom, 8)
                ComposerView(agentName: "claude", draft: ComposerDraft(), onSend: { _ in })
                    .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { composer = $0 }
                    .inputColumn()
            }
        }
        .environment(\.flatGlass, true).environment(\.colorScheme, dark ? .dark : .light)
        .frame(width: size.width, height: size.height)
        let away = await renderSince(view, size: size, dark: dark, path: path)
        return (cardFrame, composer.minY - cardFrame.maxY, away)
    }

    @MainActor private static func shotSettings(_ v: some View, dark: Bool, path: String, inspect: () -> Void = {}) async -> Bool {
        let width: CGFloat = 760
        let fit = NSHostingController(rootView: v.environment(\.colorScheme, dark ? .dark : .light)).sizeThatFits(in: CGSize(width: width, height: 0))
        let size = NSSize(width: width, height: max(80, fit.height))
        return await renderSince(ZStack(alignment: .top) { Color(nsColor: .windowBackgroundColor); v }
            .environment(\.colorScheme, dark ? .dark : .light).frame(width: size.width, height: size.height), size: size, dark: dark, path: path, inspect: inspect)
    }

    @MainActor private static func renderSince(_ view: some View, size: NSSize, dark: Bool, path: String, inspect: () -> Void = {}) async -> Bool {
        let host = NSHostingView(rootView: view)
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        var away = true
        func check() { away = away && !NSScreen.screens.contains(where: { $0.frame.intersects(win.frame) }) }
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless(); check()
        for _ in 0..<10 { try? await Task.sleep(for: .milliseconds(150)); host.layoutSubtreeIfNeeded(); host.displayIfNeeded(); check() }
        var wrote = false
        if let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
            host.cacheDisplay(in: host.bounds, to: rep)
            if let png = rep.representation(using: .png, properties: [:]) {
                wrote = (try? png.write(to: URL(fileURLWithPath: path))) != nil && FileManager.default.fileExists(atPath: path)
            }
        }
        if !wrote { print("snapshot-since: the picture \(path) was not written") }
        inspect()
        win.orderOut(nil)
        return away && wrote
    }
}
