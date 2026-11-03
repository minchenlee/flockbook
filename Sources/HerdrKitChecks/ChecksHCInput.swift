import Foundation
@testable import HerdrKit

// The hc terminal's input: queued keys, descriptors, stamps, credit and failed connects. FakeHC only.

/// A: keys queued during the upgrade go out before any key typed after it. B: queued keys are capped and expire.
@MainActor func checksHCInputKeys() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    func attaches() -> [[String: Any]] { fake.calls.filter { $0.method == "terminal.attach" }.map(\.params) }

    // A: 2000 keys queue while the controller attach is in flight; a second writer keeps typing through the flush.
    let ts = TerminalSession(hcTerminal: "t1", client: hc, mode: .control(takeover: false))
    ts.pendingCap = 10_000   // order only here; B checks the cap
    ts.start()
    check(await eventually { attaches().count == 1 }, "A: watcher attached")
    try await Task.sleep(for: .milliseconds(100))
    fake.attachDelay = 0.3
    for i in 0..<2000 { ts.send(text: "q\(i);") }
    let typer = Task.detached { for j in 0..<1500 { ts.send(text: "d\(j);"); if j % 10 == 0 { try? await Task.sleep(for: .milliseconds(1)) } } }
    await typer.value
    fake.attachDelay = 0
    let ch = fake.lastChannel
    check(await eventually(5) { String(decoding: fake.viewerBytes(ch), as: UTF8.self).hasSuffix("d1499;") }, "A: all keys arrive")
    let toks = String(decoding: fake.viewerBytes(ch), as: UTF8.self).split(separator: ";").map(String.init)
    let want = (0..<2000).map { "q\($0)" } + (0..<1500).map { "d\($0)" }
    let firstBad = zip(toks, want).enumerated().first { $0.element.0 != $0.element.1 }
    check(toks == want, "A: keys arrive in the order typed (\(toks.count) tokens, first mismatch at \(firstBad.map { "\($0.offset): \($0.element.0) vs \($0.element.1)" } ?? "-"))")
    ts.stop()

    // B1: keys that wait longer than the window are dropped, and the user is told.
    let t2 = TerminalSession(hcTerminal: "t1", client: hc, mode: .control(takeover: false))
    t2.pendingWindow = 0.4
    let notes = Locked<[String]>([])
    let r2 = Task { for await e in t2.events { if case .notice(let n) = e { notes.update { $0.append(n) } } } }
    t2.start()
    check(await eventually { attaches().count >= 3 }, "B: watcher attached")
    try await Task.sleep(for: .milliseconds(100))
    fake.attachError = "unavailable"
    let n0 = attaches().count
    t2.send(text: "old\r")
    check(await eventually(2) { attaches().count >= n0 + 2 }, "B: control attach keeps failing (unavailable)")
    try await Task.sleep(for: .milliseconds(500))
    fake.attachError = nil
    let before = fake.lastChannel
    check(await eventually(4) { fake.lastChannel > before }, "B: control attach succeeds later")
    try await Task.sleep(for: .milliseconds(200))
    check(fake.viewerBytes(fake.lastChannel).isEmpty, "B: stale keys are not sent (\(String(decoding: fake.viewerBytes(fake.lastChannel), as: UTF8.self).debugDescription))")
    check(notes.get().contains { $0.contains("not sent") }, "B: the user is told (\(notes.get()))")
    r2.cancel(); t2.stop()

    // B2: the queue is capped; the overflow is dropped and reported.
    let t3 = TerminalSession(hcTerminal: "t1", client: hc, mode: .control(takeover: false))
    t3.pendingCap = 5
    let notes3 = Locked<[String]>([])
    let r3 = Task { for await e in t3.events { if case .notice(let n) = e { notes3.update { $0.append(n) } } } }
    t3.start()
    let n3 = attaches().count
    check(await eventually { attaches().count == n3 + 1 }, "B: watcher attached")
    try await Task.sleep(for: .milliseconds(100))
    fake.attachDelay = 0.3
    for i in 0..<20 { t3.send(text: "k\(i);") }
    fake.attachDelay = 0
    check(await eventually(3) { attaches().count == n3 + 2 }, "B: controller attached")
    try await Task.sleep(for: .milliseconds(300))
    let got = String(decoding: fake.viewerBytes(fake.lastChannel), as: UTF8.self)
    check(got.isEmpty, "B: an overflow drops the whole queue, no partial line (\(got))")
    check(notes3.get().contains { $0.contains("not sent") }, "B: overflow reported (\(notes3.get()))")
    r3.cancel(); t3.stop()
}

/// C: LineSocket never closes a descriptor another thread is still using (its number could go to a new socket).
@MainActor func checksHCInputFD() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    let sock = try LineSocket(path: fake.path)
    let fd = try sock.acquireFD()          // a reader or writer is inside a syscall
    sock.close()
    check(fcntl(fd, F_GETFD) != -1, "C: close() while in use only shuts the socket down; the descriptor stays ours")
    var threw = false
    do { _ = try sock.acquireFD() } catch { threw = true }
    check(threw, "C: no new use after close()")
    sock.releaseFD()
    // Only our own view: another thread may already have the freed number again, so fcntl on it proves nothing.
    check(sock.fileDescriptor == -1, "C: the last user closes it")

    // A blocked reader is woken by close() and the descriptor is closed after it left.
    let s2 = try LineSocket(path: fake.path)
    let ended = Locked(false)
    Thread.detachNewThread { _ = try? s2.readChunk(); ended.set(true) }
    try await Task.sleep(for: .milliseconds(100))
    s2.close()
    check(await eventually { ended.get() } && s2.fileDescriptor == -1, "C: close() wakes a blocked reader, then the fd is closed")
}

/// D: the resolver watches the pid file that actually named the session, also when it belongs to a child of the agent.
@MainActor func checksHCInputStamp() async throws {
    typealias R = SessionResolver
    let procs = R.parsePS("""
      800     1 Sat Oct 31 10:00:00 2026     -zsh HC_TERMINAL_ID=t1
      801   800 Sat Oct 31 10:01:00 2026     claude HC_TERMINAL_ID=t1
      802   801 Sat Oct 31 10:01:30 2026     node /opt/claude/cli.js HC_TERMINAL_ID=t1
    """)
    let dir = NSTemporaryDirectory() + "hkst-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let utc = DateFormatter(); utc.locale = Locale(identifier: "en_US_POSIX"); utc.timeZone = TimeZone(identifier: "UTC"); utc.dateFormat = "EEE MMM d HH:mm:ss yyyy"
    let start = procs.first { $0.pid == 802 }!.start
    func write(_ sid: String, mtime: Date) throws {
        let p = "\(dir)/802.json"
        let o: [String: Any] = ["pid": 802, "sessionId": sid, "procStart": utc.string(from: start)]
        FileManager.default.createFile(atPath: p, contents: try JSONSerialization.data(withJSONObject: o))
        try FileManager.default.setAttributes([.modificationDate: mtime], ofItemAtPath: p)
    }
    try write("sid-a", mtime: Date(timeIntervalSinceNow: -100))
    let res = R(socketPath: "/nonexistent", claudeDir: dir, retry: 3600, recheck: 3600)
    let need = R.Need(paneId: "hc:t1", kind: "claude", cwd: "/tmp", pids: [800])
    let first = await res.resolve([need], claimed: [], procs: procs)
    check(first["hc:t1"] == "sid-a", "stamp: session found in a child pid's file")
    try write("sid-b", mtime: Date())   // /clear: the child rewrites its file, same process
    let second = await res.resolve([need], claimed: [], procs: procs)
    check(second["hc:t1"] == "sid-b", "stamp: a rewrite of the child pid's file is picked up before the recheck")
}

/// E: a frame the view finishes after its stream closed sends no CREDIT (the channel is gone or reused).
@MainActor func checksHCInputCredit() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.terminals = [FakeHC.terminal("t1")]
    let conn = try await HCFramedConnection.open(socketPath: fake.path)
    defer { conn.shutdown() }
    let a = try await conn.attach(terminal: "t1", cols: 80, rows: 24, observe: true)
    let ch = a.channel
    a.close()
    a.consumed(100)
    // Same socket, written in order: once b's CREDIT is in, a late CREDIT for `ch` would have arrived too.
    let b = try await conn.attach(terminal: "t1", cols: 80, rows: 24, observe: true)
    b.consumed(7)
    check(await eventually { fake.credits(b.channel) >= 7 }, "E: the open stream's CREDIT arrives")
    check(b.channel == ch || fake.credits(ch) == 0, "E: no CREDIT for a closed stream")
    b.close()
}

/// A failed connect closes its descriptor once. (A thrown init still runs deinit, which closed the same number again:
/// a reconnect loop against a dead path then closed whatever socket another thread had just opened.)
@MainActor func checksHCInputFailedConnect() async throws {
    let stop = Locked(false)
    Thread.detachNewThread {
        while !stop.get() { _ = try? LineSocket(path: NSTemporaryDirectory() + "hk-none-\(getpid()).sock") }
    }
    var lost = 0
    for _ in 0..<20_000 {
        var p: [Int32] = [0, 0]
        guard pipe(&p) == 0 else { continue }
        for _ in 0..<20 { if fcntl(p[0], F_GETFD) == -1 || fcntl(p[1], F_GETFD) == -1 { lost += 1; break } }
        close(p[0]); close(p[1])
        if lost > 0 { break }
    }
    stop.set(true)
    check(lost == 0, "failed connect: no other thread's descriptor is closed by it")
}
