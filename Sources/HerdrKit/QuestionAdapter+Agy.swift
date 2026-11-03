import Foundation

/// agy's `ask_question` (agy 1.4.12, live): the call's args name the questions; the picker is answered with keys.
extension QuestionAdapter {
    /// `{questions:[{question, options:[String], is_multi_select}]}`. Labels are kept as agy shows them.
    static func agy(id: String, input: [String: Any]) -> QuestionRequest? {
        guard let qs = input["questions"] as? [[String: Any]], !qs.isEmpty else { return nil }
        let questions = qs.enumerated().map { k, q -> QuestionRequest.Question in
            let opts = (q["options"] as? [Any] ?? []).compactMap { $0 as? String }.map { QuestionRequest.Option(label: $0) }
            return .init(id: "q\(k)", header: "", text: q["question"] as? String ?? "", options: opts,
                         multiSelect: q["is_multi_select"] as? Bool ?? false)
        }
        return QuestionRequest(toolUseId: id, agent: "agy", questions: questions)
    }

    /// Result lines `A1: Blue`, `A2: …` (one per question). `User Skipped` everywhere (Esc) is a decline.
    static func agyState(_ q: QuestionRequest, text: String) -> QuestionRequest.State {
        var byIndex: [Int: String] = [:]
        for m in text.matches(of: /(?m)^A(\d+): (.*)$/) { if let i = Int(m.1) { byIndex[i - 1] = String(m.2).trimmingCharacters(in: .whitespaces) } }
        if !byIndex.isEmpty, byIndex.values.allSatisfy({ $0 == "User Skipped" }) { return .declined }
        return answered(q.questions.indices.map { i in byIndex[i].flatMap { $0 == "User Skipped" ? nil : $0 } })
    }

    /// Verified for one single-choice question: the digit picks and submits (no Enter). Write-in and multi-select are
    /// not verified, so they are refused before anything is sent. Several questions: wait for the next.
    static func agySteps(_ q: QuestionRequest, _ answers: [QuestionAnswer]) throws -> [Step] {
        var out: [Step] = []
        for (k, question) in q.questions.enumerated() {
            if question.multiSelect { throw Failure.unsupported("agy multi-select is not supported yet: answer in the terminal") }
            if otherText(answers[k]) != nil { throw Failure.unsupported("agy write-in is not supported yet: answer in the terminal") }
            out.append(.text("\(answers[k].picks[0] + 1)"))
            if k + 1 < q.questions.count { out.append(.waitFor(needle(q.questions[k + 1]))) }
        }
        return out
    }

    /// The picker shows question `k` exactly: "Question k/n: <text>" (it may wrap), the options as rows "1. …" in
    /// order, then agy's own "Write-in..." row, one ">" cursor, and the key line below. Single choice only.
    static func agyExact(_ q: QuestionRequest, question k: Int, screen: String) -> Bool {
        guard q.agent == "agy", q.questions.indices.contains(k), !q.questions[k].multiSelect else { return false }
        let want = q.questions[k]
        let lines = screen.components(separatedBy: "\n").map { $0.trimmingCharacters(in: .whitespaces) }
        guard let head = lines.lastIndex(where: { $0.firstMatch(of: #/^Question \d+/\d+: /#) != nil }),
              let m = lines[head].wholeMatch(of: /Question (\d+)\/(\d+): (.*)/),
              Int(m.1) == k + 1, Int(m.2) == q.questions.count else { return false }
        let bare: (String) -> String = { String($0.unicodeScalars.filter { !CharacterSet.whitespacesAndNewlines.contains($0) }) }
        let rowRe = /(>\s*)?(\d+)\.\s+(.*)/
        var at = head + 1, text = [String(m.3)]
        while at < lines.count, lines[at].wholeMatch(of: rowRe) == nil { text.append(lines[at]); at += 1 }
        guard bare(text.joined()) == bare(want.text) else { return false }
        var rows: [String] = [], cursors = 0
        while at < lines.count, let r = lines[at].wholeMatch(of: rowRe) {
            guard Int(r.2) == rows.count + 1 else { return false }
            if r.1 != nil { cursors += 1 }
            rows.append(String(r.3))
            at += 1
        }
        guard cursors == 1, rows == want.options.map(\.label) + ["Write-in..."],
              let foot = lines[at...].first(where: { !$0.isEmpty }) else { return false }
        return foot == "↑/↓ Navigate · enter Select · esc Skip"
    }
}
