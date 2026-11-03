import Foundation
@testable import HerdrKit

// Layout writes on another machine go to that machine's `herdr-app` document (`doc.get/set {on}`)
// and terminals (`terminal.run {on}`), with that machine's own ids, as the official app does (mirror.rs local).
// A temporary or unconnected machine's layout is not the official app's: refused, nothing written.

private func json(_ s: String) -> Any { try! JSONSerialization.jsonObject(with: Data(s.utf8)) }

@MainActor func checksHCRemoteWrite() async throws {
    let lab = ["herdr.space": "spR", "herdr.tab": "tb1"]
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.machines = [FakeHC.machine("box"), FakeHC.machine("tmp", temporary: true)]
    fake.execReplies = [("$HOME", "/home/b\n")]   // box's home (`where`, adoption)
    fake.terminals = [
        FakeHC.terminal("t1", labels: ["herdr.space": "sp1", "herdr.tab": "tb1"], argv: ["zsh"]),
        FakeHC.terminal("box/t3", labels: lab, argv: ["zsh"], cwd: "/r"),
        FakeHC.terminal("box/t4", labels: lab, argv: ["zsh"], cwd: "/r"),
        FakeHC.terminal("box/t9", argv: ["claude"], parent: "t4"),
        FakeHC.terminal("tmp/t2", labels: ["herdr.space": "spT", "herdr.tab": "tb1"], argv: ["zsh"]),
    ]
    let localDoc = json(#"{"spaces":[{"id":"sp1","name":"Here","folder":"/w","tabs":[{"id":"tb1","name":"","root":{"id":"r","type":"row","children":[{"id":"p1","type":"block","content":{"terminal":"t1"}}]}}]}]}"#)
    fake.setDoc("herdr-app", localDoc)
    fake.setDoc("box/herdr-app", json(#"{"spaces":[{"id":"spR","name":"Remote","folder":"/r","tabs":[{"id":"tb1","name":"","root":{"id":"r","type":"row","children":[{"id":"p3","type":"block","content":{"terminal":"t3"}},{"id":"p4","type":"block","content":{"terminal":"t4"}}]}}]}]}"#))
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func boxDoc() -> HCAppDoc? { try? HCAppDoc(value: fake.doc("box/herdr-app")) }
    func localUnchanged() -> Bool { NSDictionary(dictionary: fake.doc("herdr-app") as? [String: Any] ?? [:]).isEqual(to: localDoc as! [String: Any]) }
    func calls(_ from: Int, _ m: String) -> [[String: Any]] { fake.calls[from...].filter { $0.method == m }.map(\.params) }
    func addrs() -> Set<String> { Set(fake.terminals.compactMap { $0["address"] as? String }) }

    check(HCClient.layoutWritable(id: "hc:space:box/spR") && HCClient.layoutWritable(id: "hc:tab:box/spR/tb1"),
          "remote write: remote space and tab ids offer layout actions")

    // New tab: the terminal runs on box, labeled with box's ids; box's document gets it by its local address.
    var n = fake.calls.count
    var before = addrs()
    try await hc.createTab(workspaceId: "hc:space:box/spR", label: "logs", cwd: nil)
    let runs = calls(n, "terminal.run")
    let newTab = addrs().subtracting(before).first ?? "?"
    check(runs.count == 1 && runs.first?["on"] as? String == "box" && (runs.first?["labels"] as? [String: String])?["herdr.space"] == "spR",
          "remote write: new tab runs on box with box's space id (\(runs))")
    let logs = boxDoc()?.tabIds(space: "spR").first { boxDoc()?.tabName($0) == "logs" }
    check(newTab.hasPrefix("box/") && logs.map { boxDoc()?.terminals(tab: $0) } == [String(newTab.dropFirst(4))],
          "remote write: box's document shows the new tab's terminal by its local address (\(newTab), \(boxDoc()?.value ?? [:]))")
    check(localUnchanged(), "remote write: this machine's document is not touched")
    check(calls(n, "doc.set").allSatisfy { $0["on"] as? String == "box" }, "remote write: every doc.set goes to box")

    // Split: right of t3 in box's document.
    before = addrs()
    try await hc.split(paneId: "hc:box/t3", direction: .right)
    let splitAddr = addrs().subtracting(before).first ?? "?"
    check(boxDoc()?.terminals(tab: "tb1") == ["t3", String(splitAddr.dropFirst(4)), "t4"],
          "remote write: split goes right of t3 in box's document (\(boxDoc()?.terminals(tab: "tb1") ?? []))")

    // Renames and move.
    try await hc.renameWorkspace(id: "hc:space:box/spR", to: "R2")
    try await hc.renameTab("hc:tab:box/spR/tb1", to: "main")
    check(boxDoc()?.spaceName("spR") == "R2" && boxDoc()?.tabName("tb1") == "main", "remote write: renames land in box's document")
    if let logs {
        try await hc.moveTab("hc:tab:box/spR/\(logs)", toIndex: 0)
        check(boxDoc()?.tabIds(space: "spR").first == logs, "remote write: move reorders box's tabs (\(boxDoc()?.tabIds(space: "spR") ?? []))")
    }

    // Where: remote terminals get it too.
    let w = fake.terminals.first { $0["address"] as? String == "box/t3" }.flatMap { ($0["labels"] as? [String: String])?["where"] }
    check(w == "R2 / main", "remote write: box/t3's where label follows (\(w ?? "nil"))")

    // Close pane: ends box/t4 and its sub-agent box/t9 (parent t4 on box), by their qualified addresses.
    n = fake.calls.count
    try await hc.closePane(paneId: "hc:box/t4")
    let killed = calls(n, "terminal.kill").compactMap { $0["terminal"] as? String }
    check(killed == ["box/t4", "box/t9"], "remote write: closing a remote pane ends it and its sub-agent there (\(killed))")
    check(boxDoc()?.terminals(tab: "tb1").contains("t4") == false, "remote write: the pane left box's document")
    check(localUnchanged(), "remote write: this machine's document is still untouched")

    // A temporary machine: refused, nothing written.
    n = fake.calls.count
    var threw = false
    do { try await hc.renameWorkspace(id: "hc:space:tmp/spT", to: "x") } catch { threw = true }
    let w2 = fake.calls[n...].map(\.method).filter { ["terminal.run", "terminal.kill", "doc.set"].contains($0) }
    check(threw && w2.isEmpty, "remote write: a temporary machine's layout is refused (\(w2))")

    // While the official app runs: its commands, with the ids it knows (qualified).
    let dir = NSTemporaryDirectory() + "hkrw-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let log = dir + "/argv.log", cli = dir + "/herdr-app"
    try "#!/bin/sh\necho \"$@\" >> '\(log)'\ncase \"$1\" in status) exit 0;; do) echo '{}';; esac\n".write(toFile: cli, atomically: true, encoding: .utf8)
    chmod(cli, 0o755)
    await hc.setAppCLI(HerdrAppCLI(path: cli))
    try await hc.renameTab("hc:tab:box/spR/tb1", to: "via app")
    try await hc.split(paneId: "hc:box/t3", direction: .down)
    await hc.setAppCLI(nil)
    let dos = ((try? String(contentsOfFile: log, encoding: .utf8)) ?? "").split(separator: "\n").filter { $0.hasPrefix("do ") }.map(String.init)
    check(dos == [#"do tab.rename {"id":"box/tb1","name":"via app"}"#, #"do pane.split {"direction":"down","pane":"box/p3"}"#],
          "remote write: the official app gets qualified ids (\(dos))")
}

/// on another machine `where` and new folders use that machine's home (`$HOME` there) and its git
/// checkout top (git run there), not this Mac's.
@MainActor func checksHCRemoteHome() async throws {
    let lab = ["herdr.space": "spU", "herdr.tab": "tb1"]
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [
        FakeHC.terminal("box/t3", labels: lab, argv: ["zsh"], cwd: "/home/b/proj/sub"),
        FakeHC.terminal("box/t4", labels: lab, argv: ["zsh"], cwd: "/home/b"),
        FakeHC.terminal("box/t5", labels: ["herdr.space": "spH", "herdr.tab": "tb2"], argv: ["zsh"], cwd: "/home/b"),
    ]
    fake.setDoc("box/herdr-app", json(#"{"spaces":[{"id":"spU","name":"","folder":"/home/b/proj","tabs":[{"id":"tb1","name":"","root":{"id":"r","type":"row","children":[{"id":"p3","type":"block","content":{"terminal":"t3"}},{"id":"p4","type":"block","content":{"terminal":"t4"}}]}}]},{"id":"spH","name":"","folder":"","tabs":[{"id":"tb2","name":"","root":{"id":"r2","type":"row","children":[{"id":"p5","type":"block","content":{"terminal":"t5"}}]}}]}]}"#))
    fake.execReplies = [
        ("rev-parse", "/home/b/proj/sub\t/home/b/proj\n"),
        ("$HOME", "/home/b\n"),
    ]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setGitTop { _ in "/home/should-not-be-used" }
    try await hc.renameTab("hc:tab:box/spU/tb1", to: "x")
    func whereOf(_ a: String) -> String? { fake.terminals.first { $0["address"] as? String == a }.flatMap { ($0["labels"] as? [String: String])?["where"] } }
    check(whereOf("box/t3") == "proj / x" && whereOf("box/t5") == "~ / 1",
          "remote home: where uses box's git top and box's home (\(whereOf("box/t3") ?? "nil"), \(whereOf("box/t5") ?? "nil"))")
    let execs = fake.calls.filter { $0.method == "machine.exec" }.map { $0.params["on"] as? String }
    check(!execs.isEmpty && execs.allSatisfy { $0 == "box" }, "remote home: home and git are asked on box (\(execs))")

    // A new tab in a space with no folder: box's home, not this Mac's.
    try await hc.createTab(workspaceId: "hc:space:box/spH", label: "n", cwd: nil)
    let run = fake.calls.last { $0.method == "terminal.run" }?.params
    check(run?["on"] as? String == "box" && run?["cwd"] as? String == "/home/b",
          "remote home: a new tab in a space with no folder starts in box's home (\(run?["cwd"] ?? "nil"))")
}
