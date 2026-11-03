import Foundation
@testable import HerdrKit

// The approval and question card path on hc.

/// 1: one answer per prompt. A press claims the pane; more presses are ignored until the caller's re-check ends it.
@MainActor func checksHCCardFlowInFlight() async throws {
    let s = ApprovalSender()
    check(s.begin("hc:t1"), "1: the first press claims the pane")
    check(!s.begin("hc:t1"), "1: a second press (Allow twice, or Allow then Deny) is ignored")
    check(s.isAnswering("hc:t1") && !s.isAnswering("hc:t2"), "1: only that pane is busy")
    check(s.begin("hc:t2"), "1: another pane's card still works")
    s.end("hc:t1")
    check(!s.isAnswering("hc:t1") && s.begin("hc:t1"), "1: after the re-check the pane can be answered again")
}

/// 2: the screen is read again right before the keys; a prompt that is gone gets nothing.
@MainActor func checksHCCardFlowStale() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission")]
    let screen: any PaneScreen = HCClient(session: HCSession(socketPath: fake.path))
    func keys() -> [[String]] { fake.calls.filter { $0.method == "terminal.keys" }.compactMap { $0.params["keys"] as? [String] } }
    fake.setScreen("t1", "> the agent moved on")
    let gone = await screen.answerPrompt(paneId: "hc:t1", allow: true)
    check(gone == .promptGone && keys().isEmpty, "2: prompt gone at press time: nothing sent (\(gone), keys \(keys()))")
    fake.setScreen("t1", "Bash command\n  ls\nDo you want to proceed?\n❯ 1. Yes\n  2. No")
    let sent = await screen.answerPrompt(paneId: "hc:t1", allow: true)
    check(sent == .sent && keys() == [["1"]], "2: prompt still there: the answer goes out (\(sent), keys \(keys()))")
}

/// 3: the card answers the pane its text came from: the blocked pane whose screen shows the prompt.
@MainActor func checksHCCardFlowCardPane() async throws {
    func pane(_ id: String, _ s: AgentStatus, sid: String) -> Pane {
        Pane(id: id, terminalId: id, cwd: "/tmp", agent: Agent(name: id, kind: "claude", status: s, sessionId: sid, title: "", backend: .hc), backend: .hc)
    }
    let a = pane("hc:t1", .blocked, sid: "s1"), b = pane("hc:t2", .blocked, sid: "s2"), c = pane("hc:t3", .working, sid: "s3")
    check(PermissionGate.cardPane([a, b], promptVisible: ["hc:t2"])?.id == "hc:t2", "3: two blocked panes: the one whose screen shows the prompt")
    check(PermissionGate.cardPane([c, a], promptVisible: ["hc:t1"])?.id == "hc:t1", "3: not the first agent pane of the tab")
    check(PermissionGate.cardPane([a, c], promptVisible: ["hc:t3"]) == nil, "3: a prompt flag on a pane that is not blocked shows no card")
    check(PermissionGate.cardPane([a, b], promptVisible: []) == nil, "3: no prompt on screen: no card")
}

/// 4: only Claude's approval prompt counts, not a question menu or text that merely contains "Do you want".
@MainActor func checksHCCardFlowMarker() async throws {
    let G = PermissionGate.self
    check(G.screenShowsPrompt(" Do you want to proceed?\n ❯ 1. Yes\n   2. Yes, and don't ask again\n   3. No (esc)"), "4: the approval prompt")
    check(G.screenShowsPrompt("│ Do you want to make this edit to main.swift?\n│ ❯ 1. Yes\n│   2. No, and tell Claude what to do differently (esc)"), "4: edit prompt inside a box")
    let question = """
    ←  ☐ Deploy  ✔ Submit  →
    Do you want to deploy now?
    ❯ 1. Yes
      2. Later
      3. Type something.
    ────────────────────
      4. Chat about this
    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    check(!G.screenShowsPrompt(question), "4: a question menu that asks 'Do you want' is not an approval")
    check(!G.screenShowsPrompt("⏺ Do you want me to refactor this too?\n> "), "4: the agent's own text with 'Do you want' is not an approval")
    check(!G.screenShowsPrompt("Do you want to proceed?\n(no options drawn yet)"), "4: no '1. Yes' option, no approval")
    check(G.screenShowsPrompt(" Do you want to make this edit to\n Sources/HerdrKit/SomeVeryLongFileName.swift?\n ❯ 1. Yes\n   2. No"), "4: a wrapped question still counts")
}

/// 5: a slowly typed line whose start expired is dropped whole: never the rest of it plus Enter.
@MainActor func checksHCCardFlowPartialKeys() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func attaches() -> Int { fake.calls.filter { $0.method == "terminal.attach" }.count }
    let ts = TerminalSession(hcTerminal: "t1", client: hc, mode: .control(takeover: false))
    ts.pendingWindow = 0.4
    let notes = Locked<[String]>([])
    let r = Task { for await e in ts.events { if case .notice(let n) = e { notes.update { $0.append(n) } } } }
    ts.start()
    check(await eventually { attaches() == 1 }, "5: watcher attached")
    try await Task.sleep(for: .milliseconds(100))
    fake.attachError = "unavailable"
    ts.send(text: "rm -rf ")
    try await Task.sleep(for: .milliseconds(600))   // the start of the line expires
    ts.send(text: "build\r")
    fake.attachError = nil
    let before = fake.lastChannel
    check(await eventually(4) { fake.lastChannel > before }, "5: control attach succeeds later")
    try await Task.sleep(for: .milliseconds(300))
    let got = String(decoding: fake.viewerBytes(fake.lastChannel), as: UTF8.self)
    check(got.isEmpty, "5: nothing of the line is sent (\(got.debugDescription))")
    check(notes.get().contains { $0.contains("not sent") }, "5: the user is told (\(notes.get()))")
    ts.send(text: "x")
    check(await eventually { String(decoding: fake.viewerBytes(fake.lastChannel), as: UTF8.self) == "x" }, "5: keys typed after that go straight out")
    r.cancel(); ts.stop()
}
