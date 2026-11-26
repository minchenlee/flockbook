import Foundation
@testable import HerdrKit

// Fifth review: every paired key is gated with what it is for (no prompt for text; this exact question for an
// answer), one committing key per call, no session id = no binding, fresh agent status, a refused second Enter fails.
@MainActor func checksHubReview5() async throws {
    let fake = FakeHerdr(path: NSTemporaryDirectory() + "hkr5-\(getpid()).sock")
    try fake.start()
    defer { fake.kill() }
    let client = HerdrClient(session: HerdrSession(name: "fake", socketPath: fake.path))
    func agents(_ session: String?, status: String = "idle") -> String {
        let s = session.map { #","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"\#($0)"}"# } ?? ""
        return #"[{"pane_id":"w1:p1","agent":"claude","name":"claude","agent_status":"\#(status)"\#(s)}]"#
    }
    let seen = Agent(name: "claude", kind: "claude", status: .idle, sessionId: "s1", title: "")
    let keyMethods: Set<String> = ["pane.send_keys", "pane.send_text", "pane.send_input"]
    func keys(after n: Int) -> [(method: String, params: [String: Any])] { fake.requests.dropFirst(n).filter { keyMethods.contains($0.method) } }
    let open = KeyGate(agent: seen, allowed: { true })

    let color = """
    ←  ☐ Color  ☐ Size  ✔ Submit  →
    Pick a color?
    ❯ 1. Red
         Choose red
      2. Blue
         Choose blue
      3. Type something.
    ────────────────────
      4. Chat about this
    Enter to select · Tab/Arrow keys to navigate · Esc to cancel
    """
    let size = color.replacingOccurrences(of: "Pick a color?", with: "Pick a size?").replacingOccurrences(of: "Red", with: "Small")
        .replacingOccurrences(of: "Blue", with: "Large").replacingOccurrences(of: "Choose red", with: "Choose small").replacingOccurrences(of: "Choose blue", with: "Choose large")
    let lookalike = color.replacingOccurrences(of: "1. Red", with: "1. Green")
    let review = """
    ←  ☒ Color  ☒ Size  ✔ Submit  →

    Review your answers

     ● Pick a color?
       → Red
     ● Pick a size?
       → Small

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    """
    let q = QuestionAdapter.screenRequest(headers: ["Color", "Size"], questions: [
        .init(header: "Color", text: "Pick a color?", options: [.init(label: "Red"), .init(label: "Blue")], multiSelect: false),
        .init(header: "Size", text: "Pick a size?", options: [.init(label: "Small"), .init(label: "Large")], multiSelect: false)])

    // (2) The exact question: text, every option, the headers.
    check(QuestionAdapter.shows(q, question: 0, screen: color) && !QuestionAdapter.shows(q, question: 1, screen: color), "r5 bind: question 1 shown, not question 2")
    check(!QuestionAdapter.shows(q, question: 0, screen: lookalike), "r5 bind: same text, another option -> not this question")
    check(!QuestionAdapter.shows(q, question: 0, screen: color.replacingOccurrences(of: "☐ Color", with: "☐ Hue")), "r5 bind: other headers -> not this request")
    check(QuestionAdapter.showsAny(q, screen: size) && !QuestionAdapter.showsAny(q, screen: lookalike), "r5 bind: any question of the request, exactly")
    check(!QuestionAdapter.showsReview(q, screen: review)
          && QuestionAdapter.showsReview(q, screen: review, answers: [QuestionAnswer(picks: [0]), QuestionAnswer(picks: [0])])
          && !QuestionAdapter.showsReview(q, screen: review.replacingOccurrences(of: "Pick a size?", with: "Pick a shape?"), answers: [QuestionAnswer(picks: [0]), QuestionAnswer(picks: [0])])
          && !QuestionAdapter.showsReview(q, screen: color, answers: [QuestionAnswer(picks: [0]), QuestionAnswer(picks: [0])]), "r5 bind: the review screen matches the exact answers on Claude")

    fake.agents = agents("s1", status: "blocked")
    let one = QuestionAdapter.screenRequest(headers: ["Color"], questions: [q.questions[0]])
    fake.screen = lookalike.replacingOccurrences(of: "☐ Color  ☐ Size  ✔ Submit", with: "☐ Color  ✔ Submit")
    var n = fake.requests.count
    do { try await client.answerQuestion(paneId: "w1:p1", request: one, answers: [QuestionAnswer(picks: [0])], gate: open) } catch {}
    check(keys(after: n).isEmpty, "r5 bind: a picker with the same question text but other options gets no key (\(keys(after: n).count))")

    // (3) One key per call for a paired device: the committing key is its own gated call.
    let multi = """
    ←  ☐ Tops  ✔ Submit  →
    Which toppings?
    ❯ 1. [ ] Cheese
      2. [ ] Olive
      3. [ ] Type something
         Submit
    ────────────────────
      4. Chat about this
    """
    fake.screen = multi
    if let shown = QuestionAdapter.claudeScreen(multi) {
        let mq = QuestionAdapter.screenRequest(headers: shown.headers, questions: [shown.question])
        n = fake.requests.count
        do { try await client.answerQuestion(paneId: "w1:p1", request: mq, answers: [QuestionAnswer(picks: [0], other: "ham")], gate: open) } catch {}
        let named = keys(after: n).filter { $0.method == "pane.send_keys" }.map { ($0.params["keys"] as? [String]) ?? [] }
        check(!named.isEmpty && named.allSatisfy { $0.count == 1 }, "r5 batch: every named key its own gated call (\(named))")
    } else { check(false, "r5 batch: fixture parses") }

    // (4) No session id: nothing to bind to, the gate says no.
    let noSession = Agent(name: "claude", kind: "claude", status: .idle, sessionId: nil, title: "")
    fake.agents = agents(nil)
    fake.screen = "> "
    check(await !client.passes(KeyGate(agent: noSession, allowed: { true }), paneId: "w1:p1"), "r5 identity: no session id on either side -> no")

    // (5) Approve reads the agent's status fresh: not blocked now -> no key.
    let box = """
     Bash command
     npm run build
     Do you want to proceed?
     ❯ 1. Yes
       2. No

     Esc to cancel · Tab to amend
    """
    fake.screen = box
    fake.agents = agents("s1", status: "working")
    n = fake.requests.count
    let ap = try await client.respondIfOpen(paneId: "w1:p1", prompt: ApprovalNotice.screenPrompt(box) ?? "", allow: true, agentKind: "claude",
                                            blocked: true, pending: nil, gate: open)
    check(!ap && keys(after: n).isEmpty, "r5 approve: the agent is not blocked now (stale 'blocked' from the check) -> no key")

    // (1) Text from a paired device: no key while a prompt is on screen or the agent is blocked.
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("hkr5-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let img = dir.appendingPathComponent("remote-\(UUID().uuidString).jpg").path
    FileManager.default.createFile(atPath: img, contents: Data([1]))
    fake.agents = agents("s1")
    fake.screen = box
    n = fake.requests.count
    let p1 = await MessageSend.send("1", to: "w1:p1", client: client, kind: "claude", directory: dir.path, gate: open)
    check(!p1 && keys(after: n).isEmpty, "r5 text: a prompt on screen -> plain text not sent")
    n = fake.requests.count
    let p2 = await MessageSend.send("look \(img)", to: "w1:p1", client: client, kind: "claude", directory: dir.path, gate: open)
    check(!p2 && keys(after: n).isEmpty, "r5 text: a prompt on screen -> image text not sent")
    fake.screen = "❯ \n"
    fake.agents = agents("s1", status: "blocked")
    n = fake.requests.count
    let p3 = await MessageSend.send("hi", to: "w1:p1", client: client, kind: "claude", directory: dir.path, gate: open)
    check(!p3 && keys(after: n).isEmpty, "r5 text: the agent is blocked now -> not sent")
    fake.agents = agents("s1")
    n = fake.requests.count
    let p4 = await MessageSend.send("hi", to: "w1:p1", client: client, kind: "claude", directory: dir.path, gate: open)
    check(p4 && keys(after: n).count == 1, "r5 text: at input, no prompt -> sent")

    // (6) The second Enter refused: the send failed (the upload stays).
    final class Budget: @unchecked Sendable { var left = 3 }
    let budget = Budget()
    fake.screen = "❯ \n"
    fake.onInput = { method, _ in if method == "pane.send_text" { fake.screen = "❯ look [Image #1]\n" } }
    defer { fake.onInput = nil }
    let p5 = await MessageSend.send("look \(img)", to: "w1:p1", client: client, kind: "claude", directory: dir.path,
                                    gate: KeyGate(agent: seen, allowed: { budget.left -= 1; return budget.left >= 0 }), maxWait: .seconds(2))
    check(!p5, "r5 enter: Enter eaten and the second Enter refused -> not sent")
}
