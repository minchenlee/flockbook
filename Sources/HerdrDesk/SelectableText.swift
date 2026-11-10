import AppKit
import SwiftUI

/// One conversation message as one selectable text view: a drag selects across paragraphs, lists, code and tables;
/// Cmd-C and the context menu copy plain text; the menu's "Copy Message" copies the message's markdown.
///
/// SwiftUI's `.textSelection(.enabled)` puts an AppKit overlay on each Text and re-sets its font on every update, which asks
/// for another layout; a long conversation could lay out forever (100% CPU). Here the text is built once per (text, size)
/// and kept in `MessageContent`; the height is measured once per width; `updateNSView` replaces the text only when the
/// content changed.
struct MessageText: NSViewRepresentable {
    let content: MessageContent
    /// As wide as the text (a bubble); otherwise as wide as offered.
    var fit = false
    /// Folded: this tall at most, the bottom 48 pt fading out.
    var maxHeight: CGFloat? = nil

    /// Calls so far (the bench counts how many rows a change touches).
    static var makes = 0, updates = 0, fits = 0
    func makeNSView(context: Context) -> MessageTextView {
        Self.makes += 1
        let tv = MessageTextView.make()
        tv.load(content)
        tv.fold = maxHeight
        return tv
    }

    func updateNSView(_ tv: MessageTextView, context: Context) {
        Self.updates += 1
        SettingsFocus.cover(tv, context.environment.underSettings)
        if tv.contentID != content.id { tv.load(content) }
        if tv.fold != maxHeight { tv.fold = maxHeight }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: MessageTextView, context: Context) -> CGSize? {
        Self.fits += 1
        let offered = proposal.width ?? .infinity
        let width = offered.isFinite ? max(1, offered) : 100_000
        var s = content.size(width: width, fit: fit || !offered.isFinite)
        if let maxHeight { s.height = min(s.height, maxHeight) }
        return s
    }
}

/// A message's text, built once per (kind, text, size) and shared by every view that shows it.
@MainActor final class MessageContent {
    let id: Int
    let text: NSAttributedString
    private var sizes: [Int: CGSize] = [:]

    private init(_ text: NSAttributedString) {
        Self.next += 1
        id = Self.next
        self.text = text
    }

    private static var next = 0
    private enum Kind: Hashable { case markdown, bubble, plain(mono: CGFloat, error: Bool) }
    private struct Key: Hashable { let kind: Kind; let size: ReadingSize; let text: String }
    private static var store: [Key: (content: MessageContent, used: Int, chars: Int)] = [:]
    private static var uses = 0
    /// Texts kept, and their characters (the source text and the built text, UTF-16). The store used to drop every text
    /// once it held 400: a tab of four panes with 100 rows each passed that on one update, so a new row rebuilt more
    /// than a hundred texts and measured them several hundred times (a 281 ms stall in --test-busy-tab).
    static let keep = 3000
    static let keepChars = 8_000_000
    /// A text larger than this never goes in the store: it lives only while a view holds it.
    static let largeChars = 1_000_000
    private(set) static var chars = 0
    /// Every text still held somewhere (a view on screen holds its own): a text the store dropped, or a large text, is
    /// found here again with the same id, so the view keeps it and does not reload or measure it.
    /// By the key's hash: an entry whose text is gone keeps no text alive. The text holds its own key to compare.
    private final class Held { weak var content: MessageContent?; init(_ c: MessageContent) { content = c } }
    private static var held: [Int: [Held]] = [:]
    private var key: Key?
    /// The message as it was written, its markdown ("Copy Message"); nil for plain text (tool output, a command).
    var source: String? {
        guard let key else { return nil }
        switch key.kind {
        case .markdown, .bubble: return key.text
        case .plain: return nil
        }
    }
    private static var heldPrune = 1024
    /// Texts built and sizes measured so far (checks: a change must not rebuild what is on screen).
    static var builds = 0
    static var measures = 0
    /// Main-thread time spent building and measuring texts (the bench splits a stall by these).
    static var buildTime = Duration.zero, measureTime = Duration.zero

    private static func cached(_ key: Key, _ build: () -> NSAttributedString) -> MessageContent {
        uses += 1
        if let c = store[key] { store[key]!.used = uses; return c.content }
        let h = key.hashValue
        if let c = held[h]?.lazy.compactMap(\.content).first(where: { $0.key == key }) { insert(c, key); return c }
        builds += 1
        let t = ContinuousClock.now, parsed = MarkdownCache.parseTime
        let c = MessageContent(build())
        buildTime += ContinuousClock.now - t - (MarkdownCache.parseTime - parsed)   // a parse inside the build counts as parse
        c.key = key
        if held.count >= heldPrune {
            held = held.compactMapValues { e in let live = e.filter { $0.content != nil }; return live.isEmpty ? nil : live }
            heldPrune = max(1024, held.count * 2)
        }
        held[h, default: []].removeAll { $0.content == nil }
        held[h, default: []].append(Held(c))
        insert(c, key)
        return c
    }

    /// Puts a text in the store, dropping the least recently used down to three quarters of each limit.
    private static func insert(_ c: MessageContent, _ key: Key) {
        let n = key.text.utf16.count + c.text.length
        guard n <= largeChars else { return }
        if store.count >= keep || chars + n > keepChars {
            for (k, e) in store.sorted(by: { $0.value.used < $1.value.used }) {
                guard store.count > keep * 3 / 4 || chars + n > keepChars * 3 / 4 else { break }
                store[k] = nil; chars -= e.chars
            }
        }
        store[key] = (c, uses, n)
        chars += n
    }

    /// Assistant markdown (reading.md): headings, lists, quotes, code blocks, tables.
    static func markdown(_ text: String, size: ReadingSize) -> MessageContent {
        cached(Key(kind: .markdown, size: size, text: text)) { MessageBuilder(size: size).markdown(MarkdownCache.blocks(for: text)) }
    }

    /// A user bubble: inline markdown only.
    static func bubble(_ text: String, size: ReadingSize) -> MessageContent {
        cached(Key(kind: .bubble, size: size, text: text)) { MessageBuilder(size: size).bubble(text) }
    }

    /// Monospaced plain text (tool input and output, a command).
    static func plain(_ text: String, monoSize: CGFloat, error: Bool = false) -> MessageContent {
        cached(Key(kind: .plain(mono: monoSize, error: error), size: .m, text: text)) {
            NSAttributedString(string: text, attributes: [.font: NSFont.monospacedSystemFont(ofSize: monoSize, weight: .regular),
                                                           .foregroundColor: error ? NSColor.systemRed : NSColor.labelColor])
        }
    }

    /// The laid-out size at `width`, measured once per width.
    func size(width: CGFloat, fit: Bool) -> CGSize {
        let key = Int((width * 2).rounded()) * 2 + (fit ? 1 : 0)
        if let s = sizes[key] { return s }
        Self.measures += 1
        let t = ContinuousClock.now
        let s = TextMeasure.size(self, width: width, fit: fit)
        Self.measureTime += ContinuousClock.now - t
        if sizes.count > 16 { sizes.removeAll() }
        sizes[key] = s
        return s
    }
}

/// Lays text out away from the screen, for heights. One shared TextKit 1 stack: tables need `NSTextTable`.
@MainActor private enum TextMeasure {
    static let storage = NSTextStorage()
    static let layout = NSLayoutManager()
    static let container: NSTextContainer = {
        let c = NSTextContainer(size: NSSize(width: 100, height: CGFloat.greatestFiniteMagnitude))
        c.lineFragmentPadding = 0
        layout.addTextContainer(c)
        storage.addLayoutManager(layout)
        return c
    }()

    static func size(_ content: MessageContent, width: CGFloat, fit: Bool) -> CGSize {
        let c = container
        c.size = NSSize(width: width, height: .greatestFiniteMagnitude)
        storage.setAttributedString(content.text)
        layout.ensureLayout(for: c)
        let used = layout.usedRect(for: c)
        probeTables(width: width)
        storage.setAttributedString(NSAttributedString())   // do not keep the last message alive
        return CGSize(width: fit ? min(width, ceil(used.width)) : width, height: ceil(used.height))
    }

    /// Fills `TableLayoutProbe`: each table's row frames, and whether every cell holds its text.
    private static func probeTables(width: CGFloat) {
        var rows: [ObjectIdentifier: (table: NSTextTable, cells: Int, rows: [Int: (y: CGFloat, maxY: CGFloat)], misfit: Bool)] = [:]
        let s = storage.string as NSString
        var i = 0
        while i < s.length {
            let para = s.paragraphRange(for: NSRange(location: i, length: 0))
            i = NSMaxRange(para)
            guard let style = storage.attribute(.paragraphStyle, at: para.location, effectiveRange: nil) as? NSParagraphStyle,
                  let cell = style.textBlocks.last as? NSTextTableBlock else { continue }
            let glyphs = layout.glyphRange(forCharacterRange: para, actualCharacterRange: nil)
            let box = layout.boundsRect(for: cell, glyphRange: glyphs)
            let text = layout.boundingRect(forGlyphRange: glyphs, in: container)
            let id = ObjectIdentifier(cell.table)
            var e = rows[id] ?? (cell.table, 0, [:], false)
            e.cells += 1
            let r = e.rows[cell.startingRow] ?? (box.minY, box.maxY)
            e.rows[cell.startingRow] = (min(r.y, box.minY), max(r.maxY, box.maxY))
            if text.maxY > box.maxY + 0.5 { e.misfit = true }
            rows[id] = e
        }
        for e in rows.values {
            let key = "\(e.table.numberOfColumns)x\(e.cells)"
            let sorted = e.rows.keys.sorted().compactMap { e.rows[$0] }
            let top = sorted.first?.y ?? 0
            TableLayoutProbe.rows[key] = sorted.map { ($0.y - top, $0.maxY - $0.y) }
            if e.misfit { TableLayoutProbe.misfit.insert(key) }
        }
    }
}

extension NSAttributedString.Key {
    /// What a plain-text copy puts in place of this range: list markers, table cell breaks, a code block's label.
    static let messageCopy = NSAttributedString.Key("flockbook.messageCopy")
}

/// Not editable, selectable, no own scrolling: SwiftUI sizes it from `MessageContent`.
final class MessageTextView: NSTextView {
    private(set) var contentID = -1
    /// The text shown: held here, so the text store finds it again while this view is on screen.
    private var content: MessageContent?
    /// Folded height (the rest is cut, the bottom fades), or nil.
    var fold: CGFloat? { didSet { needsLayout = true } }
    private let fade = CAGradientLayer()

    static func make() -> MessageTextView {
        let tv = MessageTextView(usingTextLayoutManager: false)
        tv.isEditable = false
        tv.isSelectable = true
        tv.isRichText = true
        tv.drawsBackground = false
        tv.textContainerInset = .zero
        tv.textContainer?.lineFragmentPadding = 0
        tv.textContainer?.widthTracksTextView = true
        tv.textContainer?.heightTracksTextView = false
        tv.textContainer?.size = NSSize(width: 100, height: CGFloat.greatestFiniteMagnitude)
        tv.isVerticallyResizable = false
        tv.isHorizontallyResizable = false
        tv.isAutomaticLinkDetectionEnabled = false
        tv.usesFontPanel = false
        // No idle-time layout chunks on the main thread. (Non-contiguous layout made the churn check's stalls longer.)
        tv.layoutManager?.backgroundLayoutEnabled = false
        tv.clipsToBounds = true
        tv.linkTextAttributes = [.foregroundColor: ReadingColor.NS.link, .underlineStyle: NSUnderlineStyle.single.rawValue,
                                 .cursor: NSCursor.pointingHand]
        return tv
    }

    func load(_ content: MessageContent) {
        textStorage?.setAttributedString(content.text)
        contentID = content.id
        self.content = content
    }

    // Never scroll the conversation from here. On a resize NSTextView shows a rect of itself through the enclosing scroll
    // view; SwiftUI does not see that scroll, can keep the rows of the old range and leave the viewport blank.
    // (Drag-selection autoscroll goes through autoscroll(with:), not these.)
    override func scrollToVisible(_ rect: NSRect) -> Bool { false }
    override func scrollRangeToVisible(_ range: NSRange) {}

    override func layout() {
        super.layout()
        guard let fold, bounds.height >= fold - 0.5 else { if layer?.mask != nil { layer?.mask = nil }; return }
        wantsLayer = true
        fade.colors = [NSColor.black.cgColor, NSColor.black.cgColor, NSColor.clear.cgColor]
        let h = max(1, bounds.height)
        fade.locations = [0, NSNumber(value: Double(max(0, (h - 48) / h))), 1]
        // A flipped view: location 0 is the top.
        fade.startPoint = CGPoint(x: 0.5, y: isFlipped ? 0 : 1)
        fade.endPoint = CGPoint(x: 0.5, y: isFlipped ? 1 : 0)
        CATransaction.begin(); CATransaction.setDisableActions(true)
        fade.frame = bounds
        layer?.mask = fade
        CATransaction.commit()
    }

    /// The selection as plain text: list markers as "- " / "1. ", table cells split by tabs, code labels left out.
    func plainText(_ range: NSRange) -> String {
        guard let storage = textStorage, range.length > 0 else { return "" }
        let s = storage.string as NSString
        var out = ""
        storage.enumerateAttribute(.messageCopy, in: range) { v, r, _ in
            if let v = v as? String { out += v } else { out += s.substring(with: r).replacingOccurrences(of: "\u{FFFC}", with: "") }
        }
        return out
    }

    /// Copy, drag and Services write plain text only: rich text would bring back code labels and raw list markers.
    override var writablePasteboardTypes: [NSPasteboard.PasteboardType] { [.string] }

    override func writeSelection(to pboard: NSPasteboard, type: NSPasteboard.PasteboardType) -> Bool {
        guard type == .string else { return false }
        let text = selectedRanges.map { plainText($0.rangeValue) }.filter { !$0.isEmpty }.joined(separator: "\n")
        return pboard.setString(text, forType: .string)
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        let copy = menu.addItem(withTitle: "Copy", action: #selector(copy(_:)), keyEquivalent: "")
        copy.target = self
        if let code = codeBlock(at: convert(event.locationInWindow, from: nil)) {
            let item = menu.addItem(withTitle: "Copy Code", action: #selector(copyCode(_:)), keyEquivalent: "")
            item.target = self; item.representedObject = code.code
        }
        if messageMarkdown != nil {
            menu.addItem(withTitle: "Copy Message", action: #selector(copyMessage(_:)), keyEquivalent: "").target = self
        }
        menu.addItem(.separator())
        menu.addItem(withTitle: "Select All", action: #selector(selectAll(_:)), keyEquivalent: "").target = self
        return menu
    }

    private func codeBlock(at p: NSPoint) -> CodeTextBlock? {
        guard let storage = textStorage, storage.length > 0 else { return nil }
        let i = min(characterIndexForInsertion(at: p), storage.length - 1)
        let style = storage.attribute(.paragraphStyle, at: i, effectiveRange: nil) as? NSParagraphStyle
        return style?.textBlocks.first { $0 is CodeTextBlock } as? CodeTextBlock
    }

    @objc private func copyCode(_ sender: NSMenuItem) {
        guard let code = sender.representedObject as? String else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(code, forType: .string)
    }

    /// The whole message as its markdown, as it was written; nil when the text is no message (tool output, a command)
    /// or is empty. "Copy Message" works whatever the selection: Copy needs one, and a selection ends at this text view.
    var messageMarkdown: String? { content?.source.flatMap { $0.isEmpty ? nil : $0 } }

    @objc private func copyMessage(_ sender: Any?) { writeMessage(to: .general) }

    /// Puts the message's markdown on `pboard` as plain text. False: no message.
    @discardableResult func writeMessage(to pboard: NSPasteboard) -> Bool {
        guard let text = messageMarkdown else { return false }
        pboard.clearContents()
        return pboard.setString(text, forType: .string)
    }
}

/// A code block: rounded fill and hairline border; the code it holds for "Copy Code".
final class CodeTextBlock: NSTextBlock {
    var code = ""
    override func drawBackground(withFrame frameRect: NSRect, in controlView: NSView, characterRange charRange: NSRange, layoutManager: NSLayoutManager) {
        let r = frameRect.inset(top: width(for: .margin, edge: .minY), bottom: width(for: .margin, edge: .maxY))
        let path = NSBezierPath(roundedRect: r.insetBy(dx: 0.25, dy: 0.25), xRadius: Radius.card, yRadius: Radius.card)
        ReadingColor.NS.codeFill.setFill(); path.fill()
        ReadingColor.NS.hairline.setStroke(); path.lineWidth = 0.5; path.stroke()
    }
}

/// A block quote: a 3 pt bar on the leading side.
final class QuoteTextBlock: NSTextBlock {
    override func drawBackground(withFrame frameRect: NSRect, in controlView: NSView, characterRange charRange: NSRange, layoutManager: NSLayoutManager) {
        let r = frameRect.inset(top: width(for: .margin, edge: .minY), bottom: width(for: .margin, edge: .maxY))
        ReadingColor.NS.primary(0.16).setFill()
        NSBezierPath(roundedRect: NSRect(x: r.minX, y: r.minY, width: 3, height: r.height), xRadius: 1.5, yRadius: 1.5).fill()
    }
}

/// A thematic break: a 120 pt hairline in the middle of the column.
final class RuleTextBlock: NSTextBlock {
    override func drawBackground(withFrame frameRect: NSRect, in controlView: NSView, characterRange charRange: NSRange, layoutManager: NSLayoutManager) {
        let r = frameRect.inset(top: width(for: .margin, edge: .minY), bottom: width(for: .margin, edge: .maxY))
        ReadingColor.NS.hairline.setFill()
        NSRect(x: r.midX - 60, y: r.midY.rounded(.down), width: 120, height: 1).fill()
    }
}

/// A table: rounded outer border (the cells draw their own fills and row lines).
final class RoundedTextTable: NSTextTable {
    override func drawBackground(withFrame frameRect: NSRect, in controlView: NSView, characterRange charRange: NSRange, layoutManager: NSLayoutManager) {
        let r = frameRect.inset(top: width(for: .margin, edge: .minY), bottom: width(for: .margin, edge: .maxY))
        let path = NSBezierPath(roundedRect: r.insetBy(dx: 0.25, dy: 0.25), xRadius: Radius.card, yRadius: Radius.card)
        ReadingColor.NS.primary(0.14).setStroke(); path.lineWidth = 0.5; path.stroke()
    }
}

private extension NSRect {
    func inset(top: CGFloat, bottom: CGFloat) -> NSRect { NSRect(x: minX, y: minY + top, width: width, height: max(0, height - top - bottom)) }
}
