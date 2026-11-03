import Foundation
@testable import HerdrKit
import ModFeedKit

// The herdr-app mod's sidecar (ModFeedKit): rows, footers, the tail reader, the incremental feed, status tags.
func checksModFeed() throws {
    var utc = Calendar(identifier: .gregorian); utc.timeZone = TimeZone(identifier: "UTC")!
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("modfeed-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    func ts(_ s: Int) -> String { "2026-11-03T01:\(String(format: "%02d", 40 + s / 60)):\(String(format: "%02d", s % 60)).000Z" }
    func user(_ id: String, _ t: Int, _ text: String) -> String {
        #"{"type":"user","uuid":"\#(id)","timestamp":"\#(ts(t))","message":{"role":"user","content":"\#(text)"}}"#
    }
    func say(_ id: String, _ t: Int, _ text: String) -> String {
        #"{"type":"assistant","uuid":"\#(id)","timestamp":"\#(ts(t))","message":{"role":"assistant","model":"claude-test-1","content":[{"type":"text","text":"\#(text)"}]}}"#
    }
    func tool(_ id: String, _ t: Int, _ call: String, _ cmd: String) -> String {
        #"{"type":"assistant","uuid":"\#(id)","timestamp":"\#(ts(t))","message":{"role":"assistant","content":[{"type":"tool_use","id":"\#(call)","name":"Bash","input":{"command":"\#(cmd)"}}]}}"#
    }
    func result(_ id: String, _ t: Int, _ call: String) -> String {
        #"{"type":"user","uuid":"\#(id)","timestamp":"\#(ts(t))","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"\#(call)","content":"ok"}]}}"#
    }

    // Rows: adjacent tool items are one row; the turn ends with a footer.
    let turn = [user("u1", 0, "Fix it"), tool("a1", 5, "c1", "ls"), result("r1", 8, "c1"), tool("a2", 10, "c2", "swift build"),
                result("r2", 20, "c2"), tool("a3", 21, "c3", "git diff"), result("r3", 25, "c3"), say("a4", 36, "Done.")]
    let rows = FeedRows.rows(FeedTail.items(turn), now: Date(timeIntervalSince1970: 1_793_670_000), calendar: utc)
    check(rows.map(\.k) == ["user", "tools", "assistant", "footer"], "modfeed rows: user, one tools row, reply, footer (\(rows.map(\.k)))")
    check(rows.first(where: { $0.k == "tools" })?.chips?.flatMap(\.calls).count == 3, "modfeed rows: the tools row holds the 3 calls")
    check(rows.first(where: { $0.k == "tools" })?.chips?.flatMap(\.calls).allSatisfy(\.done) == true, "modfeed rows: results mark calls done")
    check(rows.last?.text == "01:40 · Worked 36s · 3 tools", "modfeed footer: clock, worked, tools (\(rows.last?.text ?? "nil"))")
    check(rows.last?.id == "f-" + (rows[rows.count - 2].id), "modfeed footer: keyed by the turn's last row")

    // A gap of 15 minutes or more gets a separator.
    let gap = [user("u1", 0, "a"), say("a1", 2, "b"), user("u2", 1000, "c")]
    let g = FeedRows.rows(FeedTail.items(gap), now: Date(timeIntervalSince1970: 1_793_670_000), calendar: utc)
    check(g.map(\.k) == ["user", "assistant", "footer", "sep", "user"], "modfeed separator after 15 min (\(g.map(\.k)))")

    // The tail reader drops a cut first line and an unfinished last line.
    let f = dir.appendingPathComponent("s.jsonl")
    try Data((turn.joined(separator: "\n") + "\n" + #"{"type":"user","par"#).utf8).write(to: f)
    let all = try FeedTail.read(f)
    check(all.lines.count == turn.count && all.start == 0, "modfeed tail: whole lines, unfinished one left out (\(all.lines.count))")
    let cut = try FeedTail.read(f, bytes: turn.last!.utf8.count + 30)
    check(cut.lines == [turn.last!], "modfeed tail: a cut first line is dropped (\(cut.lines.count))")
    check(cut.start == UInt64(turn.dropLast().joined(separator: "\n").utf8.count + 1), "modfeed tail: start is the first whole line")
    check(FeedTail.model(turn) == "claude-test-1", "modfeed tail: model from the newest assistant line")

    // Paste: a new PNG file in the folder; pastes older than a day go.
    let pdir = dir.appendingPathComponent("paste")
    let p1 = try FeedPaste.save(Data([1, 2, 3]), dir: pdir, now: Date(timeIntervalSince1970: 1_000_000))
    try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSince1970: 1_000_000)], ofItemAtPath: p1)
    let p2 = try FeedPaste.save(Data([4]), dir: pdir, now: Date(timeIntervalSince1970: 1_000_000 + 2 * 86400))
    check(!FileManager.default.fileExists(atPath: p1) && (try? Data(contentsOf: URL(fileURLWithPath: p2))) == Data([4]) && p2.hasSuffix(".png"),
          "modfeed paste: new file, old one removed")

    // Older rows: the whole lines before a start, a cut first line dropped; start 0 at the file's start.
    let before = try FeedTail.read(f, before: cut.start, bytes: turn[6].utf8.count + turn[5].utf8.count + 5)
    check(before.lines == [turn[5], turn[6]], "modfeed older: lines before the tail, cut first one dropped (\(before.lines.count))")
    let first = try FeedTail.read(f, before: cut.start)
    check(first.start == 0 && first.lines == Array(turn.dropLast()), "modfeed older: reaches the file's start (\(first.lines.count))")
    let oo = JSONLines()
    try Feed.older(url: f, before: cut.start, out: oo, calendar: utc)
    check(oo.lines.first?.contains(#""type":"older""#) == true && oo.lines.first?.contains(#""start":0"#) == true,
          "modfeed older: an older line with start 0 first (\(oo.lines.first ?? "nil"))")
    check(oo.lines.dropFirst().allSatisfy { $0.contains(#""type":"row""#) } && oo.lines.count > 1, "modfeed older: then rows")

    // Feed: whole, then unchanged, then a splice from the first changed row.
    try Data((turn.prefix(5).joined(separator: "\n") + "\n").utf8).write(to: f)
    let o1 = JSONLines()
    let s1 = try Feed.run(url: f, agent: "claude", cursor: nil, previous: nil, out: o1, calendar: utc)
    check(o1.lines.contains { $0.contains(#""at":0"#) && $0.contains(#""type":"splice""#) }, "modfeed feed: first answer splices at 0")
    let n1 = o1.lines.filter { $0.contains(#""type":"row""#) }.count
    check(n1 == 2, "modfeed feed: user + tools row (\(n1))")
    let o2 = JSONLines()
    let s2 = try Feed.run(url: f, agent: "claude", cursor: s1?.cursor, previous: s1, out: o2, calendar: utc)
    check(o2.lines.contains(#"{"type":"unchanged"}"#) && s2 == s1, "modfeed feed: same size, unchanged")
    let h = try FileHandle(forWritingTo: f); try h.seekToEnd(); try h.write(contentsOf: Data((turn.dropFirst(5).joined(separator: "\n") + "\n").utf8)); try h.close()
    let o3 = JSONLines()
    let s3 = try Feed.run(url: f, agent: "claude", cursor: s1?.cursor, previous: s1, out: o3, calendar: utc)
    let splice3 = o3.lines.first { $0.contains(#""type":"splice""#) } ?? ""
    check(splice3.contains(#""at":1"#) && splice3.contains(#""count":4"#), "modfeed feed: growth splices at the changed tools row (\(splice3))")
    check(o3.lines.filter { $0.contains(#""type":"row""#) }.count == 3, "modfeed feed: prints rows from the splice on")
    check(s3?.start == s1?.start, "modfeed feed: keeps the range start while the log grows")
    let o4 = JSONLines()
    _ = try Feed.run(url: f, agent: "claude", cursor: "1.bad.0.0.x", previous: s3, out: o4, calendar: utc)
    check(o4.lines.contains { $0.contains(#""at":0"#) && $0.contains(#""type":"splice""#) }, "modfeed feed: an unknown cursor gets everything again")
    let o5 = JSONLines()
    _ = try Feed.run(url: nil, agent: "codex", cursor: nil, previous: nil, out: o5)
    check(o5.lines.first?.contains(#""path":null"#) == true, "modfeed feed: no log, path null")

    // Status: the wrapper's folder from Claude Code's settings, a quoted path with a space.
    let words = FeedStatus.shellWords(#"'/opt/a b/flockbook-statusline' --dir "/tmp/x y" --x\ z"#)
    check(words == ["/opt/a b/flockbook-statusline", "--dir", "/tmp/x y", "--x z"], "modfeed status: shell words (\(words))")
    let sdir = dir.appendingPathComponent("status dir")
    let settings = dir.appendingPathComponent("settings.json")
    let cmd = "'/opt/flockbook-statusline' --dir '\(sdir.path)'"
    try JSONSerialization.data(withJSONObject: ["statusLine": ["type": "command", "command": cmd]]).write(to: settings)
    check(FeedStatus.folder(settings: settings)?.path == sdir.path, "modfeed status: wrapper folder from settings")
    let log = dir.appendingPathComponent("sess-1.jsonl")
    try Data((turn.joined(separator: "\n") + "\n").utf8).write(to: log)
    StatusLineFolder(url: sdir).save(input: Data(#"{"session_id":"sess-1","model":{"display_name":"Opus 9"},"effort":{"level":"high"},"context_window":{"used_percentage":84}}"#.utf8),
                                     output: Data("own part".utf8), session: "sess-1")
    let o6 = JSONLines()
    FeedStatus.run(agent: "claude", log: log, settings: settings, out: o6)
    let st = o6.lines.first ?? ""
    check(st.contains(#""source":"statusline""#) && st.contains(#""text":"Opus 9""#) && st.contains(#""text":"ctx 84%""#)
          && st.contains(#""warn":true"#), "modfeed status: wrapper facts, full ctx warns (\(st))")
    let o7 = JSONLines()
    FeedStatus.run(agent: "codex", log: log, settings: settings, out: o7)
    check(o7.lines.first?.contains(#""source":"log""#) == true && o7.lines.first?.contains("claude-test-1") == true, "modfeed status: model from the log otherwise")

    // Locate: a terminal by address or pane id; a cache entry holds only for the same processes, lately checked.
    let pane = Pane(id: "hc:t7", terminalId: "t7", cwd: "/w", agent: Agent(name: "c", kind: "claude", status: .idle, sessionId: nil, title: "c", backend: .hc),
                    backend: .hc, processIds: [42, 41])
    let ws = [Workspace(id: "w", label: "w", tabs: [Tab(id: "t", label: "t", number: 1, panes: [pane], status: .idle, backend: .hc)], backend: .hc)]
    check(FeedLocate.pane(ws, terminal: "t7")?.id == "hc:t7" && FeedLocate.pane(ws, terminal: "hc:t7") != nil, "modfeed locate: by address or pane id")
    let now = Date()
    let e = FeedLocate.Entry(kind: "claude", cwd: "/w", pids: [42, 41], path: log.path, checked: now.timeIntervalSince1970 - 5)
    check(FeedLocate.fresh(e, for: pane, kind: "claude", now: now), "modfeed locate: fresh entry used")
    var other = e; other.pids = [43, 41]
    check(!FeedLocate.fresh(other, for: pane, kind: "claude", now: now), "modfeed locate: other processes, looked up again")
    check(!FeedLocate.fresh(e, for: pane, kind: "claude", now: now + 40), "modfeed locate: old entry, looked up again")
    var cache = FeedLocate.Cache(); cache.entries["t7"] = e; cache.feeds["t7"] = s3
    let cf = dir.appendingPathComponent("c/cache.json")
    FeedLocate.writeCache(cache, to: cf)
    check(FeedLocate.readCache(cf) == cache, "modfeed locate: cache round trip")
}
