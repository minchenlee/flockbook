import Foundation
@testable import HerdrKit

// Edge cases of the hc client, terminal and store: one block per case. FakeHC only.

/// #1 late frames after close, #2 backlog order.
@MainActor func checksHCEdgeChannels() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let conn = try await HCFramedConnection.open(socketPath: fake.path)
    defer { conn.shutdown() }

    // #1: frames for a closed channel are dropped, never parked.
    fake.reuseChannels = true
    let a = try await conn.attach(terminal: "t1", cols: 80, rows: 24)
    var ia = a.bytes.makeAsyncIterator()
    _ = try await ia.next()
    a.close()
    check(await eventually { fake.closedByViewer(a.channel) }, "#1: viewer CLOSE reaches the core")
    fake.rawFrame(a.channel, .data, Data("late".utf8))
    try await Task.sleep(for: .milliseconds(100))
    check(conn.earlyCount == 0, "#1: late DATA and the core's CLOSE answer for a closed channel are dropped (early=\(conn.earlyCount))")
    // The core reuses the id: the new attach must not see the old channel's CLOSE.
    let b = try await conn.attach(terminal: "t1", cols: 80, rows: 24)
    check(b.channel == a.channel, "#1: fake reused channel \(a.channel)")
    var ib = b.bytes.makeAsyncIterator()
    let first = try await ib.next()
    check(first == fake.firstPaint, "#1: reused channel starts with its own first paint, not a stale CLOSE")
    // A stale CLOSE answer arriving after the new attach registered would have to be a new close: the core sends it only
    // after a viewer CLOSE, so nothing else to check here.
    b.close()
    fake.reuseChannels = false

    // #2: the backlog (first paint, frames before registration) comes before live frames, every time.
    fake.paintBurst = 300
    let want = String(decoding: fake.firstPaint, as: UTF8.self) + (1...300).map { "#\($0);" }.joined()
    var bad = 0
    for _ in 0..<15 {
        let s = try await conn.attach(terminal: "t1", cols: 80, rows: 24)
        var got = ""
        for try await c in s.bytes { got += String(decoding: c, as: UTF8.self); if got.count >= want.count { break } }
        if got != want { bad += 1 }
        s.close()
    }
    check(bad == 0, "#2: first paint and early frames delivered in order before live ones (\(bad)/15 out of order)")
    fake.paintBurst = 0
    try await Task.sleep(for: .milliseconds(100))
    check(conn.earlyCount == 0, "#1: nothing parked after many attach/close cycles (early=\(conn.earlyCount))")
}

/// #3: `terminal.send` gets its own, longer timeout; a timeout means "sent, not confirmed", never "not sent".
@MainActor func checksHCEdgeSend() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    // Scaled down: calls time out after 0.3 s, sends after 1 s.
    let hc = HCClient(session: HCSession(socketPath: fake.path), callTimeout: 0.3, sendTimeout: 1)
    fake.sendDelay = 0.6
    check(await hc.deliver(text: "hi", to: "hc:t1") == .sent, "#3: a send slower than a plain call's timeout still confirms")
    fake.sendDelay = 1.6
    check(await hc.deliver(text: "hi", to: "hc:t1") == .unconfirmed, "#3: a send past its timeout is 'sent, not confirmed' (the core may still deliver it)")
    fake.sendDelay = 0
    var timedOut = false
    fake.sendDelay = 0   // plain calls keep the short timeout
    do { _ = try await hc.call("terminal.list") } catch { timedOut = true }
    check(!timedOut, "#3: plain calls still answer")
    check(await hc.deliver(text: "hi", to: "w1:p1") == .failed, "#3: a send that never reached the core is 'not sent'")
    let down = HCClient(session: HCSession(socketPath: fakeHCPath()))
    check(await down.deliver(text: "hi", to: "hc:t1") == .failed, "#3: no core: 'not sent'")
}

/// #4: viewing never resizes the user's terminals. A control session watches (observe) until the user types; then it
/// reattaches as a controller with the view's real size, sends the keys, and comes back `resumed` after a drop.
@MainActor func checksHCEdgeObserveFirst() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func attaches() -> [[String: Any]] { fake.calls.filter { $0.method == "terminal.attach" }.map(\.params) }
    let ts = TerminalSession(hcTerminal: "t1", client: hc, mode: .control(takeover: false), cols: 100, rows: 30)
    ts.start()
    let frames = Locked<[TerminalFrame]>([])
    let reader = Task { for await e in ts.events { if case .frame(let f) = e { frames.update { $0.append(f) } } } }
    defer { reader.cancel() }
    check(await eventually { frames.get().first?.full == true }, "#4: first paint")
    check(attaches().count == 1 && attaches()[0]["observe"] as? Bool == true, "#4: a control session starts as a watcher (observe)")
    ts.resize(cols: 90, rows: 20)
    try await Task.sleep(for: .milliseconds(150))
    check(fake.resizes(1).isEmpty, "#4: resizing the view before typing sends no RESIZE")
    check(ts.send(bytes: Array("ls\r".utf8)), "#4: typing is accepted while watching")
    check(await eventually { attaches().count == 2 }, "#4: typing takes control (a second attach)")
    let a2 = attaches().last ?? [:]
    check(a2["observe"] as? Bool == false && a2["cols"] as? Int == 90 && a2["rows"] as? Int == 20 && a2["resumed"] as? Bool != true,
          "#4: control attach with the view's real size (\(a2["cols"] ?? "-")x\(a2["rows"] ?? "-"), observe \(a2["observe"] ?? "-"))")
    check(await eventually { fake.viewerBytes(2) == Data("ls\r".utf8) }, "#4: the keys that took control reach the new channel")
    check(await eventually { fake.closedByViewer(1) }, "#4: the watching channel is closed")
    check(await eventually { frames.get().dropFirst().contains { $0.full } }, "#4: the control attach starts with a full frame")
    ts.resize(cols: 85, rows: 22)
    check(await eventually { fake.resizes(2).last.map { $0 == (85, 22) } ?? false }, "#4: once in control, resizes reach the core")
    // Dropped line (not a final reason): back in control, `resumed`.
    fake.rawFrame(2, .close, Data(#"{"reason":"lost"}"#.utf8))
    check(await eventually(3) { attaches().count == 3 }, "#4: reattach after a drop")
    let a3 = attaches().last ?? [:]
    check(a3["observe"] as? Bool == false && a3["resumed"] as? Bool == true, "#4: a controller comes back resumed (\(a3))")
    ts.stop()

    // Observe sessions never take control.
    let ob = TerminalSession(hcTerminal: "t1", client: hc, mode: .observe)
    ob.start()
    check(await eventually { attaches().count == 4 }, "#4: observe session attaches")
    check(!ob.send(bytes: [0x61]), "#4: an observe session refuses keys")
    try await Task.sleep(for: .milliseconds(100))
    check(attaches().count == 4, "#4: and does not reattach")
    ob.stop()
}

/// #5: a remote child's `parent` is local to its machine: `host2/t9` with parent `t3` belongs to `host2/t3`, even when
/// this machine also has a `t3`.
@MainActor func checksHCEdgeRemoteParent() async throws {
    let ws = HCMapper.workspaces(terminals: [
        FakeHC.terminal("t3", labels: ["herdr.space": "local", "herdr.tab": "a"], argv: ["zsh"]),
        FakeHC.terminal("host2/t3", labels: ["herdr.space": "remote", "herdr.tab": "b"], argv: ["claude"]),
        FakeHC.terminal("host2/t9", argv: ["claude"], parent: "t3"),
    ], appDoc: nil)
    let home = ws.first { $0.tabs.flatMap(\.panes).contains { $0.terminalId == "host2/t9" } }?.id
    check(home == "hc:space:host2/remote", "#5: remote child joins its own machine's parent (got \(home ?? "nil"))")
}

/// #6: Claude rewrites `sessionId` in `<pid>.json` on /clear or /resume, in the same process: the resolver follows.
@MainActor func checksHCEdgeClaudeClear() async throws {
    let ps = """
      500     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t1
      501   500 Sat Oct 31 10:01:00 2026     claude HC_TERMINAL_ID=t1
    """
    let procs = SessionResolver.parsePS(ps)
    let dir = NSTemporaryDirectory() + "hkcc-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let utc = DateFormatter(); utc.locale = Locale(identifier: "en_US_POSIX"); utc.timeZone = TimeZone(identifier: "UTC"); utc.dateFormat = "EEE MMM d HH:mm:ss yyyy"
    let start = procs.first { $0.pid == 501 }!.start
    func write(_ sid: String, mtime: Date) {
        let o: [String: Any] = ["pid": 501, "sessionId": sid, "procStart": utc.string(from: start)]
        let p = "\(dir)/501.json"
        FileManager.default.createFile(atPath: p, contents: try! JSONSerialization.data(withJSONObject: o))
        try? FileManager.default.setAttributes([.modificationDate: mtime], ofItemAtPath: p)
    }
    write("sid-a", mtime: Date().addingTimeInterval(-60))
    let res = SessionResolver(socketPath: "/nonexistent", claudeDir: dir)
    let need = SessionResolver.Need(paneId: "hc:t1", kind: "claude", cwd: "/tmp", pids: [501, 500])
    check(await res.resolve([need], claimed: [], procs: procs)["hc:t1"] == "sid-a", "#6: first session")
    write("sid-b", mtime: Date())
    check(await res.resolve([need], claimed: [], procs: procs)["hc:t1"] == "sid-b", "#6: /clear in the same process: the new sessionId is picked up at once")
    let again = await res.resolve([need], claimed: [], procs: procs)["hc:t1"]
    check(again == "sid-b", "#6: unchanged pid file keeps the cached session")
}

/// #7: attach errors that can never succeed end the session instead of a reconnect loop.
@MainActor func checksHCEdgeFinalErrors() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    for kind in ["finished", "machine_unknown", "needs_framed", "not_found"] {
        fake.attachError = kind
        let ts = TerminalSession(hcTerminal: "t1", client: hc, mode: .observe)
        ts.start()
        let evs = Locked<[TerminalEvent]>([])
        let reader = Task { for await e in ts.events { evs.update { $0.append(e) } } }
        let closed = await eventually(1.5) { evs.get().contains { if case .closed = $0 { true } else { false } } }
        let retried = evs.get().contains { if case .reconnecting = $0 { true } else { false } }
        check(closed && !retried, "#7: attach error \(kind) closes the session, no retry")
        reader.cancel(); ts.stop()
    }
    fake.attachError = "unavailable"
    let ts = TerminalSession(hcTerminal: "t1", client: hc, mode: .observe)
    ts.start()
    let evs = Locked<[TerminalEvent]>([])
    let reader = Task { for await e in ts.events { evs.update { $0.append(e) } } }
    check(await eventually(1.5) { evs.get().contains { if case .reconnecting = $0 { true } else { false } } }, "#7: unavailable is retried")
    reader.cancel(); ts.stop()
    fake.attachError = nil
}

/// #8: CREDIT bounds memory: it goes out when the view has processed a frame (`frame.consumed()`), not when the frame
/// enters the session's event stream.
@MainActor func checksHCEdgeCredit() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    let ts = TerminalSession(hcTerminal: "t1", client: hc, mode: .observe)
    ts.start()
    var it = ts.events.makeAsyncIterator()
    guard case .frame(let f)? = await it.next() else { check(false, "#8: first frame"); return }
    fake.output(1, Data("more".utf8))
    try await Task.sleep(for: .milliseconds(200))
    check(fake.credits(1) == 0, "#8: no CREDIT while frames wait for the view (\(fake.credits(1)))")
    f.consumed()
    check(await eventually { fake.credits(1) == fake.firstPaint.count }, "#8: CREDIT for exactly what the view consumed (\(fake.credits(1)))")
    guard case .frame(let g)? = await it.next() else { check(false, "#8: second frame"); return }
    g.consumed(); g.consumed()
    check(await eventually { fake.credits(1) == fake.firstPaint.count + 4 }, "#8: each frame credits once (\(fake.credits(1)))")
    ts.stop()
}

/// #9: concurrent first attaches share one framed connection; LineSocket close from another thread ends a blocked read.
@MainActor func checksHCEdgeSingleFlight() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    async let a = hc.attach(terminal: "t1", cols: 80, rows: 24, observe: true)
    async let b = hc.attach(terminal: "t1", cols: 80, rows: 24, observe: true)
    async let c = hc.attach(terminal: "t1", cols: 80, rows: 24, observe: true)
    let atts = try await [a, b, c]
    let hellos = fake.calls.filter { $0.method == "hello" && $0.params["framed"] as? Bool == true }.count
    check(hellos == 1, "#9: three concurrent first attaches open one framed connection (\(hellos))")
    atts.forEach { $0.close() }

    let sock = try LineSocket(path: fake.path)
    let ended = Locked(false)
    Thread.detachNewThread { _ = try? sock.readChunk(); ended.set(true) }
    try await Task.sleep(for: .milliseconds(100))
    sock.close()
    check(await eventually { ended.get() }, "#9: close from another thread ends a blocked read")
    check(sock.fileDescriptor == -1, "#9: fd is cleared under the lock")
}

/// looking at an hc terminal never takes it over. Focus reports (`CSI I`/`CSI O`, sent by the view on
/// focus) and mouse reports (X10 `CSI M`, SGR `CSI <`) are dropped while watching; only real input upgrades.
@MainActor func checksHCEdgeReportsWatch() async throws {
    check(TerminalSession.isReportOnly(Array("\u{1b}[I".utf8)) && TerminalSession.isReportOnly(Array("\u{1b}[O\u{1b}[I".utf8))
          && TerminalSession.isReportOnly(Array("\u{1b}[<0;12;5M\u{1b}[<0;12;5m".utf8)) && TerminalSession.isReportOnly([0x1b, 0x5b, 0x4d, 0x20, 0x21, 0x21]),
          "reports: focus and mouse reports are told apart")
    check(!TerminalSession.isReportOnly(Array("\u{1b}[A".utf8)) && !TerminalSession.isReportOnly(Array("a".utf8))
          && !TerminalSession.isReportOnly(Array("\u{1b}[Ia".utf8)) && !TerminalSession.isReportOnly(Array("\u{1b}".utf8)),
          "reports: keys (arrows, text, Esc) are input")
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func attaches() -> [[String: Any]] { fake.calls.filter { $0.method == "terminal.attach" }.map(\.params) }
    let ts = TerminalSession(hcTerminal: "t1", client: hc, mode: .control(takeover: false), cols: 100, rows: 30)
    ts.start()
    let reader = Task { for await _ in ts.events {} }
    defer { reader.cancel(); ts.stop() }
    check(await eventually { attaches().count == 1 }, "reports: watching")
    _ = ts.send(bytes: Array("\u{1b}[I".utf8))
    _ = ts.send(bytes: Array("\u{1b}[<0;3;4M".utf8))
    _ = ts.send(bytes: Array("\u{1b}[O".utf8))
    try await Task.sleep(for: .milliseconds(200))
    check(attaches().count == 1, "reports: focus and mouse reports do not take control (\(attaches().count) attaches)")
    _ = ts.send(bytes: Array("x".utf8))
    check(await eventually { attaches().count == 2 }, "reports: a real key still takes control")
    check(await eventually { fake.viewerBytes(2) == Data("x".utf8) }, "reports: only the key went out, no report before it")
}
