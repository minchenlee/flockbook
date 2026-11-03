import Foundation

/// Where herdr-core (`hc`) listens. Rules from upstream docs/security.md ("State and endpoints"):
/// `HC_SOCKET` overrides the endpoint; else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock` when that variable is set;
/// else `/run/user/<uid>/herdr-core/` when it exists (Linux only in practice); else the session's state directory,
/// `${XDG_STATE_HOME:-~/.local/state}/herdr-core/<session>/<session>.sock`. The session name comes from `HC_SESSION`,
/// default `default`. Flockbook adds its own overrides on top: `--hc-socket <path>` and `HERDR_DESK_HC_SOCKET`.
/// The config file's `socket` setting is not read (rare; use an override instead).
public struct HCSession: Sendable, Hashable {
    public let socketPath: String
    public init(socketPath: String) { self.socketPath = socketPath }

    /// The session with no override (`HC_SESSION`, `HC_SOCKET`, `--hc-socket` unset): the one the official app uses.
    public static var defaultSession: HCSession {
        var env: [String: String] = [:]
        for k in ["XDG_RUNTIME_DIR", "XDG_STATE_HOME", "HOME"] { env[k] = ProcessInfo.processInfo.environment[k] }
        return resolve(environment: env)
    }

    /// Whether a launch follows herdr-core: yes unless `--no-hc`. A headless run (checks, snapshots, `--snapshot-live`
    /// too) only with an explicit `--hc-socket`: none reads the user's default hc, and its machines, on its own.
    public static func wanted(arguments args: [String], headless: Bool) -> Bool {
        !args.contains("--no-hc") && (!headless || args.contains("--hc-socket"))
    }

    public static func resolve(environment env: [String: String] = ProcessInfo.processInfo.environment,
                               arguments: [String] = []) -> HCSession {
        if let i = arguments.firstIndex(of: "--hc-socket"), arguments.indices.contains(i + 1) { return HCSession(socketPath: arguments[i + 1]) }
        if let p = env["HERDR_DESK_HC_SOCKET"], !p.isEmpty { return HCSession(socketPath: p) }
        if let p = env["HC_SOCKET"], !p.isEmpty { return HCSession(socketPath: p) }
        let session = env["HC_SESSION"].flatMap { $0.isEmpty ? nil : $0 } ?? "default"
        if let r = env["XDG_RUNTIME_DIR"], !r.isEmpty { return HCSession(socketPath: "\(r)/herdr-core/\(session).sock") }
        let run = "/run/user/\(getuid())"
        if FileManager.default.fileExists(atPath: run) { return HCSession(socketPath: "\(run)/herdr-core/\(session).sock") }
        let state = env["XDG_STATE_HOME"].flatMap { $0.isEmpty ? nil : $0 } ?? NSHomeDirectory() + "/.local/state"
        return HCSession(socketPath: "\(state)/herdr-core/\(session)/\(session).sock")
    }

    /// True when a socket file is there. Missing is normal (hc not installed or not running), not an error.
    public var isAvailable: Bool {
        var st = stat()
        return lstat(socketPath, &st) == 0 && (st.st_mode & S_IFMT) == S_IFSOCK
    }
}
