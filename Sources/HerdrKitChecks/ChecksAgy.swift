import Foundation
@testable import HerdrKit

// agy (Antigravity CLI): synthetic fixtures shaped like agy 1.4.12 transcripts.

// Transcript (`brain/<id>/.system_generated/logs/transcript_full.jsonl`): one step per line, a tool's result is the
// next GENERIC step (no call ids), thinking and system steps hidden, a call without a result is pending.
@MainActor func checksAgyParser() async throws {
    let al = String(decoding: fixture("agy.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    let items = SessionLog.parse(lines: al)
    check(describe(items) == ["user:List the files.", "asst:Looking.", "chips:Ran a command|Read a file|Edited a file", "asst:Done with a.txt.",
                              "status", "user:Build it.", "asst:Building.", "perm:make"], "agy: items \(describe(items))")
    let calls = chipCalls(items)
    check(calls.map(\.toolName) == ["Bash", "Read", "Edit"] && calls.map(\.summary) == ["ls", "/tmp/demo/a.txt", "/tmp/demo/a.txt"],
          "agy: tool names and summaries \(calls.map(\.toolName)) \(calls.map(\.summary))")
    check(calls.map(\.isError) == [false, false, true] && calls[0].output?.hasSuffix("a.txt") == true
          && calls[1].output?.hasSuffix("hello") == true && calls[2].output == "no match"
          && !(calls[0].output ?? "").contains("Created At"), "agy: results by order, header lines cut \(calls.map { $0.output ?? "nil" })")
    check(!items.contains { if case .assistantText(let t) = $0.kind { return t.contains("Hidden") || t.contains("SYSTEM_MESSAGE") }; return false },
          "agy: thinking and system messages hidden")
    check(items.first?.timestamp == ISO8601DateFormatter().date(from: "2026-11-02T02:00:00Z"), "agy: created_at as time")
    if case .status(let s) = items[4].kind { check(s == "Context compacted", "agy: checkpoint is a compaction marker") }
    if case .permissionRequest(let r) = items.last?.kind { check(r.toolName == "Bash" && r.title == "Wants to run a command", "agy: pending call as a request") }
    var p = ConversationParser()
    for l in al { p.feed(line: l) }
    check(p.hasOpenToolUse && p.items().map(\.id) == items.map(\.id), "agy: incremental feed = batch, open call")
    check(SessionLog.parse(lines: al).map(\.id) == items.map(\.id) && Set(items.map(\.id)).count == items.count, "agy: stable unique ids")
    // A background task ("RUNNING") answers its call: it is no longer pending.
    let bg = #"{"step_index":12,"source":"MODEL","type":"GENERIC","status":"RUNNING","created_at":"2026-11-02T02:00:12Z","content":"Created At: x\nTool is running as a background task with task id: c/task-12"}"#
    check(!SessionLog.parse(lines: al + [bg]).contains { if case .permissionRequest = $0.kind { return true }; return false },
          "agy: background task closes the call")
    // Prompts without the wrapper are shown as typed; tagged injected text is not.
    let plain = #"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-02T02:00:00Z","content":"hi"}"#
    let tagged = #"{"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-02T02:00:00Z","content":"<USER_SETTINGS_CHANGE>x</USER_SETTINGS_CHANGE>"}"#
    check(describe(SessionLog.parse(lines: [plain, tagged])) == ["user:hi"], "agy: plain prompt kept, tagged-only dropped")
}

// Results pair by step_index (call i of PLANNER step p ↔ GENERIC p+1+i), not by line order. Line order
// from a real transcript: a result line before its call (4 before 3, 10 before 9), a call with no result (5: slot 6
// passed), two calls in one step, and a question after an unanswered call.
@MainActor func checksAgyOrder() async throws {
    let al = String(decoding: fixture("agy-order.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    let items = SessionLog.parse(lines: al)
    let calls = chipCalls(items)
    check(calls.map { $0.output ?? "nil" } == ["A", "B", "Not run", "D", "task", "X", "Y", "Not run"]
          && calls.map(\.isError) == [false, false, true, false, false, false, false, true],
          "agy: results by step_index \(calls.map { $0.output ?? "nil" })")
    check(!items.contains { if case .permissionRequest = $0.kind { return true }; return false }, "agy: no false pending call \(describe(items))")
    let qs = items.compactMap { if case .question(let q) = $0.kind { return q.state }; return nil }
    check(qs == [.answered(["Blue"])], "agy: the question takes its own result \(qs)")
    var p = ConversationParser()
    for l in al { p.feed(line: l) }
    check(!p.hasOpenToolUse && p.items().map(\.id) == items.map(\.id), "agy: incremental feed = batch, nothing open")
    // A late result still fills a call whose slot had passed.
    let late = SessionLog.parse(lines: al + [#"{"step_index":6,"source":"MODEL","type":"GENERIC","status":"DONE","created_at":"2026-11-02T03:00:19Z","content":"C"}"#])
    check(chipCalls(late)[2].output == "C" && chipCalls(late)[2].isError == false, "agy: late result fills the call")

    // An injected step (SYSTEM_MESSAGE, CHECKPOINT …) between a call and its result pushes the result one step on.
    let il = String(decoding: fixture("agy-injected.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    func outs(_ lines: [String]) -> [String] { chipCalls(SessionLog.parse(lines: lines)).map { "\($0.output ?? "nil")\($0.isError ? "!" : "")" } }
    let orders: [(String, [Int])] = [("in order", Array(0..<10)), ("injected step before the call", [0, 2, 1, 3, 4, 5, 7, 6, 8, 9]),
                                     ("result before the call", [0, 1, 2, 3, 4, 5, 7, 8, 6, 9]), ("result, call, then injected step", [0, 1, 2, 3, 4, 5, 8, 6, 7, 9])]
    for (name, order) in orders {
        check(outs(order.map { il[$0] }) == ["a", "b", "c"], "agy: injected steps: \(name) \(outs(order.map { il[$0] }))")
    }
    var ip = ConversationParser()
    for l in il { ip.feed(line: l) }
    check(!ip.hasOpenToolUse && !ip.items().contains { if case .permissionRequest = $0.kind { return true }; return false },
          "agy: injected steps: nothing left open")
}

// Log from the process: agy holds `presence/<conversation id>.lock` open; the transcript sits under `brain/<id>`.
@MainActor func checksAgyResolver() async throws {
    typealias R = SessionResolver
    let root = NSTemporaryDirectory() + "agy-\(UUID().uuidString)"
    let id = "00000000-0000-4000-8000-000000000001", id2 = "00000000-0000-4000-8000-000000000002"
    let logs = "\(root)/brain/\(id)/.system_generated/logs"
    try? FileManager.default.createDirectory(atPath: logs, withIntermediateDirectories: true)
    FileManager.default.createFile(atPath: logs + "/transcript_full.jsonl", contents: Data())
    let lock = "\(root)/presence/\(id).lock", lock2 = "\(root)/presence/\(id2).lock"
    let open = ["\(root)/log/cli-20260101_000000.log", "\(root)/settings.json", "\(root)/brain/\(id)", lock]
    check(R.agyLogPath(openFiles: open, dataDir: root) == logs + "/transcript_full.jsonl", "agy: open presence lock names the transcript")
    check(R.agyLogPath(openFiles: open + [lock2], dataDir: root) == nil, "agy: two locks: no guess")
    check(R.agyLogPath(openFiles: [lock2], dataDir: root) == nil, "agy: no transcript yet: nil")
    check(R.agyLogPath(openFiles: Array(open.dropLast()), dataDir: root) == nil, "agy: no lock (no conversation yet): nil")
    check(R.agyLogPath(openFiles: ["\(root)/presence/../x.lock", "\(root)/presence/not-a-uuid.lock"], dataDir: root) == nil, "agy: odd lock names ignored")
    try? FileManager.default.removeItem(atPath: root)
    let procs = R.parsePS("""
      10     1 Thu Jan  1 00:00:00 2026     -zsh TERM=x
      11    10 Thu Jan  1 00:00:20 2026     agy TERM=x
    """)
    check(R.hcAgentProcess(pids: [10], kind: "agy", procs: procs)?.pid == 11, "agy: hc finds the agy process under the shell")
    // /new or /resume inside the same agy process switches the lock; a recheck follows it.
    let dir = NSTemporaryDirectory() + "agy-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: dir) }
    func transcript(_ cid: String) -> String {
        let l = "\(dir)/brain/\(cid)/.system_generated/logs"
        try? FileManager.default.createDirectory(atPath: l, withIntermediateDirectories: true)
        FileManager.default.createFile(atPath: l + "/transcript_full.jsonl", contents: Data())
        return l + "/transcript_full.jsonl"
    }
    let pathA = transcript(id), pathB = transcript(id2)
    let held = Locked(["\(dir)/presence/\(id).lock"])
    let res = R(socketPath: "/nonexistent", agyDir: dir, recheck: 0, openFiles: { _ in held.get() })
    let need = R.Need(paneId: "hc:t1", kind: "agy", cwd: "/tmp", pids: [10])
    check(await res.resolve([need], claimed: [], procs: procs)["hc:t1"] == pathA, "agy: resolver follows the open lock")
    held.set(["\(dir)/presence/\(id2).lock"])
    let switched = await res.resolve([need], claimed: [], procs: procs)["hc:t1"]
    check(switched == pathB, "agy: same pid, new lock id: the recheck switches to the new transcript \(switched ?? "nil")")
}


// Kind on hc: the foreground program is `agy`; 7501's `app` (what `hc ls` prints as agent) is `antigravity`.
@MainActor func checksAgyKind() async throws {
    func kind(_ o: [String: Any]) -> String? { var d = o; d["address"] = "t1"; return HCMapper.Term(d).flatMap(HCMapper.kind) }
    check(kind(["argv": ["/bin/zsh", "-l"], "foreground": ["name": "agy", "argv": ["agy"]]]) == "agy", "agy: foreground agy")
    check(kind(["argv": ["antigravity"]]) == "agy", "agy: antigravity name maps to agy")
    check(kind(["argv": ["/bin/zsh", "-l"], "foreground": ["name": "zsh", "argv": ["zsh"]], "status": ["state": "done", "app": "antigravity"]]) == "agy",
          "agy: 7501 app names the kind when the foreground does not")
    check(kind(["argv": ["/bin/zsh", "-l"], "foreground": ["name": "zsh", "argv": ["zsh"]], "status": ["state": "done", "app": "zsh"]]) == nil,
          "agy: unknown 7501 app is no kind")
    // Only the 7501 names seen live are a fallback (agy, omp, claude-code, codex; issue #15): every other agent keeps the
    // kind the processes give.
    for fg in [["name": "zsh", "argv": ["zsh"]], ["name": "vim", "argv": ["vim", "a.txt"]]] as [[String: Any]] {
        check(kind(["argv": ["/bin/zsh", "-l"], "foreground": fg, "status": ["state": "done", "app": "codex"]]) == "codex",
              "agy: 7501 app codex with \(fg["name"] ?? "") in front: codex (its own 7501 name)")
    }
    for app in ["claude", "pi", "grok", "cursor", "cursor-agent", "gemini", "opencode", "amp", "agy"] {
        for fg in [["name": "zsh", "argv": ["zsh"]], ["name": "vim", "argv": ["vim", "a.txt"]]] as [[String: Any]] {
            check(kind(["argv": ["/bin/zsh", "-l"], "foreground": fg, "status": ["state": "done", "app": app]]) == nil,
                  "agy: 7501 app \(app) with \(fg["name"] ?? "") in front: no kind, as before")
        }
    }
    check(kind(["argv": ["/bin/zsh", "-l"], "foreground": ["name": "pi", "argv": ["pi"]], "status": ["state": "done", "app": "antigravity"]]) == "pi",
          "agy: the foreground wins over the 7501 app")
}


// Approval: agy 1.4.12's command prompt (live sample): "Run this command?" with rows "1. Yes, run command" … "4. No,
// cancel", cursor ">". `1` approves, Escape denies (the same keys as Claude Code).
let agyPermScreen = """
● Bash(ls -d demo*) (ctrl+o to expand)

Command
──────────────────────────────────────────

Requesting permission for:
   ls -d demo*

Run this command?
> 1. Yes, run command
  2. Yes, and always allow in this conversation for commands that start with 'ls'
  3. Yes, and always allow for commands that start with 'ls' (Persist to settings.json)
  4. No, cancel

  ↑/↓ Navigate · tab Amend · ctrl+g edit/expand command
esc to cancel                                                     accept-edits · Gemini 4 Argon · medium
"""
let agyQuestionScreen = """
> Use your ask_question tool: which color do I prefer, red or blue?

? Which color do you prefer, red or blue?

Question
──────────────────────────────────────────

Question 1/1: Which color do you prefer, red or blue?

> 1. Red
  2. Blue
  3. Write-in...

  ↑/↓ Navigate · enter Select · esc Skip
esc to cancel                                                     accept-edits · Gemini 4 Argon · medium
"""

@MainActor func checksAgyApproval() async throws {
    typealias N = ApprovalNotice
    check(PermissionGate.screenShowsPrompt(agyPermScreen), "agy: permission prompt on screen")
    check(!PermissionGate.screenShowsPrompt(agyQuestionScreen), "agy: question picker is not a permission prompt")
    func state(_ screen: String, kind: String = "agy", cmd: String? = "ls -d demo*", blocked: Bool = true) -> N.PaneState {
        N.PaneState(agentKind: kind, blocked: blocked, pendingToolUseId: cmd == nil ? nil : "ag1-0", screen: screen,
                    pendingTool: cmd == nil ? nil : "Bash", pendingCommand: cmd)
    }
    let ids = N.promptIDs(state(agyPermScreen), writes: true)
    check(ids.count == 2 && ids[0] == "ag1-0" && ids[1].hasPrefix("screen-"), "agy: the log's call is the box on screen \(ids)")
    check(N.promptIDs(state(agyPermScreen, cmd: "rm -rf demo"), writes: true) == [ids.last ?? "?"], "agy: another command in the log: the box only")
    check(N.promptIDs(state(agyPermScreen, cmd: nil), writes: true) == [ids.last ?? "?"], "agy: no log call: the box")
    check(N.promptIDs(state(agyPermScreen), writes: false).isEmpty && N.promptIDs(state(agyPermScreen, blocked: false), writes: true).isEmpty,
          "agy: no buttons without input or block")
    check(N.promptIDs(state(agyQuestionScreen), writes: true).isEmpty, "agy: a question picker gets no approval buttons")
    check(N.promptIDs(state(agyPermScreen, kind: "claude"), writes: true).isEmpty, "agy: the agy box is not read for Claude")
    let moved = agyPermScreen.replacingOccurrences(of: "> 1. Yes", with: "  1. Yes").replacingOccurrences(of: "  4. No", with: "> 4. No")
    check(N.promptIDs(state(moved, cmd: nil), writes: true) == [ids.last ?? "?"], "agy: the cursor moving keeps the prompt id")
    let two = agyPermScreen.replacingOccurrences(of: "  2. Yes", with: "> 2. Yes")
    check(N.promptIDs(state(two), writes: true).isEmpty, "agy: two cursors: not read")
    let clipped = agyPermScreen.components(separatedBy: "\n").dropFirst(4).joined(separator: "\n")
    check(N.promptIDs(state(clipped), writes: true).isEmpty, "agy: box top off screen: not read")
    let r = N.screenRequest(agyPermScreen, agent: "agy")
    check(r?.command == "ls -d demo*" && r?.toolName == "Bash" && r?.toolUseId == ids.last, "agy: card request from the screen \(String(describing: r))")
    let agy = Agent(name: "a", kind: "agy", status: .blocked, sessionId: "/x.jsonl", title: "a", backend: .hc, statusKind: "permission")
    var q = agy; q.statusKind = "question"
    check(PermissionGate.checkApproval(agy, pendingTool: true) && !PermissionGate.checkApproval(q, pendingTool: true), "agy: hc 7501 kind gates the approval card")
}

// ask_question: in the log as a call, answered by its GENERIC ("A1: Blue" / "A1: User Skipped"); picked by digit (live).
@MainActor func checksAgyQuestions() async throws {
    func step(_ i: Int, _ type: String, _ extra: String) -> String {
        #"{"step_index":\#(i),"source":"\#(type == "USER_INPUT" ? "USER_EXPLICIT" : "MODEL")","type":"\#(type)","status":"DONE","created_at":"2026-11-02T02:00:2\#(i)Z",\#(extra)}"#
    }
    let ask = step(5, "PLANNER_RESPONSE", #""tool_calls":[{"name":"ask_question","args":{"questions":[{"is_multi_select":false,"options":["Red","Blue"],"question":"Which color do you prefer, red or blue?"}]}}]"#)
    let head = [step(4, "USER_INPUT", #""content":"<USER_REQUEST>\nAsk me.\n</USER_REQUEST>""#), ask]
    let pending = SessionLog.parse(lines: head)
    guard case .question(let q)? = pending.last?.kind else { check(false, "agy: ask_question is a question item \(describe(pending))"); return }
    check(q.agent == "agy" && q.delivery == .picker && q.state == .pending && q.questions.count == 1
          && q.questions[0].text == "Which color do you prefer, red or blue?" && q.questions[0].options.map(\.label) == ["Red", "Blue"]
          && !q.questions[0].multiSelect, "agy: question parsed")
    check(!pending.contains { if case .permissionRequest = $0.kind { return true }; return false }, "agy: a question is never a permission request")
    func state(_ result: String) -> QuestionRequest.State? {
        let items = SessionLog.parse(lines: head + [step(6, "GENERIC", #""content":"Created At: x\nCompleted At: y\n\#(result)""#)])
        return items.compactMap { if case .question(let q) = $0.kind { return q.state }; return nil }.first
    }
    check(state("A1: Blue") == .answered(["Blue"]), "agy: answered")
    check(state("A1: User Skipped") == .declined, "agy: skipped is declined")
    // Screen: the picker from its "Question k/n:" line; the exact question before every key.
    check(QuestionAdapter.screenShows(q, screen: agyQuestionScreen) && !QuestionAdapter.screenShows(q, screen: agyPermScreen), "agy: picker on screen")
    check(QuestionAdapter.shows(q, question: 0, screen: agyQuestionScreen), "agy: the exact question on screen")
    let swapped = agyQuestionScreen.replacingOccurrences(of: "1. Red", with: "1. Blau")
    let twoCursors = agyQuestionScreen.replacingOccurrences(of: "  2. Blue", with: "> 2. Blue")
    let other = agyQuestionScreen.replacingOccurrences(of: "Question 1/1: Which color", with: "Question 1/1: Which colour")
    let noFoot = agyQuestionScreen.replacingOccurrences(of: " · esc Skip", with: "")
    let noWriteIn = agyQuestionScreen.replacingOccurrences(of: "  3. Write-in...\n", with: "")
    check(![swapped, twoCursors, other, noFoot, noWriteIn, agyPermScreen].contains { QuestionAdapter.shows(q, question: 0, screen: $0) },
          "agy: another option, two cursors, another text, no picker footer, no write-in row: not this question")
    check(Hub.screenTakesKeys(agyQuestionScreen) && Hub.screenTakesKeys(agyPermScreen), "agy: prompts take keys (no typed text)")
    // Keys
    check((try? QuestionAdapter.steps(for: q, answers: [QuestionAnswer(picks: [1])])) == [.text("2")], "agy: digit picks and submits")
    check((try? QuestionAdapter.steps(for: q, answers: [QuestionAnswer(picks: [], other: "Green")])) == nil, "agy: write-in not proven: refused")
    var multi = q; multi.questions[0].multiSelect = true
    check((try? QuestionAdapter.steps(for: multi, answers: [QuestionAnswer(picks: [0, 1])])) == nil, "agy: multi-select not proven: refused")
    var two = q; two.questions.append(q.questions[0]); two.questions[1].text = "And tea or coffee?"
    check((try? QuestionAdapter.steps(for: two, answers: [QuestionAnswer(picks: [0]), QuestionAnswer(picks: [1])])) == nil,
          "agy: several questions not proven: refused")
    // hc 7501: blocked kind question is not an approval; the question comes from the log, not a screen read.
    let agy = Agent(name: "a", kind: "agy", status: .blocked, sessionId: "/x.jsonl", title: "a", backend: .hc, statusKind: "question")
    check(!PermissionGate.checkApproval(agy, pendingTool: true) && !PermissionGate.readQuestion(agy), "agy: hc question kind: no approval, no screen read")
    // A denied call never gets a result: the next prompt closes it.
    let deny = [step(8, "USER_INPUT", #""content":"<USER_REQUEST>\nRun pwd\n</USER_REQUEST>""#),
                step(9, "PLANNER_RESPONSE", #""tool_calls":[{"name":"run_command","args":{"CommandLine":"pwd"}}]"#)]
    check(describe(SessionLog.parse(lines: deny)).last == "perm:pwd", "agy: call pending before the next prompt")
    let after = SessionLog.parse(lines: deny + [step(10, "USER_INPUT", #""content":"<USER_REQUEST>\nNext.\n</USER_REQUEST>""#)])
    check(!after.contains { if case .permissionRequest = $0.kind { return true }; return false } && chipCalls(after).first?.isError == true,
          "agy: next prompt closes the denied call \(describe(after))")
}
