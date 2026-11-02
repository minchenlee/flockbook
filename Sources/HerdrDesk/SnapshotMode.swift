import HerdrKit
import SwiftTerm
import SwiftUI
import AppKit

/// `HerdrDesk --snapshot <dir>`: render preview screens off-screen to PNG (no screen-recording permission needed).
/// Glass materials (.glassEffect, .bar, .thinMaterial) may not render off-screen.
@MainActor
enum SnapshotRunner {
    struct Case { let name: String; let screen: Screen; let dark: Bool; let tab: Int; let terminal: Bool; var search = ""; var real = false; var zoom = false; var quiet = false; var group: String? = nil; var dense = false; var sidebarW: CGFloat = 250; var pane: String? = nil }

    static func run(dir: String) {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let cases = [
            Case(name: "main-light", screen: .tab, dark: false, tab: 0, terminal: false),
            Case(name: "main-dark", screen: .tab, dark: true, tab: 0, terminal: false),
            Case(name: "blocked-tab", screen: .tab, dark: false, tab: 1, terminal: false),
            Case(name: "inbox", screen: .inbox, dark: false, tab: 0, terminal: false),
            Case(name: "inbox-empty", screen: .inbox, dark: false, tab: 0, terminal: false, quiet: true),
            Case(name: "overview-by-workspace", screen: .overview, dark: false, tab: 0, terminal: false, group: "workspace"),
            Case(name: "overview", screen: .overview, dark: false, tab: 0, terminal: false),
            Case(name: "overview-dark", screen: .overview, dark: true, tab: 0, terminal: false),
            Case(name: "real-main", screen: .tab, dark: false, tab: 0, terminal: false, real: true),
            Case(name: "zoom-before", screen: .tab, dark: false, tab: 0, terminal: false),
            Case(name: "zoom-after", screen: .tab, dark: false, tab: 0, terminal: false, zoom: true),
            Case(name: "sidebar-250", screen: .tab, dark: false, tab: 0, terminal: false, dense: true, sidebarW: 250),
            Case(name: "sidebar-320", screen: .tab, dark: false, tab: 0, terminal: false, dense: true, sidebarW: 320),
            Case(name: "search", screen: .search, dark: false, tab: 0, terminal: false, search: "mob"),
        ]
        for c in cases { render(c, dir: dir) }
        print("snapshots written to \(dir)")
        NSApp.terminate(nil)
    }

    /// The × on the selected tab must exist, its action must call closeTab for that tab (hook: no real tab closes), a click
    /// at its centre must reach the tab strip (not the title bar drag area), and with --read-only there must be no ×.
    /// The offscreen window is never key, so SwiftUI drops synthetic mouse events (even on plain tab selection); the
    /// action is therefore run through the probe, and the hit area is checked with hitTest.
    static func runCloseTabTest() async {
        let readOnly = CommandLine.arguments.contains("--read-only")
        let model = AppModel(store: .preview)
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        var closed: [String] = []
        model.closeTabHook = { closed.append($0.id) }
        try? await Task.sleep(for: .seconds(2))
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("closetab-test: \(n) \(ok ? "ok" : "FAIL")") }
        guard let selected = model.tab, let frame = win.contentView?.superview else { print("closetab-test: no tab"); NSApp.terminate(nil); return }
        if readOnly {
            step("read-only: no close button", CloseProbe.frames.isEmpty)
        } else {
            let box = CloseProbe.frames[selected.id]
            step("selected tab has a close button \(box.map { "\($0)" } ?? "none")", box != nil)
            if let box, let action = CloseProbe.actions[selected.id] {
                let cv = win.contentView!
                let pt = cv.convert(NSPoint(x: box.midX, y: cv.isFlipped ? box.midY : cv.bounds.height - box.midY), to: nil)
                var v = frame.hitTest(pt); var chain: [String] = []
                while let cur = v, chain.count < 12 { chain.append(String(describing: type(of: cur))); v = cur.superview }
                step("click at the × reaches the tab strip, not the title bar (\(chain.prefix(3).joined(separator: " < ")))",
                     chain.contains { $0.contains("HostingScrollView") } && !chain.prefix(4).contains { $0.contains("NSTitlebar") })
                step("the × sits above the tab's mouse view (hit \(chain.first ?? "none"))", !(frame.hitTest(pt) is TabMouseView.Catcher))
                action()
                step("× action calls closeTab for that tab (got \(closed))", closed == [selected.id])
            } else { step("× action registered", false) }
        }
        print("closetab-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Tab rename by double click (real hit test, then the catcher view's own mouseDown with a click count of 2), windowed and
    /// full-screen carrier; right-click menu entries of tabs and workspaces; the New workspace button. Writes on and off.
    /// Nothing is sent to herdr (workspace writes go to a hook; tab rename compares equal labels).
    static func runMenusTest() async {
        let model = AppModel(store: .preview)
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        var calls: [String] = []
        model.workspaceHook = { calls.append("\($0) \($1)") }
        AppDefaults.store.removeObject(forKey: AppSettings.allowInputKey)
        try? await Task.sleep(for: .seconds(2))
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("menus-test: \(n) \(ok ? "ok" : "FAIL")") }
        guard let frame = win.contentView?.superview, let cv = win.contentView, let ws = model.workspace, ws.tabs.count > 1 else { print("menus-test: no window"); NSApp.terminate(nil); return }
        let zoomBefore = win.frame
        func chain(at box: CGRect) -> [NSView] {
            let pt = cv.convert(NSPoint(x: box.midX, y: cv.isFlipped ? box.midY : cv.bounds.height - box.midY), to: nil)
            var v = frame.hitTest(pt); var out: [NSView] = []
            while let cur = v, out.count < 14 { out.append(cur); v = cur.superview }
            return out
        }
        func doubleClick(_ tab: Tab, _ tag: String) async {
            model.renamingTabID = nil
            guard let box = TabProbe.labels[tab.id] else { step("\(tag): label frame of \(tab.label)", false); return }
            let views = chain(at: box)
            let catcher = views.compactMap { $0 as? TabMouseView.Catcher }.first
            step("\(tag): hit test at the tab label reaches the click view, not the title bar (\(views.prefix(3).map { String(describing: type(of: $0)) }.joined(separator: " < ")))",
                 catcher != nil && !views.prefix(3).contains { String(describing: type(of: $0)).contains("NSTitlebar") })
            guard let catcher else { return }
            step("\(tag): click view cannot move the window", !catcher.mouseDownCanMoveWindow)
            func ev(_ n: Int) -> NSEvent? {
                NSEvent.mouseEvent(with: .leftMouseDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: win.windowNumber, context: nil, eventNumber: 0, clickCount: n, pressure: 1)
            }
            if let e = ev(1) { catcher.mouseDown(with: e) }
            step("\(tag): single click selects \(tab.label) and does not rename", model.tab?.id == tab.id && model.renamingTabID == nil)
            if let e = ev(2) { catcher.mouseDown(with: e) }
            step("\(tag): double click sets renamingTabID", model.renamingTabID == tab.id)
            try? await Task.sleep(for: .milliseconds(500))
            model.rename(tab, to: tab.label)   // Return with the unchanged name
            step("\(tag): Return ends the rename", model.renamingTabID == nil)
            step("\(tag): window not zoomed by the double click (\(win.frame.size) vs \(zoomBefore.size), zoomed \(win.isZoomed))", win.frame.size == zoomBefore.size && !win.isZoomed)
            // Right click: the view under the pointer answers with the tab menu (not the toolbar's Icon and Text menu), at the
            // label and at the pill's left padding.
            try? await Task.sleep(for: .milliseconds(600))   // the rename field is gone and the frames are current
            let right = NSEvent.mouseEvent(with: .rightMouseDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: win.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)
            if let pill = TabProbe.pills[tab.id], let label = TabProbe.labels[tab.id] {
                let edge = chain(at: CGRect(x: pill.minX + 3, y: pill.midY, width: 0, height: 0))
                for (where_, views) in [("label", chain(at: label)), ("left padding", edge)] {
                    let hit = views.first
                    let menu = right.flatMap { hit?.menu(for: $0) }
                    step("\(tag): right click at the \(where_) gives the tab menu (\(menu?.items.map(\.title) ?? []), hit \(views.prefix(3).map { String(describing: type(of: $0)).prefix(40) }.joined(separator: " < ")), pill \(pill), label \(label))",
                         menu?.items.map(\.title) == model.tabMenu(tab).map(\.title))
                }
            } else { step("\(tag): pill frame of \(tab.label)", false) }
        }
        await doubleClick(ws.tabs[1], "windowed")
        // Entries
        let t0 = ws.tabs[0]
        func titles(_ e: [MenuEntry]) -> [String] { e.map(\.title) }
        step("tab menu entries \(titles(model.tabMenu(t0)))", titles(model.tabMenu(t0)) == ["Rename…", "Move left", "Move right", "Close tab", "Split right", "Split down", "Show in Overview"])
        step("workspace menu entries \(titles(model.workspaceMenu(ws)))", titles(model.workspaceMenu(ws)) == ["Rename…", "New tab in workspace", "Close workspace…"])
        // Move left is off on the first tab only (the ends are checked by --test-tabkeys).
        step("writes on: every entry enabled", (model.tabMenu(t0).filter { $0.id != "moveLeft" } + model.workspaceMenu(ws)).allSatisfy(\.enabled))
        model.workspaceMenu(ws).first { $0.id == "close" }?.run()
        step("Close workspace… asks first (closingWorkspace set, nothing called: \(calls))", model.closingWorkspace?.id == ws.id && calls.isEmpty)
        model.close(ws)
        model.workspaceMenu(ws).first { $0.id == "newTab" }?.run()
        model.workspaceMenu(ws).first { $0.id == "rename" }?.run()
        step("Rename… starts the inline rename", model.renamingWorkspaceID == ws.id)
        model.rename(ws, to: "Renamed")
        model.createWorkspace(name: " New one ", cwd: NSHomeDirectory())
        step("workspace writes reach herdr's calls: \(calls)", calls == ["close \(ws.id)", "newTab \(ws.id)", "rename \(ws.id) Renamed", "create New one"])
        // New workspace button
        let nb = ChromeProbe.newWorkspace
        let nbChain = chain(at: nb)
        step("New workspace button is in the sidebar (frame \(nb)), hit by a button not the title bar (\(nbChain.prefix(2).map { String(describing: type(of: $0)) }.joined(separator: " < ")))",
             nb.width > 0 && !nbChain.prefix(3).contains { String(describing: type(of: $0)).contains("NSTitlebar") })
        ChromeProbe.newWorkspaceAction?()
        step("button action opens the New workspace sheet", model.newWorkspaceOpen)
        model.newWorkspaceOpen = false
        try? await Task.sleep(for: .milliseconds(800))
        // Full-screen carrier
        ChromeTests.emulateFullScreen(win, model, true)
        try? await Task.sleep(for: .seconds(1.2))
        await doubleClick(ws.tabs[0], "full screen")
        ChromeTests.emulateFullScreen(win, model, false)
        try? await Task.sleep(for: .milliseconds(800))
        // Hover reveal (the model calls the strip's and the panel's onHover make; real tracking areas need a real pointer).
        model.setSidebarVisibility(.all)
        try? await Task.sleep(for: .milliseconds(600))
        model.setPeekHover(strip: true); try? await Task.sleep(for: .milliseconds(300))
        step("peek: sidebar pinned visible, edge hover does nothing", !model.sidebarPeek)
        model.setPeekHover(strip: false)
        model.setSidebarVisibility(.detailOnly)
        try? await Task.sleep(for: .milliseconds(800))
        step("peek: strip at the left edge, ~7 pt wide (\(ChromeProbe.peekStrip))", ChromeProbe.peekStrip.minX <= 1 && (6...8).contains(ChromeProbe.peekStrip.width) && ChromeProbe.peekStrip.height > 400 && ChromeProbe.peekStrip.minY >= 11)
        // The rest and the retract are timers: a busy runner can only make them later, so each check times when the change
        // happens (a lower bound) instead of looking after a fixed sleep, which a late wake-up turns into a false fail.
        func timeUntil(_ ok: () -> Bool) async -> Duration? {
            let t = ContinuousClock.now
            while ContinuousClock.now - t < .seconds(2) { if ok() { return ContinuousClock.now - t }; try? await Task.sleep(for: .milliseconds(10)) }
            return nil
        }
        model.setPeekHover(strip: true)
        step("peek: not shown at once", !model.sidebarPeek)
        let shownAfter = await timeUntil { model.sidebarPeek }
        step("peek: shown only after the 150 ms rest (after \(shownAfter.map { "\($0)" } ?? "never"))", (shownAfter ?? .zero) >= .milliseconds(150))
        step("peek: shown after resting on the strip; sidebarVisibility unchanged (still hidden)", model.sidebarPeek && model.sidebarVisibility == .detailOnly)
        try? await Task.sleep(for: .milliseconds(600))
        step("peek: floating panel is up (\(ChromeProbe.peekPanel)), content not pushed", ChromeProbe.peekPanel.width >= 240 && ChromeProbe.peekPanel.minX < 20)
        model.setPeekHover(strip: false, panel: true); try? await Task.sleep(for: .milliseconds(500))
        step("peek: stays while the pointer is on the panel", model.sidebarPeek)
        model.setPeekHover(panel: false)
        let goneAfter = await timeUntil { !model.sidebarPeek }
        step("peek: retracts 300 ms after leaving, not sooner (after \(goneAfter.map { "\($0)" } ?? "never"))", (goneAfter ?? .zero) >= .milliseconds(300))
        model.setPeekHover(strip: true); try? await Task.sleep(for: .milliseconds(400))
        model.select(workspace: ws.id)
        step("peek: choosing a workspace retracts it", !model.sidebarPeek)
        model.setPeekHover(strip: true); try? await Task.sleep(for: .milliseconds(400))
        if let split = WindowWidthTracker.splitView(in: win.contentView), let controller = split.delegate as? NSSplitViewController {
            print("menus-trace: before pin model \(model.sidebarVisibility), native \(controller.splitViewItems.map { $0.isCollapsed }), window \(model.mainWindow === win)")
        }
        model.toggleSidebar()
        _ = await CheckWait.until { model.sidebarVisibility == .all && !model.sidebarPeek }
        step("peek: sidebar toggle / ⌘B pins it (visibility \(model.sidebarVisibility), peek \(model.sidebarPeek))", model.sidebarVisibility == .all && !model.sidebarPeek)
        model.fullScreen = true; model.setSidebarVisibility(.detailOnly); try? await Task.sleep(for: .milliseconds(600))
        model.setPeekHover(strip: true); try? await Task.sleep(for: .milliseconds(400))
        step("peek: works in full screen too (an overlay; AppKit's own edge reveal is off)", model.sidebarPeek)
        model.endPeek(); model.setSidebarVisibility(.all); model.fullScreen = false
        try? await Task.sleep(for: .milliseconds(600))
        // Writes off
        AppDefaults.store.set(false, forKey: AppSettings.allowInputKey)
        calls.removeAll()
        let off = model.tabMenu(t0) + model.workspaceMenu(ws)
        step("writes off: Rename/Close/Split/New tab entries disabled, Show in Overview stays",
             off.filter { $0.id != "overview" }.allSatisfy { !$0.enabled } && (model.tabMenu(t0).first { $0.id == "overview" }?.enabled ?? false))
        model.beginNewWorkspace()
        model.createWorkspace(name: "x", cwd: "")
        model.close(ws); model.rename(ws, to: "y"); model.newTab(in: ws); model.split(tab: t0, .right)
        step("writes off: New workspace does not open and no call goes out (\(calls))", !model.newWorkspaceOpen && calls.isEmpty)
        try? await Task.sleep(for: .milliseconds(800))
        step("writes off: the New workspace button is disabled (frame still shown)", ChromeProbe.newWorkspace.width > 0)
        AppDefaults.store.removeObject(forKey: AppSettings.allowInputKey)
        print("menus-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Tab drag to reorder: the TabDrag arithmetic, then real drags through the pill's mouse view (press, 25 drag steps, release)
    /// in the windowed and the full-screen layout. The dragged pill follows the pointer, the neighbour slides aside with an animation,
    /// the drop calls moveTab once with the final index and the strip keeps the new order until
    /// the snapshot has it (3 s fallback). Read-only: nothing moves, nothing is sent. Nothing reaches herdr (workspaceHook).
    static func runTabDragTest() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("tabdrag-test: \(n) \(ok ? "ok" : "FAIL")") }
        // Arithmetic. Widths a=100 b=80 c=120, spacing 2: slots a0 b102 c184.
        let w: [String: CGFloat] = ["a": 100, "b": 80, "c": 120]
        var d = TabDrag(id: "a", start: ["a", "b", "c"], dx: 30)
        step("small move keeps the order (\(d.nearestOrder(w, spacing: 2)))", d.nearestOrder(w, spacing: 2) == ["a", "b", "c"])
        d.dx = 60
        step("past half of b: a goes after b (\(d.nearestOrder(w, spacing: 2)))", d.nearestOrder(w, spacing: 2) == ["b", "a", "c"])
        d.dx = 400
        step("far right: a goes last (\(d.nearestOrder(w, spacing: 2)))", d.nearestOrder(w, spacing: 2) == ["b", "c", "a"])
        d.order = ["b", "a", "c"]; d.dx = 90
        step("offset keeps the pill under the pointer (\(d.offset(w, spacing: 2)) = 90 - 82)", abs(d.offset(w, spacing: 2) - 8) < 0.01)
        var back = TabDrag(id: "c", start: ["a", "b", "c"], dx: -200)
        back.order = back.nearestOrder(w, spacing: 2)
        step("leftwards: c goes first (\(back.order))", back.order == ["c", "a", "b"])
        let t = { (id: String) in Tab(id: id, label: id, number: 1, panes: [], status: .idle) }
        step("ordered: new tabs at the end, closed ones drop out",
             TopRow.ordered([t("a"), t("b"), t("d")], by: ["b", "c", "a"]).map(\.id) == ["b", "a", "d"])

        let model = AppModel(store: .preview)
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        var calls: [String] = []
        model.workspaceHook = { calls.append("\($0) \($1)") }
        AppDefaults.store.removeObject(forKey: AppSettings.allowInputKey)
        try? await Task.sleep(for: .seconds(2))
        guard let frame = win.contentView?.superview, let cv = win.contentView, let ws = model.workspace, ws.tabs.count >= 3 else {
            print("tabdrag-test: no window"); NSApp.terminate(nil); return
        }
        func catcher(_ id: String) -> TabMouseView.Catcher? {
            guard let box = TabProbe.pills[id] else { return nil }
            let pt = cv.convert(NSPoint(x: box.midX, y: cv.isFlipped ? box.midY : cv.bounds.height - box.midY), to: nil)
            var v = frame.hitTest(pt)
            while let cur = v { if let c = cur as? TabMouseView.Catcher { return c }; v = cur.superview }
            return nil
        }
        func order() -> [String] { (model.workspace?.tabs ?? []).map(\.id).sorted { (TabProbe.pills[$0]?.minX ?? 0) < (TabProbe.pills[$1]?.minX ?? 0) } }
        func run(_ tag: String) async {
            calls.removeAll()
            let ids = ws.tabs.map(\.id)
            let first = ids[0], second = ids[1]
            guard let c = catcher(first), let p0 = TabProbe.pills[first], let n0 = TabProbe.pills[second] else { step("\(tag): mouse view of the first tab", false); return }
            let winFrame = win.frame
            // Drag the first pill right until its leading edge is past the middle of the second one.
            let target = n0.width / 2 + 12
            c.press(at: 0)
            var follow = true
            for i in 1...25 {
                let dx = target * CGFloat(i) / 25
                c.drag(to: dx)
                try? await Task.sleep(for: .milliseconds(16))
                if let p = TabProbe.pills[first], i > 2, abs(p.minX - (p0.minX + dx)) > 2 { follow = false }
            }
            try? await Task.sleep(for: .milliseconds(300))
            let pAfter = TabProbe.pills[first] ?? .zero, nAfter = TabProbe.pills[second] ?? .zero
            step("\(tag): dragged pill follows the pointer (minX \(p0.minX) -> \(pAfter.minX), pointer +\(target))", follow && abs(pAfter.minX - (p0.minX + target)) <= 2)
            step("\(tag): the neighbour slid left into the first slot (\(n0.minX) -> \(nAfter.minX), slot \(p0.minX))", abs(nAfter.minX - p0.minX) <= 2)
            // Layout probes report end positions and an offscreen window renders no animation frames, so check the transaction:
            // the neighbour's change of position must carry an animation.
            step("\(tag): the neighbour's slide is animated (transaction animation: \(TabProbe.moveAnimated[second].map { "\($0)" } ?? "none"))", TabProbe.moveAnimated[second] == true)
            step("\(tag): nothing sent before the drop (\(calls))", calls.isEmpty)
            c.release()
            try? await Task.sleep(for: .milliseconds(400))
            step("\(tag): drop calls moveTab once with the final index (\(calls))", calls == ["moveTab \(first) 1"])
            step("\(tag): strip keeps the new order until the snapshot has it (\(order()))", Array(order().prefix(2)) == [second, first])
            step("\(tag): window did not move (\(win.frame.origin) vs \(winFrame.origin))", win.frame == winFrame)
            try? await Task.sleep(for: .seconds(3.2))
            step("\(tag): no snapshot change within 3 s: back to herdr's order (\(order()))", order() == ids)
        }
        await run("windowed")
        ChromeTests.emulateFullScreen(win, model, true)
        try? await Task.sleep(for: .seconds(1.2))
        await run("full screen layout")
        ChromeTests.emulateFullScreen(win, model, false)
        try? await Task.sleep(for: .seconds(1))
        // Read-only: a drag moves nothing and sends nothing.
        AppDefaults.store.set(false, forKey: AppSettings.allowInputKey)
        try? await Task.sleep(for: .milliseconds(500))
        calls.removeAll()
        let first = ws.tabs[0].id, before = TabProbe.pills[first] ?? .zero
        if let c = catcher(first) {
            c.press(at: 0)
            for i in 1...10 { c.drag(to: CGFloat(i) * 15); try? await Task.sleep(for: .milliseconds(16)) }
            let mid = TabProbe.pills[first] ?? .zero
            c.release()
            try? await Task.sleep(for: .milliseconds(400))
            step("read-only: the pill stays put (\(before.minX) -> \(mid.minX)) and nothing is sent (\(calls))", abs(mid.minX - before.minX) < 1 && calls.isEmpty)
        } else { step("read-only: mouse view of the first tab", false) }
        AppDefaults.store.removeObject(forKey: AppSettings.allowInputKey)
        print("tabdrag-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Show-terminal sheet against the named test session: resizes with `terminalSize`, the terminal's cols/rows follow,
    /// the Close button exists and dismisses. Never uses the default session.
    static func runSheetTest(store: HerdrStore, paneId: String) async {
        guard store.session.name != nil else { print("sheet-test: refuses the default session"); NSApp.terminate(nil); return }
        AppDefaults.store.removeObject(forKey: TerminalSheetSize.key)
        try? await Task.sleep(for: .seconds(2))
        guard let (ws, tab) = store.workspaces.flatMap({ w in w.tabs.map { (w, $0) } }).first(where: { $0.1.panes.contains { $0.id == paneId } }) else {
            print("sheet-test: pane \(paneId) not found"); NSApp.terminate(nil); return
        }
        let model = AppModel(store: store)
        model.select(tab: tab, in: ws)
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(2))
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("sheet-test: \(n) \(ok ? "ok" : "FAIL")") }
        model.terminalSize = TerminalSheetSize.standard
        model.showTerminal = true
        try? await Task.sleep(for: .seconds(2.5))
        func dims() -> (Int, Int)? { ChromeProbe.sheetHandle?.live.map { ($0.getTerminal().cols, $0.getTerminal().rows) } }
        func sheetSize() -> CGSize? { win.attachedSheet?.contentView?.bounds.size }
        func sheetFrame() -> String { win.attachedSheet.map { "\($0.frame)" } ?? "none" }
        let s0 = sheetSize(), d0 = dims()
        step("sheet is up (size \(s0.map { "\($0)" } ?? "none"), cols/rows \(d0.map { "\($0)" } ?? "none"))", s0 != nil && d0 != nil)
        model.terminalSize = CGSize(width: 640, height: 380)
        try? await Task.sleep(for: .seconds(1.5))
        let s1 = sheetSize(), d1 = dims()
        print("sheet-test: sheet frame after smaller: \(sheetFrame())")
        step("smaller: sheet \(s1.map { "\($0)" } ?? "none") cols/rows \(d1.map { "\($0)" } ?? "none")", (s1?.width ?? 0) < (s0?.width ?? 0) && (d1?.0 ?? 0) < (d0?.0 ?? 0) && (d1?.1 ?? 0) < (d0?.1 ?? 0))
        model.terminalSize = CGSize(width: 1100, height: 700)
        try? await Task.sleep(for: .seconds(1.5))
        let s2 = sheetSize(), d2 = dims()
        step("larger: sheet \(s2.map { "\($0)" } ?? "none") cols/rows \(d2.map { "\($0)" } ?? "none")", (s2?.width ?? 0) > (s0?.width ?? 0) && (d2?.0 ?? 0) > (d0?.0 ?? 0) && (d2?.1 ?? 0) > (d0?.1 ?? 0))
        model.terminalSize = CGSize(width: 5000, height: 5000)
        try? await Task.sleep(for: .seconds(1))
        let s3 = sheetSize()
        step("clamped to the window minus margin (\(s3.map { "\($0)" } ?? "none") in a \(model.windowSize) window)", (s3?.width ?? 9999) <= model.windowSize.width - 40 + 1 && (s3?.height ?? 9999) <= model.windowSize.height - 60 + 1)
        model.terminalSize = CGSize(width: 100, height: 100)
        try? await Task.sleep(for: .seconds(1))
        let s4 = sheetSize()
        step("minimum 600x360 (\(s4.map { "\($0)" } ?? "none"))", (s4?.width ?? 0) >= 599 && (s4?.height ?? 0) >= 359)
        // Edge drags, through the same function the grips call (screen coordinates; the real sheet window moves). The dragged edge must
        // end exactly under the pointer (+-2; the sheet stays centred, so the other edge mirrors): right edge +100 -> maxX +100, left edge -100 -> minX -100, bottom +100 down -> minY -100.
        // 20 steps of 5 pt, each measured against the press point, must end where one 100 pt drag ends. The opposite edge stays put.
        if let sheet = win.attachedSheet, let sw = model.sheetWindow, sw === sheet {
            model.windowSize = win.contentView?.bounds.size ?? model.windowSize
            model.terminalSize = CGSize(width: 800, height: 500)
            try? await Task.sleep(for: .seconds(1))
            let p0 = CGPoint(x: 1000, y: 500)
            func drag(_ edge: SheetDrag.Edge, dx: CGFloat, dy: CGFloat, steps: Int) async -> (NSRect, NSRect) {
                let before = sheet.frame
                let st = (frame: before, mouse: p0)
                for i in 1...steps {
                    let t = CGFloat(i) / CGFloat(steps)
                    SheetDrag.apply(edge: edge, start: st, mouse: CGPoint(x: p0.x + dx * t, y: p0.y + dy * t), window: sheet, model: model)
                    try? await Task.sleep(for: .milliseconds(40))
                }
                try? await Task.sleep(for: .milliseconds(500))
                model.terminalDragging = false
                let after = sheet.frame
                model.sheetTargetFrame = nil
                sheet.setFrame(before, display: true)   // back to the start for the next drag
                model.terminalSize = sheet.contentView?.bounds.size ?? model.terminalSize
                try? await Task.sleep(for: .milliseconds(500))
                return (before, after)
            }
            let (b1, a1) = await drag(.right, dx: 100, dy: 0, steps: 1)
            step("right edge +100 pt: maxX \(b1.maxX) -> \(a1.maxX), left edge \(b1.minX) -> \(a1.minX), height \(b1.height) -> \(a1.height)",
                 abs(a1.maxX - (b1.maxX + 100)) <= 2 && abs(a1.midX - b1.midX) <= 2 && abs(a1.height - b1.height) <= 2)
            let (b2, a2) = await drag(.right, dx: 100, dy: 0, steps: 20)
            step("20 steps of 5 pt = one 100 pt drag (maxX \(b2.maxX) -> \(a2.maxX))", abs(a2.maxX - (b2.maxX + 100)) <= 2 && abs(a2.maxX - a1.maxX + (b2.maxX - b1.maxX)) <= 2)
            let (b3, a3) = await drag(.left, dx: -100, dy: 0, steps: 1)
            step("left edge 100 pt left: minX \(b3.minX) -> \(a3.minX), right edge \(b3.maxX) -> \(a3.maxX)", abs(a3.minX - (b3.minX - 100)) <= 2 && abs(a3.midX - b3.midX) <= 2)
            let (b4, a4) = await drag(.bottom, dx: 0, dy: -100, steps: 1)   // pointer 100 pt down (screen y is up)
            step("bottom edge 100 pt down: minY \(b4.minY) -> \(a4.minY), top \(b4.maxY) -> \(a4.maxY)", abs(a4.minY - (b4.minY - 100)) <= 2 && abs(a4.midY - b4.midY) <= 2)
        } else { step("sheet window known for the drag test", false) }
        step("Close button registered", ChromeProbe.sheetClose != nil)
        ChromeProbe.sheetClose?()
        try? await Task.sleep(for: .seconds(1))
        step("Close dismisses the sheet", win.attachedSheet == nil && !model.showTerminal)
        AppDefaults.store.removeObject(forKey: TerminalSheetSize.key)
        print("sheet-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    nonisolated(unsafe) static var savePixelsHook: ((String) -> Void)?

    /// Window chrome, using the same `WindowChrome` code as the real window:
    /// (1) glass Off: the toolbar band and the content below it have the same colour (windowed and full-screen layout);
    /// (2) sidebar first row: windowed = same line as the traffic lights and right of them; full screen = near the top.
    static func runChromeTest() async {
        AppDefaults.store.set(WindowGlass.off.rawValue, forKey: WindowGlass.key)
        let model = AppModel(store: .preview)
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        // Wait for the first layout (island, sidebar panel, traffic lights measured), not a fixed time: under load it can
        // take longer than 2 s, and then every later step fails at once. The time taken is printed for flake reports.
        let start = Date()
        try? await Task.sleep(for: .seconds(1))
        while Date().timeIntervalSince(start) < 8, TabProbe.island.width == 0 || ChromeProbe.panel.height == 0 || model.lightsMaxX == 0 {
            try? await Task.sleep(for: .milliseconds(100))
        }
        try? await Task.sleep(for: .milliseconds(800))   // settle (the toolbar re-measures once after the first width)
        print(String(format: "chrome-test: first layout ready after %.1f s (island %.0f, panel %.0f, lights %.0f)",
                     Date().timeIntervalSince(start), TabProbe.island.width, ChromeProbe.panel.height, model.lightsMaxX))
        var failures = 0, failed: [String] = []
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1; failed.append(n) }; print("chrome-test: \(n) \(ok ? "ok" : "FAIL")") }
        guard let frame = win.contentView?.superview else { print("chrome-test: no frame"); NSApp.terminate(nil); return }
        step("chrome: fullSizeContentView + transparent title bar + no separator",
             win.styleMask.contains(.fullSizeContentView) && win.titlebarAppearsTransparent && win.titlebarSeparatorStyle == .none)
        func pixel(_ x: CGFloat, _ yFromTop: CGFloat) -> NSColor? {
            // The window server's own composite of this window (SwiftUI/toolbar layers draw only there).
            typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
            guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else { return nil }
            let f = unsafeBitCast(sym, to: Fn.self)
            guard let img = f(.null, 8 /* optionIncludingWindow */, UInt32(win.windowNumber), 1 /* boundsIgnoreFraming */)?.takeRetainedValue() else { return nil }
            let rep = NSBitmapImageRep(cgImage: img)
            if CommandLine.arguments.contains("--save-window") { try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "build/snapshots-glass/chrome-window.png")) }
            let k = CGFloat(rep.pixelsWide) / frame.bounds.width
            return rep.colorAt(x: Int(x * k), y: Int(yFromTop * k))?.usingColorSpace(.sRGB)
        }
        savePixelsHook = { path in
            typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
            guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else { return }
            let f = unsafeBitCast(sym, to: Fn.self)
            guard let img = f(.null, 8, UInt32(win.windowNumber), 1)?.takeRetainedValue() else { return }
            try? NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
        }
        func same(_ a: NSColor?, _ b: NSColor?) -> Bool {
            guard let a, let b else { return false }
            return abs(a.redComponent - b.redComponent) < 0.01 && abs(a.greenComponent - b.greenComponent) < 0.01 && abs(a.blueComponent - b.blueComponent) < 0.01
        }
        // Every tab whose first pane is an agent (a conversation): right of the tab group, inside the detail column, clear of
        // any item; the content sample is blank margin below. Split tabs (pane label) and single-pane tabs both count.
        func bandCheck(_ tag: String, bandY: CGFloat = 24) async {
            let x = size.width - 120
            for t in model.workspace?.tabs ?? [] where t.panes.first?.agent != nil {
                if let ws = model.workspace { model.select(tab: t, in: ws) }
                try? await Task.sleep(for: .milliseconds(900))
                let band = pixel(x, bandY), content = pixel(x, 300)
                step("\(tag) / tab \(t.label) (\(t.panes.count) pane\(t.panes.count == 1 ? "" : "s")): band \(band.map { String(format: "%.3f", $0.redComponent) } ?? "?") = content \(content.map { String(format: "%.3f", $0.redComponent) } ?? "?")", same(band, content) && (content?.redComponent ?? 0) > 0.05)
            }
        }
        await bandCheck("windowed")
        // One surface: the sidebar column and the content show the same colour (blank spots: below the workspace rows, right margin).
        func surfaceCheck(_ tag: String) {
            let p = ChromeProbe.panel
            let side = pixel(p.midX, size.height - 140), content = pixel(size.width - 120, size.height - 300)
            func hex(_ c: NSColor?) -> String { c.map { String(format: "%.3f/%.3f/%.3f", $0.redComponent, $0.greenComponent, $0.blueComponent) } ?? "?" }
            step("\(tag): sidebar column \(hex(side)) = content \(hex(content))", same(side, content))
        }
        surfaceCheck("windowed, glass Off")
        // Tab island: the selected pill sits `inset` from the capsule on all sides (first pill selected: left; last pill: right).
        func islandCheck(_ tag: String) {
            let island = TabProbe.island
            let pills = (model.workspace?.tabs ?? []).compactMap { TabProbe.pills[$0.id] }.filter { $0.width > 0 }
            guard let first = pills.min(by: { $0.minX < $1.minX }), let last = pills.max(by: { $0.maxX < $1.maxX }) else {
                step("\(tag): tab island has pills", false); return
            }
            let l = first.minX - island.minX, r = island.maxX - last.maxX, t = first.minY - island.minY, b = island.maxY - first.maxY
            step(String(format: "\(tag): tab island insets left %.1f right %.1f top %.1f bottom %.1f (%.0f, all equal)", l, r, t, b, TopRow.inset),
                 [l, r, t, b].allSatisfy { abs($0 - TopRow.inset) <= 0.5 })
        }
        let panelTop = ChromeProbe.panel.minY
        // Sidebar top. Windowed: the lights row has only the toggle, New tab is on the next row, every nav row shares one left edge.
        func leftEdges() -> Bool {
            let xs = ["New tab", "Needs you", "Overview", "Search"].compactMap { ChromeProbe.rows[$0]?.minX }
            return xs.count == 4 && (xs.max()! - xs.min()!) < 1
        }
        let lightsMid = model.lightsMidY, lightsMax = model.lightsMaxX
        let tg = ChromeProbe.toggle, nt = ChromeProbe.rows["New tab"] ?? .zero
        step("windowed: traffic lights measured (maxX \(lightsMax), midY \(lightsMid))", lightsMax > 0)
        step("windowed: toggle midY \(tg.midY) vs lights midY \(lightsMid) (<= 2), right of the lights (minX \(tg.minX))", abs(tg.midY - lightsMid) <= 2 && tg.minX > lightsMax)
        step("windowed: New tab starts below the toolbar row, at the list's spacing (gap \(nt.minY - tg.maxY), 3-40)", (3...40).contains(nt.minY - tg.maxY))
        step("windowed: New tab / Needs you / Overview / Search share one left edge", leftEdges())
        islandCheck("windowed")
        let wsEdge = ChromeProbe.rows["New tab"]?.minX ?? 0
        let islandMid = TabProbe.island.midY, islandMidX = TabProbe.island.midX
        // Full-screen layout: the same flag the willEnterFullScreen notification sets.
        ChromeTests.emulateFullScreen(win, model, true)
        try? await Task.sleep(for: .milliseconds(1500))
        // The system toggle is hidden with the toolbar in full screen: ours (FullScreenChrome) takes its place.
        let ntFS = ChromeProbe.rows["New tab"] ?? .zero, tgFS = ChromeProbe.fullScreenToggle
        step("full screen: sidebar panel top \(ChromeProbe.panel.minY) = windowed \(panelTop)", abs(ChromeProbe.panel.minY - panelTop) <= 1)
        surfaceCheck("full screen layout, glass Off")
        // Our toggle at the left edge, over the sidebar's icon column (the system's sits at the panel's trailing edge windowed).
        let iconCol = ChromeProbe.rows["icon:New tab"]?.midX ?? 0
        step("full screen: sidebar rows at the windowed place (New tab minY \(ntFS.minY) vs \(nt.minY)); our toggle at the left edge over the icon column (midX \(tgFS.midX) vs \(iconCol)), centred on the band (midY \(tgFS.midY) vs \(model.toolbarBand / 2))",
             abs(ntFS.minY - nt.minY) <= 1 && abs(tgFS.midX - iconCol) <= 1 && abs(tgFS.midY - model.toolbarBand / 2) <= 0.5 && tgFS.width == 36)
        step("full screen: nav rows share one left edge, same as windowed", leftEdges() && abs(ntFS.minX - wsEdge) < 1)
        // Full screen looks as windowed: the system toolbar is hidden until the menu bar slides in, so the tab island and its
        // buttons are also content (FullScreenTopRow), always visible, where the toolbar lays them out (the toolbar keeps its
        // copy, shown while revealed); the sidebar toggle is ours (FullScreenChrome); no window controls (the system's come
        // with the reveal). Offscreen this is the layout flag; the owner checks real full screen by eye.
        let islandW = TabProbe.island
        step("full screen: tab island is content and still a toolbar item (island \(islandW), buttons \(ChromeProbe.fullScreenActions))",
             Self.toolbarHasTabs(win) && islandW.width > 0 && ChromeProbe.fullScreenActions.width > 0)
        step("full screen: island centred on the toolbar's (\(islandW.midX), \(islandW.midY)) vs windowed (\(islandMidX), \(islandMid)) and measured (\(model.toolbarIsland.midX), \(model.toolbarIsland.midY))",
             abs(islandW.midY - islandMid) <= 1 && abs(islandW.midX - islandMidX) <= 1
             && abs(islandW.midX - model.toolbarIsland.midX) <= 1 && abs(islandW.midY - model.toolbarIsland.midY) <= 1)
        let acts = ChromeProbe.fullScreenActions
        step("full screen: buttons centred on the toolbar's (\(acts.midX), \(acts.midY)) vs (\(model.toolbarActions.midX), \(model.toolbarActions.midY))",
             abs(acts.midX - model.toolbarActions.midX) <= 1 && abs(acts.midY - model.toolbarActions.midY) <= 1)
        // The system toolbar revealed with the menu bar: ours step aside; content moves only by what it covers beyond the band.
        let detailTop = ChromeProbe.detail.minY
        model.toolbarReveal = model.toolbarBand; try? await Task.sleep(for: .milliseconds(500))
        step("full screen, toolbar revealed over the band (notched screen): ours hidden, content not moved (\(ChromeProbe.detail.minY) vs \(detailTop))",
             model.systemChromeRevealed && abs(ChromeProbe.detail.minY - detailTop) <= 0.5)
        step("full screen, revealed: the toolbar's tab row is the active one, the content copy is not",
             model.islandActive(.toolbar) && !model.islandActive(.content))
        model.toolbarReveal = model.toolbarBand + 24; try? await Task.sleep(for: .milliseconds(500))
        step("full screen, toolbar revealed below a 24 pt menu bar: content down 24 (\(ChromeProbe.detail.minY - detailTop))", abs(ChromeProbe.detail.minY - detailTop - 24) <= 0.5)
        model.toolbarReveal = 0; try? await Task.sleep(for: .milliseconds(500))
        step("full screen, reveal gone: content back (\(ChromeProbe.detail.minY))", abs(ChromeProbe.detail.minY - detailTop) <= 0.5 && !model.systemChromeRevealed)
        // A rename in progress moves between the copies with the reveal and keeps the text typed so far (one model).
        if let t = model.tab {
            model.renamingTabID = t.id; try? await Task.sleep(for: .milliseconds(400))
            model.renameDraft = "renamed-draft"
            model.toolbarReveal = model.toolbarBand; try? await Task.sleep(for: .milliseconds(400))
            let mid = (model.renamingTabID, model.renameDraft)
            model.toolbarReveal = 0; try? await Task.sleep(for: .milliseconds(400))
            step("full screen: a rename survives the reveal and back (renaming \(mid.0 ?? "nil") / \(model.renamingTabID ?? "nil"), text \(mid.1) / \(model.renameDraft))",
                 mid.0 == t.id && model.renamingTabID == t.id && mid.1 == "renamed-draft" && model.renameDraft == "renamed-draft")
            model.renamingTabID = nil; try? await Task.sleep(for: .milliseconds(300))
        }
        await bandCheck("full screen layout")
        islandCheck("full screen layout")
        // One left-edge reveal, ours, in both modes (an overlay: the content does not move). AppKit's own is off.
        func systemEdgeReveal() -> [Bool?] {
            guard let sv = WindowWidthTracker.splitView(in: win.contentView), let c = sv.delegate as? NSSplitViewController else { return [] }
            return c.splitViewItems.filter { $0.behavior == .sidebar }.map { $0.value(forKey: "revealsOnEdgeHoverInFullscreen") as? Bool }
        }
        step("full screen: AppKit's sidebar edge reveal is off (\(systemEdgeReveal()))", systemEdgeReveal().allSatisfy { $0 == false })
        model.setSidebarVisibility(.detailOnly)
        try? await Task.sleep(for: .milliseconds(900))
        step("full screen, sidebar hidden: our reveal strip on the left edge (\(ChromeProbe.peekStrip))", ChromeProbe.peekStrip.width > 0 && ChromeProbe.peekStrip.minX < 1)
        let hiddenToggle = ChromeProbe.fullScreenToggle
        step("full screen, sidebar hidden: our toggle stays where it is with the sidebar in (\(hiddenToggle) vs \(tgFS)); the system's shows at x \(model.toggleHidden.frame.minX)-\(model.toggleHidden.frame.maxX) when revealed",
             hiddenToggle == tgFS)
        let detailBefore = ChromeProbe.detail
        model.setPeekHover(strip: true)
        try? await Task.sleep(for: .milliseconds(600))
        step("full screen: our peek panel shows (\(ChromeProbe.peekPanel)) over the content, which does not move (\(ChromeProbe.detail.minX) vs \(detailBefore.minX))",
             ChromeProbe.peekPanel.width > 0 && ChromeProbe.detail == detailBefore)
        model.endPeek()
        try? await Task.sleep(for: .milliseconds(500))
        // Show and hide with the toggle: the panel's slide is the only motion; the toggle does not move, and nothing writes
        // the panel edge per frame. Display-link frames and CPU over the slide (offscreen window: the owner judges by eye).
        for name in ["show", "hide"] {
            let m = await ChromeTests.sampleToggle(win, model)
            // The visibility follows AppKit's slide when it ends, later than the 0.5 s of samples on a busy runner: wait for
            // it (3 s at most) before checking it and before the next toggle, which a slide still running could swallow.
            let want: NavigationSplitViewVisibility = name == "hide" ? .detailOnly : .all, settle = Date()
            while Date().timeIntervalSince(settle) < 3, model.sidebarVisibility != want { try? await Task.sleep(for: .milliseconds(50)) }
            print(String(format: "chrome-test: toggle %@: %d display frames in 0.5 s, interval avg %.1f ms, max %.1f ms, over 1.5x nominal %d; CPU %.0f ms (%.0f%% of one core)",
                         name, m.frames.count, m.frames.avg, m.frames.max, m.frames.slow, m.cpu * 1000, m.cpu / 0.5 * 100))
            print("chrome-test: toggle \(name): window content x during the slide \(Set(m.roots.map { String(format: "%.1f", $0) }).sorted()) (the test's fixed-width root, overflowed by the split for a moment)")
            step("full screen, toggle \(name): the toggle stays put in the window content (\(Set(m.toggles.map { "\($0)" }).count) frame value(s) over \(m.toggles.count) samples), panel edge written \(m.edgeWrites)x during the slide, sidebar \(model.sidebarVisibility == .detailOnly ? "hidden" : "shown") after",
                 Set(m.toggles.map { "\($0)" }).count == 1 && m.edgeWrites <= 1 && (model.sidebarVisibility == .detailOnly) == (name == "hide"))
        }
        ChromeTests.emulateFullScreen(win, model, false)
        try? await Task.sleep(for: .milliseconds(900))
        let strip = ChromeProbe.peekStrip
        step("windowed, sidebar hidden: our reveal strip on the left edge (\(strip))", strip.width > 0 && strip.minX < 1)
        model.setPeekHover(strip: true)
        try? await Task.sleep(for: .milliseconds(900))
        let peek = ChromeProbe.peekPanel
        step("windowed: our reveal shows the sidebar panel (\(peek)) at the docked panel's top (\(panelTop))", peek.width > 0 && abs(peek.minY - panelTop) <= 1)
        model.endPeek(); model.setSidebarVisibility(.all)
        try? await Task.sleep(for: .milliseconds(900))
        ChromeTests.emulateFullScreen(win, model, false)
        try? await Task.sleep(for: .milliseconds(600))
        step("leaving full screen restores the lights row (toggle midY \(ChromeProbe.toggle.midY))", abs(ChromeProbe.toggle.midY - lightsMid) <= 2)
        step("windowed: tab island in the system toolbar", win.toolbar?.isVisible == true && Self.toolbarHasTabs(win))
        // Pictures of the toolbar area with the default glass (Full): windowed layout and full-screen layout.
        AppDefaults.store.removeObject(forKey: WindowGlass.key)
        for (name, fs) in [("windowed", false), ("fullscreen-layout", true)] {
            ChromeTests.emulateFullScreen(win, model, fs)
            try? await Task.sleep(for: .milliseconds(900))
            savePixelsHook?("build/snapshots-glass/chrome-\(name).png")
            surfaceCheck("glass Full, \(name)")
        }
        print("chrome-test: pictures in build/snapshots-glass/chrome-windowed.png, chrome-fullscreen-layout.png (window-server capture; full-screen is the layout flag, not a real full-screen window)")
        for n in failed { print("chrome-test: failed step: \(n)") }
        print("chrome-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Whether a tab label (TabProbe) lies inside the window's toolbar views.
    static func toolbarHasTabs(_ win: NSWindow) -> Bool {
        (win.toolbar?.items ?? []).contains { item in
            guard let v = item.view else { return false }
            func walk(_ v: NSView) -> Bool { v is TabMouseView.Catcher || v.subviews.contains(where: walk) }
            return walk(v)
        }
    }

    /// Headless hit test: is the tab row in the title bar area reachable by clicks (not swallowed by the title bar view)?
    static func runHitTest() async {
        let model = AppModel(store: .preview)
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(2))
        guard let frame = win.contentView?.superview else { print("hittest: no frame view"); NSApp.terminate(nil); return }
        if let rep = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) {
            frame.cacheDisplay(in: frame.bounds, to: rep)
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "build/snapshots-new/toolbar-window.png"))
        }
        print("hittest: toolbar=\(win.toolbar != nil) items=\(win.toolbar?.items.count ?? 0)")
        var seen: [String: Int] = [:]
        for x in stride(from: model.sidebarVisibility == .detailOnly ? 100 : 340, to: 1500, by: 10) {
            let pt = NSPoint(x: CGFloat(x), y: frame.bounds.height - 26)   // 26pt below the top edge
            var v = frame.hitTest(pt)
            var chain: [String] = []
            while let cur = v, chain.count < 4 { chain.append(String(describing: type(of: cur)).prefix(40).description); v = cur.superview }
            seen[chain.joined(separator: " < "), default: 0] += 1
        }
        for (k, n) in seen.sorted(by: { $0.value > $1.value }) { print("hittest: \(n)x \(k)") }
        // Sidebar toggle: x=100 is inside the sidebar when visible, inside the detail when hidden.
        func probe() -> String {
            var v = frame.hitTest(NSPoint(x: 100, y: frame.bounds.height - 400)); var c: [String] = []
            while let cur = v, c.count < 3 { c.append(String(describing: type(of: cur)).prefix(30).description); v = cur.superview }
            return c.joined(separator: " < ")
        }
        let before = probe(); model.toggleSidebar(); try? await Task.sleep(for: .seconds(1.5))
        let hiddenProbe = probe(); model.toggleSidebar(); try? await Task.sleep(for: .seconds(1.5))
        let after = probe()
        print("hittest: sidebar visible: \(before)")
        print("hittest: sidebar hidden:  \(hiddenProbe)")
        print("hittest: sidebar restored: \(after == before ? "same as visible" : after)")
        // Toolbar must survive workspace and screen changes: item count and tab-strip hits after each step.
        func tabHits() -> String {
            var n = 0, titlebar = 0
            for x in stride(from: model.sidebarVisibility == .detailOnly ? 100 : 340, to: 1500, by: 10) {
                var v = frame.hitTest(NSPoint(x: CGFloat(x), y: frame.bounds.height - 26)); var c = 0
                while let cur = v, c < 4 { if String(describing: type(of: cur)).contains("NSTitlebarView") { titlebar += 1 }; if cur is NSScrollView || String(describing: type(of: cur)).contains("HostingScrollView") { n += 1; break }; v = cur.superview; c += 1 }
            }
            return "items=\(win.toolbar?.items.count ?? -1) tabHits=\(n) titlebar=\(titlebar)"
        }
        let wss = model.store.workspaces
        let steps: [(String, () -> Void)] = [
            ("start", {}),
            ("other workspace", { if wss.count > 1 { model.select(workspace: wss[1].id) } }),
            ("back to first", { if let w = wss.first { model.select(workspace: w.id) } }),
            ("overview", { model.screen = .overview }),
            ("tab screen", { model.screen = .tab }),
            ("inbox", { model.screen = .inbox }),
            ("tab screen", { model.screen = .tab }),
        ]
        for (name, act) in steps {
            act(); try? await Task.sleep(for: .seconds(1.2))
            print("hittest: [\(name)] \(tabHits())")
        }
        // Full-screen layout (the flag the willEnterFullScreen notification sets): the tabs are also content (hittable there)
        // and the toolbar keeps its copy, across selections.
        var fsFailures = 0
        ChromeTests.emulateFullScreen(win, model, true); try? await Task.sleep(for: .seconds(1.2))
        func fsState(_ tag: String) {
            let hits = tabHits()
            // The island is content in full screen (FullScreenTopRow), hittable; the toolbar keeps its copy for the reveal.
            let ok = toolbarHasTabs(win) && !hits.contains("tabHits=0") && ChromeProbe.fullScreenActions.width > 0 && ChromeProbe.rows.count > 0
            if !ok { fsFailures += 1 }
            print("hittest: [full screen / \(tag)] \(hits) tabsInToolbar=\(toolbarHasTabs(win)) \(ok ? "ok" : "FAIL")")
        }
        fsState("start")
        if let ws = model.workspace, let t = ws.tabs.last { model.select(tab: t, in: ws); try? await Task.sleep(for: .milliseconds(700)); fsState("tab \(t.label)") }
        ChromeTests.emulateFullScreen(win, model, false); try? await Task.sleep(for: .seconds(1.2))
        let backOk = win.toolbar?.isVisible == true && !tabHits().contains("tabHits=0")
        if !backOk { fsFailures += 1 }
        print("hittest: [back to windowed] \(tabHits()) \(backOk ? "ok" : "FAIL")")
        print("hittest: full screen carrier \(fsFailures == 0 ? "PASS" : "FAIL x\(fsFailures)")")
        // Overflow check: dense store (>= 15 tabs in one workspace), several window widths, switch back and forth.
        let dense = AppModel(store: .previewDense)
        host.rootView = RootView(flat: false).environment(dense).frame(width: size.width, height: size.height)
        try? await Task.sleep(for: .seconds(1.5))
        let dws = dense.store.workspaces
        let biggest = dws.max { $0.tabs.count < $1.tabs.count }
        let small = dws.min { $0.tabs.count < $1.tabs.count }
        print("hittest: dense workspaces=\(dws.count) biggest=\(biggest?.tabs.count ?? 0) tabs")
        func dump(_ tag: String) {
            guard let tb = win.toolbar else { return }
            let vis = Set((tb.visibleItems ?? []).map { ObjectIdentifier($0) })
            let d = tb.items.map { "\($0.itemIdentifier.rawValue.suffix(24))[\(vis.contains(ObjectIdentifier($0)) ? "V" : "-")w=\(Int($0.view?.frame.width ?? -1))]" }
            print("hittest: \(tag) \(d.joined(separator: " "))")
        }
        dump("dense-initial")
        var failures = 0
        func overflowState() -> (String, Bool) {
            guard let tb = win.toolbar else { return ("no toolbar", false) }
            let vis = Set((tb.visibleItems ?? []).map { ObjectIdentifier($0) })
            // The split-view separator is never "visible"; every other item must be.
            let hidden = tb.items.filter { !vis.contains(ObjectIdentifier($0)) && !$0.itemIdentifier.rawValue.contains("Separator") }
            let widest = Int(tb.items.compactMap { $0.view?.frame.width }.max() ?? 0)
            let hits = tabHits()
            return ("hidden=\(hidden.count) tabItemW=\(widest) \(hits)", hidden.isEmpty && !hits.contains("tabHits=0"))
        }
        for (w, side) in [900, 1200, 1600].flatMap({ w in [(w, true), (w, false)] }) {
            host.rootView = RootView(flat: false).environment(dense).frame(width: CGFloat(w), height: 800)
            win.setContentSize(NSSize(width: CGFloat(w), height: 800)); try? await Task.sleep(for: .seconds(0.6))
            dense.setSidebarVisibility(side ? .all : .detailOnly)
            try? await Task.sleep(for: .seconds(0.8))
            for round in 1...2 {
                if let b = biggest { dense.select(workspace: b.id) }
                try? await Task.sleep(for: .seconds(0.6))
                let (a, okA) = overflowState()
                if let s = small { dense.select(workspace: s.id) }
                try? await Task.sleep(for: .seconds(0.6))
                let (b2, okB) = overflowState()
                if !okA || !okB { failures += 1 }
                print("hittest: w=\(w) sidebar=\(side ? "on" : "off") rowW=\(Int(dense.tabRowWidth)) round \(round) big[\(a)] \(okA ? "ok" : "FAIL") small[\(b2)] \(okB ? "ok" : "FAIL")")
            }
        }
        print("hittest: overflow check \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Headless layout stress: a conversation of mixed-height items that keeps growing, then an idle window whose CPU is measured.
    static func runStress() async {
        func cpu() -> Double { var u = rusage(); getrusage(RUSAGE_SELF, &u); return Double(u.ru_utime.tv_sec + u.ru_stime.tv_sec) + Double(u.ru_utime.tv_usec + u.ru_stime.tv_usec) / 1e6 }
        let table = "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |"
        func make(_ i: Int) -> ConversationItem {
            switch i % 5 {
            case 0: return ConversationItem(id: "u\(i)", kind: .userText(String(repeating: "hello 你好 ", count: 1 + i % 17)))
            case 1: return ConversationItem(id: "a\(i)", kind: .assistantText("Intro\n\n\(table)\n\n- one\n- two **bold**\n\n```\ncode \(i)\n```"))
            case 2: return ConversationItem(id: "t\(i)", kind: .toolChips((0..<(1 + i % 7)).map { ToolChip(symbol: "doc", text: "Read file \($0) of \(i)") }))
            case 3: return ConversationItem(id: "b\(i)", kind: .assistantText(String(repeating: "Long paragraph text. ", count: 5 + i % 40)))
            default: return ConversationItem(id: "s\(i)", kind: .status("Context compacted"))
            }
        }
        var items = (0..<300).map(make)
        let model = AppModel(store: .preview)
        let size = NSSize(width: 1000, height: 700)
        let box = StressBox(); box.items = items
        let host = NSHostingView(rootView: StressView(box: box).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        _ = model
        for k in 0..<60 {
            items.append(make(300 + k)); box.items = items
            if k % 10 == 0 { items.removeFirst(20); box.items = items }
            try? await Task.sleep(for: .milliseconds(50))
        }
        try? await Task.sleep(for: .seconds(1))
        let c0 = cpu(), t0 = Date()
        try? await Task.sleep(for: .seconds(4))
        let pct = (cpu() - c0) / Date().timeIntervalSince(t0) * 100
        print(String(format: "stress: idle CPU %.1f%% (items=%d)", pct, items.count))
        NSApp.terminate(nil)
    }

    /// Read-only: renders the live store (default session unless --session is given) once data has arrived.
    @MainActor static func runLive(dir: String, store: HerdrStore) async {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        Task { do { let ws = try await store.client.snapshot(); print("probe ok: \(ws.count)") } catch { print("probe error: \(error)") } }
        try? await Task.sleep(for: .seconds(2))
        // conversation(for:) starts tailing a log on first call, so touch every agent first and let the logs load.
        let agents = store.workspaces.flatMap(\.tabs).flatMap(\.panes).compactMap { $0.agent }
        // Keep every log: past `maxLiveLogs` the store releases the oldest, and the count below would re-open them empty.
        store.maxLiveLogs = max(store.maxLiveLogs, agents.count + 1)
        for a in agents { _ = store.conversation(for: a.sessionId) }
        try? await Task.sleep(for: .seconds(25))
        var withLog = 0, report: [String] = []
        for a in agents {
            let n = store.conversation(for: a.sessionId).count
            if n > 0 { withLog += 1 }
            report.append("\(a.name):\(a.sessionId == nil ? "nosession" : String(n))")
        }
        print("live: agents=\(agents.count) withConversation=\(withLog)")
        print("live counts: " + report.joined(separator: " "))
        var found = 0, zeroFound = 0, codex = 0, noSession = 0
        for a in agents {
            guard let id = a.sessionId else { noSession += 1; continue }
            let isCodex = a.kind == "codex"
            if isCodex { codex += 1 }
            let u = isCodex ? SessionLog.locateCodex(sessionId: id) : SessionLog.locate(sessionId: id)
            if u != nil { found += 1; if store.conversation(for: id).isEmpty { zeroFound += 1 } }
        }
        print("live diag: logFound=\(found) foundButEmpty=\(zeroFound) codex=\(codex) noSession=\(noSession)")
        print("live: connected=\(store.connected) workspaces=\(store.workspaces.count)")
        // The Overview cards' ages: each tab's newest agent log time, none for a tab without one.
        await store.refreshLogTimes()
        for ws in store.workspaces { for t in ws.tabs {
            print("live age: \(ws.label) / \(t.label) agents=\(t.panes.compactMap(\.agent).map(\.kind)) \(store.lastActivity(of: t).map { LogAge.label($0, now: Date()) } ?? "none")")
        } }
        // LIVE_PANE: shoot the tab holding this pane (default: the first tab).
        render(Case(name: "live-main", screen: .tab, dark: false, tab: 0, terminal: false, pane: ProcessInfo.processInfo.environment["LIVE_PANE"]), dir: dir, store: store)
        render(Case(name: "live-overview", screen: .overview, dark: false, tab: 0, terminal: false), dir: dir, store: store)
        NSApp.terminate(nil)
    }

    /// Writes through the embedded terminal into a pane of a NAMED session (never the default one) and reads it back.
    @MainActor static func runTerminalTest(store: HerdrStore, paneId: String) async {
        guard store.session.name != nil else { print("terminal-test: refuses the default session"); NSApp.terminate(nil); return }
        try? await Task.sleep(for: .seconds(2))
        guard let pane = store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId }) else {
            print("terminal-test: pane \(paneId) not found"); NSApp.terminate(nil); return
        }
        let size = NSSize(width: 900, height: 500)
        let host = NSHostingView(rootView: ObservedTerminal(client: store.client, paneId: pane.id, terminalId: pane.terminalId, writable: true, focused: true)
            .frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(2))
        func find(_ v: NSView) -> TerminalView? { if let t = v as? TerminalView { return t }; for c in v.subviews { if let t = find(c) { return t } }; return nil }
        guard let tv = find(host) else { print("terminal-test: no terminal view"); NSApp.terminate(nil); return }
        func text() -> String { String(decoding: tv.getTerminal().getBufferAsData(), as: UTF8.self) }
        print("terminal-test: frames received, buffer chars=\(text().count)")
        // typed text contains "$((40+2))"; only the shell's output contains "desk-42-ok"
        tv.send(data: ArraySlice(Array("echo desk-$((40+2))-ok\r".utf8)))
        try? await Task.sleep(for: .seconds(3))
        print("terminal-test: output seen = \(text().contains("desk-42-ok"))")
        let want = "size-\(tv.getTerminal().rows)x\(tv.getTerminal().cols)"
        tv.send(data: ArraySlice(Array("echo size-$(stty size | tr ' ' x)\r".utf8)))
        try? await Task.sleep(for: .seconds(3))
        print("terminal-test: view \(want); pty size echoed = \(text().contains(want))")
        NSApp.terminate(nil)
    }

    /// Long Chinese table cells must wrap fully (no ellipsis), rows grow with their content and never overlap.
    @MainActor static func runTableTest() async {
        let md = """
        | # | 問題 | INSP 的答案 |
        |---|---|---|
        | Q1 | 替代品處理的是成因，還是和公司一樣處理症狀？這是一個很長很長的問題文字用來測試換行 | 成因。減重藥治肥胖，INSP 只撐開呼吸道 |
        | Q2 | 公司的客戶中，有多少比例也能改用替代品？（重疊比例）再補一段很長的說明文字確保至少換成兩行以上 | 高。這類病人大多體重過重（數字未查證，需要再確認一次才能下結論） |
        | Q3 | 對客戶來說，替代品更便宜、更不痛或更方便嗎？ | 是。打針，對比開刀植入 |
        """
        AppDefaults.store.set(ReadingSize.m.rawValue, forKey: ReadingSize.key)
        let width: CGFloat = 520
        let host = NSHostingView(rootView: ZStack { Color(nsColor: .textBackgroundColor); MarkdownView(text: md).padding(.vertical, 4) }.frame(width: width).environment(\.flatGlass, true))
        host.frame = NSRect(x: 0, y: 0, width: width, height: 800)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(1))
        host.layoutSubtreeIfNeeded()
        let h = host.fittingSize.height
        let oneLine = ReadingSize.m.table * 1.3 + 14 + 0.5
        let singleLineTable = oneLine * 4
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("table-test: \(n) \(ok ? "ok" : "FAIL")") }
        print("table-test: height \(Int(h)), single-line rows would be \(Int(singleLineTable))")
        step("rows grow with wrapped content (not one line each)", h > singleLineTable + oneLine)
        let rows = TableLayoutProbe.rows["3x12"] ?? []
        step("rows measured: \(rows.count) \(rows.map { "\(Int($0.y))+\(Int($0.h))" })", rows.count == 4)
        var overlap = false
        for i in 1..<max(rows.count, 1) where rows[i].y < rows[i - 1].y + rows[i - 1].h - 0.5 { overlap = true }
        step("rows do not overlap", !rows.isEmpty && !overlap)
        step("every cell is at least as tall as its wrapped text", !TableLayoutProbe.misfit.contains("3x12"))
        host.frame = NSRect(x: 0, y: 0, width: width, height: ceil(h) + 8)
        Self.pump(host)
        if let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
            host.cacheDisplay(in: host.bounds, to: rep)
            try? FileManager.default.createDirectory(atPath: "build/snapshots-reading", withIntermediateDirectories: true)
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "build/snapshots-reading/table-wrap.png"))
        }
        print("table-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    static func pump(_ host: NSView) {
        for _ in 0..<10 { RunLoop.current.run(until: Date().addingTimeInterval(0.15)); host.layoutSubtreeIfNeeded(); host.displayIfNeeded() }
    }

    /// Wheel on a full-screen program in a writable view reaches the program through herdr (mouse mode on: SGR wheel
    /// reports); a read-only view sends nothing. Named sessions only, on a fresh pane (no scrollback).
    @MainActor static func runWheelTest(store: HerdrStore, paneId: String) async {
        guard store.session.name != nil else { print("wheel-test: refuses the default session"); NSApp.terminate(nil); return }
        try? await Task.sleep(for: .seconds(2))
        guard let pane = store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId }) else {
            print("wheel-test: pane \(paneId) not found"); NSApp.terminate(nil); return
        }
        let client = store.client
        func screen() async -> String { (try? await client.readScrollback(paneId: paneId, lines: 60, ansi: false).text) ?? "" }
        func count(_ s: String) -> Int { s.components(separatedBy: "[<64;1;1M").count - 1 }
        let size = NSSize(width: 900, height: 500)
        func host(_ writable: Bool, _ h: TerminalHandle) -> NSWindow {
            let v = NSHostingView(rootView: ObservedTerminal(client: client, paneId: pane.id, terminalId: pane.terminalId, writable: writable, focused: writable, handle: h).frame(width: size.width, height: size.height))
            v.frame = NSRect(origin: .zero, size: size)
            let w = OffscreenWindow(contentRect: v.frame, styleMask: [.titled], backing: .buffered, defer: false)
            w.contentView = v; w.setFrameOrigin(NSPoint(x: -3000, y: -3000)); w.orderFrontRegardless()
            return w
        }
        func wheel(_ dy: Int32, win: NSWindow, view: NSView) {
            let inWin = view.convert(NSPoint(x: view.bounds.midX, y: view.bounds.midY), to: nil)
            let screenPt = win.convertPoint(toScreen: inWin)
            let h = NSScreen.screens.first?.frame.height ?? 0
            guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: dy, wheel2: 0, wheel3: 0) else { return }
            cg.location = CGPoint(x: screenPt.x, y: h - screenPt.y)
            cg.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            if let ev = NSEvent(cgEvent: cg) { NSApp.sendEvent(ev) }
        }
        func find(_ v: NSView) -> PasteTerminalView? { if let t = v as? PasteTerminalView { return t }; for c in v.subviews { if let t = find(c) { return t } }; return nil }
        var failures = 0
        func step(_ name: String, _ ok: Bool) { if !ok { failures += 1 }; print("wheel-test: \(name) \(ok ? "ok" : "FAIL")") }
        let h1 = TerminalHandle()
        let w1 = host(true, h1)
        try? await Task.sleep(for: .seconds(2))
        guard let live = w1.contentView.flatMap(find) else { print("wheel-test: no view"); NSApp.terminate(nil); return }
        live.send(data: ArraySlice(Array("clear; printf '\\033[?1000h\\033[?1006h'; /bin/cat -v\r".utf8)))
        try? await Task.sleep(for: .seconds(2))
        let info = try? await client.scrollInfo(paneId: paneId)
        step("pane is a full-screen-like program (kind=\(String(describing: info?.kind)))", info?.kind == .fullScreenProgram)
        let c0 = count(await screen())
        wheel(60, win: w1, view: live)       // ~2 steps up
        try? await Task.sleep(for: .seconds(1.5))
        let c1 = count(await screen())
        step("writable: wheel reaches the program (\(c0) -> \(c1) wheel-up reports)", c1 - c0 >= 2)
        wheel(-60, win: w1, view: live)
        try? await Task.sleep(for: .seconds(1.5))
        step("writable: wheel down reports too", (await screen()).contains("[<65;1;1M"))
        w1.orderOut(nil); w1.contentView = nil
        // Read-only view: nothing is sent.
        let h2 = TerminalHandle()
        let w2 = host(false, h2)
        try? await Task.sleep(for: .seconds(2))
        if let ro = w2.contentView.flatMap(find) {
            let before = count(await screen())
            wheel(60, win: w2, view: ro)
            try? await Task.sleep(for: .seconds(1.5))
            step("read-only: wheel sends nothing", count(await screen()) == before)
        } else { step("read-only view exists", false) }
        try? await client.sendNamedKeys(["C-c"], to: paneId)
        try? await client.send(text: "printf '\\033[?1000l\\033[?1006l'", to: paneId)
        print("wheel-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// A fake agent that drops Enter for 500 ms after it sees an image path (like Claude Code converting it to an attachment).
    /// Old path (text + Enter at once) must fail to submit; MessageSender must submit.
    @MainActor static func runSendImageTest(store: HerdrStore, paneId: String) async {
        guard store.session.name != nil else { print("send-image-test: refuses the default session"); NSApp.terminate(nil); return }
        try? await Task.sleep(for: .seconds(2))
        let script = FileManager.default.temporaryDirectory.appendingPathComponent("fake-agent-\(UUID().uuidString).py")
        let py = """
        import sys, tty, termios, time, os, select
        fd = sys.stdin.fileno(); old = termios.tcgetattr(fd); tty.setraw(fd)
        buf = ""; busy_until = 0.0; shown = False
        def out(s): sys.stdout.write(s.replace("\\n", "\\r\\n")); sys.stdout.flush()
        out("FAKE-READY\\n")
        try:
            while True:
                if busy_until and time.time() >= busy_until and not shown:
                    shown = True; busy_until = 0.0; out("\\n[Image #1]\\n")
                r, _, _ = select.select([fd], [], [], 0.05)
                if not r: continue
                for ch in os.read(fd, 4096).decode(errors="ignore"):
                    if ch in "\\r\\n":
                        if busy_until and time.time() < busy_until: continue   # Enter dropped while converting
                        pasted = "\\x1b[200~" in buf
                        out("\\nSUBMITTED:" + ("(pasted) " if pasted else "") + buf.replace("\\x1b[200~", "").replace("\\x1b[201~", "") + "\\n"); buf = ""; shown = False
                    else:
                        buf += ch
                        if ".png" in buf and not shown and not busy_until: busy_until = time.time() + 0.5
        finally:
            termios.tcsetattr(fd, termios.TCSADRAIN, old)
        """
        try? py.write(to: script, atomically: true, encoding: .utf8)
        let client = store.client
        func screen() async -> String { (try? await client.readScrollback(paneId: paneId, lines: 60, ansi: false).text) ?? "" }
        var failures = 0
        func step(_ name: String, _ ok: Bool) { if !ok { failures += 1 }; print("send-image-test: \(name) \(ok ? "ok" : "FAIL")") }
        // A file of ours: only those are pasted (`ImagePaste.plan`).
        try? FileManager.default.createDirectory(at: PasteImages.directory, withIntermediateDirectories: true)
        let image = PasteImages.directory.appendingPathComponent(UUID().uuidString + ".png")
        FileManager.default.createFile(atPath: image.path, contents: Data([0x89]))
        let msg = "look at \(image.path)"
        // Control: the old single call (text + Enter) is swallowed by the fake agent.
        try? await client.send(text: "python3 \(script.path)", to: paneId)
        try? await Task.sleep(for: .seconds(2))
        step("fake agent running", await screen().contains("FAKE-READY"))
        try? await client.send(text: msg, to: paneId)
        try? await Task.sleep(for: .seconds(1.5))
        let oldSubmitted = await screen().contains("SUBMITTED:" + msg)
        step("control: old send is NOT submitted (reproduces the bug)", !oldSubmitted)
        // Clear the typed line, then the new path.
        try? await client.sendNamedKeys(["C-c"], to: paneId)
        try? await Task.sleep(for: .seconds(0.5))
        try? await client.send(text: "python3 \(script.path)", to: paneId)
        try? await Task.sleep(for: .seconds(2))
        await MessageSender.send(msg, to: paneId, client: client, kind: "claude")
        try? await Task.sleep(for: .seconds(1.5))
        let s = await screen()
        step("MessageSender submits after the attachment appears, the path pasted", s.contains("[Image #1]") && s.contains("SUBMITTED:(pasted) " + msg))
        try? await client.sendNamedKeys(["C-c"], to: paneId)
        PasteImages.remove(image)   // here, not in a defer: terminate below ends the process before a defer runs
        try? FileManager.default.removeItem(at: script)
        print("send-image-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// After each tab switch the conversation must rest at the bottom (newest message).
    @MainActor static func runBottomTest() async {
        let model = AppModel(store: .previewDense)
        let size = NSSize(width: 1200, height: 270)   // short window: the preview conversation overflows
        let host = NSHostingView(rootView: RootView(flat: true).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        func scrolls(_ v: NSView) -> [NSScrollView] { (v as? NSScrollView).map { [$0] } ?? [] + v.subviews.flatMap(scrolls) }
        // The conversation runs under the label and composer bars; its scroll view reserves them as content insets.
        func range(_ s: NSScrollView) -> CGFloat { (s.documentView?.frame.height ?? 0) + s.contentInsets.top + s.contentInsets.bottom - s.bounds.height }
        func conversation() -> NSScrollView? {   // the widest scroll area that can scroll (the sidebar list can scroll too)
            scrolls(host).filter { range($0) > 20 }.max(by: { $0.bounds.width < $1.bounds.width })
        }
        func gap() -> CGFloat? {   // distance from the bottom of the conversation; 0 = at bottom
            guard let s = conversation(), let d = s.documentView else { return nil }
            let visibleBottom = s.contentView.bounds.origin.y + s.contentView.bounds.height
            return d.isFlipped ? d.frame.height + s.contentInsets.bottom - visibleBottom : s.contentView.bounds.origin.y + s.contentInsets.bottom
        }
        try? await Task.sleep(for: .seconds(2))
        print("bottom-test: scroll views: \(scrolls(host).map { "\(Int($0.documentView?.frame.height ?? 0))/\(Int($0.bounds.height)) inset \(Int($0.contentInsets.bottom))" })")
        var failures = 0
        func check(_ name: String) {
            let g = gap()
            let ok = g.map { $0 < 4 } ?? false
            if !ok { failures += 1 }
            print("bottom-test: \(name) gap=\(g.map { String(format: "%.0f", $0) } ?? "no scroll area") \(ok ? "ok" : "FAIL")")
        }
        check("initial")
        let tabs = (model.workspace?.tabs ?? []).filter { $0.panes.count == 1 && $0.panes.first?.agent != nil }
        guard let ws = model.workspace, tabs.count >= 2 else { print("bottom-test: needs 2 tabs"); NSApp.terminate(nil); return }
        for round in 1...3 {
            // Scroll up first, so a switch that keeps the old offset is caught.
            if let s = conversation() { s.contentView.scroll(to: NSPoint(x: 0, y: -s.contentInsets.top)); s.reflectScrolledClipView(s.contentView) }
            print("bottom-test: after scrolling up gap=\(gap().map { String(format: "%.0f", $0) } ?? "-")")
            model.select(tab: tabs[round % 2], in: ws)
            try? await Task.sleep(for: .seconds(1.5))
            print("bottom-test: tab \(tabs[round % 2].label) panes=\(tabs[round % 2].panes.count) scrolls=\(scrolls(host).map { "\(Int($0.documentView?.frame.height ?? 0))/\(Int($0.bounds.height))" })")
            check("after switch \(round)")
        }
        // Send: scroll up, then a send token must bring the conversation to the bottom.
        if let s = conversation() { s.contentView.scroll(to: NSPoint(x: 0, y: -s.contentInsets.top)); s.reflectScrolledClipView(s.contentView) }
        try? await Task.sleep(for: .seconds(0.5))
        let upGap = gap() ?? 0
        if let p = model.tab?.panes.first { model.noteSent(p.id) }
        try? await Task.sleep(for: .seconds(1))
        let after = gap() ?? -1
        let sendOK = upGap > 20 && after < 4
        if !sendOK { failures += 1 }
        print("bottom-test: after send token gap \(Int(upGap)) -> \(Int(after)) \(sendOK ? "ok" : "FAIL")")
        print("bottom-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// History view: >200 lines of output, wheel-up opens it, line 1 is there, Back to live restores, and no input reached the pane.
    @MainActor static func runScrollTest(store: HerdrStore, paneId: String) async {
        guard store.session.name != nil else { print("scroll-test: refuses the default session"); NSApp.terminate(nil); return }
        try? await Task.sleep(for: .seconds(2))
        guard let pane = store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId }) else {
            print("scroll-test: pane \(paneId) not found"); NSApp.terminate(nil); return
        }
        let handle = TerminalHandle()
        let size = NSSize(width: 900, height: 500)
        let host = NSHostingView(rootView: ObservedTerminal(client: store.client, paneId: pane.id, terminalId: pane.terminalId, writable: true, focused: true, handle: handle)
            .frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(2))
        func find<T: NSView>(_ v: NSView, _ t: T.Type) -> T? { if let x = v as? T { return x }; for c in v.subviews { if let x = find(c, t) { return x } }; return nil }
        guard let live = find(host, PasteTerminalView.self) else { print("scroll-test: no terminal view"); NSApp.terminate(nil); return }
        func lines(_ tv: TerminalView) -> [String] { String(decoding: tv.getTerminal().getBufferAsData(), as: UTF8.self).split(whereSeparator: \.isNewline).map { $0.trimmingCharacters(in: .whitespaces) } }
        var failures = 0
        func step(_ name: String, _ ok: Bool) { if !ok { failures += 1 }; print("scroll-test: \(name) \(ok ? "ok" : "FAIL")") }
        live.send(data: ArraySlice(Array("clear; seq 1 300\r".utf8)))
        try? await Task.sleep(for: .seconds(3))
        let sentBefore = handle.sentBytes
        let sbBefore = try? await store.client.readScrollback(paneId: pane.id, lines: 1000, ansi: true)
        let infoBefore = try? await store.client.scrollInfo(paneId: pane.id)
        step("pane has history (kind=\(String(describing: infoBefore?.kind)), \(sbBefore?.lineCount ?? -1) lines)", infoBefore?.kind == .history && (sbBefore?.lineCount ?? 0) > 200)
        step("live view has no history before wheel", !handle.isHistory())
        // Real path: a pixel scroll-wheel event (finger/wheel toward older content) goes through NSApp.sendEvent,
        // where the local event monitor must pick it up.
        func wheel(_ dy: Int32, at view: NSView) -> NSEvent? {
            let inWin = view.convert(NSPoint(x: view.bounds.midX, y: view.bounds.midY), to: nil)
            let screen = win.convertPoint(toScreen: inWin)
            let primaryH = NSScreen.screens.first?.frame.height ?? 0
            guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: dy, wheel2: 0, wheel3: 0) else { return nil }
            cg.location = CGPoint(x: screen.x, y: primaryH - screen.y)
            cg.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(win.windowNumber))
            cg.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            return NSEvent(cgEvent: cg)
        }
        if let ev = wheel(40, at: live) {
            print("scroll-test: event window match=\(ev.window === win) deltaY=\(ev.scrollingDeltaY) precise=\(ev.hasPreciseScrollingDeltas)")
            NSApp.sendEvent(ev)
        } else { step("scroll event created", false) }
        try? await Task.sleep(for: .seconds(2))
        step("wheel event opens history view", handle.isHistory())
        if let hv = find(host, HistoryTerminalView.self) {
            let l = lines(hv)
            step("line 1 visible in history", l.contains("1"))
            step("line 300 visible in history", l.contains("300"))
            step("history starts at bottom", hv.scrollPosition >= 0.999)
        } else { step("history terminal view exists", false) }
        step("no bytes sent while history opens", handle.sentBytes == sentBefore)
        handle.backToLive()
        try? await Task.sleep(for: .seconds(1))
        step("back to live: history gone", !handle.isHistory() && find(host, HistoryTerminalView.self) == nil)
        let sbAfter = try? await store.client.readScrollback(paneId: pane.id, lines: 1000, ansi: true)
        let infoAfter = try? await store.client.scrollInfo(paneId: pane.id)
        step("pane untouched (same screen, offset 0, no input)", sbAfter?.text == sbBefore?.text && infoAfter?.offsetFromBottom == 0 && handle.sentBytes == sentBefore)
        live.send(data: ArraySlice(Array("echo live-$((40+2))-ok\r".utf8)))
        try? await Task.sleep(for: .seconds(3))
        step("live view works again", lines(live).contains { $0.contains("live-42-ok") })
        print("scroll-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Placeholder must show only when the composer is empty AND there is no IME composition.
    @MainActor static func runComposerTest() async {
        let probe = PlaceholderProbe()
        let size = NSSize(width: 700, height: 200)
        let host = NSHostingView(rootView: ComposerView(agentName: "testagent", onSend: { _ in }, probe: probe).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        func find(_ v: NSView) -> NSTextView? { if let t = v as? NSTextView { return t }; for c in v.subviews { if let t = find(c) { return t } }; return nil }
        func placeholderShown() -> Bool { probe.shown }
        try? await Task.sleep(for: .seconds(1))
        guard let tv = find(host) else { print("composer-test: no text view"); NSApp.terminate(nil); return }
        win.makeFirstResponder(tv)
        var failures = 0
        func check(_ name: String, _ expect: Bool) async {
            try? await Task.sleep(for: .seconds(0.6))
            let got = placeholderShown()
            if got != expect { failures += 1 }
            print("composer-test: \(name): placeholder=\(got) expected=\(expect) \(got == expect ? "ok" : "FAIL")")
        }
        await check("empty", true)
        tv.setMarkedText("ㄋㄧ", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        await check("IME composing", false)
        tv.insertText("你", replacementRange: NSRange(location: NSNotFound, length: 0))
        await check("after IME commit", false)
        tv.string = ""; tv.didChangeText()
        await check("cleared", true)
        tv.insertText("hello", replacementRange: NSRange(location: NSNotFound, length: 0))
        await check("typed text", false)
        // Draft cache: a composer built from a stored draft shows it, and reports edits.
        var reported: [ComposerDraft] = []
        let host2 = NSHostingView(rootView: ComposerView(agentName: "testagent", draft: ComposerDraft(text: "saved draft"),
                                                         onSend: { _ in }, onDraft: { reported.append($0) }).frame(width: size.width, height: size.height))
        host2.frame = NSRect(origin: .zero, size: size)
        win.contentView = host2
        try? await Task.sleep(for: .seconds(1))
        if let tv2 = find(host2) {
            let restored = tv2.string == "saved draft"
            win.makeFirstResponder(tv2)
            tv2.insertText("!", replacementRange: NSRange(location: NSNotFound, length: 0))
            try? await Task.sleep(for: .seconds(0.6))
            let reportedOK = reported.last?.text.hasSuffix("!") == true
            if !restored { failures += 1 }
            if !reportedOK { failures += 1 }
            print("composer-test: draft restored=\(restored) edit reported=\(reportedOK) \(restored && reportedOK ? "ok" : "FAIL")")
        } else { failures += 1; print("composer-test: draft FAIL no text view") }
        // AppModel draft bookkeeping.
        let m = AppModel(store: HerdrStore.preview)
        m.setDraft(ComposerDraft(text: "x"), for: "p1")
        var ok = m.drafts["p1"]?.text == "x"
        m.setDraft(ComposerDraft(), for: "p1")
        ok = ok && m.drafts["p1"] == nil
        m.setDraft(ComposerDraft(text: "gone"), for: "no-such-pane")
        m.prune()
        ok = ok && m.drafts["no-such-pane"] == nil
        if !ok { failures += 1 }
        print("composer-test: model drafts set/clear/prune \(ok ? "ok" : "FAIL")")
        // Image paste (private pasteboard; the user's clipboard is never touched). Test cache dir, not the real one.
        let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("herdr-desk-paste-test-\(UUID().uuidString)")
        PasteImages.directory = tmp
        defer { try? FileManager.default.removeItem(at: tmp) }
        let pb = NSPasteboard(name: NSPasteboard.Name("herdr-desk-test-\(UUID().uuidString)"))
        let img = NSImage(size: NSSize(width: 40, height: 30), flipped: false) { r in NSColor.systemOrange.setFill(); r.fill(); return true }
        var sent: [String] = []
        let probe3 = PlaceholderProbe()
        let host3 = NSHostingView(rootView: ComposerView(agentName: "testagent", onSend: { sent.append($0) }, probe: probe3, pasteboard: pb).frame(width: size.width, height: size.height))
        host3.frame = NSRect(origin: .zero, size: size)
        win.contentView = host3
        try? await Task.sleep(for: .seconds(1))
        func files() -> [URL] { (try? FileManager.default.contentsOfDirectory(at: tmp, includingPropertiesForKeys: nil)) ?? [] }
        func step(_ name: String, _ ok: Bool) { if !ok { failures += 1 }; print("composer-test: image \(name) \(ok ? "ok" : "FAIL")") }
        if let tv3 = find(host3) {
            win.makeFirstResponder(tv3)
            // 1. PNG data on the pasteboard.
            pb.clearContents(); pb.setData(img.tiffRepresentation.flatMap { NSBitmapImageRep(data: $0)?.representation(using: .png, properties: [:]) }, forType: .png)
            // Real path: Edit > Paste is enabled by validation (image-only pasteboard), then the action goes down the responder chain.
            let pasteItem = NSMenuItem(title: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
            step("Edit>Paste enabled for image-only pasteboard", tv3.validateUserInterfaceItem(pasteItem))
            _ = tv3.tryToPerform(#selector(NSText.paste(_:)), with: nil); try? await Task.sleep(for: .seconds(0.6))
            step("png paste writes file + chip", files().count == 1 && probe3.images.count == 1 && probe3.images[0].pathExtension == "png")
            // 2. TIFF only.
            pb.clearContents(); pb.setData(img.tiffRepresentation, forType: .tiff)
            tv3.paste(nil); try? await Task.sleep(for: .seconds(0.6))
            step("tiff paste adds chip", files().count == 2 && probe3.images.count == 2)
            // 3. Image file URL.
            let src = FileManager.default.temporaryDirectory.appendingPathComponent("src-\(UUID().uuidString).tiff")
            try? img.tiffRepresentation?.write(to: src)
            pb.clearContents(); pb.writeObjects([src as NSURL])
            tv3.paste(nil); try? await Task.sleep(for: .seconds(0.6))
            try? FileManager.default.removeItem(at: src)
            step("file URL paste adds chip", files().count == 3 && probe3.images.count == 3)
            // 4. Plain text still pastes as text.
            pb.clearContents(); pb.setString("plain words", forType: .string)
            tv3.paste(nil); try? await Task.sleep(for: .seconds(0.6))
            step("text paste unchanged", tv3.string == "plain words" && probe3.images.count == 3 && files().count == 3)
            // 5. Remove deletes our file.
            let first = probe3.images[0]
            probe3.removeFirst?(); try? await Task.sleep(for: .seconds(0.6))
            step("remove deletes cache file", probe3.images.count == 2 && !FileManager.default.fileExists(atPath: first.path) && files().count == 2)
            // 6. Send includes text + absolute paths.
            let keep = probe3.images
            tv3.insertNewline(nil); try? await Task.sleep(for: .seconds(0.6))
            step("send text has paths", sent.count == 1 && sent[0].hasPrefix("plain words ") && keep.allSatisfy { sent[0].contains($0.path) } && keep.allSatisfy { $0.path.hasPrefix("/") })
            step("chips cleared, files kept after send", probe3.images.isEmpty && files().count == 2)
            // 7. Purge removes only old files.
            let old = files()[0]
            try? FileManager.default.setAttributes([.modificationDate: Date().addingTimeInterval(-8 * 86400)], ofItemAtPath: old.path)
            PasteImages.purgeOld()
            step("purge removes >7 day files only", !FileManager.default.fileExists(atPath: old.path) && files().count == 1)
            // 8. Dropped or attached files: an image file (a screenshot's name has spaces) becomes a chip, as a paste
            // does (sent as a pasted path, then Enter after its marker); any other file still inserts its path.
            let shot = FileManager.default.temporaryDirectory.appendingPathComponent("Screenshot \(UUID().uuidString).png")
            let note = FileManager.default.temporaryDirectory.appendingPathComponent("note-\(UUID().uuidString).txt")
            try? img.tiffRepresentation.flatMap { NSBitmapImageRep(data: $0)?.representation(using: .png, properties: [:]) }?.write(to: shot)
            try? Data("hi".utf8).write(to: note)
            probe3.drop?([shot, note]); try? await Task.sleep(for: .seconds(0.6))
            step("dropped image becomes chip, other file inserts its path",
                 probe3.images.count == 1 && files().count == 2 && !tv3.string.contains(shot.lastPathComponent) && tv3.string.contains(note.path))
            tv3.insertNewline(nil); try? await Task.sleep(for: .seconds(0.6))
            let pasted = sent.count == 2 ? ImagePaste.plan(sent[1], directory: tmp.path).filter { if case .paste = $0 { true } else { false } }.count : 0
            step("dropped image sends as a pasted path", pasted == 1)
            // 9. A real drop (AppKit's drag hit test, FakeDrag): the screenshot file on the text and below it, and a file
            // promise (a screenshot's thumbnail) on the text. The glass card hides its content from that hit test.
            tv3.string = ""; probe3.text = ""
            final class Promise: NSObject, NSFilePromiseProviderDelegate {
                let src: URL; init(_ s: URL) { src = s }
                func filePromiseProvider(_ p: NSFilePromiseProvider, fileNameForType t: String) -> String { src.lastPathComponent }
                func filePromiseProvider(_ p: NSFilePromiseProvider, writePromiseTo url: URL, completionHandler: @escaping (Error?) -> Void) {
                    do { try FileManager.default.copyItem(at: src, to: url); completionHandler(nil) } catch { completionHandler(error) }
                }
            }
            let promise = Promise(shot)
            // A promise needs a real drag session: hand over the file as the provider would.
            let receive = FileDrop.receive
            FileDrop.receive = { _, dir, done in
                let u = dir.appendingPathComponent(shot.lastPathComponent)
                promise.filePromiseProvider(NSFilePromiseProvider(), writePromiseTo: u) { done($0 == nil ? u : nil) }
            }
            defer { FileDrop.receive = receive }
            // Points are read at drop time: a chip row moves the text.
            let onText = { tv3.convert(NSPoint(x: 20, y: tv3.bounds.midY), to: nil) }
            let belowText = { tv3.convert(NSPoint(x: 200, y: tv3.bounds.maxY + 16), to: nil) }
            for (name, at, writers) in [("file on the text", onText, [shot as NSURL] as [NSPasteboardWriting]),
                                        ("file below the text", belowText, [shot as NSURL]),
                                        ("promise on the text", onText, [NSFilePromiseProvider(fileType: "public.png", delegate: promise)])] {
                while probe3.images.count > 0 { probe3.removeFirst?(); try? await Task.sleep(for: .milliseconds(100)) }
                tv3.selectAll(nil); tv3.delete(nil); try? await Task.sleep(for: .milliseconds(400))
                let chips = probe3.images.count, point = at()
                let fd = FakeDrag(writers: writers, at: point, in: win)
                let view = fd.enter()
                try? await Task.sleep(for: .milliseconds(300))
                step("drag \(name): the drop outline shows", probe3.dropping)
                step("drag \(name): the outline is the card's edge (\(DropOutlineProbe.outline) vs \(DropOutlineProbe.card))",
                     DropOutlineProbe.outline != .zero && DropOutlineProbe.outline.equalTo(DropOutlineProbe.card))
                if name == "file on the text" {
                    try? FileManager.default.createDirectory(atPath: ContentTests.dir, withIntermediateDirectories: true)
                    ContentTests.capture(win, to: "\(ContentTests.dir)/composer-drop-outline\(NSApp.appearance?.name == .aqua ? "-light" : "").png")
                }
                DropOutlineProbe.outline = .zero
                let took = view.map(fd.finish) ?? false
                try? await Task.sleep(for: .milliseconds(600))
                step("drag \(name): taken by \(view.map { "\(type(of: $0))" } ?? "none"), outline gone", took && !probe3.dropping)
                step("drop \(name): a chip, no path", probe3.images.count == chips + 1 && !tv3.string.contains(shot.lastPathComponent))
            }
            // Another file dropped on the text: its path, quoted as the composer quotes it.
            tv3.selectAll(nil); tv3.delete(nil); try? await Task.sleep(for: .milliseconds(300))
            let other = FakeDrag(urls: [note], at: onText(), in: win), chips = probe3.images.count
            _ = other.enter().map(other.finish); try? await Task.sleep(for: .milliseconds(600))
            step("drop a text file on the text: its path, no chip", tv3.string.contains(note.path) && probe3.images.count == chips)
            try? FileManager.default.removeItem(at: shot); try? FileManager.default.removeItem(at: note)
            // 10. An agent on a host (`RemoteHost.Via.local`, a temp host folder). A dropped image is a chip as on this
            // Mac: the send copies it to the host (`MessageSender`, the same plan as below). Another file is copied to the
            // host first and its host path inserted.
            let hostDir = tmp.appendingPathComponent("host/paste").path
            let probe4 = PlaceholderProbe()
            var sent4: [String] = []
            let host4 = NSHostingView(rootView: ComposerView(agentName: "testagent", onSend: { sent4.append($0) }, probe: probe4, pasteboard: pb,
                                                              remote: RemoteHost(via: .local), uploadFolder: hostDir).frame(width: size.width, height: size.height))
            host4.frame = NSRect(origin: .zero, size: size)
            win.contentView = host4
            try? await Task.sleep(for: .seconds(1))
            let shot4 = FileManager.default.temporaryDirectory.appendingPathComponent("Screenshot \(UUID().uuidString).png")
            let note4 = FileManager.default.temporaryDirectory.appendingPathComponent("note-\(UUID().uuidString).txt")
            defer { try? FileManager.default.removeItem(at: shot4); try? FileManager.default.removeItem(at: note4) }
            try? img.tiffRepresentation.flatMap { NSBitmapImageRep(data: $0)?.representation(using: .png, properties: [:]) }?.write(to: shot4)
            try? Data("remote note".utf8).write(to: note4)
            probe4.drop?([shot4, note4])
            for _ in 0..<40 where !probe4.text.contains(hostDir) { try? await Task.sleep(for: .milliseconds(250)) }
            let onHost = ((try? FileManager.default.contentsOfDirectory(atPath: hostDir)) ?? []).map { "\(hostDir)/\($0)" }
            step("remote: dropped image becomes a chip", probe4.images.count == 1 && !probe4.text.contains(shot4.lastPathComponent))
            step("remote: another file is copied to the host and its host path inserted (\(probe4.text))",
                 !probe4.text.contains(note4.path) && onHost.contains { probe4.text.contains($0) && FileManager.default.contents(atPath: $0) == Data("remote note".utf8) })
            if let tv4 = find(host4) {
                win.makeFirstResponder(tv4)
                let chip = probe4.images.first
                tv4.insertNewline(nil); try? await Task.sleep(for: .seconds(0.6))
                var hostImage: String?
                if sent4.count == 1, let steps = try? await RemoteUpload(host: RemoteHost(via: .local), folder: hostDir).hostSteps(ImagePaste.plan(sent4[0], directory: tmp.path)) {
                    for case .paste(let p) in steps { hostImage = p }
                }
                step("remote: the chip's image goes to the host on send (\(hostImage ?? "none"))",
                     hostImage.map { h in h.hasPrefix(hostDir + "/") && chip.map { c in FileManager.default.contents(atPath: h) == FileManager.default.contents(atPath: c.path) } == true } == true)
            } else { step("remote: text view", false) }
        } else { failures += 1; print("composer-test: image FAIL no text view") }
        print("composer-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }

    /// Long reply sample in light and dark, for comparing with docs/design/07-reading.html.
    static func runReading(dir: String) {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let md = """
        ## Where the team stands

        Four agents are active. **One needs you**: `orchard-api-dev` wants to run a build. 中文段落也要好讀：這一行混合了 English words、`inline code` 與連結 [design notes](https://example.com/design)，行距要穩定。

        ### Next steps

        1. Review the open pull request
        2. Run the test suite
           - unit tests first
           - then the integration tests
        3. Merge when green

        - [x] Parser fixed
        - [ ] Release notes

        > Note: the build takes about two minutes. Do not stop the server.

        ```swift
        // keep the newest 150 rows
        struct Row: Identifiable { let id: String }
        let rows = items.suffix(150)   // 2 + 40 numbers
        for item in rows { print("row", item.id, "total: \\(rows.count)", 42, "a very long trailing string literal that makes this line scroll sideways") }
        ```

        | Agent | Status | Tests |
        |:--|:-:|--:|
        | orchard-main-orch | Idle | 120 |
        | orchard-api-dev | Needs you | 87 |
        | jm-api-dev | Working | 214 |

        Done. Tell me if you want the long version.
        """
        let t0 = Date()
        let items: [ConversationItem] = [
            ConversationItem(id: "u0", kind: .userText("Earlier question"), timestamp: t0.addingTimeInterval(-3600)),
            ConversationItem(id: "a0", kind: .assistantText("Short earlier answer."), timestamp: t0.addingTimeInterval(-3590)),
            ConversationItem(id: "u1", kind: .userText("Where is the team at? 請用中文整理。"), timestamp: t0),
            ConversationItem(id: "c1", kind: .toolChips([ToolChip(symbol: "doc", text: "Read 3 files"), ToolChip(symbol: "terminal", text: "Ran 2 commands")]), timestamp: t0.addingTimeInterval(10)),
            ConversationItem(id: "a1", kind: .assistantText(md), timestamp: t0.addingTimeInterval(134)),
        ]
        conversationShots(items, agentName: "orchard-main-orch", dir: dir, name: "reading", height: 1300)
        print("reading snapshots written to \(dir)")
        NSApp.terminate(nil)
    }

    /// `--snapshot-log <log.jsonl> <dir>`: a session log read the way the app reads it (SessionLog), shown in the
    /// conversation view offscreen: <dir>/log-light.png and log-dark.png.
    static func runLog(path: String, dir: String) {
        try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
        guard let data = FileManager.default.contents(atPath: path) else { print("snapshot-log: cannot read \(path)"); exit(2) }
        let lines = String(decoding: data, as: UTF8.self).split(separator: "\n").map(String.init)
        let items = SessionLog.parse(lines: lines)
        conversationShots(items, agentName: "claude", dir: dir, name: "log", height: 900)
        print("snapshot-log: \(items.count) items, written to \(dir)")
        NSApp.terminate(nil)
    }

    /// The conversation view with `items`, offscreen, light and dark: <dir>/<name>-light.png and <name>-dark.png.
    private static func conversationShots(_ items: [ConversationItem], agentName: String, dir: String, name: String, height: CGFloat) {
        for dark in [false, true] {
            AppDefaults.store.set(ReadingSize.m.rawValue, forKey: ReadingSize.key)
            let size = NSSize(width: 900, height: height)
            let view = ZStack { Color(nsColor: .textBackgroundColor); ConversationView(items: items, agentName: agentName, hasLog: true) }
                .environment(\.flatGlass, true).frame(width: size.width, height: size.height)
            let host = NSHostingView(rootView: view)
            host.frame = NSRect(origin: .zero, size: size)
            let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
            win.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
            for _ in 0..<12 { RunLoop.current.run(until: Date().addingTimeInterval(0.15)); host.layoutSubtreeIfNeeded(); host.displayIfNeeded() }
            if let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) {
                host.cacheDisplay(in: host.bounds, to: rep)
                try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: "\(dir)/\(name)-\(dark ? "dark" : "light").png"))
            }
            win.orderOut(nil)
        }
    }

    private static func render(_ c: Case, dir: String, store: HerdrStore = .preview) {
        AppDefaults.store.set(c.group ?? "status", forKey: "overviewGroup")
        let model = AppModel(store: c.quiet ? .previewQuiet : c.dense ? .previewDense : store)
        model.promptVisible = Set(store.workspaces.flatMap(\.tabs).flatMap(\.panes).filter { $0.agent?.status == .blocked }.map(\.id))
        model.screen = c.screen
        model.selectTab(index: c.tab)
        if let id = c.pane, let ws = store.workspaces.first(where: { $0.tabs.contains { $0.panes.contains { $0.id == id } } }),
           let t = ws.tabs.first(where: { $0.panes.contains { $0.id == id } }) { model.select(tab: t, in: ws) }
        model.screen = c.screen
        model.searchText = c.search
        if c.zoom, let t = model.tab, let p = t.panes.first { model.toggleZoom(p, in: t) }
        let size = NSSize(width: 1440, height: 900)
        let host = NSHostingView(rootView: RootView(flat: !c.real, flatSidebarWidth: c.sidebarW).environment(model).frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: c.real ? [.titled, .closable, .resizable] : [.titled], backing: .buffered, defer: false)
        if c.real { win.titleVisibility = .hidden }
        win.appearance = NSAppearance(named: c.dark ? .darkAqua : .aqua)
        win.contentView = host
        WindowChrome.apply(win)
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000))
        win.orderFrontRegardless()
        for _ in 0..<14 { RunLoop.current.run(until: Date().addingTimeInterval(0.15)); host.layoutSubtreeIfNeeded(); host.displayIfNeeded() }
        guard let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { return }
        host.cacheDisplay(in: host.bounds, to: rep)
        if let png = rep.representation(using: .png, properties: [:]) {
            try? png.write(to: URL(fileURLWithPath: "\(dir)/\(c.name).png"))
        }
        win.orderOut(nil)
    }
}

@MainActor @Observable final class StressBox { var items: [ConversationItem] = [] }
struct StressView: View {
    let box: StressBox
    var body: some View { ConversationView(items: box.items, agentName: "stress", hasLog: true) }
}
