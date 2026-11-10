import HerdrKit
import SwiftUI
import AppKit

/// Settings > General > Machine: which herdr the app shows. This Mac, or a saved machine reached with the owner's own
/// SSH (`RemoteMachines`). One row each: a click on the row chooses it; saved machines have Test, Edit and Remove.
/// The app reads the choice when it starts (`RemoteMachines.launchSession`), so a new choice asks for a relaunch: the
/// store, the phone server and the notifications all hang on the session the app started with.
struct RemoteMachinesSection: View {
    /// The session the running app shows; nil in snapshots (no relaunch line there).
    var shown: HerdrSession? = nil
    @State private var machines = RemoteMachines(defaults: AppDefaults.store)
    @State private var editing: MachineDraft? = nil
    @State private var results: [String: (ok: Bool, text: String)] = [:]
    @State private var testing: Set<String> = []
    @State private var removing: RemoteMachine? = nil

    var body: some View {
        SettingsGroup("Machine", content: {
            MachineRow(title: "This Mac", detail: "herdr on this Mac", symbol: "laptopcomputer", chosen: machines.selected == nil) {
                choose(nil)
            } controls: { EmptyView() }
            .settingsControl(.machines)
            ForEach(machines.machines) { m in
                let result = results[m.id]
                MachineRow(title: m.title, detail: result?.text ?? m.detail, detailColor: result.map { $0.ok ? Color.green : Color.orange },
                           symbol: "server.rack", chosen: machines.selected == m.id) {
                    choose(m.id)
                } controls: {
                    if testing.contains(m.id) { Spinner() }
                    Button("Test") { test(m) }.controlSize(.small).disabled(testing.contains(m.id))
                        .help("Runs herdr's remote-api-bridge --check on the machine over SSH")
                    Button { editing = MachineDraft(m) } label: { Image(systemName: "pencil") }
                        .buttonStyle(.borderless).help("Edit").accessibilityLabel("Edit \(m.title)")
                    Button { removing = m } label: { Image(systemName: "minus.circle") }
                        .buttonStyle(.borderless).help("Remove").accessibilityLabel("Remove \(m.title)")
                }
            }
            SettingsRow("Add a machine reached with SSH") {
                Button("Add…") { editing = MachineDraft() }.controlSize(.small)
            }
        }, footer: {
            VStack(alignment: .leading, spacing: 4) {
                Text("Flockbook uses your SSH setup (ssh-agent, ~/.ssh/config, known_hosts) and stores no passwords or keys. Accept a new host's key once in Terminal with ssh <destination>.")
                relaunchLine
            }
        })
        .sheet(item: $editing) { draft in
            MachineEditor(draft: draft) { d in
                if let id = d.machine {
                    try machines.update(id, destination: d.destination, session: d.session, label: d.label)
                    results[id] = nil
                } else {
                    try machines.add(destination: d.destination, session: d.session, label: d.label)
                }
            }
        }
        .confirmationDialog("Remove “\(removing?.title ?? "")”?", isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
                            titleVisibility: .visible, presenting: removing) { m in
            Button("Remove", role: .destructive) { machines.remove(m.id); results[m.id] = nil; removing = nil }
            Button("Cancel", role: .cancel) { removing = nil }
        } message: { _ in
            Text("Only the entry in Flockbook goes. Nothing changes on the machine or in your SSH setup.")
        }
    }

    private func choose(_ id: String?) {
        try? machines.select(id)
    }

    private func test(_ m: RemoteMachine) {
        testing.insert(m.id); results[m.id] = nil
        Task { @MainActor in
            let r = await RemoteMachines.test(m)
            testing.remove(m.id)
            if machines.machines.contains(where: { $0.id == m.id }) { results[m.id] = r }
        }
    }

    /// Launch flags win over the choice here; otherwise a choice other than what the window shows waits for a relaunch.
    @ViewBuilder private var relaunchLine: some View {
        if let shown {
            let args = CommandLine.arguments, env = ProcessInfo.processInfo.environment
            let flags = RemoteMachines.launchFlags(arguments: args, environment: env)
            let next = RemoteMachines.launchSession(arguments: args, environment: env, defaults: AppDefaults.store)
            if flags {
                Text("Started with --session or --remote: the choice here applies when Flockbook starts without them.")
            } else if next != shown {
                let name = RemoteMachines.title(for: next, defaults: AppDefaults.store) ?? "this Mac"
                HStack(spacing: 8) {
                    Text(Relaunch.possible ? "Relaunch Flockbook to show \(name)." : "Quit and open Flockbook again to show \(name).")
                        .foregroundStyle(.orange)
                    if Relaunch.possible { Button("Relaunch") { Relaunch.now() }.controlSize(.small) }
                }
            }
        }
    }
}

/// One machine row: a mark when it is the one chosen, its symbol, title and a line under it; then its own buttons.
private struct MachineRow<Controls: View>: View {
    let title: String
    let detail: String
    var detailColor: Color? = nil
    let symbol: String
    let chosen: Bool
    let choose: () -> Void
    @ViewBuilder var controls: Controls

    var body: some View {
        SettingsRow(label: {
            Button(action: choose) {
                HStack(spacing: 10) {
                    Image(systemName: chosen ? "checkmark.circle.fill" : "circle")
                        .foregroundStyle(chosen ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.tertiary))
                    Image(systemName: symbol).foregroundStyle(.secondary).frame(width: 18)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(title).lineLimit(1)
                        Text(detail).font(.system(size: 11)).lineLimit(2).truncationMode(.middle)
                            .foregroundStyle(detailColor.map(AnyShapeStyle.init) ?? AnyShapeStyle(.secondary))
                            .help(detail)
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(title)
            .accessibilityValue(detail)
            .accessibilityAddTraits(chosen ? .isSelected : [])
            .help(chosen ? "Shown" : "Show this machine")
        }, control: { controls })
    }
}

/// The fields of the Add and Edit sheet. `machine` nil: a new machine.
struct MachineDraft: Identifiable {
    let id = UUID()
    var machine: String?
    var destination = ""
    var session = ""
    var label = ""

    init() {}
    init(_ m: RemoteMachine) {
        machine = m.id; destination = m.destination; session = m.session ?? ""; label = m.label ?? ""
    }
}

/// Add or edit a machine: SSH destination, herdr session (empty: its default session) and an optional name. Save is
/// refused with the reason under the fields; nothing is stored until it passes.
private struct MachineEditor: View {
    @State var draft: MachineDraft
    let save: (MachineDraft) throws -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var problem: String? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(draft.machine == nil ? "Add a Machine" : "Edit Machine").font(.title3.weight(.semibold))
            Text("Flockbook runs herdr on the machine through your own ssh, with your keys, ~/.ssh/config and known_hosts. herdr must be installed there and its server running.")
                .font(.callout).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Form {
                TextField("SSH destination", text: $draft.destination, prompt: Text("user@host or a Host alias"))
                TextField("herdr session", text: $draft.session, prompt: Text("default"))
                TextField("Name", text: $draft.label, prompt: Text("optional"))
            }
            .formStyle(.columns)
            if let problem { Text(problem).font(.callout).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true) }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button(draft.machine == nil ? "Add" : "Save") { submit() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(24)
        .frame(width: 460)
    }

    private func submit() {
        var d = draft
        d.destination = d.destination.trimmingCharacters(in: .whitespaces)
        d.session = d.session.trimmingCharacters(in: .whitespaces)
        do { try save(d); dismiss() } catch { problem = "\(error)" }
    }
}

/// Quits and opens the app again (a choice of machine is read at start). A small sh waits for this process to end (at
/// most 20 s), then opens the same app bundle; the pid and path go to it as arguments, never into its script.
enum Relaunch {
    /// Only an app bundle can be opened again; a bare binary (swift run) cannot.
    static var possible: Bool { Bundle.main.bundleURL.pathExtension == "app" }

    @MainActor static func now() {
        guard possible else { return }
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/sh")
        p.arguments = ["-c", "i=0; while kill -0 \"$1\" 2>/dev/null; do i=$((i+1)); [ $i -gt 100 ] && exit 0; sleep 0.2; done; exec /usr/bin/open \"$2\"",
                       "flockbook-relaunch", String(getpid()), Bundle.main.bundleURL.path]
        do { try p.run() } catch { return }
        NSApp.terminate(nil)
    }
}
