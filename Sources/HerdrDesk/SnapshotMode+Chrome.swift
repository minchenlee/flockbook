import AppKit
import HerdrKit
import SwiftUI

/// Chrome checks (window, toolbar, sidebar) added during the polish pass. Offscreen window only.
@MainActor enum ChromeTests {
    /// Sidebar rows do what they say, by a real click through the window: New tab = the island's + (current workspace).
    static func runSidebarTest() async {
        AppDefaults.store.set(true, forKey: AppSettings.allowInputKey)
        let model = AppModel(store: .preview)
        var calls: [String] = []
        model.newTabHook = { calls.append("newTab \($0.id)") }
        let size = NSSize(width: 1200, height: 800)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(2))
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("sidebar-test: \(n) \(ok ? "ok" : "FAIL")") }
        func click(_ r: CGRect) async {
            guard let frame = win.contentView?.superview else { return }
            let p = NSPoint(x: r.midX, y: frame.bounds.height - r.midY)
            let e = [NSEvent.EventType.leftMouseDown, .leftMouseUp].compactMap {
                NSEvent.mouseEvent(with: $0, location: p, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                   windowNumber: win.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)
            }
            guard e.count == 2 else { return }
            // The up goes in the queue first: before 26 a toolbar button's mouse-down runs AppKit's tracking loop inside
            // sendEvent, which waits there for the up (sent after it returns, the up never came and the check hung).
            NSApp.postEvent(e[1], atStart: false)
            win.sendEvent(e[0])
            if let up = NSApp.nextEvent(matching: .leftMouseUp, until: .distantPast, inMode: .default, dequeue: true) { win.sendEvent(up) }
            try? await Task.sleep(for: .milliseconds(400))
        }
        // Connection footer: shown only while not connected (none in the connected preview).
        let footer = ChromeProbe.connectionFooter
        step("connection footer only when not connected (connected \(model.store.connected), footer \(footer))",
             model.store.connected ? footer == .zero : footer.height > 0)
        let row = ChromeProbe.rows["New tab"] ?? .zero
        step("New tab row present (\(row))", row.width > 0)
        await click(row)
        step("New tab row creates a tab in the current workspace (\(calls))", calls == ["newTab \(model.workspace?.id ?? "?")"])
        // After the create, the new tab is selected: the tab that was not there before the refresh.
        if let ws = model.workspace, let last = ws.tabs.last, ws.tabs.count > 1 {
            let prev = model.tabID
            model.tabID = ws.tabs[0].id
            model.selectCreatedTab(in: ws.id, before: Set(ws.tabs.dropLast().map(\.id)))
            step("new tab selected after the refresh (\(model.tabID ?? "nil") vs \(last.id))", model.tabID == last.id && model.workspaceID == ws.id)
            let still = model.tabID
            model.selectCreatedTab(in: ws.id, before: Set(ws.tabs.map(\.id)))
            step("no new tab: selection unchanged", model.tabID == still)
            step("createdTabID picks the new id (\(AppModel.createdTabID(before: ["a", "b"], after: ["a", "c", "b"]) ?? "nil"))",
                 AppModel.createdTabID(before: ["a", "b"], after: ["a", "c", "b"]) == "c" && AppModel.createdTabID(before: ["a"], after: ["a"]) == nil)
            model.tabID = prev
        } else { step("preview has 2+ tabs", false) }
        AppDefaults.store.set(false, forKey: AppSettings.allowInputKey)
        try? await Task.sleep(for: .milliseconds(400))
        calls = []
        await click(row)
        step("read-only: New tab row is disabled (\(calls))", calls.isEmpty)
        // System sidebar toggle (NavigationSplitView's own item): beside the lights while the sidebar is shown and while hidden;
        // a real click on it brings the sidebar back.
        let lightsMax = model.lightsMaxX, lightsMid = model.lightsMidY
        let shown = ChromeProbe.toggle
        step("system sidebar toggle in the lights row, sidebar shown (\(shown))", shown.width > 0 && abs(shown.midY - lightsMid) <= 2 && shown.minX > lightsMax)
        model.toggleSidebar(); try? await Task.sleep(for: .seconds(1))
        let hidden = ChromeProbe.toggle
        step("system sidebar toggle beside the lights, sidebar hidden (\(hidden))", hidden.width > 0 && abs(hidden.midY - lightsMid) <= 2 && hidden.minX > lightsMax)
        ContentTests.capture(win, to: "build/snapshots-glass/sidebar-hidden.png")
        await click(hidden); try? await Task.sleep(for: .seconds(0.8))
        step("clicking it shows the sidebar again (\(model.sidebarVisibility))", model.sidebarVisibility != .detailOnly)
        // The list selection follows the screen.
        model.screen = .overview; try? await Task.sleep(for: .milliseconds(600))
        ContentTests.capture(win, to: "build/snapshots-glass/sidebar-overview.png")
        // Arrow keys move the list selection (system List): Overview, Down -> Search.
        func all<T: NSView>(_ t: T.Type, in v: NSView) -> [T] { ((v as? T).map { [$0] } ?? []) + v.subviews.flatMap { all(t, in: $0) } }
        if let table = all(NSTableView.self, in: win.contentView!).first {
            win.makeFirstResponder(table)
            for type in [NSEvent.EventType.keyDown, .keyUp] {
                if let e = NSEvent.keyEvent(with: type, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                            windowNumber: win.windowNumber, context: nil, characters: "\u{F701}", charactersIgnoringModifiers: "\u{F701}",
                                            isARepeat: false, keyCode: 125) { win.sendEvent(e) }
            }
            try? await Task.sleep(for: .milliseconds(500))
        }
        step("Down arrow in the sidebar moves Overview -> Search (\(model.screen))", model.screen == .search)
        model.screen = .tab
        // Workspace rows: the name first, one summary at the end.
        func ws(_ label: String, _ st: [AgentStatus]) -> Workspace {
            Workspace(id: "x", label: label, tabs: st.enumerated().map { Tab(id: "t\($0.offset)", label: "t", number: $0.offset + 1, panes: [], status: $0.element) })
        }
        step("title keeps the part before \" · \" (\(WorkspaceSummary.title(ws("[ME] Meadow Migration · Discovery", []))))",
             WorkspaceSummary.title(ws("[ME] Meadow Migration · Discovery", [])) == "Meadow Migration")
        let busy = ws("A", [.blocked, .working, .working, .done, .idle])
        step("tooltip carries the detail (\(WorkspaceSummary.tooltip(busy)))", WorkspaceSummary.tooltip(busy) == "A · 1 need you · 2 working · 1 done · 5 tabs")
        AppDefaults.store.removeObject(forKey: AppSettings.allowInputKey)
        print("sidebar-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Full-screen layout in an offscreen window: the flag the willEnterFullScreen notification sets, and the title bar
    /// (with the toolbar) out of the way, as AppKit moves it into its own hidden window in real full screen. The window
    /// keeps its toolbar inset, so the band is the toolbar's (FullScreenBand adds nothing).
    static func emulateFullScreen(_ win: NSWindow, _ model: AppModel, _ on: Bool) {
        model.fullScreen = on
        var v = win.standardWindowButton(.closeButton)?.superview
        while let cur = v, !String(describing: type(of: cur)).contains("TitlebarContainer") { v = cur.superview }
        v?.isHidden = on
    }

    /// Calls toggleSidebar and watches 0.5 s: our full-screen toggle's frame in the window content every ~4 ms (and the
    /// content's own x: the offscreen test root has a fixed width, which the split overflows for a moment while it shows
    /// the sidebar), how often the docked panel edge
    /// (sidebarMaxX) was written, display-link frame times of the window, and the process CPU time.
    static func sampleToggle(_ win: NSWindow, _ model: AppModel) async -> (toggles: [CGRect], islands: [CGRect], roots: [CGFloat], edgeWrites: Int, frames: FrameTimer.Stats, cpu: Double) {
        var toggles: [CGRect] = [], islands: [CGRect] = [], roots: [CGFloat] = [], edgeWrites = 0, lastEdge = model.sidebarMaxX
        let timer = FrameTimer(view: win.contentView!), cpu0 = FrameTimer.cpu(), t0 = Date()
        model.toggleSidebar()
        while Date().timeIntervalSince(t0) < 0.5 {
            toggles.append(ChromeProbe.fullScreenToggle.offsetBy(dx: -ChromeProbe.root.minX, dy: -ChromeProbe.root.minY)); islands.append(TabProbe.island); roots.append(ChromeProbe.root.minX)
            if abs(model.sidebarMaxX - lastEdge) > 0.25 { edgeWrites += 1; lastEdge = model.sidebarMaxX }
            try? await Task.sleep(for: .milliseconds(4))
        }
        return (toggles, islands, roots, edgeWrites, timer.stop(), FrameTimer.cpu() - cpu0)
    }

    /// RootView fills the window and no more (`--test-root-width`): at 960 (the window's minimum) and 1200 pt wide, windowed
    /// and in the full-screen layout, sidebar shown and hidden, the root's frame is the window content's and the detail
    /// column and the island's buttons end inside the right edge. A root wider than the window is centred on it by
    /// SwiftUI: the content shifts left and is clipped at both edges.
    /// `--test-newtab-live`: one real New Tab in the named session c9-input-test (never the default session), no
    /// window. The new tab must be selected right after the refresh; the test then closes that tab.
    static func runNewTabLiveTest() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("newtab-live: \(n) \(ok ? "ok" : "FAIL")") }
        let store = HerdrStore(session: .named("c9-input-test"))
        await store.refresh()
        let model = AppModel(store: store)
        guard store.connected, let ws = store.workspaces.min(by: { $0.tabs.count < $1.tabs.count }), let first = ws.tabs.first else {
            print("newtab-live: c9-input-test not reachable FAIL"); NSApp.terminate(nil); return
        }
        model.workspaceID = ws.id
        model.tabID = first.id
        let before = Set(ws.tabs.map(\.id))
        model.newTab()
        var created: String?
        for _ in 0..<50 where created == nil {
            try? await Task.sleep(for: .milliseconds(100))
            if let id = model.tabID, !before.contains(id) { created = id }
        }
        let now = store.workspaces.first { $0.id == ws.id }?.tabs.map(\.id) ?? []
        step("workspace \(ws.id): \(before.count) -> \(now.count) tabs; selected \(model.tabID ?? "nil") (was \(first.id))",
             created != nil && now.contains(created!) && model.workspaceID == ws.id)
        if let created {
            try? await store.client.closeTab(created)
            await store.refresh()
            let after = Set(store.workspaces.first { $0.id == ws.id }?.tabs.map(\.id) ?? [])
            step("closed the test tab \(created) (tabs back to \(after.count))", after == before)
        }
        print("newtab-live: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// `--test-settings-migration`: the copy from the old bundle id's settings, on two temporary suites (never the owner's
    /// domains). Copies once into an empty domain, sets the flag, never changes the old domain, never overwrites.
    static func runSettingsMigrationTest() {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("settings-migration: \(n) \(ok ? "ok" : "FAIL")") }
        let oldName = "herdr-desk.test.mig-old-\(getpid())", newName = "herdr-desk.test.mig-new-\(getpid())"
        let old = UserDefaults(suiteName: oldName)!, new = UserDefaults(suiteName: newName)!
        old.removePersistentDomain(forName: oldName); new.removePersistentDomain(forName: newName)
        let frameKey = "NSWindow Frame SwiftUI.ModifiedContent<HerdrDesk.RootView>-1-AppWindow-1"
        old.setPersistentDomain([AppSettings.allowInputKey: false, ReadingWidth.key: "wide", "terminalSheetSize": [997.0, 798.0],
                                 frameKey: "95 51 1280 820 0 0 1470 922 "], forName: oldName)
        let before = old.persistentDomain(forName: oldName) as NSDictionary?
        let copied = AppDefaults.migrate(from: old.persistentDomain(forName: oldName), into: new, domain: newName)
        step("copies into the empty new domain (copied \(copied))", copied)
        step("values: allowInput \(new.object(forKey: AppSettings.allowInputKey) ?? "nil"), readingWidth \(new.string(forKey: ReadingWidth.key) ?? "nil"), sheet \(new.array(forKey: "terminalSheetSize") ?? []), window frame kept",
             new.object(forKey: AppSettings.allowInputKey) as? Bool == false && new.string(forKey: ReadingWidth.key) == "wide"
             && new.array(forKey: "terminalSheetSize") as? [Double] == [997, 798] && new.string(forKey: frameKey) == "95 51 1280 820 0 0 1470 922 ")
        step("migrated flag set", new.bool(forKey: AppDefaults.migratedKey))
        step("old domain unchanged", (old.persistentDomain(forName: oldName) as NSDictionary?) == before)
        new.set("narrow", forKey: ReadingWidth.key)
        step("second launch: no copy, the user's new value stays",
             !AppDefaults.migrate(from: old.persistentDomain(forName: oldName), into: new, domain: newName) && new.string(forKey: ReadingWidth.key) == "narrow")
        new.removePersistentDomain(forName: newName)
        // What a test run leaves in the standard defaults (AppKit's split-view frames) and a key the user set already.
        new.set("medium", forKey: ReadingWidth.key)
        new.set(["0, 0, 258, 820, NO, NO"], forKey: "NSSplitView Subview Frames test")
        let some = AppDefaults.migrate(from: old.persistentDomain(forName: oldName), into: new, domain: newName)
        step("new domain has some keys: copies only the missing ones, keeps the existing value (readingWidth \(new.string(forKey: ReadingWidth.key) ?? "nil"), allowInput \(new.object(forKey: AppSettings.allowInputKey) ?? "nil"))",
             some && new.string(forKey: ReadingWidth.key) == "medium"
             && new.object(forKey: AppSettings.allowInputKey) as? Bool == false && new.string(forKey: frameKey) != nil && new.bool(forKey: AppDefaults.migratedKey))
        new.removePersistentDomain(forName: newName)
        step("no old settings: nothing copied, flag set", !AppDefaults.migrate(from: nil, into: new, domain: newName) && new.bool(forKey: AppDefaults.migratedKey))
        step("test runs skip the real migration (isTest \(AppDefaults.isTest); store has no flag)", AppDefaults.isTest && AppDefaults.store.object(forKey: AppDefaults.migratedKey) == nil)
        for n in [oldName, newName] {
            UserDefaults(suiteName: n)?.removePersistentDomain(forName: n)
            try? FileManager.default.removeItem(at: FileManager.default.homeDirectoryForCurrentUser.appending(path: "Library/Preferences/\(n).plist"))
        }
        print("settings-migration: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        exit(failures == 0 ? 0 : 1)
    }

    /// `--test-island-height`: the tab island and the + / terminal capsule have the same top and bottom edges (pixels of the
    /// window server's composite, within 0.5 pt): windowed (toolbar copy), full screen at rest (content copy; the toolbar is
    /// hidden there) and revealed (toolbar copy again). Full screen is the layout flag on an offscreen window.
    static func runIslandHeightTest() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("island-height: \(n) \(ok ? "ok" : "FAIL")") }
        let model = AppModel(store: ProcessInfo.processInfo.environment["ISLAND_DENSE"] != nil ? .previewDense : .preview)   // ISLAND_DENSE: many tabs, scrolled strip
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        let t0 = Date()
        while Date().timeIntervalSince(t0) < 8, WindowChrome.toolbarRow(win) == nil { try? await Task.sleep(for: .milliseconds(100)) }
        try? await Task.sleep(for: .seconds(1))
        func image() -> NSBitmapImageRep? {
            typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
            guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage"),
                  let img = unsafeBitCast(sym, to: Fn.self)(.null, 8, UInt32(win.windowNumber), 1)?.takeRetainedValue() else { return nil }
            return NSBitmapImageRep(cgImage: img)
        }
        /// Top and bottom of the shape under column x (points from the window top): the first and last rows in the band
        /// that differ from the band's own colour above the shape.
        func extent(_ rep: NSBitmapImageRep, x: CGFloat) -> (top: CGFloat, bottom: CGFloat)? {
            let k = CGFloat(rep.pixelsWide) / win.frame.width
            func lum(_ y: Int) -> CGFloat {
                guard let c = rep.colorAt(x: Int(x * k), y: y)?.usingColorSpace(.sRGB) else { return 0 }
                return 0.2126 * c.redComponent + 0.7152 * c.greenComponent + 0.0722 * c.blueComponent
            }
            let bg = lum(Int(2 * k)), rows = (Int(2 * k)..<Int(50 * k)).filter { abs(lum($0) - bg) > 0.012 }
            guard let a = rows.first, let b = rows.last else { return nil }
            return (CGFloat(a) / k, CGFloat(b + 1) / k)
        }
        func check(_ tag: String, island: CGRect, actions: CGRect, file: String) {
            guard let rep = image() else { step("\(tag): window picture", false); return }
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "build/snapshots-glass/island-height-\(file).png"))
            // Away from the rounded ends: a quarter in from the island's left end, the group's middle.
            let i = extent(rep, x: island.minX + island.width / 4), a = extent(rep, x: actions.midX)
            func f(_ e: (top: CGFloat, bottom: CGFloat)?) -> String { e.map { String(format: "%.1f...%.1f (%.1f)", $0.top, $0.bottom, $0.bottom - $0.top) } ?? "none" }
            step("\(tag): island \(f(i)) = buttons \(f(a))",
                 i != nil && a != nil && abs(i!.top - a!.top) <= 0.5 && abs(i!.bottom - a!.bottom) <= 0.5)
        }
        guard let row = WindowChrome.toolbarRow(win) else { step("toolbar row measured", false); exit(1) }
        // toolbarRow: x in the window, y from the window top.
        check("windowed (toolbar)", island: row.island, actions: row.actions, file: "windowed")
        emulateFullScreen(win, model, true)
        win.toolbar?.isVisible = false   // as at rest in real full screen: only the content copy shows
        try? await Task.sleep(for: .seconds(1))
        // Scroll-edge pockets (AppKit draws them as their own views) in the band: the island's horizontal strip must not
        // have one (owner shot 54: a band-tall dark rectangle with the text under it, over part of the island).
        func pockets(_ v: NSView) -> [NSView] { (String(describing: type(of: v)).contains("Pocket") ? [v] : []) + v.subviews.flatMap(pockets) }
        let band = pockets(win.contentView!.superview!).map { ($0, $0.convert($0.bounds, to: nil)) }
            .filter { $0.1.maxY > win.contentView!.bounds.height - model.toolbarBand - 1 && $0.1.width > 0 && !$0.0.isHidden }
        if ProcessInfo.processInfo.environment["ISLAND_POCKETS"] != nil { for (v, r) in band { print("island-height: pocket \(type(of: v)) window y \(win.contentView!.bounds.height - r.maxY)...\(win.contentView!.bounds.height - r.minY) x \(r.minX)...\(r.maxX) in \(type(of: v.superview!))") } }
        let island = TabProbe.island
        step("full screen at rest: no scroll-edge pocket over the island (\(band.count) in the band)",
             !band.contains { $0.1.minX < island.maxX && $0.1.maxX > island.minX && abs($0.1.width - win.frame.width) > 1 })
        let ci = TabProbe.island, ca = ChromeProbe.fullScreenActions
        // Probes are in the content view's space (top-left); the content view starts at the window top (full-size content).
        check("full screen at rest (content copy)", island: ci, actions: ca, file: "fullscreen-rest")
        let toggle = ChromeProbe.fullScreenToggle
        step(String(format: "full screen: sidebar toggle circle %.0f tall = island band %.0f", toggle.height, TopRow.height), abs(toggle.height - TopRow.height) <= 0.5)
        win.toolbar?.isVisible = true
        emulateFullScreen(win, model, false); model.fullScreen = true   // the toolbar row back in this window, still full screen
        model.toolbarReveal = model.toolbarBand   // revealed: the content copy hides at once, the toolbar copy shows
        try? await Task.sleep(for: .seconds(1))
        if let r = WindowChrome.toolbarRow(win) { check("full screen revealed (toolbar copy)", island: r.island, actions: r.actions, file: "fullscreen-revealed") }
        else { step("revealed: toolbar row measured", false) }
        print("island-height: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    static func runRootWidthTest() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1; print("root-width: failed step: \(n)") } else { print("root-width: \(n) ok") } }
        for w in [CGFloat(960), 1200] {
            let model = AppModel(store: .preview)
            let size = NSSize(width: w, height: 760)
            // As the app's WindowGroup has it: the shortcuts panel host outermost.
            var p = CGRect.zero
            let host = NSHostingView(rootView: RootView(flat: false).environment(model).shortcutsPanelHost(report: { p = $0 }).frame(width: size.width, height: size.height))
            host.frame = NSRect(origin: .zero, size: size)
            let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
            WindowChrome.apply(win)
            win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
            try? await Task.sleep(for: .seconds(1.5))
            if ProcessInfo.processInfo.environment["ROOT_TRACE"] != nil {
                for x in NSApp.windows { print("root-width: window \(type(of: x)) \(x.frame) visible \(x.isVisible) test host \(x === win) split view \(WindowWidthTracker.splitView(in: x.contentView) != nil)") }
            }
            ShortcutsPanel.toggle(); try? await Task.sleep(for: .milliseconds(800))
            step("\(Int(w)) windowed: shortcuts panel centred on the window (midX \(p.midX) vs \(w / 2))", abs(p.midX - w / 2) <= 1)
            ShortcutsPanel.toggle(); try? await Task.sleep(for: .milliseconds(400))
            for fs in [false, true] {
                emulateFullScreen(win, model, fs)
                // Hidden first: the very first layout pass can leave a stale detail frame (the picture shows no clip), so
                // "shown" is measured after a real change.
                for vis in [NavigationSplitViewVisibility.detailOnly, .all] {
                    model.sidebarVisibility = vis
                    try? await Task.sleep(for: .milliseconds(900))
                    let r = ChromeProbe.root, d = ChromeProbe.detail
                    let acts = fs ? ChromeProbe.fullScreenActions : .zero
                    if let dir = ProcessInfo.processInfo.environment["ROOT_SHOT"] {
                        ContentTests.capture(win, to: "\(dir)/root-\(Int(w))-\(fs ? "fs" : "win")-\(vis == .all ? "shown" : "hidden").png")
                    }
                    let tag = "\(Int(w)) \(fs ? "full-screen layout" : "windowed"), sidebar \(vis == .all ? "shown" : "hidden")"
                    step("\(tag): root \(r) = window content 0...\(Int(w)); detail \(d.minX)...\(d.maxX), panel \(ChromeProbe.panel.minX)...\(ChromeProbe.panel.maxX), host \(host.frame.width); buttons end at \(acts.maxX)",
                         abs(r.minX) <= 0.5 && abs(r.width - w) <= 0.5 && abs(d.maxX - w) <= 0.5 && acts.maxX <= w)
                }
                model.sidebarVisibility = .all
                try? await Task.sleep(for: .milliseconds(600))
                // While the sidebar slides in (toggle): the root's x, sampled every ~4 ms.
                model.toggleSidebar(); try? await Task.sleep(for: .milliseconds(700))
                var xs: [CGFloat] = [], panels: [CGFloat] = []
                let t0 = Date()
                model.toggleSidebar()
                var trace: [String] = []
                while Date().timeIntervalSince(t0) < 0.6 {
                    xs.append(ChromeProbe.root.minX)
                    if panels.last != ChromeProbe.panel.minX { panels.append(ChromeProbe.panel.minX) }
                    let r = ChromeProbe.root, d = ChromeProbe.detail, pn = ChromeProbe.panel
                    let line = String(format: "root %.0f+%.0f detail %.0f+%.0f panel %.0f+%.0f", r.minX, r.width, d.minX, d.width, pn.minX, pn.width)
                    if trace.last != line { trace.append(line) }
                    try? await Task.sleep(for: .milliseconds(4))
                }
                if ProcessInfo.processInfo.environment["ROOT_TRACE"] != nil { print(trace.joined(separator: "\n")) }
                step("\(Int(w)) \(fs ? "full-screen layout" : "windowed"), sidebar slides in: content stays at x 0 (root minX \(Set(xs.map { Int($0) }).sorted()) over \(xs.count) samples), panel moves only right (\(panels.map { Int($0) }))",
                     xs.allSatisfy { abs($0) <= 0.5 } && zip(panels.dropFirst(), panels).allSatisfy { $0 >= $1 - 0.5 } && (panels.last ?? 0) == ChromeProbe.panel.minX)
            }
            emulateFullScreen(win, model, false)
            // Gone for good: the next window's probes must not be overwritten by this one's late layout passes.
            win.contentView = nil; win.orderOut(nil)
            try? await Task.sleep(for: .milliseconds(300))
        }
        print("root-width: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Sidebar edges (dense preview: 12, 7, 4, 3, 3, 1 tabs): nav symbols, the Workspaces header and the avatars share one
    /// leading edge; the Needs-you badge, the status columns and the header button one trailing edge; the header button is
    /// centred on the header text. Frames from probes, then the same edges again from the window's pixels.
    static func runSidebarLayoutTest() async {
        let store = HerdrStore.previewDense
        let model = AppModel(store: store)
        // One blocked tab somewhere, so the Needs-you badge shows.
        let size = NSSize(width: 1200, height: 800)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        model.screen = .overview   // no workspace row selected: plain rows for the pixel scan
        try? await Task.sleep(for: .seconds(2))
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1; print("sidebar-layout: failed step: \(n)") } else { print("sidebar-layout: \(n) ok") } }
        func r(_ k: String) -> CGRect { ChromeProbe.rows[k] ?? .zero }
        func near(_ xs: [CGFloat], _ tol: CGFloat = 1) -> Bool { guard let a = xs.min(), let b = xs.max() else { return false }; return b - a <= tol }
        func f(_ xs: [CGFloat]) -> String { xs.map { String(format: "%.1f", $0) }.joined(separator: " ") }
        let ids = store.workspaces.map(\.id)
        let nav = ["New tab", "Needs you", "Overview", "Search"]
        let avatars = ids.map { r("avatar:\($0)") }, columns = ids.map { r("status:\($0)") }
        let icons = nav.map { r("icon:\($0)") }, header = r("Workspaces"), button = ChromeProbe.newWorkspace, badge = r("badge:Needs you")
        step("all probes present", (avatars + columns + icons + [header, button, badge]).allSatisfy { $0.width > 0 })
        let leading = icons.map(\.minX) + avatars.map(\.minX) + [header.minX]
        step("one leading edge: nav symbol boxes, header text, avatars (\(f(leading)))", near(leading))
        // The header button is aligned by its ink (its symbol overhangs its frame): checked in the pixel step below.
        let trailing = columns.map(\.maxX) + [badge.maxX]
        step("one trailing edge: status columns, Needs-you badge (\(f(trailing)))", near(trailing))
        step("status columns one width (\(f(columns.map(\.width))))", near(columns.map(\.width), 0.5))
        step("header button centred on the header text (\(f([button.midY, header.midY])))", abs(button.midY - header.midY) <= 1)
        let titles = ids.map { r("title:\($0)") }
        step("names end before the status column (\(f(zip(titles, columns).map { $1.minX - $0.maxX })))",
             zip(titles, columns).allSatisfy { $0.maxX <= $1.minX })
        // The 12-tab workspace: five glyphs and +7; the most urgent five.
        let (shown, more) = StatusColumn.shown(store.workspaces[1].tabs)
        step("12 tabs -> 3 glyphs + 9 (\(shown.map(\.status.rawValue)) +\(more))", shown.count == 3 && more == 9 && shown.contains { $0.status == .blocked })
        step("column fits five glyphs (\(f(columns.map(\.width))))", abs((columns.first?.width ?? 0) - StatusColumn.width(5)) <= 0.5)
        // Pixels: rightmost and leftmost ink in each row band (ink = differs from the band's median tone).
        let path = "build/snapshots-glass/sidebar-layout.png"
        ContentTests.capture(win, to: path)
        if let rep = NSBitmapImageRep(data: (try? Data(contentsOf: URL(fileURLWithPath: path))) ?? Data()) {
            let k = CGFloat(rep.pixelsWide) / win.frame.width
            let panel = ChromeProbe.panel
            func lum(_ x: Int, _ y: Int) -> CGFloat { guard let c = rep.colorAt(x: x, y: y) else { return 0 }; return 0.3 * c.redComponent + 0.59 * c.greenComponent + 0.11 * c.blueComponent }
            /// Ink extent of a band (in points), scanning the panel's inner width.
            func ink(_ band: CGRect) -> (CGFloat, CGFloat)? {
                let x0 = Int((panel.minX + 6) * k), x1 = Int((panel.maxX - 6) * k), y0 = Int(band.minY * k), y1 = Int(band.maxY * k)
                guard x1 > x0, y1 > y0 else { return nil }
                var tones: [CGFloat] = []
                for y in stride(from: y0, to: y1, by: 2) { for x in stride(from: x0, to: x1, by: 3) { tones.append(lum(x, y)) } }
                let bg = tones.sorted()[tones.count / 2]
                var lo = Int.max, hi = Int.min
                for y in y0..<y1 { for x in x0..<x1 where abs(lum(x, y) - bg) > 0.04 { lo = min(lo, x); hi = max(hi, x) } }
                return lo <= hi ? (CGFloat(lo) / k, CGFloat(hi + 1) / k) : nil
            }
            let rowsInk = avatars.compactMap { ink($0.insetBy(dx: 0, dy: 1)) }
            let wsBand = CGRect(x: 0, y: header.minY, width: 1, height: header.height).union(CGRect(x: 0, y: button.minY, width: 1, height: button.height))
            let headInk = ink(wsBand)
            let badgeInk = ink(badge)
            let lefts = rowsInk.map(\.0) + (headInk.map { [$0.0] } ?? [])
            let rights = rowsInk.map(\.1) + (headInk.map { [$0.1] } ?? []) + (badgeInk.map { [$0.1] } ?? [])
            step("pixels: one leading edge, avatars and header (\(f(lefts)))", rowsInk.count == ids.count && headInk != nil && near(lefts, 2))
            step("pixels: one trailing edge, status glyphs, header button, Needs-you badge (\(f(rights)))", rowsInk.count == ids.count && headInk != nil && badgeInk != nil && near(rights, 1.5))
        } else { step("capture readable", false) }
        print("sidebar-layout: picture \(path)")
        print("sidebar-layout: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Tabs without the mouse: the tab menu and the Tabs menu move a tab one place, at the ends the move is disabled,
    /// and every pill is one VoiceOver button with the menu's actions. Settings renders as a grouped form.
    static func runTabKeysTest() async {
        AppDefaults.store.set(true, forKey: AppSettings.allowInputKey)
        let model = AppModel(store: .preview)
        var calls: [String] = []
        model.workspaceHook = { calls.append("\($0) \($1)") }
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("tabkeys-test: \(n) \(ok ? "ok" : "FAIL")") }
        guard let ws = model.workspace, ws.tabs.count >= 3 else { print("tabkeys-test: preview has < 3 tabs FAIL"); NSApp.terminate(nil); return }
        let first = ws.tabs[0], mid = ws.tabs[1], last = ws.tabs[ws.tabs.count - 1]
        func entry(_ t: Tab, _ id: String) -> MenuEntry? { model.tabMenu(t).first { $0.id == id } }
        step("first tab: Move left disabled, Move right enabled", entry(first, "moveLeft")?.enabled == false && entry(first, "moveRight")?.enabled == true)
        step("last tab: Move right disabled", entry(last, "moveRight")?.enabled == false && entry(last, "moveLeft")?.enabled == true)
        entry(mid, "moveRight")?.run(); entry(mid, "moveLeft")?.run()
        model.moveTab(first, by: -1); model.moveTab(last, by: 1)
        step("moves call moveTab with the final index; out-of-range moves do nothing (\(calls))",
             calls == ["moveTab \(mid.id) 2", "moveTab \(mid.id) 0"])
        AppDefaults.store.set(false, forKey: AppSettings.allowInputKey)
        step("read-only: Move left/right disabled", entry(mid, "moveLeft")?.enabled == false && entry(mid, "moveRight")?.enabled == false)
        AppDefaults.store.set(true, forKey: AppSettings.allowInputKey)
        // VoiceOver tree of a real offscreen window: one button per tab, selected state, named actions.
        let size = NSSize(width: 1200, height: 800)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win); win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(2))
        var found: [String: (selected: Bool, actions: [String])] = [:]
        func walk(_ e: Any, depth: Int) {
            guard depth < 40, let el = e as? NSAccessibilityElementProtocol & NSObject else { return }
            let label = (el.value(forKey: "accessibilityLabel") as? String) ?? ""
            if ProcessInfo.processInfo.environment["AXDBG"] != nil { print("ax \(String(repeating: " ", count: depth))\(type(of: el)) role=\(el.value(forKey: "accessibilityRole") ?? "-") label=\(label)") }
            if ws.tabs.contains(where: { $0.label == label }), (el.value(forKey: "accessibilityRole") as? String) == NSAccessibility.Role.button.rawValue {
                let sel = (el.value(forKey: "isAccessibilitySelected") as? Bool) ?? false
                let acts = ((el.value(forKey: "accessibilityCustomActions") as? [NSAccessibilityCustomAction]) ?? []).map(\.name)
                found[label] = (sel, acts)
            }
            for c in (el.value(forKey: "accessibilityChildren") as? [Any]) ?? [] { walk(c, depth: depth + 1) }
        }
        // SwiftUI builds its accessibility tree only once an AX client asks: ask our own process (no trust needed for that).
        var axWindows: CFTypeRef?
        _ = AXUIElementCopyAttributeValue(AXUIElementCreateApplication(getpid()), kAXWindowsAttribute as CFString, &axWindows)
        try? await Task.sleep(for: .seconds(1))
        var roots: [Any] = [host]
        if let tb = win.toolbar { roots += tb.items.compactMap(\.view) }
        for r in roots { walk(r, depth: 0) }
        let sel = model.tab?.label ?? "?"
        step("VoiceOver: \(found.count)/\(ws.tabs.count) tabs are buttons; selected = \(found.filter { $0.value.selected }.map(\.key)); actions of \(sel): \(found[sel]?.actions ?? [])",
             found.count == ws.tabs.count && found[sel]?.selected == true && (found[sel]?.actions ?? []).contains("Close tab"))
        // Settings: grouped form picture.
        let settings = NSHostingView(rootView: SettingsView().frame(width: 500))
        settings.frame = NSRect(x: 0, y: 0, width: 500, height: settings.fittingSize.height)
        let sw = OffscreenWindow(contentRect: settings.frame, styleMask: [.titled], backing: .buffered, defer: false)
        sw.contentView = settings; sw.setFrameOrigin(NSPoint(x: -4000, y: -3000)); sw.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(1))
        if let rep = settings.bitmapImageRepForCachingDisplay(in: settings.bounds) {
            settings.cacheDisplay(in: settings.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "build/snapshots-glass/settings.png"))
        }
        step("Settings renders (\(Int(settings.fittingSize.height)) pt tall), picture build/snapshots-glass/settings.png", settings.fittingSize.height > 200)
        AppDefaults.store.removeObject(forKey: AppSettings.allowInputKey)
        print("tabkeys-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }
}

/// Real full screen, by the owner's leave only (visible window, Space switch): `FS_REAL_DIR=<dir> HerdrDesk --preview
/// --test-fullscreen-real` from a test bundle (a path argument would be taken as a file to open, and no window opens).
/// Windowed geometry, then full screen at rest (pictures, geometry, a hit test on the island), the menu-bar reveal (pointer
/// warped to the top edge; pictures while it slides), the left-edge sidebar peek (frame times and CPU, then pictures) and
/// back to windowed. A watchdog ends the process after 13 s whatever happens.
@MainActor enum FullScreenReal {
    static func run(dir: String, model: AppModel) async {
        setvbuf(stdout, nil, _IOLBF, 0)
        DispatchQueue.global().asyncAfter(deadline: .now() + 13) { print("fs-real: watchdog exit"); exit(3) }
        let t0 = Date()
        func log(_ s: String) { print(String(format: "fs-real %5.2f: ", Date().timeIntervalSince(t0)) + s) }
        func r(_ c: CGRect) -> String { String(format: "(%.1f, %.1f, %.1f, %.1f)", c.minX, c.minY, c.width, c.height) }
        // The app's own WindowGroup window (scene modifiers such as windowToolbarFullScreenVisibility apply only there).
        var win: NSWindow?
        for _ in 0..<40 where win == nil {
            try? await Task.sleep(for: .milliseconds(100))
            win = NSApp.windows.first { $0.toolbar != nil && $0.isVisible }
        }
        guard let win, let screen = win.screen ?? NSScreen.main else { log("no window"); exit(1) }
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let H = CGDisplayBounds(CGMainDisplayID()).height
        let pointer = NSEvent.mouseLocation
        func capture(_ name: String) {
            typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
            guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else { return }
            let f = unsafeBitCast(sym, to: Fn.self)
            var rect = screen.frame; rect.origin.y = H - screen.frame.maxY
            guard let img = f(rect, 1, 0, 0)?.takeRetainedValue() else { log("capture failed"); return }
            try? NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "\(dir)/\(name).png"))
        }
        func geometry(_ tag: String) {
            log("\(tag): window \(r(win.frame)) island \(r(TabProbe.island)) buttons \(r(ChromeProbe.fullScreenActions)) New tab \(r(ChromeProbe.rows["New tab"] ?? .zero)) panel \(r(ChromeProbe.panel)) detail \(r(ChromeProbe.detail)) lights \(model.lights.map(r)) toggle sys \(r(ChromeProbe.toggle)) ours \(r(ChromeProbe.fullScreenToggle)) detached \(model.toolbarDetached) reveal \(model.toolbarReveal) band \(model.toolbarBand)")
        }
        /// The tab island's two copies in screen space (top-left origin, points): the content copy (FullScreenTopRow, probe)
        /// and the toolbar's (its item view, wherever its window is now).
        func islands(_ tag: String) {
            let c = TabProbe.island.offsetBy(dx: win.frame.minX, dy: H - win.frame.maxY)
            let v = (win.toolbar?.items ?? []).compactMap(\.view).filter { $0.window != nil }.max { $0.frame.width < $1.frame.width }
            var t = CGRect.zero
            if let v, let vw = v.window { t = vw.convertToScreen(v.convert(v.bounds, to: nil)); t.origin.y = H - t.maxY }
            log(String(format: "\(tag): content island centre (%.1f, %.1f) toolbar island centre (%.1f, %.1f) diff (%.1f, %.1f); measured \(r(model.toolbarIsland)) actions \(r(model.toolbarActions)) buttons \(r(ChromeProbe.fullScreenActions)); content active \(model.islandActive(.content))",
                       c.midX, c.midY, t.midX, t.midY, t.midX - c.midX, t.midY - c.midY))
        }
        /// What a click at a window point (top-left space) reaches.
        func hit(_ p: CGPoint) -> String {
            guard let frame = win.contentView?.superview else { return "?" }
            var v = frame.hitTest(NSPoint(x: p.x, y: frame.bounds.height - p.y)); var names: [String] = []
            while let cur = v, names.count < 4 { names.append(String(describing: type(of: cur)).prefix(32).description); v = cur.superview }
            return names.joined(separator: " < ")
        }
        NSApp.activate(); win.makeKeyAndOrderFront(nil)
        try? await Task.sleep(for: .seconds(0.8))
        log("active \(NSApp.isActive), front app \(NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "?")")
        geometry("windowed")
        capture("0-windowed")
        win.toggleFullScreen(nil)
        for _ in 0..<30 { try? await Task.sleep(for: .milliseconds(100)); if win.styleMask.contains(.fullScreen) { break } }
        try? await Task.sleep(for: .seconds(1.2))
        guard win.styleMask.contains(.fullScreen) else { log("full screen failed (screen locked?)"); exit(4) }
        geometry("full screen rest")
        islands("rest")
        if let ws = model.workspace, let t = ws.tabs.first, let pill = TabProbe.pills[t.id] {
            log("hit at the first pill: \(hit(CGPoint(x: pill.midX, y: pill.midY)))")
        }
        capture("1-rest")
        // Menu-bar reveal: the pointer to the top edge.
        CGWarpMouseCursorPosition(CGPoint(x: screen.frame.midX, y: H - screen.frame.maxY))
        for i in 0..<6 { try? await Task.sleep(for: .milliseconds(60)); capture("2-reveal-\(i)"); log("reveal \(i): toolbarReveal \(model.toolbarReveal), shift \(model.revealShift)"); islands("reveal \(i)") }
        try? await Task.sleep(for: .milliseconds(400))
        geometry("revealed")
        islands("revealed")
        capture("3-revealed")
        CGWarpMouseCursorPosition(CGPoint(x: screen.frame.midX, y: H - screen.frame.midY))
        try? await Task.sleep(for: .seconds(1.0))
        geometry("reveal gone")
        // Left-edge peek with the sidebar hidden: frame times and CPU first (no captures), then pictures.
        model.toggleSidebar()
        try? await Task.sleep(for: .seconds(0.7))
        CGWarpMouseCursorPosition(CGPoint(x: screen.frame.minX, y: H - screen.frame.midY))
        let detail0 = ChromeProbe.detail
        let timer = FrameTimer(view: win.contentView!)
        let cpu0 = FrameTimer.cpu()
        model.setPeekHover(strip: true)   // what the strip's onHover calls (a warped pointer sends no mouse-entered)
        try? await Task.sleep(for: .seconds(0.7))
        let (frames, cpu) = (timer.stop(), FrameTimer.cpu() - cpu0)
        log(String(format: "peek: %d frames in 0.7 s, interval avg %.1f ms, max %.1f ms, over 1.5x nominal %d; CPU %.0f ms (%.0f%% of one core)",
                   frames.count, frames.avg, frames.max, frames.slow, cpu * 1000, cpu / 0.7 * 100))
        log("peek: panel \(r(ChromeProbe.peekPanel)), detail before \(r(detail0)) after \(r(ChromeProbe.detail)) (content must not move)")
        capture("4-peek")
        model.endPeek()
        try? await Task.sleep(for: .seconds(0.5))
        model.setPeekHover(strip: true)
        for i in 0..<5 { try? await Task.sleep(for: .milliseconds(i == 0 ? 150 : 50)); capture("5-peek-slide-\(i)") }
        model.endPeek(); model.toggleSidebar()
        CGWarpMouseCursorPosition(CGPoint(x: screen.frame.midX, y: H - screen.frame.midY))
        win.toggleFullScreen(nil)
        try? await Task.sleep(for: .seconds(1.2))
        CGWarpMouseCursorPosition(CGPoint(x: pointer.x, y: H - pointer.y))
        log("done, full screen \(win.styleMask.contains(.fullScreen))")
        exit(0)
    }
}

/// Display-link frame timestamps of a view's screen, and the process CPU time.
@MainActor final class FrameTimer: NSObject {
    private var link: CADisplayLink?
    private var stamps: [CFTimeInterval] = []
    private var nominal: CFTimeInterval = 1.0 / 120
    init(view: NSView) {
        super.init()
        link = view.displayLink(target: self, selector: #selector(tick(_:)))
        link?.add(to: .main, forMode: .common)
    }
    @objc private func tick(_ l: CADisplayLink) { stamps.append(l.timestamp); nominal = l.duration > 0 ? l.duration : nominal }
    struct Stats { var count = 0, avg = 0.0, max = 0.0, slow = 0 }
    func stop() -> Stats {
        link?.invalidate(); link = nil
        let d = zip(stamps.dropFirst(), stamps).map { ($0 - $1) * 1000 }
        guard !d.isEmpty else { return Stats() }
        return Stats(count: stamps.count, avg: d.reduce(0, +) / Double(d.count), max: d.max() ?? 0, slow: d.filter { $0 > nominal * 1500 }.count)
    }
    nonisolated static func cpu() -> Double {
        var u = rusage(); getrusage(RUSAGE_SELF, &u)
        func s(_ t: timeval) -> Double { Double(t.tv_sec) + Double(t.tv_usec) / 1e6 }
        return s(u.ru_utime) + s(u.ru_stime)
    }
}
