import HerdrKit
import SwiftUI

/// Pane context menu, same entries as herdr: Rename / Split right / Split down / Zoom / Send right-clicks / Close.
struct PaneMenuItem: Identifiable, Equatable {
    enum Kind: Equatable { case rename, splitRight, splitDown, zoom, rightClick, close }
    let kind: Kind
    let title: String
    let enabled: Bool
    var checked: Bool? = nil
    var id: String { "\(kind)" }
}

enum PaneMenu {
    /// `writes` false (read-only mode or input setting off): everything that changes herdr is disabled.
    /// hc panes: no right-click routing (N/A on hc); "Other terminals" and remote panes: only Zoom.
    static func items(pane: Pane, tab: Tab?, zoomed: Bool, routed: Bool, writes: Bool) -> [PaneMenuItem] {
        let zoom = PaneMenuItem(kind: .zoom, title: zoomed ? "Restore panes" : "Zoom", enabled: tab?.isSplit == true)
        if pane.backend == .hc {
            guard let tab, HCClient.layoutWritable(id: tab.id) else { return [zoom] }   // Other terminals, other machines
            return [
                PaneMenuItem(kind: .rename, title: "Rename pane…", enabled: writes),
                PaneMenuItem(kind: .splitRight, title: "Split right", enabled: writes),
                PaneMenuItem(kind: .splitDown, title: "Split down", enabled: writes),
                zoom,
                PaneMenuItem(kind: .close, title: "Close pane", enabled: writes),
            ]
        }
        return [
            PaneMenuItem(kind: .rename, title: "Rename pane…", enabled: writes),
            PaneMenuItem(kind: .splitRight, title: "Split right", enabled: writes),
            PaneMenuItem(kind: .splitDown, title: "Split down", enabled: writes),
            zoom,
            PaneMenuItem(kind: .rightClick, title: "Send right-clicks to pane", enabled: writes, checked: routed),
            PaneMenuItem(kind: .close, title: "Close pane", enabled: writes),
        ]
    }
}

private struct PaneMenuModifier: ViewModifier {
    @Environment(AppModel.self) private var model
    let pane: Pane
    let tab: Tab?
    /// On a terminal with "Send right-clicks to pane" on, a right click belongs to the program: no menu there.
    var suppressWhenRouted = false

    func body(content: Content) -> some View {
        let routed = model.rightClickToPane.contains(pane.id)
        content.contextMenu {
            if !(suppressWhenRouted && routed) {
                let zoomed = tab.map { model.zoomedPaneID(in: $0) == pane.id } ?? false
                let items = PaneMenu.items(pane: pane, tab: tab, zoomed: zoomed, routed: routed, writes: model.writesEnabled)
                ForEach(items) { item in
                    if item.kind == .close { Divider() }
                    if let checked = item.checked {
                        Toggle(item.title, isOn: Binding(get: { checked }, set: { _ in model.perform(item.kind, on: pane, in: tab) }))
                            .disabled(!item.enabled)
                    } else {
                        Button(item.title, role: item.kind == .close ? .destructive : nil) { model.perform(item.kind, on: pane, in: tab) }
                            .disabled(!item.enabled)
                    }
                }
            }
        }
    }
}

extension View {
    func paneMenu(_ pane: Pane, tab: Tab?, suppressWhenRouted: Bool = false) -> some View {
        modifier(PaneMenuModifier(pane: pane, tab: tab, suppressWhenRouted: suppressWhenRouted))
    }
}

extension AppModel {
    func perform(_ kind: PaneMenuItem.Kind, on pane: Pane, in tab: Tab?) {
        if kind != .zoom, !layoutOK(pane, in: tab) { return }
        if pane.backend == .hc, kind == .rightClick { return }
        switch kind {
        case .rename:
            guard writesEnabled else { return }
            paneRenameDraft = pane.label ?? ""; renamingPane = pane
        case .splitRight, .splitDown:
            guard writesEnabled else { return }
            let dir: SplitDirection = kind == .splitRight ? .right : .down
            splitDirection = dir.rawValue
            let id = pane.id
            runLayout(pane.backend, "Split") { try await $0.split(paneId: id, direction: dir) }
        case .zoom:
            if let tab { toggleZoom(pane, in: tab) }
        case .rightClick:
            guard writesEnabled else { return }
            let on = !rightClickToPane.contains(pane.id)
            if on { rightClickToPane.insert(pane.id) } else { rightClickToPane.remove(pane.id) }
            Task { try? await store.client.setRightClickRouting(paneId: pane.id, to: on ? .pane : .herdr) }
        case .close:
            guard writesEnabled else { return }
            if pane.agent != nil { closingPane = pane } else { closePane(pane) }
        }
    }
    /// The pane menu only offers these where `layoutOK` holds (on hc: panes of the official app's tabs).
    func renamePane(_ pane: Pane, to label: String) {
        guard writesEnabled else { return }
        let id = pane.id
        runLayout(pane.backend, "Rename pane") { try await $0.renamePane(paneId: id, to: label) }
    }
    func closePane(_ pane: Pane) {
        guard writesEnabled else { return }
        let id = pane.id
        runLayout(pane.backend, "Close pane") { try await $0.closePane(paneId: id) }
    }
}
