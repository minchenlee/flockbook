import Foundation
@testable import HerdrKit

// Swift twins of the dim "/" suggestion fix (issue #11, item 2): flockbook-modfeed `queue.rs` (`suggestion_in_paint`,
// `claude_slash_row`) and herdr-core `hc/observe.rs` (`first_paint`: watch only, detached at once, ends at its limit).
// FakeHC only.
@MainActor func checksDimSuggestion() async throws {
    // A first paint as Claude Code 2.1.322 gives it (spike 11-12, redacted): rule, box row, rule, footer; the cursor
    // last. Its empty-box placeholder is dim (SGR 2) with the cursor at the box start; typed text has a colour and the
    // cursor after it.
    let rule = String(repeating: "─", count: 40)
    func paint(_ row: String, _ col: Int) -> Data {
        Data("\u{1b}[2J\u{1b}[1;1H● Done.\u{1b}[3;1H\u{1b}[38;2;136;136;136m\(rule)\u{1b}[0m\u{1b}[4;1H\(row)\u{1b}[5;1H\(rule)\u{1b}[6;3H\u{1b}[2m? for shortcuts\u{1b}[0m\u{1b}[4;\(col)H".utf8)
    }
    let dim = "❯\u{a0}\u{1b}[0m\u{1b}[2m/clear\u{1b}[0m", typed = "❯\u{a0}\u{1b}[0m\u{1b}[38;2;177;185;249m/clear\u{1b}[0m"
    let s = AgentBox.suggestionInPaint
    check(s(paint(dim, 3), "/clear"), "dim: dim, cursor at the box start: a suggestion")
    check(!s(paint(typed, 9), "/clear"), "dim: typed: a colour, cursor after it")
    check(!s(paint(typed, 3), "/clear"), "dim: not dim, wherever the cursor is")
    check(!s(paint(dim, 9), "/clear"), "dim: dim but the cursor after it: no proof")
    check(!s(paint(dim, 3), "/compact"), "dim: another text than the screen's")
    check(!s(paint("❯\u{a0}\u{1b}[2m/cl\u{1b}[22mear", 3), "/clear"), "dim: only part dim")
    check(!s(paint("❯\u{a0}\u{1b}[38;2;2;2;2m/clear\u{1b}[0m", 3), "/clear"), "dim: a truecolor 2 is not dim")
    check(!s(paint("❯\u{a0}\u{1b}[38;5;2m/clear\u{1b}[0m", 3), "/clear"), "dim: a 256-colour 2 is not dim")
    check(s(paint("❯\u{a0}\u{1b}[0m\u{1b}[38;2;1;2;3;2m/clear\u{1b}[0m", 3), "/clear"), "dim: dim after a truecolor")
    check(!s(Data(), "/clear"), "dim: no paint")
    check(!s(paint("hello", 3), "/clear"), "dim: no box row")
    check(!s(paint(dim, 3), ""), "dim: nothing to match")
    // Codex R1: a paint cut inside a control sequence (its rest in a later DATA frame) is doubt: the cut sequence may
    // move the cursor or clear the row.
    for (cut, why) in [("\u{1b}[4;", "a CSI without its final byte"), ("\u{1b}", "a lone ESC"), ("\u{1b}]0;title", "an OSC without BEL or ST"),
                       ("\u{1b}]0;title\u{1b}", "an OSC cut inside its ST"), ("\u{1b}(", "a charset choice without its charset")] {
        check(!s(paint(dim, 3) + Data(cut.utf8), "/clear"), "dim: \(why) at the end: no proof")
    }
    check(s(paint(dim, 9) + Data("\u{1b}[4;3H".utf8), "/clear"), "dim: the same sequence whole: read")
    check(s(paint(dim, 9) + Data("\u{1b}[?25h\u{1b}[2 q\u{1b}[>4;2m\u{1b}[4;3H".utf8), "/clear"), "dim: private modes and a cursor style are passed over")
    // Codex R2: a number too big for a screen is doubt (no overflow, no "1" in its place).
    check(!s(paint(dim, 3) + Data("\u{1b}[4;2H\u{1b}[99999999999999999999C".utf8), "/clear"), "dim: a number too big to read is doubt")
    check(!s(paint(dim, 3) + Data("\u{1b}[4;2H\u{1b}[99999C".utf8), "/clear"), "dim: a move past any screen is doubt")

    func screen(_ row: String) -> String { "● Done.\n\n\(rule)\n\(row)\n\(rule)\n  ~/proj\n" }
    check(AgentBox.claudeSlashRow(screen("❯\u{a0}/clear")) == "/clear", "dim: the box's one \"/\" row")
    check(AgentBox.claudeSlashRow(screen("❯\u{a0}run the tests")) == nil, "dim: a row without \"/\": none")
    check(AgentBox.claudeSlashRow(screen("❯\u{a0}/clear\n  more")) == nil, "dim: two rows: none")
    check(AgentBox.claudeSlashRow(screen("❯\u{a0}")) == nil, "dim: an empty box: none")

    // MARK: hc/observe.rs: the look over hc only watches, detaches at once and keeps to its limit.
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    let typing: Set<String> = ["terminal.keys", "terminal.send"]
    fake.firstPaint = paint(dim, 3)
    let p = try? await hc.firstPaint(paneId: "hc:t1", limit: AgentBox.lookLimit)
    check(p == paint(dim, 3), "dim look: the first paint (\(p?.count ?? -1) bytes)")
    let attaches = fake.calls.filter { $0.method == "terminal.attach" }
    check(attaches.count == 1 && attaches.allSatisfy { $0.params["observe"] as? Bool == true && $0.params["terminal"] as? String == "t1" }, "dim look: one watch-only attach")
    try await Task.sleep(for: .milliseconds(150))
    check(fake.lastChannel > 0 && fake.closedByViewer(fake.lastChannel), "dim look: detached right after")
    check(!fake.calls.contains { typing.contains($0.method) }, "dim look: no keys, no text")

    // The paint's last sequence cut across two DATA frames, the rest later than the quiet time: the look gives the
    // first part, which must read as doubt.
    fake.firstPaint = paint(dim, 3) + Data("\u{1b}[4;".utf8)
    let late = Task.detached { try? await Task.sleep(for: .milliseconds(250)); fake.output(fake.lastChannel, Data("9H".utf8)) }
    let cutPaint = try? await hc.firstPaint(paneId: "hc:t1", limit: AgentBox.lookLimit)
    await late.value
    check(cutPaint != nil && !s(cutPaint ?? Data(), "/clear"), "dim look: a sequence cut across DATA frames: no proof (\(cutPaint?.count ?? -1) bytes)")
    fake.firstPaint = paint(dim, 3)

    // Codex R2: one read holds the whole dim paint and the start of the next frame (a redraw, maybe of typed text);
    // the reader hands out the paint and stalls. The frame part already read is not quiet.
    let redraw = HCFrame(channel: fake.lastChannel + 1, kind: .data, payload: Data("\u{1b}[4;3H\u{1b}[38;2;177;185;249m/clear\u{1b}[4;9H".utf8)).encoded()
    fake.paintTail = redraw.prefix(12)
    HCFramedConnection.pauseAfterData = 0.15
    let stalled = try? await hc.firstPaint(paneId: "hc:t1", limit: AgentBox.lookLimit)
    HCFramedConnection.pauseAfterData = 0
    fake.paintTail = Data()
    check(stalled.map { !s($0, "/clear") } ?? true, "dim look: a frame part in the same read as the paint is not quiet (\(stalled?.count ?? -1) bytes)")

    fake.attachError = "not_found"
    var t0 = ProcessInfo.processInfo.systemUptime
    var failed = (try? await hc.firstPaint(paneId: "hc:t1", limit: AgentBox.lookLimit)) == nil
    check(failed, "dim look: a refused attach is an error")
    fake.attachError = nil
    fake.firstPaint = Data()
    t0 = ProcessInfo.processInfo.systemUptime
    failed = (try? await hc.firstPaint(paneId: "hc:t1", limit: 0.3)) == nil
    check(failed && ProcessInfo.processInfo.systemUptime - t0 < 1.5, "dim look: no paint in time: an error at the limit (\(ProcessInfo.processInfo.systemUptime - t0) s)")
    fake.firstPaint = paint(dim, 3)
    fake.attachDelay = 2
    t0 = ProcessInfo.processInfo.systemUptime
    failed = (try? await hc.firstPaint(paneId: "hc:t1", limit: 0.3)) == nil
    check(failed && ProcessInfo.processInfo.systemUptime - t0 < 1.5, "dim look: a slow attach ends at the limit (\(ProcessInfo.processInfo.systemUptime - t0) s)")
    fake.attachDelay = 0
    check(!fake.calls.contains { typing.contains($0.method) }, "dim look: still no keys, no text")

    // Codex R2: a huge move must not overflow (it trapped the app). Last here: before the fix it ends the run.
    for big in ["\u{1b}[9223372036854775807B", "\u{1b}[9223372036854775807C", "\u{1b}[9223372036854775807;9223372036854775807H"] {
        check(!s(paint(dim, 3) + Data(big.utf8), "/clear"), "dim: a huge move does not overflow (\(big.dropFirst()))")
    }
}
