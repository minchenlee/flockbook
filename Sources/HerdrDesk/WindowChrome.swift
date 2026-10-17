import AppKit
import SwiftUI

/// Test probe: global frames of the sidebar panel, its toggle button and its nav rows (by title).
enum ChromeProbe {
    nonisolated(unsafe) static var panel: CGRect = .zero
    nonisolated(unsafe) static var peekStrip: CGRect = .zero
    nonisolated(unsafe) static var connectionFooter: CGRect = .zero
    /// The detail column (global frame, below the toolbar safe area) and its top safe-area inset.
    nonisolated(unsafe) static var detail: CGRect = .zero
    nonisolated(unsafe) static var detailTopInset: CGFloat = 0
    /// The split view's global frame (RootView's content).
    nonisolated(unsafe) static var root: CGRect = .zero
    /// Full screen: our sidebar toggle and the island's buttons.
    nonisolated(unsafe) static var fullScreenToggle: CGRect = .zero
    nonisolated(unsafe) static var fullScreenActions: CGRect = .zero
    nonisolated(unsafe) static var peekPanel: CGRect = .zero
    nonisolated(unsafe) static var newWorkspace: CGRect = .zero
    nonisolated(unsafe) static var newWorkspaceAction: (() -> Void)?
    /// Last measured window (tests read the system sidebar toggle from its toolbar).
    nonisolated(unsafe) static weak var window: NSWindow?
    /// The system sidebar toggle (NavigationSplitView's toolbar item), in content coordinates (top-left origin); zero if absent.
    @MainActor static var toggle: CGRect {
        guard let w = window, let cv = w.contentView else { return .zero }
        guard let v = (w.toolbar?.items ?? []).first(where: { $0.itemIdentifier.rawValue.localizedCaseInsensitiveContains("sidebar") && !$0.itemIdentifier.rawValue.contains("Separator") })?.view,
              v.window != nil, !v.isHiddenOrHasHiddenAncestor else { return .zero }
        var r = cv.convert(v.bounds, from: v)
        if !cv.isFlipped { r.origin.y = cv.bounds.height - r.maxY }
        return r
    }
    nonisolated(unsafe) static var sheetClose: (() -> Void)?
    nonisolated(unsafe) static var sheetHandle: TerminalHandle?
    nonisolated(unsafe) static var rows: [String: CGRect] = [:]
}

/// Window chrome the app depends on. The real window (WindowFixer) and every offscreen test window call this same code,
/// so tests cannot drift from the app: content runs under a transparent title bar, no separator.
@MainActor enum WindowChrome {
    static func apply(_ w: NSWindow) {
        w.styleMask.insert(.fullSizeContentView)
        w.titlebarAppearsTransparent = true
        w.titlebarSeparatorStyle = .none
        // Test windows never save their frame, split positions or restorable state into the owner's domain
        // (that could move the owner's real window next launch).
        if AppDefaults.isTest {
            w.isRestorable = false
            w.setFrameAutosaveName("")
            func clear(_ v: NSView) { (v as? NSSplitView)?.autosaveName = nil; v.subviews.forEach(clear) }
            if let v = w.contentView { clear(v) }
        }
    }

    /// Where the traffic lights are, in the window content's top-left space. Not measured in full screen (no lights there):
    /// the last windowed frames stay.
    static func measure(_ w: NSWindow, _ model: AppModel) {
        ChromeProbe.window = w
        guard !w.styleMask.contains(.fullScreen), let cv = w.contentView else { return }
        let buttons = [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton].compactMap { w.standardWindowButton($0) }
        guard buttons.count == 3, buttons.allSatisfy({ !$0.isHidden && $0.superview != nil }) else { return }
        let frames = buttons.map { b in
            var r = cv.convert(b.frame, from: b.superview)
            if !cv.isFlipped { r.origin.y = cv.bounds.height - r.maxY }
            return r
        }
        let moved = frames.count != model.lights.count || zip(frames, model.lights).contains { a, b in
            abs(a.minX - b.minX) > 0.5 || abs(a.minY - b.minY) > 0.5 || abs(a.width - b.width) > 0.5 || abs(a.height - b.height) > 0.5
        }
        if moved { model.lights = frames }
        // Toolbar height and the sidebar toggle's place, for full screen (FullScreenChrome).
        let band = w.frame.height - w.contentLayoutRect.height
        if band > 20, abs(band - model.toolbarBand) > 0.5 { model.toolbarBand = band }
        // Classified by where the toggle is, not by sidebarVisibility: during the sidebar's slide the item is still at its
        // old place. At the panel's trailing edge = the shown spot; beside the lights = the hidden spot.
        let t = ChromeProbe.toggle, edge = ChromeProbe.panel.maxX
        if t.width > 0, edge > 0, model.sidebarVisibility != .detailOnly, abs(edge - t.maxX) < 12 {
            let spot = AppModel.ToggleSpot(trailing: edge - t.maxX, frame: t)
            if model.toggleShown != spot { model.toggleShown = spot }
        } else if t.width > 0, model.sidebarVisibility == .detailOnly, t.minX < model.lightsMaxX + 40 {
            let spot = AppModel.ToggleSpot(trailing: 0, frame: t)
            if model.toggleHidden != spot { model.toggleHidden = spot }
        }
    }

    /// How far the system's full-screen toolbar window reaches into `w` from the top (0 while it is slid away above it).
    static func toolbarReveal(_ w: NSWindow, toolbarWindow tw: NSWindow) -> CGFloat {
        guard tw.isVisible, tw.frame.height > 0 else { return 0 }
        return min(max(0, w.frame.maxY - tw.frame.minY), tw.frame.height + 40)
    }
    /// Where the toolbar lays out the tab island (the widest item) and the buttons after it, in the top-left space of the
    /// window that holds the toolbar (`w`, or its full-screen toolbar window), x in `w`'s space. Nil while not laid out.
    static func toolbarRow(_ w: NSWindow) -> (island: CGRect, actions: CGRect)? {
        let views = (w.toolbar?.items ?? []).compactMap(\.view).filter { $0.window != nil && !$0.isHidden }
        func frame(_ v: NSView) -> CGRect {
            guard let host = v.window else { return .zero }
            var r = v.convert(v.bounds, to: nil)
            r.origin.y = host.frame.height - r.maxY
            r.origin.x += host.frame.minX - w.frame.minX
            return r
        }
        let frames = views.map(frame).filter { $0.width > 0 && $0.height > 0 }
        guard let island = frames.max(by: { $0.width < $1.width }), island.width >= 100 else { return nil }
        let actions = frames.filter { $0.minX >= island.maxX - 1 && $0.minX < island.maxX + 60 && abs($0.midY - island.midY) < 8 }
            .reduce(CGRect.null) { $0.union($1) }
        guard !actions.isNull else { return nil }
        return (island, actions)
    }
    /// The toolbar's item views and their containers (the toolbar lays them out by moving these).
    static func toolbarItemViews(_ w: NSWindow) -> [NSView] {
        (w.toolbar?.items ?? []).compactMap(\.view).flatMap { v in sequence(first: v, next: \.superview).prefix(3) }
    }
    static func measureToolbarRow(_ w: NSWindow, _ model: AppModel) {
        guard let (island, actions) = toolbarRow(w) else { return }
        func moved(_ a: CGRect, _ b: CGRect) -> Bool { abs(a.midX - b.midX) > 0.25 || abs(a.midY - b.midY) > 0.25 || abs(a.width - b.width) > 0.25 }
        if moved(island, model.toolbarIsland) { model.toolbarIsland = island }
        if moved(actions, model.toolbarActions) { model.toolbarActions = actions }
    }

    /// The window that holds the toolbar in full screen (AppKit moves it out of `w`), found by a toolbar item's view.
    static func toolbarWindow(_ w: NSWindow) -> NSWindow? {
        (w.toolbar?.items ?? []).lazy.compactMap { $0.view?.window }.first { $0 !== w }
    }

    /// One left-edge reveal of a hidden sidebar: ours (SidebarPeek, an overlay, the same in both modes). AppKit also reveals
    /// a collapsed sidebar split item when the pointer touches the screen edge in full screen, and lays the content out
    /// again beside it (the whole content moves). There is no public switch: NSSplitViewItem `revealsOnEdgeHoverInFullscreen`
    /// is set only where the item answers to it, else left alone.
    static func disableSystemEdgeReveal(_ w: NSWindow) {
        for item in sidebarItems(w) where item.value(forKey: edgeRevealKey) as? Bool != false {
            item.setValue(false, forKey: edgeRevealKey)
        }
    }
    static let edgeRevealKey = "revealsOnEdgeHoverInFullscreen"
    static func sidebarItems(_ w: NSWindow) -> [NSSplitViewItem] {
        guard let sv = WindowWidthTracker.splitView(in: w.contentView), let c = sv.delegate as? NSSplitViewController else { return [] }
        return c.splitViewItems.filter {
            $0.behavior == .sidebar && $0.responds(to: NSSelectorFromString("setRevealsOnEdgeHoverInFullscreen:"))
        }
    }
}

/// Full screen at rest: the sidebar toggle only. No window controls: the system shows its own (close, exit full screen)
/// in the toolbar band that slides in with the menu bar, so ours would only duplicate them. The toggle stays at one
/// place at the left edge (`spot`), sidebar shown or hidden: the panel's slide is the only motion (a toggle that travels
/// to the panel's trailing edge runs on SwiftUI's clock while AppKit slides the panel on its own, so the two drift apart).
/// Hidden, a glass circle marks it (nothing behind it then). While the system's toolbar is revealed this steps aside at
/// once, so one toggle shows (the system's sits at the panel's trailing edge, or beside its lights).
struct FullScreenChrome: View {
    @Environment(AppModel.self) private var model
    /// A 36 pt circle (the island buttons' height) centred on the sidebar's icon column (the panel's inset + row inset 16
    /// + half the 22 pt icon box: 35 on 26, 27 before, so the glyph stands over the icons when the panel is in) and on the
    /// band, like the island.
    static func spot(band: CGFloat) -> CGRect {
        CGRect(x: SystemSidebar.inset + 27 - TopRow.height / 2, y: (band - TopRow.height) / 2, width: TopRow.height, height: TopRow.height)
    }

    var body: some View {
        let shown = model.sidebarVisibility != .detailOnly
        let f = Self.spot(band: model.toolbarBand)
        ZStack(alignment: .topLeading) {
            Color.clear
            Button { model.toggleSidebar() } label: {
                Image(systemName: "sidebar.left").font(.system(size: 15, weight: .regular)).foregroundStyle(.secondary)
                    .frame(width: f.width, height: f.height)
                    .background { Circle().fill(.clear).glassSurface(Circle()).opacity(shown ? 0 : 1) }
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .help(shown ? "Hide sidebar" : "Show sidebar")
            .accessibilityLabel(shown ? "Hide Sidebar" : "Show Sidebar")
            .probe { ChromeProbe.fullScreenToggle = $0 }
            .onDisappear { ChromeProbe.fullScreenToggle = .zero }
            .offset(x: f.minX, y: f.minY)
        }
        // At once, not with the reveal's ease: a fade would show both toggles while the system's slides in.
        .opacity(model.systemChromeRevealed ? 0 : 1)
        .animation(nil, value: model.systemChromeRevealed)
        .allowsHitTesting(!model.systemChromeRevealed)
    }
}

/// Full screen: the band the hidden toolbar leaves, as padding. The system moves the toolbar into its own full-screen
/// window there (hidden until the menu bar slides in), so the window's content no longer gets the toolbar's top inset
/// and would start at the top; this puts it back, less `top`, the view's own distance from the window top (the floating
/// sidebar panel starts 8 pt down), so its content starts where it starts windowed. Keyed on that move
/// (AppModel.toolbarDetached), not on the full-screen flag alone: a window whose toolbar still gives the inset gets nothing.
struct FullScreenBand: ViewModifier {
    @Environment(AppModel.self) private var model
    var top: CGFloat = 0
    func body(content: Content) -> some View {
        content.safeAreaPadding(.top, model.fullScreen && model.toolbarDetached ? max(0, model.toolbarBand - top) : 0)
    }
}
