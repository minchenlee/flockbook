import Foundation
@testable import HerdrKit

// Approval and question cards for hc agents: 7501 `blocked` + kind decides which screen check runs; the screen comes
// from `terminal.read`; answers go out as `terminal.keys`, only from the card the user pressed. FakeHC only.

@MainActor func checksHCCards() async throws {
    // Which checks run, from the agent alone (pure).
    func agent(_ b: BackendKind, _ s: AgentStatus, _ k: String?, kind: String = "claude") -> Agent {
        Agent(name: "a", kind: kind, status: s, sessionId: nil, title: "", backend: b, statusKind: k)
    }
    typealias G = PermissionGate
    check(G.checkApproval(agent(.hc, .blocked, "permission"), pendingTool: false), "cards: hc blocked/permission checks the approval screen")
    check(!G.checkApproval(agent(.hc, .blocked, "question"), pendingTool: true), "cards: hc blocked/question never checks approval")
    check(G.checkApproval(agent(.hc, .blocked, nil), pendingTool: false), "cards: hc blocked without kind: the screen decides")
    check(!G.checkApproval(agent(.hc, .working, nil), pendingTool: true), "cards: hc not blocked: no check (7501 is the source)")
    check(G.checkApproval(agent(.tui, .working, nil), pendingTool: true) && !G.checkApproval(agent(.tui, .idle, nil), pendingTool: false),
          "cards: TUI unchanged (blocked or a pending tool call)")
    check(G.readQuestion(agent(.hc, .blocked, "question")) && G.readQuestion(agent(.hc, .blocked, nil)), "cards: hc question read on question / no kind")
    check(!G.readQuestion(agent(.hc, .blocked, "permission")) && !G.readQuestion(agent(.hc, .idle, "question")), "cards: no question read on permission or when not blocked")
    check(G.readQuestion(agent(.tui, .blocked, nil)) && !G.readQuestion(agent(.tui, .blocked, nil, kind: "codex")), "cards: TUI question read unchanged (Claude, blocked)")

    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    let screen: any PaneScreen = hc
    func keys() -> [[String]] { fake.calls.filter { $0.method == "terminal.keys" }.compactMap { $0.params["keys"] as? [String] } }

    // Approval: screen from terminal.read, keys from the card.
    fake.setScreen("t1", "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No")
    check(await screen.checkPrompt(paneId: "hc:t1"), "cards: approval prompt read from terminal.read")
    check(fake.calls.last { $0.method == "terminal.read" }?.params["terminal"] as? String == "t1", "cards: terminal.read on the pane's terminal")
    check(keys().isEmpty, "cards: reading sends no keys")
    try await screen.respond(to: "hc:t1", allow: true)
    check(keys().last == ["1"], "cards: Allow = '1' (\(keys().last ?? []))")
    try await screen.respond(to: "hc:t1", allow: false)
    check(keys().last == ["esc"], "cards: Deny = esc (\(keys().last ?? []))")
    fake.setScreen("t1", "> working on it")
    check(!(await screen.checkPrompt(paneId: "hc:t1")), "cards: no prompt on screen: no card")

    // Question: Claude's picker read from the screen; the answer goes out as keys and the picker goes away.
    let picker = """
    ❯ Ask me.
    ────────────────────
     ☐ Color
    Pick a color?
    ❯ 1. Red (Recommended)
         warm
      2. Blue
         cool
      3. Type something.
    ────────────────────
      4. Chat about this
    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    fake.setScreen("t1", picker)
    let q = try await screen.pendingQuestion(paneId: "hc:t1")
    check(q?.questions.first?.text == "Pick a color?" && q?.questions.first?.options.map(\.label) == ["Red", "Blue"], "cards: question read from terminal.read")
    let before = keys().count
    fake.onKeys = { addr, k in if addr == "t1", k == ["2"] { fake.setScreen("t1", "⏺ User answered: Blue\n> ") } }
    if let q { try await screen.answerQuestion(paneId: "hc:t1", request: q, answers: [QuestionAnswer(picks: [1])]) }
    check(Array(keys().dropFirst(before)) == [["2"]], "cards: answer = the picker keys, nothing else (\(Array(keys().dropFirst(before))))")
    fake.onKeys = nil

    // The TUI client refuses hc ids; the hc client refuses TUI ids (no cross-backend keys).
    var refused = false
    do { try await hc.respond(to: "w1:p1", allow: true) } catch { refused = true }
    check(refused, "cards: hc client refuses a TUI pane id")
}
