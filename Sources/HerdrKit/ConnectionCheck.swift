import Foundation

/// Why the app cannot reach herdr, with the next step the user can take. Nil from `ConnectionCheck.diagnose`: connected.
public enum ConnectionProblem: Sendable, Hashable {
    /// No herdr server and no `herdr` program: install herdr.
    case herdrMissing
    /// herdr is installed but its server for this session is not running (no socket, or a socket nobody listens on).
    /// `command` starts it in a terminal.
    case notRunning(command: String)
    /// The socket is there but this user may not open it.
    case noPermission(path: String)
    /// Any other connect error.
    case failed(path: String, reason: String)
    /// herdr session `session` (nil: default) on `host` cannot be reached (`ConnectionCheck.diagnose(remote:)`).
    case remote(RemoteProblem, host: RemoteHost, session: String?)
    /// Settings > General > Backend is herdr-core only and herdr-core does not answer at `socket`.
    case hcNotRunning(socket: String)
}

/// Read-only checks for the first run: the session's socket and the `herdr` program. Sends nothing to herdr.
public enum ConnectionCheck {
    public static func diagnose(session: HerdrSession, environment: [String: String] = ProcessInfo.processInfo.environment,
                                home: String = NSHomeDirectory(), usual: [String]? = nil) -> ConnectionProblem? {
        let path = session.socketPath
        let e = connectError(path)
        switch e {
        case 0: return nil
        case ENOENT, ENOTDIR, ECONNREFUSED:
            guard let found = findHerdr(environment: environment, home: home, usual: usual) else { return .herdrMissing }
            // HERDR_BIN is often off the Terminal's PATH: start that one by its full path.
            let program = found == environment["HERDR_BIN"] ? shellQuoted(found) : "herdr"
            return .notRunning(command: ([program] + session.cliPrefix).joined(separator: " "))
        case EACCES, EPERM: return .noPermission(path: path)
        default: return .failed(path: path, reason: String(cString: strerror(e)))
        }
    }

    /// `diagnose` for a session on another host. Runs `herdr remote-api-bridge --check` there once (`probe` replaces
    /// that command in checks), then looks at the relay's last failed connection (`bridge`), then at the relay socket.
    /// Sends nothing to herdr's API. Nil: all fine (the request failed for another reason).
    public static func diagnose(remote session: HerdrSession, bridge: RemoteBridge?, probe: String? = nil,
                                timeout: Duration = .seconds(20)) async -> ConnectionProblem? {
        guard let host = session.remote else { return diagnose(session: session) }
        func problem(_ p: RemoteProblem) -> ConnectionProblem { .remote(p, host: host, session: session.name) }
        let command = probe ?? RemoteHost.herdrCommand(session: session.name, ["remote-api-bridge", "--check"])
        let r: RemoteHost.Result
        do { r = try await host.run(command, timeout: timeout) } catch { return problem(.other("\(error)")) }
        guard r.ok else { return problem(.classify(status: r.status, stderr: r.stderr)) }
        guard String(decoding: r.stdout, as: UTF8.self).contains(RemoteProblem.bridgeMarker) else { return problem(.bridgeUnsupported) }
        // The probe reached herdr; the bridge itself says whether herdr's server runs. Only a recent failure counts.
        if let f = bridge?.lastFailure, Date().timeIntervalSince(f.at) < 60 { return problem(f.problem) }
        let path = session.socketPath
        let e = await Task.detached { connectError(path) }.value
        if e != 0 { return problem(.other("the relay socket does not answer: \(String(cString: strerror(e)))")) }
        return nil
    }

    /// Connects to `path` once and closes again (no request). 0: a server listens; else the connect errno.
    static func connectError(_ path: String) -> Int32 {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return errno }
        defer { close(fd) }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: addr.sun_path) else { return ENAMETOOLONG }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            for (i, b) in bytes.enumerated() { raw[i] = b }
            raw[bytes.count] = 0
        }
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        return rc == 0 ? 0 : errno
    }

    /// The `herdr` program: `HERDR_BIN`, else on `PATH`, else in the usual install folders (a Finder launch has a short
    /// `PATH`). Nil: none. `usual`: those folders (nil: `usualFolders`).
    public static func findHerdr(environment: [String: String] = ProcessInfo.processInfo.environment, home: String = NSHomeDirectory(), usual: [String]? = nil) -> String? {
        let onPath = (environment["PATH"] ?? "").split(separator: ":").map { String($0) + "/herdr" }
        let usual = (usual ?? usualFolders(home: home)).map { $0 + "/herdr" }
        return ([environment["HERDR_BIN"]].compactMap { $0 } + onPath + usual).first(where: isProgram)
    }

    /// An executable file (or a symlink to one). A folder named herdr is not: it has the x bit too.
    static func isProgram(_ path: String) -> Bool {
        var folder: ObjCBool = false
        return FileManager.default.fileExists(atPath: path, isDirectory: &folder) && !folder.boolValue && FileManager.default.isExecutableFile(atPath: path)
    }

    /// Where herdr is installed when it is not on `PATH`: its installer's folder first, then Homebrew, then cargo.
    public static func usualFolders(home: String) -> [String] { [home + "/.local/bin", "/opt/homebrew/bin", "/usr/local/bin", home + "/.cargo/bin"] }

    /// `path` for a shell command line: as is when it has only safe characters, else in single quotes.
    public static func shellQuoted(_ path: String) -> String { ShellQuote.quotedIfNeeded(path) }

    /// Why a terminal cannot start when `findHerdr` finds nothing.
    public static let notFound = "herdr not found on PATH or in ~/.local/bin, /opt/homebrew/bin, /usr/local/bin, ~/.cargo/bin."

    /// `programWarning` for `session`. A session on another host needs no herdr program on this Mac (its terminals run
    /// herdr on the host): nil.
    public static func programWarning(session: HerdrSession, environment: [String: String] = ProcessInfo.processInfo.environment,
                                      home: String = NSHomeDirectory(), usual: [String]? = nil) -> String? {
        session.remote == nil ? programWarning(environment: environment, home: home, usual: usual) : nil
    }

    /// Connected but no `herdr` program where the app looks: the agents show, but terminals cannot start. Nil: found.
    public static func programWarning(environment: [String: String] = ProcessInfo.processInfo.environment, home: String = NSHomeDirectory(), usual: [String]? = nil) -> String? {
        findHerdr(environment: environment, home: home, usual: usual) == nil ? "herdr program not found: terminals can't start" : nil
    }
}

extension ConnectionProblem {
    public static let herdrSite = URL(string: "https://herdr.dev")!

    /// One line for the sidebar's connection footer.
    public var short: String {
        switch self {
        case .herdrMissing: "herdr is not installed"
        case .notRunning: "herdr is not running"
        case .noPermission: "No access to herdr's socket"
        case .failed: "Cannot connect to herdr"
        case .hcNotRunning: "herdr-core is not running"
        case .remote(let p, let host, _):
            switch p {
            case .auth: "SSH login to \(host.label) failed"
            case .hostKey: "Check the host key of \(host.label)"
            case .unreachable: "Cannot reach \(host.label)"
            case .herdrMissing: "herdr is not installed on \(host.label)"
            case .notRunning: "herdr is not running on \(host.label)"
            case .bridgeUnsupported: "herdr on \(host.label) is too old"
            case .other: "Cannot connect to herdr on \(host.label)"
            }
        }
    }

    public var title: String {
        switch self {
        case .herdrMissing: "herdr Is Not Installed"
        case .notRunning: "herdr Is Not Running"
        case .noPermission: "Cannot Open herdr's Socket"
        case .failed: "Cannot Connect to herdr"
        case .hcNotRunning: "herdr-core Is Not Running"
        case .remote(let p, _, _):
            switch p {
            case .auth: "SSH Login Failed"
            case .hostKey: "Check the Host Key"
            case .unreachable: "Cannot Reach the Host"
            case .herdrMissing: "herdr Is Not Installed on the Host"
            case .notRunning: "herdr Is Not Running on the Host"
            case .bridgeUnsupported: "Update herdr on the Host"
            case .other: "Cannot Connect to herdr on the Host"
            }
        }
    }

    /// What to do next, in full sentences.
    public var detail: String {
        switch self {
        case .herdrMissing:
            "Flockbook shows the agents that run in herdr. Install herdr, start it in a terminal, then come back. Flockbook connects by itself."
        case .notRunning:
            "Start herdr in a terminal with this command. Flockbook connects by itself when herdr starts."
        case .noPermission(let path):
            "macOS does not let Flockbook open \((path as NSString).abbreviatingWithTildeInPath). Quit herdr, start it again as your own user, then try again."
        case .failed(let path, let reason):
            "Connecting to \((path as NSString).abbreviatingWithTildeInPath) failed: \(reason). Quit herdr, start it again, then try again."
        case .hcNotRunning(let socket):
            "Backend is set to herdr-core only (Settings > General), and nothing answers at \((socket as NSString).abbreviatingWithTildeInPath). Start herdr-core with this command, or choose Automatic. Flockbook connects by itself when it starts."
        case .remote(let p, let host, _):
            switch p {
            case .auth:
                "ssh could not log in to \(host.label) with a key. Flockbook never asks for a password. Load your key with ssh-add, then check that this command logs in from Terminal. Flockbook connects by itself after that."
            case .hostKey:
                "The host key of \(host.label) is unknown or has changed. Run this command once in Terminal, check the key, and accept it only if it is right. Flockbook never accepts a host key by itself."
            case .unreachable(let why):
                "Flockbook cannot reach \(host.label): \(why). Check the host name, the network and that the host is on. Flockbook tries again by itself."
            case .herdrMissing:
                "There is no herdr program on \(host.label). Install herdr on that host (on its PATH or in ~/.local/bin), start it there, then come back."
            case .notRunning:
                "herdr is installed on \(host.label) but does not run for this session. Start it with this command in Terminal, or start herdr on the host. Flockbook connects by itself when herdr starts."
            case .bridgeUnsupported:
                "herdr on \(host.label) has no remote API bridge. Update herdr on that host, start it again, then come back."
            case .other(let why):
                "Connecting to herdr on \(host.label) failed: \(why). Check that “ssh \(host.label)” works in Terminal and that herdr runs on the host."
            }
        }
    }

    /// The command to copy into a terminal, if the next step is one.
    public var command: String? {
        switch self {
        case .notRunning(let c): return c
        case .hcNotRunning: return "hc server start"
        case .remote(let p, let host, let session):
            // Only an ssh host has a command: `.local` (checks) has no destination.
            guard case .ssh(let d) = host.via else { return nil }
            let dest = ConnectionCheck.shellQuoted(d)
            switch p {
            case .auth, .hostKey: return "ssh " + dest
            case .notRunning:
                return (["herdr", "--remote", dest] + (session.map { ["--session", ConnectionCheck.shellQuoted($0)] } ?? [])).joined(separator: " ")
            default: return nil
            }
        default: return nil
        }
    }

    /// A page to open, if the next step is one.
    public var link: URL? {
        switch self {
        case .herdrMissing, .remote(.herdrMissing, _, _): Self.herdrSite
        default: nil
        }
    }
}
