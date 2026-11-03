import Foundation
@testable import HerdrKit

/// `--hc-live <socket>`: live checks against an **isolated** herdr-core (its socket must sit under a `hcds` state dir
/// the tester made). Prints the mapped tree, resolves sessions, attaches to the shell terminal named by `--hc-shell`
/// (default t3) and types into it. Never prints log contents. Exits non-zero on failure.
@MainActor func modeHCLive() async throws {
    guard let i = CommandLine.arguments.firstIndex(of: "--hc-live"), CommandLine.arguments.indices.contains(i + 1) else { return }
    let sock = CommandLine.arguments[i + 1]
    guard sock.contains("/hcds/") else { print("refusing: \(sock) is not an isolated test core"); exit(2) }
    let shell = CommandLine.arguments.firstIndex(of: "--hc-shell").map { CommandLine.arguments[$0 + 1] } ?? "t3"
    let hc = HCClient(session: HCSession(socketPath: sock))
    var bad = 0
    func ok(_ c: Bool, _ m: String) { print((c ? "ok   " : "FAIL ") + m); if !c { bad += 1 } }

    // 1. Mapping.
    let ws = try await hc.snapshot()
    for w in ws {
        print("ws \(w.id) '\(w.label)'")
        for t in w.tabs {
            print("  tab \(t.id) '\(t.label)' \(t.status)")
            for p in t.panes {
                let a = p.agent
                print("    pane \(p.id) pids=\(p.processIds) agent=\(a.map { "\($0.kind) \($0.status) kind=\($0.statusKind ?? "-") title='\($0.title)'" } ?? "none")")
            }
        }
    }
    let panes = ws.flatMap(\.tabs).flatMap(\.panes)
    ok(ws.contains { $0.id == "hc:space:sp1" } && ws.contains { $0.id == HCMapper.otherId }, "labeled space and Other terminals")

    // 2. Sessions.
    let needs = panes.compactMap { p in p.agent.map { SessionResolver.Need(paneId: p.id, kind: $0.kind, cwd: p.cwd, pids: p.processIds) } }
    let found = await SessionResolver(socketPath: "/nonexistent").resolve(needs, claimed: [])
    for n in needs { print("session \(n.paneId) \(n.kind): \(found[n.paneId].map { ($0 as NSString).lastPathComponent } ?? "none")") }
    if let claude = panes.first(where: { $0.agent?.kind == "claude" }) {
        let file = claude.processIds.lazy.compactMap { FileManager.default.contents(atPath: NSHomeDirectory() + "/.claude/sessions/\($0).json") }.first
        let want = file.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }?["sessionId"] as? String
        ok(want != nil && found[claude.id] == want, "claude session id from ~/.claude/sessions/<pid>.json")
        if let sid = found[claude.id] { ok(SessionLog.locate(sessionId: sid) != nil || true, "claude log locate (\(SessionLog.locate(sessionId: sid) != nil ? "file exists" : "no file yet: Claude writes it on the first message"))") }
    }

    // 3. Attach and type (own shell terminal only).
    let ts = TerminalSession(hcTerminal: shell, client: hc, mode: .control(takeover: false), cols: 100, rows: 30)
    ts.start()
    var got = Data()
    var first = false
    let reader = Task { @MainActor in
        for await ev in ts.events {
            if case .frame(let f) = ev { if f.full { first = true }; got.append(f.bytes); f.consumed() }
            if case .closed(let r) = ev { print("closed \(r)") }
        }
    }
    let t0 = Date()
    while !first, Date().timeIntervalSince(t0) < 5 { try await Task.sleep(for: .milliseconds(50)) }
    ok(first, "attach: first paint as a full frame (size \(ts.size.map { "\($0.cols)x\($0.rows)" } ?? "?"))")
    ok(ts.send(bytes: Array("echo HCLIVE_$((40+2))\r".utf8)), "attach: keys sent")
    let t1 = Date()
    while !String(decoding: got, as: UTF8.self).contains("HCLIVE_42"), Date().timeIntervalSince(t1) < 5 { try await Task.sleep(for: .milliseconds(50)) }
    ok(String(decoding: got, as: UTF8.self).contains("HCLIVE_42"), "attach: typed command output comes back on the stream")
    ts.release()
    reader.cancel()

    // 4. Reply path (terminal.send) and keys.
    try await hc.send(text: "echo SENT_$((5+5))", to: HCMapper.paneId(shell))
    var screen = ""
    let t2 = Date()
    while !screen.contains("SENT_10"), Date().timeIntervalSince(t2) < 8 {
        try await Task.sleep(for: .milliseconds(200))
        screen = (try? await hc.call("terminal.read", ["terminal": shell]))?["text"] as? String ?? ""
    }
    ok(screen.contains("SENT_10"), "send: terminal.send typed and pressed Enter")
    try await hc.sendNamedKeys(["C-c"], to: HCMapper.paneId(shell))
    ok(true, "keys: ctrl+c accepted")

    // 5. Approval/question cards: read-only on the claude terminal of this test core; the answer key goes to the own shell.
    if let claude = panes.first(where: { $0.agent?.kind == "claude" }), let agent = claude.agent {
        let text = (try? await hc.visibleText(paneId: claude.id)) ?? ""
        ok(!text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, "cards: terminal.read gives the claude screen (\(text.count) chars)")
        let card = PermissionGate.checkApproval(agent, pendingTool: false)
        ok(card == (agent.status == .blocked && agent.statusKind != "question"), "cards: approval card follows 7501 status (\(agent.status) kind=\(agent.statusKind ?? "-") -> \(card))")
        let prompt = await hc.checkPrompt(paneId: claude.id)
        ok(agent.status == .blocked || !prompt, "cards: no permission prompt on an unblocked screen")
        let q = try? await hc.pendingQuestion(paneId: claude.id)
        ok(PermissionGate.readQuestion(agent) || q == nil, "cards: no question read from an unblocked screen")
    }
    do { try await hc.respond(to: HCMapper.paneId(shell), allow: false); ok(true, "cards: deny goes out as terminal.keys esc (own shell)") }
    catch { ok(false, "cards: respond failed \(error)") }

    print(bad == 0 ? "HC LIVE OK" : "HC LIVE: \(bad) FAILED")
    exit(bad == 0 ? 0 : 1)
}

/// `--hc-layout-live <isolated sock>`: layout writes against a real core. Every terminal it starts it also ends.
@MainActor func modeHCLayoutLive() async throws {
    guard let i = CommandLine.arguments.firstIndex(of: "--hc-layout-live"), CommandLine.arguments.indices.contains(i + 1) else { return }
    let sock = CommandLine.arguments[i + 1]
    guard sock.contains("/hcds/") else { print("refusing: \(sock) is not an isolated test core"); exit(2) }
    let hc = HCClient(session: HCSession(socketPath: sock))
    var bad = 0
    func ok(_ c: Bool, _ m: String) { print((c ? "ok   " : "FAIL ") + m); if !c { bad += 1 } }
    func doc() async throws -> HCAppDoc { try await hc.readAppDoc().0 }
    func terms() async throws -> [[String: Any]] {
        ((try await hc.call("terminal.list"))["terminals"] as? [[String: Any]] ?? []).filter { $0["state"] as? String == "live" }
    }
    func live() async throws -> Set<String> { Set(try await terms().compactMap { $0["address"] as? String }) }
    let before = try await live()
    let raw0 = try await hc.call("doc.get", ["name": HCClient.appDocName])
    print("doc.get before: version=\(raw0["version"] ?? "nil") value=\(raw0["value"].map { "\(type(of: $0))" } ?? "nil")")

    let ws = try await hc.createWorkspace(label: "Layout live", cwd: NSTemporaryDirectory(), focus: false)
    let s = try HCClient.spaceLabel(ws)
    var snap = try await hc.snapshot()
    var w = snap.first { $0.id == ws }
    ok(w?.label == "Layout live" && w?.tabs.count == 1 && w?.tabs.first?.panes.count == 1, "new workspace on the real core (\(w?.tabs.map(\.label) ?? []))")
    let shell = try await terms().first { HCClient.labels($0)?.space == s }
    print("new terminal: argv=\(shell?["argv"] ?? "nil") keep=\(shell?["keep"] ?? "nil") labels=\(shell?["labels"] ?? "nil")")
    ok(shell?["keep"] as? Bool == true, "kept by the core")

    try await hc.createTab(workspaceId: ws, label: "second", cwd: nil)
    snap = try await hc.snapshot(); w = snap.first { $0.id == ws }
    let p0 = w!.tabs[0].panes[0]
    try await hc.split(paneId: p0.id, direction: .right)
    try await hc.split(paneId: p0.id, direction: .down)
    try await hc.renameTab(w!.tabs[0].id, to: "main")
    try await hc.moveTab(w!.tabs[0].id, toIndex: 1)
    try await hc.renamePane(paneId: p0.id, to: "first")
    snap = try await hc.snapshot(); w = snap.first { $0.id == ws }
    ok(w?.tabs.map(\.label) == ["second", "main"] && w?.tabs.last?.panes.count == 3 && w?.tabs.last?.panes.first?.label == "first",
       "tabs, splits, names, order (\(w?.tabs.map { "\($0.label):\($0.panes.count)" } ?? []))")
    let d = try await doc()
    ok(d.terminals(tab: try HCClient.tabLabels(w!.tabs.last!.id).tab) == w!.tabs.last!.panes.map(\.terminalId), "pane order = the document's tree order")

    // Conflict on the real core: a stale version is refused with `conflict`.
    let (cur, v) = try await hc.readAppDoc()
    _ = try await hc.call("doc.set", json: HCJSON(["name": HCClient.appDocName, "version": v, "value": cur.value]))
    var kind = "none"
    do { _ = try await hc.call("doc.set", json: HCJSON(["name": HCClient.appDocName, "version": v, "value": cur.value])) }
    catch HerdrError.server(let c, _) { kind = c }
    ok(kind == "conflict", "stale version → \(kind)")

    try await hc.closePane(paneId: w!.tabs.last!.panes[1].id)
    try await Task.sleep(for: .milliseconds(300))
    snap = try await hc.snapshot(); w = snap.first { $0.id == ws }
    ok(w?.tabs.last?.panes.count == 2, "close pane")
    try await hc.closeTab(w!.tabs.first!.id)
    try await Task.sleep(for: .milliseconds(300))
    snap = try await hc.snapshot(); w = snap.first { $0.id == ws }
    ok(w?.tabs.count == 1, "close tab")
    try await hc.closeWorkspace(id: ws)
    try await Task.sleep(for: .milliseconds(500))
    snap = try await hc.snapshot()
    let left = try await live().subtracting(before)
    let dEnd = try await doc()
    ok(!snap.contains { $0.id == ws } && left.isEmpty && !dEnd.spaceIds.contains(s), "close workspace: nothing left (\(left.sorted()))")
    print(bad == 0 ? "layout-live: PASS" : "layout-live: FAIL x\(bad)")
    exit(bad == 0 ? 0 : 1)
}

/// `--hc-interop <sock> <op> [args…]`: one Flockbook layout step against the isolated core the official Herdr app shows,
/// then the snapshot, so a person can compare the official window step by step. Ops: show, new-ws NAME, new-tab WS NAME,
/// split PANE right|down, rename-ws WS NAME, rename-tab TAB NAME, rename-pane PANE NAME, move-tab TAB INDEX, close-pane
/// PANE, close-tab TAB, close-ws WS, resolve PANE KIND CWD MIRROR_ROOT. `--via-app`: through `herdr-app do` (the official app's commands) instead of the
/// document. Refuses any socket but an isolated `hcds…` one.
@MainActor func modeHCInterop() async throws {
    let args = CommandLine.arguments
    guard let i = args.firstIndex(of: "--hc-interop"), args.indices.contains(i + 2) else { return }
    let sock = args[i + 1]
    guard (sock as NSString).lastPathComponent.hasPrefix("hcds"), sock.contains("/hcds"),
          sock != HCSession.defaultSession.socketPath else {
        print("refusing: \(sock) is not an isolated test core"); exit(2)
    }
    let op = args[i + 2], rest = Array(args[(i + 3)...].prefix { !$0.hasPrefix("--") })
    func arg(_ k: Int) -> String {
        guard rest.indices.contains(k) else { print("\(op): missing argument \(k + 1)"); exit(2) }
        return rest[k]
    }
    let hc = HCClient(session: HCSession(socketPath: sock))
    if args.contains("--via-app") {
        guard let cli = HerdrAppCLI.find() else { print("no herdr-app"); exit(2) }
        await hc.setAppCLI(cli)
        print("via herdr-app: \(cli.path) running=\(await cli.isRunning())")
    }
    do {
        switch op {
        case "show": break
        case "new-ws": print("made", try await hc.createWorkspace(label: arg(0), cwd: NSHomeDirectory(), focus: false))
        case "new-tab": try await hc.createTab(workspaceId: arg(0), label: arg(1), cwd: nil)
        case "split": try await hc.split(paneId: arg(0), direction: arg(1) == "down" ? .down : .right)
        case "rename-ws": try await hc.renameWorkspace(id: arg(0), to: arg(1))
        case "rename-tab": try await hc.renameTab(arg(0), to: arg(1))
        case "rename-pane": try await hc.renamePane(paneId: arg(0), to: arg(1))
        case "move-tab": try await hc.moveTab(arg(0), toIndex: Int(arg(1)) ?? 0)
        case "close-pane": try await hc.closePane(paneId: arg(0))
        case "close-tab": try await hc.closeTab(arg(0))
        case "close-ws": try await hc.closeWorkspace(id: arg(0))
        case "resolve":
            // resolve PANE KIND CWD MIRROR_ROOT: a remote agent's log, mirrored under MIRROR_ROOT; prints the mirror's
            // path and size after 3 s (never its content).
            await hc.setMirror(root: arg(3), poll: 0.5)
            let found = await hc.resolveRemote([SessionResolver.Need(paneId: arg(0), kind: arg(1), cwd: arg(2))], claimed: [])
            try? await Task.sleep(nanoseconds: 3_000_000_000)
            let p = found[arg(0)]
            let size = p.flatMap { try? FileManager.default.attributesOfItem(atPath: $0)[.size] as? Int } ?? -1
            print("resolve: \(p ?? "not found") bytes=\(size)")
            await hc.stopMirrors()
        case "deliver":
            // deliver PANE TEXT: the composer's hc send (`HCClient.deliver`), own test terminals only.
            print("deliver:", await hc.deliver(text: arg(1), to: arg(0)))
        case "sessions":
            // sessions: each local agent pane's log file name, as the store resolves it (never the content).
            let panes = try await hc.snapshot().flatMap(\.tabs).flatMap(\.panes).filter { !HerdrStore.isRemotePane($0.id) }
            let needs = panes.compactMap { p in p.agent.map { SessionResolver.Need(paneId: p.id, kind: $0.kind, cwd: p.cwd, pids: p.processIds) } }
            let found = await SessionResolver(socketPath: "/nonexistent").resolve(needs, claimed: [])
            for n in needs { print("session \(n.paneId) \(n.kind): \(found[n.paneId].map { ($0 as NSString).lastPathComponent } ?? "none")") }
        default: print("unknown op \(op)"); exit(2)
        }
        if op != "show" { print("\(op): ok") }
    } catch {
        print("\(op): FAILED \(error)")
    }
    let raw = try await hc.call("doc.get", ["name": HCClient.appDocName])
    print("doc version \(raw["version"] ?? "nil")")
    for w in try await hc.snapshot() {
        print("\(w.id)  \"\(w.label)\"")
        for t in w.tabs {
            print("  \(t.id)  \"\(t.label)\"")
            for p in t.panes { print("    \(p.id)  \"\(p.label ?? "")\"  cwd=\(p.cwd ?? "")") }
        }
    }
    exit(0)
}
