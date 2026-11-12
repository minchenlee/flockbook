import Foundation

// Claude Code 2.1.296's picker for a request the hub knows from the AskUserQuestion hook (`ClaudeQuestionHook`).
// Like `codexExact`: the screen must be the request's own rendering, compared with all whitespace dropped (wrapping
// does not count). Only the focus mark, the [✔] boxes, the tab-bar boxes and the hub's own Other text may vary,
// each at a known place. Anything else (a new Claude Code version, the user's own text) refuses: no key.
extension QuestionAdapter {
    /// Claude Code 2.1.296's key lines under a question (spacing aside), as captured live.
    static let claudeKeyLines: Set<String> = [
        "Enter to select · ↑/↓ to navigate · Esc to cancel",
        "Enter to select · ↑/↓ to navigate · ctrl+g to edit in Vim · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel",
    ]

    static func bare(_ s: String) -> String {
        String(s.unicodeScalars.filter { !CharacterSet.whitespacesAndNewlines.contains($0) })
    }

    /// The tab bar of `q`, whitespace dropped: one question, single choice: "☐Header"; else "←☐A☐B✔Submit→". A
    /// question's box is ☐ or ☒ (answered or not: Claude's own state, no key depends on it).
    static func claudeBar(_ q: QuestionRequest, _ line: String) -> Bool {
        var s = Substring(bare(line))
        func take(_ lit: String) -> Bool { guard s.hasPrefix(lit) else { return false }; s = s.dropFirst(lit.count); return true }
        let tabs = q.questions.count > 1 || q.questions.first?.multiSelect == true
        if tabs { guard take("←") else { return false } }
        for k in q.questions {
            guard take("☐") || take("☒"), take(bare(k.header)) else { return false }
        }
        if tabs { guard take("✔Submit→") else { return false } }
        return s.isEmpty
    }

    /// The label as Claude shows it: the log's label, with its "(Recommended)" when it had one.
    static func shownLabels(_ o: QuestionRequest.Option) -> [String] {
        o.recommended ? recommendedSuffixes.map { bare(o.label + $0) } : [bare(o.label)]
    }

    /// The picker on screen is question `k` of the hooked request `q`, exactly. From the bottom: the key line (the
    /// last line), "n+2. Chat about this" alone after the separator line, then from the last tab-bar line: the bar,
    /// the question text, per row an optional focus mark "❯", "i.", the box (multiSelect), the label and the
    /// description, then the Other row ("Type something." / "Type something" while empty, else `other`: the text
    /// the hub typed) and, multiSelect, the Submit row. Exactly one focus mark, on a row or Submit; `focus` says
    /// which one it must be (`.auto`: from `other`, as in `shows`). `checks`: the [✔] rows (Other = index n), when given.
    static func claudeExact(_ q: QuestionRequest, question k: Int, screen: String, other: String?, focus: FieldFocus, checks: Set<Int>?) -> Bool {
        guard q.agent == "claude", !q.toolUseId.hasPrefix("screen-"), q.questions.indices.contains(k) else { return false }
        let want = q.questions[k], n = want.options.count
        let lines = screen.components(separatedBy: "\n")
        let blank: (String) -> Bool = { $0.trimmingCharacters(in: .whitespaces).isEmpty }
        guard let foot = lines.lastIndex(where: { !blank($0) }),
              claudeKeyLines.contains(lines[foot].split(whereSeparator: \.isWhitespace).joined(separator: " ")),
              let sep = lines[..<foot].lastIndex(where: { l in
                  let t = l.trimmingCharacters(in: .whitespaces); return t.count >= 3 && t.allSatisfy { $0 == "─" }
              }),
              bare(lines[(sep + 1)..<foot].joined()) == "\(n + 2).Chataboutthis",
              let head = lines[..<sep].lastIndex(where: { l in
                  let t = l.trimmingCharacters(in: .whitespaces); return t.hasPrefix("☐") || t.hasPrefix("☒") || t.hasPrefix("←")
              }),
              claudeBar(q, lines[head]) else { return false }
        // The text between the bar and the separator, whitespace dropped, with where each line starts. Structure is
        // read from lines, never from the dropped-whitespace text: a row is a line of its own in Claude's
        // two-column focus slot ("❯ " or "  ", then "N."), Submit a line of its own; a mark or a number anywhere
        // else is text. Every row-like line must be one of the rows read (free text cannot hide or add a row).
        let region = Array(lines[(head + 1)..<sep])
        var chars: [Character] = [], starts: [Int: Int] = [:]
        for (i, l) in region.enumerated() {
            let b = bare(l)
            if !b.isEmpty { starts[chars.count] = i; chars += Array(b) }
        }
        var pos = 0, rowLines: Set<Int> = []
        func take(_ lit: String) -> Bool {
            let a = Array(lit)
            guard pos + a.count <= chars.count, Array(chars[pos..<(pos + a.count)]) == a else { return false }
            pos += a.count; return true
        }
        func has(_ lit: String) -> Bool { let a = Array(lit); return pos + a.count <= chars.count && Array(chars[pos..<(pos + a.count)]) == a }
        let rowRe = /^(❯ |  )(\d+)\.(?: |$)/, submitRe = /^(❯|\s)\s*Submit\s*$/
        /// A row line starts here: its mark (taken) or nil when it is not row `number`'s line. `lineEnd`: where
        /// that line ends (the label must end there: the label/description boundary is a line break, never dropped).
        var lineEnd = 0
        func row(_ number: Int) -> Bool? {
            guard let l = starts[pos], let m = region[l].firstMatch(of: rowRe), Int(m.2) == number else { return nil }
            rowLines.insert(l)
            lineEnd = pos + bare(region[l]).count
            let marked = m.1 == "❯ "
            if marked { guard take("❯") else { return nil } }
            guard take("\(number).") else { return nil }
            return marked
        }
        guard take(bare(want.text)) else { return false }
        var marks: [Int] = [], checked: Set<Int> = []
        func box(_ i: Int) -> Bool {
            guard want.multiSelect else { return true }
            if take("[✔]") { checked.insert(i); return true }
            return take("[]")
        }
        for (i, o) in want.options.enumerated() {
            guard let marked = row(i + 1), box(i) else { return false }
            if marked { marks.append(i) }
            let fits = shownLabels(o).filter { has($0) }
            // The label is the rest of the row line, the description starts a line (a wrapped label refuses).
            guard fits.count == 1, take(fits[0]), pos == lineEnd, take(bare(o.detail)) else { return false }
        }
        guard let otherMarked = row(n + 1), box(n) else { return false }
        if otherMarked { marks.append(n) }
        let empty = want.multiSelect ? "Typesomething" : "Typesomething."
        guard take(other.map { $0.isEmpty ? empty : bare($0) } ?? empty) else { return false }
        if want.multiSelect {
            guard let l = starts[pos], region[l].wholeMatch(of: submitRe) != nil else { return false }
            rowLines.insert(l)
            if take("❯") { marks.append(n + 1) }
            guard take("Submit") else { return false }
        }
        guard pos == chars.count, marks.count == 1, let mark = marks.first else { return false }
        // No other line looks like a row, Submit or a focus mark.
        for (i, l) in region.enumerated() where !rowLines.contains(i) {
            if l.firstMatch(of: rowRe) != nil || l.wholeMatch(of: submitRe) != nil || l.trimmingCharacters(in: .whitespaces).hasPrefix("❯") { return false }
        }
        if let checks, want.multiSelect, checks != checked { return false }
        // `.auto` as in `shows`: no hub text -> not on the Other row; "" -> on it (multiSelect: anywhere, Down moves
        // it there); text -> on it.
        let at: FieldFocus = focus != .auto ? focus : other == nil ? .auto : (other == "" && want.multiSelect) ? .any : .other
        switch at {
        case .auto: return mark < n
        case .any: return true
        case .other: return mark == n
        case .submit: return mark == n + 1
        }
    }

    /// The answer as Claude's review shows it: the picked labels in row order, then the Other text, ", " between.
    static func reviewParts(_ question: QuestionRequest.Question, _ a: QuestionAnswer) -> [[String]] {
        a.picks.sorted().map { shownLabels(question.options[$0]) } + [otherText(a)].compactMap { $0 }.map { [bare($0)] }
    }

    /// Claude's review screen of the hooked request `q` with the hub's `answers`, exactly: the tab bar, "Review your
    /// answers", per question "● text" and "→ answer", "Ready to submit your answers?", then "1. Submit answers" and
    /// "2. Cancel" with one focus mark. The last key ("1") goes only here.
    static func claudeReviewExact(_ q: QuestionRequest, answers: [QuestionAnswer], screen: String) -> Bool {
        guard q.agent == "claude", !q.toolUseId.hasPrefix("screen-"), answers.count == q.questions.count, !reviewAmbiguous(q, answers: answers),
              zip(q.questions, answers).allSatisfy({ k, a in a.picks.allSatisfy { k.options.indices.contains($0) } }) else { return false }
        let lines = screen.components(separatedBy: "\n")
        guard let title = lines.lastIndex(where: { $0.trimmingCharacters(in: .whitespaces) == "Review your answers" }),
              let head = lines[..<title].lastIndex(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }),
              claudeBar(q, lines[head]) else { return false }
        // Whitespace dropped, with where each line starts: "●", "→", "Ready…" and the two rows must each start a
        // line (a mark inside a label is text).
        var chars: [Character] = [], starts: Set<Int> = []
        for l in lines[(title + 1)...] {
            let b = bare(l)
            if !b.isEmpty { starts.insert(chars.count); chars += Array(b) }
        }
        var pos = 0
        func has(_ lit: String) -> Bool { let a = Array(lit); return pos + a.count <= chars.count && Array(chars[pos..<(pos + a.count)]) == a }
        func take(_ lit: String) -> Bool { guard has(lit) else { return false }; pos += lit.count; return true }
        func line(_ lit: String) -> Bool { starts.contains(pos) && take(lit) }
        for (k, a) in zip(q.questions, answers) {
            guard line("●"), take(bare(k.text)), line("→") else { return false }
            for (j, alts) in reviewParts(k, a).enumerated() {
                if j > 0 { guard take(",") else { return false } }
                let fits = alts.filter { has($0) }
                guard fits.count == 1, take(fits[0]) else { return false }
            }
        }
        guard line("Readytosubmityouranswers?") else { return false }
        var marks = 0
        let first = starts.contains(pos)
        if take("❯") { marks += 1 }
        guard first, take("1.Submitanswers") else { return false }
        let second = starts.contains(pos)
        if take("❯") { marks += 1 }
        guard second, take("2.Cancel"), pos == chars.count, marks == 1 else { return false }
        return true
    }

    /// The review screen cannot prove the hub's answers: a shown label with a comma or a review mark (● →), two
    /// labels alike once whitespace is dropped, a question text with a review mark, or Other text with a comma, a
    /// review mark or alike a label. Then two answer sets look the same there, so the last key would go blind; the
    /// answer is refused before any key. The comma counts only in a multi-choice question: a single-choice one's
    /// review line shows one answer (a label or the Other text), never picks joined by commas.
    /// The refusal `answerQuestion` makes before any key when the review screen cannot prove the answers. A card
    /// shows it before Send.
    static func reviewBlocked(_ q: QuestionRequest, answers: [QuestionAnswer]) -> Bool {
        let hooked = q.agent == "claude" && !q.toolUseId.hasPrefix("screen-")
        return q.delivery == .picker && hooked && (q.questions.count > 1 || q.questions.last?.multiSelect == true)
            && reviewAmbiguous(q, answers: answers)
    }

    /// What a card says before Send when `reviewBlocked` would refuse: the labels alone, or the person's own answer.
    /// Nil while questions are unread: the answer reads them first, and the rule needs them all.
    public static let reviewLabels = "The terminal's review screen could not show which options were picked (an option with a comma or a ● or → mark, or two options alike). Answer in the terminal."
    public static let reviewOwn = "Your own answer has a comma, a ● or → mark, or is the same as an option, so the terminal's review screen could not show it. Change it, or answer in the terminal."
    public static func reviewNote(_ q: QuestionRequest, answers: [QuestionAnswer]) -> String? {
        if q.unread > 0 { return nil }
        if reviewBlocked(q, answers: []) { return reviewLabels }
        return reviewBlocked(q, answers: answers) ? reviewOwn : nil
    }

    /// Why `reviewBlocked` refuses (`Failure.unsupported`).
    static let reviewBlockedWhy = "the review screen could not show which options were picked (a label with a comma or a review mark, or two alike)"

    static func reviewAmbiguous(_ q: QuestionRequest, answers: [QuestionAnswer]) -> Bool {
        let marks: (String) -> Bool = { $0.contains("●") || $0.contains("→") }
        for (k, question) in q.questions.enumerated() {
            let comma: (String) -> Bool = { question.multiSelect && $0.contains(",") }
            let alts = question.options.map { Set(shownLabels($0)) }
            let all = alts.flatMap { $0 }
            if marks(question.text) || all.contains(where: { comma($0) || marks($0) }) || Set(all).count != all.count { return true }
            if answers.indices.contains(k), let o = otherText(answers[k]).map(bare), comma(o) || marks(o) || all.contains(o) { return true }
        }
        return false
    }
}
