import Foundation

extension Resources {
    /// One thing the agent pointed the human to (the helper's \`res\` line).
    public struct Item: Identifiable, Equatable, Sendable {
        /// FNV-1a of the item's key (the normalized target): the same target is one item.
        public var id: String
        public var kind: String
        /// What opens: an absolute path for a doc (\`~/\` expanded), else a URL; a command's text for \`cmd\`.
        public var target: String
        public var title: String
        /// A doc's folder (the home as \`~\`); a URL's host, or "owner/repo" on GitHub; a command on one line.
        public var detail: String
        /// The conversation row of the last mention.
        public var row: String
        /// Milliseconds since 1970 of the first and the last mention; nil when the log has no time.
        public var firstTs: Int64?
        public var lastTs: Int64?
        public var mentions: Int
        /// Docs only: the path is a file now.
        public var exists: Bool?
    }

    /// The resources in a session log's lines, newest first (the helper's \`answer_in\`, read whole). \`home\`: what \`~/\`
    /// stands for; \`agent\`: the pane's folder, for relative doc paths.
    public static func collect(lines: [String], home: String?, agent: String? = nil) -> [Item] {
        var acc = Acc()
        var last: String? = nil
        for l in lines {
            if let c = lineCwd(l), c != last { acc.cwd(c); last = c }
        }
        acc.add(SessionLog.parse(lines: lines), home: home)
        return acc.finish(home: home, agent: agent)
    }

    /// The resources in \`items\` (a session log as parsed), newest first. \`cwds\`: the working folders the log names.
    public static func collect(_ items: [ConversationItem], home: String?, agent: String? = nil, cwds: [String] = []) -> [Item] {
        var acc = Acc()
        cwds.forEach { acc.cwd($0) }
        acc.add(items, home: home)
        return acc.finish(home: home, agent: agent)
    }

    enum Title: Equatable { case given(String), file(String) }

    struct Held {
        var key, kind, target, row: String
        var firstTs, lastTs: Int64?
        var mentions: Int
        var seq: Int
        var titles: [Title]
    }

    /// The resources of a log read so far: \`add\` the items, \`finish\` makes the list.
    public struct Acc {
        var held: [Held] = []
        var seq = 0
        var cwds = Set<String>()
        var dirs = Set<String>()
        var files = Set<String>()
        var at: [String: Int] = [:]

        public init() {}

        public mutating func cwd(_ c: String) { cwds.insert(c) }

        public mutating func add(_ items: [ConversationItem], home: String?) {
            for it in merged(items) {
                let ts = it.timestamp.map { Int64(($0.timeIntervalSince1970 * 1000).rounded()) }
                var found: [(String, String, Title?)] = []
                switch it.kind {
                case let .assistantText(s):
                    var f: [(Int, String, String, Title?)] = scan(s, rel: true).map { ($0.start, $0.kind, $0.target, nil) }
                    f += commands(s).map { ($0.start, "cmd", $0.cmd, .given($0.title)) }
                    // A stable sort, as Rust's sort_by_key.
                    f = f.enumerated().sorted { ($0.element.0, $0.offset) < ($1.element.0, $1.offset) }.map(\.element)
                    found += f.map { ($0.1, $0.2, $0.3) }
                case let .toolChips(chips):
                    let calls = chips.flatMap(\.calls)
                    for c in calls where !c.isError && c.output != nil { places(c.input, home) }
                    for c in calls {
                        guard c.toolName == "Artifact", !c.isError, let out = c.output else { continue }
                        let (action, given, file) = artifactInput(c.input)
                        let title: Title? = given.map { Title.given($0) } ?? file.flatMap { $0.hasPrefix("/") ? Title.file($0) : nil }
                        if let a = action, a != "publish" { continue }
                        found += detect(out).filter { $0.kind == "artifact" }.map { ($0.kind, $0.target, title) }
                    }
                default: break
                }
                for (kind0, target0, title) in found {
                    let kind: String, key: String, target: String
                    if kind0 == "doc", isRelDoc(target0) {
                        let r = relClean(target0)
                        (kind, key, target) = ("doc", "rel:\(r)", r)
                    } else {
                        (kind, key, target) = normal(kind0, target0, home)
                    }
                    seq += 1
                    let k: Int
                    if let i = at[key] {
                        held[i].mentions += 1
                        held[i].lastTs = ts ?? held[i].lastTs
                        held[i].firstTs = held[i].firstTs ?? ts
                        held[i].row = it.id
                        held[i].target = target
                        held[i].seq = seq
                        k = i
                    } else {
                        at[key] = held.count
                        held.append(Held(key: key, kind: kind, target: target, row: it.id, firstTs: ts, lastTs: ts, mentions: 1, seq: seq, titles: []))
                        k = held.count - 1
                    }
                    switch title {
                    case let .given(t)?: held[k].titles = [.given(t)]
                    case let .file(p)?:
                        held[k].titles.removeAll { $0 == .file(p) }
                        held[k].titles.append(.file(p))
                    case nil: break
                    }
                }
            }
        }

        /// The doc path of a file tool and the folders of a command's \`cd /…\`.
        mutating func places(_ input: String, _ home: String?) {
            guard let o = (try? JSONSerialization.jsonObject(with: Data(input.utf8))) as? [String: Any] else { return }
            for k in ["file_path", "path", "notebook_path"] {
                if let s = o[k] as? String, let (kind, t) = whole(trimWS(s)), kind == "doc" { files.insert(normal("doc", t, home).2) }
            }
            for k in ["command", "cmd"] {
                var cmds: [String] = []
                if let s = o[k] as? String { cmds = [s] } else if let a = o[k] as? [Any] { cmds = a.compactMap { $0 as? String } }
                for d in cmds.flatMap(cdTargets) {
                    var p = normal("doc", d, home).2
                    while p.hasSuffix("/") { p.removeLast() }
                    dirs.insert(p)
                }
            }
        }

        func resolve(_ rel: String, _ agent: String?) -> String? {
            func one(_ c: Set<String>) -> String? { c.count == 1 ? c.first : nil }
            func found(_ b: String) -> String? {
                var base = b
                while base.hasSuffix("/") { base.removeLast() }
                let p = "\(base)/\(rel)"
                return isFile(p) ? realDoc(p) : nil
            }
            let here = Set((Array(cwds) + (agent.map { [$0] } ?? [])).compactMap(found))
            if !here.isEmpty { return one(here) }
            if rel.split(separator: "/", omittingEmptySubsequences: false).contains("..") { return nil }
            let end = "/\(rel)"
            let there = Set(dirs.compactMap(found) + files.filter { $0.hasSuffix(end) }.map(realDoc))
            return one(there)
        }

        public func finish(home: String?, agent agent0: String? = nil) -> [Item] {
            let agent = agent0.map { normal("doc", $0, home).2 }
            var keys: [String: String] = [:]
            for h in held where h.key.hasPrefix("rel:") {
                let r = String(h.key.dropFirst(4))
                keys[h.key] = resolve(r, agent).map { "doc:\($0)" } ?? ""
            }
            var merged: [Held] = []
            var at: [String: Int] = [:]
            for h0 in held {
                var h = h0
                if let k = keys[h.key] {
                    if k.isEmpty { continue }
                    h.target = k.hasPrefix("doc:") ? String(k.dropFirst(4)) : k
                    h.key = k
                } else if h.key.hasPrefix("doc:") {
                    let p = String(h.key.dropFirst(4))
                    let r = realDoc(p)
                    if r != p { (h.key, h.target) = ("doc:\(r)", r) }
                }
                if let i = at[h.key] {
                    merged[i].mentions += h.mentions
                    if let a = merged[i].firstTs, let b = h.firstTs { merged[i].firstTs = min(a, b) } else { merged[i].firstTs = merged[i].firstTs ?? h.firstTs }
                    if h.seq > merged[i].seq {
                        merged[i].seq = h.seq; merged[i].row = h.row; merged[i].target = h.target
                        merged[i].lastTs = h.lastTs ?? merged[i].lastTs
                    }
                } else {
                    at[h.key] = merged.count
                    merged.append(h)
                }
            }
            merged.sort { ($0.lastTs ?? Int64.min, $0.seq) > ($1.lastTs ?? Int64.min, $1.seq) }
            return merged.map { h in
                var title: String? = nil
                for t in h.titles.reversed() {
                    switch t {
                    case let .given(s): title = s
                    case let .file(p): title = openable(p).flatMap(fileTitle)
                    }
                    if title != nil { break }
                }
                let kinds = ["doc", "local", "cmd", "pr", "issue", "ci", "artifact", "link"]
                let item = Item(id: fnv1a(h.key), kind: kinds.contains(h.kind) ? h.kind : "link", target: h.target, title: title ?? "", detail: "", row: h.row, firstTs: h.firstTs, lastTs: h.lastTs, mentions: h.mentions, exists: nil)
                return described(item, home)
            }
        }
    }

    /// Runs of tool chips joined into one item (the first one's id and time), as the helper's \`rows::merged\`.
    static func merged(_ items: [ConversationItem]) -> [ConversationItem] {
        var out: [ConversationItem] = []
        var run: [ConversationItem] = []
        func flush() {
            guard let first = run.first else { return }
            if run.count == 1 { out.append(first) } else {
                let chips = run.flatMap { r -> [ToolChip] in if case let .toolChips(c) = r.kind { return c }; return [] }
                out.append(ConversationItem(id: first.id, kind: .toolChips(chips), timestamp: first.timestamp))
            }
            run = []
        }
        for it in items {
            if case .toolChips = it.kind { run.append(it) } else { flush(); out.append(it) }
        }
        flush()
        return out
    }

    /// FNV-1a of the bytes, as lowercase hex without leading zeros (the helper's \`protocol::fnv1a\`).
    public static func fnv1a(_ s: String) -> String {
        var h: UInt64 = 0xcbf2_9ce4_8422_2325
        for b in s.utf8 { h ^= UInt64(b); h = h &* 0x100_0000_01b3 }
        return String(h, radix: 16)
    }

    /// The working folder a log line names (the first \`"cwd":"…"\`; absolute, no escape in it).
    static func lineCwd(_ l: String) -> String? {
        guard let r = l.range(of: "\"cwd\":\"") else { return nil }
        let rest = l[r.upperBound...]
        guard let e = rest.firstIndex(of: "\"") else { return nil }
        var c = String(rest[..<e])
        guard c.hasPrefix("/"), !c.contains("\\") else { return nil }
        while c.hasSuffix("/") { c.removeLast() }
        return c.isEmpty ? nil : c
    }

    /// The \`action\`, \`title\` and \`file_path\` of an Artifact call's input (JSON, maybe cut: then read as text).
    static func artifactInput(_ input: String) -> (String?, String?, String?) {
        func clean(_ t: String?) -> String? { t.flatMap { trimWS($0).isEmpty ? nil : trimWS($0) } }
        if let v = try? JSONSerialization.jsonObject(with: Data(input.utf8), options: [.fragmentsAllowed]) {
            let o = v as? [String: Any]
            return (o?["action"] as? String, clean(o?["title"] as? String), o?["file_path"] as? String)
        }
        func field(_ k: String) -> String? {
            guard let r = input.range(of: "\"\(k)\"") else { return nil }
            var rest = Substring(input[r.upperBound...])
            func skipWS() { while let f = rest.unicodeScalars.first, f.properties.isWhitespace { rest = Substring(rest.unicodeScalars.dropFirst()) } }
            skipWS()
            guard rest.hasPrefix(":") else { return nil }
            rest = rest.dropFirst()
            skipWS()
            guard rest.hasPrefix("\"") else { return nil }
            rest = rest.dropFirst()
            var out = ""
            var esc = false
            for c in rest.unicodeScalars {
                if esc {
                    out.unicodeScalars.append(c == "n" ? " " : c)
                    esc = false
                } else if c == "\\" { esc = true } else if c == "\"" { return out } else { out.unicodeScalars.append(c) }
            }
            return nil
        }
        return (field("action"), clean(field("title")), field("file_path"))
    }

    /// A URL's scheme (lowercase), authority without the user (lowercase), path, query (with "?").
    static func parts(_ url: String) -> (String, String, String, String) {
        var scheme = "http", rest = url
        if let r = url.range(of: "://") { scheme = String(url[..<r.lowerBound]); rest = String(url[r.upperBound...]) }
        if let h = rest.firstIndex(of: "#") { rest = String(rest[..<h]) }
        let a = rest.firstIndex(where: { $0 == "/" || $0 == "?" }) ?? rest.endIndex
        let auth0 = rest[..<a]
        let auth = asciiLower(String(auth0.split(separator: "@", omittingEmptySubsequences: false).last ?? auth0))
        let tail = rest[a...]
        var path = String(tail), query = ""
        if let q = tail.firstIndex(of: "?") { path = String(tail[..<q]); query = String(tail[q...]) }
        return (asciiLower(scheme), auth, path, query)
    }

    /// The kind, the key and the target that opens (see the helper's \`normal\`).
    static func normal(_ kind: String, _ target: String, _ home: String?) -> (String, String, String) {
        switch kind {
        case "doc":
            var p = target
            if target.hasPrefix("~/"), let h = home { p = homeJoin(h, String(target.dropFirst(2))) }
            return ("doc", "doc:\(p)", p)
        case "local":
            let (s, a, _, _) = parts(target)
            return ("local", "local:\(hostPort(s, a))", target)
        case "cmd":
            let t = trimWS(target)
            return ("cmd", "cmd:\(t)", t)
        case "pr", "issue", "ci":
            let seg = parts(target).2.split(separator: "/").map(String.init)
            if seg.count < (kind == "ci" ? 5 : 4) { return normal("link", target, home) }
            let page = kind == "ci" ? "actions/runs/\(seg[4])" : "\(seg[2])/\(seg[3])"
            let u = "https://github.com/\(seg[0])/\(seg[1])/\(page)"
            return (kind, u, u)
        default:
            let (s, a, p, q) = parts(target)
            var key = "\(s)://\(a)\(p)\(q)"
            while key.hasSuffix("/") { key.removeLast() }
            return (kind, key, target)
        }
    }

    static func relClean(_ p: String) -> String {
        var s = Substring(p)
        while s.hasPrefix("./") { s = s.dropFirst(2) }
        return String(s)
    }

    static func hostPort(_ scheme: String, _ auth: String) -> String {
        if let k = auth.lastIndex(of: ":"), !auth[k...].contains("]") { return auth }
        return "\(auth):\(scheme == "https" ? 443 : 80)"
    }

    static func homeJoin(_ home: String, _ rest: String) -> String {
        var h = home
        if h.hasSuffix("/") { h.removeLast() }
        return "\(h)/\(rest)"
    }

    /// The home as "~" at the start of a path.
    static func tilde(_ p: String, _ home: String?) -> String {
        if var h = home {
            while h.hasSuffix("/") { h.removeLast() }
            if !h.isEmpty {
                if p == h { return "~" }
                if p.hasPrefix(h + "/") { return "~/" + p.dropFirst(h.count + 1) }
            }
        }
        return p
    }

    // MARK: - Files

    /// A path's parent and name, as Rust's Path::parent and Path::file_name read them.
    static func split(_ p: String) -> (String, String)? {
        var s = p
        while s.count > 1, s.hasSuffix("/") { s.removeLast() }
        guard let k = s.lastIndex(of: "/") else { return s.isEmpty || s == ".." ? nil : ("", s) }
        let name = String(s[s.index(after: k)...])
        guard !name.isEmpty, name != ".." else { return nil }
        let dir = k == s.startIndex ? "/" : String(s[..<k])
        return (dir, name)
    }

    static func realpath(_ p: String) -> String? {
        guard let r = Darwin.realpath(p, nil) else { return nil }
        defer { free(r) }
        return String(cString: r)
    }

    static func isFile(_ p: String) -> Bool {
        var st = stat()
        return stat(p, &st) == 0 && (st.st_mode & S_IFMT) == S_IFREG
    }

    static func isLink(_ p: String) -> Bool {
        var st = stat()
        return lstat(p, &st) == 0 && (st.st_mode & S_IFMT) == S_IFLNK
    }

    /// A doc path as the file system names it: the folder's real path and the file's name, or the whole real path
    /// when the file itself is no link. The path as written when there is no such folder.
    static func realDoc(_ p: String) -> String {
        guard let (dir, name) = split(p), let rdir = realpath(dir.isEmpty ? "." : dir) else { return p }
        if !isLink(p), let r = realpath(p) { return r }
        return rdir == "/" ? "/\(name)" : "\(rdir)/\(name)"
    }

    static func ext(_ p: String) -> String {
        guard let name = split(p)?.1, let i = name.lastIndex(of: "."), i > name.startIndex else { return "" }
        return asciiLower(String(name[name.index(after: i)...]))
    }

    /// The real file a doc path opens, when it is a file of a kind Flockbook opens.
    public static func openable(_ p: String) -> String? {
        guard let real = realpath(p), isFile(real), docExt.contains(ext(real)) else { return nil }
        return real
    }

    /// A markdown file's first "# " heading, or an HTML file's <title>, in its first 4 KB.
    static func fileTitle(_ p: String) -> String? {
        let e = ext(p)
        guard let fh = FileHandle(forReadingAtPath: p) else { return nil }
        defer { try? fh.close() }
        guard let data = try? fh.read(upToCount: 4096) else { return nil }
        let head = String(decoding: data, as: UTF8.self)
        var t: String? = nil
        if e == "md" {
            t = rustLines(head).first { $0.hasPrefix("# ") }.map { trimWS($0.dropFirst(2)) }
        } else if e == "html" || e == "htm" {
            let hb = Array(head.utf8)
            let low = hb.map(lowerByte)
            func find(_ s: String, _ from: Int) -> Int? {
                let n = Array(s.utf8)
                var i = from
                while i + n.count <= low.count { if Array(low[i..<(i + n.count)]) == n { return i }; i += 1 }
                return nil
            }
            if let s0 = find("<title", 0), let gt = low[s0...].firstIndex(of: 0x3E), let e0 = find("</title", gt + 1) {
                let inner = String(decoding: hb[(gt + 1)..<e0], as: UTF8.self)
                t = inner.unicodeScalars.split(whereSeparator: { $0.properties.isWhitespace }).map { String(String.UnicodeScalarView($0)) }.joined(separator: " ")
            }
        }
        return t.flatMap { $0.isEmpty ? nil : $0 }
    }

    static func described(_ r0: Item, _ home: String?) -> Item {
        var r = r0
        switch r.kind {
        case "doc":
            let real = openable(r.target)
            let sp = split(r.target)
            r.title = real.flatMap(fileTitle) ?? sp?.1 ?? r.target
            r.detail = sp.map { tilde($0.0, home) } ?? ""
            r.exists = real != nil
        case "pr", "issue", "ci":
            var t = Substring(r.target)
            while t.hasPrefix("https://github.com/") { t = t.dropFirst("https://github.com/".count) }
            let seg = t.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
            r.detail = "\(seg[0])/\(seg.count > 1 ? seg[1] : "")"
            let n = seg.last ?? ""
            r.title = r.kind == "ci" ? "CI run \(n)" : "\(r.detail)#\(n)"
        case "cmd": r.detail = oneLine(r.target)
        case "local":
            let (s, a, _, _) = parts(r.target)
            r.title = a.contains(":") ? a : hostPort(s, a)
            r.detail = a.lastIndex(of: ":").map { String(a[..<$0]) } ?? a
        default:
            let (_, a, p, _) = parts(r.target)
            if r.title.isEmpty {
                var pp = p
                while pp.hasSuffix("/") { pp.removeLast() }
                r.title = r.kind == "artifact" ? "Artifact" : a + pp
            }
            r.detail = a
        }
        return r
    }

    // MARK: - cd

    /// The absolute folders a shell command goes to with a \`cd\` of its own (see the helper's \`cd_targets\`).
    static func cdTargets(_ s: String) -> [String] {
        enum Tok { case word(String, Bool), op }
        var toks: [Tok] = []
        var w = "", quoted = false, inWord = false
        func endWord() {
            if inWord { toks.append(.word(w, quoted)); w = "" }
            quoted = false; inWord = false
        }
        var it = Array(s.unicodeScalars)
        var i = 0
        func next() -> Unicode.Scalar? { guard i < it.count else { return nil }; defer { i += 1 }; return it[i] }
        while let c = next() {
            switch c {
            case "\\":
                guard let n = next() else { return [] }
                if n != "\n" { w.unicodeScalars.append(n); quoted = true; inWord = true }
            case "'":
                quoted = true; inWord = true
                while true {
                    guard let x = next() else { return [] }
                    if x == "'" { break }
                    w.unicodeScalars.append(x)
                }
            case "\"":
                quoted = true; inWord = true
                loop: while true {
                    guard let x = next() else { return [] }
                    switch x {
                    case "\"": break loop
                    case "\\":
                        guard let y = next() else { return [] }
                        if y == "\"" || y == "\\" || y == "$" || y == "\u{60}" { w.unicodeScalars.append(y) } else if y != "\n" { w += "\\"; w.unicodeScalars.append(y) }
                    default: w.unicodeScalars.append(x)
                    }
                }
            case ";", "|", "&", "(", ")", "{", "}", "\n":
                endWord(); toks.append(.op)
            default:
                if c.properties.isWhitespace { endWord() } else { w.unicodeScalars.append(c); inWord = true }
            }
        }
        endWord()
        it = []
        var out: [String] = []
        var atStart = true
        var k = 0
        while k < toks.count {
            switch toks[k] {
            case .op: atStart = true
            case let .word(word, q):
                if atStart, !q, word == "cd", k + 1 < toks.count, case let .word(d, _) = toks[k + 1] {
                    if (d.hasPrefix("/") || d.hasPrefix("~/")) && !d.contains(where: { "$\u{60}*?".contains($0) }) { out.append(d) }
                    k += 1
                }
                atStart = false
            }
            k += 1
        }
        return out
    }

    // MARK: - Groups

    /// The list's groups, in order: id, label, the kinds in it, and the doc extensions (nil: by kind only).
    public struct Group: Sendable, Equatable {
        public var id: String
        public var label: String
        public var kinds: [String]
        public var exts: [String]?
    }

    public static let groups: [Group] = [
        Group(id: "markdown", label: "Markdown", kinds: ["doc"], exts: ["md"]),
        Group(id: "web", label: "Web pages", kinds: ["doc"], exts: ["html", "htm"]),
        Group(id: "pdfs", label: "PDFs", kinds: ["doc"], exts: ["pdf"]),
        Group(id: "images", label: "Images", kinds: ["doc"], exts: ["png", "svg"]),
        Group(id: "otherdocs", label: "Other docs", kinds: ["doc"]),
        Group(id: "artifacts", label: "Artifacts", kinds: ["artifact"]),
        Group(id: "github", label: "PRs & CI", kinds: ["pr", "issue", "ci"]),
        Group(id: "commands", label: "Commands", kinds: ["cmd"]),
        Group(id: "links", label: "Links", kinds: ["link", "local"]),
    ]

    /// The extension of a target's last part, lowercased ("" without one), as the mod's \`extOf\`.
    public static func extOf(_ target: String) -> String {
        let name = target.split(omittingEmptySubsequences: false, whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? ""
        guard let i = name.lastIndex(of: "."), i > name.startIndex else { return "" }
        return String(name[name.index(after: i)...]).lowercased()
    }

    /// The id of the group an item is in (nil: none).
    public static func groupOf(kind: String, target: String) -> String? {
        if kind == "doc" {
            let e = extOf(target)
            return groups.first { $0.exts?.contains(e) == true }?.id ?? "otherdocs"
        }
        return groups.first { $0.exts == nil && $0.id != "otherdocs" && $0.kinds.contains(kind) }?.id
    }
}
