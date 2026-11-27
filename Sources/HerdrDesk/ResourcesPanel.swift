import AppKit
import HerdrKit
import SwiftUI

/// Resources: the docs, artifacts, PRs, commands and links the shown agent pointed you to in its replies. As the mod's
/// window (herdr-app/mods/flockbook/resources.js): a panel over the right of the chat (the chat does not move), opened
/// by ⌘⇧R and the toolbar's button; the button's badge counts the ones newer than you last saw.
///
/// The lists are in memory, the last `Resources.cacheMax` agents, so going back to an agent shows its list at once.
/// Kept in AppDefaults: the folded groups (for every agent), and per agent what was seen and what was removed.
@MainActor @Observable
final class ResourcesModel {
    static let seenKey = "resources.seen", removedKey = "resources.removed", collapsedKey = "resources.collapsed"
    nonisolated static let width: CGFloat = 320, gap: CGFloat = 12, narrow: CGFloat = 720
    /// The reply box's top before it is measured: this far over the pane's bottom.
    nonisolated static let inputGuess: CGFloat = 150

    struct Stamp: Equatable { var size: Int; var date: Date }
    struct Entry { var items: [Resources.Item]; var stamp: Stamp? }
    /// Show in chat: the pane scrolls to the row once.
    struct Jump: Equatable { var pane: String; var row: String; var token: Int }

    var open = false
    /// The "new" dots: newer than what was seen when the panel opened (or the agent changed).
    var seenAtOpen: Int64?
    private(set) var lists: [String: Entry] = [:]
    private var order: [String] = []
    private(set) var seen: [String: Int64]
    private(set) var removed: [String: [Resources.Removed]]
    private(set) var collapsed: Set<String>
    /// Groups shown past `Resources.rowLimit` in the open panel.
    var expanded: Set<String> = []
    var jump: Jump?
    /// The focused reply box's top, global (0: not measured).
    var inputTop: CGFloat = 0
    /// What a click did not open, or "Copied.", by row id.
    var note: (id: String, text: String, ok: Bool)?
    /// Runs a click's plan; returns a note (nil: done, nothing to say). Checks put their own.
    var perform: @MainActor (Resources.Plan) -> (text: String, ok: Bool)? = ResourcesModel.run
    private var reading: Set<String> = []
    /// Why an agent has no list (nil: its first read has not ended, and the panel says it is reading).
    private(set) var problem: [String: Resources.Problem] = [:]
    /// Each agent's log as read so far, newest part first: a later read parses only what the log grew by.
    private var progs: [String: Resources.Progressive] = [:]
    /// How much of the log is read (0…1) while its older parts are still being read; nil: all of it is read.
    var progress: [String: Double] = [:]
    /// The first chunk's size (checks put a small one).
    var chunk = Resources.Progressive.chunk
    /// When each agent's last read ended, and how long it took: a slow log is not read again at once.
    private var lastRead: [String: (end: Date, took: TimeInterval)] = [:]

    init() {
        let d = AppDefaults.store
        seen = (d.dictionary(forKey: Self.seenKey) as? [String: NSNumber])?.mapValues(\.int64Value) ?? [:]
        removed = d.data(forKey: Self.removedKey).flatMap { try? JSONDecoder().decode([String: [Resources.Removed]].self, from: $0) } ?? [:]
        collapsed = Set(d.stringArray(forKey: Self.collapsedKey) ?? [])
    }

    /// The agent's list key: its session's name and its session id (the pane's id without one).
    static func key(_ pane: Pane, session: String) -> String { "\(session)|\(pane.agent?.sessionId ?? pane.id)" }

    /// The agent's machine when it is not this Mac: an hc pane's machine, or a remote session's host.
    static func machine(_ pane: Pane, remote: RemoteHost?) -> String? {
        if let a = HCMapper.address(fromPaneId: pane.id) {
            guard let i = a.lastIndex(of: "/") else { return nil }
            return String(a[..<i])
        }
        if let r = pane.composerRemote(remote), case .ssh(let d) = r.via { return d }
        return nil
    }

    func items(_ key: String) -> [Resources.Item] { Resources.visible(lists[key]?.items ?? [], removed: removed[key] ?? []) }
    func hidden(_ key: String) -> Int { (lists[key]?.items.count ?? 0) - items(key).count }
    func badge(_ key: String?) -> Int {
        guard let key, let l = lists[key] else { return 0 }
        return Resources.badgeCount(l.items, seen: seen[key], removed: removed[key] ?? [])
    }

    /// Reads the agent's log when it changed (`force`: also when not, so docs are looked at again). Off the main
    /// actor; one read an agent at a time; only what the log grew by is parsed. `state` is the store's word on the
    /// log while it has no URL: still being found (the panel keeps saying it is reading), or not there.
    ///
    /// The first read is newest first (`split`: a local Claude log; else at once): the last `chunk` bytes, then the
    /// parts before them, the list growing after each. A read cancelled before the start is reached goes on next time.
    func refresh(key: String, log: URL?, state: ConversationLoadState = .loading, home: String?, agentDir: String?,
                 split: Bool = true, force: Bool = false) async {
        guard let log else {
            switch state {
            case .loading: problem[key] = nil
            case .failed: problem[key] = .unreadable
            // Ready with no file to read (a preview) is as good as none.
            case .missing, .ready: problem[key] = .noLog
            }
            return
        }
        guard !reading.contains(key) else { return }
        let a = try? FileManager.default.attributesOfItem(atPath: log.path)
        let stamp = Stamp(size: (a?[.size] as? NSNumber)?.intValue ?? -1, date: a?[.modificationDate] as? Date ?? .distantPast)
        let was = progs[key]
        let whole = was?.done == true
        if !force, whole, let l = lists[key], l.stamp == stamp { touch(key); return }
        // A log that takes long to read waits 10× that before a background read (a live agent's log changes all the time).
        if !force, whole, let l = lastRead[key], Date().timeIntervalSince(l.end) < 10 * l.took { return }
        reading.insert(key)
        defer { reading.remove(key) }
        // The first read is waited on (the panel says so): not behind background work. Later reads are background.
        let first = !whole
        let size = chunk
        let t0 = Date()
        let (p0, got) = await Task.detached(priority: first ? .userInitiated : .utility) { () -> (Resources.Progressive, [Resources.Item]?) in
            var p = was ?? Resources.Progressive(split: split, chunk: size)
            guard p.update(log) else { return (p, nil) }
            return (p, p.items(home: home, agent: agentDir))
        }.value
        lastRead[key] = (Date(), Date().timeIntervalSince(t0))
        guard let got else {
            if lists[key] == nil { problem[key] = .unreadable }
            return
        }
        var p = p0
        progs[key] = p
        problem[key] = nil
        progress[key] = p.done ? nil : p.fraction
        put(key, Entry(items: got, stamp: stamp), whole: p.done)
        // The older parts, newest first. The list (slow to build for a long log) is put at most once a second, and at the end.
        var shown = Date()
        while !p.done, !Task.isCancelled {
            let q = p, since = shown
            let (next, ok, list) = await Task.detached(priority: .userInitiated) { () -> (Resources.Progressive, Bool, [Resources.Item]?) in
                var q = q
                guard q.older(log, home: home) else { return (q, false, nil) }
                return (q, true, q.done || Date().timeIntervalSince(since) >= 1 ? q.items(home: home, agent: agentDir) : nil)
            }.value
            // Gone from memory meanwhile (eight other agents read), or the log went: stop (a later refresh goes on).
            guard progs[key] != nil, ok else { return }
            p = next
            progs[key] = p
            progress[key] = p.done ? nil : p.fraction
            if let list {
                put(key, Entry(items: list, stamp: stamp), whole: p.done)
                shown = Date()
            }
        }
    }

    /// The agent's list is not all there yet: the panel shows how much of the log is read.
    func readingEarlier(_ key: String) -> Bool { progress[key] != nil }
    /// Nothing found yet and the log not all read: the panel says it is reading ("No resources yet" only after the whole log).
    func stillReading(_ key: String) -> Bool { loading(key) || (readingEarlier(key) && items(key).isEmpty) }

    /// The agent's first read has not ended (and nothing says it cannot be read): the panel says it is reading.
    func loading(_ key: String) -> Bool { lists[key] == nil && problem[key] == nil }
    /// Keeps a list (checks put theirs).
    /// `whole`: the whole log is read (else the newest part so far).
    func put(_ key: String, _ e: Entry, whole: Bool = true) {
        if lists[key]?.items != e.items || lists[key] == nil { lists[key] = e } else { lists[key]?.stamp = e.stamp }
        touch(key)
        // The first look at an agent: what is there is seen (an old conversation does not light the badge). The newest
        // part has the newest items; when it has none, wait for the whole log, so older parts never count as new.
        if seen[key] == nil, whole || !e.items.isEmpty { setSeen(key, Resources.newest(e.items)) }
        // Only the list the panel shows: a read of another agent that ends while the panel is open stays unseen.
        if open, key == shownKey { markSeen(key) }
    }

    private func touch(_ key: String) {
        order.removeAll { $0 == key }; order.append(key)
        while order.count > Resources.cacheMax {
            let k = order.removeFirst(); lists[k] = nil; progs[k] = nil; progress[k] = nil; lastRead[k] = nil; problem[k] = nil
        }
    }

    /// Shown is seen: the badge goes while the panel shows the list.
    func markSeen(_ key: String) {
        let n = Resources.newest(lists[key]?.items ?? [])
        if (seen[key] ?? 0) < n { setSeen(key, n) }
    }

    private func setSeen(_ key: String, _ v: Int64) {
        seen[key] = v
        AppDefaults.store.set(seen.mapValues { NSNumber(value: $0) }, forKey: Self.seenKey)
    }

    func toggle(key: String?) {
        open.toggle()
        expanded = []; note = nil
        seenAtOpen = key.flatMap { seen[$0] }
        shownKey = key
        if open, let key { markSeen(key) }
    }

    /// The panel shows another agent: its dots from what was seen of it.
    func shows(_ key: String?) {
        guard key != shownKey else { return }
        shownKey = key
        seenAtOpen = key.flatMap { seen[$0] }; expanded = []; note = nil
    }
    private var shownKey: String?

    func remove(_ item: Resources.Item, key: String) {
        removed[key] = Resources.removing(item, from: removed[key] ?? [])
        if let d = try? JSONEncoder().encode(removed) { AppDefaults.store.set(d, forKey: Self.removedKey) }
    }

    /// Folds the group, or unfolds it (for every agent; kept over a restart).
    func toggleGroup(_ id: String) {
        if collapsed.contains(id) { collapsed.remove(id) } else { collapsed.insert(id) }
        AppDefaults.store.set(collapsed.sorted(), forKey: Self.collapsedKey)
    }

    func click(_ item: Resources.Item, machine: String?) {
        let r = perform(Resources.openPlan(item, machine: machine))
        note = r.map { (item.id, $0.text, $0.ok) }
    }

    /// The app's own way: a command to the pasteboard (never run), an address to the browser, a doc file (looked at
    /// again now: links followed, the real file a kind that opens) to its default app.
    static func run(_ plan: Resources.Plan) -> (text: String, ok: Bool)? {
        switch plan {
        case .copy(let s):
            NSPasteboard.general.clearContents(); NSPasteboard.general.setString(s, forType: .string)
            return ("Copied.", true)
        case .url(let u):
            return NSWorkspace.shared.open(u) ? nil : ("Could not open the address.", false)
        case .file(let p):
            guard let real = Resources.openable(p) else { return ("Flockbook does not open this file.", false) }
            return NSWorkspace.shared.open(URL(fileURLWithPath: real)) ? nil : ("Could not open the file.", false)
        case .why(let s):
            return (s, false)
        }
    }

    /// The row's Copy: its address or path, through `perform` (checks put their own).
    func copy(_ item: Resources.Item) {
        let r = perform(.copy(item.target))
        note = r.map { (item.id, $0.text, $0.ok) }
    }
}

extension AppModel {
    /// The agent the Resources panel is about: the tab's focused pane, or its first agent pane.
    var resourcesPane: Pane? {
        guard screen == .tab, let tab else { return nil }
        let id = focusedPaneID(in: tab)
        if let p = tab.panes.first(where: { $0.id == id }), p.agent != nil { return p }
        return tab.panes.first { $0.agent != nil }
    }

    var resourcesKey: String? { resourcesPane.map { ResourcesModel.key($0, session: store.session.name ?? "") } }

    /// ⌘⇧R and the toolbar's button.
    func toggleResources() {
        if screen != .tab { screen = .tab; if resources.open { return } }
        resources.toggle(key: resourcesKey)
        if resources.open, let p = resourcesPane { Task { await refreshResources(p, force: true) } }
    }

    func refreshResources(_ pane: Pane, force: Bool = false) async {
        let machine = ResourcesModel.machine(pane, remote: store.session.remote)
        let sid = pane.agent?.sessionId
        let log = store.logURL(for: sid)
        await resources.refresh(key: ResourcesModel.key(pane, session: store.session.name ?? ""), log: log,
                                state: log == nil ? store.conversationState(for: sid) : .ready,
                                home: machine == nil ? NSHomeDirectory() : nil, agentDir: machine == nil ? pane.cwd : nil,
                                // Newest first: a local Claude log (where a chunk may end is a Claude prompt line).
                                split: machine == nil && pane.agent?.kind == "claude", force: force)
    }

    /// Show in chat: the tab's pane scrolls to the row.
    func showResourceInChat(_ item: Resources.Item, pane: Pane) {
        screen = .tab
        resources.jump = .init(pane: pane.id, row: item.row, token: (resources.jump?.token ?? 0) + 1)
    }
}

/// Where the panel goes in a tab of `size` whose reply box starts `inputTop` points under the tab's top: right, 12 pt
/// under the toolbar, ending 12 pt over the reply box; one height for any list. A narrow window: a drawer down the
/// right side.
struct ResourcesPlacement: Equatable {
    var width: CGFloat, height: CGFloat, top: CGFloat, trailing: CGFloat, drawer: Bool

    static func of(size: CGSize, inputTop: CGFloat) -> ResourcesPlacement {
        let g = ResourcesModel.gap, w = ResourcesModel.width
        let bottom = inputTop > 0 && inputTop <= size.height ? inputTop : size.height - ResourcesModel.inputGuess
        if size.width < ResourcesModel.narrow { return .init(width: min(w, size.width), height: max(80, bottom - g), top: 0, trailing: 0, drawer: true) }
        return .init(width: w, height: max(80, bottom - 2 * g), top: g, trailing: g, drawer: false)
    }
}

/// The panel over the tab: TabScreen puts it in an overlay, so the chat under it keeps its place and size.
struct ResourcesOverlay: View {
    @Environment(AppModel.self) private var model
    @State private var frame = CGRect.zero

    var body: some View {
        let r = model.resources
        Color.clear
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { frame = $0 }
            .overlay(alignment: .topTrailing) {
                if r.open {
                    let p = ResourcesPlacement.of(size: frame.size, inputTop: r.inputTop > 0 ? r.inputTop - frame.minY : 0)
                    ResourcesPanel(pane: model.resourcesPane, drawer: p.drawer)
                        .frame(width: p.width, height: p.height)
                        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { if ResourcesProbe.on { ResourcesProbe.panel = $0 } }
                        .padding(.top, p.top).padding(.trailing, p.trailing)
                        .transition(.opacity)
                }
            }
            .task(id: model.resourcesKey) {
                // The shown agent's list: at once from memory, read again when its log changes.
                model.resources.shows(model.resourcesKey)
                guard let pane = model.resourcesPane else { return }
                await model.refreshResources(pane, force: model.resources.open)
                while !Task.isCancelled {
                    // No list yet: the log may be found any moment (the store does not tell), so look again soon.
                    let none = model.resourcesKey.map(model.resources.loading) ?? false
                    try? await Task.sleep(for: none ? .milliseconds(500) : .seconds(5))
                    guard let p = model.resourcesPane, ResourcesModel.key(p, session: model.store.session.name ?? "") == model.resourcesKey else { break }
                    await model.refreshResources(p)
                }
            }
    }
}

/// The checks' view of the panel.
@MainActor enum ResourcesProbe {
    static var on = false
    static var panel = CGRect.zero
}

struct ResourcesPanel: View {
    @Environment(AppModel.self) private var model
    let pane: Pane?
    var drawer = false
    var now = Date()

    var body: some View {
        let r = model.resources
        let key = pane.map { ResourcesModel.key($0, session: model.store.session.name ?? "") }
        let items = key.map(r.items) ?? []
        let hidden = key.map(r.hidden) ?? 0
        let earlier = key.map(r.readingEarlier) == true
        let loading = pane != nil && key.map(r.stillReading) == true
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Text("Resources").font(.headline)
                Spacer(minLength: 0)
                if !loading {
                    Text("\(items.count) item\(items.count == 1 ? "" : "s")\(hidden > 0 ? " · \(hidden) removed" : "")")
                        .font(.subheadline).foregroundStyle(.secondary).monospacedDigit()
                        .lineLimit(1).minimumScaleFactor(0.8)
                }
                ResourceIconButton(name: "close", help: "Close (⌘⇧R)") { model.toggleResources() }
                    .accessibilityLabel("Close Resources")
            }
            .padding(.leading, 18).padding(.trailing, 12).frame(height: 44)
            if loading {
                // The first read of this agent's log (later reads keep the list in place).
                VStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Reading this chat…").font(.subheadline).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .accessibilityElement(children: .combine)
                if earlier, let key { LoadingEarlier(fraction: r.progress[key] ?? 0) }
            } else if let key, !items.isEmpty, let pane {
                list(items, key: key, pane: pane)
                if earlier { LoadingEarlier(fraction: r.progress[key] ?? 0) }
            } else {
                let t = key.flatMap { r.problem[$0] }.map(Resources.problemText) ?? Resources.emptyText(agent: pane != nil)
                VStack(spacing: 6) {
                    Text(t.0).font(.headline)
                    Text(t.1).font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center)
                }
                .padding(.horizontal, 24).frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .modifier(PanelSurface(drawer: drawer))
        .onChange(of: items.map(\.id)) { _, _ in if let key { r.markSeen(key) } }
        .onAppear { if let key { r.markSeen(key) } }
    }

    private func list(_ items: [Resources.Item], key: String, pane: Pane) -> some View {
        let r = model.resources
        let machine = ResourcesModel.machine(pane, remote: model.store.session.remote)
        let ms = Int64(now.timeIntervalSince1970 * 1000)
        return ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(Resources.layoutGroups(items, collapsed: r.collapsed, expanded: r.expanded), id: \.group.id) { g in
                    ResourceGroupTitle(group: g) { withAnimation(.easeOut(duration: 0.15)) { r.toggleGroup(g.group.id) } }
                    ForEach(g.shown) { it in
                        ResourceRow(item: it, icon: ResourceRow.icon(it), fresh: Resources.isNew(it, seen: r.seenAtOpen),
                                    gone: it.kind == "doc" && it.exists != true && machine == nil, ago: Resources.ago(it.lastTs, now: ms),
                                    note: r.note?.id == it.id ? r.note.map { ($0.text, $0.ok) } : nil,
                                    onOpen: { r.click(it, machine: machine) },
                                    onJump: { model.showResourceInChat(it, pane: pane) },
                                    onCopy: { r.copy(it) },
                                    onRemove: { withAnimation(.easeOut(duration: 0.15)) { r.remove(it, key: key) } })
                    }
                    if g.rest > 0 {
                        Button { r.expanded.insert(g.group.id) } label: {
                            Text("Show \(g.rest) more").font(.subheadline).foregroundStyle(.secondary)
                                .padding(.horizontal, 10).padding(.vertical, 6).frame(maxWidth: .infinity, alignment: .leading).contentShape(Rectangle())
                        }
                        .buttonStyle(.plain).padding(.horizontal, 4)
                    }
                }
            }
            .padding(.vertical, 4)
        }
        .scrollIndicators(.automatic)
    }
}

/// Under the list while the log's older parts are read: a small spinner and how much is read (rounded down).
struct LoadingEarlier: View {
    let fraction: Double
    static func text(_ f: Double) -> String { "Loading earlier… \(Int((max(0, min(1, f)) * 100).rounded(.down)))%" }
    var body: some View {
        HStack(spacing: 6) {
            ProgressView().controlSize(.mini)
            Text(Self.text(fraction)).font(.subheadline).foregroundStyle(.secondary).monospacedDigit()
        }
        .frame(maxWidth: .infinity).frame(height: 30)
        .overlay(alignment: .top) { Divider().opacity(0.5) }
        .accessibilityElement(children: .combine)
    }
}

/// The sidebar's surface (as its hover panel, PeekGlass): the window surface in the sidebar's rounded shape, lifted off the
/// chat by a hairline and a shadow. One look with the sidebar; flat in snapshots (GlassReader). A drawer has no corners.
private struct PanelSurface: ViewModifier {
    let drawer: Bool
    /// The sidebar panel's corners (macOS 26); macOS 15 has a square sidebar, so a floating panel takes the window's.
    static var radius: CGFloat { SystemSidebar.cornerRadius > 0 ? SystemSidebar.cornerRadius : 10 }
    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: drawer ? 0 : Self.radius)
        content
            .background { GlassReader { WindowSurface(glass: $0) } }
            .clipShape(shape)
            .overlay(shape.strokeBorder(Color.primary.opacity(0.12), lineWidth: 0.5))
            .shadow(color: .black.opacity(0.25), radius: 18, y: 6)
    }
}

struct ResourceIcon: View {
    let name: String
    var size: CGFloat = 13
    var body: some View {
        if let i = ResourceIcons.image(name) {
            Image(nsImage: i).resizable().interpolation(.high).frame(width: size, height: size)
        } else {
            Color.clear.frame(width: size, height: size)
        }
    }
}

struct ResourceIconButton: View {
    let name: String
    let help: String
    let action: () -> Void
    @State private var hover = false
    var body: some View {
        Button(action: action) {
            ResourceIcon(name: name, size: 13)
                .frame(width: 24, height: 24)
                .foregroundStyle(hover ? .primary : .secondary)
                .background(hover ? Color.primary.opacity(0.08) : .clear, in: RoundedRectangle(cornerRadius: 6))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .help(help)
        .accessibilityLabel(help)
    }
}

/// A group's title: a click folds it.
struct ResourceGroupTitle: View {
    let group: Resources.Laid
    let onToggle: () -> Void
    @State private var hover = false
    var body: some View {
        Button(action: onToggle) {
            HStack(spacing: 4) {
                Text(group.group.label).frame(maxWidth: .infinity, alignment: .leading)
                Text("\(group.items.count)").monospacedDigit().font(.subheadline)
                ResourceIcon(name: group.collapsed ? "chevron-right" : "chevron-down", size: 11)
            }
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(hover ? .primary : .secondary)
            .padding(.horizontal, 10).padding(.top, 10).padding(.bottom, 2)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 4)
        .onHover { hover = $0 }
        .accessibilityLabel("\(group.group.label), \(group.items.count)")
        .accessibilityValue(group.collapsed ? "Folded" : "Shown")
    }
}

struct ResourceRow: View {
    let item: Resources.Item
    let icon: String
    let fresh: Bool
    let gone: Bool
    let ago: String
    let note: (String, Bool)?
    let onOpen: () -> Void
    let onJump: () -> Void
    let onCopy: () -> Void
    let onRemove: () -> Void
    /// Checks: the actions show without the pointer.
    var showActions = false
    @State private var hover = false

    /// The row's icon, as the mod's iconOf: a doc its group's, else its kind's.
    static func icon(_ it: Resources.Item) -> String {
        if it.kind == "doc" {
            switch Resources.groupOf(kind: it.kind, target: it.target) {
            case "web": return "globe"
            case "pdfs": return "file-text"
            case "images": return "image"
            default: return "doc"
            }
        }
        return ["artifact": "grid", "pr": "branch", "issue": "branch", "ci": "refresh", "local": "terminal", "link": "link", "cmd": "terminal"][it.kind] ?? "link"
    }

    var body: some View {
        let mono = item.kind == "doc" || item.kind == "cmd"
        let sub = item.detail.isEmpty ? item.target : item.detail
        HStack(alignment: .top, spacing: 8) {
            ResourceIcon(name: icon, size: 14)
                .foregroundStyle(.secondary)
                .frame(width: SidebarLine.box, height: SidebarLine.box)
                .opacity(gone ? 0.5 : 1)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 6) {
                    Text(item.title.isEmpty ? item.target : item.title)
                        .font(.body)
                        .foregroundStyle(gone ? .tertiary : .primary)
                        .strikethrough(gone)
                        .lineLimit(1).truncationMode(.tail)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if !(hover || showActions) {
                        Text(ago).font(.subheadline).foregroundStyle(.tertiary).monospacedDigit()
                    }
                }
                Text(gone ? "\(sub) · File not found" : sub)
                    .font(mono ? .subheadline.monospaced() : .subheadline)
                    .foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
                if let note {
                    Text(note.0).font(.subheadline).foregroundStyle(note.1 ? Color.secondary : Color.orange)
                }
            }
            if hover || showActions {
                HStack(spacing: 2) {
                    ResourceIconButton(name: "message", help: "Show in chat", action: onJump)
                    // A command's click copies it already.
                    if item.kind != "cmd" { ResourceIconButton(name: "copy", help: "Copy", action: onCopy) }
                    ResourceIconButton(name: "eye-off", help: "Remove from list", action: onRemove)
                }
            }
        }
        .padding(.horizontal, 10).padding(.vertical, 6)
        .background(hover ? Color.primary.opacity(0.06) : .clear, in: RoundedRectangle(cornerRadius: 6))
        .overlay(alignment: .topLeading) {
            if fresh { Circle().fill(Color.accentColor).frame(width: 6, height: 6).offset(x: 2, y: 14).accessibilityHidden(true) }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: onOpen)
        .onHover { hover = $0 }
        .padding(.horizontal, 4)
        .help(item.target)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel("\(item.title.isEmpty ? item.target : item.title)\(fresh ? ", new" : "")\(gone ? ", file not found" : "")")
        .accessibilityAction(named: "Show in chat", onJump)
        .accessibilityAction(named: "Remove from list", onRemove)
    }
}

/// The toolbar's button: on while the panel is open; a badge with the count of newer ones.
struct ResourcesButtonLabel: View {
    let count: Int
    var body: some View {
        ResourceIcon(name: "link", size: 15)
            .overlay(alignment: .topTrailing) {
                if count > 0 {
                    Text(count > 99 ? "99+" : "\(count)")
                        .font(.system(size: 9, weight: .bold)).monospacedDigit()
                        .foregroundStyle(.white)
                        .padding(.horizontal, 3).frame(minWidth: 14, minHeight: 14)
                        .background(Color.accentColor, in: Capsule())
                        .offset(x: 9, y: -7)
                        .accessibilityHidden(true)
                }
            }
    }
}
