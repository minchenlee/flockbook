import Foundation
@testable import HerdrKit

// Snapshot, session, conversation parsing, log tailing.

// Snapshot
@MainActor func checksSnapshot() async throws {
    let ws = try SnapshotParser.parse(fixture("snapshot.json"))
    check(ws.count == 1, "one workspace")
    check(ws[0].tabs.map(\.label) == ["Main", "Shell"], "tab labels")
    check(ws[0].tabs[0].panes.count == 2, "two panes")
    check(ws[0].tabs[0].status == .blocked, "rollup blocked")
    check(ws[0].tabs[1].panes[0].agent == nil && ws[0].tabs[1].status == .unknown, "shell pane")
    let a = ws[0].tabs[0].panes[0].agent
    check(a?.name == "demo-dev" && a?.kind == "claude" && a?.sessionId == "sess-1" && a?.title == "Fix the bug", "agent fields")
    check(AgentStatus.rollup([.idle, .done, .working]) == .working && AgentStatus.rollup([AgentStatus]()) == .unknown, "rollup order")

    check(HerdrStore.sessionId(forPane: ws[0].tabs[0].panes[0].id, in: ws) == "sess-1", "sessionId(forPane:) finds agent session")
    check(HerdrStore.sessionId(forPane: "nope", in: ws) == nil && HerdrStore.sessionId(forPane: ws[0].tabs[1].panes[0].id, in: ws) == nil, "sessionId(forPane:) nil for unknown pane and shell pane")
    let idleStore = await HerdrStore(session: .named("hk-none"))
    let noPane = await idleStore.loadEarlier(paneId: "nope")
    check(noPane == .end, "loadEarlier(paneId:) false for unknown pane")
}

// Session
@MainActor func checksSession() async throws {
    check(HerdrSession.fromLaunch(arguments: ["app", "--session", "x"], environment: [:]).name == "x", "--session")
    check(HerdrSession.fromLaunch(arguments: ["app"], environment: ["HERDR_DESK_SESSION": "y"]).name == "y", "env session")
    check(HerdrSession.fromLaunch(arguments: ["app"], environment: [:]).name == nil, "default session")
    check(HerdrSession.named("c9-input-test").socketPath.hasSuffix("sessions/c9-input-test/herdr.sock"), "named socket")
}

// Conversation
@MainActor func checksConversation() async throws {
    check(describe(SessionLog.parse(lines: lines)) == [
        "user:Fix the login bug", "asst:Looking at it now.", "chips:Read 2 files|Ran a command",
        "asst:Tests pass.", "perm:rm -rf build",
    ], "maps lines")
    let tail = SessionLog.parse(lines: lines)
    check(describe(tail).last == "perm:rm -rf build" && tail.count == 5, "unanswered tail tool_use becomes permissionRequest")
    if case .permissionRequest(let r)? = tail.last?.kind {
        check(r.state == .pending && r.toolName == "Bash" && r.toolUseId == "t9", "permissionRequest is pending with tool info")
    } else { check(false, "last item is permissionRequest") }
    let answered = SessionLog.parse(lines: Array(lines.prefix(5)))
    check(!describe(answered).contains { $0.hasPrefix("perm:") }, "answered tool_use emits no permissionRequest")
    check(SessionLog.parse(lines: lines).map(\.id) == SessionLog.parse(lines: lines).map(\.id), "stable ids")
}

// Injected user lines: compaction -> status, system noise dropped, real + herdr-send messages kept
@MainActor func checksInjected() async throws {
    let inj = String(decoding: fixture("injected.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    check(describe(SessionLog.parse(lines: inj)) == [
        "user:Real question", "status", "status", "chips:Read a file",
        "user:[herdr-send from orchard-main-orch] please rebuild", "asst:On it.",
    ], "injected user lines")
    if case .status(let t)? = SessionLog.parse(lines: inj).dropFirst().first?.kind { check(t == "Context compacted", "compaction status text") }
    else { check(false, "compaction is a status item") }
}

// Messages typed while the agent works: Claude Code hands them over mid-turn as a `queued_command` attachment, not a
// user line. The owner's ones (origin human, or no origin from older clients) show as user text in log order; task notices,
// subagent hand-backs and sidechains do not.
@MainActor func checksQueued() async throws {
    let q = String(decoding: fixture("queued.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    let got = describe(SessionLog.parse(lines: q))
    check(got == [
        "user:Run the tests, then summarize.", "chips:Ran a command",
        "user:Also check the lint step.", "user:Here is the failing screen.", "user:An older client logs no origin.",
        "asst:Tests pass and lint is clean.", "user:Thanks.",
    ], "queued: messages taken mid-turn show as user text, in log order (\(got))")
    let ids = SessionLog.parse(lines: q).map(\.id)
    check(ids == SessionLog.parse(lines: q).map(\.id) && Set(ids).count == ids.count, "queued: ids stable and unique")
}

// Pasted text: Claude Code wraps it in <pasted_content id="…"> … </pasted_content id="…">; it is the owner's message
@MainActor func checksPasted() async throws {
    let p = String(decoding: fixture("pasted.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    check(describe(SessionLog.parse(lines: p)) == [
        "user:Ship the Orchard banner today.",
        "user:Please review this:\n\nline A\nline B\n\nThanks",
        "user:first\n\nsecond",
        "user:Relay: <pasted_content id=\"3\">inner</pasted_content id=\"3\"> end",
        "user:block form",
        "user:cut off at the end",
    ], "pasted: shown as user text, wrappers stripped, order kept, injected tags still hidden")
    check(ConversationParser.userVisible("<environment_context>cwd</environment_context>") == nil
          && ConversationParser.userVisible("[Request interrupted by user]") == nil
          && ConversationParser.userVisible("typed <b>text</b>") == "typed <b>text</b>", "pasted: allowlist of visible tags")
    let cxp = SessionLog.parse(lines: [
        #"{"timestamp":"2026-10-10T01:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<pasted_content id=\"1\">codex paste</pasted_content id=\"1\">"}]}}"#,
        #"{"timestamp":"2026-10-10T01:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}"#,
    ])
    check(describe(cxp) == ["user:codex paste"], "pasted: codex user text uses the same rule")
}

// A note another agent sent with `hc send`: one `<hc from="<terminal> [name]">…</hc>` envelope typed into the agent.
// It is the conversation, not injected context: a user item with its tag (the row decides how to draw it).
// herdr-core tests/conversation.rs `hc_notes_are_shown`, the same rules.
@MainActor func checksHCNotes() async throws {
    let note = "<hc from=\"t2 docs-worker\">Done: notes updated.\nSee notes.md</hc>"
    check(ConversationParser.userVisible(note) == note, "hc note: visible")
    let esc = note.replacingOccurrences(of: "\"", with: "\\\"").replacingOccurrences(of: "\n", with: "\\n")
    let got = describe(SessionLog.parse(lines: [
        #"{"timestamp":"2026-10-10T01:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"\#(esc)"}]}}"#,
        #"{"type":"user","uuid":"u1","timestamp":"2026-10-10T01:00:01.000Z","message":{"role":"user","content":"<pasted_content id=\"a1\">\n<hc from=\"t3\">Short note</hc>\n</pasted_content id=\"a1\">"}}"#,
        #"{"type":"user","uuid":"u2","timestamp":"2026-10-10T01:00:02.000Z","message":{"role":"user","content":"<hc from=\"t4 review\" reach=\"hc send t1 when done or stuck\">Review x.rs</hc>"}}"#,
    ]))
    check(got == ["user:" + note, "user:<hc from=\"t3\">Short note</hc>", "user:<hc from=\"t4 review\" reach=\"hc send t1 when done or stuck\">Review x.rs</hc>"],
          "hc note: Codex and Claude Code notes, pasted or not, are user items (\(got))")
    let n = ConversationParser.hcNote(note)
    check(n?.from == "t2 docs-worker" && n?.text == "Done: notes updated.\nSee notes.md", "hc note: the sender as hc wrote it and the text without its tag")
    let r = ConversationParser.hcNote("<hc from=\"t4 review\" reach=\"hc send t1 when done or stuck\">Review x.rs</hc>")
    check(r?.from == "t4 review" && r?.text == "Review x.rs", "hc note: other attributes")
    let q = ConversationParser.hcNote(#"<hc from="t2" reach="a > b">body</hc>"#)
    check(q?.from == "t2" && q?.text == "body", "hc note: a '>' inside a quoted attribute stays out of the body")
    let bare = ConversationParser.hcNote("  <hc>A note</hc>\n")
    check(bare?.from == "" && bare?.text == "A note", "hc note: no sender, the whole text trimmed")
    for typed in ["Fix it", "<hc from=\"t2\">a</hc> and more I typed", "see <hc from=\"t2\">a</hc>", "<hcx>a</hcx>",
                  "<hc from=\"t2\">a</hc><hc from=\"t3\">b</hc>", "<hc from=t2>a</hc>"] {
        check(ConversationParser.hcNote(typed) == nil, "hc note: not a note: \(typed)")
    }
    // The one line the Mac view draws, as the phone page does: the name after the terminal id, else the id, else "agent".
    check(ConversationParser.noteLine(from: "t2 docs-worker", text: "Done: notes updated.\nSee notes.md") == "↳ docs-worker: Done: notes updated. …",
          "hc note line: the name after the terminal id, the first line, and … when more lines follow")
    check(ConversationParser.noteLine(from: "t3", text: "\n  Short note\n\n") == "↳ t3: Short note", "hc note line: only the id, blank lines skipped")
    check(ConversationParser.noteLine(from: "", text: "A\tnote") == "↳ agent: A note", "hc note line: no sender is \"agent\", tabs as spaces")
}

// Codex rollout
@MainActor func checksCodexRollout() async throws {
    check(describe(SessionLog.parse(lines: cx)) == [
        "user:Review the parser", "asst:Checking the files.", "chips:Ran 2 commands", "asst:No P1 found.", "perm:rm -rf build",
    ], "codex maps lines")
    let cxRoot = NSTemporaryDirectory() + "hkcx-\(UUID().uuidString)"
    let cxDir = cxRoot + "/2026/10/09"
    try FileManager.default.createDirectory(atPath: cxDir, withIntermediateDirectories: true)
    try "x\n".write(toFile: cxDir + "/rollout-2026-10-09T00-00-00-cx-1.jsonl", atomically: true, encoding: .utf8)
    check(SessionLog.locateCodex(sessionId: "cx-1", sessionsDir: cxRoot)?.lastPathComponent.hasSuffix("-cx-1.jsonl") == true, "codex locate by id")
    check(SessionLog.locateCodex(sessionId: cxDir + "/rollout-2026-10-09T00-00-00-cx-1.jsonl")?.path == cxDir + "/rollout-2026-10-09T00-00-00-cx-1.jsonl", "codex locate by path")
    check(SessionLog.locateCodex(sessionId: "../cx-1", sessionsDir: cxRoot) == nil, "codex locate rejects ..")
    try? FileManager.default.removeItem(atPath: cxRoot)
}

// Tool call details
@MainActor func checksToolCalls() async throws {
    let calls = chipCalls(SessionLog.parse(lines: lines))
    check(calls.map(\.toolName) == ["Read", "Read", "Bash"], "calls: tool names \(calls.map(\.toolName))")
    check(calls.first?.summary == "/a.swift" && calls.first?.output == "x" && calls.first?.isError == false, "calls: Read summary + output")
    check(calls.first?.input.contains("file_path") == true, "calls: input is full JSON")
    var late = ConversationParser()
    late.feed(line: #"{"type":"assistant","uuid":"l1","message":{"content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls","description":"List"}},{"type":"tool_use","id":"g1","name":"Task","input":{"description":"Sub job","prompt":"p"}}]}}"#)
    late.feed(line: #"{"type":"assistant","uuid":"l2","message":{"content":[{"type":"text","text":"Later text"}]}}"#)
    check(chipCalls(late.items()).allSatisfy { $0.output == nil } && chipCalls(late.items()).count == 2, "calls: no output before result")
    late.feed(line: #"{"type":"user","uuid":"l3","message":{"content":[{"type":"tool_result","tool_use_id":"b1","is_error":true,"content":[{"type":"text","text":"boom"}]},{"type":"tool_result","tool_use_id":"g1","content":"\#(String(repeating: "x", count: 9000))"}]}}"#)
    let lc = chipCalls(late.items())
    check(lc.first?.summary == "ls" && lc.first?.output == "boom" && lc.first?.isError == true, "calls: late result updates earlier chip")
    check(lc.last?.summary == "Sub job" && (lc.last?.output?.utf8.count ?? 0) <= 4200 && lc.last?.output?.hasSuffix("…") == true, "calls: Task summary, output truncated")
    let cxCalls = chipCalls(SessionLog.parse(lines: cx))
    check(cxCalls.count == 2 && cxCalls[0].output == "ok" && cxCalls[1].summary == "bash -lc swift test" && cxCalls[1].output == "done", "calls: codex mapped")

    check(SessionLog.parse(lines: lines) == SessionLog.parse(lines: lines), "items() equal across calls (stable chip ids)")
}

// Claude Code sometimes logs the words it shows before a tool call as a thinking block (one line per block, one message
// id). The terminal shows them, so they show as text; thinking not followed by a tool call of its message stays hidden.
@MainActor func checksThinkingBeforeTool() async throws {
    var p = ConversationParser()
    p.feed(line: #"{"type":"assistant","uuid":"k0","message":{"id":"m1","content":[{"type":"thinking","thinking":"","signature":"s"}]}}"#)
    p.feed(line: #"{"type":"assistant","uuid":"k1","message":{"id":"m1","content":[{"type":"thinking","thinking":"Shown before the tool","signature":"s"}]}}"#)
    p.feed(line: #"{"type":"assistant","uuid":"k2","message":{"id":"m1","content":[{"type":"tool_use","id":"tm","name":"mcp__memes__meme","input":{"meme":"x"}}]}}"#)
    p.feed(line: #"{"type":"assistant","uuid":"k3","message":{"id":"m2","content":[{"type":"thinking","thinking":"Private reasoning","signature":"s"}]}}"#)
    p.feed(line: #"{"type":"assistant","uuid":"k4","message":{"id":"m2","content":[{"type":"text","text":"Answer"}]}}"#)
    p.feed(line: #"{"type":"assistant","uuid":"k5","message":{"id":"m3","content":[{"type":"thinking","thinking":"Other message's reasoning","signature":"s"}]}}"#)
    p.feed(line: #"{"type":"assistant","uuid":"k6","message":{"id":"m4","content":[{"type":"tool_use","id":"tb","name":"Bash","input":{"command":"ls"}}]}}"#)
    p.feed(line: #"{"type":"assistant","uuid":"k7","message":{"id":"m5","content":[{"type":"thinking","thinking":"Same line","signature":"s"},{"type":"tool_use","id":"tc","name":"Bash","input":{"command":"pwd"}}]}}"#)
    let texts = p.items().compactMap { if case .assistantText(let t) = $0.kind { t } else { nil } }
    check(texts == ["Shown before the tool", "Answer", "Same line"], "thinking: shown only right before its message's tool call \(texts)")
    let i = p.items()
    let shown = i.firstIndex { if case .assistantText("Shown before the tool") = $0.kind { true } else { false } }
    check(shown != nil && shown! + 1 < i.count && { if case .toolChips = i[shown! + 1].kind { true } else { false } }(), "thinking: the text comes before the tool chips")
}

// loadEarlier: 300 lines, tail of ~1 KB first, then step back; ids stable, cross-segment result matched
@MainActor func checksLoadEarlier() async throws {
    do {
        let root = NSTemporaryDirectory() + "hkle-\(UUID().uuidString)"
        try FileManager.default.createDirectory(atPath: root + "/p", withIntermediateDirectories: true)
        var ls: [String] = []
        for i in 0..<100 {
            ls.append(#"{"type":"user","uuid":"u\#(i)","message":{"content":"question \#(i) ......................................"}}"#)
            ls.append(#"{"type":"assistant","uuid":"a\#(i)","message":{"content":[{"type":"tool_use","id":"t\#(i)","name":"Bash","input":{"command":"echo \#(i)"}}]}}"#)
            ls.append(#"{"type":"user","uuid":"r\#(i)","message":{"content":[{"type":"tool_result","tool_use_id":"t\#(i)","content":"out\#(i)"}]}}"#)
        }
        let path = root + "/p/le1.jsonl"
        try (ls.joined(separator: "\n") + "\n").write(toFile: path, atomically: true, encoding: .utf8)
        let log = SessionLog(sessionId: "le1")
        var it = log.snapshots(projectsDir: root, pollInterval: .milliseconds(20), initialTailBytes: 3000).makeAsyncIterator()
        _ = await it.next()
        let first = await it.next()!.items
        let ids0 = first.map(\.id)
        check(first.count > 2 && first.count < 150, "loadEarlier: initial tail is partial (\(first.count) items)")
        var more = await log.loadEarlier(bytes: 3000) == .more
        var cur = await it.next()!.items
        check(more && cur.count > first.count, "loadEarlier: older items arrive, more remain")
        check(Array(cur.suffix(ids0.count)).map(\.id) == ids0 && Set(cur.map(\.id)).count == cur.count, "loadEarlier: ids stable and unique")
        var guardN = 0
        while more && guardN < 50 { more = await log.loadEarlier(bytes: 3000) == .more; guardN += 1 }
        var drain = 0
        repeat { cur = await it.next()!.items; drain += 1 } while cur.count < 200 && drain < 60
        check(!more && cur.count == 200 && cur.first.map({ if case .userText(let t) = $0.kind { return t.hasPrefix("question 0 ") } else { return false } }) == true, "loadEarlier: reaches file start \(cur.count)")
        check(chipCalls(cur).count == 100 && chipCalls(cur).allSatisfy { $0.output != nil }, "loadEarlier: results matched across segments")
        let again = await log.loadEarlier()
        check(again == .end, "loadEarlier: false when nothing older")
        try? FileManager.default.removeItem(atPath: root)
    }
}

// Timestamps
@MainActor func checksTimestamps() async throws {
    do {
        func at(_ s: String) -> Date { ISO8601DateFormatter.withFraction.date(from: s)! }
        let items = SessionLog.parse(lines: [
            #"{"type":"user","uuid":"t1","timestamp":"2026-10-09T01:02:03.456Z","message":{"content":"hello"}}"#,
            #"{"type":"assistant","uuid":"t2","timestamp":"2026-10-09T01:02:10.000Z","message":{"content":[{"type":"tool_use","id":"x1","name":"Read","input":{"file_path":"/a"}}]}}"#,
            #"{"type":"assistant","uuid":"t3","timestamp":"2026-10-09T01:02:12Z","message":{"content":[{"type":"tool_use","id":"x2","name":"Read","input":{"file_path":"/b"}}]}}"#,
            #"{"type":"user","uuid":"t4","timestamp":"2026-10-09T01:02:13.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"x1","content":"ok"}]}}"#,
            #"{"type":"assistant","uuid":"t5","timestamp":"2026-10-09T01:20:00.000Z","message":{"content":[{"type":"text","text":"done"}]}}"#,
            #"{"type":"user","uuid":"t6","message":{"content":"no stamp"}}"#,
        ])
        check(items.count == 4, "timestamp: fixture parses to 4 items (\(items.count))")
        check(abs((items.first?.timestamp ?? .distantPast).timeIntervalSince(at("2026-10-09T01:02:03.456Z"))) < 0.002, "timestamp: user text carries the line time (fraction)")
        check(items.count > 1 && items[1].timestamp == at("2026-10-09T01:02:10.000Z"), "timestamp: chips item uses the first call's time")
        check(items.count > 2 && items[2].timestamp == at("2026-10-09T01:20:00.000Z"), "timestamp: assistant text carries the line time")
        check(items.last?.timestamp == nil, "timestamp: nil when the line has none")
        let cx = SessionLog.parse(lines: [
            #"{"timestamp":"2026-10-09T03:00:00.250Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi codex"}]}}"#,
            #"{"timestamp":"2026-10-09T03:00:05.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}}"#,
        ])
        check(cx.count == 2 && cx[0].timestamp == at("2026-10-09T03:00:00.250Z") && cx[1].timestamp == at("2026-10-09T03:00:05.000Z"), "timestamp: codex lines")
        let pend = SessionLog.parse(lines: [
            #"{"type":"assistant","uuid":"p1","timestamp":"2026-10-09T04:00:00.000Z","message":{"content":[{"type":"tool_use","id":"pp","name":"Bash","input":{"command":"ls"}}]}}"#,
        ])
        check(pend.first.map { if case .permissionRequest = $0.kind { return $0.timestamp == at("2026-10-09T04:00:00.000Z") } else { return false } } == true, "timestamp: pending permission request")
    }
}

// Pane label from the snapshot
@MainActor func checksPaneLabel() async throws {
    do {
        let snap = Data(#"{"id":"x","result":{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"ws","number":1}],"tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1}],"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","terminal_id":"term_1","cwd":"/","label":"build"},{"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","terminal_id":"term_2","cwd":"/"}],"agents":[]}}}"#.utf8)
        let ws = try SnapshotParser.parse(snap)
        let panes = ws[0].tabs[0].panes
        check(panes[0].label == "build" && panes[1].label == nil, "snapshot: pane label")
    }
}

// Prompt detection on screen text
@MainActor func checksPromptDetection() async throws {
    check(PermissionGate.screenShowsPrompt(" Do you want to proceed?\n ❯ 1. Yes") && !PermissionGate.screenShowsPrompt("Running tests…"), "screen marker")
    var parser = ConversationParser()
    for l in lines { parser.feed(line: l) }
    check(parser.hasOpenToolUse, "parser: open tool use at end of fixture")
    var closed = ConversationParser()
    for l in lines.prefix(5) { closed.feed(line: l) }
    check(!closed.hasOpenToolUse, "parser: no open tool use after results")
}

// Locate + tail
@MainActor func checksLocateTail() async throws {
    let dir = NSTemporaryDirectory() + "hk-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir + "/proj", withIntermediateDirectories: true)
    let path = dir + "/proj/abc.jsonl"
    try (lines[0] + "\n").write(toFile: path, atomically: true, encoding: .utf8)
    check(SessionLog.locate(sessionId: "abc", projectsDir: dir)?.path == path, "locate")
    check(SessionLog.locate(sessionId: "../abc", projectsDir: dir) == nil, "locate rejects ..")
    await tail(path: path, line: lines[4])
    try? FileManager.default.removeItem(atPath: dir)
}

// Load state: loading -> missing (no file) -> ready (empty file) -> ready with items
@MainActor func checksLoadState() async throws {
    do {
        let root = NSTemporaryDirectory() + "hkload-\(UUID().uuidString)"
        try FileManager.default.createDirectory(atPath: root + "/proj", withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: root) }
        var it = SessionLog(sessionId: "ls1").snapshots(projectsDir: root, pollInterval: .milliseconds(30)).makeAsyncIterator()
        let s0 = await it.next(), s1 = await it.next()
        check(s0?.load == .loading && s0?.items.isEmpty == true, "load: first yield is .loading")
        check(s1?.load == .missing, "load: no file -> .missing (\(String(describing: s1?.load)))")
        FileManager.default.createFile(atPath: root + "/proj/ls1.jsonl", contents: Data())
        let s2 = await it.next()
        check(s2?.load == .ready && s2?.items.isEmpty == true, "load: empty file -> .ready, no items")
        let fh = try FileHandle(forWritingTo: URL(fileURLWithPath: root + "/proj/ls1.jsonl"))
        try fh.write(contentsOf: Data((#"{"type":"user","uuid":"u1","message":{"role":"user","content":"hi"}}"# + "\n").utf8)); try fh.close()
        let s3 = await it.next()
        check(s3?.load == .ready && s3?.items.count == 1, "load: written file -> .ready with items")
        let st = HerdrStore(session: HerdrSession(name: "x", socketPath: "/nonexistent"))
        check(st.conversationState(for: nil) == .missing, "store: nil session id is .missing")
        check(st.conversationState(for: "no-such-session-\(UUID().uuidString)") == .loading, "store: before the first read it is .loading")
        st.stop()
    }
}

// cut: 4 KB on a character boundary, also for multi-byte text
@MainActor func checksCut() async throws {
    do {
        let c1 = ConversationParser.cut(String(repeating: "é", count: 3000)), c2 = ConversationParser.cut(String(repeating: "a😀", count: 2000))
        check(c1.hasSuffix("…") && c1.dropLast().utf8.count == 4096 && c1.dropLast().allSatisfy { $0 == "é" }, "cut: 2-byte chars end on a boundary")
        check(c2.dropLast().utf8.count <= 4096 && c2.dropLast().utf8.count > 4090 && !c2.contains("\u{FFFD}"), "cut: 4-byte chars end on a boundary")
        check(ConversationParser.cut("short") == "short", "cut: short text unchanged")
    }
}
