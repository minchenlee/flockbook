import Foundation

/// The chat's "Since you left" card: what came into a conversation while it was not on screen. Counted from the rows
/// the app already has and from `Glance`; no model runs for it. The Swift side of the mod's since.js
/// (herdr-app/mods/flockbook/since.js): the same texts, threshold and counts, and the same cases in
/// `Tests/HerdrKitTests/Fixtures/since-vectors.json`.
public enum SinceLeft {
    /// A shorter absence shows no card (ms).
    public static let awayMs: Int64 = 5 * 60_000

    public enum Texts {
        public static let title = "Since you left"
        public static let summaryMark = "Summary · written by a model, may be wrong"
        public static let needsPrefix = "Needs you: "
        public static let fromLog = "From the log"
        public static let fromLogWithSummary = "From the log, not from the summary"
        public static let jump = "Jump to that message"
    }

    /// What the owner saw of a conversation: the turn's newest item when it was last on screen (`id`), when (`at`,
    /// ms), and when it was on screen before that id came (`since`).
    public struct Seen: Codable, Sendable, Hashable {
        public var id: String
        public var at: Int64
        public var since: Int64?
        public init(id: String, at: Int64, since: Int64? = nil) { self.id = id; self.at = at; self.since = since }
    }

    /// The time away: from the last time the conversation was on screen to now (ms).
    public struct Away: Sendable, Hashable {
        public var from: Int64
        public var back: Int64
        public init(from: Int64, back: Int64) { self.from = from; self.back = back }
    }

    /// The owner's last message, for the card's line and its Jump button.
    public struct Said: Sendable, Hashable {
        public var id: String
        /// "14:02", or "" when the message has no time.
        public var at: String
        /// Its first line with text.
        public var text: String
    }

    /// What the card says.
    public struct Card: Sendable, Hashable {
        public var away: String
        public var messages: Int
        public var words: Int
        /// The line from the log (`Glance.line`), or "".
        public var facts: String
        public var said: Said?
        public init(away: String, messages: Int, words: Int, facts: String = "", said: Said? = nil) {
            self.away = away; self.messages = messages; self.words = words; self.facts = facts; self.said = said
        }
    }

    /// JavaScript's Math.round, for the same numbers as the mod.
    static func round(_ x: Double) -> Double { (x + 0.5).rounded(.down) }

    /// "47 minutes", "3 hours", "2 days".
    public static func span(_ ms: Int64) -> String {
        let m = max(1, Int(round(Double(ms) / 60000)))
        if m < 60 { return "\(m) \(m == 1 ? "minute" : "minutes")" }
        let h = Int(round(Double(m) / 60))
        if h < 24 { return "\(h) \(h == 1 ? "hour" : "hours")" }
        let d = Int(round(Double(h) / 24))
        return "\(d) \(d == 1 ? "day" : "days")"
    }

    static func cjk(_ s: Unicode.Scalar) -> Bool {
        (0x3040...0x30FF).contains(s.value) || (0x3400...0x9FFF).contains(s.value) || (0xAC00...0xD7AF).contains(s.value)
    }

    /// Words in a text: runs of letters between spaces, and each CJK character as one.
    public static func words(_ text: String) -> Int {
        var n = 0, rest = String.UnicodeScalarView()
        for s in text.unicodeScalars {
            if cjk(s) { n += 1; rest.append(" ") } else { rest.append(s) }
        }
        let wordish: (Unicode.Scalar) -> Bool = {
            switch $0.properties.generalCategory {
            case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter,
                 .decimalNumber, .letterNumber, .otherNumber: true
            default: false
            }
        }
        var token = false
        for s in rest {
            if s.properties.isWhitespace { if token { n += 1 }; token = false }
            else if wordish(s) { token = true }
        }
        if token { n += 1 }
        return n
    }

    /// 1234567 as "1,234,567" (en-US, whatever the Mac's own format).
    public static func grouped(_ n: Int) -> String {
        let s = String(abs(n))
        var out = ""
        for (i, c) in s.enumerated() {
            if i > 0 && (s.count - i) % 3 == 0 { out.append(",") }
            out.append(c)
        }
        return (n < 0 ? "-" : "") + out
    }

    /// "about 1,900": two digits kept, so it does not read as an exact count.
    public static func about(_ n: Int) -> String {
        if n < 100 { let r = Int(round(Double(n) / 10)) * 10; return String(r == 0 ? n : r) }
        let p = Int(pow(10, Double(String(n).count - 2)))
        return grouped(Int(round(Double(n) / Double(p))) * p)
    }

    /// "14:02" on this computer's clock.
    public static func clock(_ ms: Int64, timeZone: TimeZone = .current) -> String {
        var cal = Calendar(identifier: .gregorian)
        cal.timeZone = timeZone
        let c = cal.dateComponents([.hour, .minute], from: Date(timeIntervalSince1970: Double(ms) / 1000))
        return String(format: "%02d:%02d", c.hour ?? 0, c.minute ?? 0)
    }

    /// The owner came back to this conversation. `seen`: the mark from before, `id`: the turn's newest item now
    /// (`Glance.Facts.id`), `now`: ms. Nil when it was on screen less than `awayMs` ago, or nothing came.
    public static func arrive(seen: Seen?, id: String?, now: Int64) -> Away? {
        guard let seen, seen.at != 0, let id, seen.id != id, now - seen.at >= awayMs else { return nil }
        return Away(from: seen.at, back: now)
    }

    /// A user message the owner wrote (not a note from another agent).
    static func own(_ text: String) -> Bool { ConversationParser.hcNote(text) == nil }

    /// What the card says. `items`: the conversation's rows; `away`: from `arrive`; `facts`: the turn now.
    public static func card(_ items: [ConversationItem], away: Away, facts: Glance.Facts?, timeZone: TimeZone = .current) -> Card {
        var messages = 0, words = 0
        var said: ConversationItem?
        for it in items {
            switch it.kind {
            case .assistantText(let t):
                if let ts = Glance.ms(it.timestamp), ts > away.from, ts <= away.back { messages += 1; words += Self.words(t) }
            case .userText(let t) where own(t): said = it
            default: break
            }
        }
        var line: Said?
        if let said, case .userText(let t) = said.kind,
           let first = t.split(separator: "\n", omittingEmptySubsequences: false).map({ $0.trimmingCharacters(in: .whitespaces) }).first(where: { !$0.isEmpty }) {
            line = Said(id: said.id, at: Glance.ms(said.timestamp).map { clock($0, timeZone: timeZone) } ?? "", text: first)
        }
        return Card(away: span(away.back - away.from), messages: messages, words: words, facts: Glance.line(facts), said: line)
    }

    /// The line of counts: "You were away 47 minutes · 6 new messages · about 1,900 words".
    public static func countLine(_ s: Card) -> String {
        var out = ["You were away \(s.away)"]
        if s.messages > 0 { out += ["\(s.messages) new \(s.messages == 1 ? "message" : "messages")", "about \(about(s.words)) words"] }
        else { out.append("no new message in the rows loaded") }
        return out.joined(separator: " · ")
    }

    /// The line under the summary (or alone): "From the log: …". "" when the log shows nothing.
    public static func factsLine(_ s: Card, summary: Bool) -> String {
        s.facts.isEmpty ? "" : "\(summary ? Texts.fromLogWithSummary : Texts.fromLog): \(s.facts)"
    }

    /// "You said at 14:02: Fix the importer".
    public static func saidLine(_ s: Said) -> String { "You said\(s.at.isEmpty ? "" : " at \(s.at)"): \(s.text)" }
}

extension SinceLeft {
    /// A question, stop or approval card holds the reply box's place, or an approval card stands in the conversation:
    /// the Since you left card waits, so the two never take more than the pane's height.
    public static func waits(question: Bool, stopCard: Bool, composerCard: Bool, conversationCard: Bool) -> Bool {
        question || stopCard || composerCard || conversationCard
    }

    /// The way in to a conversation, as the mod's chat (`arriving`): the card is decided on a read of the log made
    /// after the conversation came on screen, never on what was shown before. Until that read: no card and no mark.
    public struct Arrival: Sendable, Hashable {
        /// The turn the card was decided on; nil until then.
        public private(set) var decided: String?
        /// The turn the app's copy of the log showed then, when it was older: nothing is marked until the copy moves on.
        private var behind: String?
        /// Decided while the app's copy showed another turn, or none (`behind` nil): nothing is marked until it moves on.
        private var waiting = false
        /// The last turn the app's copy showed (nil: none yet): a copy that shows nothing keeps it, never a turn not shown.
        private var lastShown: String?
        public init() {}

        /// `read`: the newest turn in a read of the log made now (nil: no log, or no turn yet: wait for the next read).
        /// `shown`: the turn the app's copy shows now. Once: the time away, if any.
        public mutating func first(_ read: String?, shown: String?, seen: Seen?, now: Int64) -> (decided: Bool, away: Away?) {
            guard decided == nil else { return (true, nil) }
            guard let read else { return (false, nil) }
            decided = read
            waiting = shown != read
            behind = shown
            lastShown = shown
            return (true, SinceLeft.arrive(seen: seen, id: read, now: now))
        }

        /// A mirrored log's finished copies when it was first asked for (nil: not yet, or no mirror).
        private var syncsAtStart: Int?

        /// Whether a read of the log now may decide: a log on this Mac (`syncs` nil) at once; a mirror (an agent on
        /// another machine, `MirrorSyncs`) only after a copy that finished after the first ask, so a copy that is
        /// behind does not use the one decision.
        public mutating func fresh(syncs: Int?) -> Bool {
            guard let syncs else { return true }
            guard let base = syncsAtStart else { syncsAtStart = syncs; return false }
            // Fewer than then: the mirror was made again (it counts from 0): wait for its own next copy.
            if syncs < base { syncsAtStart = syncs; return false }
            return syncs > base
        }

        /// What to mark as seen when the app's copy shows the turn `shown`; nil: nothing yet.
        public mutating func mark(_ shown: String?) -> String? {
            guard let decided else { return nil }
            // The app's copy still shows the older turn: the newer one is not marked until it shows (left before that,
            // the card comes again next time).
            if waiting {
                if shown == nil || shown == behind { return nil }
                waiting = false
            }
            if let shown { lastShown = shown }
            return lastShown ?? decided
        }
    }
}
