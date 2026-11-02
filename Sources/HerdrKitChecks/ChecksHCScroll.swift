import Foundation
@testable import HerdrKit

// Parity item 4: terminal history on scroll for hc panes. hc has no scroll position; `terminal.read {lines}` gives
// history above the screen as text. More lines than the screen: history to show; none and a program other than the
// shell in front: a full-screen program; else nothing to scroll.

@MainActor func checksHCScroll() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    let screen = (1...24).map { "s\($0)\n" }.joined()
    fake.terminals = [
        FakeHC.terminal("t3", argv: ["zsh"], pid: 30, fg: ["pid": 30, "name": "zsh", "cwd": "/w"]),
        FakeHC.terminal("t4", argv: ["zsh"], pid: 40, fg: ["pid": 41, "name": "less", "cwd": "/w"]),
        FakeHC.terminal("t5", argv: ["zsh"], pid: 50, fg: ["pid": 50, "name": "zsh", "cwd": "/w"]),
    ]
    for a in ["t3", "t4", "t5"] { fake.setScreen(a, screen) }
    fake.histories = ["t3": (1...100).map { "h\($0)\n" }.joined()]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    let src: any ScrollSource = hc

    let i3 = try await src.scrollInfo(paneId: "hc:t3")
    check(i3.kind == .history && i3.maxOffsetFromBottom == 100 && i3.viewportRows == 24, "hc scroll: history above the screen (\(i3))")
    let sb = try await src.readScrollback(paneId: "hc:t3", lines: Scrollback.readLimit, ansi: true, unwrapped: false)
    let read = fake.calls.last { $0.method == "terminal.read" }
    check(read?.params["lines"] as? Int == Scrollback.readLimit, "hc scroll: terminal.read asks for the lines (\(read?.params ?? [:]))")
    check(sb.lineCount == 124 && sb.text.hasPrefix("h1\r\nh2\r\n") && sb.text.hasSuffix("s24\r\n") && !sb.text.contains("\r\r"),
          "hc scroll: history then screen, CRLF for the terminal view (\(sb.lineCount) lines)")
    let i4 = try await src.scrollInfo(paneId: "hc:t4")
    check(i4.kind == .fullScreenProgram && i4.foregroundName == "less", "hc scroll: no history and less in front: full-screen program (\(i4))")
    let i5 = try await src.scrollInfo(paneId: "hc:t5")
    check(i5.kind == .none, "hc scroll: fresh shell: nothing to scroll (\(i5))")
}

/// Parity item 4: the terminal inside the tab screen takes typing on hc too (same path as the sheet: it watches until
/// the first key, then takes control). The wheel over a full-screen program goes to the program on the herdr TUI only.
@MainActor func checksHCInlineTerminal() {
    check(TerminalPolicy.writable(backend: .hc, writesEnabled: true) && TerminalPolicy.writable(backend: .tui, writesEnabled: true),
          "inline terminal: typing on both backends when writes are on")
    check(!TerminalPolicy.writable(backend: .hc, writesEnabled: false), "inline terminal: read-only when writes are off")
    check(TerminalPolicy.wheelToProgram(backend: .tui, writable: true) && !TerminalPolicy.wheelToProgram(backend: .hc, writable: true),
          "inline terminal: wheel to a full-screen program on the TUI only (hc: history view)")
    // while the terminal sheet is open, the hc terminal under it only watches (one controller at a time).
    check(!TerminalPolicy.writable(backend: .hc, writesEnabled: true, underSheet: true)
          && TerminalPolicy.writable(backend: .tui, writesEnabled: true, underSheet: true),
          "inline terminal: hc under an open sheet watches only")
    check(TerminalPolicy.viewKey(terminalId: "t1", backend: .hc, underSheet: true) != TerminalPolicy.viewKey(terminalId: "t1", backend: .hc, underSheet: false)
          && TerminalPolicy.viewKey(terminalId: "t1", backend: .tui, underSheet: true) == TerminalPolicy.viewKey(terminalId: "t1", backend: .tui, underSheet: false),
          "inline terminal: opening the sheet rebuilds the hc view as a fresh watcher (its controller is released)")
}
