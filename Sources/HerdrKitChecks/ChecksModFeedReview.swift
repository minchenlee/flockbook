import Foundation
@testable import HerdrKit
import ModFeedKit

// Review 1 of the herdr-app mod's sidecar: an approval binds to the box on screen, never to a lagging log's call.
func checksModFeedReview() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func keys() -> [[String]] { fake.calls.filter { $0.method == "terminal.keys" }.compactMap { $0.params["keys"] as? [String] } }
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("fb-review-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    func log(_ name: String, _ lines: [String]) throws -> URL {
        let u = dir.appendingPathComponent(name)
        try (lines.joined(separator: "\n") + "\n").write(to: u, atomically: true, encoding: .utf8)
        return u
    }
    func tool(_ id: String, _ call: String, _ cmd: String) -> String {
        #"{"type":"assistant","uuid":"\#(id)","timestamp":"2026-11-03T01:40:00.000Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"\#(call)","name":"Bash","input":{"command":"\#(cmd)"}}]}}"#
    }
    func claudeBox(_ cmd: String) -> String {
        "✻ Working… 3s\n────────────────────\n Bash command\n Run it\n╌╌╌╌╌╌╌╌╌╌\n \(cmd)\n╌╌╌╌╌╌╌╌╌╌\n Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel"
    }

    // 1. The log still names call A (its result not written yet); the screen already shows B's box.
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission")]
    let pane = FeedLocate.pane(try await hc.snapshot(), terminal: "t1")!
    let lagging = try log("a.jsonl", [tool("a1", "toolu_A", "ls -la")])
    fake.setScreen("t1", claudeBox("rm -rf build"))
    let card = await FeedCards.card(pane: pane, screen: hc, log: lagging)
    check(card.kind == "approval" && card.request != "toolu_A" && card.command?.contains("rm -rf build") == true,
          "review1 #1: log names A, screen shows B: the card is B, not A's id (\(card))")
    let k0 = keys().count
    let stale = await FeedCards.approve(pane: pane, screen: hc, log: lagging, request: "toolu_A", allow: true)
    check(stale.result == "changed" && keys().count == k0, "review1 #1: approve with A's id while B is on screen: nothing sent (\(stale))")
    // The log caught up: the same box is the log's call, its id is the card's.
    let caught = try log("b.jsonl", [tool("a2", "toolu_B", "rm -rf build")])
    let cb = await FeedCards.card(pane: pane, screen: hc, log: caught)
    check(cb.request == "toolu_B", "review1 #1: log and box are the same call: the log's id (\(cb.request ?? "nil"))")

    // The screen id covers the command: two boxes that differ only in the command get two ids.
    let r1 = await FeedCards.card(pane: pane, screen: hc, log: nil).request
    fake.setScreen("t1", claudeBox("ls"))
    let r2 = await FeedCards.card(pane: pane, screen: hc, log: nil).request
    check(r1 != nil && r2 != nil && r1 != r2, "review1 #1: two commands, two screen ids (\(r1 ?? "nil"), \(r2 ?? "nil"))")
    // An id read for one box does not answer the next one.
    let k1 = keys().count
    let next = await FeedCards.approve(pane: pane, screen: hc, log: nil, request: r1 ?? "", allow: true)
    check(next.result == "changed" && keys().count == k1, "review1 #1: the id of the box before does not answer this box (\(next))")

    // Grok: two boxes with different commands, two ids; Allow goes through Grok's own keys (Enter on "Yes, proceed").
    func grok(_ command: String, focus: Int = 2) -> String {
        let rows = ["Yes, and don't ask again for anything (always-approve mode)", "Yes, proceed", "No, reject (type to add feedback)", "Never allow: echo"]
        var l = ["  ❯ Write a file.", "", "  ◆ Working… 3.0s", "", "  ┃", "  ┃  ›  Run a command", "  ┃  │ \(command)", "  ┃  ← → narrow scope", "  ┃"]
        for (i, r) in rows.enumerated() { l.append("  ┃  \(i + 1) (\(i + 1 == focus ? "●" : "○")) \(r)") }
        l += ["  ┃", "", "  1/4:select  │  Tab:next option  │  ←/→:scope  │  Ctrl+o:always-approve  │  Ctrl+c:cancel  │  Esc:scrollback"]
        return l.joined(separator: "\n")
    }
    fake.terminals = [FakeHC.terminal("t2", argv: ["grok"], status: "blocked", kind: "permission")]
    let gpane = FeedLocate.pane(try await hc.snapshot(), terminal: "t2")!
    fake.setScreen("t2", grok("echo one"))
    let g1 = await FeedCards.card(pane: gpane, screen: hc, log: nil).request
    fake.setScreen("t2", grok("echo two"))
    let g2 = await FeedCards.card(pane: gpane, screen: hc, log: nil).request
    check(g1 != nil && g2 != nil && g1 != g2, "review1 #1: grok, two commands, two ids (\(g1 ?? "nil"), \(g2 ?? "nil"))")
    let k2 = keys().count
    fake.onKeys = { a, k in if a == "t2", k == ["enter"] || k == ["Enter"] { fake.setScreen("t2", "  ❯ ") } }
    let ga = await FeedCards.approve(pane: gpane, screen: hc, log: nil, request: g2 ?? "", allow: true)
    fake.onKeys = nil
    let sent = Array(keys().dropFirst(k2))
    check(ga.result == "sent" && sent.count == 1 && sent.first?.first?.lowercased() == "enter",
          "review1 #1: grok allow is Enter on Yes, proceed, not '1' (\(ga), \(sent))")

    // 2. One approval goes out once. Each sidecar run starts from nothing, so the claim lives next to the cache file.
    let cache = dir.appendingPathComponent("cache.json")
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission")]
    fake.setScreen("t1", claudeBox("rm -rf build"))
    let id2 = await FeedCards.card(pane: pane, screen: hc, log: nil, cache: cache).request ?? ""
    // hc holds the key for its typing-quiet rule, past the sidecar's call timeout: the key may still land.
    let slow = HCClient(session: HCSession(socketPath: fake.path), callTimeout: 1, sendTimeout: 2)
    fake.keysDelay = 2
    let k3 = keys().count
    let held = await FeedCards.approve(pane: pane, screen: slow, log: nil, request: id2, allow: true, cache: cache)
    fake.keysDelay = 0
    check(held.result == "unconfirmed", "review1 #2: a held key is unconfirmed, not failed (\(held))")
    let again = await FeedCards.approve(pane: pane, screen: hc, log: nil, request: id2, allow: true, cache: cache)
    check(again.result == "answered" && keys().count == k3 + 1, "review1 #2: a second press of the same approval sends nothing (\(again), \(keys().suffix(2)))")
    let shown = await FeedCards.card(pane: pane, screen: hc, log: nil, cache: cache)
    check(shown.request == id2 && shown.answered == true, "review1 #2: the card says it was answered, while the box is still there (\(shown))")
    // The box leaves the screen: the claim goes; the same box later is a new prompt and can be answered.
    fake.setScreen("t1", claudeBox("ls"))
    _ = await FeedCards.card(pane: pane, screen: hc, log: nil, cache: cache)
    fake.setScreen("t1", claudeBox("rm -rf build"))
    let k4 = keys().count
    let later = await FeedCards.approve(pane: pane, screen: hc, log: nil, request: id2, allow: true, cache: cache)
    check(later.result == "sent" && keys().count == k4 + 1, "review1 #2: after the box went away, the same box can be answered again (\(later))")
    // Redraw lag: sent, the screen not redrawn yet; a second press (or a second view) sends nothing.
    let lag = await FeedCards.approve(pane: pane, screen: hc, log: nil, request: id2, allow: true, cache: cache)
    check(lag.result == "answered" && keys().count == k4 + 1, "review1 #2: sent, box still drawn: the next press sends nothing (\(lag))")
    // Without a cache file (no claim possible) the old behaviour stays: one key per press.

    // 3. A Claude conversation went on in a new log (the old one ends with `continued-in`; herdr keeps the old id).
    // A cache hit must give the new log too, not the old, frozen one, so the chat does not flip between them.
    let proj = dir.appendingPathComponent("projects/-w-proj")
    try FileManager.default.createDirectory(at: proj, withIntermediateDirectories: true)
    let oldLog = proj.appendingPathComponent("old1.jsonl"), newLog = proj.appendingPathComponent("new1.jsonl")
    let lead = #"{"type":"user","uuid":"l1","sessionId":"s","message":{"role":"user","content":"start"}}"#
    try (lead + "\n" + #"{"type":"continued-in","timestamp":"2026-11-03T06:54:55.989Z","sessionId":"old1","continuedInSessionId":"new1"}"# + "\n")
        .write(to: oldLog, atomically: true, encoding: .utf8)
    try (lead + "\n").write(to: newLog, atomically: true, encoding: .utf8)
    fake.terminals = [FakeHC.terminal("t3", status: "working")]
    let ws3 = try await hc.snapshot()
    let p3 = FeedLocate.pane(ws3, terminal: "t3")!
    let cache3 = dir.appendingPathComponent("locate/cache.json")
    var c3 = FeedLocate.Cache()
    c3.entries[p3.terminalId] = FeedLocate.Entry(kind: "claude", cwd: p3.cwd, pids: p3.processIds, path: oldLog.path, checked: Date().timeIntervalSince1970)
    FeedLocate.writeCache(c3, to: cache3)
    let hit = try await FeedLocate.find(terminal: "t3", workspaces: ws3, resolver: SessionResolver(socketPath: "/nonexistent.sock"), cacheURL: cache3)
    check(hit.cached && hit.path == newLog.path, "review1 #3: a cache hit follows continued-in to the new log (\(hit.path ?? "nil"))")
}
