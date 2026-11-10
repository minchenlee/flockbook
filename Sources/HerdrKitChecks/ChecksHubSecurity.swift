import Foundation
@testable import HerdrKit

// Findings of the hub security review: each check failed before its fix.
@MainActor func checksHubSecurity() async throws {
    typealias N = ApprovalNotice
    func msg(_ json: String) -> Data { Data(json.utf8) }
    func code(_ ms: [HubMessage]) -> HubError.Code? { ms.last.flatMap { if case .reply(let r) = $0 { r.error?.code } else { nil } } }
    func isOK(_ ms: [HubMessage]) -> Bool { ms.last.map { if case .reply(let r) = $0 { r.ok } else { false } } ?? false }
    func box(_ command: String) -> String {
        """
        ────────────────────────────────────────
         Bash command
         Run it
        ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
         \(command)
        ╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌
         Do you want to proceed?
         ❯ 1. Yes
           2. No

         Esc to cancel · Tab to amend
        """
    }
    let devices = MemoryDeviceStore()
    devices.add(token: "tok", device: HubDevice(id: "d1", name: "Phone", level: .input))
    func ws(_ status: AgentStatus, agentKind: String = "claude") -> [Workspace] {
        let a = Agent(name: agentKind, kind: agentKind, status: status, sessionId: "s1", title: "t")
        return [Workspace(id: "w1", label: "w", tabs: [Tab(id: "w1:t1", label: "t", number: 1,
                          panes: [Pane(id: "w1:p1", terminalId: "term_1", cwd: "/w", agent: a), Pane(id: "w1:p2", terminalId: "term_2", cwd: "/w", agent: nil)],
                          status: status)])]
    }
    func session(_ b: FakeHubBackend) async -> HubSession {
        let s = HubSession(backend: b, devices: devices)
        _ = await s.receive(msg(#"{"v":1,"type":"hello","id":"h","token":"tok","client":"t","clientVersion":"1"}"#))
        return s
    }

    // (2) Remote input goes to agents only: a plain shell pane stays read only.
    do {
        let b = FakeHubBackend(workspaces: ws(.idle))
        b.screens["w1:p2"] = "$ "
        let s = await session(b)
        let r = await s.receive(msg(#"{"v":1,"type":"send","id":"c1","pane":"w1:p2","text":"ls"}"#))
        check(code(r) == .readOnly && b.sent.isEmpty, "security: send to a shell pane -> readOnly, nothing typed (\(String(describing: code(r))))")
        b.screens["w1:p1"] = "> "
        check(isOK(await s.receive(msg(#"{"v":1,"type":"send","id":"c2","pane":"w1:p1","text":"hi"}"#))) && b.sent.count == 1, "security: send to an agent pane still works")
    }

    // (3) Log lag: the log's pending call is A, the screen shows prompt B. An id is good only for the prompt on screen.
    do {
        let screenB = box("rm -rf build")
        let lagging = N.PaneState(agentKind: "claude", blocked: true, pendingToolUseId: "toolu_A", screen: screenB,
                                  pendingTool: "Bash", pendingCommand: "npm run build")
        check(N.prompt(lagging, writes: true) == N.screenPrompt(screenB), "security: log lags the screen -> buttons bound to the screen prompt")
        check(N.outcome(actionIdentifier: N.Action.allow.rawValue, boundPrompt: "toolu_A", now: lagging, writes: true) == .open,
              "security: an id for the log's call A does not answer screen prompt B")
        let caughtUp = N.PaneState(agentKind: "claude", blocked: true, pendingToolUseId: "toolu_B", screen: screenB,
                                   pendingTool: "Bash", pendingCommand: "rm   -rf build")
        check(N.prompt(caughtUp, writes: true) == "toolu_B", "security: the log's call matches the box (tool + command) -> its id is good")
        check(N.outcome(actionIdentifier: N.Action.allow.rawValue, boundPrompt: N.screenPrompt(screenB), now: caughtUp, writes: true) == .send(allow: true),
              "security: the screen id stays good when the log matches")
        let unknownCall = N.PaneState(agentKind: "claude", blocked: true, pendingToolUseId: "toolu_B", screen: screenB)
        check(N.prompt(unknownCall, writes: true) == N.screenPrompt(screenB), "security: a log call without tool and command is not trusted for the box")

        // The hub: a client holding A while the screen shows B.
        let b = FakeHubBackend(workspaces: ws(.blocked))
        b.items["s1"] = [ConversationItem(id: "i1", kind: .permissionRequest(PermissionRequest(title: "Bash", command: "npm run build", toolName: "Bash", toolUseId: "toolu_A")))]
        b.screens["w1:p1"] = screenB
        let s = await session(b)
        let r = await s.receive(msg(#"{"v":1,"type":"approve","id":"c1","pane":"w1:p1","prompt":"toolu_A","allow":true}"#))
        check(code(r) == .stale && b.approved.isEmpty, "security: hub approve with the lagging log id -> stale, nothing sent")
        // Re-check right before the keys: the backend refuses when the prompt changed in between.
        b.approveOK = false
        let id = N.screenPrompt(screenB) ?? ""
        let r2 = await s.receive(msg(#"{"v":1,"type":"approve","id":"c2","pane":"w1:p1","prompt":"\#(id)","allow":true}"#))
        check(code(r2) == .stale && b.approved == ["w1:p1 \(id) true"], "security: backend re-check failed -> stale (\(b.approved))")
    }

    // (3) The re-check before the keys, on the wire: reads, then sends only when the bound prompt is still open.
    do {
        let fake = FakeHerdr(path: NSTemporaryDirectory() + "hksec-\(getpid()).sock")
        try fake.start()
        defer { fake.kill() }
        let client = HerdrClient(session: HerdrSession(name: "fake", socketPath: fake.path))
        let screenA = box("npm run build")
        fake.screen = screenA
        let idA = N.screenPrompt(screenA) ?? ""
        let sent = try await client.respondIfOpen(paneId: "w1:p1", prompt: idA, allow: true, agentKind: "claude", blocked: true, pending: nil)
        check(sent && fake.requests.last?.method == "pane.send_text", "security: respondIfOpen sends when the prompt is still open")
        fake.screen = box("rm -rf /")
        let n = fake.requests.count
        let again = try await client.respondIfOpen(paneId: "w1:p1", prompt: idA, allow: true, agentKind: "claude", blocked: true, pending: nil)
        check(!again && fake.requests.dropFirst(n).allSatisfy { $0.method == "pane.read" }, "security: respondIfOpen sends nothing when another prompt is open")
    }

    // (4) Text from a client: no control characters (except newline and tab), no DEL, no C1, length capped.
    do {
        let bad = ["a\u{1b}[201~b", "a\rb", "a\u{0}b", "a\u{7f}b", "a\u{9b}b", "a\u{3}b", String(repeating: "x", count: Hub.maxSendText + 1)]
        for t in bad { check(Hub.inputProblem(t, limit: Hub.maxSendText) != nil, "security: text \(t.prefix(12).debugDescription)… refused") }
        check(Hub.inputProblem("line one\nline two\tok — ünïcode 🙂", limit: Hub.maxSendText) == nil, "security: newline, tab and Unicode are fine")
        let b = FakeHubBackend(workspaces: ws(.idle))
        b.screens["w1:p1"] = "> "
        let s = await session(b)
        let r = await s.receive(msg(#"{"v":1,"type":"send","id":"c1","pane":"w1:p1","text":"hi\u001b[201~\rrm -rf /\r"}"#))
        check(code(r) == .badRequest && b.sent.isEmpty, "security: send with ESC/CR -> badRequest, nothing typed")
        let qb = FakeHubBackend(workspaces: ws(.blocked))
        qb.screens["w1:p1"] = """
         ←  ☐ Target  ✔ Submit  →

        Which target?

        ❯ 1. Staging
          2. Production
          3. Type something.
        ──────────────────────────
          Chat about this
        """
        let qs = await session(qb)
        let w = await qs.receive(msg(#"{"v":1,"type":"watch","id":"c0","pane":"w1:p1"}"#))
        let qid = w.compactMap { if case .prompt(let p) = $0 { p.question?.id } else { nil } }.first ?? ""
        let a = await qs.receive(msg(#"{"v":1,"type":"answer","id":"c2","pane":"w1:p1","question":"\#(qid)","answers":[{"picks":[],"other":"x\u001bq"}]}"#))
        check(!qid.isEmpty && code(a) == .badRequest && qb.answered.isEmpty, "security: answer.other with ESC -> badRequest, nothing chosen")
    }

    // (5) A permission prompt the parser cannot read still counts as open (fail closed).
    do {
        let odd = "────────────────\n Do you want to make this edit?"   // the options are below the visible rows
        check(N.screenPrompt(odd) == nil, "security: (setup) the parser does not read this prompt")
        let b = FakeHubBackend(workspaces: ws(.blocked))
        b.screens["w1:p1"] = odd
        let s = await session(b)
        let r = await s.receive(msg(#"{"v":1,"type":"send","id":"c1","pane":"w1:p1","text":"hello"}"#))
        check(code(r) == .promptOpen && b.sent.isEmpty, "security: unreadable prompt on screen -> send refused (promptOpen)")
        let ap = await s.receive(msg(#"{"v":1,"type":"approve","id":"c2","pane":"w1:p1","prompt":"x","allow":true}"#))
        check(code(ap) == .stale && b.approved.isEmpty, "security: unreadable prompt -> approve stale")
        // Prose that merely asks is not a prompt: an idle agent's last line.
        let idle = FakeHubBackend(workspaces: ws(.idle))
        idle.screens["w1:p1"] = "Done. Do you want me to open a PR?\n\n> "
        let si = await session(idle)
        check(isOK(await si.receive(msg(#"{"v":1,"type":"send","id":"c3","pane":"w1:p1","text":"yes"}"#))), "security: an idle agent asking in prose still takes a message")
    }

    // (7) Revoke closes that device's live connections at once.
    do {
        let b = FakeHubBackend(workspaces: ws(.idle))
        let ds = MemoryDeviceStore()
        ds.add(token: "t1", device: HubDevice(id: "d1", name: "A", level: .read))
        ds.add(token: "t2", device: HubDevice(id: "d2", name: "B", level: .read))
        let server = HubServer(backend: b, devices: ds, tickInterval: .seconds(30))
        let port = try await server.start(host: "127.0.0.1", port: 0)
        let c1 = HubTestClient(port: port), c2 = HubTestClient(port: port)
        try await c1.send(#"{"v":1,"type":"hello","id":"h","token":"t1","client":"t","clientVersion":"1"}"#)
        try await c2.send(#"{"v":1,"type":"hello","id":"h","token":"t2","client":"t","clientVersion":"1"}"#)
        _ = await c1.untilReply(); _ = await c2.untilReply()
        ds.revoke(device: "d1")
        server.dropRevoked()
        try? await Task.sleep(for: .milliseconds(300))
        check(server.connectionCount == 1, "security: revoke closes the device's connection at once (\(server.connectionCount) left)")
        try? await c2.send(#"{"v":1,"type":"ping","id":"p"}"#)
        check(await c2.untilReply().last.map { if case .reply(let r) = $0 { r.ok } else { false } } == true, "security: other devices stay connected")
        c1.close(); c2.close(); server.stop()
    }

    // (8) Wrong pairing tries count per source: one peer cannot burn the owner's offer; a global cap stays.
    do {
        let p = HubPairing(devices: MemoryDeviceStore())
        let o = p.offer()
        for i in 0..<10 { _ = p.redeem("wrong \(i)", name: "x", source: "10.0.0.66") }
        check(p.open == o, "security: one peer's wrong tries leave the offer open")
        check(p.redeem(o.secret, name: "x", source: "10.0.0.66") == nil, "security: that peer is refused for this offer, even with the secret")
        check(p.redeem(o.code, name: "Phone", source: "100.64.0.7") != nil, "security: the owner's phone still pairs")
        let o2 = p.offer()
        for i in 0..<HubPairing.maxTries { _ = p.redeem("w\(i)", name: "x", source: "peer-\(i)") }
        check(p.open == nil && p.redeem(o2.secret, name: "x", source: "owner") == nil, "security: \(HubPairing.maxTries) wrong tries in all end the offer")
    }
}

// Second review of the fixes: each check failed before its fix.
@MainActor func checksHubSecurityReview2() async throws {
    typealias N = ApprovalNotice
    func msg(_ json: String) -> Data { Data(json.utf8) }
    func code(_ ms: [HubMessage]) -> HubError.Code? { ms.last.flatMap { if case .reply(let r) = $0 { r.error?.code } else { nil } } }
    func isOK(_ ms: [HubMessage]) -> Bool { ms.last.map { if case .reply(let r) = $0 { r.ok } else { false } } ?? false }
    func box(_ command: String) -> String {
        "────────────────────────────────────────\n Bash command\n Run it\n╌╌╌╌╌╌╌╌╌╌\n \(command)\n╌╌╌╌╌╌╌╌╌╌\n"
            + " Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel · Tab to amend"
    }
    // A Claude picker that is not a permission prompt (seen live after a turn): typed text + Enter would pick "Yes".
    let setupPicker = """
    ────────────────────────────────────────
    ❯
    ────────────────────────────────────────
      ⏵⏵ auto mode on (shift+tab to cycle)
    ────────────────────────────────────────
      Teach auto mode about your environment?
      Auto mode works better when it knows your environment. Takes about a minute.
      ❯ 1. Yes
        2. Not now
        3. Don't show again
      Enter to confirm · Esc to cancel
    """
    let devices = MemoryDeviceStore()
    devices.add(token: "tok", device: HubDevice(id: "d1", name: "Phone", level: .input))
    func ws(_ status: AgentStatus) -> [Workspace] {
        let a = Agent(name: "claude", kind: "claude", status: status, sessionId: "s1", title: "t")
        return [Workspace(id: "w1", label: "w", tabs: [Tab(id: "w1:t1", label: "t", number: 1,
                          panes: [Pane(id: "w1:p1", terminalId: "term_1", cwd: "/w", agent: a)], status: status)])]
    }
    func session(_ b: FakeHubBackend) async -> HubSession {
        let s = HubSession(backend: b, devices: devices)
        _ = await s.receive(msg(#"{"v":1,"type":"hello","id":"h","token":"tok","client":"t","clientVersion":"1"}"#))
        return s
    }

    // (a) The in-app card: it binds the request it shows, and shows the screen prompt when the log lags.
    do {
        let screenB = box("rm -rf build")
        let onScreen = N.screenRequest(screenB)
        let lagging = PermissionRequest(title: "Bash", command: "npm run build", toolName: "Bash", toolUseId: "toolu_A")
        let caughtUp = PermissionRequest(title: "Bash", command: "rm -rf build", toolName: "Bash", toolUseId: "toolu_B")
        check(N.cardRequest(logged: lagging, onScreen: onScreen)?.toolUseId == N.screenPrompt(screenB),
              "review2: log lags the screen -> the card shows (and binds) the screen prompt")
        check(N.cardRequest(logged: caughtUp, onScreen: onScreen)?.toolUseId == "toolu_B", "review2: log matches the box -> the card shows the log's call")
        check(N.cardRequest(logged: lagging, onScreen: nil)?.toolUseId == "toolu_A", "review2: no box read -> the log's call (the keys still need the box)")
        let st = N.PaneState(agentKind: "claude", blocked: true, pendingToolUseId: "toolu_A", screen: screenB, pendingTool: "Bash", pendingCommand: "npm run build")
        let bound = N.cardRequest(logged: lagging, onScreen: onScreen)?.toolUseId
        check(N.outcome(actionIdentifier: N.Action.allow.rawValue, boundPrompt: bound, now: st, writes: true) == .send(allow: true),
              "review2: the card's bound id passes the re-check")
    }

    // (b) A screen that cannot be read refuses every input (fail closed), with nothing sent.
    do {
        let b = FakeHubBackend(workspaces: ws(.idle))   // no screen: the read fails
        let s = await session(b)
        let r = await s.receive(msg(#"{"v":1,"type":"send","id":"c1","pane":"w1:p1","text":"hi"}"#))
        check(code(r) == .failed && b.sent.isEmpty, "review2: unreadable screen -> send failed, nothing typed (\(String(describing: code(r))))")
        let bb = FakeHubBackend(workspaces: ws(.blocked))
        let sb = await session(bb)
        let ap = await sb.receive(msg(#"{"v":1,"type":"approve","id":"c2","pane":"w1:p1","prompt":"x","allow":true}"#))
        check(code(ap) == .failed && bb.approved.isEmpty, "review2: unreadable screen -> approve failed (\(String(describing: code(ap))))")
        let an = await sb.receive(msg(#"{"v":1,"type":"answer","id":"c3","pane":"w1:p1","question":"q","answers":[{"picks":[1]}]}"#))
        check(code(an) == .failed && bb.answered.isEmpty, "review2: unreadable screen -> answer failed (\(String(describing: code(an))))")
    }

    // (c) A picker on screen takes typed text as a choice: send refuses it, before and right before the keys.
    do {
        check(Hub.screenTakesKeys(setupPicker), "review2: a numbered picker with its key line takes keys")
        check(Hub.screenTakesKeys(box("ls")), "review2: a permission prompt takes keys")
        check(!Hub.screenTakesKeys("Done. Two ways on:\n1. Ship it\n2. Wait\n\n> "), "review2: prose with a list is not a picker")
        check(!Hub.screenTakesKeys("> "), "review2: an empty input line is not a picker")
        let b = FakeHubBackend(workspaces: ws(.idle))
        b.screens["w1:p1"] = setupPicker
        let s = await session(b)
        let r = await s.receive(msg(#"{"v":1,"type":"send","id":"c1","pane":"w1:p1","text":"hi"}"#))
        check(code(r) == .promptOpen && b.sent.isEmpty, "review2: idle agent with a picker on screen -> promptOpen")
        // The backend's own re-read found a change: promptOpen, not ok.
        b.screens["w1:p1"] = "> "
        b.sendResult = .changed
        let r2 = await s.receive(msg(#"{"v":1,"type":"send","id":"c2","pane":"w1:p1","text":"hi"}"#))
        check(code(r2) == .promptOpen, "review2: pane changed before the keys -> promptOpen (\(String(describing: code(r2))))")
        // The backend held the message back (a screen command alone, the agent's box has text): failed, and the reply's
        // message is the backend's note, word for word.
        for (n, note) in [AgentBox.notEmpty, "/status opens a screen in the terminal. Use it there."].enumerated() {
            b.sendResult = .refused(note)
            let r = await s.receive(msg(#"{"v":1,"type":"send","id":"h\#(n)","pane":"w1:p1","text":"hi"}"#))
            let said: String? = if case .reply(let reply)? = r.last { reply.error?.message } else { nil }
            check(code(r) == .failed && said == note, "held back by the backend -> failed with its note (\(String(describing: code(r))), \(said ?? "nil"))")
        }
        b.sendResult = .sent

        // On the wire: a fresh snapshot and screen right before the keys.
        let fake = FakeHerdr(path: NSTemporaryDirectory() + "hkrr-\(getpid()).sock")
        try fake.start()
        defer { fake.kill() }
        let client = HerdrClient(session: HerdrSession(name: "fake", socketPath: fake.path))
        fake.screen = "> "
        check(await !client.readyForText(paneId: "w1:p1"), "review2: no agent in the pane now -> not ready")
        fake.agents = #"[{"pane_id":"w1:p1","agent":"claude","name":"claude","agent_status":"blocked"}]"#
        check(await !client.readyForText(paneId: "w1:p1"), "review2: agent blocked now -> not ready")
        fake.agents = #"[{"pane_id":"w1:p1","agent":"claude","name":"claude","agent_status":"idle"}]"#
        check(await client.readyForText(paneId: "w1:p1"), "review2: idle agent, plain screen -> ready")
        fake.screen = setupPicker
        check(await !client.readyForText(paneId: "w1:p1"), "review2: picker on screen now -> not ready")
        check(fake.requests.allSatisfy { ["session.snapshot", "pane.read"].contains($0.method) }, "review2: the re-check only reads")
    }
}
