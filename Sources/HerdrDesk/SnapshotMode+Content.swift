import AppKit
import HerdrKit
import SwiftUI

/// Offscreen checks for the content area (conversation, composer). One entry point; `HerdrDeskApp` dispatches the flags.
@MainActor enum ContentTests {
    static let flags = LaunchGuard.content
    /// Pictures go here; `--light` runs a test in the light appearance with its own folder.
    static var dir = "build/snapshots-content"

    /// Runs the test named by the first matching flag; false when none matches.
    static func run(_ args: [String]) -> Bool {
        if args.contains("--light") { NSApp.appearance = NSAppearance(named: .aqua); dir = "build/snapshots-content-light" }
        if args.contains("--test-textsize") { Task { await textSize() }; return true }
        if args.contains("--test-sendfail") { Task { await sendFail() }; return true }
        if args.contains("--test-loadstate") { Task { await loadState() }; return true }
        if args.contains("--test-glyph") { Task { await glyph() }; return true }
        if args.contains("--test-panebars") { Task { await paneBars() }; return true }
        if args.contains("--test-surfaces") { Task { await surfaces() }; return true }
        if args.contains("--test-search") { Task { await search() }; return true }
        if args.contains("--test-scrolltop") { Task { await scrollTop() }; return true }
        if args.contains("--test-shortcuts") { Task { await shortcuts() }; return true }
        if args.contains("--test-open-latest-log") { Task { await openLatestLog(args) }; return true }
        if args.contains("--test-open-latest-live") { Task { await openLatestLive(args) }; return true }   // before --test-open-latest
        if args.contains("--test-open-latest") { Task { await openLatest() }; return true }
        if args.contains("--test-conversation-churn") { Task { await conversationChurn(args) }; return true }
        if args.contains("--test-busy-tab") { Task { await busyTab() }; return true }
        if args.contains("--test-legacy-look") { Task { await legacyLook() }; return true }
        if args.contains("--bench-conversation") { Task { await benchConversation(args) }; return true }
        if args.contains("--test-question-live") { Task { await questionLive(args) }; return true }   // before --test-question
        if args.contains("--test-question") { Task { await question() }; return true }
        if args.contains("--test-screencolumn") { Task { await screenColumn() }; return true }
        if args.contains("--test-scroll-real") {
            // Not in `flags`: the one content check with a real window (owner's leave), so the app keeps its WindowGroup window.
            NSApplication.shared.setActivationPolicy(.regular)
            Task { await ScrollReal.run(dir: ProcessInfo.processInfo.environment["FS_REAL_DIR"] ?? "build/fullscreen-real/scroll") }
            return true
        }
        return false
    }

    static func offscreen(_ view: some View, _ size: NSSize) -> (NSWindow, NSView) {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        return (win, host)
    }
    private static func textView(in v: NSView) -> NSTextView? {
        if let t = v as? NSTextView { return t }
        for c in v.subviews { if let t = textView(in: c) { return t } }
        return nil
    }

    /// Window-server picture of an offscreen window (the only capture that shows SwiftUI layers).
    static func capture(_ win: NSWindow, to path: String) {
        typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else { return }
        let f = unsafeBitCast(sym, to: Fn.self)
        guard let img = f(.null, 8, UInt32(win.windowNumber), 1)?.takeRetainedValue() else { return }
        try? FileManager.default.createDirectory(atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        try? NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
    }

    private static func esc(_ win: NSWindow) { key(win, "\u{1b}", 53) }
    private static func key(_ win: NSWindow, _ ch: String, _ code: UInt16) {
        for t in [NSEvent.EventType.keyDown, .keyUp] {
            if let e = NSEvent.keyEvent(with: t, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: win.windowNumber,
                                        context: nil, characters: ch, charactersIgnoringModifiers: ch, isARepeat: false, keyCode: code) { win.sendEvent(e) }
        }
    }
    static func all<T: NSView>(_ type: T.Type, in v: NSView) -> [T] { ((v as? T).map { [$0] } ?? []) + v.subviews.flatMap { all(type, in: $0) } }

    /// Split tab (agent + shell) on the preview store: label and composer are bars over the conversation, an unfocused
    /// pane's composer takes focus with one click, inline pane rename, the notice toast. Nothing is sent to herdr: the
    /// rename part runs with input off and ends with Esc.
    static func paneBars() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("panebars-test: \(n) \(ok ? "ok" : "FAIL")") }
        let model = AppModel(store: .preview)
        guard let ws = model.store.workspaces.first, let tab = ws.tabs.first(where: { $0.isSplit }),
              let agentPane = tab.panes.first(where: { $0.agent != nil }), let shellPane = tab.panes.first(where: { $0.agent == nil }) else {
            print("panebars-test: preview has no agent + shell split"); NSApp.terminate(nil); return
        }
        model.select(tab: tab, in: ws)
        model.focus(shellPane, in: tab)
        let (win, host) = offscreen(ZStack { WindowSurface(glass: .off); TabScreen() }.environment(model).environment(\.flatGlass, true), NSSize(width: 900, height: 620))
        try? await Task.sleep(for: .milliseconds(800))
        capture(win, to: "\(dir)/panebars-split.png")
        // The conversation scroll view runs under both bars and reserves them as insets.
        let convo = all(NSScrollView.self, in: host).max { $0.bounds.height < $1.bounds.height }
        step("conversation reserves the composer bar as a bottom inset (\(convo?.contentInsets.bottom ?? -1))", (convo?.contentInsets.bottom ?? 0) > 40)
        let composers = all(NSTextView.self, in: host).filter { $0.isEditable }
        step("unfocused agent pane shows the real composer (\(composers.count) field)", composers.count == 1)
        if let tv = composers.first {
            // The covering button takes the click (hit test at the field reaches SwiftUI, not the text view).
            let p = tv.convert(NSPoint(x: tv.bounds.midX, y: tv.bounds.midY), to: nil)
            step("click on the unfocused field goes to the reply button, not the text view", !(host.hitTest(host.superview!.convert(p, from: nil)) is NSTextView))
            // Press it the way VoiceOver does (an offscreen, non-key window swallows a synthetic first click).
            var axWindows: CFTypeRef?
            _ = AXUIElementCopyAttributeValue(AXUIElementCreateApplication(getpid()), kAXWindowsAttribute as CFString, &axWindows)
            try? await Task.sleep(for: .milliseconds(500))
            var reply: NSObject?
            func walk(_ e: Any, _ depth: Int) {
                guard reply == nil, depth < 40, let el = e as? NSAccessibilityElementProtocol & NSObject else { return }
                if (el.value(forKey: "accessibilityLabel") as? String) == "Reply to \(agentPane.agent!.name)" { reply = el; return }
                for c in (el.value(forKey: "accessibilityChildren") as? [Any]) ?? [] { walk(c, depth + 1) }
            }
            walk(host, 0)
            step("reply button is in the accessibility tree", reply != nil)
            _ = reply?.perform(#selector(NSAccessibilityButton.accessibilityPerformPress))
            try? await Task.sleep(for: .milliseconds(400))
            step("one click focuses the pane", model.focusedPaneID(in: tab) == agentPane.id)
            step("and puts the cursor in its field", win.firstResponder === all(NSTextView.self, in: host).first { $0.isEditable })
            capture(win, to: "\(dir)/panebars-focused.png")
        }
        // Inline rename, input off so no path can reach herdr.
        AppDefaults.store.set(false, forKey: AppSettings.allowInputKey)
        model.paneRenameDraft = "Planner"; model.renamingPane = agentPane
        try? await Task.sleep(for: .milliseconds(500))
        let editor = win.firstResponder as? NSTextView
        step("rename opens an inline field with the name, focused (\(editor?.string ?? "nil"))", editor?.string == "Planner" && editor?.isFieldEditor == true)
        capture(win, to: "\(dir)/panebars-rename.png")
        esc(win)
        try? await Task.sleep(for: .milliseconds(300))
        step("Esc closes the field", model.renamingPane == nil)
        model.paneRenameDraft = ""; model.renamingPane = agentPane
        try? await Task.sleep(for: .milliseconds(500))
        key(win, "\r", 36)
        try? await Task.sleep(for: .milliseconds(300))
        step("Return closes the field (input off: nothing sent)", model.renamingPane == nil)
        model.notice = "Could not close the pane: herdr is not running"
        try? await Task.sleep(for: .milliseconds(600))
        capture(win, to: "\(dir)/panebars-toast.png")
        win.orderOut(nil)
        // Single-pane tab: no label, but the label shows while renaming.
        if let single = model.store.workspaces.flatMap({ w in w.tabs.map { (w, $0) } }).first(where: { !$0.1.isSplit && $0.1.panes.first?.agent != nil }) {
            model.notice = nil
            model.select(tab: single.1, in: single.0)
            let (w2, h2) = offscreen(ZStack { WindowSurface(glass: .off); TabScreen() }.environment(model).environment(\.flatGlass, true), NSSize(width: 700, height: 400))
            try? await Task.sleep(for: .milliseconds(600))
            let before = all(NSTextField.self, in: h2).count
            model.paneRenameDraft = ""; model.renamingPane = single.1.panes.first
            try? await Task.sleep(for: .milliseconds(500))
            step("single pane: rename shows the label with a field (\(before) -> \(all(NSTextField.self, in: h2).count) text fields)", (w2.firstResponder as? NSTextView)?.isFieldEditor == true)
            capture(w2, to: "\(dir)/panebars-single-rename.png")
            esc(w2)
            try? await Task.sleep(for: .milliseconds(300))
            step("single pane: Esc closes it", model.renamingPane == nil)
            w2.orderOut(nil)
        }
        AppDefaults.store.removeObject(forKey: AppSettings.allowInputKey)
        print("panebars-test: pictures in \(dir)/panebars-*.png")
        print("panebars-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Pixels of a saved capture, in the host view's points (top-left origin).
    static func pixels(_ path: String, _ host: NSView) -> ((CGFloat, CGFloat) -> NSColor?)? {
        guard let rep = NSImage(contentsOfFile: path).flatMap({ $0.tiffRepresentation }).flatMap(NSBitmapImageRep.init(data:)) else { return nil }
        let k = CGFloat(rep.pixelsWide) / host.bounds.width
        let titleBar = CGFloat(rep.pixelsHigh) - host.bounds.height * k   // the capture includes the title bar
        return { x, y in rep.colorAt(x: Int(x * k), y: Int(titleBar + y * k))?.usingColorSpace(.sRGB) }
    }
    static func same(_ p: NSColor?, _ q: NSColor?) -> Bool {
        guard let p, let q else { return false }
        return abs(p.redComponent - q.redComponent) < 0.01 && abs(p.greenComponent - q.greenComponent) < 0.01 && abs(p.blueComponent - q.blueComponent) < 0.01
    }
    static func hex(_ c: NSColor?) -> String { c.map { String(format: "%02X%02X%02X", Int($0.redComponent * 255), Int($0.greenComponent * 255), Int($0.blueComponent * 255)) } ?? "nil" }

    /// One surface tone: shell and agent panes on the same window surface, the real terminal view transparent with no
    /// scroller; read-only composer buttons look disabled.
    static func surfaces() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("surfaces-test: \(n) \(ok ? "ok" : "FAIL")") }
        // 1. Split tab (agent + shell card), flat mode.
        let model = AppModel(store: .preview)
        if let ws = model.store.workspaces.first, let tab = ws.tabs.first(where: { $0.isSplit }) { model.select(tab: tab, in: ws) }
        let size = NSSize(width: 900, height: 620)
        let (win, host) = offscreen(ZStack { WindowSurface(glass: .off); TabScreen() }.environment(model).environment(\.flatGlass, true), size)
        try? await Task.sleep(for: .milliseconds(800))
        let path = "\(dir)/surfaces-split.png"
        capture(win, to: path)
        if let px = pixels(path, host) {
            let agent = px(30, 200), shell = px(860, 580), label = px(860, 450)
            step("shell pane = agent pane tone (\(hex(agent)) / \(hex(shell)) / label row \(hex(label)))", same(agent, shell) && same(agent, label))
        } else { step("split picture", false) }
        win.orderOut(nil)
        // 2. The real terminal view (no session) on the window surface: blank cells show the surface, no scroller.
        let container = NSView(frame: NSRect(origin: .zero, size: NSSize(width: 500, height: 300)))
        let surface = NSHostingView(rootView: WindowSurface(glass: .off)); surface.frame = container.bounds
        let tv = PasteTerminalView(frame: NSRect(x: 14, y: 8, width: 472, height: 284))
        container.addSubview(surface); container.addSubview(tv)
        let tw = OffscreenWindow(contentRect: container.frame, styleMask: [.titled], backing: .buffered, defer: false)
        tw.contentView = container; tw.setFrameOrigin(NSPoint(x: -3000, y: -3000)); tw.orderFrontRegardless()
        let dark = tw.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        TerminalNSView.applyColors(tv, dark ? .dark : .light)
        tv.feed(text: "$ ls\r\nREADME.md  Sources\r\n$ ")
        try? await Task.sleep(for: .milliseconds(600))
        let tpath = "\(dir)/surfaces-terminal.png"
        capture(tw, to: tpath)
        step("terminal scroller hidden", tv.scroller.map { $0.isHidden } ?? true)
        if let px = pixels(tpath, container) {
            // Flipped from the container's bottom-left: (5, 150) is outside the terminal, (400, 200) a blank cell.
            let outside = px(5, 150), blank = px(400, 250), edge = px(480, 150)
            step("blank terminal cells = window surface (\(hex(outside)) / \(hex(blank)) / right edge \(hex(edge)))", same(outside, blank) && same(outside, edge))
        } else { step("terminal picture", false) }
        tw.orderOut(nil)
        // 3. Composer buttons: read-only icons are dimmer than the writable ones.
        func iconBrightness(readOnly: Bool, _ name: String) async -> CGFloat {
            let (w, h) = offscreen(ZStack { WindowSurface(glass: .off); ComposerView(agentName: "tk", readOnly: readOnly, onSend: { _ in }) }, NSSize(width: 500, height: 110))
            try? await Task.sleep(for: .milliseconds(500))
            let p = "\(dir)/surfaces-composer-\(name).png"
            capture(w, to: p); w.orderOut(nil)
            guard let px = pixels(p, h) else { return -1 }
            // Bottom row: the + at the left and the mic left of the send button; brightest contrast to the background.
            let bg = px(250, 105)
            var best: CGFloat = 0
            for x in stride(from: 10, to: 490, by: 1) { for y in stride(from: 70, to: 104, by: 1) {
                guard let c = px(CGFloat(x), CGFloat(y)), let b = bg else { continue }
                best = max(best, abs(c.brightnessComponent - b.brightnessComponent))
            } }
            return best
        }
        let rw = await iconBrightness(readOnly: false, "writable"), ro = await iconBrightness(readOnly: true, "readonly")
        step("read-only +, mic, send look disabled (contrast \(String(format: "%.2f", ro)) vs \(String(format: "%.2f", rw)) writable)", rw > 0 && ro < rw * 0.6)
        print("surfaces-test: pictures in \(dir)/surfaces-*.png")
        print("surfaces-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Search screen in the real window chrome: system search field in the toolbar, focused; the first hit is
    /// selected; Return opens it; no hits = the system "No Results" view.
    static func search() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("search-test: \(n) \(ok ? "ok" : "FAIL")") }
        let model = AppModel(store: .preview)
        let size = NSSize(width: 1200, height: 700)
        let host = NSHostingView(rootView: RootView(flat: true).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(1.5))
        model.screen = .search
        try? await Task.sleep(for: .seconds(1))
        func fields(_ v: NSView) -> [NSSearchField] { ((v as? NSSearchField).map { [$0] } ?? []) + v.subviews.flatMap(fields) }
        let frame = win.contentView?.superview ?? host
        let sf = fields(frame).first
        step("system search field in the window (\(sf.map { "\(Int($0.frame.width)) pt wide" } ?? "none"))", sf != nil)
        // isPresented focuses the field in a key window; this offscreen window is never key (owner to check: ⌘K
        // puts the cursor in the field). Put focus there by hand to test typing and Return.
        let autoFocused = (win.firstResponder as? NSTextView)?.isFieldEditor == true
        print("search-test: focus by isPresented in this non-key window: \(autoFocused) (real window: owner to check)")
        if !autoFocused, let sf { win.makeFirstResponder(sf) }
        let editor = win.firstResponder as? NSTextView
        step("search field takes keyboard focus", editor?.isFieldEditor == true && (editor?.delegate as? NSSearchField) === sf)
        step("hits for \"mob\": \(SearchView.hits("mob", in: model.store.workspaces).map(\.tab.label))", SearchView.hits("mob", in: model.store.workspaces).map(\.tab.label) == ["Mobile view"])
        // Agent names match: the last two parts of the "Core runtime" tab's agent name (e.g. "api-dev"), whatever the preview calls it.
        let coreQuery = model.store.workspaces.flatMap(\.tabs).first { $0.label == "Core runtime" }?.primaryAgent?.name
            .split(separator: "-").suffix(2).joined(separator: "-") ?? "?"
        step("agent names match (\"\(coreQuery)\")", SearchView.hits(coreQuery, in: model.store.workspaces).map(\.tab.label) == ["Core runtime"])
        if let editor { editor.insertText("mob", replacementRange: editor.selectedRange()) }
        try? await Task.sleep(for: .milliseconds(600))
        capture(win, to: "\(dir)/search-mob.png")
        step("typing reaches the model (\(model.searchText))", model.searchText == "mob")
        // The selected hit's title keeps full contrast in a window that is not key (the list default dims it).
        if let px = pixels("\(dir)/search-mob.png", host), let bg = px(size.width - 200, 300) {
            var most: CGFloat = 0
            for x in stride(from: 235, to: 310, by: 1) { for y in stride(from: 30, to: 80, by: 1) {
                if let c = px(CGFloat(x), CGFloat(y)) { most = max(most, abs(c.brightnessComponent - bg.brightnessComponent)) }
            } }
            step("selected hit title has full contrast (\(String(format: "%.2f", most)))", most > 0.6)
        } else { step("search picture", false) }
        key(win, "\r", 36)
        try? await Task.sleep(for: .milliseconds(500))
        step("Return opens the selected hit (screen \(model.screen), tab \(model.tab?.label ?? "-"))", model.screen == .tab && model.tab?.label == "Mobile view")
        model.screen = .search; model.searchText = "zzzz"
        try? await Task.sleep(for: .milliseconds(800))
        capture(win, to: "\(dir)/search-empty.png")
        step("no hits for zzzz", SearchView.hits("zzzz", in: model.store.workspaces).isEmpty)
        model.searchText = ""
        // Every screen sits on the window surface: an empty point of the sidebar = an empty point of the content.
        for (name, screen) in [("tab", Screen.tab), ("inbox", .inbox), ("overview", .overview), ("search", .search)] {
            if screen == .tab, let ws = model.store.workspaces.first, let t = ws.tabs.first(where: { $0.isSplit }) { model.select(tab: t, in: ws) }
            model.screen = screen
            try? await Task.sleep(for: .milliseconds(700))
            let p = "\(dir)/screen-\(name).png"
            capture(win, to: p)
            if let px = pixels(p, host) {
                let side = px(100, size.height - 20), content = px(size.width - 40, size.height - 20), mid = px(size.width / 2 + 120, size.height - 20)
                step("\(name): sidebar = content tone (\(hex(side)) / \(hex(content)) / \(hex(mid)))", same(side, content) && same(side, mid))
            } else { step("\(name): picture", false) }
        }
        print("search-test: pictures in \(dir)/search-{mob,empty}.png")
        print("search-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Reading size S and XL: bubbles, a table, code, chips, the approval card, the pane label and the composer with
    /// two lines, captured per size. Table rows must fit their text and not overlap; the composer field grows with it.
    static func textSize() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("textsize-test: \(n) \(ok ? "ok" : "FAIL")") }
        let md = """
        ### Status of the build
        The run on `orchard-web-dev` finished. Three checks are left, and one of them needs a long explanation that wraps over several lines in the bubble.

        | Check | Result | Note |
        |---|---|---|
        | Unit tests | 214 passed | 替代品處理的是成因，還是和公司一樣處理症狀？這段很長，會換行 |
        | UI snapshot | 2 changed | `chrome-windowed.png`, `screen-search.png` |

        ```swift
        let model = AppModel(store: .preview)   // a long line to show how code wraps or scrolls inside the block
        ```
        - [x] Build
        - [ ] Ship
        """
        let items: [ConversationItem] = [
            ConversationItem(id: "u1", kind: .userText("Can you check the build and tell me which snapshot tests changed? Keep it short, I am on my phone.")),
            ConversationItem(id: "a1", kind: .assistantText(md)),
            ConversationItem(id: "t1", kind: .toolChips([ToolChip(symbol: "doc.text", text: "Read 3 files"), ToolChip(symbol: "terminal", text: "Ran swift build")])),
            ConversationItem(id: "p1", kind: .permissionRequest(PermissionRequest(title: "orchard-web-dev wants to run a build", command: "xcodebuild -scheme Mobile -destination 'platform=iOS Simulator,name=iPhone 17' build", toolUseId: "toolu_p1"))),
        ]
        for size in [ReadingSize.s, .xl] {
            AppDefaults.store.set(size.rawValue, forKey: ReadingSize.key)
            TableLayoutProbe.rows = [:]; TableLayoutProbe.misfit = []
            let (win, host) = offscreen(ZStack { WindowSurface(glass: .off)
                ConversationView(items: items, agentName: "orchard-web-dev", liveCard: "toolu_p1", loadState: .ready, hasLog: true) }
                .environment(\.flatGlass, true), NSSize(width: 760, height: 1300))
            try? await Task.sleep(for: .milliseconds(1000))
            capture(win, to: "\(dir)/textsize-\(size.rawValue)-conversation.png")
            let rows = TableLayoutProbe.rows["3x9"] ?? []
            var overlap = false
            for i in rows.indices.dropFirst() where rows[i].y < rows[i - 1].y + rows[i - 1].h - 0.5 { overlap = true }
            step("\(size.rawValue): table rows measured (\(rows.count)), no overlap, every cell fits its text", rows.count == 3 && !overlap && !TableLayoutProbe.misfit.contains("3x9"))
            _ = host; win.orderOut(nil)
            // Split tab: pane label row and the composer with a two-line draft.
            let model = AppModel(store: .preview)
            guard let ws = model.store.workspaces.first, let tab = ws.tabs.first(where: { $0.isSplit }), let agentPane = tab.panes.first(where: { $0.agent != nil }) else { continue }
            model.select(tab: tab, in: ws); model.focus(agentPane, in: tab)
            model.setDraft(ComposerDraft(text: "First line of a reply\nand a second line"), for: agentPane.id)
            let (w2, h2) = offscreen(ZStack { WindowSurface(glass: .off); TabScreen() }.environment(model).environment(\.flatGlass, true), NSSize(width: 900, height: 620))
            try? await Task.sleep(for: .milliseconds(900))
            capture(w2, to: "\(dir)/textsize-\(size.rawValue)-tab.png")
            if let tv = all(NSTextView.self, in: h2).first(where: { $0.isEditable }), let lm = tv.layoutManager, let tc = tv.textContainer, let clip = tv.enclosingScrollView {
                let used = lm.usedRect(for: tc).height + tv.textContainerInset.height * 2
                step("\(size.rawValue): composer shows both lines (text \(Int(used)) pt, field \(Int(clip.frame.height)) pt)", tv.string.contains("\n") && clip.frame.height >= used - 0.5)
            } else { step("\(size.rawValue): composer field", false) }
            w2.orderOut(nil)
        }
        // A short conversation leaves empty space above its first row. The bare view in a titled window gets the
        // default hard top scroll edge, drawn as a full-width line where the content starts. The app shows the
        // conversation only in a pane, with soft edges: no line there.
        func seams(_ path: String, _ h: NSView) -> [Int] {
            guard let px = pixels(path, h) else { return [-1] }
            var out: [Int] = [], prev = px(10, 2)?.brightnessComponent ?? 0
            for y in stride(from: CGFloat(3), to: h.bounds.height - 2, by: 0.5) {
                let b = px(10, y)?.brightnessComponent ?? 0
                if abs(b - prev) > 0.02 { out.append(Int(y)) }; prev = b
            }
            return out
        }
        let short = HerdrStore.preview.conversation(for: nil)
        for (name, soft) in [("bare", false), ("bare-soft", true)] {
            let v = ConversationView(items: short, agentName: "orchard-main-orch", loadState: .ready, hasLog: true)
            let (w, h) = offscreen(ZStack { WindowSurface(glass: .off); if soft { v.softScrollEdges([.top, .bottom]) } else { v } }
                .environment(\.flatGlass, true), NSSize(width: 760, height: 900))
            try? await Task.sleep(for: .milliseconds(800))
            let p = "\(dir)/textsize-short-\(name).png"
            capture(w, to: p); w.orderOut(nil)
            let s = seams(p, h)
            if soft { step("short conversation, soft edges: no line above the content (\(s))", s.isEmpty) }
            else { print("textsize-test: short conversation, bare view, default edges: lines at \(s) (offscreen host only)") }
        }
        let m2 = AppModel(store: .preview)
        if let (ws, t) = m2.store.workspaces.flatMap({ w in w.tabs.map { (w, $0) } }).first(where: { !$0.1.isSplit && $0.1.panes.first?.agent != nil }) {
            m2.select(tab: t, in: ws)
            let (w, h) = offscreen(ZStack { WindowSurface(glass: .off); TabScreen() }.environment(m2).environment(\.flatGlass, true), NSSize(width: 760, height: 900))
            try? await Task.sleep(for: .milliseconds(800))
            let p = "\(dir)/textsize-short-tabscreen.png"
            capture(w, to: p); w.orderOut(nil)
            let s = seams(p, h)
            step("short conversation in a single pane (TabScreen): no line above the content (\(s))", s.isEmpty)
        } else { step("preview has a single agent pane tab", false) }
        AppDefaults.store.removeObject(forKey: ReadingSize.key)
        print("textsize-test: pictures in \(dir)/textsize-*.png")
        print("textsize-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Empty conversation: which view for which load state (no 30 s guess), with a picture of each.
    static func loadState() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("loadstate-test: \(n) \(ok ? "ok" : "FAIL")") }
        let one = [ConversationItem(id: "u1", kind: .userText("hi"))]
        step("items present: list whatever the state", ConversationView.emptyState(items: one, loadState: .loading, hasLog: true) == nil)
        for s in [ConversationLoadState.loading, .ready, .missing, .failed("Permission denied")] {
            step("empty + \(s): shows \(s)", ConversationView.emptyState(items: [], loadState: s, hasLog: true) == s)
        }
        step("no store state, no log: list (old callers)", ConversationView.emptyState(items: [], loadState: nil, hasLog: false) == nil)
        // Copy: a fresh agent (no log until its first message) is told what to do; only a worked agent without a log is an error.
        func copy(_ s: ConversationLoadState, _ a: AgentStatus?) -> ConversationView.EmptyCopy { ConversationView.emptyCopy(s, status: a, agentName: "tk") }
        let first = "Send your first message to see the session conversation."
        step("missing + idle: No messages yet, first-message hint, composer focus", copy(.missing, .idle) == .init(title: "No messages yet", detail: first, fresh: true))
        step("missing + no status: same", copy(.missing, nil).fresh && copy(.missing, nil).title == "No messages yet")
        step("missing + working/blocked/done: No conversation log + Show Terminal",
             [AgentStatus.working, .blocked, .done].allSatisfy { copy(.missing, $0).terminal && copy(.missing, $0).title == "No conversation log" })
        step("ready: always No messages yet; idle hint, working 'is starting', done 'has not written'",
             [AgentStatus.idle, .working, .blocked, .done].allSatisfy { copy(.ready, $0).title == "No messages yet" && !copy(.ready, $0).terminal }
             && copy(.ready, .idle).detail == first && copy(.ready, .working).detail.contains("is starting") && copy(.ready, .done).detail.contains("has not written"))
        var fresh = 0
        let cases: [(String, ConversationLoadState, AgentStatus?)] = [("loading", .loading, nil), ("ready", .ready, .working), ("missing", .missing, .idle),
                                                                       ("missing-worked", .missing, .done), ("failed", .failed("Permission denied"), nil)]
        for (name, s, status) in cases {
            // On the app's own window surface (glass Off, as the app draws it with Reduce Transparency), nothing else behind.
            let (win, host) = offscreen(ZStack { WindowSurface(glass: .off); ConversationView(items: [], agentName: "orchard-main-orch", agentStatus: status, onFresh: { fresh += 1 }, loadState: s) }, NSSize(width: 640, height: 360))
            try? await Task.sleep(for: .milliseconds(name == "loading" ? 900 : 400))
            let path = "\(dir)/empty-\(name).png"
            capture(win, to: path)
            // One tone: a point in the window corner and one just above the icon must match (no panel behind the empty state).
            if let rep = NSImage(contentsOfFile: path).flatMap({ $0.tiffRepresentation }).flatMap(NSBitmapImageRep.init(data:)) {
                let k = CGFloat(rep.pixelsWide) / host.bounds.width
                func px(_ x: CGFloat, _ y: CGFloat) -> NSColor? { rep.colorAt(x: Int(x * k), y: Int(y * k))?.usingColorSpace(.sRGB) }
                let a = px(20, 340), b = px(320, 70), c = px(140, 180)
                func same(_ p: NSColor?, _ q: NSColor?) -> Bool { guard let p, let q else { return false }; return abs(p.redComponent - q.redComponent) < 0.01 && abs(p.greenComponent - q.greenComponent) < 0.01 && abs(p.blueComponent - q.blueComponent) < 0.01 }
                step("\(name): no extra panel behind the empty state (corner = top-centre = left-middle)", same(a, b) && same(a, c))
            } else { step("\(name): picture", false) }
            win.orderOut(nil)
        }
        step("fresh agent's empty state asks the pane to focus the composer (\(fresh) call)", fresh == 1)
        print("loadstate-test: pictures in \(dir)/empty-{loading,ready,missing,missing-worked,failed}.png")
        print("loadstate-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Working glyph: animated by AppKit (window captures 0.3 s apart differ), CPU close to the same window with idle glyphs.
    static func glyph() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("glyph-test: \(n) \(ok ? "ok" : "FAIL")") }
        func cpu() -> Double { var u = rusage(); getrusage(RUSAGE_SELF, &u); return Double(u.ru_utime.tv_sec + u.ru_stime.tv_sec) + Double(u.ru_utime.tv_usec + u.ru_stime.tv_usec) / 1e6 }
        func row(_ s: AgentStatus) -> some View {
            VStack(alignment: .leading, spacing: 10) {
                // The picture shows all four; the still control row has no working glyph at all.
                HStack(spacing: 14) { ForEach(s == .working ? [AgentStatus.working, .blocked, .done, .idle] : [.blocked, .done, .idle], id: \.self) { StatusGlyph(status: $0, size: 13) } }
                HStack(spacing: 6) { ForEach(0..<12, id: \.self) { _ in StatusGlyph(status: s, size: 13) } }
            }.padding(16).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).background(WindowSurface(glass: .off))
        }
        func measure(_ s: AgentStatus, picture: String?) async -> (Double, Bool) {
            let (win, _) = offscreen(row(s), NSSize(width: 320, height: 90))
            try? await Task.sleep(for: .seconds(1))
            if let picture { capture(win, to: picture) }
            // Several samples: under load the window server can skip recompositing an offscreen window for a frame.
            let a = windowPNG(win); var moved = false
            for _ in 0..<6 where !moved { try? await Task.sleep(for: .milliseconds(170)); moved = a != nil && windowPNG(win) != a }
            let c0 = cpu(), t0 = Date()
            try? await Task.sleep(for: .seconds(5))
            let pct = (cpu() - c0) / Date().timeIntervalSince(t0) * 100
            win.orderOut(nil)
            return (pct, moved)
        }
        let (busy, moving) = await measure(.working, picture: "\(dir)/glyphs.png")
        let (idle, still) = await measure(.idle, picture: nil)
        step("12 working glyphs animate (window captures differ)", moving)
        step("control: idle glyphs do not (captures equal)", !still)
        step(String(format: "CPU 12 working %.1f %% vs idle glyphs %.1f %% (<= +1.5)", busy, idle), busy - idle <= 1.5)
        print("glyph-test: picture \(dir)/glyphs.png")
        print("glyph-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    static func windowPNG(_ win: NSWindow) -> Data? {
        typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else { return nil }
        let f = unsafeBitCast(sym, to: Fn.self)
        guard let img = f(.null, 8, UInt32(win.windowNumber), 1)?.takeRetainedValue() else { return nil }
        return NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:])
    }

    /// A message herdr does not take comes back into the composer with a note; a later success clears the note;
    /// text typed while a send was failing is never overwritten.
    static func sendFail() async {
        let probe = PlaceholderProbe()
        var accept = false, delivered: [String] = []
        let (win, host) = offscreen(ComposerView(agentName: "testagent", onSend: { _ in }, probe: probe, deliver: { msg in
            try? await Task.sleep(for: .milliseconds(200))
            if accept { delivered.append(msg) }
            return accept
        }), NSSize(width: 700, height: 220))
        try? await Task.sleep(for: .seconds(1))
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("sendfail-test: \(n) \(ok ? "ok" : "FAIL")") }
        guard let tv = textView(in: host) else { print("sendfail-test: no text view"); NSApp.terminate(nil); return }
        win.makeFirstResponder(tv)
        tv.insertText("hello herdr", replacementRange: NSRange(location: NSNotFound, length: 0))
        try? await Task.sleep(for: .milliseconds(300))
        tv.insertNewline(nil)
        try? await Task.sleep(for: .milliseconds(80))
        step("Return clears the composer at once (text \"\(probe.text)\")", probe.text.isEmpty && tv.string.isEmpty)
        try? await Task.sleep(for: .milliseconds(600))
        step("failed send: text comes back (\"\(tv.string)\")", tv.string == "hello herdr" && probe.text == "hello herdr")
        step("failed send: note shown (\(probe.error ?? "none"))", probe.error?.hasPrefix("Not sent") == true)
        accept = true
        tv.insertNewline(nil)
        try? await Task.sleep(for: .milliseconds(700))
        step("retry with Return delivers it (\(delivered)) and clears the note", delivered == ["hello herdr"] && probe.error == nil && tv.string.isEmpty)
        // New text typed while a failing send is in flight stays.
        accept = false
        tv.insertText("first", replacementRange: NSRange(location: NSNotFound, length: 0))
        try? await Task.sleep(for: .milliseconds(300))
        tv.insertNewline(nil)
        try? await Task.sleep(for: .milliseconds(50))
        tv.insertText("second", replacementRange: NSRange(location: NSNotFound, length: 0))
        try? await Task.sleep(for: .milliseconds(700))
        step("text typed meanwhile is kept (\"\(tv.string)\"), note still shown", tv.string == "second" && probe.error != nil)
        print("sendfail-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Scrolling screens under the toolbar (dense preview, short window so each one scrolls): layout only. At the top the
    /// content starts below the toolbar (the window's own toolbar band: 52 pt on 26, 38 pt on 15); it scrolls.
    /// 26: the system soft scroll edge is set on its scroll area (pocket views: soft mask shown, hard pocket hidden). The
    /// content fades by that edge alone, and a window-server capture of this offscreen window does not render it
    /// (scrolled pictures show text under the island; .hard does render), so the band's ink is printed for information,
    /// not checked. Real window: owner to check. Before 26 (no soft edge): the window surface covers the band, so
    /// scrolled content adds no ink there.
    static func scrollTop() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("scrolltop-test: \(n) \(ok ? "ok" : "FAIL")") }
        let model = AppModel(store: .previewDense)
        let size = NSSize(width: 1200, height: 400)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(1.5))
        let panel = ChromeProbe.panel
        for (name, screen) in [("overview", Screen.overview), ("inbox", .inbox), ("search", .search)] {
            model.screen = screen
            try? await Task.sleep(for: .seconds(1.5))
            // The screen's scroll area: the tallest one reaching right of the sidebar panel (it may run under the panel).
            let frame = win.contentView?.superview ?? host
            func top(_ s: NSScrollView) -> CGRect { var r = s.convert(s.bounds, to: nil); r.origin.y = win.frame.height - r.maxY; return r }
            guard let s = all(NSScrollView.self, in: frame).filter({ top($0).maxX > panel.maxX + 200 && $0.bounds.height > 200 }).max(by: { $0.bounds.height < $1.bounds.height }) else {
                step("\(name): scroll area found", false); continue
            }
            let inset = s.contentInsets.top, r = top(s)
            let band = win.frame.height - win.contentLayoutRect.height   // title bar and toolbar, above the layout rect
            step("\(name): content starts below the toolbar (inset \(Int(inset)), origin \(Int(s.contentView.bounds.minY)), toolbar \(Int(band)))",
                 band >= 30 && inset >= band - 0.5 && s.contentView.bounds.minY <= -inset + 0.5)
            let a = "\(dir)/scrolltop-\(name)-top.png", b = "\(dir)/scrolltop-\(name)-scrolled.png"
            capture(win, to: a)
            let maxY = (s.documentView?.frame.height ?? 0) + s.contentInsets.bottom - s.contentView.bounds.height
            s.contentView.scroll(to: NSPoint(x: s.contentView.bounds.minX, y: maxY)); s.reflectScrolledClipView(s.contentView)   // keep x: the list starts left of the panel edge
            try? await Task.sleep(for: .seconds(1))
            let moved = s.contentView.bounds.minY + inset
            step("\(name): scrolls (\(Int(moved)) pt)", moved > 30)
            capture(win, to: b)
            // Ink: pixels away from their row's median tone. The band is the toolbar above the island's bottom edge.
            func ink(_ path: String, _ y0: CGFloat, _ y1: CGFloat) -> Int {
                guard let rep = NSBitmapImageRep(data: (try? Data(contentsOf: URL(fileURLWithPath: path))) ?? Data()) else { return -1 }
                let k = CGFloat(rep.pixelsWide) / win.frame.width
                func lum(_ x: Int, _ y: Int) -> CGFloat { guard let c = rep.colorAt(x: x, y: y) else { return 0 }; return 0.3 * c.redComponent + 0.59 * c.greenComponent + 0.11 * c.blueComponent }
                var n = 0
                for y in Int(y0 * k)..<Int(y1 * k) {
                    let xs = Array(Int((max(r.minX, panel.maxX) + 4) * k)..<Int((r.maxX - 16) * k))
                    let row = xs.map { lum($0, y) }, mid = row.sorted()[row.count / 2]
                    n += row.filter { abs($0 - mid) > 0.12 }.count
                }
                return n
            }
            let atTop = ink(a, 4, inset), scrolled = ink(b, 4, inset), below = ink(b, inset + 8, inset + 60)
            if LegacyLook.on {
                step("\(name): no text in the toolbar band when scrolled (ink scrolled vs top: \(scrolled) vs \(atTop) px)", scrolled <= atTop + atTop / 50 + 20)
            } else {
                print("scrolltop-test: \(name): toolbar band ink, scrolled vs top: \(scrolled) vs \(atTop) px (information: captures do not render the soft edge)")
            }
            step("\(name): content below the toolbar is drawn (\(below) ink px)", below > 500)
            func views(_ v: NSView) -> [NSView] { [v] + v.subviews.flatMap(views) }
            let pocket = views(s).filter { String(describing: type(of: $0)).contains("Pocket") }
            func shown(_ n: String) -> Bool? { pocket.first { String(describing: type(of: $0)) == n }.map { !$0.isHidden } }
            if !LegacyLook.on {
                step("\(name): system soft edge active (soft mask shown \(shown("PocketMask").map(String.init) ?? "none"), hard pocket shown \(shown("NSHardPocketView").map(String.init) ?? "none"))",
                     shown("PocketMask") == true && shown("NSHardPocketView") != true)
            }
        }
        print("scrolltop-test: pictures \(dir)/scrolltop-*.png")
        print("scrolltop-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }
}

/// Real window, by the owner's leave only (visible window, Space switch): `FS_REAL_DIR=<dir> HerdrDesk --preview
/// --test-scroll-real` from a test bundle. Does the system soft scroll edge show under the toolbar? A window-server
/// capture of an offscreen window cannot see it. Windowed (this window only), then in full screen (the whole screen:
/// only this app is on that Space): the conversation scrolled under the island, then Overview scrolled to the end.
/// Exits full screen. Watchdog: gone after 14 s.
@MainActor enum ScrollReal {
    static func run(dir: String) async {
        setvbuf(stdout, nil, _IOLBF, 0)
        let t0 = Date()
        func log(_ s: String) { print(String(format: "scroll-real %5.2f: ", Date().timeIntervalSince(t0)) + s) }
        DispatchQueue.global().asyncAfter(deadline: .now() + 14) { print("scroll-real: watchdog exit"); exit(3) }
        var win: NSWindow?
        for _ in 0..<40 where win == nil {
            try? await Task.sleep(for: .milliseconds(100))
            win = NSApp.windows.first { $0.toolbar != nil && $0.isVisible }
        }
        guard let win, let screen = win.screen ?? NSScreen.main else { log("no window"); exit(1) }
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        log("front app \(NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "?")")
        typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
        let grab = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage").map { unsafeBitCast($0, to: Fn.self) }
        func capture(_ name: String, whole: Bool) {
            let main = CGDisplayBounds(CGMainDisplayID())
            var r = screen.frame; r.origin.y = main.height - screen.frame.maxY
            // On-screen windows in a rectangle: the window's frame (windowed) or the whole screen (full screen).
            if !whole { r = win.frame; r.origin.y = main.height - win.frame.maxY }   // the window's own rectangle on screen (it is in front)
            guard let img = grab?(r, 1, 0, 0)?.takeRetainedValue() else { log("capture failed"); return }
            try? NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "\(dir)/\(name).png"))
            log("picture \(dir)/\(name).png")
        }
        func menu(_ title: String) {
            guard let item = NSApp.mainMenu?.items.flatMap({ $0.submenu?.items ?? [] }).first(where: { $0.title == title }), let m = item.menu else { log("no menu item \(title)"); return }
            m.performActionForItem(at: m.index(of: item))
        }
        /// The detail's scroll area: the tallest one reaching well right of the sidebar.
        func area() -> NSScrollView? {
            func all(_ v: NSView) -> [NSScrollView] { ((v as? NSScrollView).map { [$0] } ?? []) + v.subviews.flatMap(all) }
            guard let root = win.contentView?.superview ?? win.contentView else { return nil }
            return all(root).filter { $0.convert($0.bounds, to: nil).maxX > 600 && $0.bounds.height > 300 }.max { $0.bounds.height < $1.bounds.height }
        }
        func scroll(_ s: NSScrollView, to y: CGFloat) {
            s.contentView.scroll(to: NSPoint(x: s.contentView.bounds.minX, y: y)); s.reflectScrolledClipView(s.contentView)
        }
        func shots(_ prefix: String) async {
            menu("Tab 1")
            try? await Task.sleep(for: .milliseconds(500))
            if let s = area() {
                scroll(s, to: max(-s.contentInsets.top, s.contentView.bounds.minY - 200))   // up from the bottom: text under the island
                try? await Task.sleep(for: .milliseconds(400))
                log("\(prefix) conversation: inset \(s.contentInsets.top) origin \(s.contentView.bounds.origin) doc \(s.documentView?.frame.size ?? .zero)")
            } else { log("\(prefix) conversation: no scroll area") }
            capture("\(prefix)-conversation", whole: prefix == "fullscreen")
            menu("Show in Overview")
            try? await Task.sleep(for: .milliseconds(500))
            for (name, title) in [("overview", "Show in Overview"), ("search", "Search…")] {
                if name != "overview" { menu(title); try? await Task.sleep(for: .milliseconds(500)) }
                if let s = area() {
                    let end = (s.documentView?.frame.height ?? 0) + s.contentInsets.bottom - s.contentView.bounds.height
                    if end > s.contentView.bounds.minY { scroll(s, to: end) }   // only when there is more below
                    try? await Task.sleep(for: .milliseconds(400))
                    log("\(prefix) \(name): inset \(s.contentInsets.top) origin \(s.contentView.bounds.origin) scrolled \(Int(s.contentView.bounds.minY + s.contentInsets.top)) pt")
                } else { log("\(prefix) \(name): no scroll area") }
                capture("\(prefix)-\(name)", whole: prefix == "fullscreen")
            }
        }
        NSApp.activate(); win.makeKeyAndOrderFront(nil)
        try? await Task.sleep(for: .seconds(1))   // the window's opening animation ends
        await shots("windowed")
        win.toggleFullScreen(nil)
        for _ in 0..<30 { try? await Task.sleep(for: .milliseconds(100)); if win.styleMask.contains(.fullScreen) { break } }
        try? await Task.sleep(for: .seconds(1.2))
        if win.styleMask.contains(.fullScreen) { await shots("fullscreen") } else { log("full screen failed") }
        win.toggleFullScreen(nil)
        try? await Task.sleep(for: .seconds(1.2))
        log("done, full screen \(win.styleMask.contains(.fullScreen))")
        exit(0)
    }
}
