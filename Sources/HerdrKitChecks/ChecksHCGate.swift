import Foundation
import HerdrKit

/// A paired device's key gate on hc panes. hc gives no session id; the store finds it in the agent's log, so the
/// gate binds to the agent's process in the terminal instead (its pid plus the terminal's creation time).
@MainActor func checksHCGate() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func seen(_ id: String) async throws -> Agent? {
        // As the store shows it: the session id found in the log.
        guard var a = try await hc.snapshot().flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == id })?.agent else { return nil }
        a.sessionId = "sid-1"; a.sessionResolved = true
        return a
    }
    func gate(_ a: Agent?) -> KeyGate { KeyGate(agent: a, allowed: { true }) }

    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission"),
                      FakeHC.terminal("box/t3", status: "working")]
    let a = try await seen("hc:t1")
    check(a != nil, "hc gate: the agent is there")
    check(await hc.passes(gate(a), paneId: "hc:t1"), "hc gate: the same program in the terminal -> keys may go")
    let r = try await seen("hc:box/t3")
    check(await hc.passes(gate(r), paneId: "hc:box/t3"), "hc gate: a terminal on another machine binds the same way")

    // Another program now: the agent quit and a new one started in the same terminal (new pid), or under the same shell.
    fake.terminals = [FakeHC.terminal("t1", pid: 5151, status: "blocked", kind: "permission")]
    check(await !hc.passes(gate(a), paneId: "hc:t1"), "hc gate: a new pid -> no")
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission", fg: ["name": "claude", "argv": ["claude"], "pid": 6161])]
    let underShell = try await seen("hc:t1")
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission", fg: ["name": "claude", "argv": ["claude"], "pid": 7171])]
    check(await !hc.passes(gate(underShell), paneId: "hc:t1"), "hc gate: a new foreground pid under the same shell -> no")
    fake.terminals = [FakeHC.terminal("t1", argv: ["codex"], status: "blocked", kind: "permission")]
    check(await !hc.passes(gate(a), paneId: "hc:t1"), "hc gate: another agent kind -> no")
    // No foreground read (before hc's first read, a failed read, a machine without process groups): the terminal's pid
    // is the agent only when the terminal's own program is the agent. A shell that runs an agent gives nothing to bind to.
    fake.terminals = [FakeHC.terminal("t1", argv: ["zsh"], status: "blocked", kind: "permission")]
    let shell = try await seen("hc:t1")
    check(shell != nil && shell?.process == nil, "hc gate: no foreground, the terminal runs a shell -> no program to bind to (\(shell?.process ?? "nil"))")
    check(await !hc.passes(gate(shell), paneId: "hc:t1"), "hc gate: no foreground under a shell -> no")
    fake.terminals = [FakeHC.terminal("t1", argv: ["zsh"], status: "blocked", kind: "permission", fg: ["name": "zsh", "argv": ["zsh"], "pid": 4242])]
    check(try await seen("hc:t1")?.process == nil, "hc gate: the shell in the foreground -> no program to bind to")
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission")]
    check(try await seen("hc:t1")?.process == "4242@1", "hc gate: no foreground, the terminal's own program is the agent -> its pid")

    // No pid at all: nothing stable to bind to, so no.
    fake.terminals = [FakeHC.terminal("t1", pid: nil, status: "blocked", kind: "permission")]
    let bare = try await seen("hc:t1")
    check(await !hc.passes(gate(bare), paneId: "hc:t1"), "hc gate: no pid -> no")
    fake.terminals = []
    check(await !hc.passes(gate(a), paneId: "hc:t1"), "hc gate: the terminal is gone -> no")

    // The phone's send: the pane is ready for text when the same program is there and it waits for none of its prompts.
    fake.terminals = [FakeHC.terminal("t1", status: "idle")]
    fake.setScreen("t1", "> ")
    let idle = try await seen("hc:t1")
    check(await hc.readyForText(paneId: "hc:t1", agent: idle), "hc gate: send to the same idle program -> ready")
    fake.terminals = [FakeHC.terminal("t1", pid: 5151, status: "idle")]
    check(await !hc.readyForText(paneId: "hc:t1", agent: idle), "hc gate: send after a new pid -> not ready")

    // A question from the log of an hc agent: the phone may answer it when the gate can bind (a program in the terminal).
    let q = QuestionRequest(toolUseId: "call-q-0", agent: "claude", questions: [
        .init(header: "Color", text: "Which color?", options: [.init(label: "Red", detail: ""), .init(label: "Blue", detail: "")])])
    check(a.map { !Hub.answerOnMac(q, agent: $0) } == true && a.map { HubQuestion(q, paneAgent: $0).onMac == nil } == true,
          "hc gate: a log question on an hc agent -> the phone answers")
    check(bare.map { Hub.answerOnMac(q, agent: $0) } == true, "hc gate: no pid -> the Mac only")
}
