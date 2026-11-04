import HerdrKit
import SwiftUI

/// The stored Backend choice and the plan it makes with this launch's flags (`BackendPlan.make`).
enum BackendSetting {
    /// Set once at launch: headless runs follow hc only with `--hc-socket`.
    nonisolated(unsafe) static var headless = false

    static var choice: BackendChoice { BackendChoice.read(AppDefaults.store.string(forKey: BackendChoice.key)) }

    static func plan(session: HerdrSession, headless: Bool = headless, choice: BackendChoice = choice) -> BackendPlan {
        BackendPlan.make(choice: choice, arguments: CommandLine.arguments, environment: ProcessInfo.processInfo.environment,
                         headless: headless, remote: session.remote != nil)
    }

    /// A pick in Settings: stores it, and the running store (nil in snapshots) follows at once. False: the store
    /// could not follow (no hc client: launched with `--no-hc`); the choice applies at the next launch without it.
    @MainActor @discardableResult
    static func pick(_ choice: BackendChoice, store: HerdrStore?) -> Bool {
        AppDefaults.store.set(choice.rawValue, forKey: BackendChoice.key)
        guard let store else { return true }
        return store.use(plan(session: store.session, choice: choice))
    }

    /// The lines under the picker: what the choice does, then what wins over it.
    static func note(_ choice: BackendChoice, flags: Bool, remote: Bool) -> String {
        var lines: [String] = []
        switch choice {
        case .automatic: lines.append("Shows the herdr TUI and herdr-core, whichever answers. While one answers, the other shows no problem and is tried again every 30 s.")
        case .hcOnly: lines.append("Never contacts the herdr TUI on this Mac.")
        case .tuiOnly: lines.append("Never contacts herdr-core.")
        case .both: lines.append("Follows both and shows the connection problems of both.")
        }
        lines.append(remote
            ? "The machine shown is not this Mac: its herdr TUI stays on with every choice. herdr-core only applies to this Mac."
            : "A machine other than This Mac (above) always shows its herdr TUI: herdr-core only applies to this Mac.")
        if flags { lines.append("Started with --no-hc, --hc-socket, --session or --remote: those win over this choice.") }
        return lines.joined(separator: " ")
    }
}

/// Settings > General > Backend: which runtimes Flockbook follows. A new choice applies at once: the store stops and
/// starts with the new plan (`HerdrStore.use`), no relaunch.
struct BackendSection: View {
    /// The running store; nil in snapshots (a choice there is only stored).
    var store: HerdrStore? = nil
    @AppStorage(BackendChoice.key, store: AppDefaults.store) private var raw = BackendChoice.automatic.rawValue

    var body: some View {
        let choice = BackendChoice.read(raw)
        let plan = store.map { BackendSetting.plan(session: $0.session, choice: choice) }
        SettingsGroup("Backend", note: BackendSetting.note(choice, flags: plan?.flags ?? false, remote: store?.session.remote != nil)) {
            SettingsRow("Follow") {
                Picker("Backend", selection: Binding(get: { choice }, set: { BackendSetting.pick($0, store: store) })) {
                    ForEach(BackendChoice.allCases) { Text($0.label).tag($0) }
                }
                .labelsHidden().fixedSize()
            }
            .settingsControl(.backend)
        }
    }
}
