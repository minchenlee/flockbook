import HerdrKit
import SwiftUI
import AppKit

@main
struct HerdrDeskApp: App {
    @State private var model: AppModel
    /// Headless checks build their own offscreen windows. The scene's window then shows no RootView: its views would
    /// write the same global probes (ChromeProbe, TabProbe) as the test's window and a check could read the wrong one.
    private let headless: Bool

    init() {
        let args = CommandLine.arguments
        // A check flag the app does not know would start a normal launch (real window, Dock icon): stop before any of it.
        if let usage = LaunchGuard.problem(args) ?? LaunchGuard.realWindowRefused(args, environment: ProcessInfo.processInfo.environment) {
            FileHandle.standardError.write(Data((usage + "\n").utf8))
            exit(2)
        }
        if !args.contains("--test-composer") { DispatchQueue.global(qos: .utility).async { PasteImages.purgeOld() } }
        // Checks must never show a window, a Dock icon, or take focus.
        let headlessFlags = LaunchGuard.headless.union(ContentTests.flags)
        if !headlessFlags.isDisjoint(with: args) || args.contains(where: { $0.hasPrefix("--test") }) {
            AppDefaults.isolateForTests()   // own settings store per run: never the owner's, never another run's
        }
        AppDefaults.migrateOldSettings()   // after the test isolation: test runs skip it
        ScrollerStyle.install()   // thin overlay scroll bars, also with a mouse attached
        headless = !headlessFlags.isDisjoint(with: args)
        // Updates: only a real launch of an installed app checks (UpdaterGate). Never a check, preview or snapshot run.
        let quiet = headless || args.contains { $0.hasPrefix("--test") || $0.hasPrefix("--preview") || $0.hasPrefix("--snapshot") || $0 == "--fuzz" }
        AppUpdater.shared.start(quiet: quiet)
        Notifier.shared.quiet = quiet
        if let i = args.firstIndex(of: "--test-update"), args.indices.contains(i + 1) {
            let feed = args[i + 1]
            DispatchQueue.main.async { HeadlessUpdateTest.run(feed: feed) }
        }
        if !headlessFlags.isDisjoint(with: args) {
            NSApplication.shared.setActivationPolicy(.prohibited)
            setvbuf(stdout, nil, _IOLBF, 0)   // line-buffered: a check killed by its timeout still shows how far it got
        }
        // Checks that drive a live pane need `<flag> <paneId>` and a named session (never the default one). Without them their
        // runner never starts and the headless app would idle silently until killed: say so and exit.
        for flag in ["--test-approve", "--test-notice", "--test-hub", "--test-terminal", "--test-send-image", "--test-wheel", "--test-sheet", "--test-scroll"] {
            guard let i = args.firstIndex(of: flag) else { continue }
            let named = HerdrSession.fromLaunch(arguments: args, environment: ProcessInfo.processInfo.environment).name != nil
            let pane = args.indices.contains(i + 1) && !args[i + 1].hasPrefix("--")
            if !named || !pane {
                print("\(flag): usage: --session <name> \(flag) <paneId> (\(named ? "" : "no named session; the default session is refused. ")\(pane ? "" : "no pane id."))")
                exit(2)
            }
        }
        // `--snapshot-live <dir>`: without the folder its runner never starts and the app would idle silently.
        if let i = args.firstIndex(of: "--snapshot-live"), !(args.indices.contains(i + 1) && !args[i + 1].hasPrefix("--")) {
            print("--snapshot-live: usage: --snapshot-live <output dir>"); exit(2)
        }
        let fuzzSeconds = args.firstIndex(of: "--fuzz").flatMap { i in args.indices.contains(i + 1) ? Double(args[i + 1]) : nil }
        let fuzzSeed = args.firstIndex(of: "--seed").flatMap { i in args.indices.contains(i + 1) ? UInt64(args[i + 1]) : nil } ?? 1
        let store: HerdrStore
        if let _ = fuzzSeconds {
            store = .previewMutate(seed: fuzzSeed)
        } else if args.contains("--preview-dense") {
            store = .previewDense   // many tabs and long names: enough to scroll under the island
        } else if args.contains("--preview") || args.contains("--snapshot") {
            store = .preview
        } else {
            let env = ProcessInfo.processInfo.environment
            // herdr-core too, unless `--no-hc`. Headless runs (`--snapshot-live` too) follow hc only with an explicit
            // `--hc-socket` (`HCSession.wanted`).
            let wantHC = HCSession.wanted(arguments: args, headless: headless)
            // Launch flags (--session, --remote) win; otherwise the machine chosen in Settings > General (read once: a new
            // choice there relaunches the app, as the store, the phone server and notifications all hang on this session).
            store = HerdrStore(session: RemoteMachines.launchSession(arguments: args, environment: env, defaults: AppDefaults.store),
                               hc: wantHC ? HCSession.resolve(environment: env, arguments: args) : nil)
            // The Claude hook and status line act on the machine Claude Code runs on: the host, for a remote session.
            ClaudeHookSetup.launchRemote = store.session.remote
            // Headless checks on the default session never read this store (the scene shows no RootView): no polling of
            // the owner's session. Live checks pass a named session or an explicit `--hc-socket` and use it.
            if !headless || store.session.name != nil || store.hcClient != nil { store.start() }
        }
        if let i = args.firstIndex(of: "--snapshot"), args.indices.contains(i + 1) {
            let dir = args[i + 1]
            DispatchQueue.main.async { SnapshotRunner.run(dir: dir) }
        }
        if let i = args.firstIndex(of: "--snapshot-log"), args.indices.contains(i + 2) {
            let path = args[i + 1], dir = args[i + 2]
            DispatchQueue.main.async { SnapshotRunner.runLog(path: path, dir: dir) }
        }
        if let i = args.firstIndex(of: "--snapshot-statusline"), args.indices.contains(i + 1) {
            let dir = args[i + 1]
            DispatchQueue.main.async { SnapshotRunner.runStatusLine(dir: dir) }
        }
        if let i = args.firstIndex(of: "--snapshot-queued"), args.indices.contains(i + 1) {
            let dir = args[i + 1]
            DispatchQueue.main.async { SnapshotRunner.runQueued(dir: dir) }
        }
        if let i = args.firstIndex(of: "--snapshot-reading"), args.indices.contains(i + 1) {
            let dir = args[i + 1]
            DispatchQueue.main.async { SnapshotRunner.runReading(dir: dir) }
        }
        // Content-area checks (SnapshotMode+Content.swift); returns true when it started one.
        if ContentTests.run(args) {}
        HCAppTests.run(args)
        if args.contains("--test-tabkeys") { Task { @MainActor in await ChromeTests.runTabKeysTest() } }
        if args.contains("--test-sidebar-layout") { Task { @MainActor in await ChromeTests.runSidebarLayoutTest() } }
        if args.contains("--test-newtab-live") { Task { @MainActor in await ChromeTests.runNewTabLiveTest() } }
        if args.contains("--test-settings-migration") { ChromeTests.runSettingsMigrationTest() }
        if args.contains("--test-island-height") { Task { @MainActor in await ChromeTests.runIslandHeightTest() } }
        if args.contains("--test-tab-overflow") { Task { @MainActor in await ChromeTests.runTabOverflowTest() } }
        if args.contains("--test-root-width") { Task { @MainActor in await ChromeTests.runRootWidthTest() } }
        if args.contains("--test-settings-page") { Task { @MainActor in await ChromeTests.runSettingsPageTest() } }
        if args.contains("--test-sidebar") { Task { @MainActor in await ChromeTests.runSidebarTest() } }
        if args.contains("--test-composer") { Task { @MainActor in await SnapshotRunner.runComposerTest() } }
        if args.contains("--hittest") { Task { @MainActor in await SnapshotRunner.runHitTest() } }
        if args.contains("--stress") { Task { @MainActor in await SnapshotRunner.runStress() } }
        if let i = args.firstIndex(of: "--snapshot-live"), args.indices.contains(i + 1) {
            let dir = args[i + 1], live = store
            Task { @MainActor in try? await Task.sleep(for: .seconds(4)); await SnapshotRunner.runLive(dir: dir, store: live) }
        }
        if let i = args.firstIndex(of: "--snapshot-demo"), args.indices.contains(i + 1) {
            let dir = args[i + 1], live = store
            Task { @MainActor in await SnapshotRunner.runDemo(dir: dir, store: live) }
        }
        if let i = args.firstIndex(of: "--snapshot-remote"), args.indices.contains(i + 1) {
            let dir = args[i + 1]
            Task { @MainActor in await SnapshotRunner.runRemote(dir: dir) }
        }
        if let i = args.firstIndex(of: "--snapshot-settings"), args.indices.contains(i + 1) {
            let dir = args[i + 1]
            Task { @MainActor in await SnapshotRunner.runSettings(dir: dir) }
        }
        let m = AppModel(store: store)
        m.pulledTexts = FilePulledStore(url: AppModel.pulledTextsFile(session: store.session.stateKey))
        if args.contains("--test-fullscreen-real") {
            let dir = ProcessInfo.processInfo.environment["FS_REAL_DIR"] ?? "build/fullscreen-real/latest"
            // The one check with a real window (owner's leave): an unbundled binary starts without one.
            NSApplication.shared.setActivationPolicy(.regular)
            Task { @MainActor in await FullScreenReal.run(dir: dir, model: m) }
        }
        if let secs = fuzzSeconds { Task { @MainActor in await FuzzRunner.run(seconds: secs, seed: fuzzSeed, model: m) } }
        if let i = args.firstIndex(of: "--test-approve"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1], allow = !args.contains("--deny")
            Task { @MainActor in await HeadlessTest.approve(model: m, paneId: paneId, allow: allow) }
        }
        if let i = args.firstIndex(of: "--test-notice"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1], allow = !args.contains("--deny")
            Task { @MainActor in await HeadlessTest.notice(model: m, paneId: paneId, allow: allow) }
        }
        if args.contains("--serve-remote"), store.session.name != nil {
            Task { @MainActor in await HubLiveTest.serve(model: m) }
        }
        if let i = args.firstIndex(of: "--test-remote-live"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1]
            Task { @MainActor in await RemoteLiveTest.run(model: m, paneId: paneId) }
        }
        if args.contains("--test-remote"), store.session.name != nil {
            Task { @MainActor in await HubLiveTest.remote(model: m) }
        }
        if let i = args.firstIndex(of: "--test-hub"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1], allow = !args.contains("--deny")
            let other = args.firstIndex(of: "--other").flatMap { args.indices.contains($0 + 1) ? args[$0 + 1] : nil }
            Task { @MainActor in await HubLiveTest.run(model: m, paneId: paneId, allow: allow, image: args.contains("--image"), other: other) }
        }
        if let i = args.firstIndex(of: "--test-terminal"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1]
            Task { @MainActor in await SnapshotRunner.runTerminalTest(store: store, paneId: paneId) }
        }
        if let i = args.firstIndex(of: "--test-send-image"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1]
            Task { @MainActor in await SnapshotRunner.runSendImageTest(store: store, paneId: paneId) }
        }
        if let i = args.firstIndex(of: "--test-wheel"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1]
            Task { @MainActor in await SnapshotRunner.runWheelTest(store: store, paneId: paneId) }
        }
        if args.contains("--test-panemenu") {
            Task { @MainActor in
                let ws = HerdrStore.preview.workspaces
                let tab = ws.flatMap(\.tabs).first { $0.panes.count > 1 } ?? ws[0].tabs[0]
                let pane = tab.panes[0]
                var failures = 0
                func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("panemenu-test: \(n) \(ok ? "ok" : "FAIL")") }
                let rw = PaneMenu.items(pane: pane, tab: tab, zoomed: false, routed: false, writes: true)
                let ro = PaneMenu.items(pane: pane, tab: tab, zoomed: false, routed: true, writes: false)
                step("entries: \(rw.map(\.title))", rw.map(\.kind) == [.rename, .splitRight, .splitDown, .zoom, .rightClick, .close])
                step("writable: all enabled", rw.allSatisfy(\.enabled))
                step("read-only: rename/split/right-click/close disabled", ro.filter { $0.kind != .zoom }.allSatisfy { !$0.enabled })
                step("read-only: zoom (GUI only) stays enabled on a split tab", ro.first { $0.kind == .zoom }?.enabled == true)
                step("right-click item carries the checked state", ro.first { $0.kind == .rightClick }?.checked == true && rw.first { $0.kind == .rightClick }?.checked == false)
                let single = PaneMenu.items(pane: pane, tab: Tab(id: "t", label: "x", number: 1, panes: [pane], status: .idle), zoomed: false, routed: false, writes: true)
                step("zoom disabled on a single-pane tab", single.first { $0.kind == .zoom }?.enabled == false)
                // hc (parity item 2): panes of the official app's tabs get the layout entries, no right-click routing
                // (no hc equivalent); "Other terminals" panes only zoom.
                let hp = Pane(id: "hc:t1", terminalId: "t1", cwd: "/tmp", agent: nil, backend: .hc)
                let hcTab = Tab(id: "hc:tab:sp1/tb1", label: "1", number: 1, panes: [hp, hp], status: .idle, backend: .hc)
                let other = Tab(id: "hc:term:t1", label: "x", number: 1, panes: [hp], status: .idle, backend: .hc)
                let hcItems = PaneMenu.items(pane: hp, tab: hcTab, zoomed: false, routed: false, writes: true)
                step("hc entries: \(hcItems.map(\.title))", hcItems.map(\.kind) == [.rename, .splitRight, .splitDown, .zoom, .close] && hcItems.allSatisfy(\.enabled))
                step("hc other terminals: zoom only", PaneMenu.items(pane: hp, tab: other, zoomed: false, routed: false, writes: true).map(\.kind) == [.zoom])
                let m = AppModel(store: HerdrStore.preview)
                let hcWs = Workspace(id: "hc:space:sp1", label: "w", tabs: [hcTab], backend: .hc)
                let otherWs = Workspace(id: HCMapper.otherId, label: "o", tabs: [other], backend: .hc)
                step("hc tab menu: \(m.tabMenu(hcTab).map(\.id))", m.tabMenu(hcTab).map(\.id) == ["rename", "moveLeft", "moveRight", "close", "splitRight", "splitDown", "overview"])
                step("hc other tab menu: overview only", m.tabMenu(other).map(\.id) == ["overview"])
                step("hc workspace menu", m.workspaceMenu(hcWs).map(\.id) == ["rename", "newTab", "close"] && m.workspaceMenu(otherWs).isEmpty)
                // Another machine's space (parity 3b): the same layout entries; the write goes to that machine's document.
                let rp = Pane(id: "hc:box/t3", terminalId: "box/t3", cwd: "/tmp", agent: nil, backend: .hc)
                let rTab = Tab(id: "hc:tab:box/sp1/tb1", label: "1", number: 1, panes: [rp, rp], status: .idle, backend: .hc)
                let rWs = Workspace(id: "hc:space:box/sp1", label: "r", tabs: [rTab], backend: .hc)
                step("hc remote pane: layout entries", PaneMenu.items(pane: rp, tab: rTab, zoomed: false, routed: false, writes: true).map(\.kind) == [.rename, .splitRight, .splitDown, .zoom, .close])
                step("hc remote tab/workspace: layout entries (\(m.tabMenu(rTab).map(\.id)), \(m.workspaceMenu(rWs).map(\.id)))",
                     m.tabMenu(rTab).map(\.id) == ["rename", "moveLeft", "moveRight", "close", "splitRight", "splitDown", "overview"]
                     && m.workspaceMenu(rWs).map(\.id) == ["rename", "newTab", "close"] && m.layoutOK(rWs) && m.layoutOK(rTab))
                var calls: [String] = []
                m.workspaceHook = { calls.append("\($0) \($1)") }
                m.createWorkspace(name: " Flock ", cwd: "/tmp", on: .hc)
                m.rename(hcWs, to: "Herd")
                m.split(tab: hcTab, .down)
                step("hc writes routed (\(calls))", calls.first == "create-hc Flock" && calls.contains("rename hc:space:sp1 Herd") && calls.contains { $0.hasPrefix("split hc:tab:sp1/tb1") })
                print("panemenu-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
                NSApp.terminate(nil)
            }
        }
        if args.contains("--test-menus") { Task { @MainActor in await SnapshotRunner.runMenusTest() } }
        if args.contains("--test-tabdrag") { Task { @MainActor in await SnapshotRunner.runTabDragTest() } }
        if args.contains("--test-closetab") { Task { @MainActor in await SnapshotRunner.runCloseTabTest() } }
        if let i = args.firstIndex(of: "--test-sheet"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1]
            Task { @MainActor in await SnapshotRunner.runSheetTest(store: store, paneId: paneId) }
        }
        if args.contains("--test-chrome") { Task { @MainActor in await SnapshotRunner.runChromeTest() } }
        if args.contains("--test-table") { Task { @MainActor in await SnapshotRunner.runTableTest() } }
        if args.contains("--test-bottom") { Task { @MainActor in await SnapshotRunner.runBottomTest() } }
        if let i = args.firstIndex(of: "--test-scroll"), args.indices.contains(i + 1), store.session.name != nil {
            let paneId = args[i + 1]
            Task { @MainActor in await SnapshotRunner.runScrollTest(store: store, paneId: paneId) }
        }
        // Remote access: only a real launch may listen, and only when the owner turned it on (off by default).
        if !quiet {
            let hub = RemoteHub.live(model: m)
            m.remote = hub
            if hub.enabled { Task { @MainActor in await hub.start() } }
        }
        _model = State(initialValue: m)
    }

    var body: some Scene {
        WindowGroup("Flockbook", id: "main") {
            if headless { Color.clear.frame(width: 1, height: 1) } else { appRoot }
        }
        .windowToolbarStyle(.unified)
        .defaultSize(width: 1280, height: 820)
        // A check opens no main window: even empty, it would cover what the owner is looking at.
        .defaultLaunchBehavior(headless ? .suppressed : .automatic)
        .commands {
            DeskCommands(model: model)
            UpdateCommands(updater: AppUpdater.shared)
        }
    }

    private var appRoot: some View {
        RootView()
            .environment(model)
            .frame(minWidth: 960, minHeight: 600)
            // The keyboard shortcuts panel (⌘/), over sidebar and detail, windowed and full screen.
            .shortcutsPanelHost()
    }
}

struct DeskCommands: Commands {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow
    @AppStorage(ReadingSize.key, store: AppDefaults.store) private var readingSize = ReadingSize.m.rawValue
    var body: some Commands {
        // Settings is a page of the main window (no Settings window): ⌘, shows it there, opening the window if it was closed.
        CommandGroup(replacing: .appSettings) {
            // Again while it shows: back where it was opened from.
            Button("Settings…") {
                if let w = model.mainWindow, w.isVisible { model.toggleSettings(); w.makeKeyAndOrderFront(nil) }
                else { model.showSettings(); openWindow(id: "main") }
            }
            .keyboardShortcut(",")
        }
        CommandGroup(replacing: .newItem) {
            Button("New Tab") { model.newTab() }.keyboardShortcut("t")
            Button("New Workspace…") { model.beginNewWorkspace() }.keyboardShortcut("n", modifiers: [.command, .shift]).disabled(!model.writesEnabled)
            Button("Close Tab") { model.closeTab() }.keyboardShortcut("w")
        }
        CommandGroup(after: .toolbar) {
            Button("Larger Text") { readingSize = (ReadingSize(rawValue: readingSize) ?? .m).larger().rawValue }.keyboardShortcut("=")
            Button("Smaller Text") { readingSize = (ReadingSize(rawValue: readingSize) ?? .m).smaller().rawValue }.keyboardShortcut("-")
            Button("Actual Text Size") { readingSize = ReadingSize.m.rawValue }.keyboardShortcut("0")
            Button("Toggle Sidebar") { model.toggleSidebar() }.keyboardShortcut("b")
            Button("Show Terminal") { if model.screen == .tab { model.showTerminal.toggle() } }.keyboardShortcut("`", modifiers: .control)
            Button("Show in Overview") { model.screen = .overview }.keyboardShortcut("\\", modifiers: [.command, .shift])
            Button("Search…") { model.screen = .search }.keyboardShortcut("k")
            Button("Needs You") { model.screen = .inbox }.keyboardShortcut("i", modifiers: [.command, .shift])
        }
        // The app has no Help book: the Help menu's item opens the shortcuts panel instead (the menu keeps its search field).
        CommandGroup(replacing: .help) {
            Button("Keyboard Shortcuts") { ShortcutsPanel.toggle() }.keyboardShortcut("/")
        }
        CommandMenu("Tabs") {
            Button("Zoom Pane") { model.toggleZoomFocused() }.keyboardShortcut(.return, modifiers: [.command, .shift])
            Button("Split Right") { model.splitFocused(.right) }.keyboardShortcut("d")
            Button("Split Down") { model.splitFocused(.down) }.keyboardShortcut("d", modifiers: [.command, .shift])
            Divider()
            Button("Rename Tab…") { if let t = model.tab, model.layoutOK(t) { model.screen = .tab; model.renamingTabID = t.id } }
                .disabled(!model.writesEnabled || model.tab == nil)
            Button("Move Tab Left") { if let t = model.tab { model.moveTab(t, by: -1) } }
                .keyboardShortcut(.leftArrow, modifiers: [.command, .control, .shift]).disabled(!model.writesEnabled)
            Button("Move Tab Right") { if let t = model.tab { model.moveTab(t, by: 1) } }
                .keyboardShortcut(.rightArrow, modifiers: [.command, .control, .shift]).disabled(!model.writesEnabled)
            Divider()
            Button("Next Tab") { model.cycleTab(1) }.keyboardShortcut(.tab, modifiers: .control)
            Button("Previous Tab") { model.cycleTab(-1) }.keyboardShortcut(.tab, modifiers: [.control, .shift])
            Button("Select Previous Tab") { model.cycleTab(-1) }.keyboardShortcut("[", modifiers: [.command, .shift])
            Button("Select Next Tab") { model.cycleTab(1) }.keyboardShortcut("]", modifiers: [.command, .shift])
            Button("Next Workspace") { model.cycleWorkspace(1) }.keyboardShortcut(.downArrow, modifiers: [.command, .option])
            Button("Previous Workspace") { model.cycleWorkspace(-1) }.keyboardShortcut(.upArrow, modifiers: [.command, .option])
            Divider()
            ForEach(0..<9, id: \.self) { i in
                Button("Tab \(i + 1)") { model.selectTab(index: i) }
                    .keyboardShortcut(KeyEquivalent(Character("\(i + 1)")), modifiers: .command)
            }
        }
    }
}

struct RootView: View {
    @Environment(AppModel.self) private var model
    var flat = false
    var flatSidebarWidth: CGFloat = 250
    var body: some View {
        content
            // Workspace modals live here, once: the sidebar can be on screen twice (docked and the hover peek).
            .sheet(isPresented: Binding(get: { model.newWorkspaceOpen }, set: { model.newWorkspaceOpen = $0 })) { NewWorkspaceSheet().environment(model) }
            .confirmationDialog("Close workspace “\(model.closingWorkspace?.label ?? "")”?",
                                isPresented: Binding(get: { model.closingWorkspace != nil }, set: { if !$0 { model.closingWorkspace = nil } }),
                                titleVisibility: .visible, presenting: model.closingWorkspace) { ws in
                Button("Close workspace", role: .destructive) { model.close(ws) }
                Button("Cancel", role: .cancel) { model.closingWorkspace = nil }
            } message: { ws in
                let n = ws.tabs.flatMap(\.panes).filter { $0.agent != nil }.count
                Text(n > 0 ? "This ends \(n) running agent\(n == 1 ? "" : "s") and everything else running in its \(ws.tabs.count) tab\(ws.tabs.count == 1 ? "" : "s")."
                           : "This ends everything running in its \(ws.tabs.count) tab\(ws.tabs.count == 1 ? "" : "s").")
            }
            .onAppear {
                Notifier.shared.onOpen = { ws, tab in
                    guard let w = model.store.workspaces.first(where: { $0.id == ws }),
                          let t = w.tabs.first(where: { $0.id == tab }) else { return }
                    model.select(tab: t, in: w)
                    NSApp.activate(ignoringOtherApps: true)
                }
                Notifier.shared.approvalPrompt = { tab in await model.approvalPrompt(in: tab) }
                Notifier.shared.onAnswer = { action, pane, prompt in await model.answerNotice(action: action, paneID: pane, prompt: prompt) }
                if !flat { Notifier.shared.start() }
            }
            .onChange(of: model.store.snapshots) { _, _ in model.prunePulled() }
            .onChange(of: model.store.workspaces) { _, ws in
                model.prune()
                if !flat { Notifier.shared.update(ws) }
            }
    }

    @ViewBuilder private var content: some View {
        if flat {
            HStack(spacing: 0) {
                SidebarView().frame(width: flatSidebarWidth)
                Divider()
                DetailView()
            }
            .background(WindowSurface(glass: .off))
            .environment(\.flatGlass, true)
        } else { split }
    }
    private var split: some View {
        // One surface for the whole window (WindowSurface): the window, the detail column and the sidebar column (inside the
        // system's floating panel, which clips it to the panel shape) all draw the same view, so sidebar and content match.
        GlassReader { glass in
            NavigationSplitView(columnVisibility: Bindable(model).sidebarVisibility) {
                SidebarView()
                    .navigationSplitViewColumnWidth(min: 220, ideal: 250, max: 320)
            } detail: {
                DetailView()
            }
            .containerBackground(for: .window) { WindowSurface(glass: glass) }
            .overlay(alignment: .leading) { SidebarPeek() }
            // While the system toolbar is revealed: down by what it covers beyond the band (a move, no new layout).
            .offset(y: model.revealShift)
            // Full screen looks as windowed. The system toolbar slides in only with the menu bar (.onHover): a toolbar that
            // stays visible (.visible) lives in AppKit's full-screen toolbar window, which paints an opaque band and lays the
            // content out below it. Not hidden either: without a toolbar the split view drops the sidebar's full-height layout
            // (panel top 32 instead of 8). At rest the band stays (toolbarBand) and holds our sidebar toggle (FullScreenChrome)
            // and the tab island (FullScreenTopRow); while the menu bar is down the system's band shows its lights, toggle and row.
            .windowToolbarFullScreenVisibility(.onHover)
        }
        .probe { ChromeProbe.root = $0 }   // the split's own frame (wider than the window during the slide)
        // While AppKit slides the sidebar in, the split is wider than the window for a moment (the detail keeps its full
        // width until the slide ends). SwiftUI centres an oversized child: the whole content would shift left by up to half
        // the sidebar's width and jump back at the end. Leading-aligned, the extra width runs off the right edge instead.
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
        // Full screen at rest: our sidebar toggle (no window controls: the system's come with the menu-bar reveal), and the
        // tab row where the system toolbar lays it out (that toolbar shows the same row while revealed). Outside the split
        // view: while it shows the sidebar, the split moves its overlays' space sideways for a moment (up to ~130 pt).
        .overlay(alignment: .topLeading) { if model.fullScreen { FullScreenChrome().ignoresSafeArea() } }
        .overlay(alignment: .topLeading) { if model.fullScreen, model.screen != .settings { FullScreenTopRow().ignoresSafeArea() } }
        .background(WindowFixer(model: model))
    }
}

/// Re-applies the window chrome and re-measures the traffic lights on window, split-view and full-screen changes.
@MainActor final class WindowWidthTracker {
    nonisolated(unsafe) private var observers: [NSObjectProtocol] = []
    weak var model: AppModel?
    weak var window: NSWindow?
    /// The full-screen toolbar window and its move/resize observers (full screen only).
    nonisolated(unsafe) private var revealObservers: [NSObjectProtocol] = []
    private weak var toolbarWindow: NSWindow?
    func refresh() {
        guard let window, let model else { return }
        WindowChrome.apply(window); WindowChrome.measure(window, model); WindowChrome.disableSystemEdgeReveal(window)
        Self.update(window, model)
        watchReveal()
        watchToolbarRow()
    }
    /// Re-measures where the toolbar lays out the tab row whenever its item views move (FullScreenTopRow sits there).
    nonisolated(unsafe) private var rowObservers: [NSObjectProtocol] = []
    private var rowViews: [ObjectIdentifier] = []
    private func watchToolbarRow() {
        guard let window, let model else { return }
        let views = WindowChrome.toolbarItemViews(window)
        let ids = views.map(ObjectIdentifier.init)
        if ids != rowViews {
            rowObservers.forEach(NotificationCenter.default.removeObserver); rowObservers = []
            rowViews = ids
            for v in views {
                v.postsFrameChangedNotifications = true
                rowObservers.append(NotificationCenter.default.addObserver(forName: NSView.frameDidChangeNotification, object: v, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { if let w = self?.window, let m = self?.model { WindowChrome.measureToolbarRow(w, m) } }
                })
            }
        }
        WindowChrome.measureToolbarRow(window, model)
    }
    /// Follows the system toolbar's slide with the menu bar in full screen (its window moves); FullScreenChrome steps aside
    /// and the content moves by what the reveal covers beyond the band, with the same ~0.25 s ease as the slide.
    private func watchReveal() {
        guard let window, let model else { return }
        let tw = model.fullScreen ? WindowChrome.toolbarWindow(window) : nil
        if model.toolbarDetached != (tw != nil) { model.toolbarDetached = tw != nil }
        if tw !== toolbarWindow {
            revealObservers.forEach(NotificationCenter.default.removeObserver); revealObservers = []
            toolbarWindow = tw
            if let tw {
                for n in [NSWindow.didMoveNotification, NSWindow.didResizeNotification] {
                    revealObservers.append(NotificationCenter.default.addObserver(forName: n, object: tw, queue: .main) { [weak self] _ in
                        MainActor.assumeIsolated { self?.updateReveal() }
                    })
                }
            }
        }
        updateReveal()
    }
    private func updateReveal() {
        guard let window, let model else { return }
        // No toolbar window: windowed (nothing revealed), or the full-screen layout flag of an offscreen check (left as set).
        guard let tw = toolbarWindow, model.fullScreen else { if !model.fullScreen, model.toolbarReveal != 0 { model.toolbarReveal = 0 }; return }
        let r = WindowChrome.toolbarReveal(window, toolbarWindow: tw)
        guard abs(r - model.toolbarReveal) > 0.5 else { return }
        if NSWorkspace.shared.accessibilityDisplayShouldReduceMotion { model.toolbarReveal = r }
        else { withAnimation(.easeOut(duration: 0.25)) { model.toolbarReveal = r } }
    }
    func attach(_ w: NSWindow, model: AppModel) {
        self.model = model; window = w; model.mainWindow = w
        guard observers.isEmpty else { refresh(); return }
        // Full screen moves the toolbar to another window and hides the traffic lights: re-apply and re-measure on each step.
        func fs(_ name: NSNotification.Name, _ on: Bool?) -> NSObjectProtocol {
            NotificationCenter.default.addObserver(forName: name, object: w, queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    if let on { self?.model?.fullScreen = on }
                    self?.refresh()
                }
            }
        }
        observers += [fs(NSWindow.willEnterFullScreenNotification, true), fs(NSWindow.didEnterFullScreenNotification, true),
                      fs(NSWindow.willExitFullScreenNotification, false), fs(NSWindow.didExitFullScreenNotification, false)]
        model.fullScreen = w.styleMask.contains(.fullScreen)
        let update: @Sendable (Notification) -> Void = { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
        let nc = NotificationCenter.default
        observers += [nc.addObserver(forName: NSWindow.didResizeNotification, object: w, queue: .main, using: update),
                      nc.addObserver(forName: NSSplitView.didResizeSubviewsNotification, object: nil, queue: .main, using: update)]
        refresh()
    }
    deinit { (observers + revealObservers + rowObservers).forEach(NotificationCenter.default.removeObserver) }

    static func splitView(in v: NSView?) -> NSSplitView? {
        guard let v else { return nil }
        if let s = v as? NSSplitView, s.subviews.count >= 2 { return s }
        for c in v.subviews { if let s = splitView(in: c) { return s } }
        return nil
    }
    static func update(_ w: NSWindow, _ model: AppModel) {
        if model.windowSize != (w.contentView?.bounds.size ?? .zero) { model.windowSize = w.contentView?.bounds.size ?? .zero }
        // The tab row width itself comes from the detail column's SwiftUI geometry (DetailView, AppModel.setDetailWidth);
        // this only gives that path a way to make the toolbar re-measure.
        model.toolbarNudge = { [weak w] in
            // An item that was moved to the overflow menu keeps its old size; nudge the toolbar to re-measure.
            DispatchQueue.main.async {
                w?.toolbar?.items.forEach { $0.view?.invalidateIntrinsicContentSize() }
                w?.toolbar?.validateVisibleItems()
            }
        }
    }
}

/// Keeps the window fully on a screen (a restored frame can leave the sidebar cut off at the left edge).
struct WindowFixer: NSViewRepresentable {
    let model: AppModel
    func makeNSView(context: Context) -> NSView {
        let v = NSView()
        DispatchQueue.main.async {
            Self.fix(v.window)
            if let w = v.window { context.coordinator.attach(w, model: model) }
        }
        return v
    }
    func updateNSView(_ v: NSView, context: Context) {
        context.coordinator.model = model
        DispatchQueue.main.async { context.coordinator.refresh() }
    }
    func makeCoordinator() -> WindowWidthTracker { WindowWidthTracker() }
    @MainActor static func fix(_ w: NSWindow?) {
        guard let w, let screen = w.screen ?? NSScreen.main else { return }
        let vis = screen.visibleFrame
        var f = w.frame
        f.size.width = min(f.width, vis.width); f.size.height = min(f.height, vis.height)
        f.origin.x = min(max(f.origin.x, vis.minX), vis.maxX - f.width)
        f.origin.y = min(max(f.origin.y, vis.minY), vis.maxY - f.height)
        if f != w.frame { w.setFrame(f, display: true) }
    }
}

struct DetailView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            if model.screen != .settings, !model.store.connected, model.store.workspaces.isEmpty, let problem = model.store.problem {
                ConnectionProblemView(problem: problem) { await model.store.refresh() }
            } else {
                // Settings covers the screen it was opened from and leaves it in place (hidden, inert), not rebuilt: Back
                // shows the conversation at the same place, older messages still open, the composer still focused.
                let settings = model.screen == .settings
                ZStack {
                    Group {
                        switch settings ? model.screenBeforeSettings : model.screen {
                        case .tab: TabScreen()
                        case .inbox: InboxView()
                        case .overview: OverviewView()
                        case .search: SearchView()
                        case .settings: EmptyView()
                        }
                    }
                    .environment(\.underSettings, settings)
                    .opacity(settings ? 0 : 1)
                    .allowsHitTesting(!settings)
                    .disabled(settings)
                    .accessibilityHidden(settings)
                    if settings { SettingsPage() }
                }
                .background { SettingsFocus(open: settings) }
            }
        }
        // The window's surface again: the split view's detail column paints its own system background over the window's.
        // It runs under the toolbar and the floating sidebar.
        .background { GlassReader { WindowSurface(glass: $0) }.ignoresSafeArea() }
        // Toolbar lives here so it keeps one identity across workspace and screen changes.
        // Free width for the tab row: the detail column's own size (its leading safe area is the docked sidebar).
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { model.setDetailWidth($0) }
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { ChromeProbe.detail = $0 }
        .onGeometryChange(for: CGFloat.self) { $0.safeAreaInsets.top } action: { ChromeProbe.detailTopInset = $0 }
        // Full screen: the band the hidden toolbar leaves (the tab island in it is FullScreenTopRow, on the window).
        .modifier(FullScreenBand())
        .toolbar { TopRowToolbar(model: model) }
        .toolbar(removing: .title)
        // Content fades out under the toolbar instead of showing through the tab island.
        .toolbarFade()
        .modifier(ToolbarGlass())
    }
}

/// No toolbar strip: the glass islands (tabs, actions, sidebar toggle) and the scroll edge effect separate it from content.
struct ToolbarGlass: ViewModifier {
    func body(content: Content) -> some View {
        content.toolbarBackgroundVisibility(.hidden, for: .windowToolbar)
    }
}
