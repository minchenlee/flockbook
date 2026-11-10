import HerdrKit
import SwiftUI

struct ConversationView: View {
    let items: [ConversationItem]
    let agentName: String
    /// The pending call whose card gets Allow / Deny (`ApprovalNotice.CardPlace.conversation`). Any other pending call
    /// shows a "Running" chip: its id is not the prompt on screen.
    var liveCard: String? = nil
    /// The card's buttons, with the request the card shows (its prompt id binds the keys).
    var onAllow: (PermissionRequest) -> Void = { _ in }
    var onDeny: (PermissionRequest) -> Void = { _ in }
    var onOpenTerminal: () -> Void = {}
    /// The agent's state: picks the empty-state copy (a fresh agent has no log until its first message).
    var agentStatus: AgentStatus? = nil
    /// The fresh-agent empty state appeared: the pane puts the cursor in the composer.
    var onFresh: () -> Void = {}
    /// Load state of the agent's session log (from the store). nil: older callers; `hasLog` then means "may still load".
    var loadState: ConversationLoadState? = nil
    var hasLog = false
    /// An agent team took the pane's log (`Agent.team`): no lead log yet reads "Waiting for the lead's log".
    var team: TeamNote? = nil
    /// The command that makes a host agent's conversation readable (`SessionResolver.hostSetup`); nil when none is needed.
    var hostSetup: String? = nil
    /// Changes when the user sends a message: back to the bottom whatever the position, following from there
    /// (the log shows the sent message a moment later).
    var scrollToken = 0
    /// The session the rows come from. Another session in the pane starts over: it may have older rows to read.
    var session: String? = nil
    /// Reads an older part of the log, distinguishing the start from a failed read. nil hides the button.
    var onLoadEarlier: (() async -> EarlierLoadResult)? = nil
    @State private var moreEarlier = true
    @State private var loadingEarlier = false
    @State private var earlierFailed = false
    /// Counts the sessions the view showed: a read of an older session's log that ends later changes nothing.
    @State private var earlierSession = 0
    @State private var spinnerDue = false
    /// The "Loading earlier" notice over the top of the rows: the number of the load it shows for, nil when none.
    @State private var notice: Int?
    /// Rows shown at first; "Show earlier" adds this many again. A check may raise it (a long session read far back).
    static var page = ConversationWindow.page
    /// The rows shown: at most `ConversationWindow.maxRows`, the newest unless "Show earlier" went past that.
    @State private var window = ConversationWindow(shown: ConversationView.page)
    /// Which view set the probe's actions (a replaced view disappears after its successor appeared).
    @State private var probeId = UUID()
    @AppStorage(ReadingSize.key, store: AppDefaults.store) private var sizeRaw = ReadingSize.m.rawValue
    @AppStorage(ReadingWidth.key, store: AppDefaults.store) private var widthRaw = ReadingWidth.medium.rawValue   // re-renders when the width changes
    private var size: ReadingSize { ReadingSize(rawValue: sizeRaw) ?? .m }

    var body: some View {
        // One view over both branches: a session switch while the empty view shows still resets the read state (a watch
        // on the list would start with the list, after the switch, and never see it).
        ZStack {
            if let state = Self.emptyState(items: items, loadState: loadState, hasLog: hasLog) {
                empty(state)
            } else {
                list
            }
        }
        .onChange(of: session) { _, _ in earlierSession += 1; moreEarlier = true; loadingEarlier = false; earlierFailed = false; notice = nil; tracker.adding = false }
    }

    /// Which empty view to show, or nil for the list.
    static func emptyState(items: [ConversationItem], loadState: ConversationLoadState?, hasLog: Bool) -> ConversationLoadState? {
        items.isEmpty ? loadState ?? (hasLog ? .loading : nil) : nil
    }

    struct EmptyCopy: Equatable { var title: String; var detail: String; var terminal = false; var fresh = false }

    /// Words for an empty conversation. A new agent has no session log until its first message: that is not an error, so it
    /// reads "No messages yet" with what to do. "No conversation log" + Show Terminal only when the agent has worked
    /// (working, blocked, done) and its log still cannot be found. An agent team with no lead log known waits for it.
    static func emptyCopy(_ state: ConversationLoadState, status: AgentStatus?, agentName: String, team: TeamNote? = nil,
                          hostSetup: String? = nil) -> EmptyCopy {
        let first = "Send your first message to see the session conversation."
        if team == .waitingForLead { return EmptyCopy(title: TeamNote.waitingTitle, detail: TeamNote.waitingDetail, terminal: true) }
        if let hostSetup, state == .missing {
            return EmptyCopy(title: "Conversation not available", detail: "herdr on the host does not say which session \(agentName) uses. Run \"\(hostSetup)\" on the host, then restart \(agentName).", terminal: true)
        }
        switch (state, status) {
        case (.missing, .idle?), (.missing, nil):
            return EmptyCopy(title: "No messages yet", detail: first, fresh: true)
        case (.missing, _):
            return EmptyCopy(title: "No conversation log", detail: "\(agentName)'s session log was not found. The terminal shows what it is doing.", terminal: true)
        case (_, .idle?), (_, nil):
            return EmptyCopy(title: "No messages yet", detail: first, fresh: true)
        case (_, .working?):
            return EmptyCopy(title: "No messages yet", detail: "\(agentName) is starting. Its messages appear here.")
        default:
            return EmptyCopy(title: "No messages yet", detail: "\(agentName) has not written anything yet.")
        }
    }

    /// No rows yet: say why. A spinner only once loading has taken longer than half a second.
    @ViewBuilder private func empty(_ state: ConversationLoadState) -> some View {
        switch state {
        case .loading:
            VStack(spacing: 10) {
                if spinnerDue {
                    Spinner()
                    Text("Loading conversation…").font(.callout).foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .task { try? await Task.sleep(for: .milliseconds(500)); spinnerDue = true }
        case .ready, .missing:
            let copy = Self.emptyCopy(state, status: agentStatus, agentName: agentName, team: team, hostSetup: hostSetup)
            if copy.terminal {
                ContentUnavailableView {
                    Label(copy.title, systemImage: "doc.text.magnifyingglass")
                } description: {
                    Text(copy.detail).textSelection(.enabled)   // the host command can be copied
                } actions: {
                    Button("Show Terminal", action: onOpenTerminal)
                }
            } else {
                ContentUnavailableView(copy.title, systemImage: "bubble.left.and.bubble.right", description: Text(copy.detail))
                    .onAppear { if copy.fresh { onFresh() } }
            }
        case .failed(let why):
            ContentUnavailableView {
                Label("Can't read the conversation", systemImage: "exclamationmark.triangle")
            } description: {
                Text(why)
            } actions: {
                Button("Show Terminal", action: onOpenTerminal)
            }
        }
    }

    /// Whether the reader sits at the bottom. A plain object, not @State: written from the scroll callback,
    /// so it never triggers a render (writing @State there caused layout feedback loops before).
    @MainActor final class BottomTracker {
        var atBottom = true
        /// The scroll offset, and each row's top in the rows' own space (both as last seen).
        var offset: CGFloat = 0
        var tops: [String: CGFloat] = [:]
        /// While older rows come in above: the row that was on top and where it was, so the view keeps it in place.
        var anchor: (key: String, top: CGFloat, offset: CGFloat, until: Date)?
        /// The top inset as last seen (scrollTo(y:) takes a content point, under it), and a fix of the anchor due next turn.
        var inset: CGFloat = 0
        /// The bottom of the view, in the rows' space, as last seen.
        var viewBottom: CGFloat = 0
        var fixDue = false
        /// The scroll view, as the anchor row's probe last saw it: the fallback fix moves its clip view the same way.
        weak var scrollView: NSScrollView?
        /// The anchor row's probe and its row: where the row is now, read from the view itself. SwiftUI can place the row
        /// (move a parent of it) before the row's top is reported, and without a frame change of the probe's own.
        weak var probe: NSView?
        var probeKey: String?
        /// The page the read of the log in flight asked for: half a page when it loads by itself.
        var loadPage = ConversationView.page
        /// The read of the log in flight is an early one: the reader's scroll up started it, away from the top.
        var loadEarly = false
        /// Older rows are going into the window, a few per frame.
        var adding = false
        /// The reader is scrolling: a gesture, its momentum, the bounce at an end.
        var scrolling = false
        /// Counts the pages of older rows put in. Jump to latest starts a new count: the rest of a page on its way stays out.
        var batches = 0
        /// Loads counted (the notice's number) and when the notice showed.
        var loads = 0
        var noticeSince = Date.distantPast
        /// How fast the reader scrolls up (pt/s; 0 going down) and when the offset was last seen.
        var speed: CGFloat = 0
        var seenAt = Date.distantPast
        /// How long older rows take, from the call to the last of them in: a read of the log (85-300 ms on a 30 MB log,
        /// longer as more of it is loaded) and the batches. Measured on each load; this is the start.
        var loadTime: TimeInterval = 0.5
        var loadSince: Date?

        /// Called while the frame is drawn, when the anchor row was placed: the view moves by as much as the row did, in
        /// the frame that shows the new rows. False when the clip view cannot move that far yet. `partly`: the fallback a frame
        /// later moves as far as the clip view goes (the end of content that does not fill the view).
        @discardableResult func fixNow(_ sv: NSScrollView, partly: Bool = false) -> Bool {
            guard !atBottom, let a = anchor, a.until > Date(), let top = top(a.key, in: sv), abs(top - a.top) > 0.5 else { return true }
            // From where the view is now: a glide may have moved it since the offset was last reported.
            let clip = sv.contentView, was = clip.bounds.origin, offset = was.y + top - a.top
            clip.scroll(to: NSPoint(x: was.x, y: offset))
            guard partly || abs(clip.bounds.minY - offset) < 0.5 else { clip.scroll(to: was); return false }   // clamped: no half move
            sv.reflectScrolledClipView(clip)
            let got = clip.bounds.minY
            anchor = (a.key, top, got, a.until); self.offset = got
            return true
        }
        /// The row's top in the rows' space: from the probe's place in the document when the probe is on that row (the
        /// rows sit `listPadding` below the document's top), else as last reported.
        func top(_ key: String, in sv: NSScrollView) -> CGFloat? {
            if probeKey == key, let p = probe, p.window != nil, let doc = sv.documentView, p.isDescendant(of: doc) {
                return p.convert(p.bounds, to: doc).minY - ConversationView.listPadding
            }
            return tops[key]
        }
        let merged = MergedRows()
    }
    /// Closer than this to the top, the reader's scrolling loads older rows by itself (Show earlier stays as the button).
    /// Half a page at a time: the stack lays out every new row in one pass, and 100 at once came near the 250 ms stall limit.
    static let nearTop: CGFloat = 400
    /// A fast scroll up starts the load farther away: as far as the view goes, at its speed, in the time a load takes (with
    /// half again to spare), so the rows are in before the top.
    static func lead(speed: CGFloat, loadTime: TimeInterval) -> CGFloat { max(nearTop, speed * CGFloat(loadTime) * 1.5) }
    static var autoPage: Int { max(1, page / 2) }
    /// Rows that go into the window per frame. All at once, the stack lays out and draws every new row in one pass: a
    /// hitch the reader sees mid-scroll (about 100 ms for half a page).
    static let batch = 10
    /// Nearer the top than this while the reader scrolls (the bounce, a push into the top), older rows wait.
    static let edge: CGFloat = 1
    /// The notice stays at least this long (a flash reads as a glitch).
    static let noticeMin: TimeInterval = 0.5
    @State private var tracker = BottomTracker()
    @State private var position = ScrollPosition(edge: .bottom)

    private var list: some View {
        ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    let ids = items.map(\.id)
                    let range = window.range(ids)
                    if range.lowerBound > 0 || (onLoadEarlier != nil && moreEarlier) {
                        Button { showEarlier() } label: {
                            HStack(spacing: 6) {
                                if loadingEarlier { Spinner(size: .mini) }
                                Text(earlierFailed ? "Try again" : range.lowerBound > 0 ? "Show earlier (\(range.lowerBound))" : "Show earlier")
                            }
                        }
                        .buttonStyle(.plain).font(.caption).foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity).padding(.bottom, 20).help("Load older messages")
                        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { ConversationProbe.earlier = $0 }
                        .onAppear { ConversationProbe.set(.earlier, probeId) { showEarlier() } }
                        // The action reads the rows it was made with: new rows (a check that swaps them) make it anew.
                        .onChange(of: ConversationProbe.enabled ? ids : []) { ConversationProbe.set(.earlier, probeId) { showEarlier() } }
                        .onDisappear { ConversationProbe.clear(.earlier, probeId) }
                    }
                    if earlierFailed {
                        Text("Could not load earlier messages. Check the connection and try again.")
                            .font(.caption).foregroundStyle(.secondary).frame(maxWidth: .infinity)
                            .padding(.bottom, 12)
                            .onAppear { if ConversationProbe.enabled { ConversationProbe.earlierFailed = true } }
                            .onDisappear { if ConversationProbe.enabled { ConversationProbe.earlierFailed = false } }
                    }
                    let rows = Self.displayRows(items[range], merged: tracker.merged)
                    let lastReply = window.detached ? nil : rows.last { if case .assistantText = $0.kind { true } else { false } }?.id
                    let entries = Self.entries(rows)
                    // By key, with the gap above worked out here: a row's inputs stay the same when rows come in above it, so
                    // SwiftUI skips it (an index in the inputs re-rendered every row of the window on each page).
                    ForEach(Self.placed(entries), id: \.entry.key) { p in
                        EntryRow(entry: p.entry, foldable: p.entry.item?.id != lastReply, size: size, liveCard: liveCard, agentName: agentName,
                                 onAllow: onAllow, onDeny: onDeny, onOpenTerminal: onOpenTerminal)
                            .equatable()
                            // The row's own top, without the gap above it (the first row has none; it gets one when older rows come in).
                            // The anchor row moved: older rows came in above (also when as many newer rows left, same height).
                            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.rowSpace)).minY } action: {
                                tracker.tops[p.entry.key] = $0
                                if tracker.anchor?.key == p.entry.key { holdAnchor() }
                            }
                            .background { if tracker.anchor?.key == p.entry.key { AnchorProbe(tracker: tracker, key: p.entry.key) } }
                            .padding(.top, p.gap)
                    }
                    if window.detached {
                        Button { jumpToLatest() } label: {
                            Label("Jump to latest (\(ids.count - range.upperBound) newer)", systemImage: "arrow.down")
                        }
                        .buttonStyle(.bordered).controlSize(.small)
                        .frame(maxWidth: .infinity).padding(.top, 24).help("Show the newest messages")
                        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { ConversationProbe.latest = $0 }
                        .onAppear { ConversationProbe.set(.latest, probeId) { jumpToLatest() } }
                        .onDisappear { ConversationProbe.latest = .zero; ConversationProbe.clear(.latest, probeId) }
                    }
                }
                .coordinateSpace(.named(Self.rowSpace))
                .readingColumn(size, ReadingWidth(rawValue: widthRaw) ?? .medium, vertical: Self.listPadding)
            }
            .defaultScrollAnchor(.bottom)
            .scrollPosition($position)
            // Over the rows, not among them: it moves none of them. Its fade is its own: on the scroll view, the fade also
            // animated a scroll to hold the row on top that happened in the same update, and the rows came in mid-way.
            .overlay(alignment: .top) {
                ZStack {
                    if notice != nil {
                        EarlierNotice().padding(.top, 8).transition(.opacity)
                    }
                }
                .animation(.easeOut(duration: 0.15), value: notice != nil)
            }
            // Following: the reader is "at the bottom" only through their own scrolling (offset changes alone).
            // When the content or the viewport changes size (a new message, a tool result, markdown settling, a window or
            // composer resize, safe-area insets settling) a reader at the bottom stays there.
            // One rule for every growth, so no per-event scroll calls and no retry timers.
            .onScrollGeometryChange(for: ScrollMetrics.self) { g in
                ScrollMetrics(offset: g.contentOffset.y, content: g.contentSize.height, container: g.containerSize.height,
                              insets: g.contentInsets.top + g.contentInsets.bottom, top: g.contentInsets.top)
            } action: { old, new in
                tracker.inset = new.top
                tracker.viewBottom = new.offset + new.top + new.container - Self.listPadding
                if abs(new.content - old.content) > 0.5 || abs(new.container - old.container) > 0.5 || abs(new.insets - old.insets) > 0.5 {
                    if tracker.atBottom {
                        // Next turn of the run loop: scrolling inside the update pass makes SwiftUI report "multiple times per frame".
                        Task { @MainActor in position.scrollTo(edge: .bottom) }
                    } else {
                        holdAnchor()
                    }
                } else {
                    tracker.atBottom = new.atBottom
                    tracker.offset = new.offset
                    // Only the offset: rows that came in above but are not made up for yet (the anchor row's move) are still due, also
                    // when the view glides on before the fix.
                    tracker.anchor?.offset = new.offset
                    let now = Date(), dt = now.timeIntervalSince(tracker.seenAt)
                    tracker.seenAt = now
                    tracker.speed = new.offset < old.offset && dt > 0.002 && dt < 0.25 ? (old.offset - new.offset) / CGFloat(dt) : 0
                    // The reader scrolls up near the top (farther from it the faster): older rows load by themselves. Only a
                    // scroll of the reader's own counts for speed: a move of the view's own (to the bottom after a send, the
                    // offset kept in range as rows go) can look like a fast scroll up.
                    let ahead = tracker.scrolling || ConversationProbe.scrolling ? Self.lead(speed: tracker.speed, loadTime: tracker.loadTime) : Self.nearTop
                    if new.offset < old.offset, new.offset + new.top < ahead,
                       !tracker.adding, !loadingEarlier {
                        Task { @MainActor in showEarlier(page: Self.autoPage, early: true) }
                    }
                }
            }
            .onScrollPhaseChange { _, phase in tracker.scrolling = phase != .idle }
            .onChange(of: scrollToken) { _, _ in jumpToLatest() }
            .onChange(of: items.first?.id) { _, _ in
                // Older rows arrived in front after "Show earlier" read the log: the window takes a page of them.
                // A few at a time, like rows at hand (all at once is one long layout pass).
                guard window.pendingLoad != nil, !tracker.adding else { return }
                window.loadEnded(window.pendingLoad!)
                // A notice of its own: the read's own notice may be on its way out (nothing seemed to come), and the rows
                // may wait a while (the reader still scrolls).
                add(tracker.loadPage, notice: showNotice(), early: tracker.loadEarly)
            }
    }

    /// A page of older rows: from the rows at hand, else read from the log (the window takes them when they arrive). A
    /// notice shows over the top of the rows until they are in. `early`: the reader's scroll up asked for them.
    private func showEarlier(page: Int = Self.page, early: Bool = false) {
        let ids = items.map(\.id)
        guard !tracker.adding, !(early && earlierFailed) else { return }
        if window.range(ids).lowerBound > 0 { tracker.loadSince = Date(); add(page, notice: showNotice(), early: early); return }
        guard let load = onLoadEarlier, moreEarlier, !loadingEarlier else { return }
        loadingEarlier = true
        earlierFailed = false
        tracker.loadSince = Date()
        tracker.loadPage = page
        tracker.loadEarly = early
        keepTop(ids, for: 4)
        let token = window.beginLoad(), mine = earlierSession, shown = showNotice()
        Task {
            let result = await load()
            guard earlierSession == mine else { return }
            loadingEarlier = false
            moreEarlier = result != .end
            earlierFailed = result == .failed
            if earlierFailed {
                window.loadEnded(token)
                hideNotice(shown)
                return
            }
            // Nothing came in (the log had nothing older, or a read that failed): the notice goes.
            try? await Task.sleep(for: .milliseconds(300))
            if window.pendingLoad == token, !tracker.adding { hideNotice(shown) }
            try? await Task.sleep(for: .seconds(3)); window.loadEnded(token)   // nothing arrived (a newer read stays pending)
            if notice == shown, !tracker.adding { hideNotice(shown) }
        }
    }

    /// `count` rows at hand go into the window, `batch` per frame, the row on top staying in place; then the notice goes.
    /// The notice is drawn before the first rows go in.
    private func add(_ count: Int, notice shown: Int, early: Bool = false) {
        tracker.adding = true
        tracker.batches += 1
        let mine = earlierSession, batch = tracker.batches
        Task { @MainActor in
            defer { if earlierSession == mine, tracker.batches == batch { tracker.adding = false; hideNotice(shown) } }
            try? await Task.sleep(for: .milliseconds(16))
            var left = count, held = false, waited: TimeInterval = 0
            while left > 0 {
                // The reader still scrolls at the top (the bounce there, a trackpad's push into it): the rows wait. Put in
                // now, the bounce ends at the new top, and the message being read is out of view. Farther down they go in
                // while the view glides (a trackpad's momentum moves the view from where it is), so a fast fling up does
                // not meet the top first.
                // At the cap, a batch drops as many rows from the bottom. In a load the reader's scroll up started
                // early (a fast scroll up from near the bottom of a short window starts it at once), while any of them is
                // in view the rows wait for the glide to take them out; if the reader stops away from the top, the rest
                // stays at hand for the top. Show earlier, or a load at the top, puts them in all the same: rows that all
                // fit in view (tool rows merged into a few short ones) would else never go in.
                let since = Date()
                while (tracker.scrolling || ConversationProbe.scrolling)
                        && (tracker.offset + tracker.inset < Self.edge || early && dropsInView(items.map(\.id), page: min(Self.batch, left))) {
                    try? await Task.sleep(for: .milliseconds(16))
                    guard earlierSession == mine, tracker.batches == batch else { return }
                }
                guard earlierSession == mine, tracker.batches == batch else { return }
                let ids = items.map(\.id)
                guard window.range(ids).lowerBound > 0 else { return }
                if early, tracker.offset + tracker.inset >= Self.nearTop, dropsInView(ids, page: min(Self.batch, left)) { return }
                // One row stays in place for the whole page. A new one per batch could be taken before the last batch's
                // rows were laid out and its move made up for (a busy frame), and that move then stayed.
                waited += Date().timeIntervalSince(since)
                if held, tracker.anchor != nil { tracker.anchor?.until = Date().addingTimeInterval(1.5) } else { keepTop(ids); held = true }
                window.showEarlier(ids, page: min(Self.batch, left))
                left -= Self.batch
                // Two frames: the rows are laid out and drawn, and the anchor fixed, before the next ones go in.
                try? await Task.sleep(for: .milliseconds(32))
            }
            // The load's own time (not the wait at the top), for how far ahead the next one starts.
            if let t = tracker.loadSince {
                tracker.loadSince = nil
                tracker.loadTime = min(2, max(0.2, 0.7 * tracker.loadTime + 0.3 * (Date().timeIntervalSince(t) - waited)))
            }
        }
    }

    /// Whether a batch of `page` older rows would drop rows from the bottom of the window (at the cap) that are in view.
    private func dropsInView(_ ids: [String], page: Int) -> Bool {
        var next = window
        next.showEarlier(ids, page: page)
        let r = window.range(ids), end = next.range(ids).upperBound
        guard end < r.upperBound else { return false }
        let keys = Set(Self.entries(Self.displayRows(items[r], merged: tracker.merged)).map(\.key))
        // The first row that goes (with a time line above it), or the row it merges into (an older id's key).
        for i in stride(from: end, through: r.lowerBound, by: -1) {
            if let top = ["s-\(ids[i])", "r-\(ids[i])"].filter(keys.contains).compactMap({ tracker.tops[$0] }).min() {
                return top < tracker.viewBottom
            }
        }
        return false
    }

    private func showNotice() -> Int {
        tracker.loads += 1
        if notice == nil { tracker.noticeSince = Date() }
        notice = tracker.loads
        return tracker.loads
    }

    /// The notice of load `shown` goes, once it has been up `noticeMin` (a newer load keeps its own).
    private func hideNotice(_ shown: Int) {
        let wait = Self.noticeMin - Date().timeIntervalSince(tracker.noticeSince)
        Task { @MainActor in
            if wait > 0 { try? await Task.sleep(for: .seconds(wait)) }
            if notice == shown { notice = nil }
        }
    }

    /// The row now on top of the window stays where it is on screen while older rows come in above it (for `seconds`).
    private func keepTop(_ ids: [String], for seconds: Double = 1.5) {
        // No top for the row yet: no anchor, not the last one (it would be held again for these rows).
        guard let key = Self.anchorKey(Self.entries(Self.displayRows(items[window.range(ids)]))), let top = tracker.tops[key] else {
            tracker.anchor = nil; return
        }
        tracker.anchor = (key, top, tracker.offset, Date().addingTimeInterval(seconds))
    }

    /// The row to keep in place: the first that is not a tool row. A tool row on top merges with an older tool row that
    /// comes in above it and takes that row's id, so its key would be gone.
    static func anchorKey(_ entries: [Entry]) -> String? {
        (entries.first { if case .row(let i) = $0.kind, case .toolChips = i.kind { false } else if case .row = $0.kind { true } else { false } } ?? entries.first)?.key
    }

    /// Older rows came in above and moved the anchor row: the view moves by as much. Next turn of the run loop, after the
    /// update pass, when every row reported its new top (scrolling inside the pass makes SwiftUI report "multiple times per frame").
    private func holdAnchor() {
        // The anchor row's probe moves the view while the frame is drawn; this is the fallback a frame later (a clip view
        // that could not move that far yet). Moving here, before the rows are drawn, would show them in the wrong place.
        guard !tracker.fixDue else { return }
        tracker.fixDue = true
        Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(20))
            tracker.fixDue = false
            // From where the clip view is now, as the probe does. A scrollTo(y:) of the position lands later, at a place worked
            // out now: when the next batch's probe fix came first (a busy frame), it put the view back by that batch's rows.
            if let sv = tracker.scrollView {
                let was = sv.contentView.bounds.minY
                tracker.fixNow(sv, partly: true)
                if abs(sv.contentView.bounds.minY - was) > 0.5 {
                    ConversationProbe.lateFixes += 1
                    if ConversationProbe.enabled { print("churn-trace: fallback compensation \(sv.contentView.bounds.minY - was) pt, from \(was), document \(sv.documentView?.frame.height ?? 0)") }
                }
                return
            }
            guard !tracker.atBottom, let a = tracker.anchor, a.until > Date(), let top = tracker.tops[a.key], abs(top - a.top) > 0.5 else { return }
            let offset = a.offset + top - a.top
            position.scrollTo(y: offset + tracker.inset)
            tracker.anchor = (a.key, top, offset, a.until)
        }
    }

    /// The newest page, at the bottom, following again.
    private func jumpToLatest() {
        tracker.batches += 1; tracker.adding = false
        if let shown = notice { hideNotice(shown) }
        window.jumpToLatest(page: Self.page)
        tracker.atBottom = true
        tracker.anchor = nil
        Task { @MainActor in position.scrollTo(edge: .bottom) }
    }

    struct ScrollMetrics: Equatable {
        var offset: CGFloat; var content: CGFloat; var container: CGFloat; var insets: CGFloat; var top: CGFloat
        /// The newest row is in view (within 40 pt). The container is the area between the bars, and the offset starts
        /// above the top bar: at the end, offset + top + container = content. Leaving out `top` read a reader at the end
        /// as scrolled up whenever the top bar was over 40 pt (the toolbar and a pane label), and following stopped.
        var atBottom: Bool { offset + top + container >= content - 40 }
    }
    static let rowSpace = "conversation-rows"
    /// Above and below the rows, in the scroll view.
    static let listPadding: CGFloat = 24

    struct Entry {
        enum Kind { case row(ConversationItem), separator(String), footer(String) }
        let key: String
        let kind: Kind
        /// The last real row before this entry, for the usual row-to-row gap.
        let prevRow: ConversationItem?
        var item: ConversationItem? { if case .row(let i) = kind { i } else { nil } }
        func gap(after prev: Entry) -> CGFloat {
            switch (prev.kind, kind) {
            case (.separator, _), (_, .separator): return 24
            case (_, .footer): return 12
            case (.footer, .row(let i)): return prev.prevRow.map { ConversationView.gap($0, i) } ?? 14
            case (.row(let a), .row(let b)): return ConversationView.gap(a, b)
            default: return 14
            }
        }
    }

    /// Rows plus time separators (gap >= 15 min or a new day) and one footer per agent turn.
    static func entries(_ rows: [ConversationItem]) -> [Entry] {
        var out: [Entry] = []
        var prevTime: Date?, prevRow: ConversationItem?
        var turnStart: Date?, turnTools = 0, turnHasReply = false, turnEnd: Date?
        func closeTurn() {
            guard turnHasReply, let end = turnEnd, let last = prevRow else { return }
            var parts = [clock(end)]
            if let s = turnStart, end.timeIntervalSince(s) >= 1 { parts.append("Worked \(duration(end.timeIntervalSince(s)))") }
            if turnTools > 0 { parts.append("\(turnTools) tool\(turnTools == 1 ? "" : "s")") }
            out.append(Entry(key: "f-\(last.id)", kind: .footer(parts.joined(separator: " · ")), prevRow: last))
        }
        for it in rows {
            if case .userText = it.kind { closeTurn(); turnStart = it.timestamp; turnTools = 0; turnHasReply = false; turnEnd = nil }
            if let t = it.timestamp {
                if let p = prevTime, t.timeIntervalSince(p) >= 900 || !Calendar.current.isDate(t, inSameDayAs: p) {
                    out.append(Entry(key: "s-\(it.id)", kind: .separator(separatorText(t)), prevRow: prevRow))
                }
                prevTime = t; turnEnd = t
            }
            if case .assistantText = it.kind { turnHasReply = true }
            if case .toolChips(let c) = it.kind { turnTools += c.reduce(0) { $0 + max(1, $1.calls.count) } }
            out.append(Entry(key: "r-\(it.id)", kind: .row(it), prevRow: prevRow))
            prevRow = it
        }
        closeTurn()
        return out
    }

    /// Each entry with the gap above it (none above the first).
    struct Placed { let entry: Entry; let gap: CGFloat }
    static func placed(_ entries: [Entry]) -> [Placed] {
        entries.indices.map { i in Placed(entry: entries[i], gap: i == 0 ? 0 : entries[i].gap(after: entries[i - 1])) }
    }

    private static func clock(_ d: Date) -> String { d.formatted(.dateTime.hour(.twoDigits(amPM: .omitted)).minute(.twoDigits)) }
    private static func duration(_ s: TimeInterval) -> String {
        let n = Int(s.rounded())
        return n >= 3600 ? "\(n / 3600)h \(n % 3600 / 60)m" : n >= 60 ? "\(n / 60)m \(n % 60)s" : "\(n)s"
    }
    private static func separatorText(_ d: Date) -> String {
        let cal = Calendar.current
        if cal.isDateInToday(d) { return clock(d) }
        if cal.isDateInYesterday(d) { return "Yesterday \(clock(d))" }
        return d.formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated)) + " " + clock(d)
    }

    /// Adjacent tool-chip items form one chip row. With `merged`, a run of tool rows that is the same as last time gives
    /// the same chip row (the same arrays): comparing it with the row on screen is then cheap.
    static func displayRows(_ items: ArraySlice<ConversationItem>, merged: MergedRows? = nil) -> [ConversationItem] {
        var out: [ConversationItem] = []
        var run: [ConversationItem] = []
        func flush() {
            guard let first = run.first else { return }
            defer { run = [] }
            if run.count == 1 { out.append(first); return }
            if let m = merged?.rows[first.id], m.parts == run { out.append(m.row); merged?.next[first.id] = m; return }
            let chips: [ToolChip] = run.flatMap { (r: ConversationItem) -> [ToolChip] in if case .toolChips(let c) = r.kind { c } else { [] } }
            let row = ConversationItem(id: first.id, kind: .toolChips(chips), timestamp: first.timestamp)
            merged?.next[first.id] = (run, row)
            out.append(row)
        }
        for it in items {
            if case .toolChips = it.kind { run.append(it) } else { flush(); out.append(it) }
        }
        flush()
        merged?.turn()
        return out
    }
    /// The chip rows made last time, by the id of their first tool row (only those still shown are kept).
    @MainActor final class MergedRows {
        fileprivate var rows: [String: (parts: [ConversationItem], row: ConversationItem)] = [:]
        fileprivate var next: [String: (parts: [ConversationItem], row: ConversationItem)] = [:]
        fileprivate func turn() { rows = next; next = [:] }
    }

    /// Vertical gap between rows (reading.md §3): 32 before a user bubble, 20 after it, 8 around chips.
    static func gap(_ a: ConversationItem, _ b: ConversationItem) -> CGFloat {
        switch (a.kind, b.kind) {
        case (_, .userText): return 32
        case (.userText, _): return 20
        case (.toolChips, _), (_, .toolChips): return 8
        case (_, .status), (.status, _): return 16
        default: return 14
        }
    }

}

/// One entry of the conversation. Equatable: a row whose entry and settings are the same is not rendered again when the
/// rows around it change (a page of older rows coming in). A pending request is always rendered again: its card's
/// actions come from the view above.
private struct EntryRow: View, Equatable {
    typealias Entry = ConversationView.Entry
    let entry: Entry
    let foldable: Bool
    let size: ReadingSize
    let liveCard: String?
    let agentName: String
    let onAllow: (PermissionRequest) -> Void
    let onDeny: (PermissionRequest) -> Void
    let onOpenTerminal: () -> Void

    nonisolated static func == (a: EntryRow, b: EntryRow) -> Bool {
        guard a.entry.key == b.entry.key, a.foldable == b.foldable, a.size == b.size, a.liveCard == b.liveCard, a.agentName == b.agentName else { return false }
        switch (a.entry.kind, b.entry.kind) {
        case (.row(let x), .row(let y)):
            if case .permissionRequest = x.kind { return false }
            return x == y
        case (.separator(let x), .separator(let y)), (.footer(let x), .footer(let y)): return x == y
        default: return false
        }
    }

    var body: some View { entryView(entry, foldable: foldable) }

    @ViewBuilder private func entryView(_ e: Entry, foldable: Bool) -> some View {
        switch e.kind {
        case .row(let item): row(item, foldable: foldable).id(item.id)
        case .separator(let t):
            HStack(spacing: 10) {
                Rectangle().fill(ReadingColor.hairline).frame(width: 48, height: 0.5)
                Text(t).font(.subheadline.weight(.medium).monospacedDigit()).foregroundStyle(.tertiary)
                Rectangle().fill(ReadingColor.hairline).frame(width: 48, height: 0.5)
            }
            .frame(maxWidth: .infinity)
        case .footer(let t):
            Text(t).font(.subheadline.weight(.medium).monospacedDigit()).foregroundStyle(.tertiary)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    @ViewBuilder private func row(_ item: ConversationItem, foldable: Bool) -> some View {
        switch item.kind {
        case .userText(let t):
            if let note = ConversationParser.hcNote(t) {
                NoteRow(from: note.from, text: note.text, foldable: foldable)
            } else {
                HStack {
                    Spacer(minLength: min(size.column * 0.2, 120))
                    UserBubble(text: t, size: size, images: item.images)
                }
            }
        case .assistantText(let t):
            MarkdownView(text: t, foldable: foldable)
        case .toolChips(let chips):
            ChipGroup(chips: chips, maxWidth: size.column)
        case .permissionRequest(let r):
            if r.state == .pending {
                if r.toolUseId == liveCard { approval(r) } else { FlowLayout(spacing: 6) { ChipView(chip: ToolChip(symbol: "hourglass", text: "Running \(r.toolName.isEmpty ? "tool" : r.toolName)…")) } }
            }
        case .status(let s):
            Text(s).font(.caption.monospacedDigit()).foregroundStyle(.tertiary).frame(maxWidth: .infinity)
        case .question(let q):
            // Pending: the card stands in the composer slot (PaneView); answered or declined: one row here.
            if q.state != .pending { QuestionAnsweredRow(model: q, time: item.timestamp) }
        }
    }

    @ViewBuilder private func approval(_ r: PermissionRequest) -> some View {
        ApprovalCard(request: r, agentName: agentName, canRespond: true, onAllow: { onAllow(r) }, onDeny: { onDeny(r) }, onOpen: onOpenTerminal)
    }
}

/// In the background of the anchor row: when SwiftUI places the row (while the frame is drawn), the view moves by as much
/// as the row did, so that frame shows the row where it was. It watches its host and the document: SwiftUI can move a
/// parent of the row, and then only the document's frame changes (it grew by the rows that came in above).
struct AnchorProbe: NSViewRepresentable {
    let tracker: ConversationView.BottomTracker
    let key: String
    final class Probe: NSView {
        weak var tracker: ConversationView.BottomTracker?
        var key = ""
        private let watched = NSHashTable<NSView>.weakObjects()
        private func watch(_ views: [NSView]) {
            for w in watched.allObjects { NotificationCenter.default.removeObserver(self, name: NSView.frameDidChangeNotification, object: w) }
            watched.removeAllObjects()
            for w in views {
                watched.add(w)
                w.postsFrameChangedNotifications = true
                NotificationCenter.default.addObserver(self, selector: #selector(moved), name: NSView.frameDidChangeNotification, object: w)
            }
        }
        override func viewDidMoveToSuperview() { super.viewDidMoveToSuperview(); rewatch() }
        /// The first placement comes before the probe is in a window (no scroll view yet).
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); rewatch() }
        private func rewatch() {
            // SwiftUI may place any intermediate ancestor while the immediate host and document keep their frames.
            // Observe that placement in the same transaction, before the delayed fallback can move a drawn row.
            var ancestors = [self as NSView]
            let document = enclosingScrollView?.documentView
            var next = superview
            while let view = next {
                ancestors.append(view)
                if view === document { break }
                next = view.superview
            }
            watch(ancestors)
            moved()
        }
        deinit { NotificationCenter.default.removeObserver(self) }
        func bind(_ tracker: ConversationView.BottomTracker, key: String) {
            self.tracker = tracker; self.key = key
            moved()   // a reused, already placed NSView need not receive another frame notification
        }
        @objc func moved() {
            guard let t = tracker, window != nil, let sv = enclosingScrollView else { return }
            t.scrollView = sv; t.probe = self; t.probeKey = key
            t.fixNow(sv)
        }
    }
    func makeNSView(context: Context) -> Probe { let v = Probe(); v.bind(tracker, key: key); return v }
    func updateNSView(_ v: Probe, context: Context) { v.bind(tracker, key: key) }
}

/// "Loading earlier…" over the top of the rows while older rows come in.
private struct EarlierNotice: View {
    var body: some View {
        HStack(spacing: 6) {
            Spinner()
            Text("Loading earlier…").font(.callout).foregroundStyle(.secondary)
        }
        .padding(.horizontal, 12).padding(.vertical, 6)
        .glassCard(radius: 14)
        .allowsHitTesting(false)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Loading earlier messages")
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: {
            ConversationProbe.loading = $0
            if ConversationProbe.enabled, $0.height > 0 {
                ConversationProbe.loadingShows += 1
                ConversationProbe.lastLoadingFrame = $0
            }
        }
        .onDisappear { ConversationProbe.loading = .zero }
    }
}

/// Where the conversation's Show earlier and Jump to latest buttons sit (window coordinates, top left), for the checks.
/// The checks press them through the actions (a synthetic click does not reach a button in a scroll view of a window that is
/// not key, and a headless run never has a key window).
@MainActor enum ConversationProbe {
    /// Only a check turns them on: the actions keep the view's state alive.
    static var enabled = false
    static var earlier = CGRect.zero, latest = CGRect.zero
    /// The "Loading earlier" notice while older rows load (window coordinates), else zero.
    static var loading = CGRect.zero
    /// A delayed test observer can miss a notice that correctly appeared and disappeared between samples.
    static var loadingShows = 0
    static var lastLoadingFrame = CGRect.zero
    static var earlierFailed = false
    /// A check holds the reader's scrolling here (an offscreen window gets no trackpad gestures).
    static var scrolling = false
    /// Anchor moves the fallback made a frame late (the probe missed them): that frame showed the rows out of place.
    static var lateFixes = 0
    private(set) static var pressEarlier: (() -> Void)?, pressLatest: (() -> Void)?
    private static var owners: [Button: UUID] = [:]
    enum Button { case earlier, latest }

    static func set(_ b: Button, _ owner: UUID, _ action: @escaping () -> Void) {
        guard enabled else { return }
        owners[b] = owner
        if b == .earlier { pressEarlier = action } else { pressLatest = action }
    }

    /// Only the view that set the action clears it.
    static func clear(_ b: Button, _ owner: UUID) {
        guard owners[b] == owner else { return }
        owners[b] = nil
        if b == .earlier { pressEarlier = nil } else { pressLatest = nil }
    }
}

/// Gray bubble, right aligned, inline markdown only (reading.md §10). The owner's pictures sit under the text.
struct UserBubble: View {
    let text: String
    let size: ReadingSize
    var images: [LogImage.Ref] = []
    var body: some View {
        VStack(alignment: .trailing, spacing: 8) {
            if !text.isEmpty || images.isEmpty { MessageText(content: .bubble(text, size: size), fit: true) }
            if !images.isEmpty { PictureStrip(refs: images, maxWidth: size.column * 0.8 - 28) }
        }
            .padding(.horizontal, 14).padding(.vertical, 10)
            .background(Color.primary.opacity(0.08), in: RoundedRectangle(cornerRadius: Radius.bubble))
            .frame(maxWidth: size.column * 0.8, alignment: .trailing)
    }
}

/// A note another agent sent with `hc send`: not the owner's message, so no bubble on the right. One line on the left,
/// `↳ <name>: <first line>`, in the chip style; a click opens the whole note as Markdown under it.
struct NoteRow: View {
    let from: String
    let text: String
    var foldable = false
    @State private var open = false
    @State private var hover = false
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button { withAnimation(.snappy(duration: 0.15)) { open.toggle() } } label: {
                HStack(spacing: 5) {
                    Text(ConversationParser.noteLine(from: from, text: text)).font(.callout.weight(.medium)).foregroundStyle(.secondary)
                        .lineLimit(1).truncationMode(.tail)
                    Image(systemName: "chevron.right").font(.caption2).imageScale(.small).foregroundStyle(.tertiary)
                        .rotationEffect(.degrees(open ? 90 : 0))
                }
                .padding(.horizontal, 10).frame(height: 26)
                .background(Color.primary.opacity(hover ? 0.12 : 0.07), in: Capsule())
                .contentShape(Capsule())
                .onHover { hover = $0 }
            }
            .buttonStyle(.plain)
            .help(open ? "Hide the note" : "Show the note")
            if open { MarkdownView(text: text, foldable: foldable).padding(.leading, 10) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct ChipView: View {
    let chip: ToolChip
    var expanded = false
    @State private var hover = false
    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: chip.symbol).font(.subheadline).foregroundStyle(.secondary)
            Text(chip.text).font(.callout.weight(.medium)).foregroundStyle(chip.calls.contains { $0.isError } ? Color.primary : Color.secondary)
            if !chip.calls.isEmpty {
                Image(systemName: "chevron.right").font(.caption2).imageScale(.small).foregroundStyle(.tertiary)
                    .rotationEffect(.degrees(expanded ? 90 : 0))
            }
        }
        .padding(.horizontal, 10).frame(height: 26)
        .background(Color.primary.opacity(hover && !chip.calls.isEmpty ? 0.12 : 0.07), in: Capsule())
        .contentShape(Capsule())
        .onHover { hover = $0 }
    }
}

/// A row of tool chips. Clicking a chip expands its calls below (no popover); clicking a call shows input/output.
/// Pictures the tools returned sit under the group, folded or not; `maxWidth` is the column they fit.
struct ChipGroup: View {
    let chips: [ToolChip]
    var maxWidth: CGFloat = 480
    @State private var open: Int?
    @State private var openCall: String?
    @State private var showAll = false
    private var visible: [ToolChip] { chips.count > 5 && !showAll ? Array(chips.prefix(4)) : chips }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            FlowLayout(spacing: 6) {
                ForEach(Array(visible.enumerated()), id: \.offset) { i, chip in
                    Button {
                        guard !chip.calls.isEmpty else { return }
                        withAnimation(.snappy(duration: 0.15)) { open = open == i ? nil : i; openCall = nil }
                    } label: { ChipView(chip: chip, expanded: open == i) }
                    .buttonStyle(.plain)
                    .help(chip.calls.isEmpty ? chip.text : "Show calls")
                }
                if chips.count > 5 {
                    Button { withAnimation(.snappy(duration: 0.15)) { showAll.toggle(); open = nil } } label: {
                        ChipView(chip: ToolChip(symbol: showAll ? "chevron.up" : "ellipsis", text: showAll ? "Show fewer" : "+\(chips.count - 4) more"))
                    }
                    .buttonStyle(.plain)
                }
            }
            if let i = open, visible.indices.contains(i) {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(visible[i].calls) { call in callRow(call, symbol: visible[i].symbol) }
                }
                .padding(6)
                .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: Radius.card))
            }
            let pictures = chips.flatMap(\.calls).flatMap(\.images)
            if !pictures.isEmpty { PictureStrip(refs: pictures, single: maxWidth, maxWidth: maxWidth) }
        }
    }

    @ViewBuilder private func callRow(_ call: ToolCall, symbol: String) -> some View {
        let isOpen = openCall == call.id
        CallHeader(call: call, symbol: symbol, isOpen: isOpen) {
            withAnimation(.snappy(duration: 0.15)) { openCall = isOpen ? nil : call.id }
        }
        if isOpen {
            VStack(alignment: .leading, spacing: 6) {
                CodeBox(label: "Input", text: call.input)
                if let out = call.output { CodeBox(label: call.isError ? "Error" : "Output", text: out, isError: call.isError) }
                else { Text("Running…").font(.caption).foregroundStyle(.secondary) }
            }
            .padding(.leading, 22).padding(.bottom, 4)
        }
    }
}

private struct CallHeader: View {
    let call: ToolCall
    let symbol: String
    let isOpen: Bool
    let action: () -> Void
    @State private var hover = false
    var body: some View {
        Button(action: action) {
            HStack(spacing: 8) {
                Image(systemName: symbol).font(.subheadline).foregroundStyle(.secondary).frame(width: 14)
                Text(call.summary.isEmpty ? call.toolName : call.summary)
                    .font(.system(.callout, design: .monospaced)).lineLimit(1).truncationMode(.middle)
                Spacer(minLength: 4)
                if call.isError { Image(systemName: "exclamationmark.triangle.fill").font(.caption).foregroundStyle(.red) }
                else if call.output == nil { Spinner(size: .mini) }
                Image(systemName: "chevron.right").font(.caption2).imageScale(.small).foregroundStyle(.tertiary)
                    .rotationEffect(.degrees(isOpen ? 90 : 0))
            }
            .padding(.horizontal, 6).frame(height: 24)
            .background(Color.primary.opacity(hover ? 0.07 : 0), in: RoundedRectangle(cornerRadius: Radius.control))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hover = $0 }
        .help(call.toolName)
    }
}

private struct CodeBox: View {
    let label: String
    let text: String
    var isError = false
    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
            ScrollView(.vertical) {
                MessageText(content: .plain(text.isEmpty ? "(empty)" : text, monoSize: NSFont.preferredFont(forTextStyle: .subheadline).pointSize,
                                            error: isError))
                    .frame(maxWidth: .infinity, alignment: .leading).padding(8)
            }
            .frame(maxHeight: 190)
            .fixedSize(horizontal: false, vertical: true)
            .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: Radius.control))
        }
    }
}

/// Tests (`--test-permission`): a card every blocked pane shows in its composer slot, as if read from its screen; nil in
/// the app.
@MainActor enum ApprovalProbe {
    static var injected: PermissionRequest?
    /// The composer card's global frame, last reported.
    static var cardFrame = CGRect.zero
}

struct ApprovalCard: View {
    let request: PermissionRequest
    let agentName: String
    var canRespond = false
    let onAllow: () -> Void
    let onDeny: () -> Void
    let onOpen: () -> Void
    /// Outside a scroll view (the composer slot): the card's height limit; the command scrolls inside.
    var maxHeight: CGFloat? = nil
    @State private var commandHeight: CGFloat = 0
    /// The title row's and the button row's heights: the card's height without the command is these, two spacings and the
    /// padding. Measured on their own, never from the card's height, which follows the command.
    @State private var head: CGFloat = 0
    @State private var foot: CGFloat = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 10) {
                Image(systemName: "exclamationmark")
                    .font(.body.weight(.bold)).foregroundStyle(.white)
                    .frame(width: 30, height: 30)
                    .background(Color.orange, in: RoundedRectangle(cornerRadius: Radius.control))
                Text(request.title).font(.headline).lineLimit(maxHeight == nil ? nil : 3)
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { head = $0 }
            if request.command.isEmpty {
                // No command to show (a prompt box taller than the pane: `ApprovalNotice.clippedTitle`).
            } else if maxHeight != nil {
                // A text view's own height is taken at the narrowest width (a window's minimum size asks at width 0): a long
                // command there made the window tens of thousands of points tall. In a scroll view of measured height it never is.
                ScrollView {
                    command.onGeometryChange(for: CGFloat.self) { $0.size.height } action: { commandHeight = $0 }
                }
                .scrollBounceBehavior(.basedOnSize)
                .frame(minHeight: min(commandHeight, 60), maxHeight: scrollHeight)   // never a fixed height: the window may get smaller
                .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: Radius.control))
            } else {
                command.background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: Radius.control))
            }
            HStack {
                if canRespond {
                    Button(action: onOpen) { Label("Open terminal", systemImage: "apple.terminal") }
                    Spacer()
                    Button("Deny", action: onDeny)
                    Button("Allow", action: onAllow).buttonStyle(.borderedProminent)
                } else {
                    Spacer()
                    Button(action: onOpen) { Label("Open terminal", systemImage: "apple.terminal") }.buttonStyle(.borderedProminent)
                }
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { foot = $0 }
        }
        .padding(16)
        .glassCard(radius: Radius.card, tint: .orange.opacity(0.08))
        .shadow(color: .black.opacity(0.12), radius: 14, y: 6)
    }

    private var command: some View {
        MessageText(content: .plain(request.command, monoSize: NSFont.preferredFont(forTextStyle: .callout).pointSize))
            .padding(.horizontal, 10).padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The command's height: all of it when it fits under `maxHeight`, else what is left after the rest of the card.
    private var scrollHeight: CGFloat {
        let chrome: CGFloat = head + foot + 2 * 12 + 2 * 16   // the rows, the VStack's two spacings, the padding
        let limit: CGFloat = maxHeight ?? .infinity
        return min(commandHeight, max(60, limit - chrome))
    }
}

/// Wrapping row layout for chips.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(proposal.width ?? .infinity, subviews).size
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let r = arrange(bounds.width, subviews)
        for (i, p) in r.points.enumerated() {
            subviews[i].place(at: CGPoint(x: bounds.minX + p.x, y: bounds.minY + p.y), proposal: ProposedViewSize(r.sizes[i]))
        }
    }
    /// A subview wider than the width gets the width (it wraps or cuts its text).
    private func arrange(_ width: CGFloat, _ subviews: Subviews) -> (size: CGSize, points: [CGPoint], sizes: [CGSize]) {
        var x: CGFloat = 0, y: CGFloat = 0, rowH: CGFloat = 0, maxW: CGFloat = 0
        var pts: [CGPoint] = [], sizes: [CGSize] = []
        for s in subviews {
            var sz = s.sizeThatFits(.unspecified)
            if sz.width > width, width.isFinite { sz = s.sizeThatFits(ProposedViewSize(width: width, height: nil)); sz.width = min(sz.width, width) }
            sizes.append(sz)
            if x + sz.width > width, x > 0 { x = 0; y += rowH + spacing; rowH = 0 }
            pts.append(CGPoint(x: x, y: y))
            x += sz.width + spacing; rowH = max(rowH, sz.height); maxW = max(maxW, x - spacing)
        }
        return (CGSize(width: maxW, height: y + rowH), pts, sizes)
    }
}

/// One line of small text over the conversation when it is the lead's last known log, not the one herdr names.
struct TeamLine: View {
    let team: TeamNote?
    static func text(_ team: TeamNote?) -> String? { team == .lastLead ? TeamNote.lastLeadLine : nil }
    var body: some View {
        if let t = Self.text(team) {
            Text(t).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                .frame(maxWidth: .infinity).padding(.vertical, 4)
        }
    }
}
