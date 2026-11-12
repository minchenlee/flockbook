import Foundation
@testable import HerdrKit

// Twins of herdr-core `claude_review_comma_single_choice` and `review_blocked_is_the_answer_refusal` (PR #14 and
// issue #11 item 4): a comma counts only in a multi-choice question; `reviewBlocked` is the whole refusal the
// answer makes, so the card can show it before Send.
@MainActor func checksReviewBlocked() {
    func req(_ id: String, _ questions: String) -> QuestionRequest {
        ClaudeQuestionHook.record(Data((#"{"session_id":"sess-rb","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":""# + id + #"","tool_input":{"questions":["# + questions + #"]}}"#).utf8), time: Date())!.request
    }
    func q(_ text: String, _ header: String, _ multi: Bool, _ labels: [String]) -> String {
        let opts = labels.map { #"{"label":""# + $0 + #"","description":""}"# }.joined(separator: ",")
        return #"{"question":""# + text + #"","header":""# + header + #"","multiSelect":"# + (multi ? "true" : "false") + #","options":["# + opts + "]}"
    }
    let pick = { (p: [Int]) in QuestionAnswer(picks: p) }
    let own = { (p: [Int], t: String) in QuestionAnswer(picks: p, other: t) }

    // A single-choice question's review line shows one answer, never picks joined by commas.
    let c = req("toolu_C", q("Remind you?", "Nudge", false, ["Yes, remind me Dec 28", "No / not yet"]) + "," + q("Second?", "Two", false, ["C", "D"]))
    let want = [pick([0]), pick([1])]
    check(!QuestionAdapter.reviewAmbiguous(c, answers: want), "single choice: a comma in a label is not ambiguous")
    check(!QuestionAdapter.reviewAmbiguous(c, answers: [own([], "Later, in January"), pick([1])]), "single choice: Other text with a comma is not ambiguous")
    check(QuestionAdapter.reviewAmbiguous(c, answers: [own([], "No / not yet"), pick([1])]), "single choice: Other text alike a label is still ambiguous")
    let review = """
    ←  ☒ Nudge  ☒ Two  ✔ Submit  →

    Review your answers

     ● Remind you?
       → Yes, remind me Dec 28
     ● Second?
       → D

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    """
    check(QuestionAdapter.claudeReviewExact(c, answers: want, screen: review), "single choice: the review of a comma label matches")
    check(!QuestionAdapter.claudeReviewExact(c, answers: [pick([1]), pick([1])], screen: review), "single choice: another pick -> no match")
    check(!QuestionAdapter.claudeReviewExact(c, answers: want, screen: review.replacingOccurrences(of: "→ Yes, remind me Dec 28", with: "→ No / not yet")),
          "single choice: another answer shown -> no match")
    let ownReview = review.replacingOccurrences(of: "→ Yes, remind me Dec 28", with: "→ Later, in January")
    check(QuestionAdapter.claudeReviewExact(c, answers: [own([], "Later, in January"), pick([1])], screen: ownReview), "single choice: Other text with a comma matches")
    let multi = req("toolu_M2", q("Which?", "Many", true, ["Yes, remind me", "Yes", "remind me"]))
    check(QuestionAdapter.reviewAmbiguous(multi, answers: [pick([0])]), "multi choice: a comma in a label is still ambiguous")
    check(QuestionAdapter.reviewAmbiguous(multi, answers: [own([1], "x, y")]), "multi choice: Other text with a comma is still ambiguous")

    // reviewBlocked: only a hooked Claude picker with 2+ questions or a multi-choice last one.
    func two(_ a: String, _ b: String, _ multi: Bool) -> QuestionRequest {
        req("toolu_B", q("First?", "One", multi, [a, b]) + "," + q("Second?", "Two", false, ["C", "D"]))
    }
    check(QuestionAdapter.reviewBlocked(two("A B", "AB", false), answers: []), "reviewBlocked: two labels alike -> blocked before any answer")
    check(QuestionAdapter.reviewBlocked(two("x, y", "z", true), answers: []), "reviewBlocked: multi choice, a comma label -> blocked")
    check(!QuestionAdapter.reviewBlocked(two("x, y", "z", false), answers: []), "reviewBlocked: single choice, a comma label -> fine")
    check(!QuestionAdapter.reviewBlocked(two("A", "B", true), answers: []), "reviewBlocked: clean labels -> fine")
    check(QuestionAdapter.reviewBlocked(two("A", "B", true), answers: [own([], "x, y"), pick([0])]), "reviewBlocked: own answer with a comma (multi) -> blocked")
    check(QuestionAdapter.reviewBlocked(two("A", "B", false), answers: [own([], "B"), pick([0])]), "reviewBlocked: own answer alike a label -> blocked")
    check(!QuestionAdapter.reviewBlocked(two("A", "B", false), answers: [own([], "later, maybe"), pick([0])]), "reviewBlocked: own answer with a comma (single) -> fine")
    let one = req("toolu_1", q("First?", "One", false, ["A B", "AB"]))
    check(!QuestionAdapter.reviewBlocked(one, answers: []), "reviewBlocked: one single-choice question -> fine")
    var oneMulti = one
    oneMulti.questions[0].multiSelect = true
    check(QuestionAdapter.reviewBlocked(oneMulti, answers: []), "reviewBlocked: one multi-choice question -> checked")
    var screen = two("A B", "AB", false); screen.toolUseId = "screen-1"
    check(!QuestionAdapter.reviewBlocked(screen, answers: []), "reviewBlocked: screen question -> fine")
    var codex = two("A B", "AB", false); codex.agent = "codex"
    check(!QuestionAdapter.reviewBlocked(codex, answers: []), "reviewBlocked: another agent -> fine")
    var message = two("A B", "AB", false); message.delivery = .message
    check(!QuestionAdapter.reviewBlocked(message, answers: []), "reviewBlocked: a message answer -> no review screen")

    // reviewNote: what the card says before Send (the Herdr app mod's words). The labels first, then the own answer.
    check(QuestionAdapter.reviewNote(two("x, y", "z", true), answers: []) == QuestionAdapter.reviewLabels, "reviewNote: a comma label -> blocked from the start")
    check(QuestionAdapter.reviewNote(two("A B", "AB", false), answers: [own([], "fine"), pick([0])]) == QuestionAdapter.reviewLabels, "reviewNote: labels first, whatever is typed")
    check(QuestionAdapter.reviewNote(two("A", "B", true), answers: []) == nil, "reviewNote: clean labels -> none")
    check(QuestionAdapter.reviewNote(two("A", "B", true), answers: [own([], "x, y"), pick([0])]) == QuestionAdapter.reviewOwn, "reviewNote: own answer with a comma")
    check(QuestionAdapter.reviewNote(two("A", "B", true), answers: [own([], "  B "), pick([0])]) == QuestionAdapter.reviewOwn, "reviewNote: own answer alike an option")
    check(QuestionAdapter.reviewNote(two("A", "B", true), answers: [own([], "olive"), pick([0])]) == nil, "reviewNote: a clean own answer -> none")
    var unread = two("x, y", "z", true); unread.headers = ["One", "Two", "Three"]
    check(QuestionAdapter.reviewNote(unread, answers: []) == nil, "reviewNote: questions unread -> none (the answer reads them first)")
    check(QuestionAdapter.reviewLabels.hasSuffix("Answer in the terminal.") && QuestionAdapter.reviewOwn.hasSuffix("Change it, or answer in the terminal."), "reviewNote: the mod's words")
}
