import Foundation
@testable import HerdrKit

// Twins of herdr-core `claude_screen_wrapped_label_is_refused_before_keys` and `claude_review_quote_bar_arrow_line`
// (Codex round 5 P2s on the exact review check), with the same synthetic screens.
@MainActor func checksReviewExactWrap() {
    func req(_ id: String, _ questions: String) -> QuestionRequest {
        ClaudeQuestionHook.record(Data((#"{"session_id":"sess-rw","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":""# + id + #"","tool_input":{"questions":["# + questions + #"]}}"#).utf8), time: Date())!.request
    }
    func qJson(_ text: String, _ header: String, _ multi: Bool, _ options: [(String, String)]) -> String {
        let enc: (String) -> String = { String(decoding: try! JSONEncoder().encode($0), as: UTF8.self) }
        let opts = options.map { "{\"label\":\(enc($0.0)),\"description\":\(enc($0.1))}" }.joined(separator: ",")
        return "{\"question\":\(enc(text)),\"header\":\(enc(header)),\"multiSelect\":\(multi ? "true" : "false"),\"options\":[\(opts)]}"
    }
    let pick = { (p: [Int]) in QuestionAnswer(picks: p) }

    // P2 (1): a wrapped option label on a screen read.
    let sep = String(repeating: "─", count: 72)
    func screen(_ firstRow: String, _ firstNext: String) -> String {
        "←  ☐ Size  ☐ Tops  ✔ Submit  →\n\nWhich size?\n\n❯ 1. \(firstRow)\n     \(firstNext)\n  2. Small\n     a short one\n  3. Type something.\n\(sep)\n  4. Chat about this\n"
    }
    let wrapped = QuestionAdapter.claudeParse(screen("A very long option label that goes on and on until it reaches the", "right edge"))
    check(wrapped?.question.options.first?.label == "A very long option label that goes on and on until it reaches the", "review wrap: the row's first line is the label")
    check(wrapped?.question.cutLabels == true, "review wrap: a row that fills the line, then more text: the label may be cut")
    let plain = QuestionAdapter.claudeParse(screen("Large", "warm"))
    check(plain != nil && plain?.question.cutLabels == false, "review wrap: a short row with a description: not cut")
    func two(_ first: QuestionRequest.Question) -> QuestionRequest {
        QuestionAdapter.screenRequest(headers: ["Size", "Tops"], questions: [first, .init(header: "Tops", text: "Tops?", options: [.init(label: "Ham"), .init(label: "Egg")])])
    }
    let a = [pick([1]), pick([0])]
    if let w = wrapped?.question, let p = plain?.question {
        check(QuestionAdapter.reviewCollision(two(w), answers: a) == .cut, "review wrap: a cut label: refused before any key")
        check(QuestionAdapter.reviewBlocked(two(w), answers: a), "review wrap: reviewBlocked")
        check(QuestionAdapter.reviewNote(two(w), answers: a) == QuestionAdapter.reviewCut, "review wrap: the card says why")
        check(QuestionAdapter.reviewCollision(two(p), answers: a) == nil, "review wrap: no cut label: as before")
    }

    // P2 (2): a quoted question line that starts with "→".
    func rv(_ first: String) -> String {
        """
        ────────────────────────────────────────────────────────────────
        ←  ☒ Size  ☒ Tops  ✔ Submit  →

        Review your answers

        \(first)
         ● Which toppings?
           → Cheese, Egg

        Ready to submit your answers?

        ❯ 1. Submit answers
          2. Cancel
        """
    }
    let tops = qJson("Which toppings?", "Tops", true, [("Cheese", "y"), ("Ham", "p"), ("Egg", "r")])
    let long = "This is a deliberately long first question so the picker draws its text with a side bar, → choose a mode?"
    let q = req("toolu_rba", qJson(long, "Size", false, [("Small", "s"), ("Large", "l")]) + "," + tops)
    let ans = [pick([1]), pick([0, 2])]
    let arrowLine = " │ ● This is a deliberately long first question so the picker draws its text with a side bar,\n     │ → choose a mode?\n       → Large"
    check(QuestionAdapter.claudeReviewExact(q, answers: ans, screen: rv(arrowLine)), "review wrap: a quoted line that starts with → is question text")
    check(!QuestionAdapter.claudeReviewExact(q, answers: [pick([0]), pick([0, 2])], screen: rv(arrowLine)), "review wrap: another pick still fails")
    let short = "This is a deliberately long first question so the picker draws its text with a side bar, choose a mode?"
    let q2 = req("toolu_rbb", qJson(short, "Size", false, [("Small", "s"), ("Large", "l")]) + "," + tops)
    check(!QuestionAdapter.claudeReviewExact(q2, answers: ans, screen: rv(" │ ● \(short)\n     │ → Large")), "review wrap: a barred → line after the whole question is no answer")
    check(QuestionAdapter.claudeReviewExact(q2, answers: ans, screen: rv(" │ ● \(short)\n       → Large")), "review wrap: the plain case still passes")
}
