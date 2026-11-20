import Foundation
@testable import HerdrKit

// The third review of the Since you left card (each failed before its fix).

/// The app's copy shows no turn yet when the read decides (a mirror still empty, a log not loaded): nothing is marked
/// until the copy shows a turn, so the newer turn is not marked seen before it is on screen.
func checksSinceArrivalNoCopy() {
    let now: Int64 = 100 * 60_000, seen = SinceLeft.Seen(id: "a1", at: now - 47 * 60_000)
    var a = SinceLeft.Arrival()
    let d = a.first("a2", shown: nil, seen: seen, now: now)
    check(d.decided && d.away != nil, "since arrival no copy: the read decides: a card")
    check(a.mark(nil) == nil, "since arrival no copy: the app's copy shows nothing yet: nothing is marked (\(a.mark(nil) ?? "nil"))")
    check(a.mark("a2") == "a2", "since arrival no copy: once the read's turn shows, it is marked")
    check(a.mark(nil) == "a2", "since arrival no copy: after that, a copy that shows nothing keeps the decided turn")
    var older = SinceLeft.Arrival()
    _ = older.first("a2", shown: nil, seen: seen, now: now)
    check(older.mark("a1") == "a1", "since arrival no copy: the copy shows an older turn: that turn is marked (the card comes again)")
    var same = SinceLeft.Arrival()
    _ = same.first("a2", shown: "a2", seen: seen, now: now)
    check(same.mark(nil) == "a2", "since arrival no copy: decided on the turn shown: a copy that shows nothing keeps it")
}

/// Outcomes of the next host reads, in order (true: read; false: fail); reads after them all succeed.
private final class ReadPlan: @unchecked Sendable {
    private let lock = NSLock()
    private var plan: [Bool] = []
    func set(_ p: [Bool]) { lock.withLock { plan = p } }
    func next() -> Bool { lock.withLock { plan.isEmpty ? true : plan.removeFirst() } }
}

/// A copy over ssh that stops part way (a read fails after the first chunk) has not caught up: it does not count as a
/// finished copy, on the first fill, after a reset, or when the log grew. The next copy that reads to the end counts.
@MainActor func checksMirrorPartialSync() async throws {
    let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hkmpart-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let hostFile = dir.appendingPathComponent("host.jsonl").path
    let line = { (n: Int) in "{\"n\":\(n),\"pad\":\"\(String(repeating: "x", count: 20))\"}\n" }
    try (0..<4).map(line).joined().write(toFile: hostFile, atomically: false, encoding: .utf8)
    let plan = ReadPlan()
    let m = try RemoteLogMirror(host: RemoteHost(via: .local), path: hostFile, dir: dir.appendingPathComponent("cache"), chunk: 16,
                                read: { p, at, n in
        guard plan.next(), let fh = FileHandle(forReadingAtPath: p) else { return nil }
        defer { try? fh.close() }
        try? fh.seek(toOffset: at)
        return try? fh.read(upToCount: n) ?? Data()
    })
    let key = m.url.path
    let bytes = { (p: String) in (try? Data(contentsOf: URL(fileURLWithPath: p))) ?? Data() }
    func step(_ name: String, _ p: [Bool], want: RemoteLogMirror.Change) async {
        let c0 = MirrorSyncs.count(key) ?? -1
        plan.set(p)
        let part = await m.sync(tail: 1 << 20)
        let c1 = MirrorSyncs.count(key) ?? -1
        check(part == want && c1 == c0, "mirror partial: \(name): a copy that stopped part way does not count (\(part), \(c0) then \(c1))")
        let rest = await m.sync(tail: 1 << 20)
        let c2 = MirrorSyncs.count(key) ?? -1
        check(c2 == c1 + 1 && bytes(key) == bytes(hostFile), "mirror partial: \(name): the next copy reads to the end and counts (\(rest), \(c2))")
    }
    await step("first fill", [true, false], want: .grew)
    try FileHandle(forWritingAtPath: hostFile).map { fh in defer { try? fh.close() }; try fh.seekToEnd(); try fh.write(contentsOf: Data((4..<7).map(line).joined().utf8)) }
    await step("the log grew", [true, false], want: .grew)
    try (10..<13).map(line).joined().write(toFile: hostFile, atomically: false, encoding: .utf8)   // smaller: replaced
    await step("after a reset", [true, false], want: .reset)
}

/// A mirror that is stopped is no mirror any more (count nil), and a late copy of its poller does not bring it back.
/// A mirror made again at the same path starts at 0, and a read that waits on it is not stuck on the old count.
func checksMirrorSyncsDrop() {
    let p = "/tmp/hk-mirror-drop-\(UUID().uuidString).jsonl"
    MirrorSyncs.register(p); MirrorSyncs.bump(p); MirrorSyncs.bump(p)
    check(MirrorSyncs.count(p) == 2, "mirror drop: two copies counted")
    MirrorSyncs.drop(p); MirrorSyncs.bump(p)
    check(MirrorSyncs.count(p) == nil, "mirror drop: a late copy of a stopped mirror does not bring it back (\(MirrorSyncs.count(p) ?? -1))")
    MirrorSyncs.bump(p); MirrorSyncs.register(p)
    check(MirrorSyncs.count(p) == 0, "mirror drop: made again: it starts at 0 (\(MirrorSyncs.count(p) ?? -1))")
    MirrorSyncs.drop(p)
    var a = SinceLeft.Arrival()
    check(!a.fresh(syncs: 3), "mirror drop: arrival: the count when it came on screen")
    check(!a.fresh(syncs: 0), "mirror drop: arrival: the mirror was made again (0): wait for its own copy")
    check(a.fresh(syncs: 1), "mirror drop: arrival: the new mirror's first finished copy: read (not stuck waiting for 4)")
}

/// hc gives at most 8 MB per read: a read cut short by that cap ends before the file does, so it is not a finished copy
/// of an agent on another machine. Only the read that reaches the file's last line counts.
@MainActor func checksMirrorByteCap() async throws {
    let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hkmcap-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("box/t8", argv: ["claude"], pid: 800, cwd: "/home/b/w")]
    let sid = "0b1c2d3e-aaaa-bbbb-cccc-000000000088"
    let log = "/home/b/.claude/projects/-home-b-w/\(sid).jsonl"
    fake.execReplies = [
        ("ps ", "/home/b\n  800     1 Mon Nov  2 10:00:00 2026 claude --resume\n"),
        (".claude/sessions", "800\t{\"pid\":800,\"sessionId\":\"\(sid)\",\"procStart\":\"Mon Nov  2 10:00:00 2026\"}\n"),
        (".claude/projects", log + "\n"),
    ]
    let whole = (0..<6).map { "{\"type\":\"user\",\"n\":\($0)}\n" }.joined()   // 25 bytes a line
    fake.files = ["box:\(log)": whole]
    fake.fileByteCap = 60   // two lines a read: three reads to the end
    fake.readDelays = ["box:\(log)": 0.15]   // slow reads: the count is seen between them
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setMirror(root: dir.appendingPathComponent("hcm").path, poll: 0.05)
    defer { Task { await hc.stopMirrors() } }
    let found = await hc.resolveRemote([SessionResolver.Need(paneId: "hc:box/t8", kind: "claude", cwd: "/home/b/w")], claimed: [])
    let p = found["hc:box/t8"] ?? ""
    for _ in 0..<600 where (MirrorSyncs.count(p) ?? 0) == 0 { try? await Task.sleep(for: .milliseconds(5)) }
    let copy = (try? String(contentsOfFile: p, encoding: .utf8)) ?? ""
    let n = copy.reduce(0) { $1 == "\n" ? $0 + 1 : $0 }
    check(!p.isEmpty && (MirrorSyncs.count(p) ?? 0) >= 1 && copy == whole,
          "mirror byte cap: hc: the first copy that counts holds the whole file (\(n) of 6 lines, count \(MirrorSyncs.count(p) ?? -1))")
    await hc.stopMirrors()
}
