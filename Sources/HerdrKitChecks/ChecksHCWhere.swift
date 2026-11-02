import Foundation
import HerdrKit

/// The `where` label (`<space> / <tab>`) the official app writes on every terminal it shows (window.rs write_where):
/// Flockbook writes it too after each layout write, follows renames and moves, gives sub-agents their parent's, and
/// writes only labels that changed.
@MainActor func checksHCWhere() async throws {
    func j(_ s: String) -> Any { try! JSONSerialization.jsonObject(with: Data(s.utf8)) }
    let home = NSHomeDirectory()
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.writable = true
    fake.terminals = [
        FakeHC.terminal("t1", labels: ["herdr.space": "sp1", "herdr.tab": "tb1"], argv: ["zsh"], cwd: "/w/proj/sub"),
        FakeHC.terminal("t2", labels: ["herdr.space": "sp2", "herdr.tab": "tb2"], argv: ["zsh"], cwd: home),
        FakeHC.terminal("t9", argv: ["claude"], cwd: "/w/proj", parent: "t1"),
        FakeHC.terminal("box/t7", labels: ["herdr.space": "spR", "herdr.tab": "tbR"], argv: ["zsh"]),
        FakeHC.terminal("box/t8", argv: ["claude"], parent: "t1"),
    ]
    fake.setDoc("herdr-app", j(#"""
    {"spaces":[
      {"id":"sp1","name":"","folder":"/w","tabs":[{"id":"tb1","name":"","root":{"id":"r1","type":"row","children":[{"id":"p1","type":"block","content":{"terminal":"t1"}}]}}]},
      {"id":"sp2","name":"","folder":"/elsewhere","tabs":[{"id":"tb2","name":"","root":{"id":"r2","type":"row","children":[{"id":"p2","type":"block","content":{"terminal":"t2"}}]}}]}
    ]}
    """#))
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    // An unnamed space is called after its first terminal's git checkout top (window.rs folder_info), else its folder;
    // home is `~` without asking git.
    let asked = Asked()
    await hc.setGitTop { cwd in asked.add(cwd); return cwd.hasPrefix("/w/proj") ? "/w/proj" : nil }
    func wheres() -> [String: String] {
        var o: [String: String] = [:]
        for t in fake.terminals {
            if let a = t["address"] as? String, let w = (t["labels"] as? [String: String])?["where"] { o[a] = w }
        }
        return o
    }
    func labeled(_ from: Int) -> [String] {
        fake.calls[from...].filter { $0.method == "terminal.label" }.compactMap { $0.params["terminal"] as? String }.sorted()
    }

    // split: the new terminal and the rest get `<space> / <tab>`; an unnamed space is named after its first terminal's
    // folder (home is `~`), an unnamed tab by its place; a sub-agent gets its parent's; a remote terminal nothing.
    try await hc.split(paneId: "hc:t2", direction: .right)
    var w = wheres()
    let t3 = fake.terminals.compactMap { $0["address"] as? String }.first { !["t1", "t2", "t9", "box/t7", "box/t8"].contains($0) } ?? "?"
    check(w["t1"] == "proj / 1" && w["t2"] == "~ / 1" && w[t3] == "~ / 1", "where: split labels every shown terminal (\(w))")
    check(!asked.list.contains(home), "where: home is ~ without asking git (\(asked.list))")
    check(w["t9"] == "proj / 1", "where: a sub-agent gets its parent's place (\(w["t9"] ?? "nil"))")
    check(w["box/t7"] == nil && w["box/t8"] == nil, "where: a remote terminal is not labeled, even one whose parent is t1 there (\(w))")

    // rename: follows; only the changed ones are written.
    var n = fake.calls.count
    try await hc.renameWorkspace(id: "hc:space:sp2", to: "Flock")
    w = wheres()
    check(w["t2"] == "Flock / 1" && w[t3] == "Flock / 1", "where: a space rename follows (\(w))")
    check(labeled(n) == [t3, "t2"].sorted(), "where: only changed labels are written (\(labeled(n)))")

    // new tab: its terminal gets the tab's name.
    try await hc.createTab(workspaceId: "hc:space:sp2", label: "logs", cwd: nil)
    w = wheres()
    let t4 = fake.terminals.compactMap { $0["address"] as? String }.first { !["t1", "t2", "t9", "box/t7", "box/t8", t3].contains($0) } ?? "?"
    check(w[t4] == "Flock / logs", "where: a new tab's terminal (\(w[t4] ?? "nil"))")

    // move: the unnamed tab's number follows its new place.
    try await hc.moveTab("hc:tab:sp2/\(fake.tabId(of: t4) ?? "?")", toIndex: 0)
    w = wheres()
    check(w["t2"] == "Flock / 2" && w[t3] == "Flock / 2" && w[t4] == "Flock / logs", "where: a tab move renumbers (\(w))")

    // tab rename: only that tab's terminal.
    n = fake.calls.count
    try await hc.renameTab("hc:tab:sp2/\(fake.tabId(of: t4) ?? "?")", to: "notes")
    check(wheres()[t4] == "Flock / notes" && labeled(n) == [t4], "where: a tab rename writes only its terminals (\(labeled(n)))")
}

private final class Asked: @unchecked Sendable {
    private let lock = NSLock()
    private var _list: [String] = []
    func add(_ s: String) { lock.withLock { _list.append(s) } }
    var list: [String] { lock.withLock { _list } }
}

/// The real lookup: a folder inside this checkout gives the checkout's top; a folder outside any gives nil.
func checksHCGitTop() {
    let repo = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().path
    let top = HCClient.gitTopLevel(repo + "/Sources")
    check(top.map { URL(fileURLWithPath: $0).resolvingSymlinksInPath().path } == URL(fileURLWithPath: repo).resolvingSymlinksInPath().path,
          "where: git top of Sources is the checkout (\(top ?? "nil"))")
    check(HCClient.gitTopLevel("/") == nil, "where: / is in no checkout")
}

extension FakeHC {
    /// The `herdr.tab` label of a terminal.
    func tabId(of address: String) -> String? {
        terminals.first { $0["address"] as? String == address }.flatMap { ($0["labels"] as? [String: String])?["herdr.tab"] }
    }
}
