import HerdrKit
import SwiftUI

enum AppSettings { static let allowInputKey = "allowInput" }

/// One tab of Settings, like a pane of System Settings: the tab's icon, title and one line on top, then groups of compact
/// rows (a title over each group, one line per row: the name on the left, a value and the control on the right). What a
/// row's control does in more words goes in a line under its group. The page keeps to the left beside the tab list, at
/// most 760 pt wide. The tabs and the rows each holds: `SettingsTab` (HerdrKit). Shown beside the tab list (SettingsSplit).
struct SettingsView: View {
    var tab: SettingsTab = .general
    @AppStorage(AppSettings.allowInputKey, store: AppDefaults.store) private var allowInput = true
    @AppStorage(WindowGlass.key, store: AppDefaults.store) private var glass = WindowGlass.full.rawValue
    @Environment(\.accessibilityReduceTransparency) private var systemReduce
    /// Snapshots only: stands in for the system's Reduce Transparency.
    var reduceTransparency: Bool? = nil
    private var reduce: Bool { reduceTransparency ?? systemReduce }
    @AppStorage(ReadingWidth.key, store: AppDefaults.store) private var readingWidth = ReadingWidth.medium.rawValue
    @AppStorage(ReadingSize.key, store: AppDefaults.store) private var readingSize = ReadingSize.m.rawValue
    @AppStorage(TerminalColors.key, store: AppDefaults.store) private var terminalColors = TerminalColors.flockbook.rawValue
    @AppStorage(PlusAction.key, store: AppDefaults.store) private var plusAction = PlusAction.newTab.rawValue
    private var readOnlyLaunch: Bool { CommandLine.arguments.contains("--read-only") }
    /// Nil: no Remote access section (snapshots of the other settings).
    var remote: RemoteHub? = nil
    /// Claude Code's and Grok Build's status line switches; snapshots get a preview that never reads or writes.
    var statusLine: StatusLineSetup = .previewOff
    var grokStatusLine: StatusLineSetup = .grokPreviewOff
    /// The Settings page: fills its column and scrolls, so a long page (Remote access) never runs off the screen.
    /// Snapshots keep the full height at a fixed width.
    var scrolls = false
    /// Snapshots only: the Tailscale checks open as after a click on Details.
    var stepDetails = false
    /// The session the window shows (Machine asks for a relaunch when the choice differs); nil in snapshots.
    var shown: HerdrSession? = nil
    /// The running store, for Backend (a new choice applies at once); nil in snapshots.
    var store: HerdrStore? = nil
    /// Widest the rows get, and their distance from the tab list.
    static let column: CGFloat = 760
    static let margin: CGFloat = 40

    var body: some View {
        if scrolls {
            ScrollView { page }
                .scrollContentBackground(.hidden)
                .frame(minHeight: 360, idealHeight: 640, maxHeight: .infinity)
        } else {
            page.frame(width: 520).fixedSize(horizontal: false, vertical: true)
        }
    }

    private var page: some View {
        VStack(alignment: .leading, spacing: 6) {
            header
            switch tab {
            case .general: general
            case .agents: agents
            case .remote: if let remote { RemoteAccessSection(hub: remote, details: stepDetails) }
            case .updates: updates
            }
        }
        .frame(maxWidth: Self.column, alignment: .leading)
        .padding(.leading, Self.margin).padding(.trailing, scrolls ? Self.margin : 24)
        .padding(.top, 16).padding(.bottom, 24)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The tab's icon, its title and one line on what it is for.
    private var header: some View {
        HStack(spacing: 12) {
            Image(systemName: tab.symbol).font(.system(size: 15, weight: .medium)).foregroundStyle(.white)
                .frame(width: 32, height: 32)
                .background(RoundedRectangle(cornerRadius: 8, style: .continuous).fill(Color.accentColor))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text(tab.title).font(.system(size: 20, weight: .bold)).accessibilityAddTraits(.isHeader)
                Text(tab.summary).font(.system(size: 12)).foregroundStyle(.secondary)
            }
        }
        .padding(.bottom, 4)
        .probe { SettingsProbe.header = $0 }
        .onDisappear { SettingsProbe.header = .zero }
    }

    @ViewBuilder private var general: some View {
        RemoteMachinesSection(shown: shown)
        BackendSection(store: store)
        SettingsGroup("Appearance", note: reduce ? "Opaque while Reduce Transparency is on in System Settings." : "Glass lets the desktop show through, blurred.") {
            SettingsRow("Window") {
                Picker("Window", selection: reduce ? .constant(WindowGlass.off.rawValue) : $glass) {
                    ForEach(WindowGlass.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .pickerStyle(.segmented).labelsHidden().fixedSize()
                .disabled(reduce)
            }
            .settingsControl(.window)
        }
        SettingsGroup("Reading", note: "Text size: ⌘= larger, ⌘− smaller, ⌘0 reset. Width: \(widthNote)") {
            SettingsRow("Text size") {
                Picker("Text size", selection: $readingSize) {
                    ForEach(ReadingSize.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .pickerStyle(.segmented).labelsHidden().fixedSize()
            }
            .settingsControl(.textSize)
            SettingsRow("Width") {
                Picker("Width", selection: $readingWidth) {
                    ForEach(ReadingWidth.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .pickerStyle(.segmented).labelsHidden().fixedSize()
            }
            .settingsControl(.readingWidth)
        }
        let colors = TerminalColors(rawValue: terminalColors) ?? .flockbook, themes = TerminalThemes.shared
        // The theme in use, or why not, goes in the line under the group: a reason does not fit in the row.
        let problem = themes.problem(colors), value = problem ?? themes.names(colors)
        let note = problem ?? themes.names(colors).map { "Ghostty theme: \($0). Reload after you change it in Ghostty." }
            ?? "Ghostty theme: your Ghostty config's theme, its own background too."
        SettingsGroup("Terminal", note: note, noteColor: problem == nil ? nil : .orange) {
            SettingsRow("Colors") {
                if colors == .ghostty { Button("Reload") { themes.reload() }.controlSize(.small) }
                Picker("Colors", selection: $terminalColors) {
                    ForEach(TerminalColors.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .pickerStyle(.segmented).labelsHidden().fixedSize()
            }
            .settingsControl(.terminalColors)
            .onChange(of: value, initial: true) { _, v in SettingsProbe.terminalColors = v }
        }
        SettingsGroup("Toolbar", note: "A split splits the focused pane. ⌘T, the menu's New Tab and the sidebar's New tab always open a tab.") {
            SettingsRow("+ button") {
                Picker("+ button", selection: $plusAction) {
                    ForEach(PlusAction.allCases) { Text($0.label).tag($0.rawValue) }
                }
                .pickerStyle(.segmented).labelsHidden().fixedSize()
            }
            .settingsControl(.plusButton)
        }
    }

    @ViewBuilder private var agents: some View {
        SettingsGroup("Input") {
            SettingsRow("Allow sending input to agents") {
                Toggle("Allow sending input to agents", isOn: $allowInput).toggleStyle(.switch).controlSize(.small).labelsHidden()
            }
            .settingsControl(.allowInput)
        } footer: {
            Label {
                Text(readOnlyLaunch ? "Launched with --read-only: the app only reads, whatever this says."
                     : allowInput ? "Typing, Allow and Deny, new tab, close tab and split. Flockbook types into your agents' terminals. Turn this off to only read."
                                  : "Read only: typing, Allow and Deny, new tab, close tab and split are disabled.")
            } icon: {
                Image(systemName: allowInput && !readOnlyLaunch ? "exclamationmark.triangle.fill" : "lock.fill")
                    .foregroundStyle(allowInput && !readOnlyLaunch ? .orange : .secondary)
            }
        }
        StatusLineRow(setup: statusLine)
        StatusLineRow(setup: grokStatusLine)
        // The question hook serves the phone: shown while Remote access is on, as before.
        if let remote, remote.enabled {
            ClaudeHookRow(setup: remote.claudeHook)
        }
    }

    @ViewBuilder private var updates: some View {
        let updater = AppUpdater.shared.updater != nil
        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "?"
        SettingsGroup("Updates", note: updater ? "Once an hour. Flockbook asks before it installs an update." : nil) {
            if updater {
                SettingsRow("Check for updates automatically") {
                    Toggle("Check for updates automatically", isOn: Binding(get: { AppUpdater.shared.automaticallyChecks },
                                                                           set: { AppUpdater.shared.automaticallyChecks = $0 }))
                        .toggleStyle(.switch).controlSize(.small).labelsHidden()
                }
                .settingsControl(.autoUpdate)
            }
            SettingsRow("Version \(version)", value: AppUpdater.shared.updater?.lastUpdateCheckDate.map { "Last checked \($0.formatted(.relative(presentation: .named)))" }
                        // A build from source or a check run: Sparkle is off (UpdaterGate), so the tab says why it has no switch.
                        ?? (updater ? nil : "This build does not update itself.")) {
                if updater {
                    Button("Check Now") { AppUpdater.shared.checkForUpdates() }.disabled(!AppUpdater.shared.canCheck)
                }
            }
            .settingsControl(.version)
        }
    }

    private var widthNote: String {
        switch ReadingWidth(rawValue: readingWidth) ?? .medium {
        case .full: "the text fills the window."
        case let w: "a text column up to \(Int(w.points)) pt."
        }
    }
}

/// A group of Settings rows: its title above, the rows on one rounded fill with a hairline between them, and a line of
/// small text under it (what the rows do in more words).
struct SettingsGroup<Content: View, Footer: View>: View {
    let title: String
    @ViewBuilder var content: Content
    @ViewBuilder var footer: Footer

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.system(size: 12, weight: .semibold)).foregroundStyle(.secondary)
                .padding(.horizontal, 4).padding(.top, 8)
                .accessibilityAddTraits(.isHeader)
            VStack(spacing: 0) {
                Group(subviews: content) { rows in
                    ForEach(rows) { row in
                        if row.id != rows.first?.id { Divider().padding(.leading, 14) }
                        row
                    }
                }
            }
            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(.quinary))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(.separator.opacity(0.6), lineWidth: 0.5))
            footer
                .font(.system(size: 12)).foregroundStyle(.secondary)
                .padding(.horizontal, 4)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

extension SettingsGroup where Footer == SettingsNote {
    init(_ title: String, note: String? = nil, noteColor: Color? = nil, @ViewBuilder content: () -> Content) {
        self.init(title: title, content: content, footer: { SettingsNote(text: note, color: noteColor) })
    }
}

extension SettingsGroup {
    init(_ title: String, @ViewBuilder content: () -> Content, @ViewBuilder footer: () -> Footer) {
        self.init(title: title, content: content, footer: footer)
    }
}

/// The small line under a group; nothing when there is no text.
struct SettingsNote: View {
    let text: String?
    var color: Color? = nil
    var body: some View {
        if let text { Text(text).foregroundStyle(color.map(AnyShapeStyle.init) ?? AnyShapeStyle(.secondary)) }
    }
}

/// One compact Settings row: the name on the left; a value (secondary, one line) and the control on the right.
struct SettingsRow<Label: View, Control: View>: View {
    @ViewBuilder var label: Label
    @ViewBuilder var control: Control

    var body: some View {
        HStack(spacing: 12) {
            // The label takes the room the control leaves: a value in it sits right next to the control.
            HStack(spacing: 12) { label }.frame(maxWidth: .infinity, alignment: .leading)
            control
        }
        .padding(.horizontal, 14).padding(.vertical, 6)
        .frame(minHeight: 40)
    }
}

extension SettingsRow where Label == SettingsRowLabel {
    init(_ title: String, value: String? = nil, valueColor: Color? = nil, @ViewBuilder control: () -> Control) {
        self.init(label: { SettingsRowLabel(title: title, value: value, valueColor: valueColor) }, control: control)
    }
}

/// A row's name, then its value right-aligned before the control (cut in the middle when it does not fit; the whole
/// value in the tooltip).
struct SettingsRowLabel: View {
    let title: String
    var value: String?
    var valueColor: Color?

    var body: some View {
        Text(title).lineLimit(1).layoutPriority(1)
        if let value {
            Spacer(minLength: 8)
            Text(value).lineLimit(1).truncationMode(.middle)
                .foregroundStyle(valueColor.map(AnyShapeStyle.init) ?? AnyShapeStyle(.secondary))
                .help(value)
        }
    }
}

/// Settings: the tab list on the left, the chosen tab's form on the right, like System Settings. Opens on the tab
/// chosen last (`SettingsTab.key`). `fixed`: snapshots show that tab and leave the remembered one alone. `list` false: the
/// form alone (the page while the window's sidebar holds the tabs); `back`: a Back row above the tabs.
struct SettingsSplit: View {
    @AppStorage(SettingsTab.key, store: AppDefaults.store) private var stored = SettingsTab.general.rawValue
    var fixed: SettingsTab? = nil
    var remote: RemoteHub? = nil
    var statusLine: StatusLineSetup = .previewOff
    var grokStatusLine: StatusLineSetup = .grokPreviewOff
    var scrolls = false
    var reduceTransparency: Bool? = nil
    var list = true
    var back: (() -> Void)? = nil
    var shown: HerdrSession? = nil
    var store: HerdrStore? = nil
    static let listWidth: CGFloat = 196

    private var tab: SettingsTab { fixed ?? SettingsTab(stored: stored) }

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            if list {
                SettingsTabList(selection: Binding(get: { tab }, set: { if fixed == nil { stored = $0.rawValue } }), back: back)
                    .frame(width: Self.listWidth)
                    .frame(maxHeight: .infinity, alignment: .top)
                    .probe { SettingsProbe.pageList = $0 }
                    .onDisappear { SettingsProbe.pageList = .zero }
                Divider()
            }
            SettingsView(tab: tab, reduceTransparency: reduceTransparency, remote: remote, statusLine: statusLine, grokStatusLine: grokStatusLine,
                         scrolls: scrolls, shown: shown, store: store)
                .frame(maxWidth: scrolls ? .infinity : nil, maxHeight: scrolls ? .infinity : nil, alignment: .top)
                .probe { SettingsProbe.content = $0 }
                .id(tab)
        }
        .frame(minHeight: scrolls ? nil : 420, alignment: .top)
    }
}

/// The tabs as the window sidebar draws them (`SettingsSidebar`): the line symbol and the name, the chosen row on a light
/// grey fill. Shown in the page only while the window sidebar is hidden.
struct SettingsTabList: View {
    @Binding var selection: SettingsTab
    var back: (() -> Void)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if let back {
                Button(action: back) {
                    SidebarLine(title: "Back", symbol: "chevron.left").padding(.horizontal, 8).frame(height: 30)
                }
                .buttonStyle(.plain)
                .help("Back (Esc)")
                .probe { SettingsProbe.back = $0 }
                .onDisappear { SettingsProbe.back = .zero }
                .padding(.bottom, 6)
            }
            Text("Settings").font(.subheadline.weight(.semibold)).foregroundStyle(.secondary)
                .padding(.horizontal, 8).padding(.bottom, 2)
                .accessibilityAddTraits(.isHeader)
            ForEach(SettingsTab.allCases) { t in
                let on = t == selection
                Button { selection = t } label: {
                    SidebarLine(title: t.title, symbol: t.symbol)
                        .padding(.horizontal, 8).frame(maxWidth: .infinity, minHeight: 30, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 7, style: .continuous).fill(on ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear)))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(on ? .isSelected : [])
                .probe { SettingsProbe.tabs[t] = $0 }
            }
        }
        .padding(.horizontal, 10).padding(.top, 12)
    }
}

/// Settings as a page of the main window (the sidebar's foot row, ⌘,): the form fills the detail column, and the window's
/// sidebar holds Back and the tabs (`SettingsSidebar`). With the sidebar hidden the page brings that list itself. The page
/// scrolls, so the wheel works over its margin too, and its rows keep to the left at a readable width. Esc goes back.
struct SettingsPage: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let own = model.sidebarVisibility == .detailOnly
        SettingsSplit(remote: model.remote, statusLine: .live, grokStatusLine: .grokLive, scrolls: true, list: own, back: own ? { model.closeSettings() } : nil,
                      shown: model.store.session, store: model.store)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background { Button("Back") { model.closeSettings() }.keyboardShortcut(.cancelAction).hidden() }
            .probe { SettingsProbe.page = $0 }
            .onDisappear { SettingsProbe.page = .zero }
    }
}

extension View {
    /// Marks a Settings row: checks find each control's frame (and so its tab) in `SettingsProbe.controls`.
    func settingsControl(_ c: SettingsControl) -> some View {
        probe { SettingsProbe.controls[c] = $0 }
            .onDisappear { SettingsProbe.controls[c] = nil }
    }
}

private struct UnderSettingsKey: EnvironmentKey { static let defaultValue = false }
extension EnvironmentValues {
    /// The screen is covered by the Settings page (hidden, inert, kept for Back).
    var underSettings: Bool { get { self[UnderSettingsKey.self] } set { self[UnderSettingsKey.self] = newValue } }
}

/// Keys while Settings shows go to Settings, not to the hidden screen under it: opening it takes the window's first
/// responder (the composer, say) away and keeps it; Back gives that same responder its focus again. While it shows, a
/// typing view of the hidden screen that takes the focus anyway (a fresh session focuses its composer, a focused pane
/// its terminal, Tab walks the key views) loses it again at once: keys typed there would go to an agent no one sees.
/// Those views are our own AppKit ones (`.disabled` keeps SwiftUI's controls from taking the focus): each marks itself
/// with `cover(_:_:)` from `underSettings`. Nothing else loses the focus: the page's own controls, the ⌘/ panel's
/// search field, sheets.
/// A SwiftUI List's table under the Settings page: marked like our own views. Put in the list's rows: the row's views
/// are inside the table, so the table is the first one up its superviews.
struct CoveredList: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView { NSView() }
    func updateNSView(_ view: NSView, context: Context) {
        let on = context.environment.underSettings
        // After this update: a new row is in its table by then.
        DispatchQueue.main.async { [weak view] in
            var v = view?.superview
            while let cur = v, !(cur is NSTableView) { v = cur.superview }
            if let table = v { SettingsFocus.cover(table, on) }
        }
    }
}

struct SettingsFocus: NSViewRepresentable {
    let open: Bool
    final class Coordinator { weak var saved: NSResponder?; var open = false; var watch: NSKeyValueObservation? }
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeNSView(context: Context) -> NSView { NSView() }
    private static let covered = NSHashTable<NSView>.weakObjects()
    /// Marks a typing view of the hidden screen while Settings covers it. Called from its representable's updates.
    static func cover(_ view: NSView, _ on: Bool) { on ? covered.add(view) : covered.remove(view) }
    /// The responder is (in) a covered view. A field editor counts as the field it edits.
    static func covered(_ r: NSResponder?) -> Bool {
        var v = r as? NSView
        if let t = v as? NSText, t.isFieldEditor { v = t.delegate as? NSView }
        while let cur = v { if covered.contains(cur) { return true }; v = cur.superview }
        return false
    }
    func updateNSView(_ view: NSView, context: Context) {
        let c = context.coordinator
        guard open != c.open else { return }
        c.open = open
        guard let win = view.window else { return }
        if open {
            // A text field's own view, not the window's field editor: the editor leaves when editing ends.
            let r = win.firstResponder
            c.saved = r === win ? nil : (r as? NSText).flatMap { $0.isFieldEditor ? $0.delegate as? NSResponder : nil } ?? r
            win.makeFirstResponder(nil)
            c.watch = win.observe(\.firstResponder, options: [.new]) { [weak c] win, _ in
                MainActor.assumeIsolated {
                    guard c?.open == true, Self.covered(win.firstResponder) else { return }
                    win.makeFirstResponder(nil)
                }
            }
            return
        }
        c.watch?.invalidate(); c.watch = nil
        if let saved = c.saved {
            c.saved = nil
            // After this update: the screen under Settings is enabled again first. SwiftUI can apply that screen's last
            // update later and drop the focus to the window: while the page stays closed, give it back for a short
            // while, but only when nothing has the focus (a click elsewhere keeps its view).
            func give(_ step: Int) {
                DispatchQueue.main.asyncAfter(deadline: .now() + (step == 0 ? 0 : 0.1)) { [weak win, weak saved] in
                    guard let win, let saved, !c.open, (saved as? NSView)?.window === win else { return }
                    if step == 0 || win.firstResponder === win { win.makeFirstResponder(saved) }
                    if step < 5 { give(step + 1) }
                }
            }
            give(0)
        }
    }
}
