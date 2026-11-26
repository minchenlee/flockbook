import Foundation
@testable import HerdrKit

// The "Since you left" card (Glance, SinceLeft, SinceStore) and Summaries (Summary). The shared cases come from
// Tests/HerdrKitTests/Fixtures/since-vectors.json, which the mod (herdr-app/test/since-vectors.test.mjs) and the helper
// (xplat/flockbook-modfeed/tests/since_vectors.rs) read too; the rest follow herdr-app/test/since.test.mjs,
// herdr-app/test/summary.test.mjs, xplat/flockbook-modfeed/tests/glance.rs and xplat/flockbook-modfeed/src/summary.rs.

private let MIN: Int64 = 60_000

/// A fresh folder for one check, removed after `body`.
private func scratch<T>(_ body: (URL) async throws -> T) async rethrows -> T {
    let root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hksince-\(UUID().uuidString)")
    try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    return try await body(root)
}

private func str(_ v: Any?) -> String? { v as? String }
private func facts(_ o: Any?) -> Glance.Facts? {
    guard let o = o as? [String: Any] else { return nil }
    return Glance.Facts(edited: o["edited"] as? Int ?? 0, commands: o["commands"] as? Int ?? 0, failed: str(o["failed"]), tests: str(o["tests"]), test: str(o["test"]))
}

func checksSinceVectors() throws {
    let v = try JSONSerialization.jsonObject(with: fixture("since-vectors.json")) as! [String: Any]
    check(v["away_ms"] as? Int == Int(SinceLeft.awayMs), "since vectors: 5 minutes away")
    for c in v["span"] as! [[Any]] { check(SinceLeft.span(Int64(c[0] as! Int)) == c[1] as! String, "since vectors: span \(c[0]) = \(c[1])") }
    for c in v["words"] as! [[Any]] { check(SinceLeft.words(c[0] as! String) == c[1] as! Int, "since vectors: words in \(c[0])") }
    for c in v["about"] as! [[Any]] { check(SinceLeft.about(c[0] as! Int) == c[1] as! String, "since vectors: about \(c[0]) = \(c[1])") }
    for c in v["count_line"] as! [[Any]] {
        let s = c[0] as! [String: Any]
        let got = SinceLeft.countLine(.init(away: s["away"] as! String, messages: s["messages"] as! Int, words: s["words"] as! Int))
        check(got == c[1] as! String, "since vectors: count line \(got)")
    }
    for c in v["arrive"] as! [[String: Any]] {
        let seen = (c["seen"] as? [String: Any]).map { SinceLeft.Seen(id: $0["id"] as! String, at: Int64($0["at"] as! Int)) }
        let want = (c["want"] as? [String: Any]).map { SinceLeft.Away(from: Int64($0["from"] as! Int), back: Int64($0["back"] as! Int)) }
        check(SinceLeft.arrive(seen: seen, id: str(c["id"]), now: Int64(c["now"] as! Int)) == want, "since vectors: arrive, \(c["why"]!)")
    }
    for c in v["facts"] as! [[Any]] { check(Glance.line(facts(c[0])) == c[1] as! String, "since vectors: the line from the log reads \(c[1])") }
    for c in v["test_names"] as! [[Any]] { check(Glance.testName(c[0] as! String) == str(c[1]), "since vectors: test name of \(c[0])") }
    for c in v["first_sentence"] as! [[Any]] {
        let got = Glance.firstSentence(c[0] as! String)
        check(got == str(c[1]), "since vectors: first sentence of \(c[0]) is \(got ?? "nil")")
    }
    for l in v["logs"] as! [[String: Any]] {
        let items = SessionLog.parse(lines: l["lines"] as! [String])
        let g = Glance.facts(items, agent: l["agent"] as! String), w = l["want"] as! [String: Any], name = l["name"] as! String
        check(g.edited == w["edited"] as! Int && g.commands == w["commands"] as! Int, "since vectors: \(name): counts \(g.edited) \(g.commands)")
        check(g.failed == str(w["failed"]) && g.tests == str(w["tests"]) && g.test == str(w["test"]), "since vectors: \(name): \(g.failed ?? "-") \(g.tests ?? "-") \(g.test ?? "-")")
        check(g.said == str(w["said"]), "since vectors: \(name): said \(g.said ?? "nil")")
        check(Glance.line(g) == l["line"] as! String, "since vectors: \(name): line \(Glance.line(g))")
        check((g.id != nil) == !items.isEmpty && (g.ended != nil) == !items.isEmpty, "since vectors: \(name): an id and a time when there is a turn")
    }
    for c in v["summary_parse"] as! [[Any]] {
        let want = (c[1] as? [Any]).map { ($0[0] as! String, str($0[1])) }
        let got = Summary.parse(c[0] as! String)
        check(got?.state == want?.0 && got?.needs == want?.1 && (got == nil) == (want == nil), "since vectors: summary answer \(c[0])")
    }
    for c in v["model_name"] as! [[Any]] { check(Summary.modelName(c[0] as! String) == str(c[1]), "since vectors: model name \(c[0])") }
    let si = v["summary_input"] as! [String: Any]
    let got = Summary.input(SessionLog.parse(lines: si["lines"] as! [String]), agent: si["agent"] as! String)
    check(got?.text == si["text"] as? String, "since vectors: what the model reads\n\(got?.text ?? "nil")")
    let t = v["texts"] as! [String: Any]
    check(t["card_title"] as? String == SinceLeft.Texts.title && t["summary_mark"] as? String == SinceLeft.Texts.summaryMark
          && t["needs_prefix"] as? String == SinceLeft.Texts.needsPrefix && t["from_log"] as? String == SinceLeft.Texts.fromLog
          && t["from_log_with_summary"] as? String == SinceLeft.Texts.fromLogWithSummary && t["jump"] as? String == SinceLeft.Texts.jump, "since vectors: the card's words")
    check(t["summaries_about"] as? String == Summary.Texts.summariesAbout && t["summaries_sent"] as? String == Summary.Texts.sent
          && t["by_about"] as? String == Summary.Texts.byAbout && (t["by_notes"] as? [String: String])?.filter { $0.key != "own" } == Summary.Texts.byNotes.filter { $0.key != "own" }
          && t["model_about"] as? String == Summary.Texts.modelAbout && t["model_empty"] as? [String: String] == Summary.Texts.modelEmpty
          && t["model_source"] as? [String: String] == Summary.Texts.modelSource, "since vectors: the settings' words")
    check((t["model_empty"] as? [String: String])?["claude"] == Summary.claudeModel, "since vectors: Claude Code's default model")
    // One known difference: "Each agent's own" says on the Mac that the program runs in herdr-core (checksSummaryOwnNote).
    check((t["by_notes"] as? [String: String])?["own"] != Summary.Texts.byNotes["own"] && Summary.Texts.byNotes.keys == (t["by_notes"] as? [String: String])?.keys,
          "since vectors: the settings' words: only the note of Each agent's own differs, on purpose")
    for c in v["usage_text"] as! [[Any]] {
        let u = c[0] as! [String: [String: Int]]
        func use(_ x: [String: Int]) -> Summary.Usage { .init(runs: x["runs"]!, failed: x["failed"]!, sent: x["sent"]!, got: x["got"]!) }
        check(Summary.usageText(today: use(u["today"]!), week: use(u["week"]!)) == c[1] as! String, "since vectors: usage line \(c[1])")
    }
    check(v["summary_batch"] as? Int == Summary.batch, "since vectors: three agents at a time")
}

private func item(_ id: String, _ kind: ConversationItem.Kind, _ ms: Int64?) -> ConversationItem {
    ConversationItem(id: id, kind: kind, timestamp: ms.map { Date(timeIntervalSince1970: Double($0) / 1000) })
}

func checksSinceCard() {
    // since.test.mjs "counts the agent's messages that came while away; names the owner's last message".
    let items = [
        item("u0", .userText("Old one"), 1 * MIN),
        item("a0", .assistantText("seen before"), 2 * MIN),
        item("n1", .userText(#"<hc from="t2 docs-writer">a note from an agent</hc>"#), 9 * MIN),
        item("u1", .userText("\nFix the importer\nand the tests"), 10 * MIN),
        item("a1", .assistantText("one two three"), 20 * MIN),
        item("t1", .toolChips([]), 21 * MIN),
        item("a2", .assistantText("four five"), 30 * MIN),
        item("a3", .assistantText("after the owner came back"), 70 * MIN),
        item("n2", .userText(#"<hc from="t3 lead">later note</hc>"#), 71 * MIN),
    ]
    let g = Glance.Facts(id: "a3", edited: 5, commands: 2, tests: "passed", test: "pytest")
    let utc = TimeZone(identifier: "UTC")!
    let s = SinceLeft.card(items, away: .init(from: 12 * MIN, back: 59 * MIN), facts: g, timeZone: utc)
    check([s.away, "\(s.messages)", "\(s.words)"] == ["47 minutes", "2", "5"], "since card: away, messages and words that came while away")
    check(s.facts == "Edited 5 files · pytest passed after the last edit", "since card: the line from the log")
    check(s.said == .init(id: "u1", at: "00:10", text: "Fix the importer"), "since card: the owner's last message, not an agent's note: \(String(describing: s.said))")
    check(SinceLeft.countLine(s) == "You were away 47 minutes · 2 new messages · about 10 words", "since card: count line")
    check(SinceLeft.factsLine(s, summary: false) == "From the log: Edited 5 files · pytest passed after the last edit"
          && SinceLeft.factsLine(s, summary: true) == "From the log, not from the summary: Edited 5 files · pytest passed after the last edit", "since card: from the log, with and without a summary")
    check(SinceLeft.saidLine(s.said!) == "You said at 00:10: Fix the importer", "since card: you said")
    let empty = SinceLeft.card([], away: .init(from: 0, back: 6 * MIN), facts: nil)
    check(SinceLeft.countLine(empty).hasPrefix("You were away 6 minutes · no new message") && empty.said == nil && SinceLeft.factsLine(empty, summary: false).isEmpty, "since card: nothing came")
    let noTime = SinceLeft.card([item("u9", .userText("Go"), nil)], away: .init(from: 0, back: 6 * MIN), facts: nil)
    check(noTime.said.map(SinceLeft.saidLine) == "You said: Go", "since card: a message with no time")
    check(SinceLeft.clock(14 * 60 * MIN + 2 * MIN, timeZone: utc) == "14:02", "since card: clock")
}

func checksSinceMarks() async {
    await scratch { dir in
        await MainActor.run {
            let k = SinceStore.key(session: nil, terminal: "t1"), other = SinceStore.key(session: "c9-input-test", terminal: "t1")
            check(k == "|t1" && k != other, "since marks: the session is part of the key")
            let s = SinceStore(dir: dir)
            check(s.seen.isEmpty && s.summaries.isEmpty, "since marks: none at first")
            s.markSeen(k, id: nil, now: 1000)
            check(s.seen[k] == nil, "since marks: no turn, no mark")
            s.markSeen(k, id: "a1", now: 10 * MIN)
            s.markSeen(k, id: "a1", now: 10 * MIN + 30_000)
            check(s.seen[k] == .init(id: "a1", at: 10 * MIN), "since marks: the same turn moves the time at most once a minute")
            s.markSeen(k, id: "a1", now: 12 * MIN)
            check(s.seen[k] == .init(id: "a1", at: 12 * MIN), "since marks: a minute later it moves")
            s.markSeen(k, id: "a2", now: 20 * MIN)
            check(s.seen[k] == .init(id: "a2", at: 20 * MIN, since: 12 * MIN), "since marks: a new turn keeps when it was on screen before")
            let again = SinceStore(dir: dir)
            check(again.seen[k] == s.seen[k] && again.seen[other] == nil, "since marks: kept in the folder over a restart")
            check(FileManager.default.fileExists(atPath: dir.appendingPathComponent("since-marks.json").path), "since marks: since-marks.json")
            // Back 47 minutes later with a new turn: a card; less than 5 minutes, or the same turn: none.
            check(SinceLeft.arrive(seen: again.seen[k], id: "a3", now: 67 * MIN) == .init(from: 20 * MIN, back: 67 * MIN), "since marks: back after 47 minutes")
            check(SinceLeft.arrive(seen: again.seen[k], id: "a2", now: 67 * MIN) == nil && SinceLeft.arrive(seen: again.seen[k], id: "a3", now: 24 * MIN) == nil, "since marks: nothing new, or back too soon")
            s.keep(k, .init(id: "a2", by: "own/", state: "Done."))
            s.keep(other, .init(id: "x", by: "own/", failed: true))
            s.prune(keeping: [k])
            check(SinceStore(dir: dir).summaries == [k: .init(id: "a2", by: "own/", state: "Done.")], "since marks: summaries kept, gone conversations dropped")
            s.noteRun(at: 5 * MIN, by: "claude", model: "claude-haiku-5-5", sent: 900, got: 40, ms: 1200, ok: true)
            s.noteRun(at: 9 * MIN, by: "claude", model: "claude-haiku-5-5", sent: 100, got: 0, ms: 60000, ok: false)
            let u = s.usage(since: [0, 6 * MIN], now: 10 * MIN)
            check(u == [.init(runs: 2, failed: 1, sent: 1000, got: 40, ms: 61200), .init(runs: 1, failed: 1, sent: 100, got: 0, ms: 60000)], "since marks: runs counted from each time")
            let line = (try? String(contentsOf: dir.appendingPathComponent("summary-usage.jsonl"), encoding: .utf8)) ?? ""
            check(line.contains("\"sent\":900") && !line.contains("Done."), "since marks: a run line holds counts only")
        }
    }
}

func checksSummaryArgv() {
    let (a, model) = Summary.argv("claude")!
    check(a[0] == "claude" && model == Summary.claudeModel, "summary argv: Claude Code, its small model")
    for flag in ["-p", "--no-session-persistence", "--disable-slash-commands", "--strict-mcp-config"] { check(a.contains(flag), "summary argv: claude \(flag)") }
    func at(_ a: [String], _ f: String) -> String? { a.firstIndex(of: f).map { a[$0 + 1] } }
    check(at(a, "--tools") == "" && at(a, "--setting-sources") == "" && at(a, "--model") == Summary.claudeModel && at(a, "--system-prompt") == Summary.system, "summary argv: claude without tools, settings; the fixed text")
    check(!a.contains("--bare"), "summary argv: no --bare (it reads no login)")
    check(Summary.argv("claude", program: "/x/fake")!.argv[0] == "/x/fake", "summary argv: another program path")
    let s = Summary.argv("claude", model: "claude-sonnet-5-5")!
    check(at(s.argv, "--model") == "claude-sonnet-5-5" && s.model == "claude-sonnet-5-5", "summary argv: a chosen model")
    check(Summary.argv("agy") == nil && Summary.argv("own") == nil, "summary argv: no other program")
    let c = Summary.argv("codex")!
    check(c.argv[0] == "codex" && c.argv[1] == "exec" && c.argv.last == "-" && at(c.argv, "--sandbox") == "read-only", "summary argv: codex reads only, the text on stdin")
    for flag in ["--ephemeral", "--ignore-rules", "--ignore-user-config", "--skip-git-repo-check"] { check(c.argv.contains(flag), "summary argv: codex \(flag)") }
    check(!c.argv.contains("-m") && c.model == "Codex's default model" && !c.argv.contains { $0.contains("dangerously") }, "summary argv: codex default model")
    check(at(Summary.argv("codex", model: "gpt-x-mini")!.argv, "-m") == "gpt-x-mini", "summary argv: codex -m")
    check(Summary.stdin("codex", "T").hasPrefix(Summary.system) && Summary.stdin("codex", "T").hasSuffix("\n\nT") && Summary.stdin("claude", "T") == "T", "summary argv: Codex gets the fixed text on stdin")
    let p = Summary.argv("pi")!
    for flag in ["-p", "--no-session", "--no-tools", "--no-extensions", "--no-mcp", "--no-skills", "--no-context-files", "--no-approve"] { check(p.argv.contains(flag), "summary argv: pi \(flag)") }
    check(!p.argv.contains("--model") && p.model == "pi's default model" && at(Summary.argv("pi", model: "opencode-go/glm-5.3-flash")!.argv, "--model") == "opencode-go/glm-5.3-flash", "summary argv: pi models")
    check(Summary.endOf("abc", 5) == "abc" && Summary.endOf("añcdé", 2) == "…dé", "summary: the end of a long text")
    check(Summary.parse("STATE: " + String(repeating: "é", count: 400))!.state.count == Summary.lineCap + 1, "summary: a long line is cut, with the mark")
    let table = "provider     model            context  max-out  thinking  images\nopencode-go  glm-5.3-flash    1M       131.1K   yes       yes\nopenrouter   x/y:free         262K     64K      no        no\n\nbad;row  a b\n"
    check(Summary.piModels(table).map { "\($0.id) \($0.note)" } == ["opencode-go/glm-5.3-flash 1M context", "openrouter/x/y:free 262K context"], "summary: pi's models")
    let file = #"{"models": [{"slug": "gpt-x", "description": "Workhorse.", "visibility": "list"}, {"slug": "gpt-hidden", "visibility": "hide"}, {"slug": "--bad", "visibility": "list"}]}"#
    check(Summary.codexModels(file).map { "\($0.id) \($0.note)" } == ["gpt-x Workhorse."] && Summary.codexModels("not json").isEmpty && Summary.codexModels("").isEmpty, "summary: Codex's models")
    check(Summary.claudeModels[0].id == Summary.claudeModel, "summary: Claude Code's list starts with the default")
    let all = (0..<11).map { (id: "go/m\($0)", note: "1M context") } + [(id: "go/glm-flash", note: "262K context")]
    check(Summary.matches(all, typed: "glm", saved: "").map(\.id) == ["go/glm-flash"] && Summary.matches(all, typed: "go/m1", saved: "go/m1").count == 12, "summary: typing narrows the list; the saved name shows all")
    check(Summary.Texts.empty("codex", failed: true) == "Empty: the model Codex uses by default. The list of models could not be read." && Summary.Texts.more(4) == "4 more. Type a part of a name to see fewer.", "summary: the model row's notes")
}

func checksSummaryNext() {
    // summary.test.mjs: the stopped agents, the newest first, three at a time, each turn once.
    let now: Int64 = 1_000 * MIN
    func cand(_ t: String, _ status: AgentStatus = .done, kind: String = "claude", ended: Int64 = 1_000 * MIN - MIN, id: String? = nil, said: String? = "The parser skips empty lines.") -> Summary.Candidate {
        .init(key: t, kind: kind, status: status, facts: .init(id: id ?? "\(t)-last", ended: ended, edited: 2, commands: 1, tests: "passed", test: "pytest", said: said))
    }
    let on = Summary.Settings(on: true)
    check(Summary.Settings() == .init(on: false, by: "own", model: ""), "summary next: off by default, each agent's own program")
    let agents = [cand("t1"), cand("t2", ended: now - 2 * MIN), cand("t3", .working), cand("t4", kind: "agy"), cand("t5", ended: now - 13 * 60 * MIN), cand("t6", .blocked), cand("t7", said: nil)]
    var have: [String: Summary.Kept] = [:]
    check(Summary.next(agents, have: have, settings: on).map(\.key) == ["t1", "t2", "t5"], "summary next: stopped agents, the newest first; one that stopped 13 hours ago too; none working, waiting, unsupported or silent")
    have["t1"] = .init(id: "t1-last", by: "own/", state: "Fixed.")
    have["t2"] = .init(id: "t2-last", by: "own/", failed: true)
    check(Summary.next(agents, have: have, settings: on).map(\.key) == ["t5"], "summary next: each turn once; a failed turn is not asked again")
    check(Summary.next([cand("t1", id: "t1-next")], have: have, settings: on).map(\.key) == ["t1"], "summary next: the agent wrote more: asked again")
    have["t5"] = .init(id: "t5-last", by: "own/", v: 4, state: "Old words.")
    check(Summary.next(agents, have: have, settings: on).map(\.key) == ["t5"] && Summary.shown(have["t5"], id: "t5-last", settings: on)?.state == "Old words.", "summary next: a summary kept by an older version shows and is asked once more")
    let many = (1...5).map { cand("t\($0)") }
    check(Summary.next(many, have: [:], settings: on).map(\.key) == ["t1", "t2", "t3"], "summary next: at most three at a time")
    let pi = Summary.Settings(on: true, by: "pi", model: " opencode-go/glm-5.3-flash ")
    check(pi.writer == "pi/opencode-go/glm-5.3-flash" && on.writer == "own/", "summary next: the writer is the program and model")
    check(Summary.next(agents, have: have, settings: pi).map(\.key) == ["t1", "t4", "t2"], "summary next: one program for all: every agent's turn again, other programs' agents too")
    check(Summary.shown(have["t1"], id: "t1-last", settings: on)?.state == "Fixed.", "summary shown: the turn's words")
    check(Summary.shown(have["t1"], id: "t1-last", settings: .init(on: false)) == nil, "summary shown: off: none, also kept from before")
    check(Summary.shown(have["t1"], id: "t1-next", settings: on) == nil && Summary.shown(have["t1"], id: "t1-last", settings: pi) == nil, "summary shown: another turn or writer: none")
    check(Summary.shown(have["t2"], id: "t2-last", settings: on) == nil, "summary shown: a failed one shows nothing")
}

/// A program that stands in for the model: it keeps its stdin and arguments and answers as told.
private func stub(_ dir: URL, answer: String, code: Int = 0) -> String {
    let p = dir.appendingPathComponent("fake-model")
    let script = "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"\(dir.path)/args\"\ncat > \"\(dir.path)/stdin\"\nprintf '%s' '\(answer)'\nexit \(code)\n"
    try? Data(script.utf8).write(to: p)
    try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: p.path)
    return p.path
}

func checksSummaryAsk() async {
    let lines = [
        #"{"type":"user","uuid":"u1","timestamp":"2026-11-03T01:40:00.000Z","message":{"role":"user","content":"Fix the parser"}}"#,
        #"{"type":"assistant","uuid":"a3","timestamp":"2026-11-03T01:40:05.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Fixed it. Tests pass."}]}}"#,
    ]
    let items = SessionLog.parse(lines: lines)
    await scratch { dir in
        let store = await MainActor.run { SinceStore(dir: dir.appendingPathComponent("app")) }
        let ok = stub(dir, answer: "a plugin said hello\nSTATE: The parser is fixed.\nNEEDS: Say whether to update the changelog.\n")
        let r = await Summary.ask(items: items, kind: "claude", settings: .init(on: true), runner: ProcessRunner(), store: store, program: ok, cwd: dir.appendingPathComponent("run").path, now: { 7 })
        if case .success(let s) = r {
            check(s.state == "The parser is fixed." && s.needs == "Say whether to update the changelog." && s.model == Summary.claudeModel && !s.id.isEmpty, "summary ask: the two lines from the stub")
        } else { check(false, "summary ask: the stub answers: \(r)") }
        let stdin = (try? String(contentsOf: dir.appendingPathComponent("stdin"), encoding: .utf8)) ?? ""
        let args = (try? String(contentsOf: dir.appendingPathComponent("args"), encoding: .utf8)) ?? ""
        check(stdin.hasPrefix("OWNER ASKED:\nFix the parser\n\nAGENT WROTE:\nFixed it. Tests pass.") && args.contains("--no-session-persistence"), "summary ask: the turn goes on stdin, with the fixed arguments")
        let u = await MainActor.run { store.usage(since: [0], now: 10) }
        check(u.first?.runs == 1 && u.first?.failed == 0 && (u.first?.sent ?? 0) > Summary.system.count, "summary ask: the run is counted")
        let bad = stub(dir, answer: "nope", code: 3)
        if case .failure(.failed(let m)) = await Summary.ask(items: items, kind: "claude", settings: .init(on: true), runner: ProcessRunner(), store: store, program: bad) {
            check(m.hasPrefix("The model's program failed."), "summary ask: a failing program")
        } else { check(false, "summary ask: a failing program fails") }
        let shape = stub(dir, answer: "I think it is fine.")
        if case .failure(.failed(let m)) = await Summary.ask(items: items, kind: "claude", settings: .init(on: true), runner: ProcessRunner(), store: store, program: shape) {
            check(m == "The model's answer had no STATE line.", "summary ask: an answer in another shape")
        } else { check(false, "summary ask: an answer in another shape fails") }
        if case .failure(.failed(let m)) = await Summary.ask(items: items, kind: "claude", settings: .init(on: true), runner: ProcessRunner(), store: store, program: dir.appendingPathComponent("none").path) {
            check(m.contains("was not found"), "summary ask: a missing program")
        } else { check(false, "summary ask: a missing program fails") }
        let runs = await MainActor.run { store.usage(since: [0], now: 10) }
        check(runs.first?.runs == 3 && runs.first?.failed == 2, "summary ask: failed runs counted; a missing program is no run")
        if case .failure(.unsupported) = await Summary.ask(items: items, kind: "agy", settings: .init(on: true), runner: ProcessRunner(), store: store, program: ok) {} else { check(false, "summary ask: own program of an agent that writes none") }
        if case .failure(.none) = await Summary.ask(items: Array(items.prefix(1)), kind: "claude", settings: .init(on: true), runner: ProcessRunner(), store: store, program: ok) {} else { check(false, "summary ask: nothing written: nothing asked") }
        // One program for every agent, with a model named: that program and model.
        _ = await Summary.ask(items: items, kind: "agy", settings: .init(on: true, by: "pi", model: "go/glm"), runner: ProcessRunner(), store: store, program: ok)
        let piArgs = (try? String(contentsOf: dir.appendingPathComponent("args"), encoding: .utf8)) ?? ""
        check(piArgs.contains("--no-tools\n") && piArgs.contains("--model\ngo/glm\n"), "summary ask: pi with the chosen model for another agent's turn")
    }
    let env = ProcessRunner.environment(["PATH": "/usr/bin", "CLAUDECODE": "1", "CLAUDE_CODE_ENTRYPOINT": "cli", "CLAUDE_CODE_USE_BEDROCK": "1", "HOME": "/h"], home: "/h")
    check(env["CLAUDECODE"] == nil && env["CLAUDE_CODE_ENTRYPOINT"] == nil && env["PATH"]!.hasPrefix("/usr/bin:/h/.local/bin:"), "summary ask: no nested-session marks; the usual program folders on PATH")
    check(env["CLAUDE_CODE_USE_BEDROCK"] == "1" && env["HOME"] == "/h", "summary ask: the provider and login settings stay")
    await scratch { dir in
        func script(_ name: String, _ body: String) -> String {
            let p = dir.appendingPathComponent(name)
            try? Data("#!/bin/sh\n\(body)\n".utf8).write(to: p)
            try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: p.path)
            return p.path
        }
        let began = Date()
        let slow = await ProcessRunner().run(argv: [script("slow", "sleep 30")], input: "", cwd: dir.path, timeout: 1)
        check(slow.timedOut && slow.code == nil && Date().timeIntervalSince(began) < 10, "process runner: a program past its time is stopped")
        let began2 = Date()
        let left = await ProcessRunner().run(argv: [script("left", "sleep 30 &\necho STATE: done")], input: "", cwd: dir.path, timeout: 20)
        check(left.code == 0 && left.stdout.contains("STATE: done") && Date().timeIntervalSince(began2) < 10, "process runner: a child that keeps the pipe open does not hold the run (\(left))")
        let deaf = await ProcessRunner().run(argv: [script("deaf", "echo ok")], input: String(repeating: "x", count: 1_000_000), cwd: dir.path, timeout: 10)
        check(deaf.code == 0 && deaf.stdout == "ok\n", "process runner: a program that reads no stdin")
    }
}

func checksSinceJump() {
    // The card's Jump: the owner's message comes into the window (with up to maxRows after it), then the view goes to it.
    let ids = (0..<400).map { "m\($0)" }
    var w = ConversationWindow(shown: ConversationWindow.page)
    check(w.reveal("m350", ids) && w.range(ids) == 300..<400 && !w.detached, "since jump: a row in the window: nothing moves")
    check(w.reveal("m120", ids) && w.range(ids).contains(120) && w.range(ids).lowerBound == 120 && w.range(ids).count == ConversationWindow.maxRows, "since jump: an older row: the window starts at it, at most maxRows")
    check(w.detached && w.range(ids).upperBound == 270, "since jump: newer rows wait behind Jump to latest")
    check(w.reveal("m380", ids) && w.range(ids).contains(380), "since jump: a newer row than the window")
    check(!w.reveal("gone", ids), "since jump: a row not in the log: false")
    var near = ConversationWindow(shown: ConversationWindow.page)
    check(near.reveal("m260", ids) && near.range(ids) == 260..<400 && !near.detached, "since jump: the window grows to the newest when it can")
}

func checksLogTails() async {
    await scratch { dir in
        let url = dir.appendingPathComponent("s.jsonl")
        let one = #"{"type":"user","uuid":"u1","timestamp":"2026-11-03T01:40:00.000Z","message":{"role":"user","content":"Go"}}"#
        let two = #"{"type":"assistant","uuid":"a1","timestamp":"2026-11-03T01:40:01.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Done. All good."}]}}"#
        try? Data((one + "\n" + two + "\n").utf8).write(to: url)
        let t = LogTails()
        let items = t.items(url) ?? []
        check(Glance.facts(items, agent: "claude").said == "Done.", "log tails: the end of a log, parsed")
        check(Glance.facts(t.items(url, bytes: two.utf8.count + 3) ?? [], agent: "claude").said == "Done.", "log tails: a cut first line is dropped")
        check(t.items(dir.appendingPathComponent("none.jsonl")) == nil, "log tails: no file")
    }
}
