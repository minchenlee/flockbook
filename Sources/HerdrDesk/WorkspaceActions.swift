import AppKit
import HerdrKit
import SwiftUI

/// One entry of a right-click menu. The menu views and the offscreen tests both read this list, so what the test sees is what the owner gets.
struct MenuEntry: Identifiable {
    let id: String
    let title: String
    let symbol: String
    var enabled = true
    var destructive = false
    let run: () -> Void
}

struct MenuEntries: View {
    let entries: [MenuEntry]
    var body: some View {
        ForEach(entries) { e in
            Button(role: e.destructive ? .destructive : nil, action: e.run) { Label(e.title, systemImage: e.symbol) }
                .disabled(!e.enabled)
        }
    }
}

extension AppModel {
    func tabMenu(_ t: Tab) -> [MenuEntry] {
        let w = writesEnabled
        let overview = MenuEntry(id: "overview", title: "Show in Overview", symbol: "square.grid.2x2") { [self] in screen = .overview }
        if !layoutOK(t) { return [overview] }   // hc "Other terminals": not in the official app's layout
        return [
            MenuEntry(id: "rename", title: "Rename…", symbol: "pencil", enabled: w) { [self] in renamingTabID = t.id },
            MenuEntry(id: "moveLeft", title: "Move left", symbol: "arrow.left", enabled: w && tabIndex(t).map { $0 > 0 } == true) { [self] in moveTab(t, by: -1) },
            MenuEntry(id: "moveRight", title: "Move right", symbol: "arrow.right",
                      enabled: w && tabIndex(t).map { $0 < (workspace?.tabs.count ?? 0) - 1 } == true) { [self] in moveTab(t, by: 1) },
            MenuEntry(id: "close", title: "Close tab", symbol: "xmark", enabled: w, destructive: true) { [self] in closeTab(t) },
            MenuEntry(id: "splitRight", title: "Split right", symbol: "rectangle.split.2x1", enabled: w) { [self] in split(tab: t, .right) },
            MenuEntry(id: "splitDown", title: "Split down", symbol: "rectangle.split.1x2", enabled: w) { [self] in split(tab: t, .down) },
            overview,
        ]
    }

    /// Keyboard and menu path of drag reordering: one place left (-1) or right (+1) in the current workspace.
    func moveTab(_ t: Tab, by d: Int) {
        guard let i = tabIndex(t), let n = workspace?.tabs.count, (0..<n).contains(i + d) else { return }
        moveTab(t, to: i + d)
    }
    private func tabIndex(_ t: Tab) -> Int? { workspace?.tabs.firstIndex { $0.id == t.id } }

    func workspaceMenu(_ ws: Workspace) -> [MenuEntry] {
        let w = writesEnabled
        if !layoutOK(ws) { return [] }
        return [
            MenuEntry(id: "rename", title: "Rename…", symbol: "pencil", enabled: w) { [self] in renamingWorkspaceID = ws.id },
            MenuEntry(id: "newTab", title: "New tab in workspace", symbol: "square.and.pencil", enabled: w) { [self] in newTab(in: ws) },
            MenuEntry(id: "close", title: "Close workspace…", symbol: "trash", enabled: w, destructive: true) { [self] in if writesEnabled { closingWorkspace = ws } },
        ]
    }

    func split(tab t: Tab, _ dir: SplitDirection) {
        guard writesEnabled, layoutOK(t), let fid = focusedPaneID(in: t) else { return }
        splitDirection = dir.rawValue
        if let hook = workspaceHook { hook("split", "\(t.id) \(dir.rawValue)"); return }
        runLayout(t.backend, "Split") { try await $0.split(paneId: fid, direction: dir) }
    }

    /// Moves the tab to `index` (final 0-based position) in its workspace.
    func moveTab(_ t: Tab, to index: Int) {
        guard writesEnabled, layoutOK(t) else { return }
        if let hook = workspaceHook { hook("moveTab", "\(t.id) \(index)"); return }
        let id = t.id
        runLayout(t.backend, "Move tab") { try await $0.moveTab(id, toIndex: index) }
    }

    func beginNewWorkspace() { if writesEnabled { newWorkspaceOpen = true } }

    /// Where a new workspace goes when the sheet does not say: the TUI when it is connected, else herdr-core.
    var defaultNewWorkspaceBackend: BackendKind { store.tuiConnected || !store.hcConnected ? .tui : .hc }

    /// Creates the workspace (one tab, one shell pane), then selects it here.
    func createWorkspace(name: String, cwd: String, on backend: BackendKind? = nil) {
        guard writesEnabled else { return }
        let label = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let b = backend ?? defaultNewWorkspaceBackend
        if let hook = workspaceHook { hook(b == .hc ? "create-hc" : "create", label); return }
        guard let l = store.layout(b) else { return }
        Task {
            do {
                let id = try await l.createWorkspace(label: label.isEmpty ? nil : label, cwd: cwd.isEmpty ? nil : cwd, focus: false)
                await store.refresh()
                select(workspace: id)
            } catch {
                notice = "New workspace failed: \(error)"
                await store.refresh()
            }
        }
    }

    func rename(_ ws: Workspace, to label: String) {
        renamingWorkspaceID = nil
        let t = label.trimmingCharacters(in: .whitespacesAndNewlines)
        guard writesEnabled, layoutOK(ws), !t.isEmpty, t != ws.label else { return }
        if let hook = workspaceHook { hook("rename", "\(ws.id) \(t)"); return }
        let id = ws.id
        runLayout(ws.backend, "Rename workspace") { try await $0.renameWorkspace(id: id, to: t) }
    }

    func newTab(in ws: Workspace) {
        guard writesEnabled, layoutOK(ws) else { return }
        if let hook = workspaceHook { hook("newTab", ws.id); return }
        let cwd = ws.tabs.first?.panes.first?.cwd, id = ws.id
        select(workspace: ws.id)
        runLayout(ws.backend, "New tab") { try await $0.createTab(workspaceId: id, label: nil, cwd: cwd) }
    }

    func close(_ ws: Workspace) {
        closingWorkspace = nil
        guard writesEnabled, layoutOK(ws) else { return }
        if let hook = workspaceHook { hook("close", ws.id); return }
        if workspaceID == ws.id, let other = store.workspaces.first(where: { $0.id != ws.id }) { select(workspace: other.id) }
        let id = ws.id
        runLayout(ws.backend, "Close workspace") { try await $0.closeWorkspace(id: id) }
    }
}

/// "New workspace" sheet: a name (optional, herdr names it otherwise) and a start folder (home by default).
/// A remote session (`HerdrSession.remote`) takes a host path as text instead (no panel on this Mac's disk); empty:
/// herdr's default folder on the host.
struct NewWorkspaceSheet: View {
    @Environment(AppModel.self) private var model
    @State private var name = ""
    @State private var folder = NSHomeDirectory()
    @State private var hostFolder = ""
    @State private var backend: BackendKind?
    @FocusState private var focused: Bool

    /// The herdr session's host, for a workspace on the herdr server; an hc workspace's folder is on this Mac.
    private var remote: RemoteHost? { (backend ?? model.defaultNewWorkspaceBackend) == .hc ? nil : model.store.session.remote }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("New workspace").font(.headline)
            TextField("Name", text: $name).textFieldStyle(.roundedBorder).focused($focused).onSubmit(create)
            if let remote {
                TextField("Folder on \(remote.label) (empty: herdr's default)", text: $hostFolder)
                    .textFieldStyle(.roundedBorder).onSubmit(create)
            } else {
                HStack {
                    Text(Self.pretty(folder)).font(.callout).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                    Spacer()
                    Button("Choose…") { choose() }
                }
            }
            // Both runtimes connected: say where the workspace goes.
            if model.store.tuiConnected && model.store.hcConnected {
                Picker("Runs on", selection: Binding(get: { backend ?? model.defaultNewWorkspaceBackend }, set: { backend = $0 })) {
                    Text("Herdr TUI").tag(BackendKind.tui)
                    Text("herdr-core").tag(BackendKind.hc)
                }
                .pickerStyle(.segmented)
            }
            HStack {
                Spacer()
                Button("Cancel") { model.newWorkspaceOpen = false }.keyboardShortcut(.cancelAction)
                Button("Create", action: create).keyboardShortcut(.defaultAction).disabled(!model.writesEnabled)
            }
        }
        .padding(20).frame(width: 380)
        .onAppear { focused = true }
    }

    private func create() {
        model.newWorkspaceOpen = false
        model.createWorkspace(name: name, cwd: remote == nil ? folder : hostFolder.trimmingCharacters(in: .whitespacesAndNewlines), on: backend)
    }

    private func choose() {
        let p = NSOpenPanel()
        p.canChooseDirectories = true; p.canChooseFiles = false; p.allowsMultipleSelection = false
        p.directoryURL = URL(fileURLWithPath: folder)
        if p.runModal() == .OK, let u = p.url { folder = u.path }
    }

    static func pretty(_ path: String) -> String {
        let home = NSHomeDirectory()
        return path == home ? "~" : (path.hasPrefix(home + "/") ? "~" + path.dropFirst(home.count) : path)
    }
}
