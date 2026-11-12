import Foundation
@testable import HerdrKit

// Review round 2 on b04d6cf: red first.
@MainActor func checksClaudeHookReview2() async throws {
    func req(_ id: String, _ questions: String) -> QuestionRequest {
        ClaudeQuestionHook.record(Data((#"{"session_id":"sess-2","hook_event_name":"PreToolUse","tool_name":"AskUserQuestion","tool_use_id":""# + id + #"","tool_input":{"questions":["# + questions + #"]}}"#).utf8), time: Date())!.request
    }
    let one = #"{"question":"Pick a color?","header":"Color","multiSelect":false,"options":[{"label":"Red","description":"warm"},{"label":"Blue","description":"cool"}]}"#

    // P2 (1): the bound record changes between the hub's check and the keys (A ends, B opens with the same screen):
    // no key for A's answer. Same when the log gets A's result meanwhile.
    let a = req("toolu_A", one), b = req("toolu_B", one)
    let agent = Agent(name: "claude", kind: "claude", status: .blocked, sessionId: "sess-2", title: "")
    let screen = ClaudePickerSim([.init(header: "Color", text: "Pick a color?", options: [("Red", "warm"), ("Blue", "cool")], multi: false)]).render()
    let devices = MemoryDeviceStore()
    devices.add(token: "tok", device: HubDevice(id: "d", name: "Phone", level: .input))
    func reply(_ ms: [HubMessage]) -> HubReply? { ms.last.flatMap { if case .reply(let r) = $0 { r } else { nil } } }
    var answeredA = a; answeredA.state = .answered(["Blue"])
    for (change, why) in [("record", "a newer hook record"), ("log", "the log has A's result"), ("none", "nothing changed")] {
        let fb = FakeHubBackend(workspaces: [Workspace(id: "w1", label: "w", tabs: [Tab(id: "w1:t1", label: "t", number: 1,
                                  panes: [Pane(id: "w1:p1", terminalId: "t1", cwd: "/w", agent: agent)], status: .blocked)])])
        fb.screens["w1:p1"] = screen
        fb.hooked["sess-2"] = a
        let hs = HubSession(backend: fb, devices: devices)
        _ = await hs.receive(Data(#"{"v":1,"type":"hello","id":"h","token":"tok","client":"test","clientVersion":"1"}"#.utf8))
        let w = await hs.receive(Data(#"{"v":1,"type":"watch","id":"c1","pane":"w1:p1"}"#.utf8))
        let hq: HubQuestion? = w.compactMap { if case .prompt(let p) = $0 { p.question } else { nil } }.first
        fb.beforeKeys = {
            if change == "record" { fb.hooked["sess-2"] = b }
            if change == "log" { fb.items["sess-2"] = [ConversationItem(kind: .question(answeredA))] }
        }
        let r = await hs.receive(Data(#"{"v":1,"type":"answer","id":"c2","pane":"w1:p1","question":"\#(hq?.id ?? "")","answers":[{"picks":[1]}]}"#.utf8))
        if change == "none" {
            check(hq?.id == "toolu_A" && reply(r)?.ok == true && fb.answered.count == 1, "r2 P2: \(why) -> answered")
        } else {
            check(hq?.id == "toolu_A" && reply(r)?.ok != true && fb.answered.isEmpty, "r2 P2: \(why) before the keys -> nothing sent")
        }
    }

    // P2 (2): review structure marks inside labels. Wanted [A, "B●Second?→C"]; the screen shows ["A●Second?→B", C].
    let marks = req("toolu_M", #"{"question":"First?","header":"One","multiSelect":false,"options":[{"label":"A","description":""},{"label":"A●Second?→B","description":""}]},{"question":"Second?","header":"Two","multiSelect":false,"options":[{"label":"B●Second?→C","description":""},{"label":"C","description":""}]}"#)
    let want = [QuestionAnswer(picks: [0]), QuestionAnswer(picks: [0])]
    let review = """
    ←  ☒ One  ☒ Two  ✔ Submit  →

    Review your answers

     ● First?
       → A●Second?→B
     ● Second?
       → C

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    """
    check(QuestionAdapter.reviewAmbiguous(marks, answers: want), "r2 P2: a label with the review marks ● or → -> ambiguous")
    check(!QuestionAdapter.claudeReviewExact(marks, answers: want, screen: review), "r2 P2: the forged review does not match")
    // Line structure: ● and → start their lines (a plain request still matches; the same text run together does not).
    let plain = req("toolu_P", #"{"question":"First?","header":"One","multiSelect":false,"options":[{"label":"A","description":""},{"label":"B","description":""}]},{"question":"Second?","header":"Two","multiSelect":false,"options":[{"label":"C","description":""},{"label":"D","description":""}]}"#)
    let plainWant = [QuestionAnswer(picks: [0]), QuestionAnswer(picks: [1])]
    let plainReview = review.replacingOccurrences(of: "A●Second?→B", with: "A").replacingOccurrences(of: "→ C", with: "→ D")
    check(QuestionAdapter.claudeReviewExact(plain, answers: plainWant, screen: plainReview), "r2 P2: the plain review still matches")
    check(!QuestionAdapter.claudeReviewExact(plain, answers: plainWant, screen: plainReview.replacingOccurrences(of: "→ A\n ● Second?", with: "→ A ● Second?")),
          "r2 P2: a review mark not at a line start -> no match")

    // P2 (3): the label/description boundary. Record: "Delete nothing" with no description; screen: "Delete", "nothing".
    let del = req("toolu_D", #"{"question":"Do it?","header":"Act","multiSelect":false,"options":[{"label":"Delete nothing","description":""},{"label":"Keep","description":"k"}]}"#)
    let delScreen = ClaudePickerSim([.init(header: "Act", text: "Do it?", options: [("Delete", "nothing"), ("Keep", "k")], multi: false)]).render()
    check(!QuestionAdapter.claudeExact(del, question: 0, screen: delScreen, other: nil, focus: .any, checks: nil), "r2 P2: label and description moved across their boundary -> no match")
    let delTrue = ClaudePickerSim([.init(header: "Act", text: "Do it?", options: [("Delete nothing", ""), ("Keep", "k")], multi: false)]).render()
    check(QuestionAdapter.claudeExact(del, question: 0, screen: delTrue, other: nil, focus: .any, checks: nil), "r2 P2: the true rendering still matches")
    let rev = req("toolu_V", #"{"question":"Do it?","header":"Act","multiSelect":false,"options":[{"label":"Delete","description":"nothing"},{"label":"Keep","description":"k"}]}"#)
    check(!QuestionAdapter.claudeExact(rev, question: 0, screen: delTrue, other: nil, focus: .any, checks: nil), "r2 P2: the other way round -> no match")

    // P3 (4): an ambiguous review with the picker on question 2: refused before the Left that goes back to question 1.
    // Two labels alike once spaces go (a single-choice comma is not ambiguous since PR #14).
    let amb = req("toolu_Q", #"{"question":"First?","header":"One","multiSelect":false,"options":[{"label":"A B","description":""},{"label":"AB","description":""}]},{"question":"Second?","header":"Two","multiSelect":false,"options":[{"label":"C","description":""},{"label":"D","description":""}]}"#)
    let sim = ClaudePickerSim([.init(header: "One", text: "First?", options: [("A B", ""), ("AB", "")], multi: false),
                               .init(header: "Two", text: "Second?", options: [("C", ""), ("D", "")], multi: false)])
    sim.at = 1
    let fake = FakeHerdr(path: NSTemporaryDirectory() + "hkB2-\(getpid()).sock")
    try fake.start()
    defer { fake.kill() }
    let client = HerdrClient(session: HerdrSession(name: "fake", socketPath: fake.path))
    fake.agents = #"[{"pane_id":"w1:p1","agent":"claude","name":"claude","agent_status":"blocked","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"sess-2"}}]"#
    fake.screen = sim.render()
    let n = fake.requests.count
    var err: Error?
    do { try await client.answerQuestion(paneId: "w1:p1", request: amb, answers: [QuestionAnswer(picks: [0]), QuestionAnswer(picks: [0])],
                                         gate: KeyGate(agent: agent, allowed: { true })) } catch { err = error }
    check(err != nil && !fake.requests.dropFirst(n).contains { ["pane.send_keys", "pane.send_text"].contains($0.method) },
          "r2 P3: ambiguous review, picker on question 2 -> refused before any key (no Left)")
}
