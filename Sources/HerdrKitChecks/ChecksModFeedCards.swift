import Foundation
@testable import HerdrKit
import ModFeedKit

// The sidecar's cards on a fake herdr-core: read the card, then keys only for the same request.
func checksModFeedCards() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func keys() -> [[String]] { fake.calls.filter { $0.method == "terminal.keys" }.compactMap { $0.params["keys"] as? [String] } }
    let ws = try await hc.snapshot()
    guard let pane = FeedLocate.pane(ws, terminal: "t1") else { check(false, "modfeed cards: pane t1 in the snapshot"); return }

    // Approval with no log: the id is the prompt's hash; a spinner above it does not change it.
    fake.setScreen("t1", "✻ Working… 3s\nBash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No")
    let c1 = await FeedCards.card(pane: pane, screen: hc, log: nil)
    check(c1.kind == "approval" && c1.request?.hasPrefix("screen-") == true, "modfeed cards: approval card from the screen (\(c1))")
    check(c1.command?.contains("rm -rf build") == true, "modfeed cards: the command from above the prompt (\(c1.command ?? "nil"))")
    fake.setScreen("t1", "✻ Working… 9s\nBash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No")
    let c2 = await FeedCards.card(pane: pane, screen: hc, log: nil)
    check(c2.request == c1.request, "modfeed cards: same prompt, same id while the spinner moves")
    check(keys().isEmpty, "modfeed cards: reading sends no keys")

    // Approve: refused for another request, sent for this one.
    let wrong = await FeedCards.approve(pane: pane, screen: hc, log: nil, request: "screen-other", allow: true)
    check(wrong.result == "changed" && keys().isEmpty, "modfeed approve: another request, nothing sent")
    let ok = await FeedCards.approve(pane: pane, screen: hc, log: nil, request: c1.request ?? "", allow: true)
    check(ok.result == "sent" && keys().last == ["1"], "modfeed approve: allow sends '1' (\(ok), \(keys()))")
    fake.setScreen("t1", "> done")
    let gone = await FeedCards.approve(pane: pane, screen: hc, log: nil, request: c1.request ?? "", allow: false)
    check(gone.result == "changed" && keys().count == 1, "modfeed approve: prompt gone, nothing sent")
    check((await FeedCards.card(pane: pane, screen: hc, log: nil)).kind == nil, "modfeed cards: no prompt, no card")

    // Question: Claude's picker read from the screen, answered with its keys.
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "question")]
    let qpane = FeedLocate.pane(try await hc.snapshot(), terminal: "t1")!
    fake.setScreen("t1", """
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
    """)
    let qc = await FeedCards.card(pane: qpane, screen: hc, log: nil)
    check(qc.kind == "question" && qc.question?.questions.first?.options.map(\.label) == ["Red", "Blue"]
          && qc.question?.questions.first?.options.first?.recommended == true, "modfeed cards: question card (\(qc))")
    let k0 = keys().count
    let qa = await FeedCards.card(pane: qpane, screen: hc, log: nil, all: true)
    check(qa == qc && keys().count == k0, "modfeed card --all: one question, same card, no keys")

    // card --all walks the picker with a gate, as the hub does: before each key the pane must still show the agent
    // that was read. Here another program took the pane after the read, so not one key goes out.
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "question")]
    let mpane = FeedLocate.pane(try await hc.snapshot(), terminal: "t1")!
    fake.setScreen("t1", """
    ←  ☐ Color  ☐ Size  ✔ Submit  →
    Pick a color?
    ❯ 1. Red
      2. Blue
      3. Type something.
    ────────────────────
      4. Chat about this
    Enter to select · Tab/Arrow keys to navigate · Esc to cancel
    """)
    fake.terminals = [FakeHC.terminal("t1", argv: ["codex"], pid: 5151, status: "blocked", kind: "question")]
    let k1 = keys().count
    _ = await FeedCards.card(pane: mpane, screen: hc, log: nil, all: true)
    check(keys().count == k1, "modfeed card --all: gated, another agent in the pane, no keys (\(keys().suffix(3)))")
    // The Mac card's Show all (TabScreen) goes through the same gate: another agent, KeyGateStop, no keys.
    var macStopped = false
    do { _ = try await hc.loadAllQuestionsGated(pane: mpane, expect: nil, allowed: { true }) } catch is KeyGateStop { macStopped = true } catch {}
    check(macStopped && keys().count == k1, "mac card load all: gated, another agent, stopped before any key (\(keys().suffix(3)))")
    // The Mac (the owner at the screen): an agent with no identity (no session id, no process) is not refused; each
    // key checks the screen instead. A picker that answers Left/Right; read all completes; a switch to another
    // request stops the walk; the Mac's answer keys go through the same rule.
    final class Sim: @unchecked Sendable { var i = 0; var other = false }
    let sim = Sim()
    @Sendable func picker(_ i: Int, other: Bool = false) -> String {
        let tabs = other ? "←  ☐ Lunch  ☐ Drink  ✔ Submit  →" : "←  ☐ Color  ☐ Size  ✔ Submit  →"
        let q = other ? (i == 0 ? "Pick a lunch?\n❯ 1. Soup\n  2. Rice" : "Pick a drink?\n❯ 1. Tea\n  2. Milk")
                      : (i == 0 ? "Pick a color?\n❯ 1. Red\n  2. Blue" : "Which size?\n❯ 1. S\n  2. M")
        return tabs + "\n" + q + "\n  3. Type something.\n────────────────────\n  4. Chat about this\nEnter to select · Tab/Arrow keys to navigate · Esc to cancel"
    }
    var bare = mpane; bare.agent?.process = nil; bare.agent?.sessionId = nil
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "question")]
    sim.i = 0; sim.other = false; fake.setScreen("t1", picker(0))
    fake.onKeys = { a, k in
        guard a == "t1" else { return }
        if k == ["left"] { sim.i = max(0, sim.i - 1) } else if k == ["right"] { sim.i = min(1, sim.i + 1) }
        fake.setScreen("t1", picker(sim.i, other: sim.other))
    }
    let one = try? await hc.pendingQuestion(paneId: mpane.id)
    let macAll = try? await hc.loadAllQuestions(paneId: mpane.id, gate: KeyGate.onMac(bare.agent, allowed: { true }), expect: one)
    check(macAll?.questions.count == 2, "mac load all: no agent identity, still reads every question (\(macAll?.questions.map(\.text) ?? []))")
    // Another request comes up after the first key: the next key's screen check stops the walk.
    sim.i = 0; sim.other = false; fake.setScreen("t1", picker(0))
    fake.onKeys = { a, k in guard a == "t1" else { return }; sim.other = true; fake.setScreen("t1", picker(0, other: true)) }
    let kA = keys().count
    var switched = false
    do { _ = try await hc.loadAllQuestions(paneId: mpane.id, gate: KeyGate.onMac(bare.agent, allowed: { true }), expect: one) } catch { switched = true }
    check(switched && keys().count - kA <= 1, "mac load all: another question on screen, stopped (\(keys().suffix(3)))")
    // The Mac's answer keys: another agent (with an identity) in the pane, not one key.
    fake.onKeys = nil; sim.i = 0; sim.other = false; fake.setScreen("t1", picker(0))
    fake.terminals = [FakeHC.terminal("t1", argv: ["codex"], pid: 5151, status: "blocked", kind: "question")]
    let kB = keys().count
    var answerStopped = false
    if let full = macAll {
        do { try await hc.answerQuestion(paneId: mpane.id, request: full, answers: [QuestionAnswer(picks: [0]), QuestionAnswer(picks: [0])],
                                         gate: KeyGate.onMac(mpane.agent, allowed: { true })) } catch is KeyGateStop { answerStopped = true } catch {}
    }
    check(answerStopped && keys().count == kB, "mac answer: gated, another agent, no keys (\(keys().suffix(3)))")
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "question")]
    fake.setScreen("t1", picker(0))
    // The same agent: the gate lets the walk start (the fake screen does not move, so it stops after that).
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "question")]
    _ = await FeedCards.card(pane: mpane, screen: hc, log: nil, all: true)
    check(keys().dropFirst(k1).first == ["left"], "modfeed card --all: gated, same agent, the walk starts (\(keys().suffix(3)))")
    fake.setScreen("t1", """
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
    """)
    let bad = await FeedCards.answer(pane: qpane, screen: hc, log: nil, request: "screen-nope", answers: [QuestionAnswer(picks: [1])])
    check(bad.result == "changed", "modfeed answer: another request, nothing sent")

    // Every key is gated as the hub gates it: another program in the pane after the read, nothing goes out, and the
    // reply says why (not a silent failure).
    let other = [FakeHC.terminal("t1", argv: ["codex"], pid: 5151, status: "blocked", kind: "question")]
    let claude = [FakeHC.terminal("t1", status: "blocked", kind: "question")]
    func sends() -> Int { fake.calls.filter { $0.method == "terminal.send" }.count }
    let g0 = keys().count, s0 = sends()
    fake.terminals = other
    let ga = await FeedCards.answer(pane: qpane, screen: hc, log: nil, request: qc.request ?? "", answers: [QuestionAnswer(picks: [1])])
    check(ga.result == "changed" && ga.message?.contains("agent") == true && keys().count == g0,
          "modfeed answer: gated, another agent, no keys and says why (\(ga), \(keys().suffix(2)))")
    let gs = await FeedCards.send(pane: qpane, client: hc, text: "hello", files: [])
    check(gs.result == "changed" && gs.message?.contains("agent") == true && sends() == s0,
          "modfeed send: gated, another agent, nothing typed and says why (\(gs))")
    let saved = fake.terminals
    fake.terminals = [FakeHC.terminal("t1", status: "blocked", kind: "permission")]
    let apane = FeedLocate.pane(try await hc.snapshot(), terminal: "t1")!
    fake.setScreen("t1", "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No")
    let ac = await FeedCards.card(pane: apane, screen: hc, log: nil)
    fake.terminals = [FakeHC.terminal("t1", argv: ["codex"], pid: 5151, status: "blocked", kind: "permission")]
    let gp = await FeedCards.approve(pane: apane, screen: hc, log: nil, request: ac.request ?? "", allow: true)
    check(gp.result == "changed" && gp.message?.contains("agent") == true && keys().count == g0,
          "modfeed approve: gated, another agent, no keys and says why (\(gp))")
    _ = saved
    fake.terminals = claude
    fake.setScreen("t1", """
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
    """)
    let before = keys().count
    fake.onKeys = { addr, k in if addr == "t1", k == ["2"] { fake.setScreen("t1", "⏺ User answered: Blue\n> ") } }
    let answered = await FeedCards.answer(pane: qpane, screen: hc, log: nil, request: qc.request ?? "",
                                          answers: FeedCards.parseAnswers(#"[{"picks":[1]}]"#) ?? [])
    fake.onKeys = nil
    check(answered.result == "sent" && Array(keys().dropFirst(before)) == [["2"]], "modfeed answer: the picker keys (\(answered))")
    check(FeedCards.parseAnswers(#"[{"picks":[],"other":"teal"}]"#) == [QuestionAnswer(picks: [], other: "teal")], "modfeed answer: other text")
    check(FeedCards.parseAnswers("{}") == nil && FeedCards.parseAnswers(#"[{"picks":"x"}]"#) == nil, "modfeed answer: bad JSON refused")

    // Send: terminal.send with the text.
    let sent = await FeedCards.send(pane: qpane, client: hc, text: "hello", files: [])
    let sendCall = fake.calls.last { $0.method == "terminal.send" }
    check(sent.result == "sent" && sendCall?.params["text"] as? String == "hello" && sendCall?.params["terminal"] as? String == "t1",
          "modfeed send: terminal.send to the terminal (\(sent))")
}
