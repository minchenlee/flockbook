import Foundation

/// The CommonMark that Resources reads (see \`Resources.detect\` and \`Resources.commands\`): the events the Rust helper
/// gets from pulldown-cmark (xplat/flockbook-modfeed/src/resources.rs), with their byte ranges in the text. Blocks
/// (quotes, lists, paragraphs, headings, code, HTML, tables, rules) follow the CommonMark spec's parsing strategy;
/// inside them code spans, autolinks, inline HTML, links and pictures (inline, full, collapsed and shortcut
/// references), escapes, entities, and emphasis and strikethrough marks (taken off the text, as pulldown-cmark does).
/// Not a renderer: only what Resources needs is kept.
enum ResMD {
    enum Tag {
        case paragraph, heading, quote, list, item, table, cell, rule, indentedCode, htmlBlock, image
        /// A fenced code block: where it starts (bytes), its info's first word lowercased, and its body.
        case fenced(start: Int, lang: String, body: String)
        /// A link: its address's bytes when it is an inline link or an autolink and the address is written as read.
        case link(dest: (Int, Int)?)
    }

    enum Event {
        case start(Tag)
        case end(Tag)
        /// Text: its bytes, what it reads, and whether the bytes are what it reads (no escape, no entity).
        case text(Int, Int, String, Bool)
        /// A code span: its bytes (backticks too) and its content.
        case code(Int, Int, String)
        case inlineHTML, softBreak, hardBreak
    }

    static func events(_ text: String) -> [Event] {
        let b = Array(text.utf8)
        let p = BlockParser(b)
        p.run()
        var defs = Set<String>()
        p.doc.collectDefs(into: &defs)
        var out: [Event] = []
        p.doc.emit(b, defs, tight: false, into: &out)
        return out
    }
}

// MARK: - Blocks

extension ResMD {
    final class Node {
        enum Kind { case doc, quote, list, item, paragraph, heading, fenced, indented, html, table, rule }
        var kind: Kind
        weak var parent: Node?
        var children: [Node] = []
        var open = true
        var lastLineBlank = false
        var lastLineChecked = false
        var startLine = 0
        /// Paragraph and heading: the content lines (byte ranges). Table: the rows' cells.
        var lines: [(Int, Int)] = []
        var rows: [[(Int, Int)]] = []
        var cols = 0
        // Lists and items.
        var ordered = false, delim: UInt8 = 0, bullet: UInt8 = 0, markerOffset = 0, padding = 0, tight = true
        // Fenced code.
        var fenceChar: UInt8 = 0, fenceLength = 0, fenceOffset = 0, fenceStart = 0, lang = "", body = "", sawInfo = false
        var htmlType = 0
        var defs: [String] = []
        init(_ k: Kind) { kind = k }

        func canContain(_ k: Kind) -> Bool {
            switch kind {
            case .doc, .quote, .item: return k != .item
            case .list: return k == .item
            default: return false
            }
        }

        func collectDefs(into d: inout Set<String>) {
            d.formUnion(defs)
            for c in children { c.collectDefs(into: &d) }
        }

        func emit(_ b: [UInt8], _ defs: Set<String>, tight: Bool, into out: inout [Event]) {
            switch kind {
            case .doc: for c in children { c.emit(b, defs, tight: false, into: &out) }
            case .quote:
                out.append(.start(.quote)); for c in children { c.emit(b, defs, tight: false, into: &out) }; out.append(.end(.quote))
            case .list:
                out.append(.start(.list)); for c in children { c.emit(b, defs, tight: self.tight, into: &out) }; out.append(.end(.list))
            case .item:
                out.append(.start(.item)); for c in children { c.emit(b, defs, tight: tight, into: &out) }; out.append(.end(.item))
            case .paragraph:
                if !tight { out.append(.start(.paragraph)) }
                out += Inline.run(b, lines, defs)
                if !tight { out.append(.end(.paragraph)) }
            case .heading:
                out.append(.start(.heading)); out += Inline.run(b, lines, defs); out.append(.end(.heading))
            case .fenced:
                let t = Tag.fenced(start: fenceStart, lang: lang, body: body)
                out.append(.start(t)); out.append(.end(t))
            case .indented: out.append(.start(.indentedCode)); out.append(.end(.indentedCode))
            case .html: out.append(.start(.htmlBlock)); out.append(.end(.htmlBlock))
            case .rule: out.append(.start(.rule)); out.append(.end(.rule))
            case .table:
                out.append(.start(.table))
                for row in rows {
                    for cell in row {
                        out.append(.start(.cell))
                        if cell.0 < cell.1 { out += Inline.run(b, [cell], defs) }
                        out.append(.end(.cell))
                    }
                }
                out.append(.end(.table))
            }
        }
    }

    /// The block phase of the spec's parsing strategy (as in commonmark.js), with GFM tables.
    final class BlockParser {
        let b: [UInt8]
        let doc = Node(.doc)
        var tip: Node
        var oldtip: Node
        var lastMatched: Node
        var allClosed = true
        var lineStart = 0, lineEnd = 0, lineNumber = 0
        var offset = 0, column = 0, nextNonspace = 0, nextNonspaceColumn = 0, indent = 0
        var indented = false, blank = false, partialTab = false

        init(_ b: [UInt8]) { self.b = b; tip = doc; oldtip = doc; lastMatched = doc }

        func peek(_ i: Int) -> UInt8? { i < lineEnd ? b[i] : nil }
        static func spaceOrTab(_ c: UInt8?) -> Bool { c == 0x20 || c == 0x09 }

        func run() {
            var i = 0
            let n = b.count
            while true {
                var e = i
                while e < n, b[e] != 0x0A, b[e] != 0x0D { e += 1 }
                lineStart = i; lineEnd = e; lineNumber += 1
                line()
                if e >= n { break }
                i = e + (b[e] == 0x0D && e + 1 < n && b[e + 1] == 0x0A ? 2 : 1)
                if i >= n { break }
            }
            while let t = Optional(tip), t !== doc { finalize(t) }
            finalize(doc)
        }

        func findNextNonspace() {
            var i = offset, cols = column
            while i < lineEnd {
                if b[i] == 0x20 { i += 1; cols += 1 } else if b[i] == 0x09 { i += 1; cols += 4 - (cols % 4) } else { break }
            }
            blank = i >= lineEnd
            nextNonspace = i; nextNonspaceColumn = cols
            indent = cols - column
            indented = indent >= 4
        }

        func advanceNextNonspace() { offset = nextNonspace; column = nextNonspaceColumn; partialTab = false }

        func advance(_ count0: Int, columns: Bool) {
            var count = count0
            while count > 0, offset < lineEnd {
                if b[offset] == 0x09 {
                    let toTab = 4 - (column % 4)
                    if columns {
                        partialTab = toTab > count
                        let step = min(toTab, count)
                        column += step
                        offset += partialTab ? 0 : 1
                        count -= step
                    } else {
                        partialTab = false; column += toTab; offset += 1; count -= 1
                    }
                } else {
                    partialTab = false; offset += 1; column += 1; count -= 1
                }
            }
        }

        // MARK: one line

        func line() {
            offset = lineStart; column = 0; blank = false; partialTab = false
            allClosed = true
            var container = doc
            oldtip = tip
            var allMatched = true
            while let last = container.children.last, last.open {
                container = last
                findNextNonspace()
                switch cont(container) {
                case 0: continue
                case 1:
                    container = container.parent!
                    allMatched = false
                case 2: return
                default: break
                }
                if !allMatched { break }
            }
            allClosed = container === oldtip
            lastMatched = container
            var matchedLeaf = container.kind != .paragraph && container.kind != .table && acceptsLines(container.kind)
            while !matchedLeaf {
                findNextNonspace()
                if !indented, !maybeSpecial(peek(nextNonspace)) { advanceNextNonspace(); break }
                var res = 0
                for start in 0..<8 {
                    res = blockStart(start, container)
                    if res != 0 { break }
                }
                if res == 1 { container = tip; continue }
                if res == 2 { container = tip; matchedLeaf = true; break }
                advanceNextNonspace()
                break
            }
            if !allClosed, !blank, tip.kind == .paragraph {
                addLine(tip)   // lazy continuation
                return
            }
            closeUnmatched()
            if blank, let last = container.children.last { last.lastLineBlank = true }
            let t = container.kind
            let lastBlank = blank && !(t == .quote || t == .fenced || (t == .item && container.children.isEmpty && container.startLine == lineNumber))
            var c: Node? = container
            while let x = c { x.lastLineBlank = lastBlank; c = x.parent }
            if t == .paragraph, container.lines.count == 1, !blank, tableStart(container) { return }
            if t == .table {
                if !blank { container.rows.append(Array(cells(offset, lineEnd).prefix(container.cols))) }
                return
            }
            if acceptsLines(t) {
                addLine(container)
                if t == .html, (1...5).contains(container.htmlType), htmlEnds(container.htmlType, from: offset) { finalize(container) }
            } else if offset < lineEnd, !blank {
                let p = addChild(.paragraph)
                advanceNextNonspace()
                addLine(p)
            }
        }

        func acceptsLines(_ k: Node.Kind) -> Bool { k == .paragraph || k == .fenced || k == .indented || k == .html || k == .table }

        func maybeSpecial(_ c: UInt8?) -> Bool {
            guard let c else { return false }
            return "#\u{60}~*+_=<>-".utf8.contains(c) || (c >= 0x30 && c <= 0x39)
        }

        func addLine(_ n: Node) {
            switch n.kind {
            case .fenced:
                if !n.sawInfo { n.sawInfo = true; return }
                var s = ""
                if partialTab { offset += 1; s = String(repeating: " ", count: 4 - (column % 4)) }
                n.body += s + String(decoding: b[min(offset, lineEnd)..<lineEnd], as: UTF8.self) + "\n"
            case .paragraph, .heading:
                n.lines.append((min(offset, lineEnd), lineEnd))
            default: break
            }
        }

        /// 0: matched, 1: not matched, 2: the line is done (a fence closed).
        func cont(_ n: Node) -> Int {
            switch n.kind {
            case .quote:
                if !indented, peek(nextNonspace) == 0x3E {
                    advanceNextNonspace(); advance(1, columns: false)
                    if Self.spaceOrTab(peek(offset)) { advance(1, columns: true) }
                    return 0
                }
                return 1
            case .item:
                if blank {
                    if n.children.isEmpty { return 1 }
                    advanceNextNonspace(); return 0
                }
                if indent >= n.markerOffset + n.padding { advance(n.markerOffset + n.padding, columns: true); return 0 }
                return 1
            case .list, .doc: return 0
            case .heading, .rule: return 1
            case .fenced:
                if indent <= 3, peek(nextNonspace) == n.fenceChar {
                    var i = nextNonspace
                    while i < lineEnd, b[i] == n.fenceChar { i += 1 }
                    var k = i
                    while Self.spaceOrTab(peek(k)) { k += 1 }
                    if i - nextNonspace >= n.fenceLength, k >= lineEnd { finalize(n); return 2 }
                }
                var i = n.fenceOffset
                while i > 0, Self.spaceOrTab(peek(offset)) { advance(1, columns: true); i -= 1 }
                return 0
            case .indented:
                if indent >= 4 { advance(4, columns: true) } else if blank { advanceNextNonspace() } else { return 1 }
                return 0
            case .html: return blank && n.htmlType >= 6 ? 1 : 0
            case .paragraph, .table: return blank ? 1 : 0
            }
        }

        func closeUnmatched() {
            if !allClosed {
                while oldtip !== lastMatched {
                    let p = oldtip.parent!
                    finalize(oldtip)
                    oldtip = p
                }
                allClosed = true
            }
        }

        @discardableResult
        func addChild(_ k: Node.Kind) -> Node {
            while !tip.canContain(k) { finalize(tip) }
            let n = Node(k)
            n.parent = tip
            n.startLine = lineNumber
            tip.children.append(n)
            tip = n
            return n
        }

        func finalize(_ n: Node) {
            guard n.open else { return }
            n.open = false
            if let p = n.parent { tip = p }
            switch n.kind {
            case .paragraph:
                while !n.lines.isEmpty, let (label, used) = Defs.parse(b, n.lines) {
                    n.defs.append(label)
                    n.lines.removeFirst(used)
                }
                if n.lines.isEmpty, let p = n.parent {
                    // Only definitions: no paragraph (they stay with its parent).
                    p.defs += n.defs
                    p.children.removeAll { $0 === n }
                }
            case .list:
                for (i, item) in n.children.enumerated() {
                    let hasNext = i + 1 < n.children.count
                    if hasNext, endsWithBlank(item) { n.tight = false; break }
                    var loose = false
                    for (j, sub) in item.children.enumerated() where endsWithBlank(sub) && (hasNext || j + 1 < item.children.count) { loose = true; break }
                    if loose { n.tight = false; break }
                }
            default: break
            }
        }

        func endsWithBlank(_ n0: Node) -> Bool {
            var n: Node? = n0
            while let x = n {
                if x.lastLineBlank { return true }
                if !x.lastLineChecked, x.kind == .list || x.kind == .item {
                    x.lastLineChecked = true
                    n = x.children.last
                } else {
                    x.lastLineChecked = true
                    break
                }
            }
            return false
        }

        // MARK: block starts

        func rest() -> ArraySlice<UInt8> { b[nextNonspace..<lineEnd] }

        func blockStart(_ i: Int, _ container: Node) -> Int {
            switch i {
            case 0: // block quote
                guard !indented, peek(nextNonspace) == 0x3E else { return 0 }
                advanceNextNonspace(); advance(1, columns: false)
                if Self.spaceOrTab(peek(offset)) { advance(1, columns: true) }
                closeUnmatched()
                addChild(.quote)
                return 1
            case 1: // ATX heading
                guard !indented, let h = atx(nextNonspace) else { return 0 }
                advanceNextNonspace()
                closeUnmatched()
                let n = addChild(.heading)
                if h.0 < h.1 { n.lines = [h] }
                offset = lineEnd
                return 2
            case 2: // fenced code
                guard !indented, let f = fenceOpen(nextNonspace) else { return 0 }
                closeUnmatched()
                let n = addChild(.fenced)
                n.fenceChar = f.char; n.fenceLength = f.len; n.fenceOffset = indent; n.fenceStart = nextNonspace; n.lang = f.lang
                advanceNextNonspace(); advance(f.len, columns: false)
                return 2
            case 3: // HTML block
                guard !indented, peek(nextNonspace) == 0x3C else { return 0 }
                let lazyPara = !allClosed && !blank && tip.kind == .paragraph
                guard let t = htmlStart(nextNonspace, allow7: container.kind != .paragraph && !lazyPara) else { return 0 }
                closeUnmatched()
                let n = addChild(.html)
                n.htmlType = t
                return 2
            case 4: // setext heading
                guard !indented, container.kind == .paragraph, setext(nextNonspace) else { return 0 }
                closeUnmatched()
                var lines = container.lines
                var defs: [String] = []
                while !lines.isEmpty, let (label, used) = Defs.parse(b, lines) { defs.append(label); lines.removeFirst(used) }
                guard !lines.isEmpty else { return 0 }
                container.kind = .heading
                container.lines = lines
                container.defs += defs
                finalize(container)
                offset = lineEnd
                return 2
            case 5: // thematic break
                guard !indented, isRule(nextNonspace) else { return 0 }
                closeUnmatched()
                addChild(.rule)
                finalize(tip)
                offset = lineEnd
                return 2
            case 6: // list item
                guard !indented || container.kind == .list, let m = listMarker(container) else { return 0 }
                closeUnmatched()
                if tip.kind != .list || tip.ordered != m.ordered || tip.delim != m.delim || tip.bullet != m.bullet {
                    let l = addChild(.list)
                    l.ordered = m.ordered; l.delim = m.delim; l.bullet = m.bullet
                }
                let item = addChild(.item)
                item.ordered = m.ordered; item.markerOffset = m.markerOffset; item.padding = m.padding
                return 1
            case 7: // indented code
                guard indented, tip.kind != .paragraph, !blank else { return 0 }
                advance(4, columns: true)
                closeUnmatched()
                addChild(.indented)
                return 2
            default: return 0
            }
        }

        func atx(_ s: Int) -> (Int, Int)? {
            var i = s
            while i < lineEnd, b[i] == 0x23 { i += 1 }
            guard i - s >= 1, i - s <= 6, i == lineEnd || Self.spaceOrTab(b[i]) else { return nil }
            var cs = i
            while Self.spaceOrTab(peek(cs)) { cs += 1 }
            var ce = lineEnd
            while ce > cs, Self.spaceOrTab(b[ce - 1]) { ce -= 1 }
            var h = ce
            while h > cs, b[h - 1] == 0x23 { h -= 1 }
            if h < ce, h == cs || Self.spaceOrTab(b[h - 1]) {
                ce = h
                while ce > cs, Self.spaceOrTab(b[ce - 1]) { ce -= 1 }
            }
            return (cs, ce)
        }

        func fenceOpen(_ s: Int) -> (char: UInt8, len: Int, lang: String)? {
            let c = b[s]
            guard c == 0x60 || c == 0x7E else { return nil }
            var i = s
            while i < lineEnd, b[i] == c { i += 1 }
            guard i - s >= 3 else { return nil }
            let info = b[i..<lineEnd]
            if c == 0x60, info.contains(0x60) { return nil }
            let decoded = Inline.unescape(Array(info))
            let word = decoded.split(whereSeparator: { $0.isWhitespace }).first.map(String.init) ?? ""
            return (c, i - s, word.lowercased())
        }

        func setext(_ s: Int) -> Bool {
            let c = b[s]
            guard c == 0x3D || c == 0x2D else { return false }
            var i = s
            while i < lineEnd, b[i] == c { i += 1 }
            while Self.spaceOrTab(peek(i)) { i += 1 }
            return i >= lineEnd
        }

        func isRule(_ s: Int) -> Bool {
            let c = b[s]
            guard c == 0x2A || c == 0x2D || c == 0x5F else { return false }
            var n = 0
            for i in s..<lineEnd {
                if b[i] == c { n += 1 } else if !Self.spaceOrTab(b[i]) { return false }
            }
            return n >= 3
        }

        struct Marker { var ordered: Bool; var delim: UInt8; var bullet: UInt8; var markerOffset: Int; var padding: Int }
        func listMarker(_ container: Node) -> Marker? {
            if indent >= 4 { return nil }
            let s = nextNonspace
            let c = b[s]
            var m = Marker(ordered: false, delim: 0, bullet: 0, markerOffset: indent, padding: 0)
            var len = 0
            if c == 0x2D || c == 0x2B || c == 0x2A {
                m.bullet = c; len = 1
            } else if c >= 0x30 && c <= 0x39 {
                var i = s
                while i < lineEnd, i - s < 9, b[i] >= 0x30 && b[i] <= 0x39 { i += 1 }
                guard i < lineEnd, b[i] == 0x2E || b[i] == 0x29 else { return nil }
                if container.kind == .paragraph, !(i - s == 1 && c == 0x31) { return nil }
                m.ordered = true; m.delim = b[i]; len = i - s + 1
            } else { return nil }
            let after = peek(s + len)
            guard after == nil || Self.spaceOrTab(after) else { return nil }
            if container.kind == .paragraph {
                var k = s + len
                while Self.spaceOrTab(peek(k)) { k += 1 }
                if k >= lineEnd { return nil }
            }
            advanceNextNonspace()
            advance(len, columns: true)
            let startCol = column, startOff = offset
            repeat { advance(1, columns: true) } while column - startCol < 5 && Self.spaceOrTab(peek(offset))
            let blankItem = peek(offset) == nil
            let spaces = column - startCol
            if spaces >= 5 || spaces < 1 || blankItem {
                m.padding = len + 1
                column = startCol; offset = startOff; partialTab = false
                if Self.spaceOrTab(peek(offset)) { advance(1, columns: true) }
            } else {
                m.padding = len + spaces
            }
            return m
        }

        static let htmlNames: Set<String> = ["address", "article", "aside", "base", "basefont", "blockquote", "body", "caption", "center", "col", "colgroup", "dd", "details", "dialog", "dir", "div", "dl", "dt", "fieldset", "figcaption", "figure", "footer", "form", "frame", "frameset", "h1", "h2", "h3", "h4", "h5", "h6", "head", "header", "hr", "html", "iframe", "legend", "li", "link", "main", "menu", "menuitem", "nav", "noframes", "ol", "optgroup", "option", "p", "param", "search", "section", "summary", "table", "tbody", "td", "tfoot", "th", "thead", "title", "tr", "track", "ul"]

        func lower(_ s: Int, _ e: Int) -> [UInt8] { b[s..<e].map { $0 >= 0x41 && $0 <= 0x5A ? $0 + 32 : $0 } }
        func has(_ l: [UInt8], _ p: String) -> Bool { l.starts(with: Array(p.utf8)) }

        func htmlStart(_ s: Int, allow7: Bool) -> Int? {
            let l = lower(s, lineEnd)
            for t in ["<script", "<pre", "<style", "<textarea"] where has(l, t) {
                let k = t.utf8.count
                if k == l.count || l[k] == 0x20 || l[k] == 0x09 || l[k] == 0x3E { return 1 }
            }
            if has(l, "<!--") { return 2 }
            if has(l, "<?") { return 3 }
            if l.count > 2, l[0] == 0x3C, l[1] == 0x21, isAlpha(l[2]) { return 4 }
            if has(l, "<![cdata[") { return 5 }
            var i = 1
            if i < l.count, l[i] == 0x2F { i += 1 }
            var j = i
            while j < l.count, isAlnum(l[j]) { j += 1 }
            if j > i, isAlpha(l[i]), Self.htmlNames.contains(String(decoding: l[i..<j], as: UTF8.self)) {
                if j == l.count || l[j] == 0x20 || l[j] == 0x09 || l[j] == 0x3E || (l[j] == 0x2F && j + 1 < l.count && l[j + 1] == 0x3E) { return 6 }
            }
            if allow7, let e = Inline.htmlTag(Array(b[s..<lineEnd]), 0), e > 0 {
                var k = s + e
                while Self.spaceOrTab(peek(k)) { k += 1 }
                if k >= lineEnd { return 7 }
            }
            return nil
        }

        func htmlEnds(_ t: Int, from s: Int) -> Bool {
            let l = String(decoding: lower(min(s, lineEnd), lineEnd), as: UTF8.self)
            switch t {
            case 1: return ["</script>", "</pre>", "</style>", "</textarea>"].contains { l.contains($0) }
            case 2: return l.contains("-->")
            case 3: return l.contains("?>")
            case 4: return l.contains(">")
            case 5: return l.contains("]]>")
            default: return false
            }
        }

        /// A GFM table: the paragraph's one line is the head, this line the delimiter row with as many cells.
        func tableStart(_ p: Node) -> Bool {
            let s = nextNonspace
            guard indent <= 3, b[s..<lineEnd].contains(0x7C) || p.lines[0].0 < p.lines[0].1 && b[p.lines[0].0..<p.lines[0].1].contains(0x7C) else { return false }
            let delim = cells(s, lineEnd)
            guard !delim.isEmpty else { return false }
            for (a, z) in delim {
                var i = a, e = z
                if i < e, b[i] == 0x3A { i += 1 }
                if e > i, b[e - 1] == 0x3A { e -= 1 }
                guard e > i, b[i..<e].allSatisfy({ $0 == 0x2D }) else { return false }
            }
            let head = cells(p.lines[0].0, p.lines[0].1)
            guard head.count == delim.count else { return false }
            p.kind = .table
            p.cols = head.count
            p.rows = [head]
            p.lines = []
            return true
        }

        /// A table row's cells: split at "|" (not "\\|"), the outer pipes and the spaces around each cell dropped.
        func cells(_ s0: Int, _ e0: Int) -> [(Int, Int)] {
            var s = s0, e = e0
            while e > s, Self.spaceOrTab(b[e - 1]) { e -= 1 }
            while s < e, Self.spaceOrTab(b[s]) { s += 1 }
            if s < e, b[s] == 0x7C { s += 1 }
            if e > s, b[e - 1] == 0x7C, !(e - 2 >= s && b[e - 2] == 0x5C) { e -= 1 }
            var out: [(Int, Int)] = []
            var start = s, i = s
            while i <= e {
                if i == e || (b[i] == 0x7C && !(i > s && b[i - 1] == 0x5C)) {
                    var cs = start, ce = i
                    while cs < ce, Self.spaceOrTab(b[cs]) { cs += 1 }
                    while ce > cs, Self.spaceOrTab(b[ce - 1]) { ce -= 1 }
                    out.append((cs, ce))
                    start = i + 1
                }
                i += 1
            }
            return out
        }
    }
}

// MARK: - Content of a paragraph or a heading

extension ResMD {
    /// A block's lines as one text joined by line breaks, and where each byte of it is in the source.
    struct Content {
        var c: [UInt8] = []
        var at: [Int] = []
        init(_ b: [UInt8], _ lines: [(Int, Int)]) {
            for (k, (s, e)) in lines.enumerated() {
                if k > 0 { c.append(0x0A); at.append(lines[k - 1].1) }
                for i in s..<e { c.append(b[i]); at.append(i) }
            }
            at.append(lines.last?.1 ?? 0)
        }
        /// The source range of content bytes s..<e.
        func src(_ s: Int, _ e: Int) -> (Int, Int) { e > s ? (at[s], at[e - 1] + 1) : (at[s], at[s]) }
    }

    /// Reference definitions at a paragraph's start.
    enum Defs {
        /// The first definition: its label (normalized) and how many lines it takes.
        static func parse(_ b: [UInt8], _ lines: [(Int, Int)]) -> (String, Int)? {
            let k = Content(b, lines)
            let c = k.c
            var i = 0
            guard i < c.count, c[i] == 0x5B, let lbl = Inline.label(c, i) else { return nil }
            let label = Inline.normal(Array(c[(i + 1)..<(lbl - 1)]))
            i = lbl
            guard i < c.count, c[i] == 0x3A, !label.isEmpty else { return nil }
            i += 1
            i = Inline.spnl(c, i)
            guard let (_, de) = Inline.dest(c, i), de > i || (i < c.count && c[i] == 0x3C) else { return nil }
            i = de
            let beforeTitle = i
            var j = Inline.spnl(c, i)
            var end: Int? = nil
            if j > beforeTitle, let te = Inline.title(c, j) {
                j = te
                while j < c.count, c[j] == 0x20 || c[j] == 0x09 { j += 1 }
                if j >= c.count || c[j] == 0x0A { end = j }
            }
            if end == nil {
                j = beforeTitle
                while j < c.count, c[j] == 0x20 || c[j] == 0x09 { j += 1 }
                if j >= c.count || c[j] == 0x0A { end = j } else { return nil }
            }
            let used = c[0..<end!].filter { $0 == 0x0A }.count + 1
            return (label, min(used, lines.count))
        }
    }
}

// MARK: - Inline

extension ResMD {
    struct Inline {
        enum P {
            case text(Int, Int, String, Bool)
            case code(Int, Int, String)
            case bracket(image: Bool, at: Int)
            case linkStart((Int, Int)?), linkEnd, imageStart, imageEnd
            case html, soft, hard
            /// An emphasis or strikethrough run: its char, what is left of it (content bytes), its first length.
            case delim(UInt8, Int, Int, Int)
            case gone
        }
        struct Delim { var piece: Int; var char: UInt8; var canOpen: Bool; var canClose: Bool; var active = true }
        struct Bracket { var piece: Int; var at: Int; var image: Bool; var active = true; var after: Bool; var delims: Int }

        let k: Content
        let defs: Set<String>
        let src: [UInt8]
        var p: [P] = []
        var delims: [Delim] = []
        var brackets: [Bracket] = []

        init(_ b: [UInt8], _ lines: [(Int, Int)], _ defs: Set<String>) {
            var ls = lines
            // The last line's ending spaces are not content.
            if var l = ls.last {
                while l.1 > l.0, b[l.1 - 1] == 0x20 || b[l.1 - 1] == 0x09 { l.1 -= 1 }
                ls[ls.count - 1] = l
            }
            k = Content(b, ls)
            self.defs = defs
            src = b
        }

        var c: [UInt8] { k.c }

        static func run(_ b: [UInt8], _ lines: [(Int, Int)], _ defs: Set<String>) -> [Event] {
            var x = Inline(b, lines, defs)
            return x.events()
        }

        mutating func events() -> [Event] {
            parse()
            var out: [Event] = []
            for x in p {
                switch x {
                case let .text(s, e, v, w):
                    let (a, z) = k.src(s, e)
                    out.append(.text(a, z, v, w))
                case let .code(s, e, v):
                    let (a, z) = k.src(s, e)
                    out.append(.code(a, z, v))
                case let .linkStart(d): out.append(.start(.link(dest: d)))
                case .linkEnd: out.append(.end(.link(dest: nil)))
                case .imageStart: out.append(.start(.image))
                case .imageEnd: out.append(.end(.image))
                case .html: out.append(.inlineHTML)
                case .soft: out.append(.softBreak)
                case .hard: out.append(.hardBreak)
                case let .delim(_, s, e, _) where e > s:
                    let (a, z) = k.src(s, e)
                    out.append(.text(a, z, String(decoding: c[s..<e], as: UTF8.self), true))
                default: break
                }
            }
            return out
        }

        static func isPunct(_ c: UInt8) -> Bool { (c >= 0x21 && c <= 0x2F) || (c >= 0x3A && c <= 0x40) || (c >= 0x5B && c <= 0x60) || (c >= 0x7B && c <= 0x7E) }

        mutating func text(_ s: Int, _ e: Int) {
            guard e > s else { return }
            p.append(.text(s, e, String(decoding: c[s..<e], as: UTF8.self), true))
        }

        mutating func parse() {
            var i = 0
            var run = 0
            let n = c.count
            func special(_ x: UInt8) -> Bool { x == 0x0A || x == 0x5C || x == 0x60 || x == 0x26 || x == 0x3C || x == 0x21 || x == 0x5B || x == 0x5D || x == 0x2A || x == 0x5F || x == 0x7E }
            while i < n {
                let x = c[i]
                if !special(x) { i += 1; continue }
                text(run, i)
                switch x {
                case 0x0A:
                    // Ending spaces of the line before: a hard break with two or more.
                    var sp = 0
                    if case let .text(s, e, _, w)? = p.last {
                        var z = e
                        while z > s, c[z - 1] == 0x20 { z -= 1; sp += 1 }
                        p.removeLast()
                        if z > s { p.append(.text(s, z, String(decoding: c[s..<z], as: UTF8.self), w)) }
                    }
                    p.append(sp >= 2 ? .hard : .soft)
                    i += 1
                case 0x5C:
                    if i + 1 < n, Self.isPunct(c[i + 1]) {
                        p.append(.text(i, i + 2, String(UnicodeScalar(c[i + 1])), false))
                        i += 2
                    } else if i + 1 < n, c[i + 1] == 0x0A {
                        p.append(.hard); i += 2
                    } else { p.append(.text(i, i + 1, "\\", true)); i += 1 }
                case 0x60:
                    var j = i
                    while j < n, c[j] == 0x60 { j += 1 }
                    let len = j - i
                    var q = j, close: Int? = nil
                    while q < n {
                        if c[q] == 0x60 {
                            var r = q
                            while r < n, c[r] == 0x60 { r += 1 }
                            if r - q == len { close = q; break }
                            q = r
                        } else { q += 1 }
                    }
                    if let cl = close {
                        var body = String(decoding: c[j..<cl], as: UTF8.self).replacingOccurrences(of: "\n", with: " ")
                        if body.count >= 2, body.hasPrefix(" "), body.hasSuffix(" "), body.contains(where: { $0 != " " }) {
                            body = String(body.dropFirst().dropLast())
                        }
                        p.append(.code(i, cl + len, body))
                        i = cl + len
                    } else {
                        p.append(.text(i, j, String(repeating: "\u{60}", count: len), true))
                        i = j
                    }
                case 0x26:
                    if let (e, v) = Self.entity(c, i) { p.append(.text(i, e, v, false)); i = e } else { p.append(.text(i, i + 1, "&", true)); i += 1 }
                case 0x3C:
                    if let e = Self.autolink(c, i) {
                        let uri = Self.isURIAutolink(c, i, e)
                        p.append(.linkStart(uri ? k.src(i + 1, e - 1) : nil))
                        p.append(.text(i + 1, e - 1, String(decoding: c[(i + 1)..<(e - 1)], as: UTF8.self), true))
                        p.append(.linkEnd)
                        i = e
                    } else if let e = Self.htmlTag(c, i) {
                        p.append(.html); i = e
                    } else { p.append(.text(i, i + 1, "<", true)); i += 1 }
                case 0x21:
                    if i + 1 < n, c[i + 1] == 0x5B {
                        brackets.append(Bracket(piece: p.count, at: i + 2, image: true, after: i > 0 && c[i - 1] == 0x5D, delims: delims.count))
                        p.append(.bracket(image: true, at: i + 2))
                        i += 2
                    } else { p.append(.text(i, i + 1, "!", true)); i += 1 }
                case 0x5B:
                    brackets.append(Bracket(piece: p.count, at: i + 1, image: false, after: i > 0 && c[i - 1] == 0x5D, delims: delims.count))
                    p.append(.bracket(image: false, at: i + 1))
                    i += 1
                case 0x5D:
                    i = closeBracket(i)
                default: // * _ ~
                    var j = i
                    while j < n, c[j] == x { j += 1 }
                    let (o, cl) = flanking(i, j, x)
                    if x == 0x7E && j - i > 2 {
                        p.append(.text(i, j, String(decoding: c[i..<j], as: UTF8.self), true))
                    } else {
                        delims.append(Delim(piece: p.count, char: x, canOpen: o, canClose: cl))
                        p.append(.delim(x, i, j, j - i))
                    }
                    i = j
                }
                run = i
            }
            text(run, n)
            emphasis(bottom: 0)
            // Brackets never closed: their "[" or "![" is text.
            for (idx, piece) in p.enumerated() {
                if case let .bracket(image, at) = piece { p[idx] = Self.bracketText(at, image) }
            }
        }

        /// The "[" or "![" of a bracket that made no link, as text.
        static func bracketText(_ at: Int, _ image: Bool) -> P {
            let s = at - (image ? 2 : 1)
            return .text(s, at, image ? "![" : "[", true)
        }

        mutating func closeBracket(_ i: Int) -> Int {
            guard let bi = brackets.lastIndex(where: { _ in true }) else { p.append(.text(i, i + 1, "]", true)); return i + 1 }
            let br = brackets[bi]
            if !br.active {
                brackets.remove(at: bi)
                p[br.piece] = Self.bracketText(br.at, br.image)
                p.append(.text(i, i + 1, "]", true))
                return i + 1
            }
            var after = i + 1
            var dest: (Int, Int)? = nil
            var matched = false
            var isInline = false
            var destText = ""
            let n = c.count
            // An inline link: (dest "title").
            if after < n, c[after] == 0x28 {
                var j = Self.spnl(c, after + 1)
                if let (ds, de) = Self.dest(c, j) {
                    let raw = Array(c[ds..<de])
                    var str = raw
                    if j < n, c[j] == 0x3C { str = Array(c[(ds + 1)..<(de - 1)]) }
                    j = de
                    let beforeTitle = j
                    j = Self.spnl(c, j)
                    if j > beforeTitle, let te = Self.title(c, j) { j = Self.spnl(c, te) } else { j = Self.spnl(c, beforeTitle) }
                    if j < n, c[j] == 0x29 {
                        matched = true; isInline = true
                        destText = Self.unescape(str)
                        after = j + 1
                    }
                }
            }
            if !matched {
                var label: String? = nil
                var j = after
                if j < n, c[j] == 0x5B, let e = Self.label(c, j) {
                    if e - j > 2 { label = Self.normal(Array(c[(j + 1)..<(e - 1)])) } else if !br.after { label = Self.normal(Array(c[br.at..<i])) }
                    j = e
                } else if !br.after {
                    label = Self.normal(Array(c[br.at..<i]))
                    j = after
                }
                if let l = label, !l.isEmpty, defs.contains(l) { matched = true; after = j }
            }
            guard matched else {
                brackets.remove(at: bi)
                p[br.piece] = Self.bracketText(br.at, br.image)
                p.append(.text(i, i + 1, "]", true))
                return i + 1
            }
            let start = br.at - (br.image ? 2 : 1)
            if isInline, !br.image, !destText.isEmpty {
                dest = inlineDest(k.src(start, after), destText)
            }
            if isInline, br.image { dest = nil }
            emphasis(bottom: br.delims)
            p[br.piece] = br.image ? .imageStart : .linkStart(dest)
            p.append(br.image ? .imageEnd : .linkEnd)
            brackets.remove(at: bi)
            if !br.image { for j in brackets.indices where !brackets[j].image { brackets[j].active = false } }
            return after
        }

        /// The address of an inline link in the source, as written (Rust \`inline_dest\`).
        func inlineDest(_ r: (Int, Int), _ dest: String) -> (Int, Int)? {
            // The source is the whole text's bytes; Content keeps only the block's, so read through the map.
            let src = Array(sourceBytes(r))
            let d = Array(dest.utf8)
            guard !d.isEmpty, src.count >= d.count else { return nil }
            var p = 0
            while p + d.count <= src.count {
                if Array(src[p..<(p + d.count)]) == d {
                    var bb = p
                    while bb > 0, src[bb - 1] == 0x3C { bb -= 1 }
                    while bb > 0, [0x20, 0x09, 0x0A, 0x0D].contains(src[bb - 1]) { bb -= 1 }
                    var a = p + d.count
                    while a < src.count, src[a] == 0x3E { a += 1 }
                    if bb >= 2, src[bb - 2] == 0x5D, src[bb - 1] == 0x28, a < src.count, [0x20, 0x09, 0x0A, 0x0D, 0x29].contains(src[a]) {
                        return (r.0 + p, r.0 + p + d.count)
                    }
                    p += 1
                } else { p += 1 }
            }
            return nil
        }

        func sourceBytes(_ r: (Int, Int)) -> ArraySlice<UInt8> { src[r.0..<r.1] }

        func flanking(_ s: Int, _ e: Int, _ ch: UInt8) -> (Bool, Bool) {
            let before = s > 0 ? Self.scalarBefore(c, s) : " "
            let after = e < c.count ? Self.scalarAt(c, e) : " "
            let ws = { (u: Unicode.Scalar) in u.properties.isWhitespace }
            let pu = { (u: Unicode.Scalar) in u.isASCII ? Self.isPunct(UInt8(u.value)) : Self.uniPunct(u) }
            let left = !ws(after) && (!pu(after) || ws(before) || pu(before))
            let right = !ws(before) && (!pu(before) || ws(after) || pu(after))
            if ch == 0x5F { return (left && (!right || pu(before)), right && (!left || pu(after))) }
            return (left, right)
        }

        static func uniPunct(_ u: Unicode.Scalar) -> Bool {
            switch u.properties.generalCategory {
            case .connectorPunctuation, .dashPunctuation, .openPunctuation, .closePunctuation, .initialPunctuation, .finalPunctuation, .otherPunctuation, .mathSymbol, .currencySymbol, .modifierSymbol, .otherSymbol: return true
            default: return false
            }
        }

        static func scalarAt(_ c: [UInt8], _ i: Int) -> Unicode.Scalar {
            var it = c[i...].makeIterator()
            var d = UTF8()
            if case let .scalarValue(u) = d.decode(&it) { return u }
            return " "
        }
        static func scalarBefore(_ c: [UInt8], _ i: Int) -> Unicode.Scalar {
            var s = i - 1
            while s > 0, c[s] & 0xC0 == 0x80 { s -= 1 }
            return scalarAt(c, s)
        }

        /// Matches emphasis and strikethrough marks above \`bottom\` (an index in \`delims\`); the marks used are no text.
        mutating func emphasis(bottom: Int) {
            guard bottom < delims.count else { return }
            var ci = bottom
            while ci < delims.count {
                let cl = delims[ci]
                guard cl.active, cl.canClose, case let .delim(ch, cs, ce, corig) = p[cl.piece], ce > cs else { ci += 1; continue }
                var found: Int? = nil
                var oi = ci - 1
                while oi >= bottom {
                    let op = delims[oi]
                    if op.active, op.canOpen, op.char == ch, case let .delim(_, os, oe, oorig) = p[op.piece], oe > os {
                        if ch == 0x7E {
                            if oe - os == ce - cs { found = oi; break }
                        } else {
                            let odd = (op.canClose || cl.canOpen) && (oorig + corig) % 3 == 0 && !(oorig % 3 == 0 && corig % 3 == 0)
                            if !odd { found = oi; break }
                        }
                    }
                    oi -= 1
                }
                guard let o = found, case let .delim(_, os, oe, oorig) = p[delims[o].piece] else { ci += 1; continue }
                let use = ch == 0x7E ? ce - cs : (oe - os >= 2 && ce - cs >= 2 ? 2 : 1)
                p[delims[o].piece] = .delim(ch, os, oe - use, oorig)
                p[cl.piece] = .delim(ch, cs + use, ce, corig)
                for j in (o + 1)..<ci { delims[j].active = false }
                if ce - cs - use == 0 { ci += 1 }
            }
            for j in bottom..<delims.count { delims[j].active = false }
        }

        // MARK: pieces of syntax (over content bytes)

        static func spnl(_ c: [UInt8], _ i0: Int) -> Int {
            var i = i0, nl = false
            while i < c.count {
                if c[i] == 0x20 || c[i] == 0x09 { i += 1 } else if c[i] == 0x0A, !nl { nl = true; i += 1 } else { break }
            }
            return i
        }

        /// A link label "[…]" at i: the index after its "]".
        static func label(_ c: [UInt8], _ i: Int) -> Int? {
            guard i < c.count, c[i] == 0x5B else { return nil }
            var j = i + 1
            while j < c.count, j - i <= 1000 {
                switch c[j] {
                case 0x5C: j += 2; continue
                case 0x5B: return nil
                case 0x5D: return j + 1
                default: j += 1
                }
            }
            return nil
        }

        /// A link destination at i: its bytes (with the angle brackets when it has them).
        static func dest(_ c: [UInt8], _ i: Int) -> (Int, Int)? {
            if i < c.count, c[i] == 0x3C {
                var j = i + 1
                while j < c.count {
                    if c[j] == 0x5C, j + 1 < c.count, isPunct(c[j + 1]) { j += 2; continue }
                    if c[j] == 0x3E { return (i, j + 1) }
                    if c[j] == 0x3C || c[j] == 0x0A { return nil }
                    j += 1
                }
                return nil
            }
            var j = i, depth = 0
            while j < c.count {
                let x = c[j]
                if x == 0x5C, j + 1 < c.count, isPunct(c[j + 1]) { j += 2; continue }
                if x == 0x28 { depth += 1; if depth > 32 { return nil } } else if x == 0x29 { if depth == 0 { break }; depth -= 1 } else if x <= 0x20 || x == 0x7F { break }
                j += 1
            }
            if depth != 0 { return nil }
            return (i, j)
        }

        static func title(_ c: [UInt8], _ i: Int) -> Int? {
            guard i < c.count else { return nil }
            let open = c[i]
            let close: UInt8
            switch open { case 0x22: close = 0x22; case 0x27: close = 0x27; case 0x28: close = 0x29; default: return nil }
            var j = i + 1
            while j < c.count {
                if c[j] == 0x5C, j + 1 < c.count, isPunct(c[j + 1]) { j += 2; continue }
                if c[j] == close { return j + 1 }
                if open == 0x28, c[j] == 0x28 { return nil }
                j += 1
            }
            return nil
        }

        /// A label as a reference: case folded, its whitespace one space.
        static func normal(_ l: [UInt8]) -> String {
            String(decoding: l, as: UTF8.self).split(whereSeparator: { $0.isWhitespace }).joined(separator: " ").lowercased()
        }

        /// Escapes and entities read.
        static func unescape(_ s: [UInt8]) -> String {
            var out: [UInt8] = []
            var i = 0
            while i < s.count {
                if s[i] == 0x5C, i + 1 < s.count, isPunct(s[i + 1]) { out.append(s[i + 1]); i += 2; continue }
                if s[i] == 0x26, let (e, v) = entity(s, i) { out += Array(v.utf8); i = e; continue }
                out.append(s[i]); i += 1
            }
            return String(decoding: out, as: UTF8.self)
        }

        static let named: [String: String] = ["amp": "&", "lt": "<", "gt": ">", "quot": "\"", "apos": "'", "nbsp": "\u{A0}", "copy": "©", "reg": "®", "hellip": "…", "mdash": "—", "ndash": "–", "laquo": "«", "raquo": "»", "middot": "·", "times": "×", "rarr": "→", "larr": "←"]

        /// An entity at i ("&amp;", "&#35;", "&#x22;"): where it ends and what it reads. A name is any of letters and
        /// digits (the spec's list is longer than the one read here: an unknown name reads as itself).
        static func entity(_ c: [UInt8], _ i: Int) -> (Int, String)? {
            var j = i + 1
            guard j < c.count else { return nil }
            if c[j] == 0x23 {
                j += 1
                var hex = false
                if j < c.count, c[j] == 0x78 || c[j] == 0x58 { hex = true; j += 1 }
                let s = j
                while j < c.count, j - s < (hex ? 6 : 7), hex ? isHex(c[j]) : (c[j] >= 0x30 && c[j] <= 0x39) { j += 1 }
                guard j > s, j < c.count, c[j] == 0x3B else { return nil }
                let v = UInt32(String(decoding: c[s..<j], as: UTF8.self), radix: hex ? 16 : 10) ?? 0xFFFD
                let u = Unicode.Scalar(v == 0 ? 0xFFFD : v) ?? "\u{FFFD}"
                return (j + 1, String(u))
            }
            let s = j
            guard isAlpha(c[j]) else { return nil }
            while j < c.count, j - s < 32, isAlnum(c[j]) { j += 1 }
            guard j - s >= 2, j < c.count, c[j] == 0x3B else { return nil }
            let name = String(decoding: c[s..<j], as: UTF8.self)
            return (j + 1, named[name] ?? "&\(name);")
        }

        static func isHex(_ x: UInt8) -> Bool { (x >= 0x30 && x <= 0x39) || (x >= 0x41 && x <= 0x46) || (x >= 0x61 && x <= 0x66) }

        /// An autolink at i ("<scheme:…>" or "<a@b.c>"): the index after its ">".
        static func autolink(_ c: [UInt8], _ i: Int) -> Int? {
            if let e = uriEnd(c, i) { return e }
            // An email address.
            var j = i + 1
            let local = Set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.!#$%&'*+/=?^_\u{60}{|}~-".utf8)
            while j < c.count, local.contains(c[j]) { j += 1 }
            guard j > i + 1, j < c.count, c[j] == 0x40 else { return nil }
            j += 1
            var parts = 0
            while true {
                let s = j
                while j < c.count, isAlnum(c[j]) || c[j] == 0x2D { j += 1 }
                guard j > s, j - s <= 63, c[s] != 0x2D, c[j - 1] != 0x2D else { return nil }
                parts += 1
                if j < c.count, c[j] == 0x2E { j += 1; continue }
                break
            }
            guard j < c.count, c[j] == 0x3E else { return nil }
            return j + 1
        }

        static func uriEnd(_ c: [UInt8], _ i: Int) -> Int? {
            var j = i + 1
            guard j < c.count, isAlpha(c[j]) else { return nil }
            let s = j
            while j < c.count, isAlnum(c[j]) || c[j] == 0x2B || c[j] == 0x2E || c[j] == 0x2D { j += 1 }
            guard j - s >= 2, j - s <= 32, j < c.count, c[j] == 0x3A else { return nil }
            j += 1
            while j < c.count, c[j] > 0x20, c[j] != 0x3C, c[j] != 0x3E { j += 1 }
            guard j < c.count, c[j] == 0x3E else { return nil }
            return j + 1
        }

        static func isURIAutolink(_ c: [UInt8], _ i: Int, _ e: Int) -> Bool { uriEnd(c, i) == e }

        /// An inline HTML tag at i (open or closing tag, comment, processing instruction, declaration, CDATA): the
        /// index after it.
        static func htmlTag(_ c: [UInt8], _ i: Int) -> Int? {
            let n = c.count
            guard i + 1 < n, c[i] == 0x3C else { return nil }
            func find(_ s: [UInt8], from: Int) -> Int? {
                var j = from
                while j + s.count <= n { if Array(c[j..<(j + s.count)]) == s { return j + s.count }; j += 1 }
                return nil
            }
            let x = c[i + 1]
            if x == 0x21 {
                if i + 3 < n, c[i + 2] == 0x2D, c[i + 3] == 0x2D {
                    if i + 4 < n, c[i + 4] == 0x3E { return i + 5 }
                    if i + 5 < n, c[i + 4] == 0x2D, c[i + 5] == 0x3E { return i + 6 }
                    return find(Array("-->".utf8), from: i + 4)
                }
                if Array(c[(i + 2)..<min(n, i + 9)]) == Array("[CDATA[".utf8) { return find(Array("]]>".utf8), from: i + 9) }
                if i + 2 < n, isAlpha(c[i + 2]) { return find([0x3E], from: i + 3) }
                return nil
            }
            if x == 0x3F { return find(Array("?>".utf8), from: i + 2) }
            func ws(_ j: Int) -> Int { var j = j; while j < n, c[j] == 0x20 || c[j] == 0x09 || c[j] == 0x0A { j += 1 }; return j }
            if x == 0x2F {
                var j = i + 2
                guard j < n, isAlpha(c[j]) else { return nil }
                while j < n, isAlnum(c[j]) || c[j] == 0x2D { j += 1 }
                j = ws(j)
                return j < n && c[j] == 0x3E ? j + 1 : nil
            }
            guard isAlpha(x) else { return nil }
            var j = i + 2
            while j < n, isAlnum(c[j]) || c[j] == 0x2D { j += 1 }
            while true {
                let w = ws(j)
                if w < n, c[w] == 0x3E { return w + 1 }
                if w + 1 < n, c[w] == 0x2F, c[w + 1] == 0x3E { return w + 2 }
                guard w > j, w < n, isAlpha(c[w]) || c[w] == 0x5F || c[w] == 0x3A else { return nil }
                j = w
                while j < n, isAlnum(c[j]) || c[j] == 0x5F || c[j] == 0x2E || c[j] == 0x3A || c[j] == 0x2D { j += 1 }
                let v = ws(j)
                if v < n, c[v] == 0x3D {
                    var q = ws(v + 1)
                    guard q < n else { return nil }
                    if c[q] == 0x22 || c[q] == 0x27 {
                        let quote = c[q]
                        q += 1
                        while q < n, c[q] != quote { q += 1 }
                        guard q < n else { return nil }
                        j = q + 1
                    } else {
                        let s = q
                        while q < n, c[q] > 0x20, !"\"'=<>\u{60}".utf8.contains(c[q]) { q += 1 }
                        guard q > s else { return nil }
                        j = q
                    }
                }
            }
        }
    }
}

func isAlpha(_ c: UInt8) -> Bool { (c >= 0x41 && c <= 0x5A) || (c >= 0x61 && c <= 0x7A) }
func isAlnum(_ c: UInt8) -> Bool { isAlpha(c) || (c >= 0x30 && c <= 0x39) }
