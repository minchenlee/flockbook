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
        step("Summaries is off by default", since.settings == Summary.Settings())
        await since.tick(model)
        step("off: no model runs", since.store.summaries.isEmpty && !FileManager.default.fileExists(atPath: asked.path))
        AppDefaults.store.set(true, forKey: SinceModel.onKey)
        await since.tick(model)
        let kept = since.store.summaries[key]
        step("on: the stopped agent's last turn is asked (\(String(describing: kept)))", kept?.state?.hasPrefix("Quoted newlines") == true && kept?.needs == "Say whether to add a changelog line.")
        let sent = (try? String(contentsOf: asked, encoding: .utf8)) ?? ""
        step("the model gets the owner's message, the end of the turn and the line of facts", sent.hasPrefix("OWNER ASKED:\nFix the importer") && sent.contains("FROM THE LOG: edited 2 files; ran 1 commands; tests passed (pytest)"))
        step("only that agent: others have no log here", since.store.summaries.count == 1)
        try? FileManager.default.removeItem(at: asked)
        await since.tick(model)
        step("each turn once: not asked again", !FileManager.default.fileExists(atPath: asked.path))
        step("the usage line counts the run (\(since.usageText()))", since.usageText().hasPrefix("Today: 1 run of a model, about "))
        AppDefaults.store.set(false, forKey: SinceModel.onKey)
        let items = SessionLog.parse(lines: lines)
        let facts = Glance.facts(items, agent: "claude")
        step("off: a kept summary is not shown", since.summary(key, id: facts.id) == nil)
        AppDefaults.store.set(true, forKey: SinceModel.onKey)
        step("on: the kept summary is shown for its turn", since.summary(key, id: facts.id)?.state == kept?.state)

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
        if let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
            host.cacheDisplay(in: host.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
        }
        inspect()
        win.orderOut(nil)
        return away
    }
}
