import HerdrKit
import SwiftUI
import SwiftTerm

/// Read-only live terminal fed by `HerdrClient.observe` frames.
struct TerminalSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var handle = TerminalHandle()

    var body: some View {
        let pane = model.tab.flatMap { t in t.panes.first { $0.id == model.focusedPaneID(in: t) } ?? t.panes.first }
        let size = TerminalSheetSize.clamp(model.terminalSize, window: model.windowSize)
        VStack(spacing: 0) {
            HStack {
                Image(systemName: "apple.terminal")
                Text("Terminal · \(model.tab?.label ?? "")").font(.headline)
                if !model.writesEnabled { Text("read-only").font(.caption).foregroundStyle(.secondary) }
                Spacer()
                Button { dismiss() } label: {
                    Image(systemName: "xmark").font(.subheadline.weight(.semibold))
                        .frame(width: 28, height: 28)
                        .modifier(IslandGlass())
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .keyboardShortcut(model.terminalHistoryOpen ? nil : .cancelAction)   // Esc leaves history first
                .help("Close (Esc)")
                .accessibilityLabel("Close")
                .onAppear { ChromeProbe.sheetClose = { dismiss() } }
            }.padding(12)
            Divider()
            if let pane {
                ObservedTerminal(client: model.store.client, hc: pane.backend == .hc ? model.store.hcClient : nil, paneId: pane.id, terminalId: pane.terminalId, writable: TerminalPolicy.writable(backend: pane.backend, writesEnabled: model.writesEnabled), focused: true,
                                 handle: handle,
                                 rightClickToPane: model.rightClickToPane.contains(pane.id),
                                 onHistoryChange: { model.terminalHistoryOpen = $0 }).id(pane.terminalId)
            } else {
                ContentUnavailableView("No pane", systemImage: "apple.terminal")
            }
        }
        .frame(width: size.width, height: size.height)
        .background(Color(nsColor: .textBackgroundColor))   // terminal stays opaque
        // The sheet is attached top-centre, so a drag moves the pointer by half the width change.
        .overlay(alignment: .trailing) { ResizeGrip(edge: .right) }
        .overlay(alignment: .leading) { ResizeGrip(edge: .left) }
        .overlay(alignment: .bottom) { ResizeGrip(edge: .bottom) }
        .overlay(alignment: .bottomTrailing) { ResizeGrip(edge: .corner) }
        .background(SheetWindowSizer())
        .onAppear { ChromeProbe.sheetHandle = handle }
    }
}

/// Edge drag of the sheet. Pure geometry in screen coordinates (y up): from the sheet's frame and the pointer at the press,
/// and the pointer now. The dragged edge goes exactly where the pointer is.
/// Absolute against the press, so nothing the system does to the window meanwhile (re-centring) can feed back.
enum SheetDrag {
    enum Edge { case left, right, bottom, corner }
    static func frame(edge: Edge, start: NSRect, startMouse: CGPoint, mouse: CGPoint, minSize: CGSize, maxSize: CGSize) -> NSRect {
        let dx = mouse.x - startMouse.x, dy = mouse.y - startMouse.y
        var f = start
        func w(_ v: CGFloat) -> CGFloat { Swift.min(Swift.max(v, minSize.width), maxSize.width) }
        func h(_ v: CGFloat) -> CGFloat { Swift.min(Swift.max(v, minSize.height), maxSize.height) }
        // A sheet stays centred on its parent (AppKit puts it back), so an edge under the pointer means the size
        // changes by twice the pointer's travel, the opposite edge going the other way (measured: centred both ways).
        switch edge {
        case .right: f.size.width = w(start.width + 2 * dx)
        case .left: f.size.width = w(start.width - 2 * dx)
        case .bottom: f.size.height = h(start.height - 2 * dy)
        case .corner: f.size.width = w(start.width + 2 * dx); f.size.height = h(start.height - 2 * dy)
        }
        f.origin.x = start.midX - f.size.width / 2
        f.origin.y = start.midY - f.size.height / 2
        return f
    }

    /// One drag tick on the real sheet; the model follows the window.
    @MainActor static func apply(edge: Edge, start: (frame: NSRect, mouse: CGPoint), mouse: CGPoint, window: NSWindow, model: AppModel) {
        let lo = window.frameRect(forContentRect: NSRect(origin: .zero, size: TerminalSheetSize.min)).size
        let hiContent = TerminalSheetSize.clamp(CGSize(width: 100_000, height: 100_000), window: model.windowSize)
        let hi = window.frameRect(forContentRect: NSRect(origin: .zero, size: hiContent)).size
        model.terminalDragging = true
        let f = frame(edge: edge, start: start.frame, startMouse: start.mouse, mouse: mouse, minSize: lo, maxSize: hi)
        model.sheetTargetFrame = f
        window.setFrame(f, display: true, animate: false)
        if let cv = window.contentView { model.terminalSize = cv.bounds.size }
    }
}

/// Drag handles on the sheet's edges and bottom-right corner. A native resize is not available on a SwiftUI sheet
/// (SwiftUI keeps resetting the window's limits and ignores width changes), so these move the sheet window's own edges.
/// The pty resize behind the size change is debounced by the terminal view (150 ms).
struct ResizeGrip: View {
    let edge: SheetDrag.Edge
    @Environment(AppModel.self) private var model
    @State private var start: (frame: NSRect, mouse: CGPoint)?

    var body: some View {
        Color.clear
            .frame(width: edge == .bottom ? nil : (edge == .corner ? 18 : 6), height: edge == .left || edge == .right ? nil : (edge == .corner ? 18 : 6))
            .padding(edge == .left || edge == .right ? .vertical : .horizontal, edge == .corner ? 0 : 16)
            .contentShape(Rectangle())
            .pointerStyle(.frameResize(position: position))
            .gesture(DragGesture(minimumDistance: 0, coordinateSpace: .global)
                .onChanged { _ in
                    guard let win = model.sheetWindow else { return }
                    let now = NSEvent.mouseLocation
                    let s0 = start ?? (win.frame, now)
                    start = s0
                    SheetDrag.apply(edge: edge, start: s0, mouse: now, window: win, model: model)
                }
                .onEnded { _ in start = nil; model.terminalDragging = false; TerminalSheetSize.save(model.terminalSize) })
    }
    private var position: FrameResizePosition {
        switch edge { case .left: .leading; case .right: .trailing; case .bottom: .bottom; case .corner: .bottomTrailing }
    }
}

/// A SwiftUI sheet does not follow its content when the content changes size: set the sheet window's content size
/// ourselves (size changes from outside a drag), keeping it attached at the top centre of the parent.
struct SheetWindowSizer: NSViewRepresentable {
    @Environment(AppModel.self) private var model
    func makeNSView(context: Context) -> NSView { NSView() }
    func updateNSView(_ v: NSView, context: Context) {
        let model = model
        DispatchQueue.main.async {
            guard let w = v.window else { return }
            if model.sheetWindow !== w {
                model.sheetWindow = w
                // SwiftUI re-centres the sheet after a content size change, later than any one update pass: hold the drag's frame.
                for name in [NSWindow.didResizeNotification, NSWindow.didMoveNotification] {
                    NotificationCenter.default.addObserver(forName: name, object: w, queue: .main) { [weak model, weak w] _ in
                        MainActor.assumeIsolated {
                            guard let model, let w, let t = model.sheetTargetFrame, w.frame != t else { return }
                            w.setFrame(t, display: true, animate: false)
                        }
                    }
                }
            }
            if let t = model.sheetTargetFrame {   // during a drag the window goes exactly where the drag put it
                if w.frame != t { w.setFrame(t, display: true, animate: false) }
                if !model.terminalDragging { model.sheetTargetFrame = nil }
                return
            }
            guard let cv = w.contentView else { return }
            let size = TerminalSheetSize.clamp(model.terminalSize, window: model.windowSize)
            guard abs(cv.bounds.width - size.width) > 0.5 || abs(cv.bounds.height - size.height) > 0.5 else { return }
            var f = w.frame
            let target = w.frameRect(forContentRect: NSRect(origin: .zero, size: size)).size
            f.origin.x += (f.width - target.width) / 2
            f.origin.y += f.height - target.height
            f.size = target
            w.setFrame(f, display: true, animate: false)
        }
    }
}

/// Live terminal for one pane. Writable: a `TerminalSession` in control mode (keys, paste, resize). Read-only: observe.
/// Shows why the session ended (taken over by another client, refused, not found); it never takes over by itself.
struct ObservedTerminal: View {
    @Environment(\.underSettings) private var underSettings
    let client: HerdrClient
    /// Set for an herdr-core pane: the view attaches through hc instead of the herdr CLI channel.
    var hc: HCClient? = nil
    let paneId: String
    let terminalId: String
    var writable = false
    var focused = false
    var handle: TerminalHandle? = nil
    /// "Send right-clicks to pane" is on: a right click on the view goes to the program as an xterm mouse report.
    var rightClickToPane = false
    var onHistoryChange: (Bool) -> Void = { _ in }
    @State private var ownHandle = TerminalHandle()
    @State private var wheelState = WheelState()
    private var hnd: TerminalHandle { handle ?? ownHandle }
    /// History comes from the pane's own backend: hc (`terminal.read {lines}`) or the herdr TUI (`pane.read`).
    private var scroller: any ScrollSource { hc ?? client }

    /// Wheel bookkeeping. A plain object: written from event callbacks, never triggers a render.
    @MainActor final class WheelState {
        var accum: CGFloat = 0
        var kind: ScrollInfo.Kind?
        var kindAt = Date.distantPast
    }
    @State private var closedReason: String?
    /// The terminal could not start, or keeps stopping before it showed anything: why (`startErrorLine`).
    @State private var startError: String?
    @State private var takeover = false
    @State private var epoch = 0
    @State private var history: HistoryData?
    @State private var fetching = false
    @State private var notice: String?
    @State private var noticeShown = false
    @Environment(\.colorScheme) private var scheme

    struct HistoryData: Equatable { var text: String; var lines: Int }

    /// Wheel up on the live view. Read-only: asks herdr for the scroll state, then for the last 1000 lines.
    /// Never sends input and never moves the pane's own scroll position. Failures do nothing.
    /// Wheel over the live view (`dy` > 0 = toward older content). Decides by `ScrollInfo.kind`:
    /// - `.fullScreenProgram` + control mode: herdr forwards the wheel to the program (`TerminalSession.wheel`); never keys.
    /// - `.history`: scroll up opens the history view. `.none`: nothing.
    /// - observe/read-only on an agent pane: the notice only.
    private func wheel(_ dy: CGFloat, precise: Bool) {
        guard history == nil, !underSettings else { return }
        let st = wheelState
        if let k = st.kind, Date().timeIntervalSince(st.kindAt) < 1.5 { act(k, dy, precise); return }
        guard !fetching else { return }
        fetching = true
        Task { @MainActor in
            let info = try? await scroller.scrollInfo(paneId: paneId)
            fetching = false   // before act: wheelUp guards on it, so a deferred reset dropped the first wheel-up
            guard let info else { return }
            st.kind = info.kind; st.kindAt = Date()
            act(info.kind, dy, precise)
        }
    }
    private func act(_ kind: ScrollInfo.Kind, _ dy: CGFloat, _ precise: Bool) {
        switch kind {
        case .none: return
        case .history:
            if dy > 0 { wheelUp() }
        case .fullScreenProgram:
            if TerminalPolicy.wheelToProgram(backend: hc == nil ? .tui : .hc, writable: writable), let session = hnd.session {
                // Trackpad: one step per ~28 pt of movement (at most 4 per event); a wheel notch is one step.
                let st = wheelState
                st.accum += abs(dy)
                let unit: CGFloat = precise ? 28 : 1
                let n = min(4, Int(st.accum / unit))
                guard n > 0 else { return }
                st.accum -= CGFloat(n) * unit
                session.wheel(lines: n, up: dy > 0, eventPerLine: true)
            } else if dy > 0 { wheelUp() }
        }
    }

    private func wheelUp() {
        guard !fetching, history == nil else { return }
        fetching = true
        Task { @MainActor in
            defer { fetching = false }
            guard let info = try? await scroller.scrollInfo(paneId: paneId), info.kind != .none else { return }
            guard let sb = try? await scroller.readScrollback(paneId: paneId, lines: Scrollback.readLimit, ansi: true, unwrapped: false) else { return }
            if info.kind == .fullScreenProgram, sb.lineCount <= info.viewportRows {
                guard notice == nil else { return }
                notice = "這個程式沒有可捲動的歷史；對話內容請在對話頁看"
                try? await Task.sleep(for: .seconds(6)); notice = nil
                return
            }
            history = HistoryData(text: sb.text, lines: sb.lineCount)
            onHistoryChange(true)
        }
    }
    private func backToLive(replay: NSEvent? = nil) {
        guard history != nil else { return }
        history = nil
        onHistoryChange(false)
        guard writable, let live = hnd.live else { return }
        live.window?.makeFirstResponder(live)
        if let replay { live.keyDown(with: replay) }
    }

    var body: some View {
        TerminalNSView(client: client, hc: hc, terminalId: terminalId, writable: writable, takeover: takeover, focused: focused,
                       handle: hnd, rightClickToPane: rightClickToPane, onWheel: wheel, historyOpen: { history != nil }, onClosed: { closedReason = $0 },
                       onStartError: { startError = $0 },
                       onNotice: { n in Task { notice = n; try? await Task.sleep(for: .seconds(6)); if notice == n { notice = nil } } })
            .id("\(terminalId)-\(epoch)")
            .opacity(history == nil ? 1 : 0)   // both views are transparent: the history view replaces the live one
            .overlay {
                if let h = history {
                    HistoryTerminalNSView(text: h.text, writable: writable, scheme: scheme, handle: hnd, onExit: { backToLive(replay: $0) })
                        .overlay(alignment: .top) {
                            HStack(spacing: 10) {
                                Image(systemName: "clock.arrow.circlepath")
                                Text("History · 最後 \(h.lines) 行").font(.callout)
                                Spacer()
                                Button("Back to live") { backToLive() }.controlSize(.small)
                            }
                            .padding(.horizontal, 12).padding(.vertical, 6)
                            .background(.regularMaterial)
                        }
                }
            }
            .overlay(alignment: .bottom) {
                if let notice {
                    Text(notice).font(.callout).padding(.horizontal, 12).padding(.vertical, 6)
                        .background(.regularMaterial, in: Capsule()).padding(12)
                }
            }
            .onAppear {
                hnd.requestHistory = wheelUp
                hnd.backToLive = { backToLive() }
                hnd.isHistory = { history != nil }
            }
            .onDisappear { if history != nil { onHistoryChange(false) } }
            .overlay {
                // Not a blank view: the transparent terminal shows only the window behind it until a frame comes.
                if let e = startError {
                    // On a card: readable on an opaque Ghostty-theme background too.
                    VStack(spacing: 10) {
                        Label(Self.startErrorLine(e), systemImage: "exclamationmark.triangle.fill")
                            .font(.callout).foregroundStyle(.secondary).multilineTextAlignment(.center)
                        Button("Retry") { startError = nil; closedReason = nil; epoch += 1 }.controlSize(.small)
                    }
                    .padding(20)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: Radius.card))
                    .padding(20)
                }
            }
            .onChange(of: startError, initial: true) { _, e in
                TerminalProbe.startError = e.map(Self.startErrorLine)
                TerminalProbe.retry = e == nil ? nil : { startError = nil; closedReason = nil; epoch += 1 }
            }
            .overlay(alignment: .top) {
                if let r = closedReason, r != "detached" {
                    HStack(spacing: 10) {
                        Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.orange)
                        Text(Self.explain(r)).font(.callout).lineLimit(2)
                        if writable, !Self.isGone(r) {
                            Button("Take over") { takeover = true; closedReason = nil; epoch += 1 }.controlSize(.small)
                                .help("Make this window the controller of the terminal")
                        }
                    }
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: Radius.card))
                    .padding(10)
                }
            }
    }

    static func startErrorLine(_ reason: String) -> String { "Can't start the terminal: " + reason }

    static func explain(_ reason: String) -> String {
        if reason.contains("taken over") { return "Another client took over this terminal." }
        if reason.contains("already has an attached client") { return "Another client is controlling this terminal." }
        if reason.contains("not found") { return "This terminal no longer exists." }
        if reason.hasPrefix("finished") { return "The program in this terminal has ended." }
        if reason.hasPrefix("machine_unknown") { return "This terminal's machine is not known to herdr-core." }
        return reason
    }

    /// No controller can bring it back: no "Take over".
    static func isGone(_ reason: String) -> Bool {
        reason.contains("not found") || ["finished", "machine_unknown", "needs_framed"].contains { reason.hasPrefix($0) }
    }
}

/// Tests: what the terminal view showed, and its Retry.
@MainActor enum TerminalProbe {
    static var startError: String?
    static var retry: (() -> Void)?
}

struct TerminalNSView: NSViewRepresentable {
    /// Tests only: the program in place of herdr.
    static var testLauncher: [String]?
    let client: HerdrClient
    var hc: HCClient? = nil
    let terminalId: String
    var writable = false
    var takeover = false
    var focused = false
    var handle: TerminalHandle? = nil
    var rightClickToPane = false
    var onWheel: (CGFloat, Bool) -> Void = { _, _ in }
    var historyOpen: () -> Bool = { false }
    var onClosed: (String) -> Void = { _ in }
    /// Why the terminal does not start (nil again once a frame comes).
    var onStartError: (String?) -> Void = { _ in }
    /// Session notices worth showing (hc: keys typed while connecting that were not sent).
    var onNotice: (String) -> Void = { _ in }
    @Environment(\.colorScheme) private var scheme
    @AppStorage(TerminalColors.key, store: AppDefaults.store) private var colorsSetting = TerminalColors.flockbook.rawValue

    func makeCoordinator() -> Coordinator { Coordinator() }

    private var mode: TerminalColors { TerminalColors(rawValue: colorsSetting) ?? .flockbook }
    private var colorsKey: String { "\(scheme)-\(mode.rawValue)-\(TerminalThemes.shared.generation)" }

    func makeNSView(context: Context) -> PasteTerminalView {
        let tv = PasteTerminalView(frame: .zero)
        TerminalFont.apply(tv)
        let c = context.coordinator
        c.writable = writable
        c.onClosed = onClosed
        c.onStartError = onStartError
        c.onNotice = onNotice
        c.handle = handle
        handle?.live = tv
        tv.onWheel = onWheel; tv.historyOpen = historyOpen; tv.rightClickToPane = rightClickToPane && writable
        tv.coordinator = c
        tv.terminalDelegate = c
        tv.allowMouseReporting = writable
        // A new terminal reads the theme again (the user may have changed it in Ghostty).
        if mode == .ghostty { TerminalThemes.shared.reload() }
        Self.applyColors(tv, scheme, mode: mode)
        c.colors = colorsKey
        let sessionMode: TerminalSession.Mode = writable ? .control(takeover: takeover) : .observe
        let session = hc.map { TerminalSession(hcTerminal: terminalId, client: $0, mode: sessionMode) }
            ?? client.terminalSession(terminalId: terminalId, mode: sessionMode, launcher: Self.testLauncher)
        c.session = session
        handle?.session = session
        session.start()
        c.task = Task { @MainActor in
            var framed = false, said: String?
            for await ev in session.events {
                switch ev {
                case .frame(let frame):
                    if !framed { framed = true; c.onStartError(nil) }
                    if frame.full {
                        let t = tv.getTerminal()
                        if frame.width > 0, frame.height > 0, (t.cols != frame.width || t.rows != frame.height) { t.resize(cols: frame.width, rows: frame.height) }
                        tv.feed(text: "\u{1b}c")
                    }
                    tv.feed(byteArray: ArraySlice([UInt8](frame.bytes)))
                    frame.consumed()   // hc: CREDIT only for what the view has drawn
                case .closed(let reason): c.onClosed(reason)
                case .notice(let s):
                    said = s
                    if s.contains("not sent") { c.onNotice(s) }   // hc: keys typed while connecting; other notices are CLI stderr chatter
                // Stopped again before any frame: herdr's last words, while it keeps trying.
                case .reconnecting: if !framed { c.onStartError(said ?? "herdr's terminal stopped before it showed anything.") }
                case .failed(let reason): c.onStartError(reason)
                }
            }
        }
        return tv
    }

    func updateNSView(_ tv: PasteTerminalView, context: Context) {
        SettingsFocus.cover(tv, context.environment.underSettings)
        let c = context.coordinator
        c.writable = writable; c.onClosed = onClosed; c.onStartError = onStartError; c.onNotice = onNotice; c.covered = context.environment.underSettings
        tv.onWheel = onWheel; tv.historyOpen = historyOpen; tv.rightClickToPane = rightClickToPane && writable
        tv.allowMouseReporting = writable
        if c.colors != colorsKey {
            // Just turned on in Settings: read the theme now.
            if mode == .ghostty, c.colors?.contains("-\(TerminalColors.ghostty.rawValue)-") == false { TerminalThemes.shared.reload() }
            c.colors = colorsKey
            Self.applyColors(tv, scheme, mode: mode)
        }
        if focused, !c.wasFocused, writable {
            DispatchQueue.main.async { tv.window?.makeFirstResponder(tv) }
        }
        c.wasFocused = focused
    }

    /// Leaving the screen gives the terminal back to herdr (the pty returns to the other client's size).
    static func dismantleNSView(_ nsView: PasteTerminalView, coordinator: Coordinator) {
        coordinator.task?.cancel()
        coordinator.session?.release()
    }

    /// The default background is transparent, so a terminal sits on the same window surface as every other pane
    /// (an opaque fill was a lighter block next to the conversation). Cells with their own colour, the caret and
    /// selections stay opaque. SwiftTerm's scroller is hidden: the live view never scrolls (wheel = history), and the
    /// history view shows it only while scrolling. With the Ghostty-theme setting: the theme's background (opaque),
    /// foreground, palette, cursor and selection; a theme that cannot be read gives Flockbook's colours.
    static func applyColors(_ tv: TerminalView, _ scheme: ColorScheme, mode: TerminalColors = .current) {
        let dark = scheme == .dark
        tv.scroller?.isHidden = true
        if let t = TerminalThemes.shared.theme(mode, dark: dark) {
            themed.add(tv)
            tv.nativeBackgroundColor = t.background.ns
            tv.backgroundOpacity = 1
            tv.nativeForegroundColor = t.foreground.ns
            tv.installColors(t.palette.map(\.term))
            tv.caretColor = t.cursor.ns
            tv.caretTextColor = t.cursorText.ns
            tv.selectedTextBackgroundColor = t.selectionBackground.ns
            tv.selectedTextForegroundColor = t.selectionForeground.ns
            return
        }
        // Themed before: SwiftTerm's own palette, cursor (drawn in the foreground) and selection again. Never on a view
        // that was not themed: it keeps what the program in it set (OSC 4, OSC 12).
        if themed.contains(tv) {
            themed.remove(tv)
            tv.installColors(SwiftTerm.Color.terminalAppColors)
            tv.setCursorColor(source: tv.getTerminal(), color: nil, textColor: nil)
            tv.selectedTextBackgroundColor = NSColor(srgbRed: 0, green: 166 / 255, blue: 178 / 255, alpha: 1)
            tv.selectedTextForegroundColor = .black
        }
        tv.nativeBackgroundColor = (dark ? NSColor(srgbRed: 0.118, green: 0.118, blue: 0.118, alpha: 1) : .white)
        tv.backgroundOpacity = 0
        tv.nativeForegroundColor = dark ? NSColor(srgbRed: 0.91, green: 0.91, blue: 0.918, alpha: 1) : NSColor(srgbRed: 0.114, green: 0.114, blue: 0.122, alpha: 1)
    }

    /// Views in a Ghostty theme's colours now.
    @MainActor private static let themed = NSHashTable<TerminalView>.weakObjects()

    @MainActor final class Coordinator: NSObject, TerminalViewDelegate {
        nonisolated(unsafe) var task: Task<Void, Never>?
        nonisolated(unsafe) var session: TerminalSession?
        var writable = false
        /// Under the Settings page: nothing goes to the pane, whatever reaches the view (SwiftTerm's own mouse-moved
        /// reports do not go through hit-testing).
        var covered = false
        weak var handle: TerminalHandle?
        var wasFocused = false
        /// The colours last applied: scheme, setting and theme read.
        var colors: String?
        var onClosed: (String) -> Void = { _ in }
        var onStartError: (String?) -> Void = { _ in }
        var onNotice: (String) -> Void = { _ in }
        private var resizeTask: Task<Void, Never>?

        /// Keystrokes (IME text, Ctrl/Option/arrows are already encoded by SwiftTerm) go to the terminal session.
        nonisolated func send(source: TerminalView, data: ArraySlice<UInt8>) {
            let bytes = [UInt8](data)
            Task { @MainActor in
                guard writable, !covered, !bytes.isEmpty else { return }
                handle?.sentBytes += bytes.count
                _ = session?.send(bytes: bytes)
            }
        }
        /// The view's cell size changed: resize the real pty (debounced, so a window drag sends few resizes).
        nonisolated func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
            Task { @MainActor in
                guard writable, newCols > 0, newRows > 0 else { return }
                resizeTask?.cancel()
                resizeTask = Task { @MainActor in
                    try? await Task.sleep(for: .milliseconds(150))
                    if !Task.isCancelled { session?.resize(cols: newCols, rows: newRows) }
                }
            }
        }
        nonisolated func setTerminalTitle(source: TerminalView, title: String) {}
        nonisolated func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
        nonisolated func scrolled(source: TerminalView, position: Double) {}
        nonisolated func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
            if let u = URL(string: link), ["http", "https"].contains(u.scheme) { NSWorkspace.shared.open(u) }
        }
        nonisolated func bell(source: TerminalView) { NSSound.beep() }
        nonisolated func clipboardCopy(source: TerminalView, content: Data) {
            NSPasteboard.general.clearContents(); NSPasteboard.general.setData(content, forType: .string)
        }
        nonisolated func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {}
        nonisolated func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
    }
}

/// herdr's frames do not carry the program's paste mode, so paste goes through `TerminalSession.paste`
/// (bracketed when herdr's pane has it on) instead of SwiftTerm's own mode check.
final class PasteTerminalView: TerminalView {
    @MainActor weak var coordinator: TerminalNSView.Coordinator?
    /// Wheel up asks for the history view. The live view itself never scrolls and never turns the wheel into keys.
    /// (SwiftTerm's `scrollWheel` is not `open`, so a local event monitor takes the wheel before the view sees it.)
    var onWheel: ((CGFloat, Bool) -> Void)?
    nonisolated(unsafe) var monitor: Any?
    nonisolated(unsafe) var rightMonitor: Any?
    /// Right clicks go to the program (xterm report through the control session) instead of the pane menu.
    var rightClickToPane = false
    /// Checks: right clicks the monitor took for the program.
    static var rightClicksTaken = 0
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if let m = monitor { NSEvent.removeMonitor(m); monitor = nil }
        if let m = rightMonitor { NSEvent.removeMonitor(m); rightMonitor = nil }
        guard window != nil else { return }
        rightMonitor = NSEvent.addLocalMonitorForEvents(matching: .rightMouseDown) { [weak self] e in
            guard let self, self.rightClickToPane, !SettingsFocus.covered(self), let p = eventPoint(e, in: self), self.bounds.contains(p) else { return e }
            let t = self.getTerminal()
            guard t.cols > 0, t.rows > 0 else { return e }
            let col = min(t.cols, max(1, Int(p.x / (self.bounds.width / CGFloat(t.cols))) + 1))
            let yTop = self.isFlipped ? p.y : self.bounds.height - p.y
            let row = min(t.rows, max(1, Int(yTop / (self.bounds.height / CGFloat(t.rows))) + 1))
            Self.rightClicksTaken += 1
            if self.coordinator?.covered != true { _ = self.coordinator?.session?.rightClick(col: col, row: row) }
            return nil
        }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] e in
            guard let self, !self.isHiddenOrCovered(by: e), let p = eventPoint(e, in: self) else { return e }
            guard self.bounds.contains(p) else { return e }
            if e.scrollingDeltaY != 0 { self.onWheel?(e.scrollingDeltaY, e.hasPreciseScrollingDeltas) }
            return nil
        }
    }
    /// True when the history view is open over this one: it takes the wheel itself.
    var historyOpen: () -> Bool = { false }
    /// Hidden, under the history view, or under the Settings page.
    private func isHiddenOrCovered(by e: NSEvent) -> Bool { isHiddenOrHasHiddenAncestor || historyOpen() || SettingsFocus.covered(self) }
    deinit { if let m = monitor { NSEvent.removeMonitor(m) }; if let m = rightMonitor { NSEvent.removeMonitor(m) } }
    override func paste(_ sender: Any) {
        guard let c = coordinator, c.writable, !c.covered, let text = NSPasteboard.general.string(forType: .string), !text.isEmpty else { return }
        _ = c.session?.paste(text)
    }
}

/// Lets the owner (and tests) reach the live view and the history state of one terminal.
@MainActor final class TerminalHandle {
    weak var live: TerminalView?
    /// The control/observe session behind the live view (for `wheel`).
    weak var session: TerminalSession?
    weak var history: TerminalView?
    /// Bytes this view sent to the pane (keys, paste). History view must not add to it.
    var sentBytes = 0
    var requestHistory: () -> Void = {}
    var backToLive: () -> Void = {}
    var isHistory: () -> Bool = { false }
}

extension TerminalView {
    /// SwiftTerm's own scroller (a plain subview; it does not auto-hide like a scroll view's overlay scroller).
    var scroller: NSScroller? { subviews.lazy.compactMap { $0 as? NSScroller }.first }
}

/// Read-only local view of herdr's scrollback (`readScrollback`). Sends nothing to the pane.
final class HistoryTerminalView: TerminalView {
    var pending: String?
    var onBottomScroll: (() -> Void)?
    var onExit: ((NSEvent?) -> Void)?
    var writable = false

    override func layout() {
        super.layout()
        // Feed after the first layout, so the text wraps at the real column count and ends at the bottom.
        if let text = pending, bounds.width > 20, bounds.height > 20 {
            pending = nil
            getTerminal().changeHistorySize(5000)
            feed(text: text)
        }
    }
    nonisolated(unsafe) var monitors: [Any] = []
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        monitors.forEach(NSEvent.removeMonitor); monitors = []
        guard window != nil else { return }
        // Wheel down while already at the bottom leaves the history. Everything else scrolls the view itself.
        monitors.append(NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] e in
            guard let self, !SettingsFocus.covered(self), let p = eventPoint(e, in: self), self.bounds.contains(p) else { return e }
            let atBottom = !self.canScroll || self.scrollPosition >= 0.999
            self.flashScroller()
            if e.scrollingDeltaY < 0, atBottom, e.momentumPhase.isEmpty { self.onBottomScroll?(); return nil }
            return e
        } as Any)
        // Esc leaves. Typing goes back to live first (writable), then is sent there. The pane gets nothing from this view.
        monitors.append(NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] e in
            guard let self, e.window === self.window, self.window?.firstResponder === self else { return e }
            if e.keyCode == 53 { self.onExit?(nil); return nil }
            if e.modifierFlags.contains(.command) { return (e.charactersIgnoringModifiers == "v") ? nil : e }
            if self.writable { self.onExit?(e) }
            return nil
        } as Any)
    }
    deinit { monitors.forEach(NSEvent.removeMonitor) }
    /// Overlay-scroller behaviour: visible while the wheel moves, gone 1 s after.
    private var hideScroller: DispatchWorkItem?
    func flashScroller() {
        guard canScroll, let s = scroller else { return }
        s.isHidden = false
        hideScroller?.cancel()
        let w = DispatchWorkItem { [weak s] in s?.isHidden = true }
        hideScroller = w
        DispatchQueue.main.asyncAfter(deadline: .now() + 1, execute: w)
    }
}

struct HistoryTerminalNSView: NSViewRepresentable {
    let text: String
    let writable: Bool
    let scheme: ColorScheme
    var handle: TerminalHandle?
    var onExit: (NSEvent?) -> Void

    func makeNSView(context: Context) -> HistoryTerminalView {
        let tv = HistoryTerminalView(frame: .zero)
        TerminalFont.apply(tv)
        tv.allowMouseReporting = false
        tv.pending = text
        tv.writable = writable
        tv.onBottomScroll = { onExit(nil) }
        tv.onExit = onExit
        TerminalNSView.applyColors(tv, scheme)
        handle?.history = tv
        DispatchQueue.main.async { tv.window?.makeFirstResponder(tv) }
        return tv
    }
    func updateNSView(_ tv: HistoryTerminalView, context: Context) {
        SettingsFocus.cover(tv, context.environment.underSettings)
        tv.onBottomScroll = { onExit(nil) }
        tv.onExit = onExit
    }
}

/// Event position in `view` coordinates. Events of this view's window carry window coordinates; an event without a
/// window (synthetic, from a CGEvent) carries screen coordinates and is ours only when this window is the frontmost one
/// at that point: every terminal view's monitor sees it, and another window's view may lie under the same coordinates.
/// Events of other windows are not ours.
@MainActor func eventPoint(_ e: NSEvent, in view: NSView) -> NSPoint? {
    guard let w = view.window else { return nil }
    if e.window === w { return view.convert(e.locationInWindow, from: nil) }
    if e.window == nil {
        // The event's own window number when it has one, else this app's frontmost window there (works offscreen too).
        let p = e.locationInWindow
        let front = (e.windowNumber > 0 ? NSApp.window(withWindowNumber: e.windowNumber) : nil)
            ?? NSApp.orderedWindows.first { $0.isVisible && $0.frame.contains(p) }
        guard front === w else { return nil }
        return view.convert(w.convertPoint(fromScreen: e.locationInWindow), from: nil)
    }
    return nil
}
