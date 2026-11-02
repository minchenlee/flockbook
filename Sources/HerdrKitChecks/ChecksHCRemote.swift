import Foundation
@testable import HerdrKit

// Parity item 3: terminals on other machines (`hc machine add`). Each machine keeps its own `herdr-app` document
// (`doc.get {on}`); its ids and labels are local to that machine. The official app shows them qualified (`box/sp1`)
// and leaves temporary machines out (herdr-app 0c1a102 mirror.rs qualify/local, window.rs documents_updated).

private func json(_ s: String) -> Any { try! JSONSerialization.jsonObject(with: Data(s.utf8)) }

/// One space `sp1` with tab `tb1` holding the given terminals, in that order, with pane titles.
private func oneSpace(name: String, tab: String, terms: [(String, String)]) -> Any {
    let kids = terms.enumerated().map { i, t in
        #"{"id":"p\#(i)","type":"block","content":{"terminal":"\#(t.0)"},"title":"\#(t.1)"}"#
    }.joined(separator: ",")
    return json(#"{"spaces":[{"id":"sp1","name":"\#(name)","folder":"/w","tabs":[{"id":"tb1","name":"\#(tab)","root":{"id":"r","type":"row","children":[\#(kids)]}}]}]}"#)
}

@MainActor func checksHCRemoteLayoutRead() async throws {
    let lab = ["herdr.space": "sp1", "herdr.tab": "tb1"]
    let terms = [
        FakeHC.terminal("t1", labels: lab, argv: ["zsh"], pid: 11),
        FakeHC.terminal("box/t3", labels: lab, argv: ["zsh"], pid: 33),
        FakeHC.terminal("box/t4", labels: lab, argv: ["zsh"], pid: 44),
    ]
    let local = oneSpace(name: "Here", tab: "main", terms: [("t1", "")])
    let remote = oneSpace(name: "Remote", tab: "build", terms: [("t4", "first"), ("t3", "second")])

    // 1. Mapper: the same local ids on two machines are two spaces; the remote one takes its own document.
    let ws = HCMapper.workspaces(terminals: terms, appDoc: local, machineDocs: [("box", remote)])
    check(ws.map(\.id) == ["hc:space:sp1", "hc:space:box/sp1"], "remote: one space per machine, qualified (\(ws.map(\.id)))")
    check(ws.map(\.label) == ["Here", "Remote"], "remote: each space named from its own machine's document (\(ws.map(\.label)))")
    let rt = ws.last?.tabs.first
    check(rt?.id == "hc:tab:box/sp1/tb1" && rt?.label == "build", "remote: tab id qualified once, named (\(rt?.id ?? "-"))")
    check(rt?.panes.map(\.terminalId) == ["box/t4", "box/t3"], "remote: pane order from the remote document (\(rt?.panes.map(\.terminalId) ?? []))")
    check(rt?.panes.map(\.label) == ["first", "second"], "remote: pane titles from the remote document")
    check(ws.first?.tabs.first?.panes.map(\.terminalId) == ["t1"], "remote: the local space keeps only local terminals")
    let parsed = try HCClient.tabLabels("hc:tab:box/sp1/tb1")
    check(parsed.space == "box/sp1" && parsed.tab == "box/tb1", "remote: tab id parses to qualified space and tab (\(parsed))")
    let lp = try HCClient.tabLabels("hc:tab:sp1/tb1")
    check(lp.space == "sp1" && lp.tab == "tb1", "remote: a local tab id parses as before")

    // Without the remote document the remote terminals still group by their (qualified) labels.
    let bare = HCMapper.workspaces(terminals: terms, appDoc: local)
    check(bare.map(\.id) == ["hc:space:sp1", "hc:space:box/sp1"], "remote: labels alone keep machines apart (\(bare.map(\.id)))")

    // 2. Client: reads each connected, kept machine's document; never a temporary or disconnected one.
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = terms
    fake.setDoc("herdr-app", local)
    fake.setDoc("box/herdr-app", remote)
    fake.machines = [FakeHC.machine("box"), FakeHC.machine("tmp", temporary: true), FakeHC.machine("off", state: "unreachable")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    let snap = try await hc.snapshot()
    check(snap.map(\.label) == ["Here", "Remote"], "remote client: remote space named from doc.get {on: box} (\(snap.map(\.label)))")
    let ons = fake.calls.filter { $0.method == "doc.get" }.compactMap { $0.params["on"] as? String }
    check(ons == ["box"], "remote client: only box's document is read (\(ons))")

    // A machine's document that cannot be read does not hide the others.
    fake.setDoc("box/herdr-app", json(#"{"spaces":{"bad":1}}"#))
    await hc.markDocStale()
    let snap2 = try await hc.snapshot()
    check(snap2.first?.label == "Here" && snap2.count == 2, "remote client: an unknown remote shape falls back to labels (\(snap2.map(\.label)))")
}

/// a connected but slow machine does not hold up this machine's snapshot. Its document is read in the
/// background (every machine at once) and shows on a later snapshot.
@MainActor func checksHCRemoteSlow() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    let lab = ["herdr.space": "sp1", "herdr.tab": "tb1"]
    fake.machines = [FakeHC.machine("box"), FakeHC.machine("cube")]
    fake.terminals = [FakeHC.terminal("t1", labels: lab, argv: ["zsh"]), FakeHC.terminal("box/t3", labels: lab, argv: ["zsh"]),
                      FakeHC.terminal("cube/t5", labels: lab, argv: ["zsh"])]
    fake.setDoc("herdr-app", oneSpace(name: "Here", tab: "main", terms: [("t1", "")]))
    fake.setDoc("box/herdr-app", oneSpace(name: "Slow", tab: "b", terms: [("t3", "")]))
    fake.setDoc("cube/herdr-app", oneSpace(name: "Cube", tab: "c", terms: [("t5", "")]))
    fake.docDelays = ["box/herdr-app": 1.2, "cube/herdr-app": 0.9]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    let t0 = Date()
    let first = try await hc.snapshot()
    let took = Date().timeIntervalSince(t0)
    check(took < 0.7 && first.first?.label == "Here", "remote slow: the local snapshot does not wait for slow machines (\(String(format: "%.2f", took)) s)")
    try await Task.sleep(nanoseconds: 1_500_000_000)
    let later = try await hc.snapshot()
    check(later.map(\.label) == ["Here", "Slow", "Cube"],
          "remote slow: machines read at once, their spaces show on a later snapshot (\(later.map(\.label)))")
}

/// a failed read of a machine's document (or of `machine.list`) keeps the last good one, so its tabs do
/// not jump to label grouping. A stale mark (reconnect, doc event) starts a new read even while an old one runs.
@MainActor func checksHCRemoteDocKeep() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    let lab = ["herdr.space": "sp1", "herdr.tab": "tb1"]
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("t1", labels: lab, argv: ["zsh"]), FakeHC.terminal("box/t3", labels: lab, argv: ["zsh"])]
    fake.setDoc("herdr-app", oneSpace(name: "Here", tab: "main", terms: [("t1", "")]))
    fake.setDoc("box/herdr-app", oneSpace(name: "Box", tab: "b", terms: [("t3", "")]))
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func labels() async throws -> [String] {
        _ = try await hc.snapshot()
        try await Task.sleep(nanoseconds: 300_000_000)   // the background read lands
        return try await hc.snapshot().map(\.label)
    }
    check(try await labels() == ["Here", "Box"], "doc keep: box's space from its document")
    fake.docFails = ["box/herdr-app"]
    await hc.markDocStale()
    let failed = try await labels()
    check(failed == ["Here", "Box"], "doc keep: a failed doc.get on box keeps its last document (\(failed))")
    fake.docFails = []; fake.machineListFails = true
    await hc.markDocStale()
    let noList = try await labels()
    check(noList == ["Here", "Box"], "doc keep: a failed machine.list keeps the documents (\(noList))")
    fake.machineListFails = false
    fake.machines = [FakeHC.machine("box", state: "unreachable")]
    await hc.markDocStale()
    let gone = try await labels()
    check(gone.count == 2 && !gone.contains("Box"), "doc keep: a machine that is not connected loses its document (\(gone))")

    // A slow read under way; then a stale mark with a quick machine: the new read shows at once.
    fake.machines = [FakeHC.machine("box")]
    fake.docDelays = ["box/herdr-app": 1.5]
    await hc.markDocStale()
    _ = try await hc.snapshot()                          // starts the slow read
    try await Task.sleep(nanoseconds: 100_000_000)
    fake.docDelays = [:]
    fake.setDoc("box/herdr-app", oneSpace(name: "Box2", tab: "b", terms: [("t3", "")]))
    await hc.markDocStale()
    let quick = try await hc.snapshot().map(\.label)
    check(quick == ["Here", "Box2"], "doc keep: a stale mark reads again even while an old read runs (\(quick))")
}
