import Foundation

// The list rules of the mod's Resources window (herdr-app/mods/flockbook/resources.js): what is removed, what is new,
// the groups as drawn, the time, and what a click does. The app's window draws what these say.
extension Resources {
    /// Rows a group draws before "Show N more"; agents whose list stays in memory.
    public static let rowLimit = 30, cacheMax = 8
    /// The kinds of file a click opens (the mod's OPENS).
    public static let opens: Set<String> = ["md", "html", "htm", "pdf", "png", "svg"]

    /// The resources in a session log file, newest first: `collect(lines:)` with each line's byte offset, so the rows
    /// are the ids the conversation has (Codex rows are named by their offset). Nil when the file cannot be read.
    public static func collect(file: URL, home: String?, agent: String? = nil) -> [Item]? {
        guard let data = try? Data(contentsOf: file, options: .mappedIfSafe) else { return nil }
        var parser = ConversationParser()
        var acc = Acc()
        var last: String? = nil
        var start = data.startIndex
        while start < data.endIndex {
            let end = data[start...].firstIndex(of: 0x0A) ?? data.endIndex
            if end > start, let line = String(data: data[start..<end], encoding: .utf8) {
                if let c = lineCwd(line), c != last { acc.cwd(c); last = c }
                parser.feed(line: line, offset: UInt64(start - data.startIndex))
            }
            start = end < data.endIndex ? end + 1 : end
        }
        acc.add(parser.items(), home: home)
        return acc.finish(home: home, agent: agent)
    }

    /// An item taken off the list, with the last mention it had then (nil: for good).
    public struct Removed: Codable, Equatable, Sendable {
        public var id: String
        public var lastTs: Int64?
        public init(id: String, lastTs: Int64?) { self.id = id; self.lastTs = lastTs }
    }

    /// Removed and not mentioned since.
    public static func isRemoved(_ item: Item, _ removed: [Removed]) -> Bool {
        guard let r = removed.first(where: { $0.id == item.id }) else { return false }
        guard let ts = r.lastTs else { return true }
        return (item.lastTs ?? 0) <= ts
    }

    public static func visible(_ items: [Item], removed: [Removed]) -> [Item] { items.filter { !isRemoved($0, removed) } }

    /// `removed` with `item` in it once, at the mention it has now.
    public static func removing(_ item: Item, from removed: [Removed]) -> [Removed] {
        removed.filter { $0.id != item.id } + [Removed(id: item.id, lastTs: item.lastTs)]
    }

    public static func isNew(_ item: Item, seen: Int64?) -> Bool { seen.map { (item.lastTs ?? 0) > $0 } ?? false }

    public static func newest(_ items: [Item]) -> Int64 { items.reduce(0) { max($0, $1.lastTs ?? 0) } }

    /// The badge: the visible items newer than `seen`.
    public static func badgeCount(_ items: [Item], seen: Int64?, removed: [Removed]) -> Int {
        visible(items, removed: removed).filter { isNew($0, seen: seen) }.count
    }

    /// A group as the window draws it: `shown` the rows drawn (none when folded), `rest` the ones under "Show N more".
    public struct Laid: Equatable, Sendable {
        public var group: Group
        public var items: [Item]
        public var collapsed: Bool
        public var shown: [Item]
        public var rest: Int
    }

    /// The groups with items, in order; folded ones (`collapsed`) draw no rows; one in `expanded` draws all.
    public static func layoutGroups(_ items: [Item], collapsed: Set<String> = [], expanded: Set<String> = [], limit: Int = rowLimit) -> [Laid] {
        var by: [String: [Item]] = [:]
        for i in items { if let g = groupOf(kind: i.kind, target: i.target) { by[g, default: []].append(i) } }
        return groups.compactMap { g in
            guard let its = by[g.id], !its.isEmpty else { return nil }
            if collapsed.contains(g.id) { return Laid(group: g, items: its, collapsed: true, shown: [], rest: 0) }
            let shown = expanded.contains(g.id) ? its : Array(its.prefix(limit))
            return Laid(group: g, items: its, collapsed: false, shown: shown, rest: its.count - shown.count)
        }
    }

    /// "now", "4m", "3h", "2d" ("" without a time).
    public static func ago(_ ms: Int64?, now: Int64) -> String {
        guard let ms, ms != 0 else { return "" }
        let s = max(0, (Double(now) - Double(ms)) / 1000)   // in Double: a saturated timestamp never overflows
        if s < 60 { return "now" }
        if s < 3600 { return "\(Int(s / 60))m" }
        if s < 86400 { return "\(Int(s / 3600))h" }
        return "\(Int(s / 86400))d"
    }

    /// What a click does.
    public enum Plan: Equatable, Sendable {
        /// A command: copied, never run.
        case copy(String)
        /// An http(s) address: the browser.
        case url(URL)
        /// A doc file here, of a kind that opens: `openable` looks at it again at the click.
        case file(String)
        /// Nothing opens; the line says why.
        case why(String)
    }

    /// The mod's openPlan. `machine`: the agent's machine when it is not this one (its files are there).
    public static func openPlan(_ item: Item, machine: String? = nil) -> Plan {
        if item.kind == "cmd" { return item.target.isEmpty ? .why("Nothing to copy.") : .copy(item.target) }
        if item.kind != "doc" {
            let t = item.target
            guard t.range(of: "^https?://", options: [.regularExpression, .caseInsensitive]) != nil, let u = URL(string: t) else {
                return .why("Only web addresses open from here.")
            }
            return .url(u)
        }
        let path = item.target
        if let machine { return .why("This file is on \(machine). Open it there.") }
        if !path.hasPrefix("/") { return .why("Not a full path.") }
        if !opens.contains(extOf(path)) { return .why("Flockbook does not open this kind of file.") }
        if item.exists != true { return .why("File not found.") }
        return .file(path)
    }

    /// The empty window's title and line.
    public static func emptyText(agent: Bool) -> (String, String) {
        agent ? ("No resources yet", "Docs, artifacts and links the agent mentions in its replies show up here.")
              : ("No agent here", "Open an agent's tab to see what it pointed you to.")
    }
}
