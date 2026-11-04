import Foundation

/// Settings > General > Backend: which runtimes the app follows. Stored in the app's defaults under `key`.
public enum BackendChoice: String, CaseIterable, Sendable, Identifiable {
    /// Both are tried; only the ones that answer show. A silent one shows no problem while the other answers.
    case automatic
    /// herdr-core only: the herdr TUI is never touched (no snapshot, no diagnose, no socket, no `herdr` program).
    case hcOnly = "hc"
    /// The herdr TUI only (same as `--no-hc`).
    case tuiOnly = "tui"
    /// Both, and the problems of both show (the behaviour before this setting).
    case both

    public static let key = "backend"
    public var id: String { rawValue }

    /// The stored value; nothing or an unknown value is Automatic.
    public static func read(_ raw: String?) -> BackendChoice { raw.flatMap(BackendChoice.init(rawValue:)) ?? .automatic }

    public var label: String {
        switch self {
        case .automatic: "Automatic"
        case .hcOnly: "herdr-core only"
        case .tuiOnly: "Herdr TUI only"
        case .both: "Both"
        }
    }
}

/// What the store runs, made from the choice and the launch flags (`make`). Pure: checks cover every case.
public struct BackendPlan: Sendable, Equatable {
    /// Run the herdr TUI loop (snapshots, events, diagnose).
    public var tui: Bool
    /// Run the herdr-core loop.
    public var hc: Bool
    /// A side that does not answer is hidden (no problem, no diagnose, slow retries) while the other answers.
    public var quiet: Bool
    /// Launch flags decided part of this plan (`--no-hc`, `--hc-socket`, `--session`, `--remote`).
    public var flags = false
    /// The session is on another machine: its herdr TUI always runs (herdr-core only applies to this Mac).
    public var remote = false

    public init(tui: Bool, hc: Bool, quiet: Bool, flags: Bool = false, remote: Bool = false) {
        self.tui = tui; self.hc = hc; self.quiet = quiet; self.flags = flags; self.remote = remote
    }

    /// The plan before this setting existed: both, every problem reported.
    public static let both = BackendPlan(tui: true, hc: true, quiet: false)

    /// Launch flags win over `choice`: `--no-hc` is the TUI only; `--hc-socket` keeps hc; `--session`, `--remote`
    /// (and their environment variables) keep the TUI. A session on another machine keeps the TUI. Headless runs
    /// follow hc only with `--hc-socket` (`HCSession.wanted`). Nothing left to run: the TUI.
    public static func make(choice: BackendChoice, arguments args: [String], environment env: [String: String],
                            headless: Bool, remote: Bool) -> BackendPlan {
        var tui: Bool, hc: Bool, quiet: Bool
        switch choice {
        case .automatic: (tui, hc, quiet) = (true, true, true)
        case .hcOnly: (tui, hc, quiet) = (false, true, true)
        case .tuiOnly: (tui, hc, quiet) = (true, false, true)
        case .both: (tui, hc, quiet) = (true, true, false)
        }
        let noHC = args.contains("--no-hc"), hcSocket = args.contains("--hc-socket")
        let tuiFlags = RemoteMachines.launchFlags(arguments: args, environment: env)
        if noHC { tui = true; hc = false }
        if hcSocket, !noHC { hc = true }
        if tuiFlags || remote { tui = true }
        if !HCSession.wanted(arguments: args, headless: headless) { hc = false }
        if !tui && !hc { tui = true }
        return BackendPlan(tui: tui, hc: hc, quiet: quiet, flags: noHC || hcSocket || tuiFlags, remote: remote)
    }

    public enum Down: Sendable, Equatable {
        /// Show nothing about it and do not diagnose; retry slowly (`retry(after:hidden:)`).
        case hide
        /// Diagnose and show the problem; reconnect at the usual pace.
        case report
    }

    /// After a failed TUI snapshot.
    public func tuiDown(hcConnected: Bool) -> Down { quiet && hc && hcConnected ? .hide : .report }
    /// After hc did not answer.
    public func hcDown(tuiConnected: Bool) -> Down { quiet && tui && tuiConnected ? .hide : .report }
    /// hc's own problem (`hcNotRunning`) shows. Automatic with the TUI shows the TUI's problem only, as before.
    public var showsHCProblem: Bool { !(quiet && tui) }

    /// The wait before the next connect. Hidden: from 2 s, doubling up to 30 s. Otherwise the usual doubling to `cap`.
    public static func retry(after delay: Duration, hidden: Bool, cap: Duration = .seconds(1)) -> Duration {
        hidden ? min(max(delay * 2, .seconds(2)), .seconds(30)) : min(delay * 2, cap)
    }
}
