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
    let bad = await FeedCards.answer(pane: qpane, screen: hc, log: nil, request: "screen-nope", answers: [QuestionAnswer(picks: [1])])
    check(bad.result == "changed", "modfeed answer: another request, nothing sent")
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
