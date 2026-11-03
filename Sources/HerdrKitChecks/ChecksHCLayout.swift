import Foundation
@testable import HerdrKit

// Layout writes on hc through the `herdr-app` document (docs/research/hc-api/herdr-app-doc.md).
// Fixtures and tree rules come from the official app's own tests (herdr-app 0c1a102, layout.rs / model.rs / mirror.rs).

private func json(_ s: String) -> Any { try! JSONSerialization.jsonObject(with: Data(s.utf8)) }
private func same(_ a: Any?, _ b: Any?) -> Bool {
    switch (a, b) {
    case (nil, nil): return true
    case let (x as NSObject, y as NSObject): return x.isEqual(y)
    default: return false
    }
}
private func tabDoc(_ root: String, extra: String = "") -> String {
    #"{"spaces":[{"id":"s","name":"work","folder":"/work","tabs":[{"id":"t","name":"task","root":\#(root)}]}]\#(extra)}"#
}
private func weight(_ n: [String: Any]?) -> Double { ((n?["size"] as? [String: Any])?["weight"] as? NSNumber)?.doubleValue ?? 1 }
private func find(_ n: [String: Any], _ id: String) -> [String: Any]? {
    if n["id"] as? String == id { return n }
    for c in n["children"] as? [[String: Any]] ?? [] { if let f = find(c, id) { return f } }
    return nil
}

@MainActor func checksHCLayout() async throws {
    // 1. Round trip: the official tests' node JSON, unknown fields at every level, a mod pane and a stack come back
    //    exactly as read.
    let roots = [
        ##"{ "id": "root", "type": "column", "theme": "ink", "font": "Berkeley Mono", "font_size": 13, "terminal_opacity": 0.85, "padding": [16, 12], "shadow": "lg", "background": { "gradient": ["#123a6e", "#2a6db3"], "angle": 160 }, "children": [ { "id": "a", "type": "block", "content": { "mod": "x" }, "background": "surface", "padding": 8 }, { "id": "b", "type": "block", "content": { "mod": "y" }, "background": { "image": "bg/sky.jpg" } } ] }"##,
        #"{ "id": "root", "type": "row", "divider": {"width": 1, "color": "border"}, "children": [ { "id": "sidebar", "type": "column", "size": {"px": 260}, "children": [ { "id": "spaces", "type": "block", "content": {"mod": "spaces"} }, { "id": "agents", "type": "block", "content": {"mod": "agents"} } ] }, { "id": "main", "type": "block", "content": {"tab": "active"} } ] }"#,
        #"{ "id": "root", "type": "block", "content": {"agent": "focused"}, "floating": [ { "id": "team", "content": {"mod": "team"}, "edge": "left" } ] }"#,
        #"{ "id": "t", "type": "stack", "active": "p2", "future": {"x": [1, 2.5, true]}, "children": [ { "id": "p1", "type": "block", "content": {"terminal": "t1"}, "size": {"weight": 0.7}, "title": "build" }, { "id": "p2", "type": "block", "content": {"mod": "markdown", "props": {"file": "README.md"}} } ] }"#,
    ]
    for (i, r) in roots.enumerated() {
        let raw = json(tabDoc(r, extra: #","later":{"v":2}"#))
        let doc = try HCAppDoc(value: raw)
        check(same(doc.value, raw), "layout round trip: fixture \(i) is kept exactly")
    }
    check(same(try HCAppDoc(value: json(#"{"spaces":[{"id":"s","tabs":[{"id":"t"}]}]}"#)).value, json(#"{"spaces":[{"id":"s","tabs":[{"id":"t"}]}]}"#)),
          "layout: omitted fields stay omitted (mirror.rs shared_fields_can_be_omitted)")
    let e1 = try HCAppDoc(value: nil), e2 = try HCAppDoc(value: NSNull())
    check(e1.spaces.isEmpty && e2.spaces.isEmpty, "layout: no document = empty")

    // 2. Unknown shapes are refused.
    let bad = [
        #"[1]"#, #"{"spaces":{"s":{}}}"#, #"{"spaces":[{"id":5}]}"#, #"{"spaces":[{"id":"s","tabs":[{"id":"t","root":{"id":"r","type":"grid","children":[]}}]}]}"#,
        #"{"spaces":[{"id":"s","tabs":[{"id":"t","root":{"id":"r","type":"block"}}]}]}"#,
        #"{"spaces":[{"id":"s","tabs":[{"id":"t","root":{"id":"r","type":"row","children":[{"id":"p","type":"block","content":{"terminal":"t1"},"size":"big"}]}}]}]}"#,
        #"{"spaces":[{"id":"s","tabs":[{"id":"t","root":{"id":"r","type":"row","children":[{"id":"p","type":"block","content":{"terminal":7}}]}}]}]}"#,
    ]
    for (i, b) in bad.enumerated() {
        var refused = false
        do { _ = try HCAppDoc(value: json(b)) } catch is HCAppDoc.Refusal { refused = true }
        check(refused, "layout: unknown shape \(i) is refused")
    }

    // 3. Tree rules (layout.rs tests, ported).
    do {
        // splits_reuse_a_same_axis_parent_and_wrap_otherwise
        var n: [String: Any] = ["id": "tab", "type": "stack", "children": [HCAppDoc.paneNode(id: "a", terminal: "t-a")]]
        check(HCAppDoc.split(&n, target: "a", axis: "row", new: HCAppDoc.paneNode(id: "b", terminal: "t-b"), containerId: "r1"), "split: wraps")
        check(HCAppDoc.children(find(n, "r1") ?? [:]).count == 2, "split: wrapper r1 has 2")
        _ = HCAppDoc.split(&n, target: "b", axis: "row", new: HCAppDoc.paneNode(id: "c", terminal: "t-c"), containerId: "unused")
        check(HCAppDoc.children(find(n, "r1") ?? [:]).count == 3 && find(n, "unused") == nil, "split: a same-axis parent takes it in")
        _ = HCAppDoc.split(&n, target: "c", axis: "column", new: HCAppDoc.paneNode(id: "d", terminal: "t-d"), containerId: "c1")
        check(HCAppDoc.children(find(n, "c1") ?? [:]).compactMap { $0["id"] as? String } == ["c", "d"], "split: across the axis wraps c")
        check(HCAppDoc.blocks(n).compactMap { $0["id"] as? String } == ["a", "b", "c", "d"], "split: tree order a b c d")

        // closing_a_split_gives_its_share_back_to_the_pane_it_came_from
        var a = HCAppDoc.paneNode(id: "a", terminal: "t-a"); a["size"] = ["weight": 0.7]
        var b = HCAppDoc.paneNode(id: "b", terminal: "t-b"); b["size"] = ["weight": 0.3]
        var r: [String: Any] = ["id": "tab", "type": "row", "children": [a, b]]
        _ = HCAppDoc.split(&r, target: "b", axis: "row", new: HCAppDoc.paneNode(id: "c", terminal: "t-c"), containerId: "unused")
        _ = HCAppDoc.remove(&r, id: "c"); HCAppDoc.collapse(&r)
        check(weight(find(r, "a")) == 0.7 && weight(find(r, "b")) == 0.3, "remove: the share goes back to the pane it came from")
        _ = HCAppDoc.split(&r, target: "a", axis: "row", new: HCAppDoc.paneNode(id: "d", terminal: "t-d"), containerId: "unused")
        _ = HCAppDoc.remove(&r, id: "a"); HCAppDoc.collapse(&r)
        check(weight(find(r, "d")) == 0.7 && weight(find(r, "b")) == 0.3, "remove: the first one hands its share to the next")

        // removing_collapses_single_children_but_keeps_the_root
        var root: [String: Any] = ["id": "t1", "type": "row", "children": [HCAppDoc.paneNode(id: "a", terminal: "t-a")]]
        _ = HCAppDoc.split(&root, target: "a", axis: "column", new: HCAppDoc.paneNode(id: "c", terminal: "t-c"), containerId: "c1")
        _ = HCAppDoc.remove(&root, id: "c"); HCAppDoc.collapse(&root)
        check(HCAppDoc.children(root).first?["id"] as? String == "a" && root["id"] as? String == "t1", "collapse: a split left with one pane gives way; the root keeps its id")
    }

    // 4. Model rules: the last pane closes the tab, then the space (model.rs).
    do {
        var d = try HCAppDoc(value: nil)
        d.addSpace(id: "p1", name: "work", folder: "/w", tab: "c1", tabName: "task", terminal: "t1")
        d.addTab(space: "p1", folder: "/w", id: "c2", name: "docs", terminal: "t2")
        d.addSpace(id: "p2", name: "web", folder: "/src/web", tab: "c3", terminal: "t3")
        let r1 = d.removePane(terminal: "t3")
        check(r1.tabClosed && r1.spaceClosed && d.spaceIds == ["p1"], "model: the last pane closes its tab and space")
        let r2 = d.removePane(terminal: "t2")
        check(r2.tabClosed && !r2.spaceClosed && d.tabIds(space: "p1") == ["c1"], "model: a tab closes with its last pane; the space stays")
        d.adopt(terminal: "t9", space: "p1", tab: "c1", folder: "/w")
        check(d.terminals(tab: "c1") == ["t1", "t9"], "adopt: a known tab gets the pane right of its last one")
        d.adopt(terminal: "t9", space: "p1", tab: "c1", folder: "/w")
        check(d.terminals(tab: "c1") == ["t1", "t9"], "adopt: no second pane for a shown terminal")
        d.adopt(terminal: "t8", space: "nx", tab: "tx", folder: "/n")
        check(d.spaceIds == ["p1", "nx"] && d.spaceName("nx") == "", "adopt: an unknown space is made under the label's id, unnamed")
    }

    // 5. Over the socket.
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func docValue() -> [String: Any] { fake.doc("herdr-app") as? [String: Any] ?? [:] }
    func parsed() -> HCAppDoc { (try? HCAppDoc(value: fake.doc("herdr-app"))) ?? (try! HCAppDoc(value: nil)) }
    func count(_ m: String) -> Int { fake.calls.filter { $0.method == m }.count }

    let wsId = try await hc.createWorkspace(label: "Flock", cwd: "/tmp/flock", focus: false)
    let s = try HCClient.spaceLabel(wsId)
    let run = fake.calls.last { $0.method == "terminal.run" }?.params ?? [:]
    let lab = run["labels"] as? [String: String] ?? [:]
    check(lab["herdr.space"] == s && lab["herdr.tab"] != nil && run["keep"] as? Bool == true && (run["argv"] as? [Any])?.isEmpty == true,
          "new workspace: terminal.run kept, labeled, core's shell (\(run))")
    check(parsed().spaceName(s) == "Flock" && (docValue()["spaces"] as? [[String: Any]])?.first?["folder"] as? String == "/tmp/flock",
          "new workspace: the document has the space, name and folder")
    var snap = try await hc.snapshot()
    check(snap.first?.id == wsId && snap.first?.label == "Flock" && snap.first?.tabs.first?.label == "1", "new workspace: shown, its unnamed tab is called 1")

    try await hc.createTab(workspaceId: wsId, label: "logs", cwd: "/tmp/flock")
    snap = try await hc.snapshot()
    let tabs = snap.first?.tabs ?? []
    check(tabs.map(\.label) == ["1", "logs"], "new tab: appended and named (\(tabs.map(\.label)))")

    let firstPane = tabs[0].panes[0]
    try await hc.split(paneId: firstPane.id, direction: .right)
    try await hc.split(paneId: firstPane.id, direction: .down)
    snap = try await hc.snapshot()
    let t0 = parsed().terminals(tab: try HCClient.tabLabels(tabs[0].id).tab)
    check(t0.count == 3 && t0.first == firstPane.terminalId && snap.first?.tabs.first?.panes.map(\.terminalId) == t0,
          "split: panes in the document's tree order (\(t0))")
    let tabRoot = (((docValue()["spaces"] as? [[String: Any]])?.first?["tabs"] as? [[String: Any]])?.first?["root"] as? [String: Any]) ?? [:]
    let kinds = HCAppDoc.children(tabRoot).map { $0["type"] as? String ?? "" }
    check(tabRoot["type"] as? String == "row" && kinds == ["column", "block"], "split: right joins the row, down wraps in a column (\(kinds))")

    try await hc.renameTab(tabs[0].id, to: "  main ")
    try await hc.moveTab(tabs[0].id, toIndex: 1)
    try await hc.renameWorkspace(id: wsId, to: "Herd")
    try await hc.renamePane(paneId: firstPane.id, to: "build")
    snap = try await hc.snapshot()
    check(snap.first?.label == "Herd" && snap.first?.tabs.map(\.label) == ["logs", "main"], "rename + move: names trimmed, order from the document (\(snap.first?.tabs.map(\.label) ?? []))")
    check(snap.first?.tabs.last?.panes.first?.label == "build", "rename pane: kept as the node's title")

    // Unknown fields survive Flockbook's edits.
    var withExtra = docValue()
    withExtra["future_top"] = ["keep": true]
    var sp = (withExtra["spaces"] as? [[String: Any]]) ?? []
    sp[0]["color"] = "teal"
    withExtra["spaces"] = sp
    fake.setDoc("herdr-app", withExtra)
    try await hc.renameTab(tabs[1].id, to: "tail")
    let after = docValue()
    check(same(after["future_top"], ["keep": true]) && (after["spaces"] as? [[String: Any]])?.first?["color"] as? String == "teal",
          "unknown fields: kept exactly after an edit")

    // Conflict: someone else renamed the space in between; both changes stay.
    fake.conflicts = [{ v in
        var o = v as? [String: Any] ?? [:]
        var sp = o["spaces"] as? [[String: Any]] ?? []
        sp[0]["name"] = "Theirs"
        o["spaces"] = sp
        return o
    }]
    let v0 = fake.docVersion("herdr-app")
    try await hc.renameTab(tabs[1].id, to: "mine")
    let tb1 = try HCClient.tabLabels(tabs[1].id).tab
    check(parsed().spaceName(s) == "Theirs" && parsed().tabName(tb1) == "mine" && fake.docVersion("herdr-app") == v0 + 2,
          "conflict: read again, edit re-applied on the newer document")

    // No write when nothing changed.
    let sets = count("doc.set")
    try await hc.renameTab(tabs[1].id, to: "mine")
    check(count("doc.set") == sets, "no change: no doc.set")

    // Close pane / tab / workspace end the terminals (and their unlabeled sub-agents; a kept one is spared).
    let main = snap.first!.tabs.last!
    let doomed = main.panes[1].terminalId
    try await hc.closePane(paneId: main.panes[1].id)
    check(!fake.terminals.contains { $0["address"] as? String == doomed } && !parsed().showsTerminal(doomed), "close pane: terminal ended, pane gone")
    let logs = snap.first!.tabs.first!
    let logsTerm = logs.panes[0].terminalId
    fake.terminals += [FakeHC.terminal("t900", parent: logsTerm), FakeHC.terminal("t901", labels: ["herdr.space": "kept", "herdr.tab": "k"], parent: logsTerm)]
    try await hc.closeTab(logs.id)
    let alive = Set(fake.terminals.compactMap { $0["address"] as? String })
    check(!alive.contains(logsTerm) && !alive.contains("t900") && alive.contains("t901"), "close tab: its terminals and sub-agents end; a kept sub-agent is spared (\(alive.sorted()))")
    check(parsed().tabIds(space: s).count == 1, "close tab: gone from the document")
    try await hc.closeWorkspace(id: wsId)
    check(!parsed().spaceIds.contains(s) && !fake.terminals.contains { (($0["labels"] as? [String: String])?["herdr.space"]) == s },
          "close workspace: space and terminals gone")

    // A document Flockbook does not know: nothing starts, nothing is written.
    fake.setDoc("herdr-app", json(#"{"spaces":[{"id":"x","tabs":[{"id":"y","root":{"id":"y","type":"grid","children":[]}}]}]}"#))
    let (runs, sets2) = (count("terminal.run"), count("doc.set"))
    var refused = false
    do { _ = try await hc.createWorkspace(label: "no", cwd: nil, focus: false) } catch is HCAppDoc.Refusal { refused = true }
    check(refused && count("terminal.run") == runs && count("doc.set") == sets2, "unknown document: refused before anything starts")
    fake.setDoc("herdr-app", NSNull())

    // Ids that are not hc spaces/tabs, and other machines, are refused.
    var threw = 0
    do { try await hc.closeTab("hc:term:t1") } catch { threw += 1 }
    do { try await hc.renameWorkspace(id: "w1", to: "x") } catch { threw += 1 }
    check(threw == 2, "layout: only hc spaces and tabs")

    // 6. The official app's commands when it runs: `herdr-app do`, no document write.
    let dir = NSTemporaryDirectory() + "hkapp-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let log = dir + "/argv.log", cli = dir + "/herdr-app"
    try """
    #!/bin/sh
    echo "$@" >> '\(log)'
    case "$1" in status) exit 0;; do) echo '{"space":"spX","tab":"tbX","terminal":"t5"}';; esac
    """.write(toFile: cli, atomically: true, encoding: .utf8)
    chmod(cli, 0o755)
    await hc.setAppCLI(HerdrAppCLI(path: cli))
    let sets3 = count("doc.set")
    let made = try await hc.createWorkspace(label: "Via app", cwd: "/tmp", focus: false)
    try await hc.renameTab("hc:tab:spX/tbX", to: "named")
    let lines = (try? String(contentsOfFile: log, encoding: .utf8))?.split(separator: "\n").map(String.init) ?? []
    check(made == "hc:space:spX" && lines.contains(#"do space.new {"folder":"/tmp","name":"Via app"}"#) && lines.contains(#"do tab.rename {"id":"tbX","name":"named"}"#),
          "official app running: herdr-app do (\(lines))")
    check(count("doc.set") == sets3, "official app running: no doc.set")
    await hc.setAppCLI(nil)
}
