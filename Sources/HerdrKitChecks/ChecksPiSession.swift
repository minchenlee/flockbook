import Foundation
@testable import HerdrKit

// pi on hc: pi does not keep its session file open, so lsof finds nothing. Its file is in
// `<agent dir>/sessions/--<cwd, / \ : as ->--/` (pi session-manager.js) and starts with `{type: session, cwd,
// timestamp}`. The pane's file: the newest unclaimed one there with the pane's cwd that pi started, or wrote to
// (resume), after the agent started.

@MainActor func checksPiSession() async throws {
    let agentDir = NSTemporaryDirectory() + "hkpi-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: agentDir) }
    check(SessionResolver.piSessionDir(cwd: "/private/tmp/c9-pi-live-1", agentDir: "/h/.pi/agent") == "/h/.pi/agent/sessions/--private-tmp-c9-pi-live-1--",
          "pi: session folder named as pi names it")
    check(SessionResolver.piSessionDir(cwd: "/a/b:c", agentDir: "/x") == "/x/sessions/--a-b-c--", "pi: ':' becomes '-' too")

    let cwd = "/w/pi-proj"
    let dir = SessionResolver.piSessionDir(cwd: cwd, agentDir: agentDir)
    let fm = FileManager.default
    try fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
    let start = Date().addingTimeInterval(-600)
    let iso = ISO8601DateFormatter(); iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    func file(_ name: String, started: Date, cwd c: String = cwd, modified: Date) -> String {
        let p = dir + "/" + name + ".jsonl"
        let head = #"{"type":"session","version":3,"id":"\#(name)","timestamp":"\#(iso.string(from: started))","cwd":"\#(c)"}"# + "\n"
        fm.createFile(atPath: p, contents: Data(head.utf8))
        try? fm.setAttributes([.modificationDate: modified], ofItemAtPath: p)
        return p
    }
    let old = file("old", started: start.addingTimeInterval(-3600), modified: start.addingTimeInterval(-3000))
    let mine = file("mine", started: start.addingTimeInterval(3), modified: start.addingTimeInterval(60))
    _ = file("elsewhere", started: start.addingTimeInterval(5), cwd: "/w/other", modified: start.addingTimeInterval(90))
    let taken = file("taken", started: start.addingTimeInterval(4), modified: start.addingTimeInterval(120))

    let procs = [SessionResolver.Proc(pid: 910_001, ppid: 1, start: start, line: "node /opt/homebrew/bin/pi HC_TERMINAL_ID=t6")]
    let need = SessionResolver.Need(paneId: "hc:t6", kind: "pi", cwd: cwd, pids: [910_001])
    func resolve(_ claimed: Set<String>) async -> String? {
        await SessionResolver(socketPath: "/nonexistent", piAgentDir: agentDir).resolve([need], claimed: claimed, procs: procs)["hc:t6"]
    }
    check(await resolve([taken]) == mine, "pi: the newest unclaimed file of its cwd started after the agent (\(await resolve([taken]) ?? "nil"))")
    check(await resolve([]) == taken, "pi: newest first when nothing is claimed (/new makes a newer file)")
    // Resume: an older session that pi wrote to after it started.
    try fm.removeItem(atPath: mine); try fm.removeItem(atPath: taken)
    check(await resolve([]) == nil, "pi: an old file pi did not touch is not used")
    try fm.setAttributes([.modificationDate: start.addingTimeInterval(30)], ofItemAtPath: old)
    check(await resolve([]) == old, "pi: a resumed session (written after the agent started) is used")

    // Pure pick, as the remote path uses it: (path, mtime, first line).
    let lines: [(String, Date, Data)] = [
        ("/r/a.jsonl", start.addingTimeInterval(50), Data((#"{"type":"session","timestamp":"\#(iso.string(from: start.addingTimeInterval(2)))","cwd":"/r/w"}"#).utf8)),
        ("/r/b.jsonl", start.addingTimeInterval(80), Data(#"{"type":"model_change"}"#.utf8)),
    ]
    check(SessionResolver.piPick(lines, cwd: "/r/w", after: start, claimed: { _ in false }) == "/r/a.jsonl", "pi: pick reads only session headers")
}

/// pi on another machine: its session folder there, read with `machine.exec`, then mirrored like the others.
@MainActor func checksPiRemote() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("box/t6", argv: ["pi"], pid: 600, cwd: "/home/b/p")]
    let path = "/home/b/.pi/agent/sessions/--home-b-p--/2026-11-02T10-00-03-000Z_01aa.jsonl"
    let head = #"{"type":"session","version":3,"id":"01aa","timestamp":"2026-11-02T10:00:03.000Z","cwd":"/home/b/p"}"#
    fake.execReplies = [
        ("ps ", "/home/b\n  600     1 Mon Nov  2 10:00:00 2026 node /usr/bin/pi\n"),
        ("/.pi/agent/sessions/--home-b-p--", "\(path)\t1793613700\t\(head)\n"),
    ]
    fake.files = ["box:\(path)": head + "\n"]
    let root = NSTemporaryDirectory() + "hkpim-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setMirror(root: root, poll: 0.05)
    let found = await hc.resolveRemote([SessionResolver.Need(paneId: "hc:box/t6", kind: "pi", cwd: "/home/b/p")], claimed: [])
    check(found["hc:box/t6"]?.hasSuffix("_01aa.jsonl") == true, "pi remote: found in its cwd's session folder on box (\(found))")
    let ls = fake.calls.filter { $0.method == "machine.exec" }.compactMap { $0.params["line"] as? String }.first { $0.contains(".pi/agent") }
    check(ls?.contains("'/home/b/.pi/agent/sessions/--home-b-p--'/*.jsonl") == true, "pi remote: box's home, quoted folder (\(ls ?? "nil"))")
    await hc.stopMirrors()
}

/// pi's status on hc (live 2026-11-02: hc reports `{app: pi, state: working}` then `done`): mapped like any agent.
@MainActor func checksPiStatus() {
    func term(_ st: [String: Any]?) -> HCMapper.Term? {
        var o: [String: Any] = ["address": "t5", "argv": ["/opt/homebrew/bin/pi"], "cwd": "/w", "state": "live", "title": "π - w"]
        if let st { o["status"] = st }
        return HCMapper.Term(o)
    }
    let working = term(["app": "pi", "state": "working"]).flatMap(HCMapper.agent)
    let done = term(["app": "pi", "state": "done"]).flatMap(HCMapper.agent)
    let none = term(nil).flatMap(HCMapper.agent)
    check(working?.kind == "pi" && working?.status == .working && done?.status == .done, "pi status: hc's report maps to working / done")
    check(none?.kind == "pi" && none?.status == .unknown, "pi status: no report yet is unknown, never idle")
}

/// two pi in one cwd. A file belongs to the pi that started last at or before its header time (pi stamps the
/// header when the session is made, at start, and writes the file later). Panes are matched by start, and a found
/// path stays while its file is there, so nothing swaps or flips on rechecks.
@MainActor func checksPiTwoInOneCwd() async throws {
    let agentDir = NSTemporaryDirectory() + "hkpi2-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: agentDir) }
    let cwd = "/w/two"
    let dir = SessionResolver.piSessionDir(cwd: cwd, agentDir: agentDir)
    let fm = FileManager.default
    try fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
    let a0 = Date().addingTimeInterval(-900), b0 = a0.addingTimeInterval(300)
    let iso = ISO8601DateFormatter(); iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    func head(_ id: String, _ t: Date) -> String { #"{"type":"session","version":3,"id":"\#(id)","timestamp":"\#(iso.string(from: t))","cwd":"\#(cwd)"}"# }
    func file(_ id: String, started: Date, modified: Date) -> String {
        let p = dir + "/" + id + ".jsonl"
        fm.createFile(atPath: p, contents: Data((head(id, started) + "\n").utf8))
        try? fm.setAttributes([.modificationDate: modified], ofItemAtPath: p)
        return p
    }
    // B writes last, so B's file is the newest; A's is older.
    let fa = file("a", started: a0.addingTimeInterval(1), modified: b0.addingTimeInterval(100))
    let fb = file("b", started: b0.addingTimeInterval(1), modified: b0.addingTimeInterval(200))
    let procs = [SessionResolver.Proc(pid: 920_001, ppid: 1, start: a0, line: "node pi"),
                 SessionResolver.Proc(pid: 920_002, ppid: 1, start: b0, line: "node pi")]
    let na = SessionResolver.Need(paneId: "hc:ta", kind: "pi", cwd: cwd, pids: [920_001])
    let nb = SessionResolver.Need(paneId: "hc:tb", kind: "pi", cwd: cwd, pids: [920_002])
    for order in [[na, nb], [nb, na]] {
        let r = SessionResolver(socketPath: "/nonexistent", piAgentDir: agentDir)
        let got = await r.resolve(order, claimed: [], procs: procs)
        check(got["hc:ta"] == fa && got["hc:tb"] == fb, "pi two: each pane gets its own pi's file (\(order.map(\.paneId)): \(got))")
    }
    // A's file not written yet: A gets nothing (not B's file), B gets its own.
    try fm.removeItem(atPath: fa)
    let r = SessionResolver(socketPath: "/nonexistent", piAgentDir: agentDir)
    let got = await r.resolve([na, nb], claimed: [], procs: procs)
    check(got["hc:ta"] == nil && got["hc:tb"] == fb, "pi two: a later pi's file is not taken by the earlier one (\(got))")
    // Pure pick, remote path: rivals are the starts of the other pi in the cwd.
    let lines: [(String, Date, Data)] = [("/r/a", b0.addingTimeInterval(100), Data(head("a", a0.addingTimeInterval(1)).utf8)),
                                         ("/r/b", b0.addingTimeInterval(200), Data(head("b", b0.addingTimeInterval(1)).utf8))]
    check(SessionResolver.piPick(lines, cwd: cwd, after: a0, rivals: [b0], claimed: { _ in false }) == "/r/a"
          && SessionResolver.piPick(lines, cwd: cwd, after: b0, rivals: [a0], claimed: { _ in false }) == "/r/b",
          "pi two: pick with rivals")
}

/// two pi in one cwd on box get their own files, and a recheck keeps them (no flip, no re-copy).
@MainActor func checksPiRemoteTwo() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("box/ta", argv: ["pi"], pid: 610, cwd: "/home/b/p"),
                      FakeHC.terminal("box/tb", argv: ["pi"], pid: 620, cwd: "/home/b/p")]
    let d = "/home/b/.pi/agent/sessions/--home-b-p--/"
    let ha = #"{"type":"session","version":3,"id":"aa","timestamp":"2026-11-02T10:00:01.000Z","cwd":"/home/b/p"}"#
    let hb = #"{"type":"session","version":3,"id":"bb","timestamp":"2026-11-02T10:05:01.000Z","cwd":"/home/b/p"}"#
    // B's file has the newer mtime.
    fake.execReplies = [
        ("ps ", "/home/b\n  610     1 Mon Nov  2 10:00:00 2026 node /usr/bin/pi\n  620     1 Mon Nov  2 10:05:00 2026 node /usr/bin/pi\n"),
        ("/.pi/agent/sessions/--home-b-p--", "\(d)a.jsonl\t1793613900\t\(ha)\n\(d)b.jsonl\t1793614000\t\(hb)\n"),
    ]
    fake.files = ["box:\(d)a.jsonl": ha + "\n", "box:\(d)b.jsonl": hb + "\n"]
    let root = NSTemporaryDirectory() + "hkpim2-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setMirror(root: root, poll: 0.05)
    let needs = [SessionResolver.Need(paneId: "hc:box/ta", kind: "pi", cwd: "/home/b/p"),
                 SessionResolver.Need(paneId: "hc:box/tb", kind: "pi", cwd: "/home/b/p")]
    let found = await hc.resolveRemote(needs, claimed: [])
    check(found["hc:box/ta"]?.hasSuffix("-a.jsonl") == true && found["hc:box/tb"]?.hasSuffix("-b.jsonl") == true,
          "pi remote two: each pane its own file (\(found))")
    await hc.stopMirrors()
}

/// a missed lookup on box (the folder listing fails, the agent is not in ps once) keeps the mirror;
/// another agent in the pane drops it. A mirror ended while a read is under way gets nothing from that read when it
/// starts again.
@MainActor func checksHCRemoteKeep() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("box/t6", argv: ["pi"], pid: 600, cwd: "/home/b/p")]
    let path = "/home/b/.pi/agent/sessions/--home-b-p--/2026-11-02T10-00-03-000Z_01aa.jsonl"
    let head = #"{"type":"session","version":3,"id":"01aa","timestamp":"2026-11-02T10:00:03.000Z","cwd":"/home/b/p"}"#
    let ps = ("ps ", "/home/b\n  600     1 Mon Nov  2 10:00:00 2026 node /usr/bin/pi\n")
    let listing = ("/.pi/agent/sessions/--home-b-p--", "\(path)\t1793613700\t\(head)\n")
    fake.execReplies = [ps, listing]
    fake.files = ["box:\(path)": head + "\n"]
    let root = NSTemporaryDirectory() + "hkkeep-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setMirror(root: root, poll: 0.05, recheck: 0)
    let need = [SessionResolver.Need(paneId: "hc:box/t6", kind: "pi", cwd: "/home/b/p")]
    func content(_ p: String?) -> String { p.flatMap { try? String(contentsOfFile: $0, encoding: .utf8) } ?? "" }
    func waitFor(_ p: String?, _ want: String) async -> Bool {
        for _ in 0..<60 { if content(p) == want { return true }; try? await Task.sleep(nanoseconds: 50_000_000) }
        return false
    }
    let first = await hc.resolveRemote(need, claimed: [])["hc:box/t6"]
    check(await waitFor(first, head + "\n"), "keep: mirrored (\(first ?? "nil"))")
    fake.execReplies = [ps]   // the listing fails once
    let second = await hc.resolveRemote(need, claimed: [])["hc:box/t6"]
    check(second == first && FileManager.default.fileExists(atPath: first ?? "/"), "keep: a failed listing keeps the mirror (\(second ?? "nil"))")
    fake.execReplies = [("ps ", "/home/b\n  1     0 Mon Nov  2 09:00:00 2026 launchd\n"), listing]   // not in ps once
    let third = await hc.resolveRemote(need, claimed: [])["hc:box/t6"]
    check(third == first && FileManager.default.fileExists(atPath: first ?? "/"), "keep: an agent missing from ps once keeps the mirror (\(third ?? "nil"))")
    fake.execReplies = [("ps ", "/home/b\n  601     1 Mon Nov  2 11:00:00 2026 node /usr/bin/pi\n")]
    fake.terminals = [FakeHC.terminal("box/t6", argv: ["pi"], pid: 601, cwd: "/home/b/p")]
    let fourth = await hc.resolveRemote(need, claimed: [])["hc:box/t6"]
    check(fourth == nil && !FileManager.default.fileExists(atPath: first ?? "/"), "keep: another agent in the pane drops the old mirror (\(fourth ?? "nil"))")

    // A read under way when the mirror ends must not land in the next mirror of the same log.
    fake.terminals = [FakeHC.terminal("box/t6", argv: ["pi"], pid: 600, cwd: "/home/b/p")]
    fake.execReplies = [ps, listing]
    fake.readDelays = ["box:\(path)": 0.4]
    let again = await hc.resolveRemote(need, claimed: [])["hc:box/t6"]
    try await Task.sleep(nanoseconds: 100_000_000)            // its first read is under way
    _ = await hc.resolveRemote([], claimed: [])                // the pane goes: the mirror ends
    let back = await hc.resolveRemote(need, claimed: [])["hc:box/t6"]
    check(back == again, "keep: same log, same mirror path")
    try await Task.sleep(nanoseconds: 1_200_000_000)
    check(content(back) == head + "\n", "keep: an ended mirror's late read is not appended (\(content(back).count) bytes)")
    await hc.stopMirrors()
}
