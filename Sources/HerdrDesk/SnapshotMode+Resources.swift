import AppKit
import HerdrKit
import SwiftUI

/// `--snapshot-resources <dir>`: the Resources panel over a tab, offscreen, light and dark. Synthetic only: a log written
/// to a temp folder (docs there, one of them missing; 35 links; a command), the preview tree's agent, a click plan that
/// is recorded (never NSWorkspace, never the pasteboard). Reads and writes no settings file of the owner.
extension SnapshotRunner {
    static func runResources(dir: String) {
        Task { @MainActor in await resources(dir: dir) }
    }

    @MainActor private static func resources(dir: String) async {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("snapshot-resources: \(n) \(ok ? "ok" : "FAIL")") }
        defer { print("snapshot-resources: \(failures == 0 ? "PASS" : "FAIL x\(failures)"), written to \(dir)"); exit(failures == 0 ? 0 : 1) }

        // The icons are the mod's files.
        let iconDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("../../herdr-app/mods/flockbook/icons").standardized
        let differ = ResourceIcons.svg.filter { name, s in (try? String(contentsOf: iconDir.appendingPathComponent("\(name).svg"), encoding: .utf8))?.trimmingCharacters(in: .whitespacesAndNewlines) != s }
        step("the icons are the mod's files (\(ResourceIcons.svg.count); differ: \(differ.keys.sorted()))", differ.isEmpty && ResourceIcons.svg.count == 15)
        step("each icon draws (NSImage from its SVG)", ResourceIcons.svg.keys.allSatisfy { ResourceIcons.image($0) != nil })

        let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("flockbook-res-shot-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: tmp) }
        let root = tmp.path.withCString { Darwin.realpath($0, nil) }.map { p in defer { free(p) }; return String(cString: p) } ?? tmp.path
        for f in ["plan.md", "report.html", "notes.pdf"] { try? Data("x\n".utf8).write(to: URL(fileURLWithPath: "\(root)/\(f)")) }
        let now = Date()
        let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        func says(_ id: String, _ min: Double, _ text: String) -> String {
            let o: [String: Any] = ["type": "assistant", "uuid": id, "timestamp": f.string(from: now.addingTimeInterval(-min * 60)), "cwd": root,
                                    "message": ["role": "assistant", "content": [["type": "text", "text": text]]]]
            return String(decoding: try! JSONSerialization.data(withJSONObject: o, options: [.sortedKeys]), as: UTF8.self)
        }
        let links = (0..<35).map { "- [Page \($0)](https://example.com/page/\($0))" }.joined(separator: "\n")
        var lines = [
            says("a0", 90, "I wrote the plan in `\(root)/plan.md` and the report in `\(root)/report.html`. The old draft `\(root)/gone.md` is deleted."),
            says("a1", 60, "The figures are in `\(root)/notes.pdf`. Pages I read:\n\n\(links)"),
            says("a2", 30, "Run the tests:\n\n```sh\nmake test\n```\n\nThe PR is https://github.com/example/app/pull/12."),
        ]
        let log = tmp.appendingPathComponent("agent.jsonl")
        func writeLog() { try? Data((lines.joined(separator: "\n") + "\n").utf8).write(to: log) }
        writeLog()
        guard let items = Resources.collect(file: log, home: "/nonexistent-home", agent: root) else { step("the log reads", false); return }
        let kinds = Dictionary(grouping: items, by: { (i: Resources.Item) in i.kind }).mapValues { $0.count }
        step("the log gives docs, links, a PR and a command (\(kinds.sorted { $0.key < $1.key }))",
             kinds["doc"] == 4 && kinds["link"] == 35 && kinds["pr"] == 1 && kinds["cmd"] == 1)
        let gone = items.first { $0.target.hasSuffix("gone.md") }
        step("a missing doc is marked missing", gone?.exists == false && items.first { $0.target.hasSuffix("plan.md") }?.exists == true)

        // The tab: the preview tree's first one-pane agent tab.
        let model = AppModel(store: .previewWithSessions)
        guard let ws = model.store.workspaces.first(where: { $0.tabs.contains { !$0.isSplit && $0.panes.first?.agent != nil } }),
              let tab = ws.tabs.first(where: { !$0.isSplit && $0.panes.first?.agent != nil }), let pane = tab.panes.first else {
            step("a one-pane agent tab in the preview tree", false); return
        }
        model.select(tab: tab, in: ws)
        let r = model.resources
        var plans: [Resources.Plan] = []
        r.perform = { plans.append($0); if case .why(let s) = $0 { return (s, false) }; if case .copy = $0 { return ("Copied.", true) }; return nil }
        let key = ResourcesModel.key(pane, session: model.store.session.name ?? "")
        step("the panel is about the tab's agent", model.resourcesKey == key)
        r.put(key, .init(items: items, stamp: nil))
        step("first look at an agent: what is there is seen, no badge (\(r.badge(key)))", r.badge(key) == 0)
        lines.append(says("a3", 1, "One more: https://example.com/new."))
        writeLog()
        r.put(key, .init(items: Resources.collect(file: log, home: "/nonexistent-home", agent: root) ?? [], stamp: nil))
        step("a newer one lights the badge (\(r.badge(key)))", r.badge(key) == 1)

        // Clicks: a command is copied (recorded), never run; a missing doc says why; a link goes to the browser.
        let cmd = r.items(key).first { $0.kind == "cmd" }!
        r.click(cmd, machine: nil)
        step("a command's click copies it, runs nothing (\(plans))", plans == [.copy("make test")] && r.note?.text == "Copied.")
        r.click(r.items(key).first { $0.target.hasSuffix("gone.md") }!, machine: nil)
        step("a missing doc's click says File not found.", r.note?.text == "File not found.")
        r.click(r.items(key).first { $0.kind == "pr" }!, machine: nil)
        step("a PR's click opens its address", plans.last == .url(URL(string: "https://github.com/example/app/pull/12")!) && r.note == nil)
        r.click(r.items(key).first { $0.target.hasSuffix("plan.md") }!, machine: "box")
        step("a doc on another machine says where it is", r.note?.text == "This file is on box. Open it there.")
        r.note = nil
        step("the hc machine of a remote pane", ResourcesModel.machine(Pane(id: "hc:box/t7", terminalId: "t7", cwd: "/", agent: nil), remote: nil) == "box" && ResourcesModel.machine(Pane(id: "hc:t7", terminalId: "t7", cwd: "/", agent: nil), remote: nil) == nil)

        // The panel's place: right, under the toolbar, over the reply box; the chat keeps its frame.
        let size = NSSize(width: 1020, height: 768)
        SinceProbe.on = true; ResourcesProbe.on = true
        defer { SinceProbe.on = false; ResourcesProbe.on = false }
        func shot(_ name: String, dark: Bool) async -> (pane: CGRect, panel: CGRect, composer: CGRect) {
            QuestionProbe.paneFrame = .zero; ResourcesProbe.panel = .zero; SinceProbe.composer = .zero
            let (win, host) = ContentTests.offscreen(ZStack { WindowSurface(glass: .off); TabScreen() }
                .environment(model).environment(\.flatGlass, true).environment(\.colorScheme, dark ? .dark : .light), size)
            win.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            for _ in 0..<8 { try? await Task.sleep(for: .milliseconds(150)); host.layoutSubtreeIfNeeded(); host.displayIfNeeded() }
            let away = !NSScreen.screens.contains { $0.frame.intersects(win.frame) }
            if let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
                host.cacheDisplay(in: host.bounds, to: rep)
                try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "\(dir)/\(name).png"))
            }
            step("\(name): offscreen, picture written", away && FileManager.default.fileExists(atPath: "\(dir)/\(name).png"))
            let got = (QuestionProbe.paneFrame, ResourcesProbe.panel, SinceProbe.composer)
            win.orderOut(nil)
            return got
        }
        let closed = await shot("resources-closed", dark: false)
        step("closed: no panel", closed.panel == .zero)
        model.toggleResources()
        step("open: the badge goes (seen)", r.open && r.badge(key) == 0)
        step("open: the newer one has its dot", r.items(key).contains { Resources.isNew($0, seen: r.seenAtOpen) })
        for dark in [false, true] {
            let o = await shot("resources-\(dark ? "dark" : "light")", dark: dark)
            let p = o.panel
            step("\(dark ? "dark" : "light"): the chat does not move (\(o.pane) vs \(closed.pane))", o.pane == closed.pane && o.pane.width > 0)
            step("\(dark ? "dark" : "light"): the panel is 320 pt wide, 12 pt from the right (\(p))", abs(p.width - 320) < 0.5 && abs(size.width - p.maxX - 12) < 0.5)
            step("\(dark ? "dark" : "light"): it ends 12 pt over the reply box (bottom \(Int(p.maxY)), box top \(Int(o.composer.minY)))",
                 o.composer.height > 0 && abs(o.composer.minY - p.maxY - 12) < 1 && p.height > 300)
        }
        // Fold, Show more, remove.
        step("links: 30 rows, then Show 6 more", Resources.layoutGroups(r.items(key), collapsed: r.collapsed, expanded: r.expanded).last.map { $0.shown.count == 30 && $0.rest == 6 } == true)
        r.expanded.insert("links")
        step("Show more draws all 36", Resources.layoutGroups(r.items(key), collapsed: r.collapsed, expanded: r.expanded).last?.shown.count == 36)
        r.toggleGroup("links")
        step("a folded group is kept in the settings", AppDefaults.store.stringArray(forKey: ResourcesModel.collapsedKey) == ["links"]
             && Resources.layoutGroups(r.items(key), collapsed: r.collapsed).last?.collapsed == true)
        r.remove(r.items(key).first { $0.target.hasSuffix("report.html") }!, key: key)
        step("Remove from list: gone, counted as removed (\(r.items(key).count) items, \(r.hidden(key)) removed)", r.hidden(key) == 1 && !r.items(key).contains { $0.target.hasSuffix("report.html") })
        _ = await shot("resources-folded", dark: false)
        // Kept in memory: the last eight agents.
        for i in 0..<9 { r.put("s|other\(i)", .init(items: [], stamp: nil)) }
        step("the lists of the last \(Resources.cacheMax) agents stay in memory", r.lists.count == Resources.cacheMax && r.lists[key] == nil)
        // A narrow window: a drawer on the right side.
        let d = ResourcesPlacement.of(size: CGSize(width: 600, height: 700), inputTop: 560)
        step("narrow: a drawer, flush right, from the top", d.drawer && d.trailing == 0 && d.top == 0 && d.width == 320)
        model.toggleResources()
        step("⌘⇧R again closes it", !r.open)
        step("no click went to NSWorkspace or the pasteboard: every one recorded (\(plans.count))", plans.count == 4)
    }
}
