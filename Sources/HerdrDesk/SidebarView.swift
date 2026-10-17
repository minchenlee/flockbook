import HerdrKit
import SwiftUI

/// One selectable sidebar row (the New tab row is an action, not a selection).
enum SidebarItem: Hashable { case inbox, overview, search, settings, workspace(String) }

/// The sidebar is a system sidebar `List`: the system gives row height, selection look (accent, inactive-window dimming),
/// arrow-key navigation, type-select and VoiceOver's list semantics.
struct SidebarView: View {
    @Environment(AppModel.self) private var model
    @State private var panel: CGRect = .zero   // global frame of the sidebar panel

    /// Screen and workspace <-> list selection. Choosing the current workspace again keeps its tab.
    private var selection: Binding<SidebarItem?> {
        Binding {
            switch model.screen {
            case .inbox: .inbox
            case .overview: .overview
            case .search: .search
            case .tab: model.workspace.map { .workspace($0.id) }
            case .settings: .settings
            }
        } set: { item in
            switch item {
            case .inbox: model.screen = .inbox
            case .overview: model.screen = .overview
            case .search: model.screen = .search
            case .settings: model.showSettings()
            case .workspace(let id):
                if id == model.workspace?.id { model.screen = .tab } else { model.select(workspace: id) }
            case nil: break
            }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            List(selection: selection) {
                Section {
                    // Same path as the island's + and ⌘T: a new tab in the current workspace.
                    Button { model.newTab() } label: { SidebarLine(title: "New tab", symbol: "square.and.pencil") }
                        .buttonStyle(.plain)
                        .disabled(!model.writesEnabled || model.workspace == nil)
                        .help("New tab (⌘T)")
                        .probe { ChromeProbe.rows["New tab"] = $0 }
                    // The Needs-you count keeps the app's one orange badge (a list .badge is plain text, without the fill).
                    navRow("Needs you", "tray", .inbox, badge: model.allBlocked.count)
                    navRow("Overview", "square.grid.2x2", .overview)
                    navRow("Search", "magnifyingglass", .search)
                }
                Section {
                    // The header is a row (not a section header): it gets the rows' own leading and trailing edges, so
                    // "Workspaces" lines up with the avatars and its button with the status column.
                    workspacesHeader
                    let slots = StatusColumn.slots(model.store.workspaces)
                    ForEach(model.store.workspaces) { ws in WorkspaceRow(ws: ws, slots: slots).tag(SidebarItem.workspace(ws.id)) }
                }
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
            // The list starts below the toolbar (the lights row with the system sidebar toggle), at the system's spacing.
            // Not under it: rows inside the title bar band would lose their clicks to the title bar.
            // Remote access listening: always in sight while on (Settings > Remote access turns it off).
            if let hub = model.remote, hub.running {
                RemoteFooter(hub: hub)
            }
            // Connection state only when something is wrong (as Mail): nothing while connected.
            if !model.store.connected {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange).imageScale(.small)
                    Text(connectionText).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    Spacer()
                }
                .padding(.horizontal, 18).padding(.vertical, 10)
                .transition(.opacity)
                .probe { ChromeProbe.connectionFooter = $0 }
                .onDisappear { ChromeProbe.connectionFooter = .zero }
            }
            // Settings, pinned at the foot: a list of its own, so it keeps the system row look and selection and stays in place
            // while the workspaces above scroll.
            List(selection: selection) {
                navRow("Settings", "gearshape", .settings)
            }
            .listStyle(.sidebar)
            .scrollContentBackground(.hidden)
            .scrollDisabled(true)
            .frame(height: 44)
        }
        // Full screen: the band the hidden toolbar leaves (rows start where they start windowed).
        .modifier(FullScreenBand(top: panel.minY))
        .animation(.easeOut(duration: 0.2), value: model.store.connected)
        .onChange(of: model.store.workspaces) { _, _ in model.lastUpdate = Date() }
        .onAppear { if !model.store.workspaces.isEmpty, model.lastUpdate == nil { model.lastUpdate = Date() } }
        // The window's surface, again, inside the sidebar panel: sidebar and content are one tone (see WindowSurface).
        .background { GlassReader { WindowSurface(glass: $0) }.ignoresSafeArea() }
        // Real top edge of the panel: the frame starts below the toolbar safe area.
        .onGeometryChange(for: CGRect.self) {
            var r = $0.frame(in: .global)
            r.origin.y -= $0.safeAreaInsets.top; r.size.height += $0.safeAreaInsets.top
            return r
        } action: {
            panel = $0; ChromeProbe.panel = $0
            // The docked panel only (the peek panel is a SidebarView too, floating), and only at rest (SystemSidebar.inset
            // in from the left, where the docked panel sits): during the show/hide slide its frame moves every frame, and a
            // value written per frame would make the full-screen toggle (FullScreenChrome) chase it instead of animating once.
            if model.sidebarVisibility != .detailOnly, $0.minX >= SystemSidebar.inset - 0.5, abs(model.sidebarMaxX - $0.maxX) > 0.5 { model.sidebarMaxX = $0.maxX }
        }
    }

    /// Connected / Reconnecting… (stale data, with last update time) / Connecting… (no data yet).
    private var connectionText: String {
        guard !model.store.workspaces.isEmpty else { return model.store.problem?.short ?? "Connecting…" }
        guard let t = model.lastUpdate else { return "Reconnecting…" }
        return "Reconnecting… · updated \(t.formatted(date: .omitted, time: .standard))"
    }

    private var workspacesHeader: some View {
        HStack(alignment: .center) {
            Text("Workspaces")
                .font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
                .probe { ChromeProbe.rows["Workspaces"] = $0 }
            Spacer(minLength: 4)
            // The symbol's plus badge overhangs its frame by about 2 pt: inset it so its ink ends at the glyphs' edge.
            Button { model.beginNewWorkspace() } label: { Image(systemName: "folder.badge.plus").foregroundStyle(.secondary) }
                .buttonStyle(.plain)
                .padding(.trailing, 2)
                .disabled(!model.writesEnabled)
                .help("New workspace… (⇧⌘N)")
                .accessibilityLabel("New workspace")
                .probe { ChromeProbe.newWorkspace = $0 }
                .onAppear { ChromeProbe.newWorkspaceAction = { model.beginNewWorkspace() } }
        }
        .padding(.top, 6)
        .selectionDisabled()
    }

    private func navRow(_ title: String, _ symbol: String, _ item: SidebarItem, badge: Int = 0) -> some View {
        HStack {
            SidebarLine(title: title, symbol: symbol)
                .probe { ChromeProbe.rows[title] = $0 }
            Spacer(minLength: 4)
            if badge > 0 { BlockedBadge(count: badge).probe { ChromeProbe.rows["badge:\(title)"] = $0 } }
        }
        .tag(item)
    }
}

/// Symbol and title of a nav row. The symbol sits in a box as wide as a workspace avatar (Monogram), at the same spacing,
/// so nav symbols, avatars and all row titles share one leading edge. Title primary, symbol secondary: the list's default
/// dims both in a window that is not key.
struct SidebarLine: View {
    /// Monogram's side.
    static let box: CGFloat = 22
    let title: String
    let symbol: String
    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: symbol).foregroundStyle(.secondary)
                .frame(width: Self.box, height: Self.box)
                .probe { ChromeProbe.rows["icon:\(title)"] = $0 }
            Text(title).foregroundStyle(.primary)
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

extension View {
    /// Test probe: reports the view's global frame.
    func probe(_ report: @escaping (CGRect) -> Void) -> some View {
        onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { report($0) }
    }
}

/// Per-tab status glyphs at the end of a workspace row (the tab island's glyphs, in tab order): one right-aligned column,
/// the same width in every row (as many slots as the busiest workspace needs, at most five), so the glyphs line up and
/// the name gets the rest. Up to five tabs: one glyph each. More: the three most urgent (blocked, working, done, idle)
/// and "+N" in the last two slots. Detail is in the tooltip and the accessibility value.
struct StatusColumn: View {
    static let maxSlots = 5
    static let glyph: CGFloat = 12, spacing: CGFloat = 3
    let tabs: [Tab]
    /// Slots of the shared column.
    var slots = maxSlots

    static func slots(_ ws: [Workspace]) -> Int { min(ws.map(\.tabs.count).max() ?? 0, maxSlots) }
    static func width(_ slots: Int) -> CGFloat { CGFloat(slots) * glyph + CGFloat(max(slots - 1, 0)) * spacing }
    /// The tabs to draw (tab order) and the count left out.
    static func shown(_ tabs: [Tab]) -> (tabs: [Tab], more: Int) {
        guard tabs.count > maxSlots else { return (tabs, 0) }
        let n = maxSlots - 2
        func urgency(_ s: AgentStatus) -> Int { switch s { case .blocked: 4; case .working: 3; case .done: 2; case .idle: 1; case .unknown: 0 } }
        let keep = Set(tabs.enumerated().sorted { (urgency($0.element.status), -$0.offset) > (urgency($1.element.status), -$1.offset) }
            .prefix(n).map(\.element.id))
        return (tabs.filter { keep.contains($0.id) }, tabs.count - n)
    }

    var body: some View {
        let (shown, more) = Self.shown(tabs)
        HStack(spacing: Self.spacing) {
            ForEach(shown) { StatusGlyph(status: $0.status, size: Self.glyph) }
            if more > 0 {
                Text("+\(more)").font(.caption2.weight(.medium)).monospacedDigit().foregroundStyle(.secondary)
                    .lineLimit(1).minimumScaleFactor(0.8)
                    .frame(width: Self.width(2), alignment: .trailing)
            }
        }
        .frame(width: Self.width(slots), alignment: .trailing)
        .accessibilityHidden(true)   // the row's accessibility value carries the summary
    }
}

/// Workspace row title and tooltip/VoiceOver summary.
enum WorkspaceSummary {
    /// "Meadow · Migration" -> title "Meadow", the rest goes to the tooltip.
    static func title(_ ws: Workspace) -> String {
        let name = Monogram.displayName(ws.label)
        return name.components(separatedBy: " · ").first.map { $0.isEmpty ? name : $0 } ?? name
    }
    static func tooltip(_ ws: Workspace) -> String {
        var parts = [Monogram.displayName(ws.label)]
        if ws.blockedCount > 0 { parts.append("\(ws.blockedCount) need you") }
        if ws.workingCount > 0 { parts.append("\(ws.workingCount) working") }
        let done = ws.tabs.filter { $0.status == .done }.count
        if done > 0 { parts.append("\(done) done") }
        parts.append("\(ws.tabs.count) tab\(ws.tabs.count == 1 ? "" : "s")")
        return parts.joined(separator: " · ")
    }
}

struct WorkspaceRow: View {
    @Environment(AppModel.self) private var model
    let ws: Workspace
    var slots = StatusColumn.maxSlots
    @State private var draft = ""
    @FocusState private var editing: Bool

    var body: some View {
        let quiet = ws.blockedCount == 0 && ws.workingCount == 0
        HStack(spacing: 8) {
            Monogram(text: ws.label).probe { ChromeProbe.rows["avatar:\(ws.id)"] = $0 }
            if model.renamingWorkspaceID == ws.id {
                TextField("Workspace name", text: $draft)
                    .textFieldStyle(.plain).focused($editing)
                    .onSubmit { model.rename(ws, to: draft) }
                    .onExitCommand { model.renamingWorkspaceID = nil }
                    .onAppear { draft = ws.label; editing = true }
                    .onChange(of: editing) { _, f in if !f, model.renamingWorkspaceID == ws.id { model.rename(ws, to: draft) } }
            } else {
                Text(WorkspaceSummary.title(ws)).lineLimit(1).truncationMode(.tail)
                    .foregroundStyle(quiet ? .secondary : .primary)
                    .probe { ChromeProbe.rows["title:\(ws.id)"] = $0 }
            }
            Spacer(minLength: 0)
            StatusColumn(tabs: ws.tabs, slots: slots).probe { ChromeProbe.rows["status:\(ws.id)"] = $0 }
        }
        .help(WorkspaceSummary.tooltip(ws))
        .accessibilityElement(children: .combine)
        .accessibilityLabel(WorkspaceSummary.title(ws))
        .accessibilityValue(WorkspaceSummary.tooltip(ws))
        .contextMenu { MenuEntries(entries: model.workspaceMenu(ws)) }
    }
}

/// Hover reveal of a hidden sidebar: a thin strip on the content's left edge, and the sidebar as a floating glass panel while
/// the pointer is on the strip or the panel. Does not change `sidebarVisibility` or push the content. Hover comes from
/// AppKit tracking areas (`onHover`), no polling.
struct SidebarPeek: View {
    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        // Windowed and full screen alike: an overlay, the content does not move (AppKit's own full-screen edge reveal, which
        // lays the content out again, is off: WindowChrome.disableSystemEdgeReveal).
        let active = model.sidebarVisibility == .detailOnly
        ZStack(alignment: .leading) {
            if active {
                Color.clear.frame(width: 7)
                    .contentShape(Rectangle())
                    .onHover { model.setPeekHover(strip: $0) }
                    .probe { ChromeProbe.peekStrip = $0 }
                    .onDisappear { ChromeProbe.peekStrip = .zero }
                    .padding(.top, 12)   // top clearance: the menu-bar trigger in full screen
            }
            if active && model.sidebarPeek {
                SidebarView()
                    .frame(width: 250)
                    .modifier(PeekGlass())
                    .probe { ChromeProbe.peekPanel = $0 }
                    .padding(SystemSidebar.inset)   // where the docked panel sits
                    .onHover { model.setPeekHover(panel: $0) }
                    .onDisappear { ChromeProbe.peekPanel = .zero }
                    .transition(reduceMotion ? .opacity : .move(edge: .leading).combined(with: .opacity))
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        .ignoresSafeArea(.container, edges: .top)
        .animation(reduceMotion ? .easeOut(duration: 0.15) : .smooth(duration: 0.25), value: model.sidebarPeek)
    }
}

/// The floating panel: SidebarView already draws the window surface, so the panel is that surface in a rounded shape,
/// lifted off the content by a hairline and a shadow (no second glass layer).
private struct PeekGlass: ViewModifier {
    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: SystemSidebar.cornerRadius)
        content.clipShape(shape)
            .overlay(shape.strokeBorder(Color.primary.opacity(0.12), lineWidth: 0.5))
            .shadow(color: .black.opacity(0.25), radius: 18, y: 6)
    }
}

/// "Remote access on" with the connection count, at the foot of the sidebar; a click opens the Settings page.
struct RemoteFooter: View {
    let hub: RemoteHub
    @Environment(AppModel.self) private var model
    var body: some View {
        Button { model.showSettings() } label: {
            HStack(spacing: 6) {
                Image(systemName: "antenna.radiowaves.left.and.right").foregroundStyle(.orange).imageScale(.small)
                Text(hub.connections > 0 ? "Remote access on · \(hub.connections) connected" : "Remote access on")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Spacer()
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Settings > Remote access")
        .padding(.horizontal, 18).padding(.vertical, 10)
    }
}
