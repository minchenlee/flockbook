import Foundation

/// The Settings page's tabs, in sidebar order, like System Settings: which herdr the app shows and what it looks like
/// (General), what Flockbook may do to the agents (Agents), the phone (Remote), and updates. Each control of the page lives in exactly one tab.
public enum SettingsTab: String, CaseIterable, Identifiable, Sendable {
    case general, agents, remote, updates

    /// The defaults key of the tab Settings shows when it opens: the one chosen last.
    public static let key = "settingsTab"

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .general: "General"
        case .agents: "Agents"
        case .remote: "Remote"
        case .updates: "Updates"
        }
    }

    /// SF Symbol of the tab's icon.
    public var symbol: String {
        switch self {
        case .general: "gearshape"
        case .agents: "terminal"
        case .remote: "iphone"
        case .updates: "arrow.down.circle"
        }
    }

    /// The controls the tab shows, top to bottom. Some show only in a state (Connect with: Remote access on).
    public var controls: [SettingsControl] {
        switch self {
        case .general: [.machines, .backend, .window, .textSize, .readingWidth, .terminalColors, .plusButton]
        case .agents: [.allowInput, .statusLine, .grokStatusLine, .claudeQuestions]
        case .remote: [.remoteAccess, .connectWith, .tailscaleSteps, .address, .devices, .pictures, .phoneNotifications, .agentCommand]
        case .updates: [.autoUpdate, .version]
        }
    }

    /// The tab that holds `control`.
    public static func tab(of control: SettingsControl) -> SettingsTab {
        allCases.first { $0.controls.contains(control) } ?? .general
    }

    /// The remembered tab; nothing stored or an unknown value: General.
    public init(stored: String?) {
        self = stored.flatMap(SettingsTab.init(rawValue:)) ?? .general
    }
}

/// One control (row) of the Settings page.
public enum SettingsControl: String, CaseIterable, Sendable {
    case machines, backend, window, textSize, readingWidth, terminalColors, plusButton
    case allowInput, statusLine, grokStatusLine, claudeQuestions
    case remoteAccess, connectWith, tailscaleSteps, address, devices, pictures, phoneNotifications, agentCommand
    case autoUpdate, version
}

extension SettingsTab {
    /// One line under the page's title: what the tab is for.
    public var summary: String {
        switch self {
        case .general: "What to show, how it looks, what + does."
        case .agents: "What Flockbook may do in your agents' terminals."
        case .remote: "Use your agents from your phone."
        case .updates: "Keep Flockbook up to date."
        }
    }
}

/// Settings > Remote: the Tailscale checks, one `done` per step in step order. All passed: one summary row, the steps
/// behind Details. Any not passed: every step shows, those not passed first, so the one to fix is on top.
public enum SetupChecks {
    public static func collapsed(_ done: [Bool], details: Bool) -> Bool { done.allSatisfy { $0 } && !details }
    /// Step indices top to bottom: the ones not passed, then the passed ones, each part in step order.
    public static func order(_ done: [Bool]) -> [Int] { done.indices.filter { !done[$0] } + done.indices.filter { done[$0] } }
    public static func summary(_ done: [Bool]) -> String {
        let n = done.filter { $0 }.count
        return n == done.count ? "All \(n) checks passed" : "\(n) of \(done.count) checks passed"
    }
}
