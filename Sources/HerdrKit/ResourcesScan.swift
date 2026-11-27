import Foundation

/// What an agent points the human to in its reply: the rules of the helper's \`resources\` module
/// (xplat/flockbook-modfeed/src/resources.rs), checked against the same vectors (Tests/Fixtures/resources).
public enum Resources {
    /// A target in a reply's text: UTF-16 offsets, its kind (doc, local, cmd, pr, issue, ci, artifact, link) and the
    /// target as it opens (a URL with a scheme, or the path as written).
    public struct Found: Equatable, Sendable {
        public var start: Int
        public var end: Int
        public var kind: String
        public var target: String
    }

    /// A one-line shell command the reply gives the human to run: where its block starts (UTF-16), its title, the
    /// command as written.
    public struct Command: Equatable, Sendable {
        public var start: Int
        public var title: String
        public var cmd: String
    }

    static let docExt = ["md", "html", "htm", "pdf", "png", "svg"]
    static let cjkPunct = Set("。，、；：！？「」『』（）《》【】…—“”‘’".unicodeScalars)
    static let cmdLangs: Set<String> = ["", "sh", "bash", "zsh", "shell", "console"]
    static let cmdMax = 400, titleMax = 80, headMax = 60

    /// The targets the feed links (no relative paths).
    public static func detect(_ text: String) -> [Found] { scan(text, rel: false) }

    /// The targets in \`text\`, in order, none overlapping (see the helper's \`detect\`); with \`rel\` also relative doc
    /// paths, as Resources collects them.
    public static func scan(_ text: String, rel: Bool) -> [Found] {
        let b = Array(text.utf8)
        var taken = [Bool](repeating: true, count: b.count)
        var hits: [(Int, Int, String, String)] = []
        var inside = 0
        var html = false
        func str(_ s: Int, _ e: Int) -> String { String(decoding: b[s..<e], as: UTF8.self) }
        for ev in ResMD.events(text) {
            switch ev {
            case let .start(.link(dest)):
                if inside == 0, let (ds, de) = dest, let (k, t) = wholeRel(str(ds, de), rel) { hits.append((ds, de, k, t)) }
                inside += 1
            case .start(.fenced), .start(.indentedCode), .start(.image), .start(.htmlBlock): inside += 1
            case .end(.link), .end(.fenced), .end(.indentedCode), .end(.image), .end(.htmlBlock): inside = max(0, inside - 1)
            case .inlineHTML: html = true
            case .end(.paragraph), .end(.heading), .end(.cell), .end(.item): html = false
            case let .text(s, e, _, asWritten) where inside == 0 && !html && asWritten:
                for i in s..<e { taken[i] = false }
            case let .code(s, e, c) where inside == 0 && !html:
                var n = 0
                while s + n < e, b[s + n] == 0x60 { n += 1 }
                guard e - s >= 2 * n else { continue }
                let inner = str(s + n, e - n)
                let lead = inner.utf8.count - String(inner.drop(while: { $0.isWhitespace })).utf8.count
                let tok = inner.trimmingCharacters(in: .whitespacesAndNewlines)
                if tok == c.trimmingCharacters(in: .whitespacesAndNewlines), let (k, t) = wholeRel(tok, rel) {
                    let st = s + n + lead
                    hits.append((st, st + tok.utf8.count, k, t))
                }
            default: break
            }
        }
        bareURLs(b, &taken, &hits)
        barePaths(b, &taken, &hits)
        if rel { bareRel(b, &taken, &hits) }
        hits.sort { $0.0 < $1.0 }
        var out: [Found] = []
        var at = 0, u = 0
        func to16(_ byte: Int) -> Int {
            u += String(decoding: b[at..<byte], as: UTF8.self).utf16.count
            at = byte
            return u
        }
        for (s, e, k, t) in hits {
            let st = to16(s)
            out.append(Found(start: st, end: to16(e), kind: k, target: t))
        }
        return out
    }

    static func isURL(_ s: String) -> Bool {
        let l = String(s.utf8.prefix(8).map { Character(UnicodeScalar($0 < 0x80 ? $0 : 0x3F)) }).lowercased()
        return l.hasPrefix("http://") || l.hasPrefix("https://")
    }

    static func hasWhitespace(_ s: String) -> Bool { s.unicodeScalars.contains { $0.properties.isWhitespace } }

    /// A whole token that is one target: its kind and target.
    static func whole(_ tok: String) -> (String, String)? {
        if tok.isEmpty || hasWhitespace(tok) { return nil }
        if isURL(tok) { return hostOf(tok) != nil ? (urlKind(tok), tok) : nil }
        if let t = bareHost(tok) { return (urlKind(t), t) }
        let tb = Array(tok.utf8)
        if (tok.hasPrefix("/") && !tok.hasPrefix("//") || tok.hasPrefix("~/")), docEnd(tb, 0, tb.count) == tb.count { return ("doc", tok) }
        // A `file:` URL on this machine (agy links its files so): the doc at its path.
        if let p = fileURLPath(tok) {
            let pb = Array(p.utf8)
            if docEnd(pb, 0, pb.count) == pb.count { return ("doc", p) }
        }
        return nil
    }

    /// The absolute path a `file:` URL on this machine names (`file:///a/b.md`, `file://localhost/a/b.md`), without
    /// its `?…` and `#…` and percent-decoded (it may then hold a space). Nil: another host, not UTF-8, not a full path.
    static func fileURLPath(_ tok: String) -> String? {
        var b = Array(tok.utf8)
        func lower(_ s: ArraySlice<UInt8>) -> String { String(decoding: s.map { $0 >= 0x41 && $0 <= 0x5A ? $0 + 32 : $0 }, as: UTF8.self) }
        guard b.count >= 7, lower(b[0..<7]) == "file://" else { return nil }
        b.removeFirst(7)
        if b.count >= 9, lower(b[0..<9]) == "localhost" { b.removeFirst(9) }
        if let q = b.firstIndex(where: { $0 == 0x3F || $0 == 0x23 }) { b = Array(b[..<q]) }
        // The path starts with a written "/", not an encoded one.
        guard b.first == 0x2F, let p = percentDecode(b) else { return nil }
        return p.hasPrefix("/") && !p.hasPrefix("//") && !p.contains(where: { $0 == "\n" || $0 == "\r" || $0 == "\0" || $0 == "\r\n" }) ? p : nil
    }

    /// `%XX` to its byte (two hex digits; anything else after "%" is not a URL path); a "%" too near the end stays.
    static func percentDecode(_ b: [UInt8]) -> String? {
        func hex(_ c: UInt8) -> UInt8? {
            switch c {
            case 0x30...0x39: c - 0x30
            case 0x41...0x46: c - 0x37
            case 0x61...0x66: c - 0x57
            default: nil
            }
        }
        var out: [UInt8] = [], i = 0
        out.reserveCapacity(b.count)
        while i < b.count {
            if b[i] == 0x25, i + 2 < b.count {
                guard let h = hex(b[i + 1]), let l = hex(b[i + 2]) else { return nil }
                out.append(h << 4 | l); i += 3
            } else { out.append(b[i]); i += 1 }
        }
        return String(validating: out, as: UTF8.self)
    }

    static func wholeRel(_ tok: String, _ rel: Bool) -> (String, String)? {
        if let w = whole(tok) { return w }
        return rel && isRelDoc(tok) ? ("doc", tok) : nil
    }

    /// A relative path to a doc: a folder and a file (\`docs/a.md\`, \`./a.md\`), not a bare name, a scheme-less
    /// address, a pattern or a placeholder.
    static func isRelDoc(_ tok: String) -> Bool {
        guard let slash = tok.firstIndex(of: "/") else { return false }
        let first = tok[..<slash]
        let firstOK = first == "." || first == ".." || first.hasPrefix(".") || (!first.isEmpty && !first.contains("."))
        let bad = Set("*?<>{}$[]|")
        let tb = Array(tok.utf8)
        return firstOK && !tok.hasPrefix("~") && !tok.contains("//") && !tok.contains(where: { bad.contains($0) }) && !tok.contains(":") && !hasWhitespace(tok) && docEnd(tb, 0, tb.count) == tb.count
    }

    static func asciiLower(_ s: String) -> String {
        String(decoding: s.utf8.map { $0 >= 0x41 && $0 <= 0x5A ? $0 + 32 : $0 }, as: UTF8.self)
    }

    static func bareHost(_ tok: String) -> String? {
        let l = asciiLower(tok)
        for h in ["localhost:", "127.0.0.1:"] where l.hasPrefix(h) {
            let rest = l.utf8.dropFirst(h.utf8.count)
            return rest.first.map { $0 >= 0x30 && $0 <= 0x39 } == true ? "http://\(tok)" : nil
        }
        if l.hasPrefix("claude.ai/") || l.hasPrefix("github.com/") {
            let u = "https://\(tok)"
            return ["artifact", "pr", "issue", "ci"].contains(urlKind(u)) ? u : nil
        }
        return nil
    }

    /// The host of a URL with a scheme, lowercased, without user and port.
    static func hostOf(_ url: String) -> String? {
        guard let r = url.range(of: "://") else { return nil }
        let rest = url[r.upperBound...]
        let auth = rest[..<(rest.firstIndex(where: { $0 == "/" || $0 == "?" || $0 == "#" }) ?? rest.endIndex)]
        let h0 = auth.split(separator: "@", omittingEmptySubsequences: false).last ?? auth
        let h: Substring
        if h0.hasPrefix("[") {
            h = h0[..<(h0.firstIndex(of: "]").map { h0.index(after: $0) } ?? h0.endIndex)]
        } else {
            h = h0.split(separator: ":", omittingEmptySubsequences: false).first ?? h0
        }
        return h.isEmpty ? nil : asciiLower(String(h))
    }

    static func isLocal(_ h: String) -> Bool {
        if h == "localhost" || h.hasSuffix(".localhost") || h.hasSuffix(".local") || h == "0.0.0.0" || h == "[::1]" { return true }
        let parts = h.split(separator: ".", omittingEmptySubsequences: false)
        let p = parts.compactMap { UInt8($0) }
        guard p.count == 4, parts.count == 4 else { return false }
        return p[0] == 127 || p[0] == 10 || (p[0] == 192 && p[1] == 168) || (p[0] == 172 && (16...31).contains(p[1])) || (p[0] == 169 && p[1] == 254)
    }

    /// The kind of a URL with a scheme: artifact, pr, issue, ci, local or link.
    public static func urlKind(_ url: String) -> String {
        guard let host = hostOf(url) else { return "link" }
        if isLocal(host) { return "local" }
        let rest = url.range(of: "://").map { String(url[$0.upperBound...]) } ?? ""
        var path = rest.firstIndex(of: "/").map { String(rest[$0...]) } ?? ""
        if let q = path.firstIndex(where: { $0 == "?" || $0 == "#" }) { path = String(path[..<q]) }
        let seg = path.split(separator: "/").map(String.init)
        func num(_ i: Int) -> Bool { i < seg.count && seg[i].utf8.allSatisfy { $0 >= 0x30 && $0 <= 0x39 } }
        var hh = Substring(host)
        while hh.hasPrefix("www.") { hh = hh.dropFirst(4) }
        switch hh {
        case "claude.ai":
            let s = seg.first == "code" ? Array(seg.dropFirst()) : seg
            if s.first == "artifact", s.count >= 2 { return "artifact" }
        case "github.com" where seg.count >= 4:
            switch seg[2] {
            case "pull" where num(3): return "pr"
            case "issues" where num(3): return "issue"
            case "actions" where seg[3] == "runs" && num(4): return "ci"
            default: break
            }
        default: break
        }
        return "link"
    }

    static func trimEnd(_ s0: String) -> String {
        var s = s0
        while let c = s.unicodeScalars.last {
            let drop: Bool
            switch c {
            case ".", ",", ";", ":", "!", "?", "'", "\"", "*", "~": drop = true
            case ")": drop = s.filter { $0 == "(" }.count < s.filter { $0 == ")" }.count
            case "]": drop = s.filter { $0 == "[" }.count < s.filter { $0 == "]" }.count
            default: drop = false
            }
            if !drop { return s }
            s.unicodeScalars.removeLast()
        }
        return s
    }

    static func lowerByte(_ x: UInt8) -> UInt8 { x >= 0x41 && x <= 0x5A ? x + 32 : x }

    static func urlByte(_ c: UInt8) -> Bool { c > 0x20 && c < 0x7F && !"<>\"\u{60}{}|\\^".utf8.contains(c) }

    static func scalarBefore(_ b: [UInt8], _ i: Int) -> Unicode.Scalar? {
        guard i > 0 else { return nil }
        var s = i - 1
        while s > 0, b[s] & 0xC0 == 0x80 { s -= 1 }
        return String(decoding: b[s..<i], as: UTF8.self).unicodeScalars.first
    }

    static func startsHere(_ b: [UInt8], _ i: Int) -> Bool {
        guard let c = scalarBefore(b, i) else { return true }
        return !((c.isASCII && isAlnum(UInt8(c.value))) || "/._-~:@%+=#$&\\".unicodeScalars.contains(c))
    }

    static func bareURLs(_ b: [UInt8], _ taken: inout [Bool], _ hits: inout [(Int, Int, String, String)]) {
        var i = 0
        while i < b.count {
            if taken[i] || b[i] & 0xC0 == 0x80 || !startsHere(b, i) { i += 1; continue }
            let l = asciiLower(String(decoding: b[i..<min(b.count, i + 12)], as: UTF8.self))
            let schemed = l.hasPrefix("http://") || l.hasPrefix("https://")
            let hosty = ["localhost:", "127.0.0.1:", "claude.ai/", "github.com/"].contains { l.hasPrefix($0) }
            if !schemed && !hosty { i += 1; continue }
            var j = i
            while j < b.count, urlByte(b[j]) { j += 1 }
            let tok = trimEnd(String(decoding: b[i..<j], as: UTF8.self))
            let e = i + tok.utf8.count
            let escaped = j < b.count && b[j] == 0x5C && !(j + 1 >= b.count || b[j + 1] == 0x0A || b[j + 1] == 0x0D)
            if escaped || taken[i..<e].contains(true) { i = max(j, i + 1); continue }
            let found: (String, String)? = schemed ? (hostOf(tok) != nil ? (urlKind(tok), tok) : nil) : bareHost(tok).map { (urlKind($0), $0) }
            if let (k, t) = found {
                hits.append((i, e, k, t))
                for x in i..<e { taken[x] = true }
                i = e
            } else { i = max(j, i + 1) }
        }
    }

    static func pathChar(_ c: Unicode.Scalar) -> Bool {
        !(c.properties.isWhitespace || c.properties.generalCategory == .control || "\u{60}<>\"'|()[]{}*,;".unicodeScalars.contains(c) || cjkPunct.contains(c))
    }

    /// The end of the last doc extension in b[s..<j] that ends the path.
    static func docEnd(_ b: [UInt8], _ s: Int, _ j: Int) -> Int? {
        var best: Int? = nil
        var p = s
        while p < j {
            defer { p += 1 }
            guard b[p] == 0x2E else { continue }
            let after = p + 1
            for ext in docExt {
                let eb = Array(ext.utf8)
                guard j - after >= eb.count else { continue }
                if after + eb.count < j, b[after + eb.count] & 0xC0 == 0x80 { continue }
                var same = true
                for (x, y) in zip(b[after..<(after + eb.count)], eb) where lowerByte(x) != y { same = false }
                guard same else { continue }
                let e = after + eb.count
                let ends: Bool
                if e >= b.count { ends = true } else {
                    let c = b[e]
                    if isAlnum(c) || c == 0x5F || c == 0x2F || c == 0x2D { ends = false } else if c == 0x2E { ends = !(e + 1 < b.count && isAlnum(b[e + 1])) } else { ends = true }
                }
                if ends, p > s { best = e }
            }
        }
        return best
    }

    /// The end of a run of path characters from i.
    static func pathRun(_ b: [UInt8], _ i: Int) -> Int {
        var j = i
        for u in String(decoding: b[i...], as: UTF8.self).unicodeScalars {
            if !pathChar(u) { break }
            j += String(u).utf8.count
        }
        return j
    }

    static func barePaths(_ b: [UInt8], _ taken: inout [Bool], _ hits: inout [(Int, Int, String, String)]) {
        var i = 0
        while i < b.count {
            let begins: Bool
            switch b[i] {
            case 0x2F: begins = i + 1 < b.count && b[i + 1] != 0x2F && !(b[i + 1] == 0x20 || (b[i + 1] >= 0x09 && b[i + 1] <= 0x0D))
            case 0x7E: begins = i + 1 < b.count && b[i + 1] == 0x2F
            default: begins = false
            }
            if taken[i] || !begins || !startsHere(b, i) { i += 1; continue }
            let j = pathRun(b, i)
            if let e = docEnd(b, i, j) {
                if taken[i..<e].contains(true) { i = max(j, i + 1); continue }
                hits.append((i, e, "doc", String(decoding: b[i..<e], as: UTF8.self)))
                for x in i..<e { taken[x] = true }
                i = e
            } else { i = max(j, i + 1) }
        }
    }

    static func bareRel(_ b: [UInt8], _ taken: inout [Bool], _ hits: inout [(Int, Int, String, String)]) {
        var i = 0
        while i < b.count {
            if taken[i] || !(isAlnum(b[i]) || b[i] == 0x2E || b[i] == 0x5F) || !startsHere(b, i) { i += 1; continue }
            let j = pathRun(b, i)
            if let e = docEnd(b, i, j), isRelDoc(String(decoding: b[i..<e], as: UTF8.self)), !taken[i..<e].contains(true) {
                hits.append((i, e, "doc", String(decoding: b[i..<e], as: UTF8.self)))
                for x in i..<e { taken[x] = true }
                i = e
            } else { i = max(j, i + 1) }
        }
    }

    // MARK: - Commands

    /// The commands in \`text\`, in order (see the helper's \`commands\`).
    public static func commands(_ text: String) -> [Command] {
        var out: [Command] = []
        var before: String? = nil
        var cur: String? = nil
        let b = Array(text.utf8)
        for ev in ResMD.events(text) {
            switch ev {
            case .start(.paragraph), .start(.heading): cur = ""
            case .end(.paragraph), .end(.heading): before = cur; cur = nil
            case let .end(.fenced(start, lang, body)):
                if cmdLangs.contains(lang), let cmd = oneCommand(body) {
                    let t = before.map(titleOf).flatMap { $0.isEmpty ? nil : $0 } ?? cut(oneLine(cmd), headMax)
                    out.append(Command(start: String(decoding: b[0..<start], as: UTF8.self).utf16.count, title: t, cmd: cmd))
                }
                before = nil
            case .start(.fenced): break // its body comes with its end
            case let .text(_, _, v, _): if cur != nil { cur! += v }
            case let .code(_, _, v): if cur != nil { cur! += v }
            case .softBreak, .hardBreak: if cur != nil { cur! += "\n" }
            case .start(.link), .end(.link), .start(.image), .end(.image): break
            case .start, .end: if cur == nil { before = nil }
            default: break
            }
        }
        return out
    }

    /// Rust's str::lines: split at "\n", a "\r" before it dropped, no last empty line.
    static func rustLines(_ s: String) -> [String] {
        var parts = s.components(separatedBy: "\n")
        if parts.last == "" { parts.removeLast() }
        return parts.map { $0.hasSuffix("\r") ? String($0.dropLast()) : $0 }
    }

    static func trimWS(_ s: some StringProtocol) -> String {
        let u = Array(s.unicodeScalars)
        var a = 0, z = u.count
        while a < z, u[a].properties.isWhitespace { a += 1 }
        while z > a, u[z - 1].properties.isWhitespace { z -= 1 }
        var out = String.UnicodeScalarView()
        out.append(contentsOf: u[a..<z])
        return String(out)
    }
    static func trimEndWS(_ s: String) -> String {
        var u = s.unicodeScalars
        while let l = u.last, l.properties.isWhitespace { u.removeLast() }
        return String(u)
    }

    static func oneCommand(_ body: String) -> String? {
        var lines = rustLines(body)
        while let f = lines.first, trimWS(f).isEmpty { lines.removeFirst() }
        while let l = lines.last, trimWS(l).isEmpty { lines.removeLast() }
        guard let last = lines.last else { return nil }
        if trimEndWS(last).hasSuffix("\\") || lines.dropLast().contains(where: { !trimEndWS($0).hasSuffix("\\") }) { return nil }
        let all = trimWS(lines.joined(separator: "\n"))
        var cmd = all
        if all.hasPrefix("$ ") {
            var u = String(all.dropFirst(2)).unicodeScalars
            while let f = u.first, f.properties.isWhitespace { u.removeFirst() }
            cmd = String(u)
        }
        return !cmd.isEmpty && !cmd.hasPrefix("#") && cmd.count <= cmdMax ? cmd : nil
    }

    /// A command on one line: each line's ending "\\" dropped, the spaces made one.
    public static func oneLine(_ cmd: String) -> String {
        let joined = rustLines(cmd).map { l -> String in
            var t = trimWS(l)
            while t.hasSuffix("\\") { t.removeLast() }
            return t
        }.joined(separator: " ")
        return joined.unicodeScalars.split(whereSeparator: { $0.properties.isWhitespace }).map { String(String.UnicodeScalarView($0)) }.joined(separator: " ")
    }

    static func cut(_ s: String, _ max: Int) -> String {
        if s.unicodeScalars.count <= max { return s }
        var u = String.UnicodeScalarView()
        u.append(contentsOf: s.unicodeScalars.prefix(max - 1))
        return trimEndWS(String(u)) + "…"
    }

    static func titleOf(_ p: String) -> String {
        let line = rustLines(p).reversed().map { trimWS($0) }.first { !$0.isEmpty } ?? ""
        var u = line.unicodeScalars
        while let l = u.last, l == ":" || l == "：" || l == " " { u.removeLast() }
        return cut(trimWS(String(u)), titleMax)
    }
}
