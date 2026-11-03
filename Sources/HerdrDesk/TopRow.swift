import HerdrKit
import SwiftUI

/// Which copy of the tab row a view belongs to: the system toolbar's, or the full-screen content copy (FullScreenTopRow).
enum IslandHost: Hashable { case toolbar, content }
private struct IslandHostKey: EnvironmentKey { static let defaultValue = IslandHost.toolbar }
extension EnvironmentValues {
    var islandHost: IslandHost { get { self[IslandHostKey.self] } set { self[IslandHostKey.self] = newValue } }
}

struct TopRow: View {
    @Environment(AppModel.self) private var model
    @Environment(\.islandHost) private var host
    @State private var scroll = ScrollPosition(idType: String.self)
    /// Tabs currently (mostly) inside the strip. Written only when a tab enters or leaves, never per frame.
    @State private var visibleTabs: Set<String> = []
    /// Natural width of all tab pills; the island is this wide, up to `tabRowWidth`. Updated on change only.
    @State private var contentWidth: CGFloat = 0
    /// Width of each pill, for drag reordering. Updated on change only.
    @State private var widths: [String: CGFloat] = [:]
    /// The drag in progress: the strip shows `order` (the others make room, animated) and the dragged pill follows the pointer.
    @State private var drag: TabDrag?
    /// Order shown after a drop until herdr's snapshot has it (so the tab does not jump back for a moment).
    @State private var dropped: [String]?
    /// Whether tabs are scrolled out of view past each end; only that end fades.
    @State private var hiddenEnds = HiddenEnds()
    static let pillSpacing: CGFloat = 2
    /// One height for the island, the + / terminal capsule and the full-screen sidebar toggle circle: the height the system
    /// toolbar draws its button group (36 pt on macOS 26, measured by --test-island-height). Capsules: radius height / 2.
    static let height: CGFloat = 36
    /// Space between the pills and the capsule edge: the same on all four sides (pills are 30 tall).
    static let inset: CGFloat = (height - TabPill.height) / 2

    /// Tab strip (`+` and Show terminal are system toolbar buttons next to it, `TopRowActions`). Lives in the native window toolbar (see `TopRowToolbar`).
    var body: some View {
        let tabs = model.workspace?.tabs ?? []
        HStack(spacing: 6) {
            strip(tabs)
        }
        // Width must not depend on the tab count: a wider item makes NSToolbar move it into the ">>" overflow
        // menu and it never comes back. The width follows the window (AppModel.tabRowWidth); the strip scrolls inside.
        .frame(minWidth: 120, idealWidth: islandWidth, maxWidth: islandWidth)
        .frame(height: Self.height)
        .modifier(IslandGlass())   // the only glass shape: exactly the content width, nothing behind it
        .probe { [probes = TabProbe.writes(host, model)] r in
            if probes { TabProbe.island = r; TabProbe.observeIsland(r, model: model, host: host) }
        }
        .onAppear { TabProbe.rowsShown += 1 }
        .onDisappear { TabProbe.rowsShown -= 1 }
    }

    /// Island width: content (pills + the inset at both ends), never above the window-derived cap.
    /// The cap keeps the item inside the space NSToolbar offers, so it cannot fall into ">>" overflow.
    private var islandWidth: CGFloat {
        let wanted = contentWidth + 2 * Self.inset
        return max(120, min(model.tabRowWidth - TopRowActions.width, wanted))
    }

    private func strip(_ tabs: [Tab]) -> some View {
        let shown = Self.ordered(tabs, by: drag?.order ?? dropped)
        return ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: Self.pillSpacing) {
                ForEach(Array(shown.enumerated()), id: \.element.id) { i, tab in
                    TabPill(tab: tab, selected: tab.id == model.tab?.id, position: i + 1,
                            dragOffset: drag.map { $0.id == tab.id ? $0.offset(widths, spacing: Self.pillSpacing) : 0 } ?? 0,
                            onDrag: { dragged(tab, tabs, dx: $0) }, onDrop: { drop(tabs) })
                        .zIndex(drag?.id == tab.id ? 1 : 0)
                        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { w in if abs((widths[tab.id] ?? 0) - w) > 0.5 { widths[tab.id] = w } }
                        .id(tab.id)
                        .onScrollVisibilityChange(threshold: 0.6) { visible in
                            if visible { visibleTabs.insert(tab.id) } else { visibleTabs.remove(tab.id) }
                        }
                }
            }
            .scrollTargetLayout()
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { if abs($0 - contentWidth) > 0.5 { contentWidth = $0 } }
            .padding(.horizontal, Self.inset)
        }
        .onScrollGeometryChange(for: HiddenEnds.self) { g in
            HiddenEnds(leading: g.contentOffset.x > 1, trailing: g.contentOffset.x + g.containerSize.width < g.contentSize.width - 1)
        } action: { _, ends in hiddenEnds = ends }
        .scrollPosition($scroll, anchor: .center)
        // Scroll the selected tab into view after the update pass, not inside it.
        .task(id: model.tab?.id) {
            // Debounced: rapid tab churn cancels earlier runs; no scroll when the tab is already in view.
            try? await Task.sleep(for: .milliseconds(TabProbe.selectionDelayMilliseconds))
            // scrollTo always scrolls: the strip forgets its offset when the workspace changes, a remembered id would not.
            if !Task.isCancelled, let id = model.tab?.id {
                let needsScroll = !visibleTabs.contains(id)
                TabProbe.observeSelectionTask(id, model: model, host: host, scrolled: needsScroll)
                if needsScroll {
                    if TabProbe.traceSelection { print("tab-scroll-trace: \(host) scroll selected \(id), delay \(TabProbe.selectionDelayMilliseconds) ms") }
                    scroll.scrollTo(id: id, anchor: .center)
                }
            }
        }
        // herdr's snapshot caught up with the drop (or anything else changed the tabs): show its order again.
        .onChange(of: tabs.map(\.id)) { _, _ in dropped = nil }
        .task(id: dropped) {
            // The move failed or herdr never answered: fall back to the snapshot's order.
            guard dropped != nil else { return }
            try? await Task.sleep(for: .seconds(3))
            if !Task.isCancelled { withAnimation(.snappy(duration: 0.25)) { dropped = nil } }
        }
        .mask(fadeMask)
        .overlay(alignment: .leading) { marker(edge: .leading, tabs: tabs) }
        .overlay(alignment: .trailing) { marker(edge: .trailing, tabs: tabs) }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// `tabs` in `order` (ids). Tabs not in `order` (new ones) go at the end; ids without a tab (closed ones) drop out.
    static func ordered(_ tabs: [Tab], by order: [String]?) -> [Tab] {
        guard let order else { return tabs }
        let byID = Dictionary(tabs.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        return order.compactMap { byID[$0] } + tabs.filter { !order.contains($0.id) }
    }

    /// Live reorder: the dragged pill goes to the slot nearest to where the pointer has carried it; the others slide aside.
    private func dragged(_ tab: Tab, _ tabs: [Tab], dx: CGFloat) {
        guard model.writesEnabled else { return }   // moving a tab is a write
        var d = drag ?? TabDrag(id: tab.id, start: Self.ordered(tabs, by: dropped).map(\.id), dx: 0)
        d.dx = dx
        let order = d.nearestOrder(widths, spacing: Self.pillSpacing)
        if order != d.order {
            d.order = order
            withAnimation(.snappy(duration: 0.22)) { drag = d }
        } else {
            drag = d
        }
    }

    private func drop(_ tabs: [Tab]) {
        guard let d = drag else { return }
        let to = d.order.firstIndex(of: d.id) ?? 0
        let moved = to != tabs.firstIndex { $0.id == d.id }
        withAnimation(.snappy(duration: 0.22)) {
            if moved { dropped = d.order }
            drag = nil
        }
        if moved, let tab = tabs.first(where: { $0.id == d.id }) { model.moveTab(tab, to: to) }
    }

    /// Edge fade, only at an end with tabs scrolled out of view (the resting first and last pills sit `inset` from the edge,
    /// so a fade there would cut into them).
    private var fadeMask: some View {
        HStack(spacing: 0) {
            LinearGradient(colors: [hiddenEnds.leading ? .clear : .black, .black], startPoint: .leading, endPoint: .trailing).frame(width: 14)
            Rectangle().fill(.black)
            LinearGradient(colors: [.black, hiddenEnds.trailing ? .clear : .black], startPoint: .leading, endPoint: .trailing).frame(width: 14)
        }
    }

    /// Orange pill at the clipped edge when a blocked tab is out of view.
    @ViewBuilder private func marker(edge: HorizontalEdge, tabs: [Tab]) -> some View {
        let firstVisible = tabs.firstIndex { visibleTabs.contains($0.id) }
        let hidden: [Tab] = firstVisible.map { fv in
            tabs.enumerated().filter { i, t in
                t.status == .blocked && !visibleTabs.contains(t.id) && (edge == .leading ? i < fv : i > fv)
            }.map(\.element)
        } ?? []
        if let first = hidden.first, let last = hidden.last {
            Button {
                scroll.scrollTo(id: edge == .leading ? last.id : first.id, anchor: .center)
            } label: {
                // Same look as the Needs-you count in the sidebar (BlockedBadge), with the direction to scroll.
                HStack(spacing: 2) {
                    if edge == .leading { Image(systemName: "chevron.left").imageScale(.small) }
                    Text("\(hidden.count)").monospacedDigit()
                    if edge == .trailing { Image(systemName: "chevron.right").imageScale(.small) }
                }
                .font(.caption.weight(.semibold)).foregroundStyle(.white)
                .padding(.horizontal, 6).frame(minWidth: 18, minHeight: 18)
                .background(Color.orange, in: Capsule())
                .padding(.horizontal, 4)
            }
            .buttonStyle(.plain)
            .help("\(hidden.count) tab\(hidden.count == 1 ? "" : "s") need you: click to show")
            .accessibilityLabel("\(hidden.count) tab\(hidden.count == 1 ? "" : "s") out of view need you")
        }
    }
}

struct TabPill: View {
    static let height: CGFloat = 30
    let tab: Tab
    let selected: Bool
    /// 1-based position in the strip (Tab.number stays with a tab after a move, so it is not shown).
    var position = 0
    var dragOffset: CGFloat = 0
    var onDrag: (CGFloat) -> Void = { _ in }
    var onDrop: () -> Void = {}
    @State private var hover = false
    @FocusState private var editing: Bool
    @Environment(AppModel.self) private var model
    @Environment(\.islandHost) private var host

    var body: some View {
        let closable = (selected || hover) && model.writesEnabled && model.layoutOK(tab)
        // One field at a time, in the active copy (two focused fields would take focus from each other and end the rename).
        let renaming = model.renamingTabID == tab.id && model.islandActive(host)
        let probes = TabProbe.writes(host, model)
        HStack(spacing: 4) {
            HStack(spacing: 6) {
                StatusGlyph(status: tab.status, size: 13)
                if renaming {
                    TextField("Tab name", text: Bindable(model).renameDraft)
                        .textFieldStyle(.plain).font(.callout).frame(width: 120)
                        .focused($editing)
                        .onSubmit { model.rename(tab, to: model.renameDraft) }
                        .onExitCommand { model.renamingTabID = nil }
                        .onAppear {
                            // A new rename starts from the label; a hand-over from the other copy keeps the text typed so far.
                            if model.renameDraftTab != tab.id { model.renameDraft = tab.label; model.renameDraftTab = tab.id }
                            editing = true
                        }
                        // Focus lost = done, unless the field went away because the other copy took over.
                        .onChange(of: editing) { _, f in
                            if !f, model.renamingTabID == tab.id, model.islandActive(host) { model.rename(tab, to: model.renameDraft) }
                        }
                } else {
                    Text(tab.label)
                        .font(.callout.weight(selected || tab.status == .blocked ? .semibold : .medium))
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .foregroundStyle(selected || tab.status == .blocked ? Color.primary : Color.primary.opacity(0.78))
                }
                if tab.isSplit { Image(systemName: "rectangle.split.1x2").font(.caption).foregroundStyle(.tertiary) }
            }
            .frame(maxHeight: .infinity)
            .probe { r in if probes { TabProbe.labels[tab.id] = r } }
            if closable { Color.clear.frame(width: 20, height: 20) }   // room for the ×, which sits above the mouse view
        }
        .padding(.leading, 10).padding(.trailing, closable ? 5 : 10)
        .frame(minWidth: 72, maxWidth: 200, minHeight: Self.height, maxHeight: Self.height)
        .fixedSize(horizontal: true, vertical: false)
        .background {
            // One layer only: the island is the glass; a selected tab is a faint fill, no border or material.
            if selected {
                Capsule().fill(Color.primary.opacity(0.1))
            } else if hover {
                Capsule().fill(Color.primary.opacity(0.05))
            }
        }
        // Every mouse event on the pill goes to one AppKit view (see TabMouseView). Not while renaming: the field takes the clicks.
        .overlay {
            if !renaming {
                TabMouseView(
                    onSelect: { if let ws = model.workspace { model.select(tab: tab, in: ws) } },
                    onRename: { if model.writesEnabled, model.layoutOK(tab), let ws = model.workspace { model.select(tab: tab, in: ws); model.renamingTabID = tab.id } },
                    menu: { model.tabMenu(tab) },
                    onDrag: onDrag, onDrop: onDrop)
                    .probe { r in
                        if probes { TabProbe.pills[tab.id] = r; TabProbe.observePill(r, id: tab.id, model: model, host: host) }
                    }
            }
        }
        .overlay(alignment: .trailing) { if closable { TabCloseButton(tab: tab).padding(.trailing, 5) } }
        .offset(x: dragOffset)
        .transaction(value: position) { if probes { TabProbe.moveAnimated[tab.id] = $0.animation != nil } }
        .onHover { hover = $0 }
        .help(tooltip)
        .modifier(TabAccessibility(tab: tab, selected: selected, value: tooltip))
    }

    private var tooltip: String {
        let a = tab.primaryAgent
        return [a?.name, a.map { $0.status.label }, "Tab \(position) · \(tab.panes.count) pane\(tab.panes.count == 1 ? "" : "s")"]
            .compactMap { $0 }.joined(separator: " · ")
    }
}

/// Test probes: global frame of each tab's label and of each tab's mouse view (the whole pill).
enum TabProbe {
    /// Controlled delayed selection work in the offscreen overflow check; normal debounce remains 200 ms.
    @MainActor static var selectionDelayMilliseconds = 200
    @MainActor static var traceSelection = false
    struct GeometryOwner: Hashable { let model: ObjectIdentifier; let host: IslandHost }
    struct SelectionTiming {
        let owner: GeometryOwner
        let id: String
        let start: ContinuousClock.Instant
        var visibleAfter: Duration?
        var taskAfter: Duration?
        var scrolled = false
        var initiallyVisible = false
    }
    @MainActor static var measureSelection = false
    @MainActor static var selectionTiming: SelectionTiming?
    @MainActor private static var timingIslands: [GeometryOwner: CGRect] = [:]
    @MainActor private static var timingPills: [GeometryOwner: [String: CGRect]] = [:]
    @MainActor static func beginSelection(_ id: String, model: AppModel, host: IslandHost) {
        selectionTiming = SelectionTiming(owner: GeometryOwner(model: ObjectIdentifier(model), host: host), id: id, start: .now)
        confirmVisibility(initial: true)
    }
    @MainActor static func observeIsland(_ frame: CGRect, model: AppModel, host: IslandHost) {
        guard measureSelection, writes(host, model) else { return }
        timingIslands[GeometryOwner(model: ObjectIdentifier(model), host: host)] = frame
        confirmVisibility()
    }
    @MainActor static func observePill(_ frame: CGRect, id: String, model: AppModel, host: IslandHost) {
        guard measureSelection, writes(host, model) else { return }
        timingPills[GeometryOwner(model: ObjectIdentifier(model), host: host), default: [:]][id] = frame
        confirmVisibility()
    }
    @MainActor static func observeSelectionTask(_ id: String, model: AppModel, host: IslandHost, scrolled: Bool) {
        guard measureSelection, var timing = selectionTiming, timing.id == id, timing.taskAfter == nil,
              timing.owner == GeometryOwner(model: ObjectIdentifier(model), host: host) else { return }
        timing.taskAfter = timing.start.duration(to: .now); timing.scrolled = scrolled
        selectionTiming = timing
    }
    @MainActor private static func confirmVisibility(initial: Bool = false) {
        guard var timing = selectionTiming, timing.visibleAfter == nil,
              let island = timingIslands[timing.owner], let pill = timingPills[timing.owner]?[timing.id],
              island.width > 0, pill.width > 0, pill.minX >= island.minX - 0.5, pill.maxX <= island.maxX + 0.5 else { return }
        timing.visibleAfter = initial ? .zero : timing.start.duration(to: .now)
        timing.initiallyVisible = initial
        selectionTiming = timing
    }
    nonisolated(unsafe) static var labels: [String: CGRect] = [:]
    nonisolated(unsafe) static var pills: [String: CGRect] = [:]
    nonisolated(unsafe) static var island: CGRect = .zero
    /// Copies of the tab row in the window now (toolbar and full-screen content).
    nonisolated(unsafe) static var rowsShown = 0
    /// Whether the last change of each tab's position came with an animation.
    nonisolated(unsafe) static var moveAnimated: [String: Bool] = [:]
    /// The copy whose frames the probes hold: the toolbar's windowed, the content copy in full screen (the toolbar copy's
    /// global frames there are in its own hosting view, not the window).
    @MainActor static func writes(_ host: IslandHost, _ model: AppModel) -> Bool { (host == .content) == model.fullScreen }
}

/// All mouse input on a tab pill, in one AppKit view: a click selects, a double click renames, a right click (or control-click)
/// opens the tab menu, a sideways drag reorders. The tab strip sits in the window toolbar, where SwiftUI gestures and
/// `.contextMenu` lose to the title bar and the toolbar (double click zooms the window, right click opens the toolbar's own
/// "Icon and Text / Icon Only" menu). A view that handles the events itself gets them first, windowed and full screen alike.
struct TabMouseView: NSViewRepresentable {
    var onSelect: () -> Void
    var onRename: () -> Void
    var menu: () -> [MenuEntry]
    /// Pointer movement (x, points) since the press, once the drag has started.
    var onDrag: (CGFloat) -> Void
    var onDrop: () -> Void
    func makeNSView(context: Context) -> Catcher { Catcher() }
    func updateNSView(_ v: Catcher, context: Context) { v.handlers = self }

    final class Catcher: NSView {
        var handlers: TabMouseView?
        private var pressX: CGFloat?
        private var dragging = false
        private var menuActions: [() -> Void] = []
        /// Distance before a press becomes a drag, so a click with a small wobble still only selects.
        static let dragThreshold: CGFloat = 4

        override var mouseDownCanMoveWindow: Bool { false }
        override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

        override func mouseDown(with event: NSEvent) {
            if event.modifierFlags.contains(.control), let m = menu(for: event) {
                NSMenu.popUpContextMenu(m, with: event, for: self); return
            }
            if event.clickCount >= 2 { pressX = nil; handlers?.onRename(); return }
            handlers?.onSelect()
            pressX = NSEvent.mouseLocation.x; dragging = false
        }
        override func mouseDragged(with event: NSEvent) { drag(to: NSEvent.mouseLocation.x) }
        override func mouseUp(with event: NSEvent) { release() }

        /// Screen x of the pointer. Screen coordinates: the pill itself moves while it is dragged.
        func drag(to x: CGFloat) {
            guard let x0 = pressX else { return }
            let dx = x - x0
            if !dragging, abs(dx) < Self.dragThreshold { return }
            dragging = true
            handlers?.onDrag(dx)
        }
        func press(at x: CGFloat) { pressX = x; dragging = false }
        func release() {
            if dragging { handlers?.onDrop() }
            pressX = nil; dragging = false
        }

        override func menu(for event: NSEvent) -> NSMenu? {
            guard let entries = handlers?.menu(), !entries.isEmpty else { return nil }
            let m = NSMenu()
            m.autoenablesItems = false
            menuActions = entries.map(\.run)
            for (i, e) in entries.enumerated() {
                let item = NSMenuItem(title: e.title, action: #selector(runItem(_:)), keyEquivalent: "")
                item.target = self
                item.tag = i
                item.image = NSImage(systemSymbolName: e.symbol, accessibilityDescription: nil)
                item.isEnabled = e.enabled
                m.addItem(item)
            }
            return m
        }
        @objc private func runItem(_ sender: NSMenuItem) {
            if menuActions.indices.contains(sender.tag) { menuActions[sender.tag]() }
        }
    }
}

/// Test probe: frame of each visible close button, by tab id.
enum CloseProbe { nonisolated(unsafe) static var frames: [String: CGRect] = [:]
    nonisolated(unsafe) static var actions: [String: () -> Void] = [:] }

/// The × on a tab. A real button with its own hit area, so the pill's tap gestures (select, rename) do not take the click.
struct TabCloseButton: View {
    let tab: Tab
    @Environment(AppModel.self) private var model
    @Environment(\.islandHost) private var host
    @State private var hover = false
    var body: some View {
        let probes = TabProbe.writes(host, model)
        Button { model.closeTab(tab) } label: {
            Image(systemName: "xmark").font(.caption2.weight(.semibold))
                .foregroundStyle(hover ? Color.primary : Color.secondary)
                .frame(width: 20, height: 20)
                .background(Color.primary.opacity(hover ? 0.12 : 0), in: Circle())
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .help("Close tab (⌘W)")
        .accessibilityLabel("Close tab")
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { if probes { CloseProbe.frames[tab.id] = $0 } }
        .onAppear { if probes { CloseProbe.actions[tab.id] = { [model, tab] in model.closeTab(tab) } } }
        .onDisappear { if probes { CloseProbe.frames[tab.id] = nil; CloseProbe.actions[tab.id] = nil } }
    }
}

/// The toolbar's +: what Settings > General chose (`PlusAction`): a new tab, or a split of the focused pane. Its
/// tooltip and accessibility label say which.
struct PlusButton<Content: View>: View {
    let model: AppModel
    @ViewBuilder var label: (PlusAction) -> Content
    @AppStorage(PlusAction.key, store: AppDefaults.store) private var stored = PlusAction.newTab.rawValue

    var body: some View {
        let action = PlusAction(stored: stored)
        Button { model.plus() } label: { label(action) }
            .help(action.help)
            .accessibilityLabel(action.accessibilityLabel)
            .disabled(!model.layoutEnabled)
    }
}

/// The + (`PlusButton`) and Show terminal: plain system toolbar buttons next to the island. The system draws their shared glass
/// capsule, pressed, focus, disabled and Increase Contrast looks.
struct TopRowActions: ToolbarContent {
    /// Room the two buttons take beside the island (for the island's width cap).
    static let width: CGFloat = 84
    let model: AppModel
    var body: some ToolbarContent {
        ToolbarItemGroup(placement: .principal) {
            PlusButton(model: model) { Label($0.accessibilityLabel, systemImage: "plus") }
            Toggle(isOn: Bindable(model).showTerminal) { Label("Show Terminal", systemImage: "apple.terminal") }
                .help("Show terminal (⌃`)")
                .disabled(model.screen != .tab)
        }
    }
}

/// The tab row as native toolbar content: the system keeps it in the title bar (same row as the traffic lights),
/// routes clicks to it. In full screen too: the system hides that toolbar until the menu bar slides in, and the band it
/// slides in with then shows the same row. At rest there the row is content (`FullScreenTopRow`) at the same place.
struct TopRowToolbar: ToolbarContent {
    let model: AppModel   // passed explicitly: toolbar items are hosted outside the content's environment
    var body: some ToolbarContent {
        // Centered in the toolbar. No system background: TopRow draws its own glass capsule, so no wider shape sits behind it.
        // None while Settings shows: it is not about a workspace.
        if model.screen != .settings {
            PrincipalToolbarItem { TopRow().environment(model).environment(\.islandHost, .toolbar) }
            TopRowActions(model: model)
        }
    }
}

/// Full screen at rest: the tab island and its two buttons as content, where the system toolbar lays them out
/// (AppModel.toolbarIsland / toolbarActions; centred above the detail column until measured). One model with the toolbar
/// copy: drag, rename and menus act the same. Hidden at once while the toolbar is revealed, so one island shows.
struct FullScreenTopRow: View {
    @Environment(AppModel.self) private var model
    var body: some View {
        let island = model.toolbarIsland, actions = model.toolbarActions
        ZStack(alignment: .topLeading) {
            Color.clear
            if island.width > 0, actions.width > 0 {
                // One group, centred on the toolbar's (the island and the buttons at the toolbar's places while its frames are
                // current). At rest in real full screen the slid-away toolbar does not lay out again, so its frames can be an
                // older, narrower island's: the island grows from the same centre and pushes the buttons along, never under them.
                HStack(spacing: actions.minX - island.maxX) { TopRow(); buttons.frame(width: actions.width) }
                    .fixedSize()
                    .position(x: island.union(actions).midX, y: island.midY)
            } else {
                HStack(spacing: 10) { TopRow(); buttons }
                    .frame(height: TopRow.height)
                    .frame(maxWidth: .infinity)
                    .padding(.leading, model.sidebarVisibility == .detailOnly ? 0 : model.sidebarMaxX)
                    .padding(.top, max(0, (model.toolbarBand - TopRow.height) / 2))
            }
        }
        .environment(\.islandHost, .content)
        .opacity(model.systemChromeRevealed ? 0 : 1)
        .animation(nil, value: model.systemChromeRevealed)
        .allowsHitTesting(!model.systemChromeRevealed)
    }

    private var buttons: some View {
        GlassGroup {
            HStack(spacing: 0) {
                PlusButton(model: model) { _ in Image(systemName: "plus").frame(width: 36, height: TopRow.height).contentShape(Rectangle()) }
                Toggle(isOn: Bindable(model).showTerminal) {
                    Image(systemName: "apple.terminal").frame(width: 36, height: TopRow.height).contentShape(Rectangle())
                }
                .toggleStyle(.button)
                .help("Show terminal (⌃`)")
                .accessibilityLabel("Show Terminal")
                .disabled(model.screen != .tab)
            }
            .buttonStyle(.borderless)
            .font(.system(size: 15, weight: .medium))
            .foregroundStyle(.secondary)
            .padding(.horizontal, 2)
            .glassSurface(Capsule(), interactive: true)
        }
        .probe { ChromeProbe.fullScreenActions = $0 }
    }
}

/// Glass capsule island. Regular glass (HIG: clear is for photo/video backgrounds): text scrolling behind shows only as a blur.
/// Flat (snapshot) mode and Reduce Transparency: plain fill, its hairline inside the capsule (the same height as the
/// buttons' glass).
struct IslandGlass: ViewModifier {
    @Environment(\.flatGlass) private var flat
    @Environment(\.legacyLook) private var legacy
    @Environment(\.accessibilityReduceTransparency) private var reduce
    func body(content: Content) -> some View {
        if flat && !legacy || reduce {
            content.background(Color(nsColor: .controlBackgroundColor), in: Capsule())
                // strokeBorder is a view, not a shape: hit-testable over the whole capsule, it would take the tabs' clicks.
                .overlay(Capsule().strokeBorder(Color.primary.opacity(0.1)).allowsHitTesting(false))
        } else {
            content.glassSurface(Capsule())
        }
    }
}

/// VoiceOver: one selectable button per tab (the mouse view is opaque to it), with the tab menu's actions.
private struct TabAccessibility: ViewModifier {
    let tab: Tab
    let selected: Bool
    let value: String
    @Environment(AppModel.self) private var model
    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(tab.label)
            .accessibilityValue(value)
            .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            .accessibilityAction { if let ws = model.workspace { model.select(tab: tab, in: ws) } }
            .accessibilityActions {
                ForEach(model.tabMenu(tab).filter(\.enabled)) { e in
                    Button(e.title.replacingOccurrences(of: "…", with: ""), action: e.run)
                }
            }
    }
}

/// Tab strip ends with tabs scrolled out of view.
struct HiddenEnds: Equatable { var leading = false, trailing = false }

/// A tab drag in the strip. Pure layout arithmetic (tested by --test-tabdrag).
struct TabDrag: Equatable {
    let id: String
    /// Order when the drag started.
    let start: [String]
    /// Pointer movement since the press.
    var dx: CGFloat
    /// Order shown now.
    var order: [String]

    init(id: String, start: [String], dx: CGFloat) { self.id = id; self.start = start; self.dx = dx; order = start }

    /// Leading x of `id` when the pills are laid out in `order`.
    static func slotX(_ id: String, in order: [String], _ widths: [String: CGFloat], spacing: CGFloat) -> CGFloat {
        var x: CGFloat = 0
        for o in order { if o == id { return x }; x += (widths[o] ?? 0) + spacing }
        return x
    }

    /// Where the dragged pill is now: its start slot plus the pointer movement.
    func carriedX(_ widths: [String: CGFloat], spacing: CGFloat) -> CGFloat { Self.slotX(id, in: start, widths, spacing: spacing) + dx }

    /// The order that puts the dragged pill in the slot closest to where it is carried.
    func nearestOrder(_ widths: [String: CGFloat], spacing: CGFloat) -> [String] {
        let others = start.filter { $0 != id }
        let x = carriedX(widths, spacing: spacing)
        var best = 0, bestDist = CGFloat.infinity, slot: CGFloat = 0
        for i in 0...others.count {
            if abs(slot - x) < bestDist { bestDist = abs(slot - x); best = i }
            if i < others.count { slot += (widths[others[i]] ?? 0) + spacing }
        }
        var o = others
        o.insert(id, at: best)
        return o
    }

    /// Offset that keeps the dragged pill under the pointer while its layout slot is `order`'s.
    func offset(_ widths: [String: CGFloat], spacing: CGFloat) -> CGFloat {
        carriedX(widths, spacing: spacing) - Self.slotX(id, in: order, widths, spacing: spacing)
    }
}
