import HerdrKit
import Foundation
import AppKit

/// `--session <named> --test-approve <paneId> [--deny]`: exercises checkPrompt + approve without a window.
/// Refuses to run on the default session.
@MainActor
enum HeadlessTest {
    static func approve(model: AppModel, paneId: String, allow: Bool) async {
        try? await Task.sleep(for: .seconds(3))
        await model.store.refresh()
        guard let pane = model.store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId }) else {
            print("test: pane \(paneId) not found"); NSApp.terminate(nil); return
        }
        print("test: agent status = \(pane.agent?.status.rawValue ?? "none")")
        await model.checkPrompt(for: pane)
        print("test: promptVisible = \(model.promptVisible.contains(pane.id)) writes = \(model.writesEnabled)")
        model.approve(pane: pane, allow: allow)
        try? await Task.sleep(for: .seconds(4))
        let after = model.store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId })
        print("test: after status = \(after?.agent?.status.rawValue ?? "none") notice = \(model.notice ?? "-")")
        NSApp.terminate(nil)
    }

    /// `--session <named> --test-notice <paneId> [--deny]`: the notification buttons' path without the notification
    /// center (it needs a bundle): the bound prompt, then an action with a stale prompt id (must send nothing), then the
    /// real action (must send the card's input).
    static func notice(model: AppModel, paneId: String, allow: Bool) async {
        try? await Task.sleep(for: .seconds(3))
        await model.store.refresh()
        guard let tab = model.store.workspaces.flatMap(\.tabs).first(where: { $0.panes.contains { $0.id == paneId } }) else {
            print("test: pane \(paneId) not found"); NSApp.terminate(nil); return
        }
        // `--warm`: read the pane's log first, as the app does once the tab was open (the store tails a log from its first use).
        if CommandLine.arguments.contains("--warm"), let sid = tab.panes.first(where: { $0.id == paneId })?.agent?.sessionId {
            _ = model.store.conversation(for: sid)
            for _ in 0..<50 where model.store.conversationState(for: sid) == .loading { try? await Task.sleep(for: .milliseconds(100)) }
            print("test: log warmed, \(model.store.conversation(for: sid).count) items")
        }
        let bound = await model.approvalPrompt(in: tab)
        print("test: buttons = \(bound.map { "pane \($0.pane) prompt \($0.prompt)" } ?? "none")")
        let ws = model.store.workspaces.first { $0.tabs.contains { $0.id == tab.id } }
        let text = Notifier.text(ws, tab, bound)
        print("test: notification title = \(text.title.debugDescription) subtitle = \(text.subtitle.debugDescription) body = \(text.body.debugDescription)")
        if let pane = tab.panes.first(where: { $0.id == paneId }) {
            await model.checkPrompt(for: pane)
            let card = model.approvalRequest(pane)
            print("test: card = \(card.map { "\($0.title) | \($0.command) | \($0.toolUseId)" } ?? "none") canRespond = \(model.promptVisible.contains(pane.id)) same id = \(card?.toolUseId == bound?.prompt)")
        }
        guard let bound else { NSApp.terminate(nil); return }
        // The pane's own backend (hc or TUI), as the buttons use.
        let screen: any PaneScreen = model.screen(paneId: bound.pane) ?? model.store.client
        let action = (allow ? ApprovalNotice.Action.allow : .deny).rawValue
        let stale = await model.answerNotice(action: action, paneID: bound.pane, prompt: "toolu_stale")
        try? await Task.sleep(for: .seconds(1))
        let still = await screen.checkPrompt(paneId: bound.pane)
        print("test: stale prompt id -> sent = \(stale), prompt still on screen = \(still)")
        let sent = await model.answerNotice(action: action, paneID: bound.pane, prompt: bound.prompt)
        try? await Task.sleep(for: .seconds(4))
        let after = model.store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId })
        let gone = await !screen.checkPrompt(paneId: bound.pane)
        print("test: \(allow ? "allow" : "deny") -> sent = \(sent), prompt gone = \(gone), status = \(after?.agent?.status.rawValue ?? "none") notice = \(model.notice ?? "-")")
        NSApp.terminate(nil)
    }
}
