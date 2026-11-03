// flockbook-modfeed: the herdr-app mod's sidecar. One command per run; the answer is JSON Lines on stdout
// (contract: docs/protocol.md in the mod repo). Picks the herdr-core session as the app does (HC_SESSION, HC_SOCKET).
//
//   flockbook-modfeed version
//   flockbook-modfeed feed   --terminal <addr> [--cursor <c>] [--cache <file>]
//   flockbook-modfeed status --terminal <addr> [--cache <file>]
import Foundation
import HerdrKit
import ModFeedKit

let t0 = Date()
let out = JSONLines()
let argv = Array(CommandLine.arguments.dropFirst())
let cmd = argv.first ?? ""
let args = Array(argv.dropFirst())
func opt(_ name: String) -> String? {
    guard let i = args.firstIndex(of: name), args.indices.contains(i + 1) else { return nil }
    return args[i + 1]
}
func finish(ok: Bool) -> Never {
    out.emit(["type": "end", "ok": ok, "ms": Int(Date().timeIntervalSince(t0) * 1000)])
    FileHandle.standardOutput.write(Data(out.text().utf8))
    exit(0)
}
func fail(_ code: String, _ message: String) -> Never {
    out.emit(["type": "error", "code": code, "message": message])
    finish(ok: false)
}

out.emit(["type": "header", "protocol": FeedProtocol.version, "sidecar": FeedProtocol.sidecarVersion, "cmd": cmd])
switch cmd {
case "version":
    out.emit(["type": "version", "commands": ["version", "feed", "status"]])
    finish(ok: true)
case "feed", "status":
    guard let terminal = opt("--terminal") else { fail("usage", "\(cmd) needs --terminal <addr>") }
    let cacheURL = opt("--cache").map { URL(fileURLWithPath: $0) }
    let session = HCSession.resolve()
    guard session.isAvailable else { fail("no-core", "herdr-core is not running") }
    let ws: [Workspace]
    do { ws = try await HCClient(session: session).snapshot() } catch { fail("core", "herdr-core: \(error)") }
    let found: FeedLocate.Found
    do {
        found = try await FeedLocate.find(terminal: terminal, workspaces: ws,
                                          resolver: SessionResolver(socketPath: session.socketPath), cacheURL: cacheURL)
    } catch let e as FeedLocate.Failure {
        if case .noTerminal = e { fail("no-terminal", e.description) } else { fail("not-agent", e.description) }
    } catch { fail("locate", "\(error)") }
    let kind = found.pane.agent?.kind ?? "agent"
    let url = found.path.map { URL(fileURLWithPath: $0) }
    if cmd == "status" {
        FeedStatus.run(agent: kind, log: url, settings: FeedStatus.defaultSettings(), out: out)
        finish(ok: true)
    }
    var cache = FeedLocate.readCache(cacheURL)
    do {
        let state = try Feed.run(url: url, agent: kind, cursor: opt("--cursor"), previous: cache.feeds[found.pane.terminalId], out: out)
        if cacheURL != nil, cache.feeds[found.pane.terminalId] != state {
            cache.feeds[found.pane.terminalId] = state
            FeedLocate.writeCache(cache, to: cacheURL)
        }
    } catch { fail("read", "cannot read the log: \(error)") }
    finish(ok: true)
default:
    fail("usage", "commands: version, feed, status")
}
