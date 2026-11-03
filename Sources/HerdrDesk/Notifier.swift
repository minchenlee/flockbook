import HerdrKit
import Foundation
import UserNotifications

/// System notifications for tabs that become blocked. Needs a real app bundle (UNUserNotificationCenter
/// crashes in an unbundled binary), so it does nothing under `swift run`.
///
/// A permission prompt gets Allow / Deny buttons (`ApprovalNotice`): they send what the approval card sends, and only
/// while the pane still shows that prompt. A click on the notification itself opens the tab.
@MainActor
final class Notifier: NSObject, UNUserNotificationCenterDelegate {
    static let shared = Notifier()
    private var known: Set<String>?
    var onOpen: ((_ workspaceID: String, _ tabID: String) -> Void)?
    /// The pane and prompt the buttons answer, or nil for a notification without buttons. Read-only.
    var approvalPrompt: ((_ tab: Tab) async -> (pane: String, prompt: String, request: PermissionRequest?)?)?
    /// Allow / Deny chosen. Returns true when the input was sent; false opens the tab.
    var onAnswer: ((_ action: String, _ paneID: String, _ prompt: String?) async -> Bool)?
    /// Checks, previews and snapshots: no permission prompt and no notification, also when they run from the .app.
    var quiet = false
    private var available: Bool { !quiet && Bundle.main.bundleIdentifier != nil && Bundle.main.bundlePath.hasSuffix(".app") }

    func start() {
        guard available else { return }
        let c = UNUserNotificationCenter.current()
        c.delegate = self
        c.setNotificationCategories([UNNotificationCategory(identifier: ApprovalNotice.category, actions: [
            UNNotificationAction(identifier: ApprovalNotice.Action.allow.rawValue, title: "Allow", options: []),
            UNNotificationAction(identifier: ApprovalNotice.Action.deny.rawValue, title: "Deny", options: [.destructive]),
        ], intentIdentifiers: [])])
        c.requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    /// Call on every workspace update. The first call only records the current state.
    func update(_ workspaces: [Workspace]) {
        let (now, fresh) = BlockedTabs.fresh(known: known, in: workspaces)
        let isFirst = known == nil
        known = now
        guard !isFirst, available else { return }
        for (ws, t) in fresh {
            Task { @MainActor in
                let bound = await approvalPrompt?(t)
                let c = UNMutableNotificationContent()
                let text = Self.text(ws, t, bound)
                c.title = text.title
                c.subtitle = text.subtitle
                c.body = text.body
                c.sound = .default
                c.userInfo = ["ws": ws.id, "tab": t.id]
                if let bound {
                    c.categoryIdentifier = ApprovalNotice.category
                    c.userInfo["pane"] = bound.pane
                    c.userInfo["prompt"] = bound.prompt
                }
                try? await UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: "blocked-\(t.id)", content: c, trigger: nil))
            }
        }
    }

    /// An approval says what it approves (the owner may answer from the notification); else as before.
    static func text(_ ws: Workspace?, _ t: Tab, _ bound: (pane: String, prompt: String, request: PermissionRequest?)?) -> ApprovalNotice.Text {
        ApprovalNotice.text(agent: t.primaryAgent?.name ?? "", workspace: ws?.label ?? "", tab: t.label,
                            request: bound.flatMap { $0.request ?? PermissionRequest(title: "", command: "") })
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        let action = response.actionIdentifier
        guard action != UNNotificationDismissActionIdentifier,
              let ws = info["ws"] as? String, let tab = info["tab"] as? String else { return }
        let pane = info["pane"] as? String, prompt = info["prompt"] as? String
        await handle(action: action, ws: ws, tab: tab, pane: pane, prompt: prompt)
    }

    /// Allow / Deny answer the prompt when it is still on screen; anything else (a click, a stale prompt) opens the tab.
    private func handle(action: String, ws: String, tab: String, pane: String?, prompt: String?) async {
        if ApprovalNotice.Action(rawValue: action) != nil, let pane, await onAnswer?(action, pane, prompt) == true { return }
        onOpen?(ws, tab)
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }
}
