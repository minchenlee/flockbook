// flockbook-modfeed: the herdr-app mod's sidecar. One command per run; the answer is JSON Lines on stdout
// (contract: docs/protocol.md in the mod repo). Picks the herdr-core session as the app does (HC_SESSION, HC_SOCKET).
//
//   flockbook-modfeed version
//   flockbook-modfeed feed   --terminal <addr> [--cursor <c>] [--cache <file>]
//   flockbook-modfeed feed   --terminal <addr> --before <start> --path <log>   (older rows; no cache change)
//   flockbook-modfeed status --terminal <addr> [--cache <file>]
//   flockbook-modfeed card   --terminal <addr> [--cache <file>]
//   flockbook-modfeed approve --terminal <addr> --request <id> --allow|--deny [--cache <file>]
//   flockbook-modfeed answer --terminal <addr> --request <id> --answers <json> [--cache <file>]
//   flockbook-modfeed send   --terminal <addr> --text <s> [--file <path>]... [--cache <file>]
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
    out.emit(["type": "version", "commands": ["version", "feed", "status", "card", "approve", "answer", "send"]])
    finish(ok: true)
case "feed", "status", "card", "approve", "answer", "send":
    guard let terminal = opt("--terminal") else { fail("usage", "\(cmd) needs --terminal <addr>") }
    let cacheURL = opt("--cache").map { URL(fileURLWithPath: $0) }
    let session = HCSession.resolve()
    guard session.isAvailable else { fail("no-core", "herdr-core is not running") }
    let ws: [Workspace]
    // exec gives the sidecar 30 s; a send that waits longer reports `unconfirmed`.
    let client = HCClient(session: session, callTimeout: 8, sendTimeout: 22)
    do { ws = try await client.snapshot() } catch { fail("core", "herdr-core: \(error)") }
    let found: FeedLocate.Found
    do {
        found = try await FeedLocate.find(terminal: terminal, workspaces: ws,
                                          resolver: SessionResolver(socketPath: session.socketPath), cacheURL: cacheURL)
    } catch let e as FeedLocate.Failure {
        if case .noTerminal = e { fail("no-terminal", e.description) } else { fail("not-agent", e.description) }
    } catch { fail("locate", "\(error)") }
    let kind = found.pane.agent?.kind ?? "agent"
    let url = found.path.map { URL(fileURLWithPath: $0) }
    switch cmd {
    case "status":
        FeedStatus.run(agent: kind, log: url, settings: FeedStatus.defaultSettings(), out: out)
        finish(ok: true)
    case "card":
        out.emit(await FeedCards.card(pane: found.pane, screen: client, log: url))
        finish(ok: true)
    case "approve":
        guard let request = opt("--request"), args.contains("--allow") != args.contains("--deny") else {
            fail("usage", "approve needs --request <id> and one of --allow, --deny")
        }
        out.emit(await FeedCards.approve(pane: found.pane, screen: client, log: url, request: request, allow: args.contains("--allow")))
        finish(ok: true)
    case "answer":
        guard let request = opt("--request"), let answers = opt("--answers").flatMap(FeedCards.parseAnswers) else {
            fail("usage", "answer needs --request <id> and --answers <json>")
        }
        out.emit(await FeedCards.answer(pane: found.pane, screen: client, log: url, request: request, answers: answers))
        finish(ok: true)
    case "send":
        guard let text = opt("--text"), !text.isEmpty else { fail("usage", "send needs --text <s>") }
        let files = args.indices.filter { args[$0] == "--file" && args.indices.contains($0 + 1) }.map { args[$0 + 1] }
        out.emit(await FeedCards.send(pane: found.pane, client: client, text: text, files: files))
        finish(ok: true)
    default: break
    }
    if let b = opt("--before") {
        guard let before = UInt64(b), let want = opt("--path") else { fail("usage", "feed --before needs a number and --path <log>") }
        guard let url, url.path == want else { fail("moved", "the agent now writes another log") }
        do { try Feed.older(url: url, before: before, out: out) } catch { fail("read", "cannot read the log: \(error)") }
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
    fail("usage", "commands: version, feed, status, card, approve, answer, send")
}
