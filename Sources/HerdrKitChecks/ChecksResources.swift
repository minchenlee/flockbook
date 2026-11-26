import Foundation
@testable import HerdrKit

// Resources (HerdrKit/Resources*.swift) against the shared vectors in Tests/Fixtures/resources: the helper writes them
// (xplat/flockbook-modfeed/tests/resources_vectors.rs) and the mod reads them too (herdr-app/test/resources-vectors.test.mjs).

private let resFixtures = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
    .appendingPathComponent("../../Tests/Fixtures/resources").standardized
private func resFixture(_ n: String) throws -> Any {
    try JSONSerialization.jsonObject(with: Data(contentsOf: resFixtures.appendingPathComponent(n)))
}
private func json(_ v: Any) -> String {
    String(decoding: (try? JSONSerialization.data(withJSONObject: v, options: [.sortedKeys, .fragmentsAllowed, .withoutEscapingSlashes])) ?? Data(), as: UTF8.self)
}
private func found(_ f: [Resources.Found]) -> [[Any]] { f.map { [$0.start, $0.end, $0.kind, $0.target] } }

/// One collect vector: its files in a fresh folder (\$ROOT), its lines as the log.
private func collected(_ c: [String: Any]) -> [[String: Any]] {
    let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hkres-\(UUID().uuidString)")
    try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let root = Resources.realpath(dir.path) ?? dir.path
    for (rel, body) in c["files"] as? [String: String] ?? [:] {
        let p = URL(fileURLWithPath: root).appendingPathComponent(rel)
        try? FileManager.default.createDirectory(at: p.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? body.write(to: p, atomically: true, encoding: .utf8)
    }
    let put = { (s: String) in s.replacingOccurrences(of: "$ROOT", with: root) }
    let back = { (s: String) in s.replacingOccurrences(of: root, with: "$ROOT") }
    let lines = (c["lines"] as! [String]).map(put)
    let items = Resources.collect(lines: lines, home: (c["home"] as? String).map(put), agent: (c["agent"] as? String).map(put))
    return items.map { i in
        var o: [String: Any] = ["kind": i.kind, "target": back(i.target), "title": back(i.title), "detail": back(i.detail), "row": i.row,
                                "first_ts": i.firstTs.map { $0 as Any } ?? NSNull(), "last_ts": i.lastTs.map { $0 as Any } ?? NSNull(),
                                "mentions": i.mentions, "exists": i.exists.map { $0 as Any } ?? NSNull()]
        if !i.target.contains(root) { o["id"] = i.id }
        return o
    }
}

func checksResources() throws {
    let v = try resFixture("vectors.json") as! [String: Any]
    for c in v["detect"] as! [[String: Any]] {
        let t = c["text"] as! String
        check(json(found(Resources.detect(t))) == json(c["feed"]!), "resources vectors: detect \(json(t))")
        check(json(found(Resources.scan(t, rel: true))) == json(c["collect"]!), "resources vectors: collect finds in \(json(t))")
    }
    for c in v["commands"] as! [[String: Any]] {
        let t = c["text"] as! String
        let got: [[Any]] = Resources.commands(t).map { [$0.start, $0.title, $0.cmd] }
        check(json(got) == json(c["want"]!), "resources vectors: commands in \(json(t))")
    }
    for c in v["collect"] as! [[String: Any]] {
        let got = collected(c)
        let same = json(got) == json(c["want"]!)
        if !same { print("  got  \(json(got))\n  want \(json(c["want"]!))") }
        check(same, "resources vectors: collect, \(c["name"] ?? "")")
    }
    for g in v["groups"] as! [[String]] {
        check(Resources.groupOf(kind: g[0], target: g[1]) == g[2], "resources vectors: \(g[0]) \(g[1]) is in \(g[2])")
    }
    let order = (v["groups"] as! [[String]]).map { $0[2] }.reduce(into: [String]()) { if !$0.contains($1) { $0.append($1) } }
    check(Resources.groups.map(\.id).filter(order.contains) == order, "resources vectors: the groups' order")
    check(Resources.fnv1a("cmd:npm test") == Resources.collect(lines: (v["collect"] as! [[String: Any]])[1]["lines"] as! [String], home: nil).first { $0.kind == "cmd" && $0.target == "npm test" }?.id,
          "resources: an id is the FNV-1a of the key")

    // md-links.json: the feed's links (what mdsplit wraps is the helper's own).
    for c in try resFixture("md-links.json") as! [[String: Any]] {
        let want = (c["links"] as! [[String: Any]]).map { [$0["start"]!, $0["end"]!, $0["res"]!] }
        let got: [[Any]] = Resources.detect(c["text"] as! String).map { [$0.start, $0.end, $0.kind] }
        check(json(got) == json(want), "resources md-links: \(c["name"] ?? "")")
    }

    // conversation.jsonl → expected.json (kind, target, title, row, mentions), the home /home/me.
    let log = String(decoding: try Data(contentsOf: resFixtures.appendingPathComponent("conversation.jsonl")), as: UTF8.self)
    let items = Resources.collect(lines: log.split(separator: "\n").map(String.init), home: "/home/me")
    let got = items.map { ["kind": $0.kind, "target": $0.target, "title": $0.title, "row": $0.row, "mentions": $0.mentions] as [String: Any] }
    let want = try resFixture("expected.json")
    if json(got) != json(want) { print("  got  \(json(got))\n  want \(json(want))") }
    check(json(got) == json(want), "resources: conversation.jsonl gives expected.json")

    // The same log read as a file (with byte offsets): the same items.
    let fromFile = Resources.collect(file: resFixtures.appendingPathComponent("conversation.jsonl"), home: "/home/me") ?? []
    check(fromFile == items, "resources: collect(file:) gives what collect(lines:) gives")
    check(Resources.millis(Date(timeIntervalSince1970: 1e300)) == .max && Resources.millis(Date(timeIntervalSince1970: -1e300)) == .min
          && Resources.millis(Date(timeIntervalSince1970: 1.5)) == 1500, "resources: an out-of-range timestamp saturates, never traps")
    check(Resources.ago(.min, now: 1_000_000) == "\(Int(Double(1_000_000 - Double(Int64.min)) / 1000 / 86400))d" && Resources.ago(.max, now: 1_000_000) == "now",
          "resources: ago of a saturated timestamp does not overflow")
    check(Resources.collect(file: resFixtures.appendingPathComponent("no-such.jsonl"), home: nil) == nil, "resources: a file that cannot be read gives nil")
    // A log read in steps (as it grows, cut in a line and at a line's end) gives what one read gives.
    do {
        let whole = try Data(contentsOf: resFixtures.appendingPathComponent("conversation.jsonl"))
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("res-reader-\(UUID().uuidString).jsonl")
        defer { try? FileManager.default.removeItem(at: tmp) }
        let nl = whole.indices.filter { whole[$0] == 0x0A }
        let cuts = [nl[nl.count / 3] + 1, nl[nl.count / 2] + 5, whole.count]
        try Data().write(to: tmp)
        let fh = try FileHandle(forWritingTo: tmp)
        var r = Resources.Reader(), from = 0, steps = true
        for c in cuts {
            try fh.seekToEnd(); try fh.write(contentsOf: whole[from..<c]); from = c
            steps = steps && r.read(tmp)
            if c != whole.count { steps = steps && r.items(home: "/home/me") == Resources.collect(file: tmp, home: "/home/me") }
        }
        try fh.close()
        check(steps && r.items(home: "/home/me") == items, "resources: a log read in steps gives what one read gives")
        // A log that shrank is read again from its start.
        try whole[0..<cuts[0]].write(to: tmp)
        check(r.read(tmp) && r.items(home: "/home/me") == Resources.collect(file: tmp, home: "/home/me") && r.offset == cuts[0],
              "resources: a log that shrank is read again from its start")
    } catch { check(false, "resources: reader temp file (\(error))") }
    let cwds: [(String, String?)] = [
        (#"{"type":"user","cwd":"/a/b//","x":1}"#, "/a/b"), (#"{"cwd":"/"}"#, nil), (#"{"cwd":"rel/x"}"#, nil),
        (#"{"cwd":"/a\b"}"#, nil), (#"{"cwd":""}"#, nil), (#"{"cwd":"/a"#, nil), (#"{"c":1,"cwd":"/é/ü","cwd":"/z"}"#, "/é/ü"),
        (#"{"ccwd":"/x"}"#, nil), (#"{"type":"x"}"#, nil), (#""cwd":"/x""#, "/x"), ("", nil)]
    for (l, want) in cwds { check(Resources.lineCwd(l) == want, "resources: lineCwd of \(l) is \(want ?? "nil")") }
    checksResourcesList()
}

/// The window's list rules, as the mod's resources.test.mjs.
private func checksResourcesList() {
    func res(_ id: String, _ kind: String, _ ts: Int64?, target: String? = nil, exists: Bool? = nil) -> Resources.Item {
        Resources.Item(id: id, kind: kind, target: target ?? "https://x.com/\(id)", title: id, detail: "", row: "r\(id)", firstTs: ts, lastTs: ts, mentions: 1, exists: exists)
    }
    let items = [res("a", "link", 10), res("b", "doc", 20, target: "/p/b.md", exists: true)]
    var removed = Resources.removing(items[0], from: [])
    check(Resources.visible(items, removed: removed).map(\.id) == ["b"], "resources list: removed stays removed")
    check(Resources.visible([res("a", "link", 50)] + items.dropFirst(), removed: removed).map(\.id) == ["a", "b"], "resources list: mentioned again later it comes back")
    check(Resources.visible([res("a", "link", 99)], removed: [.init(id: "a", lastTs: nil)]).isEmpty, "resources list: removed for good")
    removed = Resources.removing(res("a", "link", 30), from: removed)
    check(removed == [.init(id: "a", lastTs: 30)], "resources list: one entry per id, with the last mention it had")
    check(Resources.badgeCount(items, seen: 10, removed: []) == 1, "resources list: the badge counts the newer ones")
    check(Resources.badgeCount(items, seen: nil, removed: []) == 0, "resources list: nothing seen yet is no badge")
    check(Resources.badgeCount(items + [res("c", "link", 30)], seen: 10, removed: [.init(id: "c", lastTs: 30)]) == 1, "resources list: a removed one is not counted")
    check(Resources.newest(items) == 20, "resources list: newest")

    let many = (0..<35).map { res("l\($0)", "link", Int64(100 - $0)) } + [res("m", "doc", 5, target: "/p/m.MD"), res("c", "cmd", 4, target: "make")]
    let laid = Resources.layoutGroups(many)
    check(laid.map(\.group.id) == ["markdown", "commands", "links"], "resources list: the groups in order, only those with items")
    check(laid.last?.shown.count == 30 && laid.last?.rest == 5, "resources list: 30 rows, then Show 5 more")
    check(Resources.layoutGroups(many, expanded: ["links"]).last?.shown.count == 35, "resources list: Show more draws them all")
    let folded = Resources.layoutGroups(many, collapsed: ["links"]).last
    check(folded?.collapsed == true && folded?.shown.isEmpty == true && folded?.rest == 0 && folded?.items.count == 35, "resources list: a folded group draws no rows")

    let now: Int64 = 1_000_000_000
    check([Resources.ago(nil, now: now), Resources.ago(now - 5_000, now: now), Resources.ago(now - 240_000, now: now),
           Resources.ago(now - 3 * 3_600_000, now: now), Resources.ago(now - 2 * 86_400_000, now: now)] == ["", "now", "4m", "3h", "2d"], "resources list: ago")

    check(Resources.openPlan(res("p", "pr", 1, target: "https://github.com/o/r/pull/1")) == .url(URL(string: "https://github.com/o/r/pull/1")!), "resources plan: a web address opens in the browser")
    if case .why = Resources.openPlan(res("j", "link", 1, target: "javascript:alert(1)")) {} else { check(false, "resources plan: no javascript:") }
    check(Resources.openPlan(res("d", "doc", 1, target: "/home/me/plan.md", exists: true)) == .file("/home/me/plan.md"), "resources plan: a doc here opens")
    check(Resources.openPlan(res("d", "doc", 1, target: "/home/me/r.HTML", exists: true)) == .file("/home/me/r.HTML"), "resources plan: any case")
    check(Resources.openPlan(res("d", "doc", 1, target: "/home/me/plan.md", exists: false)) == .why("File not found."), "resources plan: a missing file")
    check(Resources.openPlan(res("d", "doc", 1, target: "/home/me/plan.md", exists: true), machine: "box") == .why("This file is on box. Open it there."), "resources plan: a file on another machine")
    check(Resources.openPlan(res("d", "doc", 1, target: "/home/me/run.sh", exists: true)) == .why("Flockbook does not open this kind of file."), "resources plan: a kind that does not open")
    check(Resources.openPlan(res("d", "doc", 1, target: "~/plan.md", exists: true)) == .why("Not a full path."), "resources plan: not a full path")
    check(Resources.openPlan(res("c", "cmd", 1, target: "cd ~/x && \\\n  make")) == .copy("cd ~/x && \\\n  make"), "resources plan: a command is copied, never run")
}
