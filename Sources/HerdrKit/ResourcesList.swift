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
        var r = Reader()
        guard r.read(file) else { return nil }
        return r.items(home: home, agent: agent)
    }

    /// A log read in steps: each `read` parses only what the file grew by since the last one, so a live agent's big
    /// log is not parsed again every poll. A file that shrank or was replaced is read again from `base`. The last line
    /// without its newline is parsed with the rest, and again in full once the newline comes.
    public struct Reader: Sendable {
        var parser = ConversationParser()
        var cwds = Set<String>()
        /// Where this reader starts in the file (a line's start; `Progressive` reads what is before it).
        public private(set) var base = 0
        /// The byte after the last whole line parsed.
        public private(set) var offset = 0
        var file: UInt64?
        /// Bytes after the last newline at the last read (a line being written).
        var pending: String?

        public init(base: Int = 0) { self.base = base; offset = base }

        /// False: the file cannot be read (what was read before is kept).
        public mutating func read(_ url: URL) -> Bool {
            guard let (data, id) = Resources.mapped(url) else { return false }
            if data.count < offset || id != file { self = Reader(base: data.count < base ? 0 : base); file = id }
            read(data)
            return true
        }

        mutating func read(_ data: Data) {
            var start = data.startIndex + offset
            pending = nil
            while start < data.endIndex {
                guard let end = data[start...].firstIndex(of: 0x0A) else {
                    pending = String(data: data[start...], encoding: .utf8)
                    break
                }
                if end > start, let line = String(data: data[start..<end], encoding: .utf8) {
                    if let c = lineCwd(line) { cwds.insert(c) }
                    parser.feed(line: line, offset: UInt64(start - data.startIndex))
                }
                start = end + 1
                offset = start - data.startIndex
            }
        }

        /// What is read so far, as the list's raw material (`finish` makes the list).
        public func acc(home: String?) -> Acc {
            var p = parser, cs = cwds
            if let l = pending, !l.isEmpty {
                if let c = lineCwd(l) { cs.insert(c) }
                p.feed(line: l, offset: UInt64(offset))
            }
            var acc = Acc()
            for c in cs { acc.cwd(c) }
            acc.add(p.items(), home: home)
            return acc
        }

        public func items(home: String?, agent: String? = nil) -> [Item] { acc(home: home).finish(home: home, agent: agent) }
    }

    /// The file mapped, and its file number (a replaced log has another).
    static func mapped(_ url: URL) -> (Data, UInt64?)? {
        let a = try? FileManager.default.attributesOfItem(atPath: url.path)
        guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return nil }
        return (data, (a?[.systemFileNumber] as? NSNumber)?.uint64Value)
    }

    /// A log read newest first, a chunk at a time, so a big log shows its newest resources at once (`start`), then the
    /// older ones as `older` reads them. Each cut is at the start of a typed prompt, where no tool call and no reply is
    /// open, and each chunk is parsed alone; their lists join in file order, so the whole read gives what one read
    /// gives. The newest chunk is a `Reader`: later reads parse only what the log grew by. `split` false (logs of other
    /// agents, a remote copy): one chunk, the whole log.
    public struct Progressive: Sendable {
        /// The size of the first chunk (the mod reads the same); each older one is twice the last, up to `chunkMax`.
        public static let chunk = 8 << 20, chunkMax = 64 << 20
        /// Older chunks read so far (each one twice the last, up to `chunkMax`, as the helper's).
        var steps = 0
        public private(set) var live = Reader()
        /// The chunk read last and the ones newer than it, up to `live` (kept apart: a chunk that ends in an open tool
        /// call is read again with the one after it).
        var neighbor: (start: Int, end: Int, acc: Acc)?
        var rest = Acc()
        /// Bytes before `low` are not read yet; `size`: the log's size when the read began.
        public private(set) var low = 0
        public private(set) var size = 0
        public private(set) var started = false
        let split: Bool
        let chunk: Int

        public init(split: Bool = true, chunk: Int = Progressive.chunk) { self.split = split; self.chunk = max(1, chunk) }

        public var done: Bool { started && low == 0 }
        /// The part of the log read, 0...1.
        public var fraction: Double { size == 0 ? (started ? 1 : 0) : Double(size - low) / Double(size) }

        /// Reads the newest chunk (and, after that, what the log grew by; a log that shrank or was replaced starts over).
        /// False: the file cannot be read.
        public mutating func update(_ url: URL) -> Bool {
            guard let (data, id) = Resources.mapped(url) else { return false }
            if started, data.count >= live.offset, id == live.file {
                live.read(data)
                return true
            }
            self = Progressive(split: split, chunk: chunk)
            started = true
            size = data.count
            let b = split ? Resources.cut(data, near: data.count - chunk, below: data.count) : 0
            live = Reader(base: b); live.file = id
            live.read(data)
            low = b
            return true
        }

        /// Reads the next older chunk. False: the file cannot be read, or it changed (`update` starts over then).
        public mutating func older(_ url: URL, home: String?) -> Bool {
            guard !done, let (data, id) = Resources.mapped(url), id == live.file, data.count >= live.offset else { return false }
            steps += 1
            let s = Resources.cut(data, near: low - max(chunk, min(chunk << min(steps, 16), Progressive.chunkMax)), below: low)
            var e = low
            var r = Reader(base: s)
            r.read(data.prefix(e))
            if r.parser.hasOpenToolUse {
                // A tool call open at the cut: this chunk is read again with the next one.
                if let n = neighbor {
                    e = n.end
                    r = Reader(base: s); r.read(data.prefix(e))
                    neighbor = nil
                } else {
                    live = Reader(base: s); live.file = id; live.read(data)
                    low = s
                    return true
                }
            }
            if let n = neighbor { rest = n.acc.appending(rest) }
            neighbor = (s, e, r.acc(home: home))
            low = s
            return true
        }

        /// The list as read so far, newest first.
        public func items(home: String?, agent: String? = nil) -> [Item] {
            let old = (neighbor?.acc ?? Acc()).appending(rest)
            return old.appending(live.acc(home: home)).finish(home: home, agent: agent)
        }
    }

    /// Where a chunk ends: the start of the first typed prompt from `near` on, before `below`; else the last one
    /// before `near`; else 0 (the log's start). A typed prompt is a Claude Code user line whose content is a string.
    static func cut(_ data: Data, near: Int, below: Int) -> Int {
        guard near > 0 else { return 0 }
        let base = data.startIndex
        func lineStart(_ i: Int) -> Int {   // the start of the line holding byte i
            var j = i
            while j > 0, data[base + j - 1] != 0x0A { j -= 1 }
            return j
        }
        func prompt(_ s: Int) -> Bool {
            let end = data[(base + s)...].firstIndex(of: 0x0A) ?? data.endIndex
            return Resources.isPrompt(data[(base + s)..<end])
        }
        // Forward from `near`.
        var s = near == lineStart(near) ? near : (data[(base + near)...].firstIndex(of: 0x0A).map { $0 - base + 1 } ?? below)
        while s < below {
            if prompt(s) { return s }
            guard let nl = data[(base + s)..<(base + below)].firstIndex(of: 0x0A) else { break }
            s = nl - base + 1
        }
        // Back from `near`.
        s = lineStart(near)
        while s > 0 {
            if prompt(s) { return s }
            s = lineStart(s - 1)
        }
        return 0
    }

    static let promptMark = Array(#""message":{"role":"user","content":""#.utf8), userMark = Array(#""type":"user""#.utf8)
    static let sidechainMark = Array(#""isSidechain":true"#.utf8), metaMark = Array(#""isMeta":true"#.utf8)

    /// A Claude Code line with a typed prompt (not a tool's result, a note or a subagent's line).
    static func isPrompt(_ line: Data) -> Bool {
        guard line.first == 0x7B, contains(line, promptMark), contains(line, userMark) else { return false }
        return !contains(line, sidechainMark) && !contains(line, metaMark)
    }

    static func contains(_ d: Data, _ pat: [UInt8]) -> Bool {
        guard let f = pat.first, d.count >= pat.count else { return false }
        var i = d.startIndex
        let last = d.endIndex - pat.count
        while i <= last {
            guard let j = d[i...last].firstIndex(of: f) else { return false }
            var k = 1
            while k < pat.count, d[j + k] == pat[k] { k += 1 }
            if k == pat.count { return true }
            i = j + 1
        }
        return false
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
    /// Why an agent has no list: its log is not there, or cannot be read.
    public enum Problem: Sendable, Equatable { case noLog, unreadable }

    /// The panel when an agent's log is not there, or cannot be read.
    public static func problemText(_ p: Problem) -> (String, String) {
        switch p {
        case .noLog: ("Can't find this chat's log", "Flockbook keeps looking. The list shows up when the log does.")
        case .unreadable: ("Can't read this chat's log", "Flockbook tries again when the log changes.")
        }
    }

    public static func emptyText(agent: Bool) -> (String, String) {
        agent ? ("No resources yet", "Docs, artifacts and links the agent mentions in its replies show up here.")
              : ("No agent here", "Open an agent's tab to see what it pointed you to.")
    }
}
