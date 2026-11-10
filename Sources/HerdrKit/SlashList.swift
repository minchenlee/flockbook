import Foundation

/// The list the composer shows above the reply box while the text is "/" and a name with no space (the agent's
/// commands), or "/model " and a part of a value (that built-in's values). All the matches are in the list; `rows`
/// show at a time, and the window follows the pick (↑ ↓ stop at the ends, no wrap) or the wheel. Tab or Return
/// completes, Esc closes it until the text changes. Only full names go out: Claude Code's own "/" menu takes the
/// Return of a part of a name and runs its first match.
///
/// The Herdr app mod has the same rules (`herdr-app/mods/flockbook/composer.js`: slashQuery, argQuery,
/// matchCommands, matchArgs, windowTop, screenNote, slashKey, slashWheel).
public struct SlashList: Equatable, Sendable {
    /// A row: a command (its name without "/") or a value, with what the list shows beside it.
    public struct Item: Hashable, Sendable {
        public var name: String
        public var hint: String
        public var description: String
        /// A command that, sent bare, opens a screen in the terminal: the row says "needs a value".
        public var screen: Bool
        public init(name: String, hint: String = "", description: String = "", screen: Bool = false) {
            self.name = name; self.hint = hint; self.description = description; self.screen = screen
        }
    }

    /// The rows shown at a time, and a row's height in points (the wheel moves the window a row per `rowHeight`).
    public static let rows = 8
    public static let rowHeight = 30.0

    /// Nil: the items are commands. Else the command whose values they are.
    public var command: String?
    /// The part typed so far (of a name, or of a value).
    public var query: String
    /// Every match, in the list's order.
    public var items: [Item]
    /// The chosen match, and the first shown one.
    public var pick: Int
    public var top: Int

    /// The rows on screen now.
    public var shown: ArraySlice<Item> { items[top..<min(items.count, top + Self.rows)] }
    /// "4–11 of 20" when there are more matches than rows; else nil.
    public var count: String? {
        items.count > Self.rows ? "\(top + 1)–\(top + shown.count) of \(items.count)" : nil
    }
    /// A row's title: "/name" for a command, the value for a value.
    public func title(_ item: Item) -> String { command == nil ? "/\(item.name)" : item.name }
    /// The text after `item` is completed: "/name " (the space closes the list, or opens its values), or
    /// "/command value".
    public func completed(_ item: Item) -> String { command.map { "/\($0) \(item.name)" } ?? "/\(item.name) " }

    // MARK: the rules

    /// The name being typed after a leading "/", or nil: the text has a space (or is not a "/" name).
    public static func slashQuery(_ text: String) -> String? {
        guard text.hasPrefix("/") else { return nil }
        let rest = text.dropFirst()
        return rest.unicodeScalars.contains(where: space) ? nil : String(rest)
    }

    /// "/<name> <part>" (one space, no other): the command and the part of a value typed after it; else nil.
    public static func argQuery(_ text: String) -> (name: String, query: String)? {
        guard text.hasPrefix("/") else { return nil }
        let rest = text.unicodeScalars.dropFirst()
        guard let gap = rest.firstIndex(where: space), rest[gap] == " " else { return nil }
        let name = rest[..<gap], query = rest[rest.index(after: gap)...]
        guard !name.isEmpty, !query.contains(where: space) else { return nil }
        return (String(String.UnicodeScalarView(name)), String(String.UnicodeScalarView(query)))
    }

    private static func space(_ c: Unicode.Scalar) -> Bool { c.properties.isWhitespace || c == "\u{feff}" }

    /// The commands whose name starts with `query` (any case), in the list's order. All of them: the list scrolls.
    public static func matchCommands(_ list: [SlashCommands.Command], _ query: String) -> [SlashCommands.Command] {
        let q = query.lowercased()
        return list.filter { $0.name.lowercased().hasPrefix(q) }
    }

    /// The values that have `query` in their value or label (any case), in the list's order: a model can be found by
    /// a part of its name ("sonnet", "flash").
    public static func matchArgs(_ args: [SlashCommands.Arg], _ query: String) -> [SlashCommands.Arg] {
        let q = query.lowercased()
        return args.filter { !$0.value.isEmpty && (q.isEmpty || $0.value.lowercased().contains(q) || $0.label.lowercased().contains(q)) }
    }

    /// The first shown row so that `pick` is shown, from the window's last top. Keeps the window still while it can.
    public static func windowTop(_ top: Int, pick: Int, count: Int, rows: Int = SlashList.rows) -> Int {
        var t = max(0, min(top, max(0, count - rows)))
        if pick < t { t = pick } else if pick >= t + rows { t = pick - rows + 1 }
        return t
    }

    /// A screen command sent bare ("/model", spaces trimmed) opens a screen in the terminal the chat cannot show: why
    /// it is not sent, or nil. A listed screen command takes a value; the ones in `screens` take none.
    /// `SlashCommands.opensScreen` checks the same from the tables and builds the same text.
    public static func screenNote(_ listing: SlashCommands.Listing?, text: String) -> String? {
        guard let listing else { return nil }
        let t = SlashCommands.trim(text)
        guard t.hasPrefix("/") else { return nil }
        let name = String(t.dropFirst())
        guard !name.isEmpty, !name.unicodeScalars.contains(where: space) else { return nil }
        if listing.screens.contains(name) { return SlashCommands.screenNote(name: name, example: "", takesValue: false) }
        guard let c = listing.commands.first(where: { $0.name == name && $0.screen }) else { return nil }
        return SlashCommands.screenNote(name: name, example: c.args.first(where: { !$0.value.isEmpty })?.value ?? c.hint, takesValue: true)
    }
}

/// What the composer keeps for the list between key presses: the pick, the window, the wheel's left-over distance,
/// and the text Esc closed the list at.
public struct SlashPicker: Equatable, Sendable {
    public private(set) var pick = 0
    public private(set) var top = 0
    private var wheel = 0.0
    private var closedAt: String?
    public init() {}

    /// What Return does with the text.
    public enum Enter: Equatable, Sendable {
        /// A bare screen command: nothing is sent, the text stays, the note says why.
        case blocked(String)
        /// The list shows and the name (or value) is not whole yet: the text becomes this.
        case complete(String)
        case send
    }

    /// The text changed (typed, pasted, cleared): a new filter starts at its first match, and a closed list may open.
    public mutating func textChanged(_ text: String) {
        if closedAt != nil, text != closedAt { closedAt = nil }
        pick = 0; top = 0; wheel = 0
    }

    /// The list to show for `text` now, or nil: no match, no "/" name, or Esc closed it at this text.
    public func list(text: String, commands: [SlashCommands.Command]) -> SlashList? {
        text == closedAt ? nil : open(text, commands)
    }

    /// The list for `text`, closed by Esc or not.
    private func open(_ text: String, _ commands: [SlashCommands.Command]) -> SlashList? {
        var command: String?, query: String, items: [SlashList.Item]
        if let q = SlashList.slashQuery(text) {
            query = q
            items = SlashList.matchCommands(commands, q).map { .init(name: $0.name, hint: $0.hint, description: $0.description, screen: $0.screen) }
        } else {
            guard let a = SlashList.argQuery(text), let c = commands.first(where: { $0.name == a.name && !$0.args.isEmpty }) else { return nil }
            command = c.name; query = a.query
            items = SlashList.matchArgs(c.args, a.query).map { .init(name: $0.value, description: $0.label) }
        }
        guard !items.isEmpty else { return nil }
        let p = min(pick, items.count - 1)
        return SlashList(command: command, query: query, items: items, pick: p, top: SlashList.windowTop(top, pick: p, count: items.count))
    }

    /// ↑ (-1) or ↓ (+1): the pick moves and stops at the ends; the window follows. False: no list shows, so the key
    /// is the text's own (the cursor moves).
    public mutating func move(_ by: Int, text: String, commands: [SlashCommands.Command]) -> Bool {
        guard let s = list(text: text, commands: commands) else { return false }
        pick = max(0, min(s.items.count - 1, s.pick + by))
        top = SlashList.windowTop(s.top, pick: pick, count: s.items.count)
        return true
    }

    /// Tab: the text with the picked row completed, or nil when no list shows (Tab is the text's own).
    public func completion(text: String, commands: [SlashCommands.Command]) -> String? {
        list(text: text, commands: commands).map { $0.completed($0.items[$0.pick]) }
    }

    /// Esc: the list closes until the text changes. False: no list shows.
    public mutating func close(text: String, commands: [SlashCommands.Command]) -> Bool {
        guard list(text: text, commands: commands) != nil else { return false }
        closedAt = text
        return true
    }

    /// Wheel or trackpad over the list (`deltaY` in points, above zero: up): the window moves a row per `rowHeight`
    /// (small steps add up), and the pick stays on a shown row.
    public mutating func scroll(deltaY: Double, text: String, commands: [SlashCommands.Command]) {
        guard let s = list(text: text, commands: commands), s.items.count > SlashList.rows else { return }
        wheel -= deltaY
        let rows = Int((wheel / SlashList.rowHeight).rounded(.towardZero))
        guard rows != 0 else { return }
        wheel -= Double(rows) * SlashList.rowHeight
        top = max(0, min(s.items.count - SlashList.rows, s.top + rows))
        pick = max(top, min(top + SlashList.rows - 1, s.pick))
    }

    /// What Return does with `text` (as it would be sent: no spaces at its end). A message with files is never a
    /// bare command.
    public func enter(text: String, listing: SlashCommands.Listing?, hasFiles: Bool) -> Enter {
        if !hasFiles, let note = SlashList.screenNote(listing, text: text) { return .blocked(note) }
        if let s = list(text: text, commands: listing?.commands ?? []), !s.items.contains(where: { $0.name == s.query }) {
            return .complete(s.completed(s.items[s.pick]))
        }
        return .send
    }
}
