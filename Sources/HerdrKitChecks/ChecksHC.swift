import Foundation
@testable import HerdrKit

// herdr-core (hc) backend: framed codec, session paths, plain and framed RPC, attach with credit. FakeHC only.

func fakeHCPath() -> String { NSTemporaryDirectory() + "fhc-\(UInt32.random(in: 0...UInt32.max)).sock" }

/// Polls `cond` for up to `seconds`.
@MainActor func eventually(_ seconds: Double = 2, _ cond: () -> Bool) async -> Bool {
    let end = Date().addingTimeInterval(seconds)
    while Date() < end { if cond() { return true }; try? await Task.sleep(for: .milliseconds(10)) }
    return cond()
}

@MainActor func checksHCCodec() async throws {
    // Header layout: channel u32 LE, kind u8, len u32 LE, payload.
    let f = HCFrame(channel: 0x0102_0304, kind: .credit, payload: Data([9, 8]))
    check([UInt8](f.encoded()) == [4, 3, 2, 1, 1, 2, 0, 0, 0, 9, 8], "frame header is little endian")
    check([UInt8](HCFrame.credit(channel: 5, bytes: 300).payload) == [44, 1, 0, 0], "credit payload u32 LE")
    check([UInt8](HCFrame.resize(channel: 5, cols: 120, rows: 40).payload) == [120, 0, 40, 0], "resize payload u16 LE cols, rows")
    check(HCFrame.close(channel: 5).kind == .close && HCFrame.close(channel: 5).payload.isEmpty, "close frame is empty")

    // Decoder: split across arbitrary chunks, several frames in one chunk.
    let a = HCFrame(channel: 0, kind: .data, payload: Data(#"{"jsonrpc":"2.0","id":1}"#.utf8))
    let b = HCFrame(channel: 7, kind: .data, payload: Data([1, 2, 3]))
    let c = HCFrame(channel: 7, kind: .close, payload: Data())
    let all = [UInt8](a.encoded() + b.encoded() + c.encoded())
    var dec = HCFrameDecoder()
    var got: [HCFrame] = []
    for byte in all { got += try dec.feed([byte][...]) }
    check(got == [a, b, c], "decoder rebuilds frames fed one byte at a time")
    var dec2 = HCFrameDecoder()
    check(try dec2.feed(all[...]) == [a, b, c], "decoder splits one chunk into frames")

    // Oversize payload is a protocol error, not a 4 GB allocation.
    var dec3 = HCFrameDecoder()
    var bad = [UInt8](repeating: 0, count: 5); bad += [0x01, 0x00, 0x00, 0x01]   // len = 16 MiB + 1
    var threw = false
    do { _ = try dec3.feed(bad[...]) } catch { threw = true }
    check(threw, "payload over 16 MiB is rejected")
    var dec4 = HCFrameDecoder()
    var unk = [UInt8](repeating: 0, count: 4); unk += [99, 0, 0, 0, 0]
    check((try? dec4.feed(unk[...]))?.first?.kind == .unknown(99), "unknown kind is kept, not fatal")
}

@MainActor func checksHCSession() async throws {
    let home = NSHomeDirectory()
    check(HCSession.resolve(environment: [:]).socketPath == home + "/.local/state/herdr-core/default/default.sock", "default socket in state dir")
    check(HCSession.resolve(environment: ["HC_SESSION": "t1"]).socketPath == home + "/.local/state/herdr-core/t1/t1.sock", "HC_SESSION names the session")
    check(HCSession.resolve(environment: ["XDG_RUNTIME_DIR": "/r"]).socketPath == "/r/herdr-core/default.sock", "XDG_RUNTIME_DIR wins over state dir")
    check(HCSession.resolve(environment: ["XDG_STATE_HOME": "/s"]).socketPath == "/s/herdr-core/default/default.sock", "XDG_STATE_HOME moves state")
    check(HCSession.resolve(environment: ["HC_SOCKET": "/x.sock", "XDG_RUNTIME_DIR": "/r"]).socketPath == "/x.sock", "HC_SOCKET wins")
    check(HCSession.resolve(environment: ["HERDR_DESK_HC_SOCKET": "/d.sock", "HC_SOCKET": "/x.sock"]).socketPath == "/d.sock", "HERDR_DESK_HC_SOCKET wins over HC_SOCKET")
    check(HCSession.resolve(environment: [:], arguments: ["app", "--hc-socket", "/a.sock"]).socketPath == "/a.sock", "--hc-socket wins")
    check(!HCSession(socketPath: "/nonexistent/hc.sock").isAvailable, "missing socket is not available")
}

@MainActor func checksHCTransport() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1", status: "idle")]
    let client = HCClient(session: HCSession(socketPath: fake.path))

    // Plain mode.
    let r = try await client.call("terminal.list")
    check((r["terminals"] as? [[String: Any]])?.first?["address"] as? String == "t1", "plain call returns result")
    do { _ = try await client.call("terminal.get", ["terminal": "t9"]); check(false, "error thrown") }
    catch HerdrError.server(let code, let msg) { check(code == "not_found" && msg.contains("t9"), "JSON-RPC error maps to kind + message") }
    catch { check(false, "unexpected error \(error)") }
    check(fake.calls.first?.method == "terminal.list", "fake saw the call")

    // Framed mode: channel-0 calls, then an attach stream.
    let conn = try await HCFramedConnection.open(socketPath: fake.path)
    defer { conn.shutdown() }
    let r2 = try await conn.call("terminal.list")
    check((r2["terminals"] as? [[String: Any]])?.count == 1, "framed call on channel 0")
    async let p1 = conn.call("terminal.get", ["terminal": "t1"])
    async let p2 = conn.call("terminal.list")
    let (g1, g2) = try await (p1, p2)
    check(g1["address"] as? String == "t1" && g2["terminals"] != nil, "concurrent framed calls are matched by id")

    let att = try await conn.attach(terminal: "t1", cols: 100, rows: 30)
    check(att.channel >= 1, "attach returns a channel")
    var it = att.bytes.makeAsyncIterator()
    let paint = try await it.next()
    check(paint == fake.firstPaint, "first paint arrives on the channel")
    att.consumed(paint?.count ?? 0)
    check(await eventually { fake.credits(att.channel) == fake.firstPaint.count }, "CREDIT reports consumed bytes")
    fake.output(att.channel, Data("more".utf8))
    check(try await it.next() == Data("more".utf8), "live output follows")
    att.send(Data("ls\r".utf8))
    att.resize(cols: 90, rows: 20)
    check(await eventually { fake.viewerBytes(att.channel) == Data("ls\r".utf8) }, "keys go as DATA on the channel")
    check(await eventually { fake.resizes(att.channel).last.map { $0 == (90, 20) } ?? false }, "RESIZE sent")
    fake.exitProgram(att.channel)
    let end = try await it.next()
    check(end == nil, "CLOSE from core ends the stream")
    check(att.closeReason?["reason"] as? String == "exited", "close reason kept")

    // Viewer-side close.
    let att2 = try await conn.attach(terminal: "t1", cols: 80, rows: 24)
    att2.close()
    check(await eventually { fake.closedByViewer(att2.channel) }, "viewer CLOSE reaches the core")

    // Attach to a missing terminal fails cleanly.
    do { _ = try await conn.attach(terminal: "t9", cols: 80, rows: 24); check(false, "attach t9 throws") }
    catch HerdrError.server(let code, _) { check(code == "not_found", "attach error kind") }
}

// Step 2: hc terminals → the app's model (read-only grouping by herdr.space / herdr.tab labels).
@MainActor func checksHCMapping() async throws {
    let lab = { (s: String, t: String) in ["herdr.space": s, "herdr.tab": t] }
    let terms: [[String: Any]] = [
        FakeHC.terminal("t1", labels: lab("s1", "a"), argv: ["claude"], status: "working", turn: 3, title: "Fix login"),
        FakeHC.terminal("t2", labels: lab("s1", "a"), argv: ["zsh"], status: "blocked", kind: "permission",
                        fg: ["pid": 99, "name": "node", "argv": ["node", "/opt/x/bin/codex", "--yolo"], "cwd": "/tmp/proj/sub"]),
        FakeHC.terminal("t3", argv: ["pi"]),
        FakeHC.terminal("t4", argv: ["/bin/zsh", "-l"], cwd: "/home/me/notes"),
        FakeHC.terminal("t5", argv: ["claude"], status: "idle", parent: "t1"),
        FakeHC.terminal("t6", labels: lab("s2", "b"), argv: ["codex"], status: "error", msg: "rate limited", title: "Refactor", cwd: "/home/me/other-repo"),
        FakeHC.terminal("host2/t7", labels: lab("s1", "c"), argv: ["claude"], status: "done"),
    ]
    let doc: [String: Any] = ["spaces": [["id": "s1", "name": "Repo X", "tabs": [["id": "a", "name": "fix login"]]]]]
    let ws = HCMapper.workspaces(terminals: terms, appDoc: doc)
    // host2/t7's labels are local to host2: its space is `host2/s1`, not the local s1.
    check(ws.map(\.id) == ["hc:space:s1", "hc:space:s2", "hc:space:host2/s1", "hc:other"], "spaces in first-seen order, Other terminals last: \(ws.map(\.id))")
    check(ws.allSatisfy { $0.backend == .hc && $0.tabs.allSatisfy { $0.backend == .hc } }, "hc workspaces and tabs carry the backend")
    check(ws[0].label == "Repo X" && ws[0].tabs.first?.label == "fix login", "names from the herdr-app document")
    check(ws[0].tabs.map(\.id) == ["hc:tab:s1/a"] && ws[2].tabs.map(\.id) == ["hc:tab:host2/s1/c"], "tabs grouped by herdr.tab, per machine")
    check(ws[2].tabs[0].label == "c", "unnamed tab falls back to its id")
    check(ws[1].label == "other-repo", "unnamed space falls back to its first terminal's folder")
    let tabA = ws[0].tabs[0]
    check(tabA.panes.map(\.id) == ["hc:t1", "hc:t2", "hc:t5"], "child without labels joins its parent's tab: \(tabA.panes.map(\.id))")
    check(tabA.status == .blocked, "tab rollup")
    let p1 = tabA.panes[0]
    check(p1.backend == .hc && p1.terminalId == "t1" && p1.agent?.backend == .hc, "pane and agent know their backend")
    check(p1.agent?.kind == "claude" && p1.agent?.status == .working && p1.agent?.title == "Fix login", "claude agent fields")
    check(p1.agent?.sessionId == nil, "no session yet (resolved in step 3)")
    let p2 = tabA.panes[1]
    check(p2.agent?.kind == "codex", "kind from the foreground process (node … codex)")
    check(p2.agent?.status == .blocked && p2.agent?.statusKind == "permission", "blocked with its kind")
    check(p2.cwd == "/tmp/proj/sub", "cwd follows the foreground process")
    let err = ws[1].tabs[0].panes[0].agent
    check(err?.status == .done, "status error shows as done (turn ended; hc has no error state in the app)")
    check(err?.title == "error: rate limited" && err?.statusKind == "error", "error reads as a failure: subtitle 'error: <msg>' (\(err?.title ?? "nil"))")
    let bare = HCMapper.workspaces(terminals: [FakeHC.terminal("t8", argv: ["codex"], status: "error")], appDoc: nil)
    check(bare.first?.tabs.first?.panes.first?.agent?.title == "error", "error without msg says 'error'")
    let other = ws[3]
    check(other.label == "Other terminals" && other.tabs.map(\.id) == ["hc:term:t3", "hc:term:t4"], "unlabeled terminals: one tab each")
    check(other.tabs[0].panes[0].agent?.kind == "pi" && other.tabs[0].panes[0].agent?.status == .unknown, "pi without 7501 is unknown, not idle")
    check(other.tabs[1].panes[0].agent == nil && other.tabs[1].label == "notes", "a shell is a plain pane, tab named after its folder")
    check(ws[2].tabs[0].panes[0].id == "hc:host2/t7" && ws[2].tabs[0].panes[0].terminalId == "host2/t7", "remote terminals keep their address")

    // Broken or unknown documents never break the list.
    for bad: Any? in [nil, "garbage", ["spaces": 3], ["version": 9, "x": [1, 2, [["id": 5]]]]] {
        let w = HCMapper.workspaces(terminals: terms, appDoc: bad)
        check(w.count == 4 && w[0].label == "proj" && w[0].tabs[0].label == "a", "bad doc falls back (\(String(describing: bad)))")
    }
    // Keyed-by-id document shape.
    let keyed: [String: Any] = ["spaces": ["s1": ["name": "Keyed", "tabs": ["a": ["title": "T-A"]]]]]
    let wk = HCMapper.workspaces(terminals: terms, appDoc: keyed)
    check(wk[0].label == "Keyed" && wk[0].tabs[0].label == "T-A", "keyed document shape")
    check(HCMapper.workspaces(terminals: [], appDoc: nil).isEmpty, "no terminals: no groups")

    // Over the socket: snapshot = terminal.list + doc.get(herdr-app) read-only; events yield on change.
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = terms
    fake.setDoc("herdr-app", doc)
    let client = HCClient(session: HCSession(socketPath: fake.path))
    let snap = try await client.snapshot()
    check(snap.count == 4 && snap[0].label == "Repo X", "HCClient.snapshot maps the live list")
    check(fake.calls.contains { $0.method == "doc.get" && $0.params["name"] as? String == "herdr-app" }, "reads the herdr-app document")
    check(!fake.calls.contains { ["doc.set", "terminal.label", "terminal.kill", "terminal.rm", "terminal.run"].contains($0.method) }, "never writes")
    check(client.backend == .hc && client.supportsLayout, "hc backend has layout actions")
    let counter = EventCounter()
    let evTask = Task.detached { do { for try await _ in client.events() { await counter.bump() } } catch { await counter.fail("\(error)") } }
    defer { evTask.cancel() }
    var ok = false
    for _ in 0..<200 { if await counter.count >= 1 { ok = true; break }; try await Task.sleep(for: .milliseconds(10)) }
    check(ok, "events yields once subscribed")
    fake.pushEvent(type: "status", terminal: "t1", data: ["status": ["state": "idle"], "records": [String: Any](), "reported": true])
    ok = false
    for _ in 0..<200 { if await counter.count >= 2 { ok = true; break }; try await Task.sleep(for: .milliseconds(10)) }
    check(ok, "events yields on a status event")
    let before = fake.calls.filter { $0.method == "doc.get" }.count
    _ = try await client.snapshot()
    check(fake.calls.filter { $0.method == "doc.get" }.count == before + 1, "a new subscription refetches the document once")
    let docGets = before + 1
    _ = try await client.snapshot()
    check(fake.calls.filter { $0.method == "doc.get" }.count == docGets, "document is cached until a doc event")
    fake.pushEvent(type: "doc", terminal: nil, data: ["name": "herdr-app", "version": 2])
    ok = false
    for _ in 0..<200 { if await counter.count >= 3 { ok = true; break }; try await Task.sleep(for: .milliseconds(10)) }
    _ = try await client.snapshot()
    check(ok && fake.calls.filter { $0.method == "doc.get" }.count == docGets + 1, "doc event refetches the document")
}

// Store: both backends at once; either one missing is normal.
@MainActor func checksHCStore() async throws {
    func wait(_ sec: Double, _ ok: () -> Bool) async -> Bool {
        let t = Date(); while Date().timeIntervalSince(t) < sec { if ok() { return true }; try? await Task.sleep(for: .milliseconds(5)) }; return ok()
    }
    let tui = FakeHerdr(path: NSTemporaryDirectory() + "hkfk2-\(getpid()).sock")
    try tui.start()
    let hc = FakeHC(path: fakeHCPath())
    try hc.start()
    hc.terminals = [FakeHC.terminal("t1", labels: ["herdr.space": "s1", "herdr.tab": "a"], argv: ["claude"], status: "working")]
    let st = HerdrStore(session: HerdrSession(name: "fake", socketPath: tui.path), hc: HCSession(socketPath: hc.path))
    st.start()
    let both = await wait(3) { st.workspaces.count == 2 }
    check(both && st.workspaces.map(\.backend) == [.tui, .hc], "store lists TUI then hc workspaces: \(st.workspaces.map(\.id))")
    check(st.connected && st.tuiConnected && st.hcConnected, "both connected")
    check(st.backend(forPane: "hc:t1") === st.hcClient && st.backend(forPane: "w1:p1") === st.client, "panes route to their backend")

    let knownBefore = BlockedTabs.fresh(known: nil, in: st.workspaces).now
    // hc change arrives through its event stream.
    hc.terminals = [FakeHC.terminal("t1", labels: ["herdr.space": "s1", "herdr.tab": "a"], argv: ["claude"], status: "blocked", kind: "question")]
    hc.pushEvent(type: "status", terminal: "t1", data: ["status": ["state": "blocked", "kind": "question"]])
    check(await wait(2) { st.workspaces.last?.tabs.first?.panes.first?.agent?.status == .blocked }, "hc status event updates the store")
    // Parity item 4: the notification path (Notifier) sees the hc tab become blocked, and its ids open that tab.
    let note = BlockedTabs.fresh(known: knownBefore, in: st.workspaces)
    let opened = note.fresh.first.flatMap { ws, t in st.workspaces.first { $0.id == ws.id }?.tabs.first { $0.id == t.id } }
    check(note.fresh.map(\.1.id) == ["hc:tab:s1/a"] && opened?.backend == .hc,
          "hc blocked tab: a notification for it, and its ids find the tab again (\(note.fresh.map(\.1.id)))")
    check(BlockedTabs.fresh(known: note.now, in: st.workspaces).fresh.isEmpty, "hc blocked tab: notified once")

    // TUI goes away: hc stays, still connected. (A lost backend keeps its last snapshot, as the TUI always did.)
    tui.kill()
    check(await wait(4) { !st.tuiConnected && st.hcConnected }, "TUI loss keeps hc connected")
    check(st.connected && st.workspaces.contains { $0.backend == .hc }, "connected while one backend is up")
    // hc goes away too.
    hc.kill()
    check(await wait(5) { !st.hcConnected && !st.connected }, "both gone: disconnected")
    // hc comes back alone, with a change.
    hc.terminals = [FakeHC.terminal("t9", argv: ["codex"], status: "idle")]
    try hc.start()
    check(await wait(5) { st.hcConnected && st.connected && st.workspaces.last?.id == "hc:other" }, "hc alone reconnects")
    st.stop(); hc.kill()

    // No hc configured (the default for HerdrStore(session:)): TUI only, nothing else touched.
    let st2 = HerdrStore(session: .named("hk-none-\(getpid())"))
    check(st2.hcClient == nil, "no hc session: no hc client")
}

// The TUI client refuses hc ids before anything leaves (routing bug guard).
@MainActor func checksHCGuard() async throws {
    let c = HerdrClient(session: .named("hk-none-\(getpid())"))
    for call in ["rename", "close", "split", "keys"] {
        var refused = false
        do {
            switch call {
            case "rename": try await c.renameTab("hc:tab:s1/a", to: "x")
            case "close": try await c.closePane(paneId: "hc:t1")
            case "split": try await c.split(paneId: "hc:t1", direction: .right)
            default: try await c.sendNamedKeys(["Enter"], to: "hc:t1")
            }
        } catch HerdrError.protocolError(let m) { refused = m.contains("herdr-core") } catch {}
        check(refused, "TUI client refuses an hc id (\(call))")
    }
}

// Step 3: session logs for hc agents. hc has no agent_session: use the terminal's pids.
@MainActor func checksHCSessions() async throws {
    typealias R = SessionResolver
    let ps = """
      500     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t1 TERM=xterm
      501   500 Sat Oct 31 10:01:00 2026     claude HC_TERMINAL_ID=t1
      502   501 Sat Oct 31 10:02:00 2026     /bin/zsh -c ls
      600     1 Sat Oct 31 10:00:00 2026     node /opt/bin/codex HC_TERMINAL_ID=t2
      601   600 Sat Oct 31 10:05:00 2026     rg foo
      700     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t3
      701   700 Sat Oct 31 10:09:00 2026     claude HC_TERMINAL_ID=t3
    """
    let procs = R.parsePS(ps)
    let dir = NSTemporaryDirectory() + "hkcs-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let utc = DateFormatter(); utc.locale = Locale(identifier: "en_US_POSIX"); utc.timeZone = TimeZone(identifier: "UTC"); utc.dateFormat = "EEE MMM d HH:mm:ss yyyy"
    func write(_ pid: Int32, _ sid: String, start: Date) {
        let o: [String: Any] = ["pid": Int(pid), "sessionId": sid, "procStart": utc.string(from: start), "cwd": "/tmp"]
        FileManager.default.createFile(atPath: "\(dir)/\(pid).json", contents: try! JSONSerialization.data(withJSONObject: o))
    }
    let start501 = procs.first { $0.pid == 501 }!.start
    write(501, "sid-501", start: start501)
    write(701, "sid-stale", start: start501)   // pid reused: procStart does not match 701's start
    check(R.claudeSession(pids: [501, 500], procs: procs, dir: dir) == "sid-501", "claude: <pid>.json of the foreground process")
    check(R.claudeSession(pids: [500], procs: procs, dir: dir) == "sid-501", "claude: found in the terminal's process tree")
    check(R.claudeSession(pids: [701, 700], procs: procs, dir: dir) == nil, "claude: stale <pid>.json (other process start) is ignored")
    check(R.claudeSession(pids: [], procs: procs, dir: dir) == nil && R.claudeSession(pids: [999], procs: procs, dir: dir) == nil, "claude: no pid, no guess")
    check(R.hcAgentProcess(pids: [600], kind: "codex", procs: procs)?.pid == 600, "codex: the terminal process runs codex itself")
    check(R.hcAgentProcess(pids: [500], kind: "claude", procs: procs)?.pid == 501, "agent process: newest descendant running the agent")
    check(R.hcAgentProcess(pids: [601, 600], kind: "codex", procs: procs)?.pid == 600, "codex: a tool child in the foreground still maps to codex")

    // Mapper hands the pids over (foreground first), and the store resolves through them.
    let t = FakeHC.terminal("t1", argv: ["claude"], pid: 500, status: "idle", fg: ["pid": 501, "name": "claude", "argv": ["claude"], "cwd": "/tmp"])
    let pane = HCMapper.workspaces(terminals: [t], appDoc: nil).first?.tabs.first?.panes.first
    check(pane?.processIds == [501, 500], "pane carries foreground pid then terminal pid")
    let res = SessionResolver(socketPath: "/nonexistent", claudeDir: dir)
    let need = SessionResolver.Need(paneId: "hc:t1", kind: "claude", cwd: "/tmp", pids: [501, 500])
    let got = await res.resolve([need], claimed: [], procs: procs)
    check(got["hc:t1"] == "sid-501", "resolver: hc need resolved by pid")
    let taken = await SessionResolver(socketPath: "/nonexistent", claudeDir: dir).resolve([need], claimed: ["sid-501"], procs: procs)
    check(taken["hc:t1"] == nil, "resolver: a claimed session is not handed out twice")
}

// Step 4: reply (send/keys) and the terminal sheet over attach, against FakeHC.
@MainActor func checksHCInput() async throws {
    check(HCKeys.translate("Enter") == "enter" && HCKeys.translate("Escape") == "esc" && HCKeys.translate("C-c") == "ctrl+c"
          && HCKeys.translate("Down") == "down" && HCKeys.translate("M-x") == "alt+x" && HCKeys.translate("a") == "a"
          && HCKeys.translate("BTab") == "shift+tab", "herdr key names -> hc key names")
    check(HCKeys.names(for: Array("hi\r".utf8)) == ["h", "i", "enter"], "bytes -> keys: text + CR")
    check(HCKeys.names(for: [0x1b, 0x5b, 0x42, 0x03, 0x1b, 0x7f, 0x09]) == ["down", "ctrl+c", "esc", "backspace", "tab"], "bytes -> keys: arrows, ctrl, esc, del, tab")

    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1", status: "idle")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    try await hc.send(text: "你好 world", to: "hc:t1")
    let send = fake.calls.last { $0.method == "terminal.send" }
    check(send?.params["terminal"] as? String == "t1" && send?.params["text"] as? String == "你好 world", "send: terminal.send with the text (Enter is hc's)")
    check(send?.params["force"] == nil, "send never forces into a blocked dialog")
    try await hc.sendNamedKeys(["Down", "Enter"], to: "hc:t1")
    check(fake.calls.last?.method == "terminal.keys" && fake.calls.last?.params["keys"] as? [String] == ["down", "enter"], "named keys translated")
    try await hc.sendKeys([0x1b], to: "hc:t1")
    check(fake.calls.last?.params["keys"] as? [String] == ["esc"], "raw bytes as keys")
    var refused = false
    do { try await hc.send(text: "x", to: "w1:p1") } catch { refused = true }
    check(refused, "hc client refuses a TUI pane id")

    // Store routes by pane id.
    let st = HerdrStore(session: .named("hk-none-\(getpid())"), hc: HCSession(socketPath: fake.path))
    check(st.backend(forPane: "hc:t1")?.backend == .hc && st.backend(forPane: "w1:p1")?.backend == .tui, "store: panes route by id")

    // TerminalSession over attach: same events as the TUI channel.
    let ts = TerminalSession(hcTerminal: "t1", client: hc, mode: .control(takeover: false), cols: 100, rows: 30)
    ts.start()
    var it = ts.events.makeAsyncIterator()
    guard case .frame(let first)? = await it.next() else { check(false, "first event is a frame"); return }
    check(first.full && first.bytes == fake.firstPaint && first.width == 80 && first.height == 24, "first paint is a full frame with the pty size")
    let ch = UInt32(1)
    check(await eventually { fake.calls.contains { $0.method == "terminal.attach" && $0.params["observe"] as? Bool == true } }, "attach watching first (control comes with the first keys)")
    first.consumed()
    check(await eventually { fake.credits(ch) >= fake.firstPaint.count }, "credit after the view consumed the frame")
    fake.output(ch, Data("\u{1b}[?2004lx".utf8))
    guard case .frame(let next)? = await it.next() else { check(false, "second frame"); return }
    check(!next.full && next.bytes == Data("\u{1b}[?2004lx".utf8), "live output is an increment")
    check(!ts.bracketedPaste, "bracketed paste mode follows the program")
    check(ts.send(bytes: Array("ls\r".utf8)), "send returns true when attached")
    let cch = UInt32(2)   // the controller channel opened by the first keys
    guard case .frame(let paint2)? = await it.next() else { check(false, "controller first paint"); return }
    check(paint2.full, "controller attach starts with a full frame")
    ts.resize(cols: 90, rows: 20)
    check(await eventually { fake.viewerBytes(cch) == Data("ls\r".utf8) && (fake.resizes(cch).last.map { $0 == (90, 20) } ?? false) }, "keys and resize reach the channel")
    check(!ts.wheel(lines: 1, up: true), "wheel is not encoded for hc (no mode info): no bytes sent")
    fake.exitProgram(cch)
    var closed: String?
    while let ev = await it.next() { if case .closed(let r) = ev { closed = r } }
    check(closed == "exited", "program exit ends the session with its reason (\(closed ?? "nil"))")

    // Observe mode never types.
    let ob = TerminalSession(hcTerminal: "t1", client: hc, mode: .observe)
    ob.start()
    var it2 = ob.events.makeAsyncIterator()
    _ = await it2.next()
    check(!ob.send(bytes: [0x61]) && fake.calls.last { $0.method == "terminal.attach" }?.params["observe"] as? Bool == true, "observe: attach observe, no input")
    ob.release()
    check(await eventually { fake.closedByViewer(3) }, "release sends CLOSE")

    // Missing terminal: closed, not a reconnect loop.
    let gone = TerminalSession(hcTerminal: "t9", client: hc, mode: .observe)
    gone.start()
    var it3 = gone.events.makeAsyncIterator()
    if case .closed(let r)? = await it3.next() { check(r.contains("not found") || r.contains("no terminal"), "missing terminal closes: \(r)") } else { check(false, "missing terminal closes") }
}

// Step 5: FakeHerdr and FakeHC at once: events from both, interleaved; the store only reads hc.
@MainActor func checksHCBoth() async throws {
    func wait(_ sec: Double, _ ok: () -> Bool) async -> Bool {
        let t = Date(); while Date().timeIntervalSince(t) < sec { if ok() { return true }; try? await Task.sleep(for: .milliseconds(5)) }; return ok()
    }
    let tui = FakeHerdr(path: NSTemporaryDirectory() + "hkfk3-\(getpid()).sock")
    try tui.start()
    let hc = FakeHC(path: fakeHCPath())
    try hc.start()
    hc.terminals = [FakeHC.terminal("t1", labels: ["herdr.space": "s1", "herdr.tab": "a"], argv: ["claude"], status: "idle")]
    let st = HerdrStore(session: HerdrSession(name: "fake", socketPath: tui.path), hc: HCSession(socketPath: hc.path))
    st.start()
    check(await wait(3) { st.workspaces.count == 2 }, "both: first snapshots")
    for n in 1...5 {
        tui.label = "v\(n)"; tui.pushEvent()
        hc.terminals = [FakeHC.terminal("t1", labels: ["herdr.space": "s1", "herdr.tab": "a"], argv: ["claude"], status: n % 2 == 0 ? "idle" : "working", title: "turn \(n)")]
        hc.pushEvent(type: "title", terminal: "t1", data: ["title": "turn \(n)"])
    }
    let both = await wait(3) {
        st.workspaces.first?.label == "v5" && st.workspaces.last?.tabs.first?.panes.first?.agent?.title == "turn 5"
    }
    check(both, "both: interleaved events from TUI and hc land in one list (\(st.workspaces.first?.label ?? "-"), \(st.workspaces.last?.tabs.first?.panes.first?.agent?.title ?? "-"))")
    check(st.workspaces.last?.tabs.first?.panes.first?.agent?.status == .working, "both: hc status follows")
    let mutating: Set<String> = ["doc.set", "terminal.label", "terminal.kill", "terminal.rm", "terminal.run", "terminal.keep",
                                 "terminal.send", "terminal.keys", "terminal.interrupt", "terminal.attach", "server.stop", "server.restart"]
    check(!hc.calls.contains { mutating.contains($0.method) }, "both: the store never writes to hc (\(Set(hc.calls.map(\.method)).sorted()))")
    st.stop(); tui.kill(); hc.kill()
}
