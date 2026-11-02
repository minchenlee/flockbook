import Foundation

/// What the terminal view needs to scroll a pane, on either backend.
public protocol ScrollSource: Sendable {
    func scrollInfo(paneId: String) async throws -> ScrollInfo
    func readScrollback(paneId: String, lines: Int, ansi: Bool, unwrapped: Bool) async throws -> Scrollback
}

extension HerdrClient: ScrollSource {}

/// Terminal history on hc (parity item 4). hc keeps no scroll position: `terminal.read {lines}` returns the last `lines`
/// lines above the screen plus the screen, as plain text (LF, trailing blank rows dropped); `how` is `screen` when there
/// was nothing above it. Read-only: nothing is sent to the terminal.
extension HCClient: ScrollSource {
    /// Two read-only calls: `terminal.get` (rows, foreground) and `terminal.read`.
    public func scrollInfo(paneId: String) async throws -> ScrollInfo {
        let a = try address(paneId)
        let t = try await call("terminal.get", ["terminal": a])
        let r = try await call("terminal.read", ["terminal": a, "lines": Scrollback.readLimit])
        let rows = t["rows"] as? Int ?? 0
        let total = Self.lineCount(r["text"] as? String ?? "")
        let above = (r["how"] as? String ?? "screen") == "screen" ? 0 : max(1, total - rows)
        var info = ScrollInfo(maxOffsetFromBottom: above, offsetFromBottom: 0, viewportRows: rows)
        let fg = t["foreground"] as? [String: Any]
        info.foregroundName = fg?["name"] as? String
        if let fp = fg?["pid"] as? Int, let pid = t["pid"] as? Int { info.shellIsForeground = fp == pid }
        return info
    }

    /// History then screen, CRLF line ends for the terminal view. hc text has no colors (`ansi` is ignored) and no soft
    /// wraps to join (`unwrapped` is ignored). `truncated`: hc said the history is not complete.
    public func readScrollback(paneId: String, lines: Int, ansi: Bool, unwrapped: Bool) async throws -> Scrollback {
        let r = try await call("terminal.read", ["terminal": try address(paneId), "lines": max(1, lines)])
        guard let text = r["text"] as? String else { throw HerdrError.protocolError("terminal.read has no text") }
        let crlf = text.replacingOccurrences(of: "\r\n", with: "\n").replacingOccurrences(of: "\n", with: "\r\n")
        return Scrollback(text: crlf, truncated: !(r["complete"] as? Bool ?? true), revision: 0, lineCount: Self.lineCount(text))
    }

    static func lineCount(_ text: String) -> Int {
        var n = text.utf8.reduce(0) { $1 == 0x0A ? $0 + 1 : $0 }
        if let last = text.utf8.last, last != 0x0A { n += 1 }
        return n
    }
}

/// Who may type into a terminal view and where the wheel goes, the same in the sheet and in the tab screen.
public enum TerminalPolicy {
    /// Typing is allowed whenever writes are on, on both backends. hc: the view watches (never resizes the user's
    /// terminal) until the first key, then takes control with its real size (`TerminalSession`).
    /// `underSheet`: the terminal sheet is open over this view. On hc the sheet is then the one controller and this
    /// view watches only, so the two never take the terminal from each other.
    public static func writable(backend: BackendKind, writesEnabled: Bool, underSheet: Bool = false) -> Bool {
        writesEnabled && !(backend == .hc && underSheet)
    }

    /// The view's identity: on hc it changes when the sheet opens or closes, so the view is made again with a fresh
    /// session (a controller it held is released; it watches until the next key).
    public static func viewKey(terminalId: String, backend: BackendKind, underSheet: Bool) -> String {
        backend == .hc && underSheet ? terminalId + "\u{0}under-sheet" : terminalId
    }

    /// The wheel over a full-screen program goes to the program: herdr TUI only. hc streams raw bytes and the app
    /// cannot see the program's mouse mode, so on hc the wheel opens the history view instead.
    public static func wheelToProgram(backend: BackendKind, writable: Bool) -> Bool { writable && backend == .tui }
}

/// Tabs that became blocked since the last look (the app's system notifications). Backend-free: TUI and hc tabs alike.
public enum BlockedTabs {
    /// `known`: the blocked tab ids last time (nil: first look, nothing is fresh). `now`: to keep for next time.
    public static func fresh(known: Set<String>?, in workspaces: [Workspace]) -> (now: Set<String>, fresh: [(Workspace, Tab)]) {
        var now: Set<String> = []
        var fresh: [(Workspace, Tab)] = []
        for ws in workspaces { for t in ws.tabs where t.status == .blocked {
            now.insert(t.id)
            if let known, !known.contains(t.id) { fresh.append((ws, t)) }
        } }
        return (now, fresh)
    }
}
