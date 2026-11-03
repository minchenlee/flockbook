import Foundation
@testable import HerdrKit

// hc layout writes: machines, kill scope, partial failures, tab order and the official app's commands.

/// #1: a remote terminal's labels are local to its machine. Its space and tab are machine-qualified (`box/sp1`), and no
/// layout write touches them until remote layout exists (see `ChecksHCRemoteWrite`).
@MainActor func checksHCLayoutRemote() async throws {
    let lab = ["herdr.space": "sp1", "herdr.tab": "tb1"]
    let terms = [FakeHC.terminal("t1", labels: lab, argv: ["zsh"], pid: 11),
                 FakeHC.terminal("box/t3", labels: lab, argv: ["zsh"], pid: 33)]
    let ws = HCMapper.workspaces(terminals: terms, appDoc: nil)
    check(ws.map(\.id) == ["hc:space:sp1", "hc:space:box/sp1"], "the same label on two machines is two spaces (\(ws.map(\.id)))")
    check(ws.last?.tabs.first?.id == "hc:tab:box/sp1/tb1", "remote tab id is qualified (\(ws.last?.tabs.first?.id ?? "-"))")
    check(ws.first?.tabs.first?.panes.map(\.terminalId) == ["t1"], "the local space holds only the local terminal")
    let tl = try HCClient.tabLabels("hc:tab:box/sp1/tb1")
    check(tl.space == "box/sp1" && tl.tab == "box/tb1", "a remote tab id parses to its qualified space and tab (\(tl))")
    check(HCClient.layoutWritable(id: "hc:space:sp1") && HCClient.layoutWritable(id: "hc:tab:sp1/tb1"),
          "local space and tab ids are writable")
    // Remote ids offer layout actions; an op on a machine that is not connected and kept is refused below.
    check(!HCClient.layoutWritable(id: "hc:other") && !HCClient.layoutWritable(id: "hc:term:t1"),
          "Other ids are not writable")

    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.terminals = terms
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func writes(_ calls: [(method: String, params: [String: Any])]) -> [String] { calls.map(\.method).filter { ["terminal.run", "terminal.kill", "terminal.label", "doc.set"].contains($0) } }
    let ops: [(String, () async throws -> Void)] = [
        ("createTab", { try await hc.createTab(workspaceId: "hc:space:box/sp1", label: "x", cwd: nil) }),
        ("renameWorkspace", { try await hc.renameWorkspace(id: "hc:space:box/sp1", to: "x") }),
        ("closeWorkspace", { try await hc.closeWorkspace(id: "hc:space:box/sp1") }),
        ("closeTab", { try await hc.closeTab("hc:tab:box/sp1/tb1") }),
        ("moveTab", { try await hc.moveTab("hc:tab:box/sp1/tb1", toIndex: 0) }),
        ("renameTab", { try await hc.renameTab("hc:tab:box/sp1/tb1", to: "x") }),
        ("split", { try await hc.split(paneId: "hc:box/t3", direction: .right) }),
        ("renamePane", { try await hc.renamePane(paneId: "hc:box/t3", to: "x") }),
        ("closePane", { try await hc.closePane(paneId: "hc:box/t3") }),
    ]
    for (name, op) in ops {
        var threw = false
        let before = fake.calls.count
        do { try await op() } catch { threw = true }
        let w = writes(Array(fake.calls[before...]))
        check(threw && w.isEmpty, "\(name) on an unknown machine refuses and writes nothing (\(w))")
    }
    // Closing the local space never ends the remote terminal that carries the same label.
    let before = fake.calls.count
    try await hc.closeWorkspace(id: "hc:space:sp1")
    let killed = fake.calls[before...].filter { $0.method == "terminal.kill" }.compactMap { $0.params["terminal"] as? String }
    check(killed == ["t1"], "closing local sp1 kills only t1 (\(killed))")
}

/// #2: a remote child's `parent` names a terminal on the child's own machine. Ending local `t3` never ends `host2/t9`
/// (parent `t3` on host2), and ending `host2/t3` ends it.
@MainActor func checksHCLayoutParentMachine() async throws {
    let lab = ["herdr.space": "sp1", "herdr.tab": "tb1"]
    let terms = [FakeHC.terminal("t3", labels: lab, argv: ["zsh"]),
                 FakeHC.terminal("t5", argv: ["claude"], parent: "t3"),
                 FakeHC.terminal("host2/t3", labels: lab, argv: ["zsh"]),
                 FakeHC.terminal("host2/t9", argv: ["claude"], parent: "t3")]
    let local = HCClient.doomed(["t3"], terms: terms)
    check(local.sorted() == ["t3", "t5"], "ending local t3 ends its local child only (\(local))")
    let remote = HCClient.doomed(["host2/t3"], terms: terms)
    check(remote.sorted() == ["host2/t3", "host2/t9"], "ending host2/t3 ends host2/t9 (\(remote))")

    // Through closePane.
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.terminals = terms
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    try await hc.closePane(paneId: "hc:t3")
    let killed = fake.calls.filter { $0.method == "terminal.kill" }.compactMap { $0.params["terminal"] as? String }
    check(killed.sorted() == ["t3", "t5"], "closing pane t3 spares host2/t9 (\(killed))")
}

/// #3: close ends only the terminals its edit took out of the document tree, as the official app does, and a terminal
/// labeled with a space but no tab is adopted into a new tab with a fresh `tb…` id (window.rs adopt), never `tab == space`.
@MainActor func checksHCLayoutKillScope() async throws {
    func j(_ s: String) -> Any { try! JSONSerialization.jsonObject(with: Data(s.utf8)) }
    let lab = ["herdr.space": "sp1", "herdr.tab": "tb1"]
    let terms = [FakeHC.terminal("t1", labels: lab, argv: ["zsh"]),
                 FakeHC.terminal("t2", labels: lab, argv: ["zsh"]),          // labeled tb1, shown in tb2
                 FakeHC.terminal("t7", labels: ["herdr.space": "sp1"], argv: ["zsh"]),
                 FakeHC.terminal("t8", labels: ["herdr.space": "sp1"], argv: ["zsh"])]
    let doc = j(#"{"spaces":[{"id":"sp1","name":"w","folder":"/w","tabs":[{"id":"tb1","name":"one","root":{"id":"r1","type":"row","children":[{"id":"p1","type":"block","content":{"terminal":"t1"}}]}},{"id":"tb2","name":"two","root":{"id":"r2","type":"row","children":[{"id":"p2","type":"block","content":{"terminal":"t2"}}]}}]}]}"#)

    // Mapper: the document places a terminal before its labels; a tab-less one gets its own tab until adopted.
    let ws = HCMapper.workspaces(terminals: terms, appDoc: doc)
    let tabs = ws.first?.tabs ?? []
    check(tabs.map(\.id) == ["hc:tab:sp1/tb1", "hc:tab:sp1/tb2", "hc:tab:sp1/@t7", "hc:tab:sp1/@t8"], "tabs (\(tabs.map(\.id)))")
    check(tabs.count == 4 && tabs[1].panes.map(\.terminalId) == ["t2"] && tabs[0].panes.map(\.terminalId) == ["t1"],
          "t2 shows where the document puts it")

    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.terminals = terms
    fake.setDoc("herdr-app", doc)
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func kills(_ from: Int) -> [String] { fake.calls[from...].filter { $0.method == "terminal.kill" }.compactMap { $0.params["terminal"] as? String } }
    func parsed() -> HCAppDoc { (try? HCAppDoc(value: fake.doc("herdr-app"))) ?? (try! HCAppDoc(value: nil)) }

    var n = fake.calls.count
    try await hc.closeTab("hc:tab:sp1/tb1")
    check(kills(n) == ["t1"], "closing tb1 ends t1 only, not t2 shown in tb2 (\(kills(n)))")
    fake.terminals = fake.terminals.filter { $0["address"] as? String != "t1" }

    // The tab-less terminal: adopted into a fresh tab, so rename finds it and no tab is called by the space id.
    try await hc.renameTab("hc:tab:sp1/@t7", to: "seven")
    let d = parsed()
    let t7 = d.place(ofTerminal: "t7")
    check(t7 != nil && HCMapper.isFreshId(t7!.tab) && d.tabName(t7!.tab) == "seven", "t7 adopted into a fresh, renamed tab (\(String(describing: t7)))")
    check(!d.tabIds(space: "sp1").contains("sp1"), "no tab id equals the space id (\(d.tabIds(space: "sp1")))")
    let snap = try await hc.snapshot()
    check(snap.first?.tabs.contains { $0.label == "seven" && $0.panes.map(\.terminalId) == ["t7"] } == true, "the renamed tab shows t7")

    // A split of t7 labels the new terminal with t7's tab in the document.
    let t7tab = snap.first?.tabs.first { $0.label == "seven" }
    try await hc.split(paneId: "hc:t7", direction: .right)
    let run = fake.calls.last { $0.method == "terminal.run" }?.params["labels"] as? [String: String] ?? [:]
    check(run["herdr.tab"] == t7.map(\.tab) && run["herdr.space"] == "sp1", "split labels the new pane with its document tab (\(run))")
    _ = t7tab

    // A fresh core where t8 is not adopted yet: the split adopts it and labels the new pane with that same fresh tab.
    let f2 = FakeHC(path: fakeHCPath())
    try f2.start()
    defer { f2.kill() }
    f2.writable = true
    f2.terminals = [FakeHC.terminal("t8", labels: ["herdr.space": "sp1"], argv: ["zsh"])]
    f2.setDoc("herdr-app", j(#"{"spaces":[{"id":"sp1","name":"w","folder":"/w","tabs":[]}]}"#))
    let hc2 = HCClient(session: HCSession(socketPath: f2.path))
    try await hc2.split(paneId: "hc:t8", direction: .down)
    let run8 = f2.calls.last { $0.method == "terminal.run" }?.params["labels"] as? [String: String] ?? [:]
    let new8 = f2.terminals.last?["address"] as? String ?? "-"
    let d8 = (try? HCAppDoc(value: f2.doc("herdr-app"))) ?? (try! HCAppDoc(value: nil))
    let t8 = d8.place(ofTerminal: "t8")
    check(t8 != nil && run8["herdr.tab"] == t8?.tab && d8.place(ofTerminal: new8)?.tab == t8?.tab,
          "split of an unadopted pane: label, adoption and new pane share one tab (\(run8), \(String(describing: t8)))")
    let k0 = f2.calls.count
    try await hc2.closeTab("hc:tab:sp1/@t8")
    let k8 = f2.calls[k0...].filter { $0.method == "terminal.kill" }.compactMap { $0.params["terminal"] as? String }
    check(k8.sorted() == ["t8", new8].sorted(), "closing t8's tab ends its two panes only (\(k8))")

    n = fake.calls.count
    try await hc.closeTab("hc:tab:sp1/@t8")
    check(kills(n) == ["t8"], "closing t8's own tab ends t8 only (\(kills(n)))")
}

/// "Another writer" that leaves the document as it was (runs on FakeHC's thread: no actor).
private nonisolated func unchangedDoc(_ v: Any?) -> Any { v ?? NSNull() }

/// #4: a create or split whose document write fails ends the terminal it started and throws; a close whose kill fails
/// says so (the kept, labeled terminal would come back as an unnamed tab).
@MainActor func checksHCLayoutPartial() async throws {
    func j(_ s: String) -> Any { try! JSONSerialization.jsonObject(with: Data(s.utf8)) }
    let lab = ["herdr.space": "sp1", "herdr.tab": "tb1"]
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.terminals = [FakeHC.terminal("t1", labels: lab, argv: ["zsh"]), FakeHC.terminal("t2", labels: lab, argv: ["zsh"])]
    fake.setDoc("herdr-app", j(#"{"spaces":[{"id":"sp1","name":"w","folder":"/w","tabs":[{"id":"tb1","name":"","root":{"id":"r","type":"row","children":[{"id":"p1","type":"block","content":{"terminal":"t1"}},{"id":"p2","type":"block","content":{"terminal":"t2"}}]}}]}]}"#))
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    /// Every doc.set loses to "another writer" until the retries run out.
    func jam() { fake.conflicts = Array(repeating: unchangedDoc, count: 8) }
    let ops: [(String, () async throws -> Void)] = [
        ("createWorkspace", { _ = try await hc.createWorkspace(label: "x", cwd: "/tmp", focus: false) }),
        ("createTab", { try await hc.createTab(workspaceId: "hc:space:sp1", label: "x", cwd: "/tmp") }),
        ("split", { try await hc.split(paneId: "hc:t1", direction: .right) }),
    ]
    for (name, op) in ops {
        jam()
        let n = fake.calls.count
        var threw = false
        do { try await op() } catch { threw = true }
        let started = fake.calls[n...].filter { $0.method == "terminal.run" }.count
        let new = fake.terminals.map { $0["address"] as? String ?? "" }.filter { !["t1", "t2"].contains($0) }
        check(threw && started == 1 && new.isEmpty, "\(name) with a failed document write ends its new terminal and throws (left \(new))")
        fake.conflicts = []
    }

    // A kill that fails is reported, after the document change.
    fake.killFails = ["t2"]
    var msg = ""
    do { try await hc.closePane(paneId: "hc:t2") } catch { msg = "\(error)" }
    check(msg.contains("t2"), "closePane reports the terminal it could not end (\(msg))")
    msg = ""
    do { try await hc.closeTab("hc:tab:sp1/tb1") } catch { msg = "\(error)" }
    check(msg.contains("t2") && !msg.contains("t1"), "closeTab reports only the failed kill (\(msg))")
}

/// #5: Flockbook shows only the tabs with live terminals; the document also holds mod-only and empty tabs. A move by
/// shown index lands next to the shown neighbour in the document, and an unnamed tab is called by its place in the
/// document, as the official app does (model.rs tab_name).
@MainActor func checksHCLayoutMoveIndex() async throws {
    func j(_ s: String) -> Any { try! JSONSerialization.jsonObject(with: Data(s.utf8)) }
    func pane(_ t: String) -> String { #"{"id":"r\#(t)","type":"row","children":[{"id":"p\#(t)","type":"block","content":{"terminal":"\#(t)"}}]}"# }
    let doc = j(#"{"spaces":[{"id":"sp1","name":"w","folder":"/w","tabs":[{"id":"tbA","name":"","root":{"id":"rA","type":"block","content":{"mod":"notes"}}},{"id":"tb1","name":"","root":\#(pane("t1"))},{"id":"tbB","name":"","root":\#(pane("t9"))},{"id":"tb2","name":"","root":\#(pane("t2"))},{"id":"tb3","name":"","root":\#(pane("t3"))}]}]}"#)
    let lab = { (t: String) in ["herdr.space": "sp1", "herdr.tab": t] }
    let terms = [FakeHC.terminal("t1", labels: lab("tb1"), argv: ["zsh"]), FakeHC.terminal("t2", labels: lab("tb2"), argv: ["zsh"]),
                 FakeHC.terminal("t3", labels: lab("tb3"), argv: ["zsh"])]   // t9 is gone: tbB shows nothing live
    let shown = HCMapper.workspaces(terminals: terms, appDoc: doc).first?.tabs ?? []
    check(shown.map(\.label) == ["2", "4", "5"], "unnamed tabs are numbered by their document place (\(shown.map(\.label)))")

    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.terminals = terms
    fake.setDoc("herdr-app", doc)
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func order() -> [String] { ((try? HCAppDoc(value: fake.doc("herdr-app"))) ?? (try! HCAppDoc(value: nil))).tabIds(space: "sp1") }
    try await hc.moveTab("hc:tab:sp1/tb3", toIndex: 0)
    check(order() == ["tbA", "tb3", "tb1", "tbB", "tb2"], "move to shown 0 lands before tb1 (\(order()))")
    try await hc.moveTab("hc:tab:sp1/tb1", toIndex: 2)
    check(order() == ["tbA", "tb3", "tbB", "tb2", "tb1"], "move to the last shown place lands after tb2 (\(order()))")

    // With the official app running: tab.move gets the document place (1 = first).
    let dir = NSTemporaryDirectory() + "hkapp-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let log = dir + "/argv.log", cli = dir + "/herdr-app"
    try "#!/bin/sh\necho \"$@\" >> '\(log)'\ncase \"$1\" in status) exit 0;; do) echo '{}';; esac\n".write(toFile: cli, atomically: true, encoding: .utf8)
    chmod(cli, 0o755)
    await hc.setAppCLI(HerdrAppCLI(path: cli))
    try await hc.moveTab("hc:tab:sp1/tb1", toIndex: 0)   // shown: tb3 tb2 tb1 → before tb3, document place 2
    let line = ((try? String(contentsOfFile: log, encoding: .utf8)) ?? "").split(separator: "\n").last.map(String.init) ?? ""
    check(line.contains("tab.move") && line.contains("tb1") && (line.contains("\"index\":2") || line.contains("--index 2")),
          "official tab.move gets the document place (\(line))")
    await hc.setAppCLI(nil)
}

/// #6: `herdr-app do` JSON-parses `--k v` values (control.rs parse_args), so a name like "2" would go as a number.
/// Arguments go as one JSON object, which it takes as is.
@MainActor func checksHCLayoutAppArgs() async throws {
    let dir = NSTemporaryDirectory() + "hkapp-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let log = dir + "/argv.log", cli = dir + "/herdr-app"
    // One argument per line, so the check sees exactly what the app would get.
    try "#!/bin/sh\nprintf '%s\\n' \"$@\" '--' >> '\(log)'\ncase \"$1\" in status) exit 0;; do) echo '{}';; esac\n".write(toFile: cli, atomically: true, encoding: .utf8)
    chmod(cli, 0o755)
    let app = HerdrAppCLI(path: cli)
    for name in ["2", "true", "null", "{\"x\":1}", "say \"hi\""] {
        _ = try await app.do("tab.rename", ["id": "tb1", "name": name])
        let calls = ((try? String(contentsOfFile: log, encoding: .utf8)) ?? "").components(separatedBy: "--\n").filter { !$0.isEmpty }
        let argv = calls.last?.split(separator: "\n", omittingEmptySubsequences: false).map(String.init).filter { !$0.isEmpty } ?? []
        let obj = argv.count == 3 ? (try? JSONSerialization.jsonObject(with: Data(argv[2].utf8))) as? [String: Any] : nil
        check(argv.prefix(2) == ["do", "tab.rename"] && obj?["name"] as? String == name && obj?["id"] as? String == "tb1",
              "name \(name) goes as a JSON string in one object (\(argv))")
    }
    _ = try await app.do("tab.move", ["id": "tb1", "index": 3])
    _ = try await app.do("tab.close", ["id": "tb1", "confirmed": true])
    let calls = ((try? String(contentsOfFile: log, encoding: .utf8)) ?? "").components(separatedBy: "--\n").filter { !$0.isEmpty }
    let objs = calls.suffix(2).map { c -> [String: Any] in
        let a = c.split(separator: "\n").map(String.init)
        return a.count == 3 ? ((try? JSONSerialization.jsonObject(with: Data(a[2].utf8))) as? [String: Any] ?? [:]) : [:]
    }
    check((objs.first?["index"] as? NSNumber)?.intValue == 3 && objs.last?["confirmed"] as? Bool == true,
          "numbers and flags keep their JSON types (\(objs))")
}

/// #7: a pane marked `closable: false` is not closed (window.rs pane.close). While the official app runs, Flockbook
/// writes nothing behind it: a pane it has no command for (no pane id, or a rename) is refused.
@MainActor func checksHCLayoutAppOnly() async throws {
    func j(_ s: String) -> Any { try! JSONSerialization.jsonObject(with: Data(s.utf8)) }
    let lab = ["herdr.space": "sp1", "herdr.tab": "tb1"]
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.terminals = [FakeHC.terminal("t1", labels: lab, argv: ["zsh"]), FakeHC.terminal("t2", labels: lab, argv: ["zsh"]),
                      FakeHC.terminal("t5", argv: ["claude"], parent: "t1")]
    fake.setDoc("herdr-app", j(#"{"spaces":[{"id":"sp1","name":"w","folder":"/w","tabs":[{"id":"tb1","name":"","root":{"id":"r","type":"row","children":[{"id":"p1","type":"block","content":{"terminal":"t1"}},{"id":"p2","type":"block","content":{"terminal":"t2"},"closable":false}]}}]}]}"#))
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func effects(_ from: Int) -> [String] { fake.calls[from...].map(\.method).filter { ["terminal.run", "terminal.kill", "doc.set"].contains($0) } }

    var n = fake.calls.count
    var threw = false
    do { try await hc.closePane(paneId: "hc:t2") } catch { threw = true }
    check(threw && effects(n).isEmpty, "a closable:false pane stays (\(effects(n)))")

    let dir = NSTemporaryDirectory() + "hkapp-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let log = dir + "/argv.log", cli = dir + "/herdr-app"
    try "#!/bin/sh\necho \"$@\" >> '\(log)'\ncase \"$1\" in status) exit 0;; do) echo '{}';; esac\n".write(toFile: cli, atomically: true, encoding: .utf8)
    chmod(cli, 0o755)
    await hc.setAppCLI(HerdrAppCLI(path: cli))
    func dos() -> [String] { ((try? String(contentsOfFile: log, encoding: .utf8)) ?? "").split(separator: "\n").filter { $0.hasPrefix("do ") }.map(String.init) }
    let ops: [(String, () async throws -> Void)] = [
        ("split of a pane-less sub-agent", { try await hc.split(paneId: "hc:t5", direction: .right) }),
        ("close of a pane-less sub-agent", { try await hc.closePane(paneId: "hc:t5") }),
        ("rename", { try await hc.renamePane(paneId: "hc:t1", to: "x") }),
    ]
    for (name, op) in ops {
        n = fake.calls.count
        let d0 = dos().count
        threw = false
        do { try await op() } catch { threw = true }
        check(threw && effects(n).isEmpty && dos().count == d0, "app running, \(name): refused, nothing written (\(effects(n)), \(dos().suffix(1)))")
    }
    await hc.setAppCLI(nil)
}
