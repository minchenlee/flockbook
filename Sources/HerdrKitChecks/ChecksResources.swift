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
        // Read newest first in small chunks: the whole read gives what one read gives; the first step reads the end.
        let three = whole + whole + whole   // a typed prompt in the middle, so the read can cut
        try three.write(to: tmp)
        let want3 = Resources.collect(file: tmp, home: "/home/me") ?? []
        for size in [200, 1000, 2000] {
            var p = Resources.Progressive(chunk: size)
            var ok = p.update(tmp) && !p.done && p.fraction > 0 && p.fraction < 1
            var n = 0
            while ok, !p.done, n < 10_000 { ok = p.older(tmp, home: "/home/me"); n += 1 }
            check(ok && p.done && p.fraction == 1 && !want3.isEmpty && p.items(home: "/home/me") == want3,
                  "resources: a log read newest first in \(size)-byte chunks gives what one read gives (\(n) older chunks)")
        }
        var one = Resources.Progressive(split: false)
        check(one.update(tmp) && one.done && one.items(home: "/home/me") == want3, "resources: split false reads the whole log at once")
        // A tool call whose result comes several parts later: the parts are read again until it closes.
        var gap = log.split(separator: "\n").map(String.init)
        gap.insert(contentsOf: (0..<12).map { #"{"type":"user","uuid":"p\#($0)","timestamp":"2026-11-03T01:40:06.500Z","message":{"role":"user","content":"wait \#($0)"}}"# }, at: 3)
        try Data((gap.joined(separator: "\n") + "\n").utf8).write(to: tmp)
        let wantGap = Resources.collect(file: tmp, home: "/home/me") ?? []
        var g = Resources.Progressive(chunk: 150)
        var gok = g.update(tmp), gn = 0
        while gok, !g.done, gn < 1000 { gok = g.older(tmp, home: "/home/me"); gn += 1 }
        check(gok && g.done && !wantGap.isEmpty && g.items(home: "/home/me") == wantGap,
              "resources: a tool call with its result \(gn) parts later gives what one read gives")
        // Codex: a rollout read newest first in chunks gives what one read gives, field by field. Its cuts are message
        // lines with text; a call whose result comes after the next message, and a message the reader does not show
        // between two runs of calls, are not cut at.
        func cx(_ t: Int, _ payload: String) -> String {
            #"{"timestamp":"2026-11-03T02:\#(String(format: "%02d", t % 60)):00.000Z","type":"response_item","payload":"# + payload + "}"
        }
        func say(_ role: String, _ t: String) -> String {
            #"{"type":"message","role":"\#(role)","content":[{"type":"\#(role == "user" ? "input_text" : "output_text")","text":"\#(t)"}]}"#
        }
        func call(_ id: String, _ name: String, _ args: String) -> String {
            #"{"type":"function_call","name":"\#(name)","arguments":"\#(args)","call_id":"\#(id)"}"#
        }
        func out(_ id: String) -> String { #"{"type":"function_call_output","call_id":"\#(id)","output":"ok"}"# }
        var cxl = [#"{"timestamp":"2026-11-03T02:00:00.000Z","type":"session_meta","payload":{"id":"s","cwd":"/home/me/repo"}}"#,
                   cx(0, say("user", "# AGENTS.md instructions")), cx(0, say("user", "<environment_context>x</environment_context>"))]
        for i in 0..<8 {
            cxl.append(cx(i, say("user", "round \(i): see docs/plan-\(i).md")))
            cxl.append(cx(i, call("c\(i)a", "exec_command", #"{\"cmd\":\"cd /home/me/repo && ls\"}"#)))
            cxl.append(cx(i, out("c\(i)a")))
            cxl.append(cx(i, call("c\(i)b", "write_file", #"{\"file_path\":\"/home/me/repo/docs/plan-\#(i).md\"}"#)))
            cxl.append(cx(i, out("c\(i)b")))
            if i % 3 == 1 {   // a result after the next message
                cxl.append(cx(i, call("c\(i)c", "exec_command", #"{\"cmd\":\"sleep 1\"}"#)))
                cxl.append(cx(i, say("assistant", "still working on round \(i)")))
                cxl.append(cx(i, out("c\(i)c")))
            }
            if i % 3 == 2 {   // two runs of calls with a message not shown between them
                cxl.append(cx(i, call("c\(i)d", "exec_command", #"{\"cmd\":\"ls\"}"#)))
                cxl.append(cx(i, out("c\(i)d")))
                cxl.append(cx(i, say("user", "<environment_context>y</environment_context>")))
                cxl.append(cx(i, call("c\(i)e", "Artifact", #"{\"action\":\"publish\",\"title\":\"Page \#(i)\"}"#)))
                cxl.append(cx(i, #"{"type":"function_call_output","call_id":"c\#(i)e","output":"https://claude.ai/artifact/a\#(i)"}"#))
            }
            cxl.append(cx(i, say("assistant", "Wrote [plan](docs/plan-\(i).md). See https://github.com/o/r/pull/\(i) and run `npm test -- r\(i)`.")))
        }
        try Data((cxl.joined(separator: "\n") + "\n").utf8).write(to: tmp)
        let wantCx = Resources.collect(file: tmp, home: "/home/me") ?? []
        for size in [150, 400, 1000, 3000] {
            var p = Resources.Progressive(chunk: size)
            var ok = p.update(tmp)
            let first = p.items(home: "/home/me")
            var n = 0
            while ok, !p.done, n < 10_000 { ok = p.older(tmp, home: "/home/me"); n += 1 }
            check(ok && p.done && wantCx.count >= 18 && (size > 1000 || n > 0) && first.first == wantCx.first && p.items(home: "/home/me") == wantCx,
                  "resources: a Codex rollout read newest first in \(size)-byte chunks gives what one read gives (\(n) older chunks)")
        }
        let cutLines: [(String, Bool)] = [
            (cx(0, say("user", "hi")), true), (cx(0, say("assistant", "done")), true),
            (cx(0, say("user", "# AGENTS.md instructions")), false), (cx(0, say("user", "<environment_context>x</environment_context>")), false),
            (cx(0, call("c", "exec_command", "{}")), false), (cx(0, out("c")), false),
            (#"{"timestamp":"2026-11-03T02:00:00.000Z","type":"event_msg","payload":{"type":"user_message","message":"hi"}}"#, false),
        ]
        check(cutLines.allSatisfy { Resources.isPrompt(Data($0.0.utf8)) == $0.1 }, "resources: a Codex chunk ends only at a message line with text")
    } catch { check(false, "resources: reader temp file (\(error))") }
    // Two reads joined (the older first) give what one read of both gives.
    do {
        let parsed = SessionLog.parse(lines: log.split(separator: "\n").map(String.init))
        var whole = Resources.Acc(); whole.add(parsed, home: "/home/me")
        var same = true
        for i in 1..<parsed.count {
            if case .toolChips = parsed[i].kind, case .toolChips = parsed[i - 1].kind { continue }   // a run of chips is one item
            var a = Resources.Acc(), b = Resources.Acc()
            a.add(Array(parsed[..<i]), home: "/home/me"); b.add(Array(parsed[i...]), home: "/home/me")
            if a.appending(b).finish(home: "/home/me") != whole.finish(home: "/home/me") { same = false; print("  join differs at \(i)") }
        }
        check(same, "resources: two reads joined give what one read gives (\(parsed.count) cuts)")
    }
    // Where a chunk may end: a typed prompt, not a tool's result, a subagent's line or a note.
    let promptLines: [(String, Bool)] = [
        (#"{"parentUuid":null,"isSidechain":false,"type":"user","message":{"role":"user","content":"hi"},"uuid":"u1"}"#, true),
        (#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":"x"}]}}"#, false),
        (#"{"isSidechain":true,"type":"user","message":{"role":"user","content":"hi"}}"#, false),
        (#"{"isMeta":true,"type":"user","message":{"role":"user","content":"note"}}"#, false),
        (#"{"type":"user","message":{"role":"user","content":"<system-reminder>x</system-reminder>"}}"#, false),
        (#"{"type":"assistant","message":{"role":"assistant","content":"\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"x"}}"#, false),
    ]
    check(promptLines.allSatisfy { Resources.isPrompt(Data($0.0.utf8)) == $0.1 }, "resources: a chunk ends only at a typed prompt")
    // A file: URL on this machine is a doc (agy links its files so), as the helper reads it.
    func docs(_ t: String) -> [String] { Resources.detect(t).filter { $0.kind == "doc" }.map(\.target) }
    check(docs("[a](file:///home/me/My%20Notes/plan.md)") == ["/home/me/My Notes/plan.md"], "resources: a file URL, percent-decoded")
    check(docs("[a](file:///home/me/%E7%AD%86%E8%A8%98/r.md)") == ["/home/me/筆記/r.md"], "resources: a file URL, UTF-8 decoded")
    check(docs("[a](file:///home/me/r.md#L12)") == ["/home/me/r.md"], "resources: a file URL without its #…")
    check(docs("[a](FILE://localhost/home/me/r.png)") == ["/home/me/r.png"], "resources: a file://localhost URL")
    for t in ["[a](file://otherhost/home/me/r.md)", "[a](file:///home/me/src/main.ts)", "[a](file:///home/me/r.md%)", "[a](file:////srv/r.md)",
              "[a](file://%2Fhome/me/r.md)", "[a](file://localhost%2Fhome/me/r.md)", "[a](file:///home/me/r%+9.md)", "[a](file:///home/me/r%0A.md)"] {
        check(docs(t).isEmpty, "resources: no doc in \(t) (\(Resources.detect(t).map(\.target)))")
    }
    // agy: a reply's file:// links and the Path its file tool named (synthetic lines of agy's transcript shape).
    do {
        let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("res-agy-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: tmp) }
        try FileManager.default.createDirectory(atPath: tmp.path + "/proj/docs/shots", withIntermediateDirectories: true)
        try FileManager.default.createDirectory(atPath: tmp.path + "/notes/reports", withIntermediateDirectories: true)
        try FileManager.default.createDirectory(atPath: tmp.path + "/repos", withIntermediateDirectories: true)
        let root = tmp.path.withCString { Darwin.realpath($0, nil) }.map { p in defer { free(p) }; return String(cString: p) } ?? tmp.path
        for (f, s) in [("proj/docs/plan.md", "# The plan"), ("proj/docs/shots/My Shot.png", "png"), ("proj/docs/guide.md", "# Guide"), ("notes/reports/y.md", "# Y")] {
            try Data(s.utf8).write(to: URL(fileURLWithPath: "\(root)/\(f)"))
        }
        func agy(_ step: Int, _ ty: String, _ extra: [String: Any]) -> String {
            var o: [String: Any] = ["step_index": step, "source": ty == "USER_INPUT" ? "USER_EXPLICIT" : "MODEL", "type": ty, "status": "DONE",
                                    "created_at": "2026-11-03T01:40:0\(step).000Z"]
            o.merge(extra) { $1 }
            return String(decoding: try! JSONSerialization.data(withJSONObject: o), as: UTF8.self)
        }
        let p = "\(root)/proj"
        let lines = [
            agy(0, "USER_INPUT", ["content": "<USER_REQUEST>\nwrite the plan\n</USER_REQUEST>"]),
            agy(1, "PLANNER_RESPONSE", ["tool_calls": [["name": "write_file", "args": ["Path": "\(p)/docs/plan.md", "Content": "# The plan"]]]]),
            agy(2, "GENERIC", ["content": "wrote"]),
            agy(3, "PLANNER_RESPONSE", ["content": "Done.\n\n- [plan.md](file://\(p)/docs/plan.md)\n- [My Shot.png](file://\(p)/docs/shots/My%20Shot.png)\n- [guide.md](file://localhost\(p)/docs/guide.md#L3)\n- [steps.ts](file://\(p)/src/steps.ts)\n- [other](file://otherhost\(p)/docs/plan.md)"]),
            agy(4, "USER_INPUT", ["content": "<USER_REQUEST>\ngo\n</USER_REQUEST>"]),
            agy(5, "PLANNER_RESPONSE", ["tool_calls": [["name": "read_file", "args": ["Path": "\(root)/notes/reports/y.md"]]]]),
            agy(6, "GENERIC", ["content": "# Y"]),
            agy(7, "PLANNER_RESPONSE", ["content": "See `reports/y.md`."]),
        ]
        let got = Resources.collect(lines: lines, home: "/home/me", agent: "\(root)/repos").filter { $0.kind == "doc" }.map(\.target).sorted()
        check(got == ["\(p)/docs/guide.md", "\(p)/docs/plan.md", "\(p)/docs/shots/My Shot.png", "\(root)/notes/reports/y.md"].sorted(),
              "resources: agy's file:// docs and a relative doc found by its tool's Path (\(got.map { ($0 as NSString).lastPathComponent }))")
    } catch { check(false, "resources: agy temp files (\(error))") }
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
