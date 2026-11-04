import AppKit
import CoreImage
import HerdrKit
import SwiftUI
import SwiftTerm

/// Settings (the tab list and a tab's form) in a window of its own, offscreen, in every look it must survive:
///   .build/debug/HerdrDesk --snapshot-settings <dir>
/// -> <dir>/settings-<glass>-<light|dark>.png (the General tab) for Glass, Subtle and Opaque, and settings-reduce-<light|dark>.png
/// (Reduce Transparency, through a stand-in flag); then <dir>/settings-tab-<tab>-<light|dark>.png, one per tab, with Remote
/// access on. Each tab shows only its own controls (`SettingsTab.controls`), every control shows in some tab, and the
/// tab chosen last comes back. Offscreen there is no desktop and a behind-window blur samples nothing, so each still is
/// composed: a stand-in wallpaper, blurred as the material would, under a capture of the settings window alone. Only this
/// app's own windows are captured, never the screen.
/// Increase Contrast is not rendered here: offscreen, the high-contrast appearances draw the same pixels as
/// the plain ones. The view uses only system colors and controls, so the system handles it.
/// Own settings store (test isolation), no Dock icon, nothing sent anywhere.
extension SnapshotRunner {
    @MainActor static func runSettings(dir: String) async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("snapshot-settings: \(n) \(ok ? "ok" : "FAIL")") }
        let phone = TailscaleSetup.Peer(name: "Phone", os: "iOS", online: true)
        let ts = TailscaleSetup.Status(signedIn: true, backendState: "Running", dnsName: "mac.example.ts.net", magicDNS: true, certificates: true, peers: [phone])
        let hub = RemoteHub.preview(running: true, devices: [(HubDevice(id: "d1", name: "Phone", level: .input), Date(timeIntervalSince1970: 1_790_000_000))],
                                    offer: false, tailscale: ts, serve: .ours, mode: .tailscale)
        let direct = RemoteHub.preview(running: true, devices: [], offer: false)

        /// Shows `view` in an offscreen window, saves the composed still to `path` (when given), and returns its size
        /// and whether it drew; the window stays up for `inspect`, which reads the probes.
        func shoot(_ view: some View, glass: WindowGlass = .full, reduce: Bool = false, dark: Bool, to path: String?,
                   inspect: (NSWindow) async -> Void = { _ in }) async -> (size: NSSize, ink: Double, formInk: Double, bytes: Int) {
            AppDefaults.store.set(glass.rawValue, forKey: WindowGlass.key)
            // In the app the page sits on the main window's surface (WindowSurface); here its blur is the blurred wallpaper
            // below (offscreen the material view paints solid), so only its tint goes here.
            let surface = reduce ? WindowGlass.off : glass
            let host = NSHostingView(rootView: ZStack {
                Color(nsColor: .textBackgroundColor).opacity(surface.tint).ignoresSafeArea()
                view
            })
            let size = host.fittingSize
            let win = OffscreenWindow(contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled, .closable, .fullSizeContentView], backing: .buffered, defer: false)
            win.title = "Flockbook Settings"
            win.titlebarAppearsTransparent = true
            win.isOpaque = false; win.backgroundColor = .clear
            win.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            win.contentView = host
            // The form keeps clear of the title bar (its safe area), which the fitting size leaves out: add it, so a long
            // tab's last line is not cut.
            let bar = win.frame.height - win.contentLayoutRect.height
            if bar > 0 { win.setContentSize(NSSize(width: size.width, height: size.height + bar)) }
            win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
            try? await Task.sleep(for: .milliseconds(900))
            var ink = 1.0, formInk = 1.0, bytes = 0
            if let path {
                // The window's own capture must show content: a blank one (no screen-capture permission) is a FAIL,
                // not a wallpaper-only PASS.
                // The first window of a run can still be undrawn after 900 ms on a busy machine: capture again, up to 2 s
                // more. A window that never draws stays blank and fails.
                ink = compose(win, over: wallpaper(win.frame.size, dark: dark), blur: surface != .off, to: path)
                for _ in 0..<4 where ink <= 0.02 {
                    try? await Task.sleep(for: .milliseconds(500))
                    ink = compose(win, over: wallpaper(win.frame.size, dark: dark), blur: surface != .off, to: path)
                }
                bytes = (try? FileManager.default.attributesOfItem(atPath: path)[.size] as? Int) ?? 0
                // The form alone (the tab list and the window chrome draw without it).
                formInk = windowImage(win).flatMap { img in
                    let scale = CGFloat(img.width) / win.frame.width, f = SettingsProbe.content
                    return img.cropping(to: CGRect(x: f.minX * scale, y: f.minY * scale, width: f.width * scale, height: f.height * scale))
                }.map { Pixels($0).ink } ?? 0
            }
            await inspect(win)
            win.orderOut(nil); win.contentView = nil
            try? await Task.sleep(for: .milliseconds(100))
            return (size, ink, formInk, bytes)
        }
        func shown() -> Set<SettingsControl> { Set(SettingsProbe.controls.filter { $0.value.width > 0 }.keys) }
        func names(_ s: Set<SettingsControl>) -> String { s.map(\.rawValue).sorted().joined(separator: ",") }
        let width = SettingsSplit.listWidth + 521

        // The looks, on the General tab.
        let looks: [(String, WindowGlass, Bool)] = [("glass", .full, false), ("subtle", .subtle, false), ("opaque", .off, false), ("reduce", .full, true)]
        for (name, glass, reduce) in looks {
            for dark in [false, true] {
                let path = "\(dir)/settings-\(name)-\(dark ? "dark" : "light").png"
                let r = await shoot(SettingsSplit(fixed: .general, reduceTransparency: reduce ? true : nil), glass: glass, reduce: reduce, dark: dark, to: path)
                let ok = r.ink > 0.02 && r.bytes > 5_000 && abs(r.size.width - width) < 4 && r.size.height >= 420 && r.size.height < 900
                if !ok { failures += 1 }
                print("snapshot-settings: \(name)-\(dark ? "dark" : "light") \(Int(r.size.width))x\(Int(r.size.height)) pt, \(r.bytes / 1024) KB, window \(String(format: "%.1f", r.ink * 100))% ink \(ok ? "ok" : "FAIL")")
            }
        }

        // One still per tab; each tab shows its own controls and no other tab's. The form's own ink: the tab list and the
        // window chrome draw even when the form does not.
        var seen: Set<SettingsControl> = []
        for tab in SettingsTab.allCases {
            for dark in [false, true] {
                let path = "\(dir)/settings-tab-\(tab.rawValue)-\(dark ? "dark" : "light").png"
                var here: Set<SettingsControl> = [], selected: [SettingsTab] = []
                let r = await shoot(SettingsSplit(fixed: tab, remote: hub), dark: dark, to: path) { _ in
                    here = shown()
                    selected = SettingsTab.allCases.filter { SettingsProbe.tabs[$0].map { $0.width > 0 } ?? false }
                }
                seen.formUnion(here)
                step("tab \(tab.rawValue)-\(dark ? "dark" : "light"): \(Int(r.size.width))x\(Int(r.size.height)) pt, \(r.bytes / 1024) KB, \(String(format: "%.1f", r.ink * 100))% ink, form \(String(format: "%.2f", r.formInk * 100))%",
                     r.formInk > 0.01 && r.bytes > 5_000 && abs(r.size.width - width) < 4 && r.size.height >= 420 && r.size.height < 1500)
                step("tab \(tab.rawValue)-\(dark ? "dark" : "light"): shows [\(names(here))], all in the tab, list of \(selected.count) tabs",
                     !here.isEmpty && here.isSubset(of: Set(tab.controls)) && selected == SettingsTab.allCases)
            }
        }
        // The direct address (Remote access without Tailscale), and the Claude question hook only while Remote access is on.
        _ = await shoot(SettingsSplit(fixed: .remote, remote: direct), dark: false, to: nil) { _ in seen.formUnion(shown()) }
        var offAgents: Set<SettingsControl> = []
        _ = await shoot(SettingsSplit(fixed: .agents, remote: RemoteHub.preview(running: false, devices: [], offer: false)), dark: false, to: nil) { _ in offAgents = shown() }
        step("agents with Remote access off: [\(names(offAgents))], no question hook", offAgents == [.allowInput, .statusLine, .grokStatusLine])
        // Sparkle never starts in a check run (UpdaterGate), so the automatic-update switch cannot show here.
        let missing = Set(SettingsControl.allCases).subtracting(seen).subtracting([.autoUpdate])
        step("every control shows in some tab (missing: [\(names(missing))])", missing.isEmpty)

        // Backend (General): a pop-up with the four choices that shows the stored one. A pick (what the pop-up's
        // binding calls) is stored and the pop-up shows it. Offscreen a menu pick never reaches SwiftUI (no key window).
        func backendPopup(_ win: NSWindow) -> NSPopUpButton? {
            func popups(_ v: NSView) -> [NSPopUpButton] { (v as? NSPopUpButton).map { [$0] } ?? v.subviews.flatMap(popups) }
            let row = SettingsProbe.controls[.backend] ?? .zero
            guard let content = win.contentView else { return nil }
            return popups(content).first { b in
                let f = b.convert(b.bounds, to: nil)
                return row.contains(CGPoint(x: f.midX, y: content.bounds.height - f.midY))
            }
        }
        AppDefaults.store.set(BackendChoice.hcOnly.rawValue, forKey: BackendChoice.key)
        var items: [String] = [], picked = "", repicked = "", inGeneral = false
        _ = await shoot(SettingsSplit(fixed: .general), dark: false, to: nil) { win in
            inGeneral = shown().contains(.backend)
            items = backendPopup(win)?.itemTitles ?? []
            picked = backendPopup(win)?.titleOfSelectedItem ?? ""
            BackendSetting.pick(.both, store: nil)
            try? await Task.sleep(for: .milliseconds(400))
            repicked = backendPopup(win)?.titleOfSelectedItem ?? ""
        }
        step("backend: in General, a pop-up of [\(items.joined(separator: ", "))]", inGeneral && items == BackendChoice.allCases.map(\.label))
        step("backend: shows the stored choice (\(picked))", picked == BackendChoice.hcOnly.label)
        step("backend: a pick is stored and shown (\(repicked), stored \(AppDefaults.store.string(forKey: BackendChoice.key) ?? "nil"))",
             repicked == BackendChoice.both.label && AppDefaults.store.string(forKey: BackendChoice.key) == BackendChoice.both.rawValue)
        AppDefaults.store.removeObject(forKey: BackendChoice.key)
        let remoteNote = BackendSetting.note(.hcOnly, flags: false, remote: true)
        step("backend: on another machine, herdr-core only says its TUI stays on", remoteNote.contains("its herdr TUI stays on"))

        // The tab chosen last comes back, and a click on a tab chooses it.
        AppDefaults.store.set(SettingsTab.updates.rawValue, forKey: SettingsTab.key)
        var reopened: Set<SettingsControl> = [], clicked: Set<SettingsControl> = []
        _ = await shoot(SettingsSplit(remote: hub), dark: false, to: nil) { win in
            reopened = shown()
            if let r = SettingsProbe.tabs[.agents], let content = win.contentView {
                let p = NSPoint(x: r.midX, y: content.bounds.height - r.midY)
                for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                    if let e = NSEvent.mouseEvent(with: type, location: p, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                                  windowNumber: win.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1) { win.sendEvent(e) }
                }
                try? await Task.sleep(for: .milliseconds(600))
                clicked = shown()
            }
        }
        step("stored updates: Settings opens on Updates ([\(names(reopened))])", !reopened.isEmpty && reopened.isSubset(of: Set(SettingsTab.updates.controls)))
        step("a click on Agents shows it and remembers it ([\(names(clicked))], stored \(AppDefaults.store.string(forKey: SettingsTab.key) ?? "nil"))",
             clicked.contains(.allowInput) && AppDefaults.store.string(forKey: SettingsTab.key) == SettingsTab.agents.rawValue)
        AppDefaults.store.removeObject(forKey: SettingsTab.key)
        var fresh: Set<SettingsControl> = []
        _ = await shoot(SettingsSplit(), dark: false, to: nil) { _ in fresh = shown() }
        step("nothing stored: Settings opens on General ([\(names(fresh))])", fresh == Set(SettingsTab.general.controls))

        print("snapshot-settings: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        exit(failures == 0 ? 0 : 1)
    }

    /// The stand-in wallpaper (blurred like the behind-window material when the window is see-through), then the window's
    /// own pixels over it. The capture is of this one window only.
    /// Returns the share of the window capture that is not background (0 = blank capture).
    /// The capture of this one window (nil without screen-capture permission).
    @MainActor static func windowImage(_ win: NSWindow) -> CGImage? {
        typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage") else { return nil }
        return unsafeBitCast(sym, to: Fn.self)(.null, 8, UInt32(win.windowNumber), 1)?.takeRetainedValue()
    }
    @discardableResult
    @MainActor private static func compose(_ win: NSWindow, over paper: NSImage, blur: Bool, to path: String) -> Double {
        typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
        guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage"),
              let top = unsafeBitCast(sym, to: Fn.self)(.null, 8, UInt32(win.windowNumber), 1)?.takeRetainedValue(),
              var back = paper.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
            print("snapshot-settings: no window capture (screen-capture permission: \(CGPreflightScreenCaptureAccess()))")
            return 0
        }
        let ink = Pixels(top).ink
        if ink <= 0.02 { print("snapshot-settings: the window capture is blank (screen-capture permission: \(CGPreflightScreenCaptureAccess()))") }
        if blur, let f = CIFilter(name: "CIGaussianBlur") {
            f.setValue(CIImage(cgImage: back).clampedToExtent(), forKey: kCIInputImageKey)
            f.setValue(30, forKey: kCIInputRadiusKey)
            if let out = f.outputImage, let cg = CIContext().createCGImage(out, from: CIImage(cgImage: back).extent) { back = cg }
        }
        let w = top.width, h = top.height
        guard let ctx = CGContext(data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: 0,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return ink }
        let all = CGRect(x: 0, y: 0, width: w, height: h)
        // Rounded like the window, so the corners stay clear.
        ctx.addPath(CGPath(roundedRect: all, cornerWidth: 24 * CGFloat(w) / win.frame.width, cornerHeight: 24 * CGFloat(w) / win.frame.width, transform: nil))
        ctx.clip()
        ctx.draw(back, in: all)
        ctx.resetClip()
        ctx.draw(top, in: all)
        guard let img = ctx.makeImage() else { return ink }
        try? FileManager.default.createDirectory(atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        try? NSBitmapImageRep(cgImage: img).representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
        return ink
    }

    /// A stand-in desktop: soft colour fields with a few hard shapes, so blur and tint both show.
    @MainActor private static func wallpaper(_ size: NSSize, dark: Bool) -> NSImage {
        NSImage(size: size, flipped: false) { r in
            let colors: [NSColor] = dark ? [.systemIndigo, .systemPurple, .systemTeal] : [.systemOrange, .systemPink, .systemTeal]
            NSGradient(colors: colors)?.draw(in: r, angle: 35)
            (dark ? NSColor.black : NSColor.white).withAlphaComponent(0.85).setFill()
            NSBezierPath(ovalIn: NSRect(x: r.width * 0.55, y: r.height * 0.5, width: 180, height: 180)).fill()
            NSColor.systemYellow.setFill()
            NSBezierPath(roundedRect: NSRect(x: 40, y: 60, width: 220, height: 90), xRadius: 20, yRadius: 20).fill()
            return true
        }
    }
}

/// Where the Settings page and its first row lie (global frames); the page is zero while it is not shown.
enum SettingsProbe {
    nonisolated(unsafe) static var page: CGRect = .zero
    /// The form beside the tab list.
    nonisolated(unsafe) static var content: CGRect = .zero
    nonisolated(unsafe) static var tabs: [SettingsTab: CGRect] = [:]
    /// The rows on screen now (`settingsControl`).
    nonisolated(unsafe) static var controls: [SettingsControl: CGRect] = [:]
    /// The terminal colours row's value (the theme's names, or why it is not used).
    nonisolated(unsafe) static var terminalColors: String?
    /// The Back row (the sidebar's while Settings has it, or the page's own list while the sidebar is hidden).
    nonisolated(unsafe) static var back: CGRect = .zero
    /// The page's own tab list (shown only while the sidebar is hidden); zero while not shown.
    nonisolated(unsafe) static var pageList: CGRect = .zero
    /// The page's title block (icon, title, summary line).
    nonisolated(unsafe) static var header: CGRect = .zero
    /// The Tailscale steps shown now, by step number (1...5); empty while the checks are one summary row.
    nonisolated(unsafe) static var steps: [Int: CGRect] = [:]
}

extension ChromeTests {
    /// `--test-settings-page`: Settings is a page of the main window, not a window of its own. A Settings row at the foot of
    /// the sidebar opens it in the detail column (a real click); the sidebar then holds Back and the tabs, the toolbar has no
    /// tab row, the form fills the column and scrolls (the long Remote tab, chosen last). Back, Esc and ⌘, again return to
    /// the screen from before, unchanged; with the sidebar hidden the page brings its own Back and tabs. Offscreen window
    /// only; stills of every tab in light and dark, and of the scrolled page, go to build/snapshots-settings-page/.
    static func runSettingsPageTest() async {
        let dir = "build/snapshots-settings-page"
        AppDefaults.store.set(SettingsTab.remote.rawValue, forKey: SettingsTab.key)
        let model = AppModel(store: .previewWithSessions)   // session ids: which logs count as on screen
        // Tailscale on: the long Remote tab, taller than the column.
        let ts = TailscaleSetup.Status(signedIn: true, backendState: "Running", dnsName: "mac.example.ts.net", magicDNS: true, certificates: true,
                                       peers: [TailscaleSetup.Peer(name: "Phone", os: "iOS", online: true)])
        model.remote = RemoteHub.preview(running: true, devices: [(HubDevice(id: "d1", name: "Phone", level: .input), Date(timeIntervalSince1970: 1_790_000_000))],
                                         offer: false, tailscale: ts, serve: .ours, mode: .tailscale)
        let size = NSSize(width: 1200, height: 600)
        // The window's size, not a fixed frame: the reading-place checks make it short for a while.
        let host = NSHostingView(rootView: RootView(flat: false).environment(model).shortcutsPanelHost().frame(maxWidth: .infinity, maxHeight: .infinity))
        host.frame = NSRect(origin: .zero, size: size); host.autoresizingMask = [.width, .height]
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
        WindowChrome.apply(win)
        // Light stills light even on a Mac set to dark.
        win.appearance = NSAppearance(named: .aqua)
        win.contentView = host
        win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        try? await Task.sleep(for: .seconds(2))
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("settings-page-test: \(n) \(ok ? "ok" : "FAIL")") }
        // Selects the sidebar list row under the middle of a probed frame, as a click does. A synthetic mouse click selects a
        // List row only in a key window, which a headless run never has: the table row at that point is selected instead.
        func click(_ r: CGRect) async {
            guard let frame = win.contentView?.superview else { return }
            let p = NSPoint(x: r.midX, y: frame.bounds.height - r.midY)
            var v = frame.hitTest(p)
            while let cur = v, !(cur is NSTableView) { v = cur.superview }
            if let table = v as? NSTableView {
                let row = table.row(at: table.convert(p, from: nil))
                if row >= 0 { table.selectRowIndexes([row], byExtendingSelection: false) }
            } else {
                // Not a list row (the page's own Back and tabs are buttons): a plain mouse click.
                for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                    if let e = NSEvent.mouseEvent(with: type, location: p, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                                                  windowNumber: win.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1) { win.sendEvent(e) }
                }
            }
            try? await Task.sleep(for: .milliseconds(600))
        }

        // The row: below every workspace row, at the foot of the sidebar panel.
        let row = ChromeProbe.rows["Settings"] ?? .zero
        let lastRow = model.store.workspaces.compactMap { ChromeProbe.rows["title:\($0.id)"]?.maxY }.max() ?? 0
        let panel = ChromeProbe.panel
        step("Settings row at the foot of the sidebar (row y \(Int(row.minY))..\(Int(row.maxY)), last workspace \(Int(lastRow)), panel bottom \(Int(panel.maxY)))",
             row.width > 0 && row.minY > lastRow && panel.maxY - row.maxY < 24)
        step("no page before it is chosen (\(SettingsProbe.page))", SettingsProbe.page == .zero)
        await click(row)
        step("a click opens the Settings page (\(model.screen))", model.screen == .settings)
        try? await Task.sleep(for: .milliseconds(600))
        let page = SettingsProbe.page, detail = ChromeProbe.detail
        step("the page fills the detail column (page \(page.integral), detail \(detail.integral))",
             page.width > 0 && abs(page.minX - detail.minX) < 2 && abs(page.maxX - detail.maxX) < 2 && page.height > detail.height - 40)
        let list = SettingsTab.allCases.compactMap { SettingsProbe.tabs[$0] }, content = SettingsProbe.content
        let side = ChromeProbe.panel, back = SettingsProbe.back
        // Settings has the sidebar: Back on top, then the tabs, in the sidebar panel; no list of its own in the page.
        step("the sidebar shows Back (\(back.integral)) then \(list.count) tabs top to bottom, inside the sidebar panel (\(side.integral))",
             back.width > 0 && list.count == SettingsTab.allCases.count && back.maxY <= (list.first?.minY ?? 0)
             && zip(list, list.dropFirst()).allSatisfy { $0.maxY <= $1.minY } && list.allSatisfy { side.contains(CGPoint(x: $0.midX, y: $0.midY)) })
        step("no workspace rows while Settings has the sidebar (Overview row \(ChromeProbe.rowsShown["Overview"] ?? false))", ChromeProbe.rowsShown["Overview"] != true)
        step("no tab list in the page (\(SettingsProbe.pageList)); the form fills it (\(content.integral))",
             SettingsProbe.pageList == .zero && abs(content.minX - page.minX) < 2 && abs(content.maxX - page.maxX) < 2 && content.height > page.height - 40)
        step("no tab row, + or terminal button in the toolbar (tab rows shown: \(TabProbe.rowsShown))", TabProbe.rowsShown == 0)

        // It scrolls: the page is taller than the column (with the remote section, even here); a wheel over it moves it.
        func scrollViews(_ v: NSView) -> [NSScrollView] { ((v as? NSScrollView).map { [$0] } ?? []) + v.subviews.flatMap(scrollViews) }
        // At the top of the page: the Remote tab's title block, then its first row, start just below the toolbar (the
        // detail column's top, under its safe area). The rows' column keeps to the left, about 40 pt from the tab list, at
        // most 760 pt wide; each row is one compact line.
        func topCheck(_ tag: String) {
            let row = SettingsProbe.controls[.remoteAccess] ?? .zero, detail = ChromeProbe.detail, content = SettingsProbe.content
            let head = SettingsProbe.header
            step("\(tag): title y \(Int(head.minY)) (toolbar \(Int(detail.minY))), first row y \(Int(row.minY)) under it",
                 head.width > 0 && head.minY >= detail.minY && head.minY - detail.minY <= 40 && head.maxY <= row.minY && row.minY - head.maxY <= 48)
            step("\(tag): rows from x \(Int(row.minX - content.minX)) of the form (about 40), \(Int(row.width)) pt wide (at most 760), \(Int(row.height)) pt high",
                 row.width > 0 && abs(row.minX - content.minX - 40) <= 2 && row.width <= 760 && row.height <= 44)
            let rows: [SettingsControl] = [.devices, .pictures, .phoneNotifications, .tailscaleSteps]
            let tall = rows.filter { (SettingsProbe.controls[$0]?.height ?? 99) > 44 }
            step("\(tag): compact rows, the Tailscale checks one row (taller than 44 pt: [\(tall.map(\.rawValue).joined(separator: ","))], steps shown \(SettingsProbe.steps.keys.sorted()))",
                 tall.isEmpty && SettingsProbe.steps.isEmpty)
        }
        let p = NSPoint(x: content.midX, y: content.midY)
        let hit = host.hitTest(host.convert(NSPoint(x: p.x, y: host.isFlipped ? p.y : host.bounds.height - p.y), to: host.superview))
        if let sv = hit?.enclosingScrollView {
            let shown = sv.contentView.bounds.height, content = sv.documentView?.frame.height ?? 0
            topCheck("light, top")
            ContentTests.capture(win, to: "\(dir)/settings-page-light.png")
            let top = sv.contentView.bounds.origin, y0 = top.y
            if let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: -400, wheel2: 0, wheel3: 0),
               let e = NSEvent(cgEvent: cg) { hit?.scrollWheel(with: e) }
            try? await Task.sleep(for: .milliseconds(600))
            let y1 = sv.contentView.bounds.origin.y, routed = y1 <= y0 + 30 && WheelRoute.reaches(hit, sv, by: 30)
            step("the page scrolls: shows \(Int(shown)) of \(Int(content)) pt, wheel \(Int(y0)) -> \(Int(y1))\(routed ? WheelRoute.note : "")",
                 sv.verticalScrollElasticity != .none && (y1 > y0 + 30 || routed))
            // Overlay unless the owner chose "Always" (legacy); the page follows the app's answer.
            let setting = ScrollerStyle.showScrollBars, want = ScrollerStyle.style(showScrollBars: setting)
            step("scroll bars: setting \(setting ?? "unset"), page \(sv.scrollerStyle.rawValue), preferred \(NSScroller.preferredScrollerStyle.rawValue), want \(want.rawValue) (1 = overlay)",
                 ScrollerStyle.installed && sv.scrollerStyle == want && NSScroller.preferredScrollerStyle == want)
            // A mouse plugged in or the setting changed: AppKit sets the system style on every scroll view; ours wins again.
            sv.scrollerStyle = want == .overlay ? .legacy : .overlay
            NotificationCenter.default.post(name: NSScroller.preferredScrollerStyleDidChangeNotification, object: nil)
            try? await Task.sleep(for: .milliseconds(300))
            step("scroll bars after a style change notice: page \(sv.scrollerStyle.rawValue), want \(want.rawValue)", sv.scrollerStyle == want)
            step("scroll bar setting map: Always -> legacy; WhenScrolling, Automatic, unset -> overlay",
                 ScrollerStyle.style(showScrollBars: "Always") == .legacy && ScrollerStyle.style(showScrollBars: "WhenScrolling") == .overlay
                 && ScrollerStyle.style(showScrollBars: "Automatic") == .overlay && ScrollerStyle.style(showScrollBars: nil) == .overlay)
            ContentTests.capture(win, to: "\(dir)/settings-page-light-scrolled.png")
            win.appearance = NSAppearance(named: .darkAqua)
            try? await Task.sleep(for: .milliseconds(600))
            ContentTests.capture(win, to: "\(dir)/settings-page-dark-scrolled.png")
            // Back to where the page started, insets included (the origin at the top is negative: the toolbar's inset above).
            // Scrolling to .zero would put the first row under the toolbar.
            sv.contentView.scroll(to: top); sv.reflectScrolledClipView(sv.contentView)
            try? await Task.sleep(for: .milliseconds(600))
            let back = sv.contentView.bounds.origin
            step("dark: back at the top origin (\(Int(back.x)), \(Int(back.y))) = (\(Int(top.x)), \(Int(top.y)))", abs(back.x - top.x) < 1 && abs(back.y - top.y) < 1)
            topCheck("dark, top")
            ContentTests.capture(win, to: "\(dir)/settings-page-dark.png")
            win.appearance = NSAppearance(named: .aqua)
        } else { step("a scroll view under the page (\(hit.map { "\(type(of: $0))" } ?? "no view"))", false) }

        // Every tab, chosen from the sidebar, light and dark.
        for t in SettingsTab.allCases {
            await click(SettingsProbe.tabs[t] ?? .zero)
            step("\(t.rawValue): the sidebar row chooses it (\(AppDefaults.store.string(forKey: SettingsTab.key) ?? "nil"))", AppDefaults.store.string(forKey: SettingsTab.key) == t.rawValue)
            ContentTests.capture(win, to: "\(dir)/settings-\(t.rawValue)-light.png")
            win.appearance = NSAppearance(named: .darkAqua); try? await Task.sleep(for: .milliseconds(500))
            ContentTests.capture(win, to: "\(dir)/settings-\(t.rawValue)-dark.png")
            win.appearance = NSAppearance(named: .aqua); try? await Task.sleep(for: .milliseconds(300))
        }

        // A tab row in the sidebar chooses the tab, and it is remembered.
        await click(SettingsProbe.tabs[.agents] ?? .zero)
        step("the sidebar's Agents row shows the Agents tab (\(AppDefaults.store.string(forKey: SettingsTab.key) ?? "nil"))",
             AppDefaults.store.string(forKey: SettingsTab.key) == SettingsTab.agents.rawValue && SettingsProbe.controls[.allowInput] != nil)

        // Back, Esc and ⌘, again: the screen, workspace and tab from before, unchanged.
        model.screen = .tab; try? await Task.sleep(for: .milliseconds(800))
        let ws = model.workspaceID, tab = model.tabID
        // The reading place and the typing focus too: the conversation scrolled up from the bottom, the composer focused.
        // Back must show that same conversation view (not a new one at the bottom) and the same composer, still focused.
        // The app's smallest window (960 x 600) with the largest, narrowest text, so the preview conversation can scroll.
        AppDefaults.store.set(ReadingSize.xl.rawValue, forKey: ReadingSize.key)
        AppDefaults.store.set(ReadingWidth.narrow.rawValue, forKey: ReadingWidth.key)
        win.setContentSize(NSSize(width: 960, height: 600)); try? await Task.sleep(for: .milliseconds(1200))
        func views<T: NSView>(_ v: NSView, _ t: T.Type) -> [T] { ((v as? T).map { [$0] } ?? []) + v.subviews.flatMap { views($0, t) } }
        let root = win.contentView?.superview ?? host
        let conversation = views(root, NSScrollView.self)
            // On macOS 15 (smaller toolbar inset) the preview conversation scrolls only some 17 pt.
            .filter { ($0.documentView?.frame.height ?? 0) + $0.contentInsets.top + $0.contentInsets.bottom - $0.bounds.height > 10 && $0.frame.width > 400 }
            .max { ($0.documentView?.frame.height ?? 0) < ($1.documentView?.frame.height ?? 0) }
        let composer = views(root, ComposerTextView.SubmitTextView.self).first
        if let sv = conversation {
            // Up 50 pt, not past the top.
            sv.contentView.scroll(to: NSPoint(x: sv.contentView.bounds.minX, y: max(-sv.contentInsets.top, sv.contentView.bounds.minY - 50)))
            sv.reflectScrolledClipView(sv.contentView)
        }
        if let composer { win.makeFirstResponder(composer) }
        try? await Task.sleep(for: .milliseconds(400))
        let readAt = conversation?.contentView.bounds.minY ?? .nan
        if conversation == nil {   // which scroll views there are, and why none counts (macOS 15 on CI)
            for sv in views(root, NSScrollView.self) {
                print("settings-page-test: scroll view \(type(of: sv)) frame \(sv.frame) document \(sv.documentView?.frame.size ?? .zero) insets \(sv.contentInsets.top)/\(sv.contentInsets.bottom)")
            }
        }
        step("a conversation to scroll (\(conversation.map { "\(Int($0.documentView?.frame.height ?? 0)) pt doc, at \(Int(readAt))" } ?? "none")) and a focused composer (\(win.firstResponder === composer))",
             conversation != nil && composer != nil && win.firstResponder === composer)
        func same(_ how: String) {
            step("\(how) returns to the tab screen as it was (\(model.screen), \(model.workspaceID ?? "nil")/\(model.tabID ?? "nil")), the sidebar's rows again, the tab row again (\(TabProbe.rowsShown))",
                 model.screen == .tab && model.workspaceID == ws && model.tabID == tab && ChromeProbe.rowsShown["Overview"] == true && TabProbe.rowsShown > 0 && SettingsProbe.page == .zero)
            let at = conversation?.contentView.bounds.minY ?? .nan
            step("\(how): the same conversation view, in the window (\(conversation?.window != nil)), still at \(Int(readAt)) (\(at.isNaN ? "none" : "\(Int(at))")), the same composer focused (\(win.firstResponder === composer))",
                 conversation?.window != nil && abs(at - readAt) < 2 && composer?.window != nil && win.firstResponder === composer)
        }
        // The logs on screen stay on screen under Settings: they keep their full depth (and the older messages read).
        let onScreen = model.store.visibleSessions ?? []
        model.showSettings(); try? await Task.sleep(for: .milliseconds(600))
        step("⌘, opens the page from the tab screen (\(model.screen))", model.screen == .settings && SettingsProbe.page.width > 0)
        step("the tab's logs still count as on screen under Settings (\(onScreen.sorted()) -> \((model.store.visibleSessions ?? []).sorted()))",
             !onScreen.isEmpty && model.store.visibleSessions == onScreen)
        // Keys while Settings shows go to Settings: the hidden screen cannot take them back, by a focus call (a fresh
        // session focuses its composer, a focused pane its terminal) or by the Tab key.
        func typing(_ r: NSResponder?) -> Bool { r is NSText || r is TerminalView }
        func name(_ r: NSResponder?) -> String { r.map { "\(type(of: $0))" } ?? "nil" }
        step("while Settings shows, keys do not go to the composer (first responder \(name(win.firstResponder)))", !typing(win.firstResponder))
        if let composer { win.makeFirstResponder(composer) }
        step("the hidden composer loses the keys at once, in the same call (\(name(win.firstResponder)))", composer != nil && !typing(win.firstResponder))
        try? await Task.sleep(for: .milliseconds(300))
        step("a focus call on the hidden composer does not take the keys (\(name(win.firstResponder)))", composer != nil && !typing(win.firstResponder))
        let terminal = views(root, TerminalView.self).first
        if let terminal { win.makeFirstResponder(terminal) }
        try? await Task.sleep(for: .milliseconds(300))
        step("a focus call on the hidden terminal does not take the keys (\(terminal.map { "\(type(of: $0))" } ?? "no terminal"): \(name(win.firstResponder)))", terminal != nil && !typing(win.firstResponder))
        for _ in 0..<4 { win.selectNextKeyView(nil) }
        try? await Task.sleep(for: .milliseconds(300))
        step("Tab does not reach the hidden screen (\(name(win.firstResponder)))", !typing(win.firstResponder))
        // The page's own controls keep the keys (Full Keyboard Access walks them with Tab). `under`: the views of the
        // screen under the page, which are not its controls.
        func pageKeeps(_ how: String, under: [NSView] = []) async {
            let height = win.contentView?.bounds.height ?? 0
            let control = views(root, NSControl.self).first { v in
                let f = v.convert(v.bounds, to: nil)
                return v.acceptsFirstResponder && !under.contains { v === $0 || v.isDescendant(of: $0) }
                    && SettingsProbe.content.contains(CGPoint(x: f.midX, y: height - f.midY))
            }
            if let control { win.makeFirstResponder(control) }
            try? await Task.sleep(for: .milliseconds(300))
            step("\(how)a control of the page keeps the keys (\(control.map { "\(type(of: $0))" } ?? "no control"): \(name(win.firstResponder)))",
                 control != nil && win.firstResponder === control)
            win.makeFirstResponder(nil)
        }
        await pageKeeps("")
        // The mouse too: the hidden terminal's own wheel monitor (it does not go through hit-testing) lets the wheel pass.
        if let terminal = terminal as? PasteTerminalView {
            var wheels = 0
            let saved = terminal.onWheel
            terminal.onWheel = { _, _ in wheels += 1 }
            let mid = win.convertPoint(toScreen: terminal.convert(NSPoint(x: terminal.bounds.midX, y: terminal.bounds.midY), to: nil))
            if let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: 60, wheel2: 0, wheel3: 0) {
                cg.location = CGPoint(x: mid.x, y: (NSScreen.screens.first?.frame.height ?? 0) - mid.y)
                if let e = NSEvent(cgEvent: cg) { NSApp.sendEvent(e) }
            }
            try? await Task.sleep(for: .milliseconds(300))
            terminal.onWheel = saved
            step("a wheel over the hidden terminal does not reach it (\(wheels) wheels)", wheels == 0)
            // Nothing goes out of it, whatever reaches it (SwiftTerm's own mouse-moved reports, say). No session here:
            // were the gate open, the bytes would only be counted.
            if let c = terminal.coordinator, let h = c.handle {
                let session = c.session, writable = c.writable, sent = h.sentBytes
                c.session = nil; c.writable = true
                c.send(source: terminal, data: ArraySlice(Array("x".utf8)))
                try? await Task.sleep(for: .milliseconds(300))
                step("the hidden terminal sends nothing (\(h.sentBytes - sent) bytes)", h.sentBytes == sent)
                c.session = session; c.writable = writable
            } else { step("the hidden terminal sends nothing: no coordinator or handle", false) }
        } else { step("a wheel over the hidden terminal: no terminal", false) }
        // Return in the hidden composer sends nothing and keeps the draft (its send is gated, so nothing goes out either way).
        if let composer {
            composer.string = "a draft"
            composer.insertNewline(nil)
            try? await Task.sleep(for: .milliseconds(200))
            step("Return in the hidden composer keeps the draft (\"\(composer.string)\")", composer.string == "a draft")
            composer.string = ""
        } else { step("Return in the hidden composer: no composer", false) }
        // A terminal sheet asked for under Settings (a late approval result) does not open over the page.
        model.showTerminal = true; try? await Task.sleep(for: .milliseconds(800))
        step("no terminal sheet over Settings (\(win.attachedSheet.map { "\(type(of: $0))" } ?? "none"))", win.attachedSheet == nil)
        model.showTerminal = false; try? await Task.sleep(for: .milliseconds(800))
        if let sheet = win.attachedSheet { win.endSheet(sheet); try? await Task.sleep(for: .milliseconds(400)) }
        // The close-pane question neither.
        model.closingPane = model.tab?.panes.first; try? await Task.sleep(for: .milliseconds(800))
        step("no close-pane dialog over Settings (\(win.attachedSheet.map { "\(type(of: $0))" } ?? "none"))", win.attachedSheet == nil)
        model.closingPane = nil; try? await Task.sleep(for: .milliseconds(800))
        if let sheet = win.attachedSheet { win.endSheet(sheet); try? await Task.sleep(for: .milliseconds(400)) }
        // Only the hidden screen loses the keys: a field over Settings (the ⌘/ panel's search) keeps them.
        ShortcutsPanel.open(); try? await Task.sleep(for: .seconds(1))
        let search = views(root, NSSearchField.self).first
        // The field focuses itself a turn after it is made, which may come before it is in the window: focused here.
        if let search { win.makeFirstResponder(search) }
        try? await Task.sleep(for: .milliseconds(300))
        let editor = win.firstResponder as? NSTextView
        step("⌘/ over Settings: its search field has the keys (\(search == nil ? "no field" : name(win.firstResponder)))",
             search != nil && editor != nil && search?.currentEditor() === editor)
        editor?.insertText("split", replacementRange: NSRange(location: NSNotFound, length: 0))
        try? await Task.sleep(for: .milliseconds(300))
        step("⌘/ over Settings: typing goes into its search field (\"\(ShortcutsPanel.state.query)\")", ShortcutsPanel.state.query == "split")
        ShortcutsPanel.close(); try? await Task.sleep(for: .milliseconds(600))
        await click(SettingsProbe.back)
        same("Back")
        // And with the page gone the terminal sends again (no session: the bytes are only counted).
        if let terminal = terminal as? PasteTerminalView, let c = terminal.coordinator, let h = c.handle {
            let session = c.session, writable = c.writable, sent = h.sentBytes
            c.session = nil; c.writable = true
            c.send(source: terminal, data: ArraySlice(Array("x".utf8)))
            try? await Task.sleep(for: .milliseconds(300))
            step("after Back the terminal sends again (\(h.sentBytes - sent) bytes)", h.sentBytes == sent + 1)
            c.session = session; c.writable = writable
        } else { step("after Back the terminal sends again: no coordinator or handle", false) }
        model.toggleSettings(); try? await Task.sleep(for: .milliseconds(600))
        step("⌘, opens it (\(model.screen))", model.screen == .settings)
        if let cg = CGEvent(keyboardEventSource: nil, virtualKey: 53, keyDown: true), let e = NSEvent(cgEvent: cg) { win.sendEvent(e) }
        if let cg = CGEvent(keyboardEventSource: nil, virtualKey: 53, keyDown: false), let e = NSEvent(cgEvent: cg) { win.sendEvent(e) }
        try? await Task.sleep(for: .milliseconds(600))
        same("Esc")
        model.toggleSettings(); try? await Task.sleep(for: .milliseconds(400))
        model.toggleSettings(); try? await Task.sleep(for: .milliseconds(600))
        same("⌘, again")
        AppDefaults.store.removeObject(forKey: ReadingSize.key); AppDefaults.store.removeObject(forKey: ReadingWidth.key)
        win.setContentSize(size); try? await Task.sleep(for: .milliseconds(600))
        // From another screen: back to that screen.
        model.screen = .overview
        model.showSettings(); try? await Task.sleep(for: .milliseconds(400))
        model.closeSettings(); try? await Task.sleep(for: .milliseconds(400))
        step("from Overview, Back returns to Overview (\(model.screen))", model.screen == .overview)
        // From Search: its list and the toolbar search field do not take the keys under Settings.
        model.screen = .search; try? await Task.sleep(for: .milliseconds(800))
        // The tables of the Search screen: those still in the window once Settings shows (the sidebar's list is replaced).
        let onSearch = views(root, NSTableView.self)
        // A query in the focused search field: Back gives the field its focus again.
        let searchField = views(win.contentView?.superview ?? root, NSSearchField.self).first
        model.searchText = "a"; try? await Task.sleep(for: .milliseconds(400))
        if let searchField { win.makeFirstResponder(searchField) }
        try? await Task.sleep(for: .milliseconds(400))
        step("Search: its field has the keys (\(name(win.firstResponder)))", searchField?.currentEditor() != nil && searchField?.currentEditor() === win.firstResponder)
        model.showSettings(); try? await Task.sleep(for: .milliseconds(800))
        await pageKeeps("from Search: ", under: onSearch)
        let lists = onSearch.filter { $0.window === win }
        var taken: [String] = []
        if let searchField {
            win.makeFirstResponder(searchField)
            if win.firstResponder === searchField || (searchField.currentEditor().map { $0 === win.firstResponder } ?? false) { taken.append("search field") }
            win.makeFirstResponder(nil)
        }
        for table in lists {
            win.makeFirstResponder(table)
            if win.firstResponder === table { taken.append("\(type(of: table))") }
            win.makeFirstResponder(nil)
        }
        try? await Task.sleep(for: .milliseconds(300))
        step("from Search under Settings: no search field or list takes the keys (search field \(searchField == nil ? "gone" : "there"), \(lists.count) lists; taken \(taken))", !lists.isEmpty && taken.isEmpty)
        model.closeSettings(); try? await Task.sleep(for: .milliseconds(600))
        step("from Search, Back returns to Search, the search field focused again (\(model.screen), \(name(win.firstResponder)))",
             model.screen == .search && searchField?.currentEditor() != nil && searchField?.currentEditor() === win.firstResponder)
        model.searchText = ""
        model.screen = .tab; try? await Task.sleep(for: .milliseconds(400))

        // The live terminal's right-click monitor (right clicks to the program): under the page it lets the click through,
        // with the page gone it takes it. A view of its own in a window of its own (no session: nothing goes out), so the
        // click let through lands there.
        for covered in [true, false] {
            let tv = PasteTerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 200))
            tv.rightClickToPane = true
            SettingsFocus.cover(tv, covered)
            let tw = OffscreenWindow(contentRect: tv.frame, styleMask: [.titled], backing: .buffered, defer: false)
            tw.contentView = tv; tw.setFrameOrigin(NSPoint(x: -3000, y: -2000)); tw.orderFrontRegardless()
            try? await Task.sleep(for: .milliseconds(400))
            let taken = PasteTerminalView.rightClicksTaken
            let mid = tw.convertPoint(toScreen: NSPoint(x: 200, y: 100))
            if let cg = CGEvent(mouseEventSource: nil, mouseType: .rightMouseDown, mouseCursorPosition: CGPoint(x: mid.x, y: (NSScreen.screens.first?.frame.height ?? 0) - mid.y), mouseButton: .right),
               let e = NSEvent(cgEvent: cg) { NSApp.sendEvent(e) }
            try? await Task.sleep(for: .milliseconds(300))
            step("live terminal \(covered ? "under the page" : "uncovered"): a right click \(covered ? "passes" : "is taken") (\(PasteTerminalView.rightClicksTaken - taken) taken)",
                 PasteTerminalView.rightClicksTaken - taken == (covered ? 0 : 1))
            SettingsFocus.cover(tv, false); tw.orderOut(nil); tw.contentView = nil
        }
        // The history view's wheel monitor: under the page a wheel down at its foot does not leave it; with the page gone
        // it does.
        for covered in [true, false] {
            var exits = 0
            let hv = NSHostingView(rootView: HistoryTerminalNSView(text: "one\r\ntwo", writable: false, scheme: .light, onExit: { _ in exits += 1 })
                .environment(\.underSettings, covered).frame(width: 400, height: 200))
            hv.frame = NSRect(x: 0, y: 0, width: 400, height: 200)
            let hw = OffscreenWindow(contentRect: hv.frame, styleMask: [.titled], backing: .buffered, defer: false)
            hw.contentView = hv; hw.setFrameOrigin(NSPoint(x: -3000, y: -2000)); hw.orderFrontRegardless()
            try? await Task.sleep(for: .milliseconds(600))
            if let h = views(hv, HistoryTerminalView.self).first {
                let mid = hw.convertPoint(toScreen: h.convert(NSPoint(x: h.bounds.midX, y: h.bounds.midY), to: nil))
                if let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: -60, wheel2: 0, wheel3: 0) {
                    cg.location = CGPoint(x: mid.x, y: (NSScreen.screens.first?.frame.height ?? 0) - mid.y)
                    if let e = NSEvent(cgEvent: cg) { NSApp.sendEvent(e) }
                }
                try? await Task.sleep(for: .milliseconds(300))
            }
            step("history view \(covered ? "under the page" : "uncovered"): a wheel down at its foot \(covered ? "stays" : "leaves") (\(exits) exits)", exits == (covered ? 0 : 1))
            hw.orderOut(nil); hw.contentView = nil
        }

        // The sidebar hidden: the page brings its own list, Back on top.
        model.setSidebarVisibility(.detailOnly); try? await Task.sleep(for: .milliseconds(800))
        model.showSettings(); try? await Task.sleep(for: .milliseconds(800))
        let own = SettingsProbe.pageList, page2 = SettingsProbe.page
        step("sidebar hidden: the page shows Back and the tabs on its left (\(own.integral) in \(page2.integral)), Back at \(SettingsProbe.back.integral)",
             own.width > 0 && own.minX - page2.minX < 20 && SettingsProbe.back.width > 0 && own.contains(CGPoint(x: SettingsProbe.back.midX, y: SettingsProbe.back.midY)))
        step("sidebar hidden: still no tab row in the toolbar (\(TabProbe.rowsShown))", TabProbe.rowsShown == 0)
        ContentTests.capture(win, to: "\(dir)/settings-page-sidebar-hidden-light.png")
        win.appearance = NSAppearance(named: .darkAqua); try? await Task.sleep(for: .milliseconds(500))
        ContentTests.capture(win, to: "\(dir)/settings-page-sidebar-hidden-dark.png")
        win.appearance = NSAppearance(named: .aqua); try? await Task.sleep(for: .milliseconds(300))
        await click(SettingsProbe.back)
        step("sidebar hidden: Back returns (\(model.screen)), the sidebar stays hidden (\(model.sidebarVisibility))", model.screen == .tab && model.sidebarVisibility == .detailOnly)
        model.setSidebarVisibility(.all); try? await Task.sleep(for: .milliseconds(600))

        // Full screen (the model's flag: the row there is window content, FullScreenTopRow): none while Settings shows.
        model.fullScreen = true; try? await Task.sleep(for: .milliseconds(600))
        let rowsFull = TabProbe.rowsShown
        model.showSettings(); try? await Task.sleep(for: .milliseconds(600))
        step("full screen: no tab row while Settings shows (\(TabProbe.rowsShown), \(rowsFull) before)", rowsFull > 0 && TabProbe.rowsShown == 0)
        model.toggleSettings(); try? await Task.sleep(for: .milliseconds(600))
        step("full screen: ⌘, again brings the rows back (\(TabProbe.rowsShown))", model.screen == .tab && TabProbe.rowsShown == rowsFull)
        model.fullScreen = false

        AppDefaults.store.removeObject(forKey: SettingsTab.key)
        print("settings-page-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }
}
