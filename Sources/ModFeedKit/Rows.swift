import Foundation
import HerdrKit

/// What the herdr-app mod draws, one row each: the conversation items, plus time separators and one footer per agent
/// turn, as the app's conversation view makes them (`ConversationView.entries`). Encoded as JSON for the mod.
public struct FeedRow: Encodable, Sendable, Equatable {
    /// Always "row" (the JSON Lines message type).
    public var type = "row"
    /// "user", "assistant", "tools", "perm", "question", "status", "sep", "footer".
    public var k: String
    /// Stable across reads of the same log: the mod keys rows (and their measured heights) by it.
    public var id: String
    public var text: String?
    /// A user row that is a note from another agent (`hc send`): its sender as hc wrote it ("t2 docs-worker"), and
    /// `text` without the `<hc>` tag.
    public var from: String?
    /// Milliseconds since 1970.
    public var ts: Double?
    /// user: how many pictures the owner attached.
    public var images: Int?
    public var chips: [FeedChip]?
    public var perm: FeedPerm?
    public var question: FeedQuestion?
}

public struct FeedChip: Encodable, Sendable, Equatable {
    public var symbol: String
    public var text: String
    public var calls: [FeedCall]
}

public struct FeedCall: Encodable, Sendable, Equatable {
    public var name: String
    public var summary: String
    public var done: Bool
    public var error: Bool
    /// The start of the result (`FeedRows.outputCap` characters), nil until it arrives.
    public var output: String?
}

public struct FeedPerm: Encodable, Sendable, Equatable {
    public var title: String
    public var command: String
    public var tool: String
    public var toolUseId: String
    /// "pending" or "resolved".
    public var state: String
}

public struct FeedQuestion: Encodable, Sendable, Equatable {
    public struct Option: Encodable, Sendable, Equatable { public var label: String; public var detail: String; public var recommended: Bool }
    public struct Question: Encodable, Sendable, Equatable {
        public var id: String; public var header: String; public var text: String; public var multi: Bool; public var options: [Option]
    }
    public var toolUseId: String
    public var agent: String
    /// "picker" (keys drive the agent's picker) or "message" (the answer is a normal message).
    public var delivery: String
    /// "pending", "answered", "declined".
    public var state: String
    public var answers: [String]?
    public var questions: [Question]
}

public enum FeedRows {
    /// Longer text is cut (a trailing "…" marks the cut). The mod has 8 MiB per `exec` and 50 ms per render.
    public static let textCap = 20_000
    public static let outputCap = 400
    /// A gap this long between rows (or a new day) gets a time separator.
    public static let separatorGap: TimeInterval = 900

    static func cut(_ s: String, _ n: Int) -> String { s.count > n ? String(s.prefix(n)) + "…" : s }
    static func ms(_ d: Date?) -> Double? { d.map { ($0.timeIntervalSince1970 * 1000).rounded() } }

    /// Adjacent tool items become one row (`ConversationView.displayRows`), keyed by the first one's id.
    public static func merged(_ items: [ConversationItem]) -> [ConversationItem] {
        var out: [ConversationItem] = [], run: [ConversationItem] = []
        func flush() {
            guard let first = run.first else { return }
            defer { run = [] }
            if run.count == 1 { out.append(first); return }
            let chips = run.flatMap { r -> [ToolChip] in if case .toolChips(let c) = r.kind { c } else { [] } }
            out.append(ConversationItem(id: first.id, kind: .toolChips(chips), timestamp: first.timestamp))
        }
        for it in items { if case .toolChips = it.kind { run.append(it) } else { flush(); out.append(it) } }
        flush()
        return out
    }

    public static func row(_ it: ConversationItem) -> FeedRow {
        var r = FeedRow(k: "status", id: it.id, ts: ms(it.timestamp))
        switch it.kind {
        case .userText(let s):
            r.k = "user"
            if let note = ConversationParser.hcNote(s) { r.text = cut(note.text, textCap); r.from = note.from } else { r.text = cut(s, textCap) }
            if !it.images.isEmpty { r.images = it.images.count }
        case .assistantText(let s):
            r.k = "assistant"; r.text = cut(s, textCap)
        case .status(let s):
            r.text = s
        case .toolChips(let chips):
            r.k = "tools"
            r.chips = chips.map { c in
                FeedChip(symbol: c.symbol, text: c.text, calls: c.calls.map {
                    FeedCall(name: $0.toolName, summary: cut($0.summary, 300), done: $0.output != nil, error: $0.isError,
                             output: $0.output.map { cut($0, outputCap) })
                })
            }
        case .permissionRequest(let p):
            r.k = "perm"; r.text = p.title
            r.perm = FeedPerm(title: p.title, command: cut(p.command, textCap), tool: p.toolName, toolUseId: p.toolUseId, state: p.state.rawValue)
        case .question(let q):
            r.k = "question"
            var state = "pending", answers: [String]? = nil
            switch q.state {
            case .pending: break
            case .answered(let a): state = "answered"; answers = a
            case .declined: state = "declined"
            }
            r.question = FeedQuestion(
                toolUseId: q.toolUseId, agent: q.agent, delivery: q.delivery == .picker ? "picker" : "message", state: state, answers: answers,
                questions: q.questions.map { x in
                    .init(id: x.id, header: x.header, text: x.text, multi: x.multiSelect,
                          options: x.options.map { .init(label: $0.label, detail: $0.detail, recommended: $0.recommended) })
                })
        }
        return r
    }

    /// Rows with time separators and turn footers ("01:41 · Worked 36s · 3 tools"), as `ConversationView.entries`.
    public static func rows(_ items: [ConversationItem], now: Date = Date(), calendar: Calendar = .current) -> [FeedRow] {
        var out: [FeedRow] = []
        var prevTime: Date?, prevRow: ConversationItem?
        var turnStart: Date?, turnTools = 0, turnHasReply = false, turnEnd: Date?
        func closeTurn() {
            guard turnHasReply, let end = turnEnd, let last = prevRow else { return }
            var parts = [clock(end, calendar)]
            if let s = turnStart, end.timeIntervalSince(s) >= 1 { parts.append("Worked \(duration(end.timeIntervalSince(s)))") }
            if turnTools > 0 { parts.append("\(turnTools) tool\(turnTools == 1 ? "" : "s")") }
            out.append(FeedRow(k: "footer", id: "f-\(last.id)", text: parts.joined(separator: " · ")))
        }
        for it in merged(items) {
            if case .userText = it.kind { closeTurn(); turnStart = it.timestamp; turnTools = 0; turnHasReply = false; turnEnd = nil }
            if let t = it.timestamp {
                if let p = prevTime, t.timeIntervalSince(p) >= separatorGap || !calendar.isDate(t, inSameDayAs: p) {
                    out.append(FeedRow(k: "sep", id: "s-\(it.id)", text: separator(t, now: now, calendar)))
                }
                prevTime = t; turnEnd = t
            }
            if case .assistantText = it.kind { turnHasReply = true }
            if case .toolChips(let c) = it.kind { turnTools += c.reduce(0) { $0 + max(1, $1.calls.count) } }
            out.append(row(it))
            prevRow = it
        }
        closeTurn()
        return out
    }

    static func clock(_ d: Date, _ cal: Calendar) -> String {
        let c = cal.dateComponents([.hour, .minute], from: d)
        return String(format: "%02d:%02d", c.hour ?? 0, c.minute ?? 0)
    }

    static func duration(_ s: TimeInterval) -> String {
        let n = Int(s.rounded())
        return n >= 3600 ? "\(n / 3600)h \(n % 3600 / 60)m" : n >= 60 ? "\(n / 60)m \(n % 60)s" : "\(n)s"
    }

    static func separator(_ d: Date, now: Date, _ cal: Calendar) -> String {
        if cal.isDate(d, inSameDayAs: now) { return clock(d, cal) }
        if let y = cal.date(byAdding: .day, value: -1, to: now), cal.isDate(d, inSameDayAs: y) { return "Yesterday \(clock(d, cal))" }
        let f = DateFormatter(); f.calendar = cal; f.timeZone = cal.timeZone; f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "EEE d MMM"
        return f.string(from: d) + " " + clock(d, cal)
    }
}
