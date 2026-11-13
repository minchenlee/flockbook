import Foundation
@testable import HerdrKit

// Swift twins of `herdr-core/src/question/tests.rs`:
// - `claude_hook_exact_quote_bar` (displayWidth + QUOTE_MIN = 80)
// - `review_exact_screen_cases_red` (#1..#10, screen- parity, duplicate/comma/quoted/cap checks)
// - `claude_review_wrapped_bar_dot_in_answer_red`
// - `claude_review_vertical_bar_fail_closed_red`
// - `claude_review_variant_cap_keeps_full_answer_red`
// - `claude_review_variant_and_tuple_caps_fail_closed`
@MainActor func checksReviewExact() async throws {
    func req(_ id: String, _ questions: String) -> QuestionRequest {
        ClaudeQuestionHook.record(Data((#"{"session_id":"sess-re","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":""# + id + #"","tool_input":{"questions":["# + questions + #"]}}"#).utf8), time: Date())!.request
    }
    func qJson(_ text: String, _ header: String, _ multi: Bool, _ options: [(String, String, Bool)]) -> String {
        let enc: (String) -> String = {
            String(decoding: try! JSONEncoder().encode($0), as: UTF8.self)
        }
        let opts = options.map { label, desc, rec in
            let full = rec ? "\(label) (Recommended)" : label
            return "{\"label\":\(enc(full)),\"description\":\(enc(desc))}"
        }.joined(separator: ",")
        return "{\"question\":\(enc(text)),\"header\":\(enc(header)),\"multiSelect\":\(multi ? "true" : "false"),\"options\":[\(opts)]}"
    }
    func rv(_ tabs: String, _ body: String) -> String {
        """
        ────────────────────────────────────────────────────────────────
        ←  \(tabs)  ✔ Submit  →

        Review your answers

        \(body)

        Ready to submit your answers?

        ❯ 1. Submit answers
          2. Cancel
        """
    }
    let pick = { (p: [Int]) in QuestionAnswer(picks: p) }
    let own = { (p: [Int], t: String) in QuestionAnswer(picks: p, other: t) }

    // MARK: claude_hook_exact_quote_bar (80 display columns in claudeExact and claudeParse)
    let long = #"The throwaway "Greenhouse badge test" is back in greenhouse (last tab). Did it move to Orbit?"#
    let longQ = req("toolu_bar1", qJson(long, "Final try", false, [
        ("Moved + notice", "It's in Orbit now and the notice showed.", false),
        ("Didn't move", "It's still in greenhouse.", false),
    ]))
    func pickerScreen(_ qBody: String) -> String {
        """
         ☐ Final try
        \(qBody)
        ❯ 1. Moved + notice
             It's in Orbit now and the notice showed.
          2. Didn't move
             It's still in greenhouse.
          3. Type something.
        ────────────────────
          4. Chat about this
        Enter to select · ↑/↓ to navigate · Esc to cancel
        """
    }
    let oneBar = pickerScreen("│ \(long)")
    let wrappedBar = pickerScreen("│ The throwaway \"Greenhouse badge test\" is back in greenhouse\n    │ (last tab). Did it move to Orbit?")
    check(QuestionAdapter.claudeExact(longQ, question: 0, screen: pickerScreen(long), other: nil, focus: .auto, checks: nil),
          "quote bar: plain long question matches")
    check(QuestionAdapter.claudeExact(longQ, question: 0, screen: oneBar, other: nil, focus: .auto, checks: nil),
          "quote bar: one barred line matches")
    check(QuestionAdapter.claudeExact(longQ, question: 0, screen: wrappedBar, other: nil, focus: .auto, checks: nil),
          "quote bar: every wrapped line barred matches")
    let halfBar = pickerScreen("│ The throwaway \"Greenhouse badge test\" is back in greenhouse\n    (last tab). Did it move to Orbit?")
    check(!QuestionAdapter.claudeExact(longQ, question: 0, screen: halfBar, other: nil, focus: .auto, checks: nil),
          "quote bar: bar on some lines only is text, not drawing -> no match")
    check(QuestionAdapter.claudeScreen(wrappedBar)?.question.text == long,
          "quote bar: claudeParse drops the bar on long text")

    let barredQ = req("toolu_bar2", qJson("│ starts with a bar?", "Final try", false, [
        ("Moved + notice", "It's in Orbit now and the notice showed.", false),
        ("Didn't move", "It's still in greenhouse.", false),
    ]))
    check(!QuestionAdapter.claudeExact(barredQ, question: 0, screen: pickerScreen("│ │ starts with a bar?"), other: nil, focus: .auto, checks: nil),
          "quote bar: short question starting with │ is never unquoted")
    check(QuestionAdapter.claudeExact(barredQ, question: 0, screen: pickerScreen("│ starts with a bar?"), other: nil, focus: .auto, checks: nil),
          "quote bar: short text starting with │ matches its own bar")

    // Narrow wrap inside needle + screen-derived review across barred wrap
    let narrowBar = pickerScreen("│ The throwaway\n    │ \"Greenhouse badge test\" is back in\n    │ greenhouse (last tab). Did it move to\n    │ Orbit?")
    check(QuestionAdapter.claudeExact(longQ, question: 0, screen: narrowBar, other: nil, focus: .auto, checks: nil),
          "quote bar: narrow wrap matches")
    let reviewBar = rv("☒ Final try", """
     │ ● The throwaway
     │ "Greenhouse badge test" is back in greenhouse (last tab). Did it move to Orbit?
       → Moved + notice
    """)
    var parsedLong = longQ
    parsedLong.toolUseId = "screen-2"
    parsedLong.questions[0].multiSelect = true
    check(!QuestionAdapter.showsReview(parsedLong, screen: reviewBar, answers: nil),
          "quote bar: None returns false on Claude")
    check(QuestionAdapter.showsReview(parsedLong, screen: reviewBar, answers: [own([], "Moved + notice")]),
          "quote bar: screen-derived review across barred wrap matches")

    // Own │ on long text refuses in claudeExact
    var ownLongQ = longQ
    ownLongQ.toolUseId = "toolu_bar3"
    ownLongQ.questions[0].text = "│ \(long)"
    check(!QuestionAdapter.claudeExact(ownLongQ, question: 0, screen: oneBar, other: nil, focus: .auto, checks: nil),
          "quote bar: long text starting with │ refused")
    check(QuestionAdapter.claudeExact(longQ, question: 0, screen: oneBar, other: nil, focus: .auto, checks: nil),
          "quote bar: plain long question matches barred screen")
    check(!QuestionAdapter.claudeExact(longQ, question: 0, screen: pickerScreen("│ │ \(long)"), other: nil, focus: .auto, checks: nil),
          "quote bar: double bar stays refused")

    // MARK: #1 Screen 1 — screen- multi-question + CJK Q2 (>= 80 display cols, < 80 graphemes)
    let q1Text = "Reproduce issue part 1: pick the module and mode (this line is 74 cols!!)"
    let q2Text = "重現用第2題：請問你最想針對哪一個模組進行重構與測試，用來確認寬度超過八十欄時是否出現線條？"
    check(q2Text.count < 80, "#1 fixture: CJK character count < 80")
    let cjkPicker = """
    ←  ☒ Mode  ☐ Module  ✔ Submit  →
    │ \(q2Text)
    ❯ 1. Option X
      2. Option Y
      3. Type something.
    ────────────────────
      4. Chat about this
    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    check(QuestionAdapter.claudeScreen(cjkPicker)?.question.text == q2Text,
          "#1: CJK >= 80 display cols strips quote bar in claudeParse")
    let cjk79 = String(repeating: "寬", count: 39) + "a"   // 79 cols < 80
    let cjk80 = String(repeating: "寬", count: 40)         // 80 cols >= 80
    func cjkOf(_ t: String) -> String {
        """
        ←  ☐ H  ✔ Submit  →
        │ \(t)
        ❯ 1. Yes
          2. No
          3. Type something.
        ────────────────────
          4. Chat about this
        Enter to select · ↑/↓ to navigate · Esc to cancel
        """
    }
    check(QuestionAdapter.claudeScreen(cjkOf(cjk79))?.question.text == "│ \(cjk79)", "#1 boundary: 79 cols keeps │")
    check(QuestionAdapter.claudeScreen(cjkOf(cjk80))?.question.text == cjk80, "#1 boundary: 80 cols strips │")

    var screenQ = req("toolu_s1", qJson(q1Text, "Mode", true, [("Primary", "", false), ("Secondary", "", false)]) + ","
                      + qJson(q2Text, "Module", false, [("Option X", "", false), ("Option Y", "", false)]))
    screenQ.toolUseId = "screen-1"
    let s1Review = rv("☒ Mode  ☒ Module", " ● \(q1Text)\n   → Primary\n │ ● \(q2Text)\n   → Option Y")
    let s1Ans = [pick([0]), pick([1])]
    check(!QuestionAdapter.showsReview(screenQ, screen: s1Review, answers: nil), "#1: screen- without answers returns false")
    check(QuestionAdapter.showsReview(screenQ, screen: s1Review, answers: s1Ans), "#1: screen- with matching answers matches review")
    check(!QuestionAdapter.showsReview(screenQ, screen: s1Review, answers: [pick([1]), pick([1])]), "#1: wrong pick does not match")

    // MARK: #2 & #4 Screen 2 — multi-select comma option quoted vs old unquoted
    let qRed = req("toolu_S2", qJson("Pick colors?", "Colors", true, [("Red, Blue", "", false), ("Red", "", false), ("Blue", "", false)]))
    check(QuestionAdapter.reviewBlocked(qRed, answers: []), "#2: empty answers flags potential Red,Blue collision")
    check(!QuestionAdapter.reviewBlocked(qRed, answers: [pick([0])]), "#2: picking 'Red, Blue' alone is safe under quoted rendering")
    let quotedReview = rv("☒ Colors", " ● Pick colors?\n   → \"Red, Blue\"")
    check(QuestionAdapter.claudeReviewExact(qRed, answers: [pick([0])], screen: quotedReview), "#2: quoted comma pick matches exactly")
    let unquotedReview = rv("☒ Colors", " ● Pick colors?\n   → Red, Blue")
    check(!QuestionAdapter.claudeReviewExact(qRed, answers: [pick([0])], screen: unquotedReview), "#4: unquoted 'Red, Blue' is ambiguous -> false")
    check(!QuestionAdapter.claudeReviewExact(qRed, answers: [pick([1, 2])], screen: unquotedReview), "#4: unquoted [Red, Blue] is ambiguous -> false")

    // MARK: #3 Screen 3 — single-choice comma option on a 2-question review screen
    let qSingle = req("toolu_S3", qJson("Apply?", "Go", false, [("Yes, apply it", "", false), ("No", "", false)]) + ","
                      + qJson("Next?", "Step", false, [("Done", "", false), ("Skip", "", false)]))
    check(!QuestionAdapter.reviewBlocked(qSingle, answers: []), "#3: single-choice comma is clean")
    check(QuestionAdapter.claudeReviewExact(qSingle, answers: [pick([0]), pick([0])],
                                            screen: rv("☒ Go  ☒ Step", " ● Apply?\n   → Yes, apply it\n ● Next?\n   → Done")),
          "#3: single-choice comma review matches")

    // MARK: #5 'A B' vs 'AB' on screen- request (no bypass)
    var screenAB = req("toolu_ab", qJson("First?", "One", false, [("A B", "", false), ("AB", "", false)]) + ","
                       + qJson("Second?", "Two", false, [("C", "", false), ("D", "", false)]))
    screenAB.toolUseId = "screen-ab"
    check(QuestionAdapter.reviewBlocked(screenAB, answers: []), "#5: screen- no longer bypasses reviewBlocked")
    check(QuestionAdapter.reviewBlocked(screenAB, answers: [pick([0]), pick([0])]), "#5: colliding pick on screen- is blocked")
    check(!QuestionAdapter.reviewBlocked(screenAB, answers: [own([], "fine"), pick([0])]), "#5: non-colliding Other on screen- is allowed")
    let abReview = rv("☒ One  ☒ Two", " ● First?\n   → AB\n ● Second?\n   → C")
    check(!QuestionAdapter.claudeReviewExact(screenAB, answers: [pick([0]), pick([0])], screen: abReview), "#5: screen- rejects colliding review")
    let fineReview = rv("☒ One  ☒ Two", " ● First?\n   → fine\n ● Second?\n   → C")
    check(QuestionAdapter.claudeReviewExact(screenAB, answers: [own([], "fine"), pick([0])], screen: fineReview), "#5: screen- accepts non-colliding Other")

    // MARK: #6 Duplicate labels ["Yes", "Yes", "No"] — empty / partial / [0] / [2]
    let qDup = req("toolu_dup", qJson("Q1?", "H1", false, [("Yes", "", false), ("Yes", "", false), ("No", "", false)]) + ","
                   + qJson("Q2?", "H2", false, [("Alpha", "", false), ("Beta", "", false)]))
    check(QuestionAdapter.reviewNote(qDup, answers: []) == QuestionAdapter.reviewLabels, "#6: empty answers -> reviewLabels")
    check(QuestionAdapter.reviewBlocked(qDup, answers: [QuestionAnswer(), pick([0])]), "#6: partial with Q1 unanswered -> blocked")
    check(QuestionAdapter.reviewBlocked(qDup, answers: [pick([0]), QuestionAnswer()]), "#6: partial picking Yes -> blocked")
    check(!QuestionAdapter.reviewBlocked(qDup, answers: [pick([2]), QuestionAnswer()]), "#6: partial picking unique No -> allowed")
    check(!QuestionAdapter.reviewBlocked(qDup, answers: [pick([2]), pick([0])]), "#6: complete picking unique No -> allowed")
    check(QuestionAdapter.claudeReviewExact(qDup, answers: [pick([2]), pick([0])],
                                            screen: rv("☒ H1  ☒ H2", " ● Q1?\n   → No\n ● Q2?\n   → Alpha")),
          "#6: picking unique No passes claudeReviewExact")

    // MARK: #7 Hard caps: literal quote and empty bare
    let qQuoteOpt = req("toolu_q", qJson("Q?", "H", true, [("say \"hi\"", "", false), ("ok", "", false)]))
    check(QuestionAdapter.reviewBlocked(qQuoteOpt, answers: []), "#7: literal quote in option -> blocked")
    let qClean = req("toolu_cl", qJson("Q?", "H", true, [("A", "", false), ("B", "", false)]))
    check(QuestionAdapter.reviewNote(qClean, answers: [own([], "say \"hi\"")]) == QuestionAdapter.reviewOwn, "#7: literal quote in Other -> reviewOwn")
    let qEmptyOpt = req("toolu_em", qJson("Q?", "H", true, [("   ", "", false), ("ok", "", false)]))
    check(QuestionAdapter.reviewBlocked(qEmptyOpt, answers: []), "#7: empty bare option -> blocked")

    // MARK: #8 Other collisions & multi-select comma Other
    let qXY = req("toolu_xy", qJson("Q?", "H", true, [("x", "", false), ("y", "", false)]))
    check(QuestionAdapter.reviewNote(qXY, answers: [own([], "x, y")]) == QuestionAdapter.reviewOwn, "#8: Other('x, y') collides conservatively with [x, y]")
    check(!QuestionAdapter.reviewBlocked(qClean, answers: [own([], "x, y")]), "#8: Other('x, y') with clean ['A','B'] is safe")
    check(QuestionAdapter.claudeReviewExact(qClean, answers: [own([], "x, y")], screen: rv("☒ H", " ● Q?\n   → x, y")),
          "#8: clean unquoted Other with comma passes")
    check(QuestionAdapter.claudeReviewExact(qClean, answers: [own([], "x, y")], screen: rv("☒ H", " ● Q?\n   → \"x, y\"")),
          "#8: clean quoted Other with comma passes")
    check(!QuestionAdapter.claudeReviewExact(qXY, answers: [own([], "x, y")], screen: rv("☒ H", " ● Q?\n   → x, y")),
          "#8: colliding unquoted Other fails")
    check(QuestionAdapter.claudeReviewExact(qXY, answers: [own([], "x, y")], screen: rv("☒ H", " ● Q?\n   → \"x, y\"")),
          "#8: unambiguously quoted Other passes")
    check(QuestionAdapter.reviewBlocked(qClean, answers: [own([0], "B")]), "#8: A + Other('B') collides with [A, B]")

    // MARK: #9 & Wrapped │B / │●B regression guards + Round 2/1 vertical bar fail-closed
    var screenBar = req("toolu_rvbar", qJson("This is a deliberately long first question so the picker draws its text with a side bar, which size?", "Size", true,
                                             [("A", "a", false), ("B", "b", false), ("│B", "c", false)]))
    screenBar.toolUseId = "screen-rvbar"
    let barReview = rv("☒ Size", " │ ● This is a deliberately long first question so the picker draws its text with a side bar, which size?\n   → A,\n   │B")
    check(QuestionAdapter.reviewNote(screenBar, answers: []) == QuestionAdapter.reviewLabels, "#9: '│' in option label refused on screen-")
    check(!QuestionAdapter.showsReview(screenBar, screen: barReview, answers: [pick([0, 2])]), "#9: '│B' fails closed in review matching")
    check(!QuestionAdapter.showsReview(screenBar, screen: barReview, answers: [pick([0, 1])]), "#9: wrapped │B does not match B")
    check(!QuestionAdapter.showsReview(screenBar, screen: barReview, answers: nil), "#9: screen- without answers never matches")

    let dotQ = req("toolu_p1_bar_dot", qJson("Pick options?", "Pick", true, [
        ("Long first pick that wraps onto the next line", "", false),
        ("│●B", "", false),
        ("●B", "", false),
    ]))
    let dotReview = rv("☒ Pick", " ● Pick options?\n   → Long first pick that wraps onto the next line,\n   │●B")
    check(!QuestionAdapter.claudeReviewExact(dotQ, answers: [pick([0, 2])], screen: dotReview), "P1: wrapped '│●B' must not match '●B'")
    check(!QuestionAdapter.claudeReviewExact(dotQ, answers: [pick([0, 1])], screen: dotReview), "P1: fail closed whenever option contains '│'")
    check(QuestionAdapter.reviewNote(dotQ, answers: []) == QuestionAdapter.reviewLabels, "P1: '│●B' option refused before any key")

    let q2Long = "This is a deliberately long second question so the picker draws its text with a side bar, which mode?"
    let dotTwoQ = req("toolu_p1_bar_dot_2", qJson("Pick options?", "Pick", true, [
        ("Long first pick that wraps onto the next line", "", false),
        ("│●B", "", false),
        ("●B", "", false),
    ]) + "," + qJson(q2Long, "Mode", false, [("Safe", "", false), ("Fast", "", false)]))
    let dotReviewTwo = rv("☒ Pick  ☒ Mode", " ● Pick options?\n   → Long first pick that wraps onto the next line,\n   │●B\n │ ● \(q2Long)\n   → Safe")
    check(!QuestionAdapter.claudeReviewExact(dotTwoQ, answers: [pick([0, 2]), pick([0])], screen: dotReviewTwo),
          "P1 multi-q: wrapped '│●B' must not match wrong pick '●B'")
    check(!QuestionAdapter.claudeReviewExact(dotTwoQ, answers: [pick([0, 1]), pick([0])], screen: dotReviewTwo),
          "P1 multi-q: fail closed whenever option contains '│'")

    // Round 2 P1: wrapped answer line "│ ● <full long question>" followed by "→ ..."
    let qLongR2 = "This is a deliberately long question used to test quote bar stripping boundary on review screens"
    let r2P1 = req("toolu_r2_p1", qJson(qLongR2, "R2", true, [
        ("Wrap,\n │ ● \(qLongR2)\n → Tail", "", false),
        ("Wrap,\n ● \(qLongR2)\n → Tail", "", false),
    ]))
    let reviewR2P1 = rv("☒ R2", " │ ● \(qLongR2)\n   → Wrap,\n │ ● \(qLongR2)\n → Tail")
    check(QuestionAdapter.reviewNote(r2P1, answers: []) == QuestionAdapter.reviewLabels, "R2 P1: '│' in option label refused before any key")
    check(!QuestionAdapter.claudeReviewExact(r2P1, answers: [pick([1])], screen: reviewR2P1), "R2 P1: wrong option [1] must not match")
    check(!QuestionAdapter.claudeReviewExact(r2P1, answers: [pick([0])], screen: reviewR2P1), "R2 P1: option [0] with '│' must also fail closed")

    let qTextBar = req("toolu_qbar", qJson("│ Q1?", "One", false, [("A", "", false), ("B", "", false)]) + ","
                       + qJson("Q2?", "Two", false, [("C", "", false), ("D", "", false)]))
    check(QuestionAdapter.reviewNote(qTextBar, answers: []) == QuestionAdapter.reviewLabels, "R2 P1: question text with '│' -> reviewLabels")
    let cleanPair = req("toolu_ownbar", qJson("Q1?", "One", false, [("A", "", false), ("B", "", false)]) + ","
                        + qJson("Q2?", "Two", false, [("C", "", false), ("D", "", false)]))
    check(QuestionAdapter.reviewNote(cleanPair, answers: [own([], "bad │ own"), pick([0])]) == QuestionAdapter.reviewOwn,
          "R2 P1: Other text with '│' -> reviewOwn")
    check(!QuestionAdapter.claudeReviewExact(cleanPair, answers: [own([], "bad │ own"), pick([0])],
                                             screen: rv("☒ One  ☒ Two", " ● Q1?\n   → bad │ own\n ● Q2?\n   → C")),
          "R2 P1: Other text with '│' fails closed in claudeReviewExact")
    let recBar = req("toolu_rec_bar", qJson("Q?", "H", true, [("│Choice", "", true), ("Plain", "", false)]))
    check(QuestionAdapter.reviewNote(recBar, answers: []) == QuestionAdapter.reviewLabels, "R2 P1: recommended option with '│' fails closed")

    // MARK: #10 n_k > 8, missing title, cross-question newline marks, Round 3 P2 conservative pre-flight
    let nineOpts = (0..<9).map { ("Opt\($0)", "", false) }
    let qNine = req("toolu_9", qJson("Q?", "H", true, nineOpts))
    check(QuestionAdapter.reviewBlocked(qNine, answers: []), "#10: n_k > 8 -> fail closed")
    check(!QuestionAdapter.claudeReviewExact(qRed, answers: [pick([0])], screen: quotedReview.replacingOccurrences(of: "Review your answers", with: "")),
          "#10: missing title -> false")
    let crossQ = req("toolu_cross", qJson("First?", "One", false, [("A", "", false), ("A\n ● Second?\n → B", "", false)]) + ","
                     + qJson("Second?", "Two", false, [("B\n ● Second?\n → C", "", false), ("C", "", false)]))
    check(QuestionAdapter.reviewBlocked(crossQ, answers: [pick([0]), pick([0])]), "#10: newline mark cross-question collision blocked")
    check(!QuestionAdapter.claudeReviewExact(crossQ, answers: [pick([0]), pick([0])],
                                             screen: rv("☒ One  ☒ Two", " ● First?\n   → A\n ● Second?\n   → B\n ● Second?\n   → C")),
          "#10: ambiguous newline mark screen rejected")
    let p2Req = req("toolu_P2", qJson("Colors?", "C", true, [("Red, Blue", "", false), ("Red", "", false), ("Blue", "", false)]) + ","
                    + qJson("More?", "M", true, [("X, Y", "", false), ("Z", "", false)]))
    check(QuestionAdapter.reviewBlocked(p2Req, answers: [pick([1, 2]), pick([0])]),
          "#10 Round 3 P2: [Red, Blue] stays conservatively blocked even when Q2 has quoted comma")

    // MARK: Variant & tuple cap checks (Round 3 + Round 4 P2-b)
    // Under cap: 2 recommended picks + → D = 36 <= 64 variants -> allowed and matches exact review
    let underCap = req("toolu_r4_under", qJson("Features?", "Features", true, [("Opt1", "", true), ("Opt2", "", true), ("→ D", "", false)]) + ","
                       + qJson("Confirm?", "Confirm", false, [("Yes", "", false), ("No", "", false)]))
    let underAns = [pick([0, 1, 2]), pick([0])]
    check(!QuestionAdapter.reviewBlocked(underCap, answers: underAns),
          "cap <= 64: 2 recommended picks + → D (36 bodies) -> not blocked")
    check(QuestionAdapter.claudeReviewExact(underCap, answers: underAns,
                                            screen: rv("☒ Features  ☒ Confirm", " ● Features?\n   → Opt1 (Recommended), Opt2 (Recommended), → D\n ● Confirm?\n   → Yes")),
          "cap <= 64: matching review screen passes claudeReviewExact")

    // Over 64-body cap: 3 recommended picks + → D (216 > 64) -> fails closed
    let over64 = req("toolu_r4_over64", qJson("Features?", "Features", true, [("Opt1", "", true), ("Opt2", "", true), ("Opt3", "", true), ("→ D", "", false)]) + ","
                     + qJson("Confirm?", "Confirm", false, [("Yes", "", false), ("No", "", false)]))
    check(QuestionAdapter.reviewBlocked(over64, answers: [pick([0, 1, 2, 3]), pick([0])]),
          "cap > 64: exceeding 64 raw body variants fails closed")

    // Codex round-4 counterexample: 3 recommended picks + synthetic label matching #216 ("A(推薦),B(推薦),C(推薦)")
    let codexR4 = req("toolu_r4_codex_ex", qJson("Pick features?", "Pick", true, [("A", "", true), ("B", "", true), ("C", "", true), ("A(推薦),B(推薦),C(推薦)", "", false), ("→", "", false)]) + ","
                      + qJson("Confirm?", "Confirm", false, [("Yes", "", false), ("No", "", false)]))
    check(QuestionAdapter.reviewBlocked(codexR4, answers: [pick([0, 1, 2]), pick([0])]),
          "cap > 64: Codex round-4 hidden collision beyond 64 fails closed")

    // Over 256-tuple cap: 36 * 36 = 1296 > 256 -> fails closed
    let over256 = req("toolu_r4_over256", qJson("Q1?", "Q1", true, [("P1", "", true), ("P2", "", true), ("→", "", false)]) + ","
                      + qJson("Q2?", "Q2", true, [("Q1", "", true), ("Q2", "", true)]))
    check(QuestionAdapter.reviewBlocked(over256, answers: [pick([0, 1]), pick([0, 1])]),
          "cap > 256: exceeding 256 cross-question tuples fails closed")
}
