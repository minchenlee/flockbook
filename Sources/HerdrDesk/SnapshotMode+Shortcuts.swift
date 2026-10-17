import AppKit
import SwiftUI

/// Offscreen check for the ⌘/ keyboard shortcuts panel (`--test-shortcuts`, `--light` for the light pictures).
extension ContentTests {
    static func shortcuts() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("shortcuts-test: \(n) \(ok ? "ok" : "FAIL")") }

        // Glyphs as the system menu draws them.
        let g = ShortcutList.glyphs
        let cases: [(String, NSEvent.ModifierFlags, String)] = [
            ("t", [.command], "⌘T"), ("d", [.command, .shift], "⇧⌘D"), ("D", [.command], "⇧⌘D"),
            (String(Character(UnicodeScalar(NSLeftArrowFunctionKey)!)), [.command, .control, .shift], "⌃⇧⌘←"),
            ("\r", [.command, .shift], "⇧⌘↩"), ("\t", [.control], "⌃⇥"), ("`", [.control], "⌃`"),
            (String(Character(UnicodeScalar(NSDownArrowFunctionKey)!)), [.command, .option], "⌥⌘↓"), ("/", [.command], "⌘/"),
        ]
        let bad = cases.filter { g($0.0, $0.1).joined() != $0.2 }.map { "\($0.2) got \(g($0.0, $0.1).joined())" }
        step("glyphs in ⌃⌥⇧⌘ order (\(cases.count) cases\(bad.isEmpty ? "" : ": " + bad.joined(separator: ", ")))", bad.isEmpty)
        let left = g(String(Character(UnicodeScalar(NSLeftArrowFunctionKey)!)), [.command, .control, .shift]), fnF = g("f", [.function])
        step("one keycap per key: \(left) and \(fnF)", left == ["⌃", "⇧", "⌘", "←"] && fnF == ["fn", "F"])
        // A Services submenu (other apps' actions) never reaches the list.
        let saved = NSApp.servicesMenu
        let bar = NSMenu(), app = NSMenu(title: "App"), services = NSMenu(title: "Services")
        app.addItem(withTitle: "Settings…", action: nil, keyEquivalent: ",")
        services.addItem(withTitle: "Make New Sticky Note", action: nil, keyEquivalent: "Y")
        let sItem = NSMenuItem(title: "Services", action: nil, keyEquivalent: ""); sItem.submenu = services; app.addItem(sItem)
        let aItem = NSMenuItem(title: "App", action: nil, keyEquivalent: ""); aItem.submenu = app; bar.addItem(aItem)
        NSApp.servicesMenu = services
        let synthetic = ShortcutList.groups(from: bar).first?.items.map(\.title) ?? []
        NSApp.servicesMenu = saved
        step("Services submenu skipped (\(synthetic))", synthetic == ["Settings…"])

        // The real menu bar of this process: SwiftUI builds it from DeskCommands.
        try? await Task.sleep(for: .milliseconds(600))
        let groups = ShortcutList.groups(from: NSApp.mainMenu)
        let rows = groups.flatMap(\.items)
        print("shortcuts-test: groups \(groups.map { "\($0.title) \($0.items.count)" }.joined(separator: ", ")); \(rows.count) rows")
        let newTab = rows.first { $0.title == "New Tab" }
        step("reads the real menu: New Tab \(newTab?.keys ?? "missing")", newTab?.keys == "⌘T")
        let own = rows.first { $0.title == "Keyboard Shortcuts" }
        step("the panel's own Help item: Keyboard Shortcuts \(own?.keys ?? "missing")", own?.keys == "⌘/" && groups.first { $0.items.contains { $0.title == "Keyboard Shortcuts" } }?.title == "Help")
        step("Move Tab Left from the Tabs menu: \(rows.first { $0.title == "Move Tab Left" }?.keys ?? "missing")",
             rows.first { $0.title == "Move Tab Left" }?.keys == "⌃⇧⌘←")
        step("Tab 1…9 read as one row: \(rows.first { $0.title.hasPrefix("Go to Tab") }.map { "\($0.title) \($0.caps)" } ?? "missing")",
             rows.contains { $0.title == "Go to Tab 1–9" && $0.caps == ["⌘", "1", "–", "9"] } && !rows.contains { $0.title == "Tab 3" })
        step("non-menu keys last (Send message ↩, New line ⇧↩)", groups.last?.title == "Other Keys" && rows.contains(Shortcut(title: "New line in message", caps: ["⇧", "↩"])))
        step("no Emoji & Symbols row (its stored key is wrong)", !rows.contains { $0.title.hasPrefix("Emoji") })
        step("no empty or duplicate rows inside a group", groups.allSatisfy { Set($0.items).count == $0.items.count && $0.items.allSatisfy { !$0.keys.isEmpty } })
        let hits = ShortcutList.filter(groups, "split")
        step("search 'split' keeps only Split rows (\(hits.flatMap(\.items).map(\.title)))", !hits.isEmpty && hits.flatMap(\.items).allSatisfy { $0.title.localizedCaseInsensitiveContains("split") })

        // The panel over the whole window. `frame` is this host's panel (the app's own window may show one too).
        var frame = CGRect.zero
        let model = AppModel(store: .preview)
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).shortcutsPanelHost { frame = $0 }.frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(1.5))
        ShortcutsPanel.toggle()
        try? await Task.sleep(for: .seconds(1))
        let f = frame, bounds = host.bounds
        step("⌘/ action opens the panel", ShortcutsPanel.state.shown)
        step("centred on the window (centre \(Int(f.midX)),\(Int(f.midY)) vs \(Int(bounds.midX)),\(Int(bounds.midY)); \(Int(f.width))x\(Int(f.height)))",
             abs(f.midX - bounds.midX) < 2 && abs(f.midY - bounds.midY) < 2 && f.width > 600)
        step("system search field shown for \(rows.count) rows (> \(ShortcutsPanel.searchThreshold))",
             (rows.count > ShortcutsPanel.searchThreshold) == textFields(in: host).contains { $0 is NSSearchField })
        let windowed = "\(dir)/shortcuts-windowed.png"
        capture(win, to: windowed)
        // Counted at 1x (pixels / scale²): a 1x screen (CI runners) has a quarter of a Retina screen's pixels. The panel's
        // text gives about 3400 on 26 and 2850 on 15 (1x, light); an empty panel gives next to none.
        let inkAt1x = ink(windowed, f, scale: win.backingScaleFactor, height: host.bounds.height) / Int(win.backingScaleFactor * win.backingScaleFactor)
        step("text drawn in the panel (\(inkAt1x) ink px at 1x, > 1500)", inkAt1x > 1500)

        ChromeTests.emulateFullScreen(win, model, true)
        try? await Task.sleep(for: .seconds(1))
        let ff = frame
        step("full screen: same place and size (\(Int(ff.midX)),\(Int(ff.midY)) \(Int(ff.width))x\(Int(ff.height)))", abs(ff.midX - f.midX) < 2 && abs(ff.midY - f.midY) < 2 && abs(ff.height - f.height) < 2)
        capture(win, to: "\(dir)/shortcuts-fullscreen.png")
        ChromeTests.emulateFullScreen(win, model, false)

        ShortcutsPanel.state.query = "zzzz"
        try? await Task.sleep(for: .milliseconds(500))
        capture(win, to: "\(dir)/shortcuts-nomatch.png")
        let nf = frame
        step("search with no match: panel keeps its place and height (\(Int(nf.minY))..\(Int(nf.maxY)) vs \(Int(f.minY))..\(Int(f.maxY)))", abs(nf.minY - f.minY) < 2 && abs(nf.height - f.height) < 2)
        ShortcutsPanel.state.query = ""

        // Esc goes through the panel's key monitor; ⌘/ again toggles; click outside presses the close button.
        let escEvent = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: win.windowNumber, context: nil,
                                        characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        step("Esc is consumed and closes", ShortcutsPanel.handle(escEvent) == nil && !ShortcutsPanel.state.shown)
        ShortcutsPanel.toggle(); try? await Task.sleep(for: .milliseconds(400))
        ShortcutsPanel.toggle(); try? await Task.sleep(for: .milliseconds(400))
        step("⌘/ again closes", !ShortcutsPanel.state.shown)
        let other = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: win.windowNumber, context: nil,
                                     characters: "a", charactersIgnoringModifiers: "a", isARepeat: false, keyCode: 0)!
        step("closed: Esc and other keys pass through", ShortcutsPanel.handle(escEvent) != nil && ShortcutsPanel.handle(other) != nil)
        ShortcutsPanel.toggle(); try? await Task.sleep(for: .milliseconds(600))
        // Ask for the windows over AX first: SwiftUI builds its accessibility tree on that request.
        var axWindows: CFTypeRef?
        _ = AXUIElementCopyAttributeValue(AXUIElementCreateApplication(getpid()), kAXWindowsAttribute as CFString, &axWindows)
        try? await Task.sleep(for: .milliseconds(500))
        var close: NSObject?
        func walk(_ e: Any, _ depth: Int) {
            guard close == nil, depth < 40, let el = e as? NSAccessibilityElementProtocol & NSObject else { return }
            if (el.value(forKey: "accessibilityLabel") as? String) == "Close Keyboard Shortcuts" { close = el; return }
            for c in (el.value(forKey: "accessibilityChildren") as? [Any]) ?? [] { walk(c, depth + 1) }
        }
        walk(host, 0)
        _ = close?.perform(#selector(NSAccessibilityButton.accessibilityPerformPress))
        try? await Task.sleep(for: .milliseconds(500))
        step("click outside closes (\(close == nil ? "no close button" : "pressed"))", close != nil && !ShortcutsPanel.state.shown)
        capture(win, to: "\(dir)/shortcuts-closed.png")

        // Narrow roots: 1200x760 and the window's minimum (WindowGroup minWidth 960, minHeight 600). The root must fit, so
        // the panel stays on the window centre. Fails while the split overflows a narrow root.
        win.orderOut(nil)
        for small in [NSSize(width: 1200, height: 760), NSSize(width: 960, height: 600)] {
            var mine = CGRect.zero   // this host's panel only: closed hosts may still report while they tear down
            let host2 = NSHostingView(rootView: RootView(flat: false).environment(AppModel(store: .preview)).shortcutsPanelHost { mine = $0 }.frame(width: small.width, height: small.height))
            host2.frame = NSRect(origin: .zero, size: small)
            let win2 = OffscreenWindow(contentRect: host2.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
            WindowChrome.apply(win2)
            win2.contentView = host2; win2.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win2.orderFrontRegardless()
            try? await Task.sleep(for: .seconds(1.5))
            ShortcutsPanel.open()
            try? await Task.sleep(for: .seconds(1))
            let sf = mine, sb = host2.bounds
            step("window \(Int(small.width))x\(Int(small.height)): centred (centre \(Int(sf.midX)),\(Int(sf.midY)) vs \(Int(sb.midX)),\(Int(sb.midY)); \(Int(sf.width))x\(Int(sf.height)))",
                 abs(sf.midX - sb.midX) < 2 && abs(sf.midY - sb.midY) < 2)
            capture(win2, to: "\(dir)/shortcuts-\(Int(small.width)).png")
            ShortcutsPanel.close()
            try? await Task.sleep(for: .milliseconds(400))
            win2.orderOut(nil)
        }

        // Keycap strip at 1x and 2x: ⌃ must not read as a caret at 100 %.
        for scale in [1, 2] as [CGFloat] {
            let strip = HStack(spacing: 16) {
                KbdGroup(caps: ["⌃", "⇧", "⌘", "←"]); KbdGroup(caps: ["⌥", "⌘", "↓"]); KbdGroup(caps: ["fn", "F"])
                KbdGroup(caps: ["⌘", "1", "–", "9"]); KbdGroup(caps: ["⇧", "↩"]); KbdGroup(caps: ["⎋"])
            }.padding(12).scaleEffect(scale, anchor: .topLeading)
            let sz = NSSize(width: 520 * scale, height: 44 * scale)
            let h = NSHostingView(rootView: ZStack(alignment: .topLeading) { WindowSurface(glass: .off); strip }.frame(width: sz.width, height: sz.height))
            h.frame = NSRect(origin: .zero, size: sz)
            let w = OffscreenWindow(contentRect: h.frame, styleMask: [.borderless], backing: .buffered, defer: false)
            w.contentView = h; w.setFrameOrigin(NSPoint(x: -3000, y: -3000)); w.orderFrontRegardless()
            try? await Task.sleep(for: .milliseconds(500))
            capture(w, to: "\(dir)/kbd-strip-\(Int(scale))x.png")
            w.orderOut(nil)
        }

        print("shortcuts-test: pictures in \(dir)")
        print("shortcuts-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    private static func textFields(in v: NSView) -> [NSTextField] {
        ((v as? NSTextField).flatMap { $0.isEditable ? [$0] : nil } ?? []) + v.subviews.flatMap { textFields(in: $0) }
    }

    /// Pixels inside `rect` (points, top-left origin) far from the median brightness of that area: text and glyph ink.
    private static func ink(_ path: String, _ rect: CGRect, scale: CGFloat, height: CGFloat) -> Int {
        guard let rep = NSImage(contentsOfFile: path).flatMap({ $0.representations.first as? NSBitmapImageRep }) else { return 0 }
        // The capture includes the title bar above the content: offset by the difference in height.
        let dy = CGFloat(rep.pixelsHigh) / scale - height
        let r = CGRect(x: rect.minX * scale, y: (rect.minY + dy) * scale, width: rect.width * scale, height: rect.height * scale).insetBy(dx: 30 * scale, dy: 30 * scale)
        var lums: [CGFloat] = []
        for y in stride(from: Int(r.minY), to: min(Int(r.maxY), rep.pixelsHigh), by: 2) {
            for x in stride(from: Int(r.minX), to: min(Int(r.maxX), rep.pixelsWide), by: 2) {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { continue }
                lums.append(0.2126 * c.redComponent + 0.7152 * c.greenComponent + 0.0722 * c.blueComponent)
            }
        }
        guard !lums.isEmpty else { return 0 }
        let median = lums.sorted()[lums.count / 2]
        return lums.filter { abs($0 - median) > 0.3 }.count
    }
}
