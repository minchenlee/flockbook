import Foundation
@testable import HerdrKit

// Swift twins of PR #12 (herdr-core `question/tests.rs`: `claude_key_line_any_editor`, `claude_hook_answer`'s "cannot
// type" case, `claude_hook_exact_quote_bar`'s narrow wrap; `hc/keys.rs`: `names`, `typeable_chars`). FakeHC only.
@MainActor func checksHCParity() async throws {
    func req(_ id: String, _ questions: String) -> QuestionRequest {
        ClaudeQuestionHook.record(Data((#"{"session_id":"sess-par","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":""# + id + #"","tool_input":{"questions":["# + questions + #"]}}"#).utf8), time: Date())!.request
    }

    // MARK: claude_key_line_any_editor: "ctrl+g to edit in <the person's editor>" is the same key line as "Vim".
    let color = req("toolu_par_color", #"{"question":"Pick a color?","header":"Color","multiSelect":false,"options":[{"label":"Red (Recommended)","description":"warm and a long description that should wrap on a narrow terminal screen for sure because it is long"},{"label":"Blue","description":"cool"}]}"#)
    func single(_ keyLine: String) -> String {
        """
        ────────────────────────────────────────────────────────────────
         ☐ Color

        Pick a color?

          1. Red (Recommended)
             warm and a long description that should wrap on a narrow terminal screen for sure because it is long
          2. Blue
             cool
        ❯ 3. Type something.
        ────────────────────────────────────────────────────────────────
          4. Chat about this

        \(keyLine)
        """
    }
    let other = { (line: String) in QuestionAdapter.claudeExact(color, question: 0, screen: single(line), other: "", focus: .auto, checks: nil) }
    for l in [
        "Enter to select · ↑/↓ to navigate · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in VS Code · Esc to cancel",
        "Enter to select · ↑/↓ to navigate · ctrl+g to edit in VS Code · Esc to cancel",
        "  Enter to select  ·  ↑/↓ to navigate · ctrl+g to edit in Sublime Text · Esc to cancel",
    ] {
        check(other(l), "hc parity: key line \(l)")
    }
    for l in [
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel now",
        "Enter to select · ctrl+g to edit in Vim · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in A · B · Esc to cancel",
    ] {
        check(!other(l), "hc parity: not a key line \(l)")
    }

    // MARK: hc/keys.rs: a capital goes as shift+letter (hc reads key names without case).
    check(HCKeys.names(for: Array("Ol 1.".utf8)) == ["shift+o", "l", "space", "1", "."], "hc parity: a capital goes as shift+letter (\(HCKeys.names(for: Array("Ol 1.".utf8))))")
    check(HCKeys.names(for: Array("a b\r".utf8)) == ["a", "space", "b", "enter"], "hc parity: lower case and space unchanged")

    // MARK: claude_hook_exact_quote_bar: a narrow pane wraps inside the needle; the bar between the parts is drawing.
    let long = #"The throwaway "Greenhouse badge test" is back in greenhouse (last tab). Did it move to Orbit?"#
    let longQ = req("toolu_par_bar", #"{"question":"The throwaway \"Greenhouse badge test\" is back in greenhouse (last tab). Did it move to Orbit?","header":"Final try","multiSelect":false,"options":[{"label":"Moved + notice","description":"It's in Orbit now and the notice showed."},{"label":"Didn't move","description":"It's still in greenhouse."}]}"#)
    let narrow = """
    ❯ the prompt
    ────────────────────────────────────────────────────────────────
     ☐ Final try

    │ The throwaway
    │ "Greenhouse badge test" is back in
    │ greenhouse (last tab). Did it move to
    │ Orbit?

    ❯ 1. Moved + notice
         It's in Orbit now and the notice showed.
      2. Didn't move
         It's still in greenhouse.
      3. Type something.
    ────────────────────────────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    check(longQ.questions.first?.text == long, "hc parity: the long record reads")
    check(QuestionAdapter.claudeExact(longQ, question: 0, screen: narrow, other: nil, focus: .auto, checks: nil), "hc parity: a narrow barred wrap matches")
    check(QuestionAdapter.screenShows(longQ, screen: narrow), "hc parity: the entry check finds the needle across a barred wrap")
    if let parsed = QuestionAdapter.claudeScreen(narrow)?.question {
        var sq = longQ
        sq.toolUseId = "screen-1"
        sq.questions = [parsed]
        check(QuestionAdapter.screenShows(sq, screen: narrow), "hc parity: a screen-derived question's needle is found too")
    } else { check(false, "hc parity: the narrow screen parses") }

    // MARK: claude_hook_answer: a typed answer hc cannot type is refused before any key; capitals type.
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "question")]
    let screen: any PaneScreen = HCClient(session: HCSession(socketPath: fake.path))
    func keys() -> [[String]] { fake.calls.filter { $0.method == "terminal.keys" }.compactMap { $0.params["keys"] as? [String] } }
    let picker = """
    ❯ Ask me.
    ────────────────────
     ☐ Color
    Pick a color?
    ❯ 1. Red (Recommended)
         warm
      2. Blue
         cool
      3. Type something.
    ────────────────────
      4. Chat about this
    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    fake.setScreen("t1", picker)
    guard let q = try await screen.pendingQuestion(paneId: "hc:t1") else { check(false, "hc parity: the picker reads"); return }
    var before = keys().count
    var err: Error?
    do { try await screen.answerQuestion(paneId: "hc:t1", request: q, answers: [QuestionAnswer(other: "Green typed!")]) } catch { err = error }
    let refused: Bool = { if case .unsupported(let m)? = err as? QuestionAdapter.Failure { return m.contains("cannot type") && m.contains("answer in the terminal") }; return false }()
    check(keys().count == before && refused, "hc parity: '!' cannot be typed over hc: refused before any key (\(Array(keys().dropFirst(before))), \(String(describing: err)))")
    before = keys().count
    fake.onKeys = { addr, k in if addr == "t1", k == ["enter"] { fake.setScreen("t1", "⏺ User answered: Olive Oil\n> ") } }
    err = nil
    do { try await screen.answerQuestion(paneId: "hc:t1", request: q, answers: [QuestionAnswer(other: "Olive Oil")]) } catch { err = error }
    fake.onKeys = nil
    let sent = Array(keys().dropFirst(before))
    check(err == nil && sent == [["3"], ["shift+o", "l", "i", "v", "e", "space", "shift+o", "i", "l"], ["enter"]],
          "hc parity: a typed answer with capitals goes as shift+letter (\(sent), \(String(describing: err)))")
}
