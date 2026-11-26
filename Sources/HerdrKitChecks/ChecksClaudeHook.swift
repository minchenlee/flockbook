import Foundation
@testable import HerdrKit

// Option B: a Claude question answered from the phone. The AskUserQuestion hook saves the call (session, tool use
// id, questions) before the picker opens; the hub binds the picker to the session's newest record and sends a key
// only when the screen is that record's exact rendering (Claude Code 2.1.296, live captures in c9-input-test).

/// Claude Code 2.1.296's picker, as captured live: enough to drive `answerQuestion` end to end.
final class ClaudePickerSim: @unchecked Sendable {
    struct Q { var header: String; var text: String; var options: [(String, String)]; var multi: Bool }
    let qs: [Q]
    var at = 0, focus = 0, review = false, done = false, typing = false
    var checks: [Set<Int>], typed: [String?]
    /// Text above the picker (the user's prompt).
    var above = "❯ Use AskUserQuestion\n────────────────────────────────────────"
    init(_ qs: [Q]) { self.qs = qs; checks = qs.map { _ in [] }; typed = qs.map { _ in nil } }

    var bar: String {
        if qs.count == 1 && !qs[0].multi { return " \(checks[0].isEmpty && typed[0] == nil ? "☐" : "☒") \(qs[0].header)" }
        return "←  " + qs.indices.map { "\(answered($0) ? "☒" : "☐") \(qs[$0].header)" }.joined(separator: "  ") + "  ✔ Submit  →"
    }
    var single: [Int: Int] = [:]
    func answered(_ k: Int) -> Bool { single[k] != nil || !checks[k].isEmpty || (qs[k].multi ? false : typed[k] != nil && k < at) }

    func render() -> String {
        if done { return above + "\n⏺ User answered Claude's questions:\n❯ " }
        if review {
            var l = [above, bar, "", "Review your answers", ""]
            for (k, q) in qs.enumerated() {
                let a = q.multi ? (checks[k].filter { $0 < q.options.count }.sorted().map { q.options[$0].0 } + (checks[k].contains(q.options.count) ? [typed[k] ?? ""] : []))
                                : single[k].map { [q.options[$0].0] } ?? [typed[k] ?? ""]
                l += [" ● \(q.text)", "   → \(a.joined(separator: ", "))"]
            }
            l += ["", "Ready to submit your answers?", "", "❯ 1. Submit answers", "  2. Cancel"]
            return l.joined(separator: "\n")
        }
        let q = qs[at], n = q.options.count
        var l = [above, bar, "", q.text, ""]
        for (i, o) in q.options.enumerated() {
            let box = q.multi ? (checks[at].contains(i) ? "[✔] " : "[ ] ") : ""
            l.append("\(focus == i ? "❯" : " ") \(i + 1). \(box)\(o.0)")
            if !o.1.isEmpty { l.append((q.multi ? "         " : "     ") + o.1) }
        }
        let box = q.multi ? (checks[at].contains(n) ? "[✔] " : "[ ] ") : ""
        l.append("\(focus == n ? "❯" : " ") \(n + 1). \(box)\(typed[at] ?? (q.multi ? "Type something" : "Type something."))")
        if q.multi { l.append("\(focus == n + 1 ? "❯" : " ")    Submit") }
        l += [String(repeating: "─", count: 60), "  \(n + 2). Chat about this", ""]
        let nav = qs.count > 1 ? "Tab/Arrow keys to navigate" : "↑/↓ to navigate"
        l.append("Enter to select · \(nav)\(focus >= n && (typing || typed[at] != nil) ? " · ctrl+g to edit in Vim" : "") · Esc to cancel")
        return l.joined(separator: "\n")
    }

    private func next() {
        focus = 0; typing = false
        if at + 1 < qs.count { at += 1 } else if qs.count > 1 || qs[at].multi { review = true } else { done = true }
    }

    func input(_ method: String, _ params: [String: Any]) {
        if method == "pane.send_keys", let keys = params["keys"] as? [String] {
            for k in keys { key(k) }
        } else if method == "pane.send_text", let t = params["text"] as? String {
            text(t)
        }
    }

    func text(_ t: String) {
        if review { if t == "1" { review = false; done = true }; return }
        let q = qs[at], n = q.options.count
        if typing && focus == n { typed[at] = (typed[at] ?? "") + t; return }
        guard let d = Int(t), (1...(n + 1)).contains(d) else { return }
        if q.multi { if checks[at].contains(d - 1) { checks[at].remove(d - 1) } else { checks[at].insert(d - 1) }; if d == n + 1 { typing = true } ; return }
        if d == n + 1 { focus = n; typing = true; return }
        single[at] = d - 1; next()
    }

    func key(_ k: String) {
        guard !review, !done else { return }
        let q = qs[at], n = q.options.count
        switch k {
        case "Down": focus = min(focus + 1, q.multi ? n + 1 : n)
        case "Tab": if q.multi && focus == n && typed[at] != nil { focus = n + 1 } else { next() }
        case "Enter":
            if q.multi { if focus == n + 1 { next() } } else if focus == n, typed[at] != nil { next() } else if focus < n { single[at] = focus; next() }
        case "Left": if at > 0 { at -= 1; focus = 0 }
        default: break
        }
    }
}

@MainActor func checksClaudeHook() async throws {
    // MARK: the hook's record
    let payload = #"""
    {"session_id":"sess-1","transcript_path":"/home/u/.claude/projects/p/sess-1.jsonl","cwd":"/home/u/p","permission_mode":"default",
     "hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":"toolu_01ABC",
     "tool_input":{"questions":[{"question":"Pick a color?","header":"Color","multiSelect":false,"options":[
       {"label":"Red (Recommended)","description":"warm and a long description that should wrap on a narrow terminal screen for sure because it is long"},
       {"label":"Blue","description":"cool"}]}]}}
    """#
    let t0 = Date(timeIntervalSince1970: 1_800_000_000)
    let rec = ClaudeQuestionHook.record(Data(payload.utf8), time: t0)
    check(rec?.sessionId == "sess-1" && rec?.request.toolUseId == "toolu_01ABC" && rec?.request.agent == "claude"
          && rec?.request.questions.first?.options.map(\.label) == ["Red", "Blue"] && rec?.request.questions.first?.options.first?.recommended == true
          && rec?.request.unread == 0, "hookB record: session, id and questions from the hook JSON")
    for (bad, why) in [(payload.replacingOccurrences(of: "PreToolUse", with: "PostToolUse"), "another event"),
                       (payload.replacingOccurrences(of: #""tool_name":"AskUserQuestion""#, with: #""tool_name":"Bash""#), "another tool"),
                       (payload.replacingOccurrences(of: #""session_id":"sess-1""#, with: #""session_id":"""#), "no session"),
                       (payload.replacingOccurrences(of: "toolu_01ABC", with: "toolu_01/../x"), "an id with other characters"),
                       (payload.replacingOccurrences(of: #""questions":["#, with: #""qs":["#), "no questions"),
                       ("not json", "not JSON")] {
        check(ClaudeQuestionHook.record(Data(bad.utf8), time: t0) == nil, "hookB record: \(why) -> no record")
    }
    let r1 = rec!
    var r2 = r1; r2.request.toolUseId = "toolu_02"; r2.time = t0.addingTimeInterval(30)
    var other = r1; other.sessionId = "sess-2"; other.time = t0.addingTimeInterval(60)
    check(ClaudeQuestionHook.newest([r1, r2, other], session: "sess-1", now: t0.addingTimeInterval(90))?.toolUseId == "toolu_02",
          "hookB newest: the session's newest record, never another session's")
    check(ClaudeQuestionHook.newest([r1, r2], session: "sess-1", now: t0.addingTimeInterval(2 * 86400)) == nil, "hookB newest: older than a day -> none")
    var tie = r2; tie.request.toolUseId = "toolu_03"
    check(ClaudeQuestionHook.newest([r1, r2, tie], session: "sess-1", now: t0.addingTimeInterval(90)) == nil, "hookB newest: two newest at the same time -> none")
    check(ClaudeQuestionHook.newest([r1], session: "sess-9", now: t0) == nil, "hookB newest: no record for the session -> none")

    // MARK: the script, run as Claude Code runs it (stdin in, nothing out, exit 0)
    let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hookB-\(getpid())-\(UUID().uuidString.prefix(6))", isDirectory: true)
    defer { try? FileManager.default.removeItem(at: tmp) }
    let folder = ClaudeQuestionHook.Folder(url: tmp.appendingPathComponent("it's questions", isDirectory: true))
    let script = tmp.appendingPathComponent("hook dir/ask.sh")
    try folder.writeScript(to: script)
    let cmd = ClaudeSettingsHook.command(scriptPath: script.path)
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/bin/sh"); p.arguments = ["-c", cmd]
    let inPipe = Pipe(), outPipe = Pipe()
    p.standardInput = inPipe; p.standardOutput = outPipe; p.standardError = outPipe
    try p.run()
    inPipe.fileHandleForWriting.write(Data(payload.utf8)); try inPipe.fileHandleForWriting.close()
    p.waitUntilExit()
    let out = outPipe.fileHandleForReading.readDataToEndOfFile()
    let files = (try? FileManager.default.contentsOfDirectory(atPath: folder.url.path)) ?? []
    let mode = files.first.flatMap { try? FileManager.default.attributesOfItem(atPath: folder.url.appendingPathComponent($0).path)[.posixPermissions] as? Int }
    check(p.terminationStatus == 0 && out.isEmpty && files.count == 1 && files[0].hasSuffix(".json") && mode == 0o600,
          "hookB script: one .json record, mode 600, no output, exit 0 (\(p.terminationStatus), \(files), \(String(describing: mode)))")
    check(folder.records().map(\.request.toolUseId) == ["toolu_01ABC"], "hookB folder: the script's record reads back")
    let scriptMode = (try? FileManager.default.attributesOfItem(atPath: script.path))?[.posixPermissions] as? Int
    check(scriptMode == 0o700, "hookB script: mode 700")
    // An old record goes; a fresh one stays.
    let old = folder.url.appendingPathComponent("1.1.json")
    try Data(payload.utf8).write(to: old)
    try FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(-3 * 86400)], ofItemAtPath: old.path)
    _ = folder.records()
    check(!FileManager.default.fileExists(atPath: old.path) && folder.records().count == 1, "hookB folder: records over a day old are removed")

    // MARK: Claude Code's settings file: only the owner's click, only our entry
    let settings = tmp.appendingPathComponent("claude/settings.json")
    try FileManager.default.createDirectory(at: settings.deletingLastPathComponent(), withIntermediateDirectories: true)
    let mine = #"{"model":"opus","env":{"A":"1"},"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo hi"}]}],"Stop":[{"hooks":[{"type":"command","command":"x"}]}]},"permissions":{"allow":["Bash(ls:*)"]}}"#
    try Data(mine.utf8).write(to: settings)
    try FileManager.default.setAttributes([.posixPermissions: 0o640], ofItemAtPath: settings.path)
    let backup = try ClaudeSettingsHook.apply(add: true, file: settings, command: cmd, now: t0)
    let after = try ClaudeSettingsHook.read(settings)
    let before = try JSONSerialization.jsonObject(with: Data(mine.utf8)) as! [String: Any]
    check(ClaudeSettingsHook.installed(after, command: cmd), "hookB install: our entry is in the file")
    check(NSDictionary(dictionary: ClaudeSettingsHook.removing(after, command: cmd)).isEqual(to: before),
          "hookB install: every other key kept (removing ours gives the old object back)")
    check(backup.map { (try? Data(contentsOf: $0)) == Data(mine.utf8) } == true, "hookB install: the old file backed up next to it, byte for byte")
    check((try? FileManager.default.attributesOfItem(atPath: settings.path))?[.posixPermissions] as? Int == 0o640, "hookB install: the file keeps its mode")
    let pre = ((after["hooks"] as? [String: Any])?["PreToolUse"] as? [[String: Any]]) ?? []
    check(pre.count == 2 && pre[1]["matcher"] as? String == "AskUserQuestion"
          && ((pre[1]["hooks"] as? [[String: Any]])?.first?["timeout"] as? Int) == 5, "hookB install: a PreToolUse entry, matcher AskUserQuestion, timeout 5")
    let twice = try ClaudeSettingsHook.apply(add: true, file: settings, command: cmd, now: t0)
    let reread = try ClaudeSettingsHook.read(settings)
    check(twice == nil && NSDictionary(dictionary: reread).isEqual(to: after), "hookB install twice: no change, no new backup")
    try ClaudeSettingsHook.apply(add: false, file: settings, command: cmd, now: t0.addingTimeInterval(1))
    check(NSDictionary(dictionary: try ClaudeSettingsHook.read(settings)).isEqual(to: before), "hookB uninstall: only our entry leaves")
    // Our hook next to another in one group: the other stays.
    let shared = try ClaudeSettingsHook.adding([:], command: cmd)
    var grp = ((shared["hooks"] as! [String: Any])["PreToolUse"] as! [[String: Any]])
    grp[0]["hooks"] = (grp[0]["hooks"] as! [Any]) + [["type": "command", "command": "theirs"]]
    let mixed: [String: Any] = ["hooks": ["PreToolUse": grp]]
    let left = (((ClaudeSettingsHook.removing(mixed, command: cmd)["hooks"] as? [String: Any])?["PreToolUse"] as? [[String: Any]])?.first?["hooks"] as? [[String: Any]])
    check(left?.count == 1 && left?.first?["command"] as? String == "theirs", "hookB uninstall: another hook in our group stays")
    let added = try ClaudeSettingsHook.adding([:], command: cmd)
    check(ClaudeSettingsHook.removing([:], command: cmd).isEmpty && added.count == 1, "hookB: an empty file gets only hooks")
    // Not JSON, or hooks of another shape: refused, the file untouched.
    for (text, why) in [("{ // comment\n}", "not JSON"), (#"{"hooks":[1]}"#, "hooks not an object"), (#"{"hooks":{"PreToolUse":{}}}"#, "PreToolUse not a list")] {
        try Data(text.utf8).write(to: settings)
        let threw = (try? ClaudeSettingsHook.apply(add: true, file: settings, command: cmd, now: t0)) == nil
        check(threw && (try? Data(contentsOf: settings)) == Data(text.utf8), "hookB install: \(why) -> refused, file unchanged")
    }
    let fresh = tmp.appendingPathComponent("new/settings.json")
    let freshBackup = try ClaudeSettingsHook.apply(add: true, file: fresh, command: cmd, now: t0)
    let freshRead = try ClaudeSettingsHook.read(fresh)
    check(freshBackup == nil && ClaudeSettingsHook.installed(freshRead, command: cmd), "hookB install: no file yet -> created with our entry, no backup")
    check(ClaudeSettingsHook.preview(command: cmd).contains("\"matcher\" : \"AskUserQuestion\"") && ClaudeSettingsHook.preview(command: cmd).contains("PreToolUse"),
          "hookB preview: the exact entry is shown")

    // MARK: the exact rendering (Claude Code 2.1.296, live)
    let q = r1.request
    let single = """
    ❯ Call the AskUserQuestion tool once with one question
    ────────────────────────────────────────────────────────────────
     ☐ Color

    Pick a color?

    ❯ 1. Red (Recommended)
         warm and a long description that should wrap on a narrow terminal screen for sure because it is long
      2. Blue
         cool
      3. Type something.
    ────────────────────────────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    func ex(_ s: String, other: String? = nil, focus: QuestionAdapter.FieldFocus = .auto, checks: Set<Int>? = nil, _ req: QuestionRequest? = nil, k: Int = 0) -> Bool {
        QuestionAdapter.claudeExact(req ?? q, question: k, screen: s, other: other, focus: focus, checks: checks)
    }
    check(ex(single), "hookB exact: the live single picker is the record's question")
    check(QuestionAdapter.shows(q, question: 0, screen: single), "hookB shows: a hooked request uses the exact compare")
    check(ex(single.replacingOccurrences(of: "screen for sure because it is long", with: "screen for sure\n     because it is long")), "hookB exact: a wrapped description still matches")
    let otherEmpty = single.replacingOccurrences(of: "❯ 1. Red", with: "  1. Red").replacingOccurrences(of: "  3. Type something.", with: "❯ 3. Type something.")
        .replacingOccurrences(of: "↑/↓ to navigate · Esc", with: "↑/↓ to navigate · ctrl+g to edit in Vim · Esc")
    let otherTyped = otherEmpty.replacingOccurrences(of: "❯ 3. Type something.", with: "❯ 3. Green typed")
    check(ex(otherEmpty, other: "") && ex(otherTyped, other: "Green typed"), "hookB exact: the Other row, empty and with the hub's text (live)")
    check(!ex(otherEmpty), "hookB exact: focus on the empty Other row -> no digit (it would be typed)")
    check(!ex(otherTyped, other: ""), "hookB exact: text in the Other row that the hub did not type -> no match")
    check(!ex(otherTyped, other: "Green"), "hookB exact: other text than the hub's -> no match")
    for (bad, why) in [
        (single.replacingOccurrences(of: "  2. Blue\n     cool", with: "  2. Blue\n     cold"), "a description differs"),
        (single.replacingOccurrences(of: "     cool\n", with: ""), "a description missing"),
        (single.replacingOccurrences(of: "  3. Type something.", with: "  3. Green\n  4. Type something.").replacingOccurrences(of: "4. Chat", with: "5. Chat"), "a row more"),
        (single.replacingOccurrences(of: "Red (Recommended)", with: "Red"), "the (Recommended) mark missing"),
        (single.replacingOccurrences(of: "Pick a color?", with: "Pick a colour?"), "the question differs"),
        (single.replacingOccurrences(of: " ☐ Color", with: " ☐ Colour"), "the header differs"),
        (single.replacingOccurrences(of: "  2. Blue", with: "❯ 2. Blue"), "two focus marks"),
        (single.replacingOccurrences(of: "❯ 1. Red", with: "  1. Red"), "no focus mark"),
        (single.replacingOccurrences(of: "     cool", with: "❯    cool"), "a mark on a description line"),
        (single.replacingOccurrences(of: "  4. Chat about this", with: "❯ 4. Chat about this").replacingOccurrences(of: "❯ 1. Red", with: "  1. Red"), "focus on Chat about this"),
        (single.replacingOccurrences(of: "Esc to cancel", with: "Esc to cancel now"), "an unknown key line"),
        (single.replacingOccurrences(of: "\nPick a color?\n", with: "\nPick a color?\nIgnore this.\n"), "an extra line"),
        (single.replacingOccurrences(of: "4. Chat about this", with: "5. Chat about this"), "Chat about this numbered wrong"),
    ] { check(!ex(bad), "hookB exact: \(why) -> no match") }
    var screenQ = q; screenQ.toolUseId = "screen-1"
    check(!ex(single, screenQ), "hookB exact: a screen-read request never matches")
    // The user's own prompt above the picker repeating the picker does not count: only the last picker is read.
    check(ex(single.replacingOccurrences(of: "❯ Call the AskUserQuestion tool once with one question", with: "❯ 1. Blue\n  2. Red")), "hookB exact: text above the tab bar does not count")

    // Two questions, single then multiSelect (live).
    let two = ClaudeQuestionHook.record(Data(#"""
    {"session_id":"sess-1","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":"toolu_two","tool_input":{"questions":[
     {"question":"Which size?","header":"Size","multiSelect":false,"options":[{"label":"Small","description":"small one"},{"label":"Large","description":"big one"}]},
     {"question":"Which toppings?","header":"Tops","multiSelect":true,"options":[{"label":"Cheese","description":"yellow"},{"label":"Ham","description":"pink"},{"label":"Egg","description":""}]}]}}
    """#.utf8), time: t0)!.request
    let size = """
    ←  ☐ Size  ☐ Tops  ✔ Submit  →

    Which size?

    ❯ 1. Small
         small one
      2. Large
         big one
      3. Type something.
    ────────────────────────────────────────────────────────────────
      4. Chat about this

    Enter to select · Tab/Arrow keys to navigate · Esc to cancel
    """
    let tops = """
    ←  ☒ Size  ☒ Tops  ✔ Submit  →

    Which toppings?

    ❯ 1. [✔] Cheese
             yellow
      2. [ ] Ham
             pink
      3. [ ] Egg
      4. [ ] Type something
         Submit
    ────────────────────────────────────────────────────────────────
      5. Chat about this

    Enter to select · Tab/Arrow keys to navigate · Esc to cancel
    """
    check(ex(size, two) && !ex(size, two, k: 1), "hookB exact: question 1 of 2 matches as question 1 only")
    check(ex(tops, two, k: 1) && ex(tops, checks: [0], two, k: 1), "hookB exact: multiSelect with one box checked (live)")
    check(!ex(tops, checks: [], two, k: 1) && !ex(tops, checks: [0, 1], two, k: 1), "hookB exact: other [✔] boxes than expected -> no match")
    let topsOther = tops.replacingOccurrences(of: "❯ 1. [✔] Cheese", with: "  1. [✔] Cheese").replacingOccurrences(of: "  4. [ ] Type something", with: "❯ 4. [✔] olive")
        .replacingOccurrences(of: "navigate · Esc", with: "navigate · ctrl+g to edit in Vim · Esc")
    let topsSubmit = topsOther.replacingOccurrences(of: "❯ 4. [✔] olive", with: "  4. [✔] olive").replacingOccurrences(of: "     Submit", with: "❯    Submit")
    check(ex(topsOther, other: "olive", focus: .other, checks: [0, 3], two, k: 1), "hookB exact: multiSelect Other with the hub's text, focus on it (live)")
    check(ex(topsSubmit, other: "olive", focus: .submit, checks: [0, 3], two, k: 1) && !ex(topsSubmit, other: "olive", focus: .other, checks: [0, 3], two, k: 1),
          "hookB exact: focus on Submit (live) only where Submit is expected")
    check(!ex(tops.replacingOccurrences(of: "  3. [ ] Egg", with: "  3. [x] Egg"), two, k: 1), "hookB exact: an unknown box -> no match")
    let reviewScreen = """
    ←  ☒ Size  ☒ Tops  ✔ Submit  →

    Review your answers

     ● Which size?
       → Large
     ● Which toppings?
       → Cheese, olive

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    """
    let answers = [QuestionAnswer(picks: [1]), QuestionAnswer(picks: [0], other: "olive")]
    check(QuestionAdapter.claudeReviewExact(two, answers: answers, screen: reviewScreen), "hookB review: the live review of the hub's answers")
    check(!QuestionAdapter.claudeReviewExact(two, answers: [QuestionAnswer(picks: [0]), answers[1]], screen: reviewScreen), "hookB review: other answers than the hub's -> no match")
    check(!QuestionAdapter.claudeReviewExact(two, answers: answers, screen: reviewScreen.replacingOccurrences(of: "  2. Cancel", with: "❯ 2. Cancel")), "hookB review: two marks -> no match")
    check(!QuestionAdapter.claudeReviewExact(two, answers: answers, screen: reviewScreen.replacingOccurrences(of: "1. Submit answers", with: "1. Cancel")), "hookB review: rows swapped -> no match")
    check(!QuestionAdapter.showsReview(two, screen: reviewScreen), "hookB review: a hooked request without the answers never matches")

    // MARK: the hub binds the picker to the record
    let agent = Agent(name: "c", kind: "claude", status: .blocked, sessionId: "sess-1", title: "")
    check(Hub.openQuestion(agent: agent, screen: single, items: [], hooked: q)?.toolUseId == "toolu_01ABC", "hookB hub: the exact picker -> the record's question")
    check(Hub.openQuestion(agent: agent, screen: otherTyped, items: [], hooked: q).map(Hub.screenRead) == true, "hookB hub: the user typed in the picker -> screen read (Mac)")
    check(Hub.openQuestion(agent: agent, screen: size, items: [], hooked: q).map(Hub.screenRead) == true, "hookB hub: another picker than the record -> screen read (Mac)")
    check(Hub.openQuestion(agent: agent, screen: single, items: [], hooked: nil).map(Hub.screenRead) == true, "hookB hub: no record -> screen read (Mac)")
    var answeredQ = q; answeredQ.state = .answered(["Blue"])
    let loggedItem = ConversationItem(id: "toolu_01ABC", kind: .question(answeredQ))
    check(Hub.openQuestion(agent: agent, screen: single, items: [loggedItem], hooked: q).map(Hub.screenRead) == true, "hookB hub: the record's call is in the log (answered) -> stale, screen read")
    var declinedQ = q; declinedQ.state = .declined
    check(Hub.openQuestion(agent: agent, screen: single, items: [ConversationItem(id: "x", kind: .question(declinedQ))], hooked: q).map(Hub.screenRead) == true, "hookB hub: declined in the log -> stale")
    // Live: with the hook, Claude logs the call while the picker is still open (no result yet): still the open question.
    check(Hub.openQuestion(agent: agent, screen: single, items: [ConversationItem(id: "toolu_01ABC", kind: .question(q))], hooked: q)?.toolUseId == "toolu_01ABC",
          "hookB hub: the call pending in the log (live) -> still the record's question")
    check(Hub.openQuestion(agent: agent, screen: tops, items: [], hooked: two)?.toolUseId == "toolu_two", "hookB hub: question 2 of the record on screen -> the record")
    check(HubQuestion(q, paneAgent: agent).onMac == nil && HubQuestion(screenQ, paneAgent: agent).onMac == true, "hookB hub: the record's question may be answered from the phone")

    // MARK: answering end to end against the live layout
    let fake = FakeHerdr(path: NSTemporaryDirectory() + "hkB-\(getpid()).sock")
    try fake.start()
    defer { fake.kill() }
    let client = HerdrClient(session: HerdrSession(name: "fake", socketPath: fake.path))
    fake.agents = #"[{"pane_id":"w1:p1","agent":"claude","name":"claude","agent_status":"blocked","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"sess-1"}}]"#
    let gate = KeyGate(agent: agent, allowed: { true })
    let keyMethods: Set<String> = ["pane.send_keys", "pane.send_text", "pane.send_input"]
    func sent(after n: Int) -> [String] {
        fake.requests.dropFirst(n).filter { keyMethods.contains($0.method) }.map { ($0.params["keys"] as? [String])?.joined(separator: ",") ?? ($0.params["text"] as? String ?? "?") }
    }
    func run(_ sim: ClaudePickerSim, _ req: QuestionRequest, _ a: [QuestionAnswer], before: ((ClaudePickerSim) -> Void)? = nil) async -> (keys: [String], error: Error?) {
        before?(sim)
        fake.screen = sim.render()
        fake.onInput = { m, p in sim.input(m, p); fake.screen = sim.render() }
        let n = fake.requests.count
        var err: Error?
        do { try await client.answerQuestion(paneId: "w1:p1", request: req, answers: a, gate: gate) } catch { err = error }
        fake.onInput = nil
        return (sent(after: n), err)
    }
    let colorSim = { ClaudePickerSim([.init(header: "Color", text: "Pick a color?", options: [("Red (Recommended)", "warm and a long description that should wrap on a narrow terminal screen for sure because it is long"), ("Blue", "cool")], multi: false)]) }
    var s = colorSim()
    var res = await run(s, q, [QuestionAnswer(picks: [1])])
    check(res.error == nil && res.keys == ["2"] && s.done && s.single[0] == 1, "hookB answer: single pick -> '2' (\(res.keys), \(String(describing: res.error)))")
    s = colorSim()
    res = await run(s, q, [QuestionAnswer(other: "Green typed")])
    check(res.error == nil && res.keys == ["3", "Green typed", "Enter"] && s.done && s.typed[0] == "Green typed", "hookB answer: single Other -> 3, text, Enter (\(res.keys))")
    s = colorSim()
    res = await run(s, q, [QuestionAnswer(other: "Green typed")]) { $0.typed[0] = "mine" }
    check(res.keys.isEmpty && res.error != nil, "hookB answer: text the user typed in the Other row -> nothing sent (\(res.keys))")
    let twoSim = { ClaudePickerSim([.init(header: "Size", text: "Which size?", options: [("Small", "small one"), ("Large", "big one")], multi: false),
                                    .init(header: "Tops", text: "Which toppings?", options: [("Cheese", "yellow"), ("Ham", "pink"), ("Egg", "")], multi: true)]) }
    s = twoSim()
    res = await run(s, two, answers)
    check(res.error == nil && res.keys == ["2", "1", "4", "Down", "Down", "Down", "olive", "Tab", "Enter", "1"] && s.done,
          "hookB answer: 2 questions, multiSelect + Other, review submit (\(res.keys), \(String(describing: res.error)))")
    s = twoSim()
    res = await run(s, two, [QuestionAnswer(picks: [1]), QuestionAnswer(picks: [0, 2])])
    check(res.error == nil && res.keys == ["2", "1", "3", "Tab", "1"] && s.done && s.checks[1] == [0, 2], "hookB answer: multiSelect picks, Tab, review (\(res.keys))")
    // The user checked a box on question 2 before: the hub's digits would toggle it off, so no key on question 2.
    s = twoSim()
    res = await run(s, two, [QuestionAnswer(picks: [1]), QuestionAnswer(picks: [0])]) { $0.checks[1] = [0] }
    check(!res.keys.contains("1") && !res.keys.contains("Tab") && res.error != nil, "hookB answer: a box checked by someone else -> stops before toggling (\(res.keys))")
    // Another request on screen (same layout, other options): nothing sent.
    s = ClaudePickerSim([.init(header: "Color", text: "Pick a color?", options: [("Red (Recommended)", "warm"), ("Blue", "cool")], multi: false)])
    res = await run(s, q, [QuestionAnswer(picks: [1])])
    check(res.keys.isEmpty && res.error != nil, "hookB answer: a picker that is not the record's exact rendering -> nothing sent (\(res.keys))")
    // The picker changes mid-way: after question 1 another request opens; no key goes to it.
    s = twoSim()
    let swap = ClaudePickerSim([.init(header: "Size", text: "Which size?", options: [("Small", "small one"), ("Large", "big one")], multi: false),
                                .init(header: "Tops", text: "Which toppings?", options: [("Cheese", "yellow"), ("Ham", "pink"), ("Bacon", "")], multi: true)])
    swap.at = 1; swap.single[0] = 1
    fake.screen = s.render()
    fake.onInput = { m, p in s.input(m, p); fake.screen = s.at == 1 ? swap.render() : s.render() }
    let n0 = fake.requests.count
    var swapErr: Error?
    do { try await client.answerQuestion(paneId: "w1:p1", request: two, answers: answers, gate: gate) } catch { swapErr = error }
    fake.onInput = nil
    check(sent(after: n0) == ["2"] && swapErr != nil, "hookB answer: another request after question 1 -> stops, nothing more sent (\(sent(after: n0)))")

    // MARK: the hub session: a hooked question is answered from the phone; without a record the Mac answers
    func msg(_ json: String) -> Data { Data(json.utf8) }
    func reply(_ ms: [HubMessage]) -> HubReply? { ms.last.flatMap { if case .reply(let r) = $0 { r } else { nil } } }
    let devices = MemoryDeviceStore()
    devices.add(token: "tok", device: HubDevice(id: "d", name: "Phone", level: .input))
    for withRecord in [true, false] {
        let fb = FakeHubBackend(workspaces: [Workspace(id: "w1", label: "w", tabs: [Tab(id: "w1:t1", label: "t", number: 1,
                                  panes: [Pane(id: "w1:p1", terminalId: "t1", cwd: "/w", agent: agent)], status: .blocked)])])
        fb.screens["w1:p1"] = single
        if withRecord { fb.hooked["sess-1"] = q }
        let hs = HubSession(backend: fb, devices: devices)
        _ = await hs.receive(msg(#"{"v":1,"type":"hello","id":"h","token":"tok","client":"test","clientVersion":"1"}"#))
        let w = await hs.receive(msg(#"{"v":1,"type":"watch","id":"c1","pane":"w1:p1"}"#))
        let hq: HubQuestion? = w.compactMap { if case .prompt(let p) = $0 { p.question } else { nil } }.first
        let a = await hs.receive(msg(#"{"v":1,"type":"answer","id":"c2","pane":"w1:p1","question":"\#(hq?.id ?? "")","answers":[{"picks":[1]}]}"#))
        if withRecord {
            check(hq?.id == "toolu_01ABC" && hq?.onMac == nil && reply(a)?.ok == true && fb.answered.count == 1, "hookB hub session: the record's question -> the phone answers it")
        } else {
            check(hq?.onMac == true && reply(a)?.ok != true && fb.answered.isEmpty, "hookB hub session: no record -> onMac, nothing sent")
        }
    }
}

// Review round 1 on 5b66a4d: red first.
@MainActor func checksClaudeHookReview1() async throws {
    func req(_ json: String) -> QuestionRequest {
        ClaudeQuestionHook.record(Data((#"{"session_id":"s","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":"toolu_r1","tool_input":{"questions":["# + json + #"]}}"#).utf8), time: Date())!.request
    }
    // P1: free text that dropping whitespace turns into rows. A forged record: option 1 "KEEP" whose description
    // swallows the real row 1 (DELETE). The phone would pick KEEP and '1' would choose DELETE.
    let forged = req(#"{"question":"Choose?","header":"H","multiSelect":false,"options":[{"label":"KEEP","description":"❯ 1. DELETE danger 2. KEEP"},{"label":"DELETE","description":"ok"}]}"#)
    let screen = """
     ☐ H

    Choose? ❯ 1. KEEP
    ❯ 1. DELETE
         danger
      2. KEEP
         2. DELETE ok
      3. Type something.
    ────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    check(!QuestionAdapter.claudeExact(forged, question: 0, screen: screen, other: nil, focus: .any, checks: nil), "r1 P1: rows hidden in free text (whitespace dropped) -> no match")
    // A row must be a line of its own, at the row column; a row-like line that is not a row refuses.
    let plain = req(#"{"question":"Choose?","header":"H","multiSelect":false,"options":[{"label":"KEEP","description":"k"},{"label":"DELETE","description":"d"}]}"#)
    let good = """
     ☐ H

    Choose?

    ❯ 1. KEEP
         k
      2. DELETE
         d
      3. Type something.
    ────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    check(QuestionAdapter.claudeExact(plain, question: 0, screen: good, other: nil, focus: .auto, checks: nil), "r1 P1: the plain rendering still matches")
    check(!QuestionAdapter.claudeExact(plain, question: 0, screen: good.replacingOccurrences(of: "  2. DELETE", with: "   2. DELETE"), other: nil, focus: .auto, checks: nil),
          "r1 P1: a row number outside the row column -> no match")
    check(!QuestionAdapter.claudeExact(plain, question: 0, screen: good.replacingOccurrences(of: "     k\n", with: "     k\n  7. x\n"), other: nil, focus: .any, checks: nil),
          "r1 P1: a row-like line that is not a row -> no match")
    // P2 review: comma labels that collide with split siblings, or two labels alike without spaces, are refused before any key.
    let comma = req(#"{"question":"Pick?","header":"H","multiSelect":true,"options":[{"label":"A, B","description":""},{"label":"A","description":""},{"label":"B","description":""}]}"#)
    check(!QuestionAdapter.reviewAmbiguous(comma, answers: [QuestionAnswer(picks: [0])]), "r1 P2: quoted comma pick alone is unique")
    check(QuestionAdapter.reviewAmbiguous(comma, answers: [QuestionAnswer(picks: [1, 2])]), "r1 P2: [A, B] with 'A, B' sibling -> blocked")
    let spaces = req(#"{"question":"Pick?","header":"H","multiSelect":true,"options":[{"label":"A B","description":""},{"label":"AB","description":""}]}"#)
    check(QuestionAdapter.reviewAmbiguous(spaces, answers: [QuestionAnswer(picks: [0])]), "r1 P2: two labels alike without spaces -> ambiguous")
    let clean = req(#"{"question":"Pick?","header":"H","multiSelect":true,"options":[{"label":"A","description":""},{"label":"B","description":""}]}"#)
    check(QuestionAdapter.reviewAmbiguous(clean, answers: [QuestionAnswer(picks: [], other: "A")])
          && QuestionAdapter.reviewAmbiguous(clean, answers: [QuestionAnswer(picks: [0], other: "B")]),
          "r1 P2: Other text equal to a label or completing a sibling pick -> ambiguous")
    check(!QuestionAdapter.reviewAmbiguous(clean, answers: [QuestionAnswer(picks: [0, 1], other: "olive")]), "r1 P2: distinct labels, no commas -> fine")
    let fake = FakeHerdr(path: NSTemporaryDirectory() + "hkB1-\(getpid()).sock")
    try fake.start()
    defer { fake.kill() }
    let client = HerdrClient(session: HerdrSession(name: "fake", socketPath: fake.path))
    fake.agents = #"[{"pane_id":"w1:p1","agent":"claude","name":"claude","agent_status":"blocked","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"s"}}]"#
    let sim = ClaudePickerSim([.init(header: "H", text: "Pick?", options: [("A, B", ""), ("A", ""), ("B", "")], multi: true)])
    fake.screen = sim.render()
    let n = fake.requests.count
    var err: Error?
    do { try await client.answerQuestion(paneId: "w1:p1", request: comma, answers: [QuestionAnswer(picks: [1, 2])],
                                         gate: KeyGate(agent: Agent(name: "c", kind: "claude", status: .blocked, sessionId: "s", title: ""), allowed: { true })) } catch { err = error }
    check(err != nil && !fake.requests.dropFirst(n).contains { ["pane.send_keys", "pane.send_text"].contains($0.method) }, "r1 P2: an ambiguous review -> refused before any key")

    // P2 ownership: our entry is the AskUserQuestion matcher with our command; the same command elsewhere is not ours.
    let cmd = "'/x/ask.sh'"
    let other: [String: Any] = ["hooks": ["PreToolUse": [["matcher": "Bash", "hooks": [["type": "command", "command": cmd]]]]]]
    check(!ClaudeSettingsHook.installed(other, command: cmd), "r1 P2: our command under another matcher is not our entry")
    let both = try ClaudeSettingsHook.adding(other, command: cmd)
    let kept = ClaudeSettingsHook.removing(both, command: cmd)
    check(NSDictionary(dictionary: kept).isEqual(to: other), "r1 P2: uninstall keeps the same command under another matcher")
    // P3 shapes: a PreToolUse element that is not a group object -> refused.
    for (bad, why) in [(#"{"hooks":{"PreToolUse":[7]}}"#, "a number"), (#"{"hooks":{"PreToolUse":[{"matcher":"x","hooks":7}]}}"#, "hooks not a list"),
                       (#"{"hooks":{"PreToolUse":[{"matcher":"x","hooks":["y"]}]}}"#, "a hook not an object")] {
        let o = try JSONSerialization.jsonObject(with: Data(bad.utf8)) as! [String: Any]
        check((try? ClaudeSettingsHook.adding(o, command: cmd)) == nil, "r1 P3: PreToolUse with \(why) -> refused")
    }
    // P2 symlink: a linked settings file is refused untouched; P2 a change between read and write is refused.
    let tmp = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hookB1-\(getpid())", isDirectory: true)
    try? FileManager.default.removeItem(at: tmp)
    try FileManager.default.createDirectory(at: tmp, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: tmp) }
    let target = tmp.appendingPathComponent("real.json"), link = tmp.appendingPathComponent("settings.json")
    try Data(#"{"a":1}"#.utf8).write(to: target)
    try FileManager.default.createSymbolicLink(at: link, withDestinationURL: target)
    check((try? ClaudeSettingsHook.apply(add: true, file: link, command: cmd)) == nil && (try? Data(contentsOf: target)) == Data(#"{"a":1}"#.utf8)
          && (try? FileManager.default.destinationOfSymbolicLink(atPath: link.path)) != nil, "r1 P2: a linked settings file -> refused, untouched")
    let plainFile = tmp.appendingPathComponent("plain.json")
    try Data(#"{"a":1}"#.utf8).write(to: plainFile)
    let raced = (try? ClaudeSettingsHook.apply(add: true, file: plainFile, command: cmd, beforeWrite: { try? Data(#"{"a":2}"#.utf8).write(to: plainFile) })) == nil
    check(raced && (try? Data(contentsOf: plainFile)) == Data(#"{"a":2}"#.utf8), "r1 P2: the file changed after the read -> refused, the other change kept")
    // P3: the script prints nothing even when its folder is not writable.
    let ro = tmp.appendingPathComponent("ro", isDirectory: true)
    let folder = ClaudeQuestionHook.Folder(url: ro)
    let script = tmp.appendingPathComponent("ask.sh")
    try folder.writeScript(to: script)
    try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: ro.path)
    defer { try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: ro.path) }
    check(ClaudeQuestionHook.script(recordsDir: "/x").contains("exec >/dev/null 2>&1"), "r1 P3: the script closes stdout and stderr first")
    let p = Process(); p.executableURL = URL(fileURLWithPath: "/bin/sh"); p.arguments = [script.path]
    let pin = Pipe(), pout = Pipe(); p.standardInput = pin; p.standardOutput = pout; p.standardError = pout
    try p.run(); pin.fileHandleForWriting.write(Data("{}".utf8)); try pin.fileHandleForWriting.close(); p.waitUntilExit()
    check(p.terminationStatus == 0 && pout.fileHandleForReading.readDataToEndOfFile().isEmpty, "r1 P3: a read-only folder -> no output, exit 0")
}
