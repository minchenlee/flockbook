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
}
