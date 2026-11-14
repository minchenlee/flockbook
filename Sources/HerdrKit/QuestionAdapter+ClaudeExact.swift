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

    static let quoteMin = 80
    public static let maxReviewOptions = 8

    /// Why `reviewCollision` blocked a pre-flight question card / answer set.
    public enum CollisionKind: Sendable, Equatable {
        case labels, picks, own
        /// A screen read whose option labels may be cut (a wrapped label): the review cannot be checked.
        case cut
    }

    private enum ReviewForm: Equatable {
        case quoted, unquoted
    }

    private struct ReviewCand: Equatable {
        let picks: [Int]
        let other: String?
    }

    private struct InterState: Hashable {
        let i1, a1, o1, i2, a2, o2: Int
    }

    /// Terminal display width of `s`: combining/control scalars = 0, East Asian Wide/Fullwidth/emoji = 2, others = 1.
    public static func displayWidth(_ s: String) -> Int {
        s.unicodeScalars.reduce(0) { $0 + charWidth($1) }
    }

    private static func charWidth(_ c: UnicodeScalar) -> Int {
        if CharacterSet.controlCharacters.contains(c) { return 0 }
        switch c.value {
        case 0x0300...0x036F, 0x1160...0x11FF, 0x1AB0...0x1AFF, 0x1DC0...0x1DFF,
             0x20D0...0x20FF, 0xFE00...0xFE0F, 0xFE20...0xFE2F, 0xE0100...0xE01EF:
            return 0
        case 0x1100...0x115F, 0x231A...0x231B, 0x2329...0x232A, 0x23E9...0x23EC,
             0x23F0, 0x23F3, 0x25FD...0x25FE, 0x2614...0x2615, 0x2648...0x2653,
             0x267F, 0x2693, 0x26A1, 0x26AA...0x26AB, 0x26BD...0x26BE,
             0x26C4...0x26C5, 0x26CE, 0x26D4, 0x26EA, 0x26F2...0x26F3, 0x26F5,
             0x26FA, 0x26FD, 0x2705, 0x270A...0x270B, 0x2728, 0x274C, 0x274E,
             0x2753...0x2755, 0x2757, 0x2795...0x2797, 0x27B0, 0x27BF,
             0x2B1B...0x2B1C, 0x2B50, 0x2B55, 0x2E80...0x303E, 0x3041...0x3247,
             0x3250...0x4DBF, 0x4E00...0xA4C6, 0xA960...0xA97C, 0xAC00...0xD7A3,
             0xF900...0xFAFF, 0xFE10...0xFE19, 0xFE30...0xFE6B, 0xFF01...0xFF60,
             0xFFE0...0xFFE6, 0x16FE0...0x16FE4, 0x16FF0...0x16FF1, 0x17000...0x18CD5,
             0x18D00...0x18D08, 0x1AFF0...0x1B2FB, 0x1F004, 0x1F0CF, 0x1F18E,
             0x1F191...0x1F19A, 0x1F1E6...0x1F1FF, 0x1F200...0x1F2FF, 0x1F300...0x1F64F,
             0x1F680...0x1F6FF, 0x1F900...0x1F9FF, 0x1FA70...0x1FAFF,
             0x20000...0x2FFFD, 0x30000...0x3FFFD:
            return 2
        default:
            return 1
        }
    }

    /// Claude Code 2.1.322 draws a long question (~80+ columns) as a block quote: "│ " in front of every line.
    /// When every non-empty line in `lines` starts with "│", strips that leading "│"; else leaves unchanged.
    static func dropQuoteBar(_ lines: ArraySlice<String>) -> [String] {
        let nonEmpty = lines.filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        guard !nonEmpty.isEmpty,
              nonEmpty.allSatisfy({ $0.trimmingCharacters(in: .whitespaces).hasPrefix("│") }) else {
            return Array(lines)
        }
        return lines.map { l in
            let t = l.trimmingCharacters(in: .whitespaces)
            return t.isEmpty ? l : String(t.dropFirst())
        }
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
        var region = Array(lines[(head + 1)..<sep])
        let rowRe = /^(❯ |  )(\d+)\.(?: |$)/, submitRe = /^(❯|\s)\s*Submit\s*$/
        let firstRow = region.firstIndex(where: { $0.firstMatch(of: rowRe) != nil }) ?? region.count
        if displayWidth(want.text) >= quoteMin {
            if want.text.trimmingCharacters(in: .whitespaces).hasPrefix("│") { return false }
            region.replaceSubrange(..<firstRow, with: dropQuoteBar(region[..<firstRow]))
        }
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

    private static func hasMarks(_ s: String) -> Bool {
        s.contains("●") || s.contains("→")
    }

    private static func pushUnique(_ out: inout [String], _ s: String) {
        if !out.contains(s) { out.append(s) }
    }

    private static func quoteIfComma(_ s: String, multi: Bool, form: ReviewForm) -> String {
        (multi && form == .quoted && s.contains(",")) ? "\"\(s)\"" : s
    }

    private static func candParts(_ question: QuestionRequest.Question, _ cand: ReviewCand, _ form: ReviewForm) -> [[[UInt8]]]? {
        var parts: [[[UInt8]]] = []
        parts.reserveCapacity(cand.picks.count + (cand.other != nil ? 1 : 0))
        for p in cand.picks {
            guard question.options.indices.contains(p) else { return nil }
            var alts: [String] = []
            for s in shownLabels(question.options[p]) {
                if s.isEmpty || s.contains("\"") || s.contains("│") { return nil }
                pushUnique(&alts, quoteIfComma(s, multi: question.multiSelect, form: form))
            }
            parts.append(alts.map { Array($0.utf8) })
        }
        if let o = cand.other {
            if o.isEmpty || o.contains("\"") || o.contains("│") { return nil }
            var alts = [o]
            if question.multiSelect && form == .quoted && o.contains(",") {
                pushUnique(&alts, "\"\(o)\"")
            }
            parts.append(alts.map { Array($0.utf8) })
        }
        return parts
    }

    private static func candVariants(_ question: QuestionRequest.Question, otherBare: String?) -> [ReviewCand] {
        let n = question.options.count
        guard n <= maxReviewOptions else { return [] }
        var out: [ReviewCand] = []
        if question.multiSelect {
            for mask in 1..<(1 << n) {
                let picks = (0..<n).filter { (mask & (1 << $0)) != 0 }
                out.append(ReviewCand(picks: picks, other: nil))
            }
            if let o = otherBare {
                for mask in 0..<(1 << n) {
                    let picks = (0..<n).filter { (mask & (1 << $0)) != 0 }
                    out.append(ReviewCand(picks: picks, other: o))
                }
            }
        } else {
            for p in 0..<n {
                out.append(ReviewCand(picks: [p], other: nil))
            }
            if let o = otherBare {
                out.append(ReviewCand(picks: [], other: o))
            }
        }
        return out
    }

    private static func canonicalCand(_ a: QuestionAnswer) -> ReviewCand? {
        var seen: Set<Int> = []
        let picks = a.picks.sorted().filter { seen.insert($0).inserted }
        let other = otherText(a).map(bare)
        if picks.isEmpty && other == nil { return nil }
        return ReviewCand(picks: picks, other: other)
    }

    private static func validateQuestionCaps(_ q: QuestionRequest.Question) -> Bool {
        let qt = bare(q.text)
        if q.cutLabels || q.options.count > maxReviewOptions || qt.isEmpty || qt.contains("│") { return false }
        return q.options.allSatisfy { o in
            let labels = shownLabels(o)
            return !labels.isEmpty && labels.allSatisfy { !$0.isEmpty && !$0.contains("\"") && !$0.contains("│") }
        }
    }

    private static func validateAnswerCaps(_ q: QuestionRequest.Question, _ a: QuestionAnswer) -> CollisionKind? {
        if a.picks.contains(where: { !q.options.indices.contains($0) }) { return .picks }
        let hasOther = otherText(a)
        if !q.multiSelect && a.picks.count + (hasOther != nil ? 1 : 0) > 1 { return .picks }
        if let o = hasOther {
            let b = bare(o)
            if b.isEmpty || b.contains("\"") || b.contains("│") { return .own }
        }
        return nil
    }

    /// Match `parts[0] , parts[1] , ...` starting at `tail`, returning all matched UTF-8 byte lengths.
    private static func matchPartsAt(_ tail: ArraySlice<UInt8>, _ parts: [[[UInt8]]]) -> [Int] {
        var offsets = [0]
        let comma = UInt8(ascii: ",")
        for (j, alts) in parts.enumerated() {
            if j > 0 {
                var next: [Int] = []
                for off in offsets {
                    let idx = tail.startIndex + off
                    if idx < tail.endIndex && tail[idx] == comma && !next.contains(off + 1) {
                        next.append(off + 1)
                    }
                }
                offsets = next
            }
            var next: [Int] = []
            for off in offsets {
                let idx = tail.startIndex + off
                guard idx <= tail.endIndex else { continue }
                let rest = tail[idx...]
                for alt in alts {
                    if rest.starts(with: alt) {
                        let end = off + alt.count
                        if !next.contains(end) { next.append(end) }
                    }
                }
            }
            if next.isEmpty { return [] }
            offsets = next
        }
        return offsets
    }

    private static func toTokens(_ parts: [[[UInt8]]], comma: [[UInt8]]) -> [[[UInt8]]] {
        var t: [[[UInt8]]] = []
        t.reserveCapacity(parts.count * 2)
        for (i, p) in parts.enumerated() {
            if i > 0 { t.append(comma) }
            t.append(p)
        }
        return t
    }

    /// Exact language intersection `L(p1) ∩ L(p2) != ∅` where items in each list are joined by `","`.
    private static func partsIntersect(_ p1: [[[UInt8]]], _ p2: [[[UInt8]]]) -> Bool {
        let comma: [[UInt8]] = [[UInt8(ascii: ",")]]
        let t1 = toTokens(p1, comma: comma)
        let t2 = toTokens(p2, comma: comma)
        guard let f1 = t1.first, let f2 = t2.first else { return false }
        var visited: Set<InterState> = []
        var stack: [InterState] = []
        for a1 in f1.indices {
            for a2 in f2.indices {
                stack.append(InterState(i1: 0, a1: a1, o1: 0, i2: 0, a2: a2, o2: 0))
            }
        }
        while let st = stack.popLast() {
            if !visited.insert(st).inserted { continue }
            let s1 = t1[st.i1][st.a1]
            let s2 = t2[st.i2][st.a2]
            let rem1 = s1[st.o1...]
            let rem2 = s2[st.o2...]
            let common = min(rem1.count, rem2.count)
            if !rem1.prefix(common).elementsEqual(rem2.prefix(common)) { continue }
            let done1 = common == rem1.count
            let done2 = common == rem2.count
            let ni1 = done1 ? st.i1 + 1 : st.i1
            let ni2 = done2 ? st.i2 + 1 : st.i2
            if ni1 == t1.count && ni2 == t2.count { return true }
            if ni1 == t1.count || ni2 == t2.count { continue }
            let a1Range = done1 ? t1[ni1].indices : (st.a1..<(st.a1 + 1))
            let a2Range = done2 ? t2[ni2].indices : (st.a2..<(st.a2 + 1))
            let no1 = done1 ? 0 : st.o1 + common
            let no2 = done2 ? 0 : st.o2 + common
            for na1 in a1Range {
                for na2 in a2Range {
                    stack.append(InterState(i1: ni1, a1: na1, o1: no1, i2: ni2, a2: na2, o2: no2))
                }
            }
        }
        return false
    }

    /// Line-start-anchored DP over `bodyBytes` for one `ReviewForm`. Returns `(chosenMatched, otherMatched)`.
    private static func dpForm(
        _ q: QuestionRequest,
        bodyBytes: [UInt8],
        bodyStarts: Set<Int>,
        candsPerQ: [[ReviewCand]],
        chosenIdx: [Int],
        form: ReviewForm
    ) -> (Bool, Bool) {
        let dotBytes = Array("●".utf8)
        let arrowBytes = Array("→".utf8)
        var chosenPos: Set<Int> = [0]
        var otherPos: Set<Int> = []

        for (k, cands) in candsPerQ.enumerated() {
            var nextChosen: Set<Int> = []
            var nextOther: Set<Int> = []
            let qtext = Array(bare(q.questions[k].text).utf8)
            let positions = Array(chosenPos.union(otherPos)).sorted()

            for pos in positions {
                let fromChosen = chosenPos.contains(pos)
                let fromOther = otherPos.contains(pos)
                if !bodyStarts.contains(pos) || !bodyBytes[pos...].starts(with: dotBytes) { continue }
                let afterDot = pos + dotBytes.count
                if !bodyBytes[afterDot...].starts(with: qtext) { continue }
                let arrowPos = afterDot + qtext.count
                if !bodyStarts.contains(arrowPos) || !bodyBytes[arrowPos...].starts(with: arrowBytes) { continue }
                let ansStart = arrowPos + arrowBytes.count
                let tail = bodyBytes[ansStart...]
                for (ci, cand) in cands.enumerated() {
                    guard let parts = candParts(q.questions[k], cand, form) else { continue }
                    for clen in matchPartsAt(tail, parts) {
                        let endPos = ansStart + clen
                        if ci == chosenIdx[k] && fromChosen { nextChosen.insert(endPos) }
                        if ci != chosenIdx[k] || fromOther { nextOther.insert(endPos) }
                    }
                }
            }
            chosenPos = nextChosen
            otherPos = nextOther
            if chosenPos.isEmpty && otherPos.isEmpty { return (false, false) }
        }
        return (chosenPos.contains(bodyBytes.count), otherPos.contains(bodyBytes.count))
    }

    private static func candsForQuestion(_ q: QuestionRequest.Question, _ a: QuestionAnswer?) -> ([ReviewCand], Int?)? {
        let chosen = a.flatMap(canonicalCand)
        let otherBare = chosen?.other
        let cands = candVariants(q, otherBare: otherBare)
        if let c = chosen {
            guard let ci = cands.firstIndex(of: c) else { return nil }
            return (cands, ci)
        }
        return (cands, nil)
    }

    private static func singleQuestionCollides(_ q: QuestionRequest.Question, _ a: QuestionAnswer?) -> CollisionKind? {
        guard let (cands, chosenIdx) = candsForQuestion(q, a) else { return nil }
        guard let ci = chosenIdx else {
            for (i, c1) in cands.enumerated() {
                guard let p1 = candParts(q, c1, .quoted) else { return nil }
                for (j, c2) in cands.enumerated() where j != i {
                    guard let p2q = candParts(q, c2, .quoted),
                          let p2u = candParts(q, c2, .unquoted) else { return nil }
                    if partsIntersect(p1, p2q) || partsIntersect(p1, p2u) { return .labels }
                }
            }
            return nil
        }
        let c1 = cands[ci]
        guard let p1 = candParts(q, c1, .quoted) else { return nil }
        var collides = false
        for (j, c2) in cands.enumerated() where j != ci {
            guard let p2q = candParts(q, c2, .quoted),
                  let p2u = candParts(q, c2, .unquoted) else { return nil }
            if partsIntersect(p1, p2q) || partsIntersect(p1, p2u) {
                collides = true
                break
            }
        }
        if !collides { return nil }
        if c1.other == nil { return .picks }
        if c1.picks.isEmpty { return .own }
        let picksOnly = ReviewCand(picks: c1.picks, other: nil)
        if let pOnlyQ = candParts(q, picksOnly, .quoted) {
            for c2 in candVariants(q, otherBare: nil) where c2.picks != c1.picks {
                if let p2q = candParts(q, c2, .quoted), let p2u = candParts(q, c2, .unquoted),
                   partsIntersect(pOnlyQ, p2q) || partsIntersect(pOnlyQ, p2u) {
                    return .picks
                }
            }
        }
        return .own
    }

    private static func rawBodyVariantsFor(_ q: QuestionRequest.Question, _ a: QuestionAnswer, cap: Int) -> [String]? {
        var itemAlts: [[String]] = []
        var seen: Set<Int> = []
        let picks = a.picks.sorted().filter { seen.insert($0).inserted }
        for p in picks {
            guard q.options.indices.contains(p) else { return nil }
            let opt = q.options[p]
            let alts: [String]
            if opt.recommended {
                alts = recommendedSuffixes.map { suf in
                    let s = opt.label + suf
                    return (q.multiSelect && s.contains(",")) ? "\"\(s)\"" : s
                }
            } else if q.multiSelect && opt.label.contains(",") {
                alts = ["\"\(opt.label)\""]
            } else {
                alts = [opt.label]
            }
            itemAlts.append(alts)
        }
        if let o = otherText(a) {
            var alts = [o]
            if q.multiSelect && o.contains(",") { pushUnique(&alts, "\"\(o)\"") }
            itemAlts.append(alts)
        }
        var acc = [""]
        for (j, alts) in itemAlts.enumerated() {
            var next: [String] = []
            for cur in acc {
                for alt in alts {
                    var s = cur
                    if j > 0 { s.append(",") }
                    s.append(contentsOf: alt)
                    if !next.contains(s) {
                        if next.count >= cap { return nil }
                        next.append(s)
                    }
                }
            }
            acc = next
        }
        return acc
    }

    private static func pushRenderedLines(_ raw: String, bodyBytes: inout [UInt8], bodyStarts: inout Set<Int>) {
        for l in raw.components(separatedBy: "\n") {
            let b = bare(l)
            if !b.isEmpty {
                bodyStarts.insert(bodyBytes.count)
                bodyBytes.append(contentsOf: b.utf8)
            }
        }
    }

    private static func dpCrossCollides(_ q: QuestionRequest, _ answers: [QuestionAnswer]) -> CollisionKind? {
        var candsPerQ: [[ReviewCand]] = []
        var chosenIdx: [Int] = []
        var perQBodies: [[String]] = []
        for (k, qq) in q.questions.enumerated() {
            guard let (cands, ci) = candsForQuestion(qq, answers[k]), let ci else { return .picks }
            candsPerQ.append(cands)
            chosenIdx.append(ci)
            guard let bodies = rawBodyVariantsFor(qq, answers[k], cap: 64), !bodies.isEmpty else { return .picks }
            perQBodies.append(bodies)
        }

        var tuples: [[Int]] = [[]]
        for bs in perQBodies {
            var next: [[Int]] = []
            for t in tuples {
                for i in bs.indices {
                    if next.count >= 256 { return .picks }
                    var nt = t
                    nt.append(i)
                    next.append(nt)
                }
            }
            tuples = next
        }

        for tuple in tuples {
            var bodyBytes: [UInt8] = []
            var bodyStarts: Set<Int> = []
            for (k, qq) in q.questions.enumerated() {
                pushRenderedLines("● \(qq.text)", bodyBytes: &bodyBytes, bodyStarts: &bodyStarts)
                pushRenderedLines("→ \(perQBodies[k][tuple[k]])", bodyBytes: &bodyBytes, bodyStarts: &bodyStarts)
            }
            let (_, otherQ) = dpForm(q, bodyBytes: bodyBytes, bodyStarts: bodyStarts, candsPerQ: candsPerQ, chosenIdx: chosenIdx, form: .quoted)
            let (_, otherU) = dpForm(q, bodyBytes: bodyBytes, bodyStarts: bodyStarts, candsPerQ: candsPerQ, chosenIdx: chosenIdx, form: .unquoted)
            if otherQ || otherU {
                let own = answers.compactMap(otherText).contains(where: hasMarks)
                    || zip(q.questions, answers).contains { qq, a in singleQuestionCollides(qq, a) == .own }
                return own ? .own : .picks
            }
        }
        return nil
    }

    /// Structured pre-flight collision check over both `.quoted` and `.unquoted`, supporting empty, partial, and
    /// complete `answers`. Returns `nil` iff every allowed body variant uniquely identifies the choice.
    public static func reviewCollision(_ q: QuestionRequest, answers: [QuestionAnswer]) -> CollisionKind? {
        guard q.agent == "claude",
              !q.questions.isEmpty,
              q.delivery == .picker,
              q.questions.count > 1 || q.questions.last?.multiSelect == true else { return nil }
        if q.questions.contains(where: \.cutLabels) { return .cut }
        for qq in q.questions {
            if !validateQuestionCaps(qq) { return .labels }
        }
        for (k, a) in answers.enumerated() {
            guard q.questions.indices.contains(k) else { break }
            if let kind = validateAnswerCaps(q.questions[k], a) { return kind }
        }

        let isAnswered: (Int) -> Bool = { k in
            answers.indices.contains(k) && (!answers[k].picks.isEmpty || otherText(answers[k]) != nil)
        }
        let anyUnanswered = q.questions.indices.contains { !isAnswered($0) }
        let hasCrossMarks = q.questions.count > 1 && q.questions.enumerated().contains { k, qq in
            hasMarks(qq.text)
                || qq.options.contains { hasMarks($0.label) }
                || (answers.indices.contains(k) ? otherText(answers[k]).map(hasMarks) ?? false : false)
        }

        if anyUnanswered {
            for (k, qq) in q.questions.enumerated() where isAnswered(k) {
                if let kind = singleQuestionCollides(qq, answers[k]) { return kind }
            }
            if hasCrossMarks {
                let ownMarks = answers.compactMap(otherText).contains(where: hasMarks)
                let labelMarks = q.questions.contains { hasMarks($0.text) || $0.options.contains { hasMarks($0.label) } }
                if labelMarks { return .labels }
                if ownMarks { return .own }
            }
            for (k, qq) in q.questions.enumerated() where !isAnswered(k) {
                if let kind = singleQuestionCollides(qq, nil) { return kind }
            }
            return nil
        }

        if !hasCrossMarks {
            for (k, qq) in q.questions.enumerated() {
                if let kind = singleQuestionCollides(qq, answers[k]) { return kind }
            }
            return nil
        }

        return dpCrossCollides(q, answers)
    }

    /// Claude's review screen of `q` with `answers`, enforcing line-start anchors and unique joint solution
    /// `S_valid = Solutions(Quoted) ∪ Solutions(Unquoted) == {A_chosen}`.
    static func claudeReviewExact(_ q: QuestionRequest, answers: [QuestionAnswer], screen: String) -> Bool {
        guard q.agent == "claude",
              !q.questions.isEmpty,
              answers.count == q.questions.count,
              q.questions.allSatisfy(validateQuestionCaps),
              zip(q.questions, answers).allSatisfy({ k, a in
                  (!a.picks.isEmpty || otherText(a) != nil) && validateAnswerCaps(k, a) == nil
              }) else { return false }
        let lines = screen.components(separatedBy: "\n")
        guard let title = lines.lastIndex(where: { $0.trimmingCharacters(in: .whitespaces) == "Review your answers" }),
              let head = lines[..<title].lastIndex(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty }),
              claudeBar(q, lines[head]),
              let ready = lines.lastIndex(where: { $0.trimmingCharacters(in: .whitespaces) == "Ready to submit your answers?" }),
              ready > title else { return false }

        // Strip quote bars ONLY inside confirmed quote-barred question-text sections ("│ ● <q.text>" up to the
        // immediately following "→" line), never inside "→" answer continuations such as a wrapped "│●B".
        let bodyLines = lines[(title + 1)..<ready]
        var bodyBytes: [UInt8] = []
        var bodyStarts: Set<Int> = []
        var i = bodyLines.startIndex
        while i < bodyLines.endIndex {
            let t = bodyLines[i].trimmingCharacters(in: .whitespaces)
            if t.hasPrefix("│") {
                let firstRest = String(t.dropFirst())
                if firstRest.trimmingCharacters(in: .whitespaces).hasPrefix("●") {
                    var j = i
                    var strippedLines: [String] = []
                    var strippedBare = ""
                    // A "→" line still goes on a long question whose text is not whole yet.
                    let goesOn = { (soFar: String, more: String) -> Bool in
                        q.questions.contains { qq in
                            let want = "●" + bare(qq.text)
                            return displayWidth(qq.text) >= quoteMin && soFar != want && want.hasPrefix(soFar + more)
                        }
                    }
                    while j < bodyLines.endIndex {
                        let tj = bodyLines[j].trimmingCharacters(in: .whitespaces)
                        guard tj.hasPrefix("│") else { break }
                        let rj = String(tj.dropFirst())
                        if rj.trimmingCharacters(in: .whitespaces).hasPrefix("→") && !goesOn(strippedBare, bare(rj)) { break }
                        strippedBare.append(bare(rj))
                        strippedLines.append(rj)
                        j = bodyLines.index(after: j)
                    }
                    let nextIsArrow = bodyLines[j...]
                        .first(where: { !$0.trimmingCharacters(in: .whitespaces).isEmpty })?
                        .trimmingCharacters(in: .whitespaces).hasPrefix("→") == true
                    let isQuotedQuestion = nextIsArrow && q.questions.contains { qq in
                        displayWidth(qq.text) >= quoteMin
                            && !qq.text.trimmingCharacters(in: .whitespaces).hasPrefix("│")
                            && strippedBare == "●" + bare(qq.text)
                    }
                    if isQuotedQuestion {
                        for rj in strippedLines {
                            let b = bare(rj)
                            if !b.isEmpty {
                                bodyStarts.insert(bodyBytes.count)
                                bodyBytes.append(contentsOf: b.utf8)
                            }
                        }
                        i = j
                        continue
                    }
                }
            }
            let b = bare(bodyLines[i])
            if !b.isEmpty {
                bodyStarts.insert(bodyBytes.count)
                bodyBytes.append(contentsOf: b.utf8)
            }
            i = bodyLines.index(after: i)
        }

        var fchars: [Character] = [], fstarts: Set<Int> = []
        for l in lines[ready...] {
            let b = bare(l)
            if !b.isEmpty { fstarts.insert(fchars.count); fchars += Array(b) }
        }

        var candsPerQ: [[ReviewCand]] = []
        var chosenIdx: [Int] = []
        for (k, a) in zip(q.questions, answers) {
            guard let (cands, ci) = candsForQuestion(k, a), let ci else { return false }
            candsPerQ.append(cands)
            chosenIdx.append(ci)
        }

        let (chosenQ, otherQ) = dpForm(q, bodyBytes: bodyBytes, bodyStarts: bodyStarts, candsPerQ: candsPerQ, chosenIdx: chosenIdx, form: .quoted)
        let (chosenU, otherU) = dpForm(q, bodyBytes: bodyBytes, bodyStarts: bodyStarts, candsPerQ: candsPerQ, chosenIdx: chosenIdx, form: .unquoted)
        guard (chosenQ || chosenU) && !otherQ && !otherU else { return false }

        var pos = 0
        func has(_ lit: String) -> Bool {
            let a = Array(lit)
            return pos + a.count <= fchars.count && Array(fchars[pos..<(pos + a.count)]) == a
        }
        func take(_ lit: String) -> Bool { guard has(lit) else { return false }; pos += lit.count; return true }
        guard fstarts.contains(pos), take("Readytosubmityouranswers?") else { return false }
        var marks = 0
        let first = fstarts.contains(pos)
        if take("❯") { marks += 1 }
        guard first, take("1.Submitanswers") else { return false }
        let second = fstarts.contains(pos)
        if take("❯") { marks += 1 }
        return second && take("2.Cancel") && pos == fchars.count && marks == 1
    }

    /// The refusal `answerQuestion` makes before any key when pre-flight review checks find a collision
    /// (`reviewCollision`). A card shows it before Send.
    static func reviewBlocked(_ q: QuestionRequest, answers: [QuestionAnswer]) -> Bool {
        reviewCollision(q, answers: answers) != nil
    }

    /// What a card says before Send when `reviewBlocked` would refuse: the labels alone, a colliding selection,
    /// or the person's own answer. Nil while questions are unread: the answer reads them first.
    public static let reviewLabels = "Two answers here would look identical on the review screen. Answer in the terminal."
    public static let reviewPicks = "This selection looks the same as another choice on the review screen. Change a selection, or answer in the terminal."
    public static let reviewOwn = "Your own answer would look the same as another choice on the review screen. Change it, or answer in the terminal."
    public static let reviewCut = "An option is too long for one line on the screen, so the review screen cannot be checked. Answer in the terminal."
    public static func reviewNote(_ q: QuestionRequest, answers: [QuestionAnswer]) -> String? {
        if q.unread > 0 { return nil }
        switch reviewCollision(q, answers: answers) {
        case .none: return nil
        case .labels: return reviewLabels
        case .picks: return reviewPicks
        case .own: return reviewOwn
        case .cut: return reviewCut
        }
    }

    /// Why `reviewBlocked` refuses (`Failure.unsupported`).
    static let reviewBlockedWhy = "two distinct answers would look identical on the review screen"

    static func reviewAmbiguous(_ q: QuestionRequest, answers: [QuestionAnswer]) -> Bool {
        reviewCollision(q, answers: answers) != nil
    }
}
