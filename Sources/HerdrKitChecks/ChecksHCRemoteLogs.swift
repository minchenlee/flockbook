import Foundation
@testable import HerdrKit

// Parity item 3c: conversations of agents on another machine. The log is found there (`machine.exec`: ps with
// TZ=UTC0, Claude's `<pid>.json`, lsof, Codex rollouts' first lines) and mirrored into a local file by polling
// `file.read {on, offset}`, so the same SessionLog tails it. Only whole lines are copied.

@MainActor func checksHCRemoteLogs() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [
        FakeHC.terminal("box/t7", argv: ["claude"], pid: 700, cwd: "/home/b/w"),
        FakeHC.terminal("box/t8", argv: ["codex"], pid: 800, cwd: "/home/b/w"),
        FakeHC.terminal("box/t9", argv: ["codex"], pid: 900, cwd: "/home/b/x"),
        FakeHC.terminal("t1", argv: ["claude"], pid: 700, cwd: "/w"),
    ]
    let sid = "0b1c2d3e-aaaa-bbbb-cccc-000000000001"
    let claudeLog = "/home/b/.claude/projects/-home-b-w/\(sid).jsonl"
    let codexLog = "/home/b/.codex/sessions/2026/11/02/rollout-2026-11-02T10-00-00-abc.jsonl"
    let codexMeta = #"{"type":"session_meta","timestamp":"2026-11-02T10:00:01.000Z","payload":{"cwd":"/home/b/x","timestamp":"2026-11-02T10:00:01.000Z"}}"#
    let codexLog2 = "/home/b/.codex/sessions/2026/11/02/rollout-2026-11-02T10-00-01-def.jsonl"
    fake.execReplies = [
        ("ps ", "/home/b\n  700     1 Mon Nov  2 10:00:00 2026 claude --resume\n  800     1 Mon Nov  2 10:00:00 2026 codex\n  900     1 Mon Nov  2 10:00:00 2026 codex\n"),
        (".claude/sessions", "700\t{\"pid\":700,\"sessionId\":\"\(sid)\",\"procStart\":\"Mon Nov  2 10:00:00 2026\"}\n"),
        (".claude/projects", claudeLog + "\n"),
        ("lsof", "p800\nn/home/b/.codex/sessions/2026/11/02/rollout-2026-11-02T10-00-00-abc.jsonl\nn/dev/null\n"),
        ("rollout-", "\(codexLog2)\t\(codexMeta)\n"),
    ]
    fake.files = ["box:\(claudeLog)": "{\"type\":\"user\",\"n\":1}\n{\"type\":\"assistant\",\"n\":2}\n",
                  "box:\(codexLog)": "{\"a\":1}\n"]
    let root = NSTemporaryDirectory() + "hkmirror-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    // copies left by an earlier run are removed when mirroring starts (a gone process's).
    try FileManager.default.createDirectory(atPath: root + "/999999", withIntermediateDirectories: true)
    FileManager.default.createFile(atPath: root + "/999999/0123456789abcdef-stale.jsonl", contents: Data("x\n".utf8))
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setMirror(root: root, poll: 0.05)
    let needs = [SessionResolver.Need(paneId: "hc:box/t7", kind: "claude", cwd: "/home/b/w"),
                 SessionResolver.Need(paneId: "hc:box/t8", kind: "codex", cwd: "/home/b/w"),
                 SessionResolver.Need(paneId: "hc:box/t9", kind: "codex", cwd: "/home/b/x"),
                 SessionResolver.Need(paneId: "hc:t1", kind: "claude", cwd: "/w")]
    let found = await hc.resolveRemote(needs, claimed: [])
    check(found["hc:t1"] == nil, "remote logs: a local pane is not resolved remotely")
    let ps = fake.calls.filter { $0.method == "machine.exec" }.first { (($0.params["line"] as? String) ?? "").contains("ps ") }
    check(ps?.params["on"] as? String == "box" && (ps?.params["env"] as? [String: String])?["TZ"] == "UTC0",
          "remote logs: ps runs on box with TZ=UTC0 (\(ps?.params ?? [:]))")

    func content(_ p: String?) -> String { p.flatMap { try? String(contentsOfFile: $0, encoding: .utf8) } ?? "" }
    func waitFor(_ p: String?, _ want: String) async -> Bool {
        for _ in 0..<60 { if content(p) == want { return true }; try? await Task.sleep(nanoseconds: 50_000_000) }
        return false
    }
    let cp = found["hc:box/t7"]
    check(cp?.hasPrefix(root) == true && cp?.hasSuffix("\(sid).jsonl") == true, "remote logs: Claude on box mirrors to a local .jsonl (\(cp ?? "nil"))")
    check(await waitFor(cp, "{\"type\":\"user\",\"n\":1}\n{\"type\":\"assistant\",\"n\":2}\n"), "remote logs: the mirror holds the remote log (\(content(cp)))")
    let reads = fake.calls.filter { $0.method == "file.read" }
    check(reads.contains { $0.params["on"] as? String == "box" && $0.params["path"] as? String == claudeLog },
          "remote logs: file.read on box for the Claude log")

    // Appends follow; a half-written last line waits for its end.
    fake.files["box:\(claudeLog)"]! += "{\"n\":3}\n{\"n\":4"
    check(await waitFor(cp, "{\"type\":\"user\",\"n\":1}\n{\"type\":\"assistant\",\"n\":2}\n{\"n\":3}\n"), "remote logs: new whole lines follow, a partial one waits (\(content(cp)))")
    fake.files["box:\(claudeLog)"]! += "}\n"
    check(await waitFor(cp, "{\"type\":\"user\",\"n\":1}\n{\"type\":\"assistant\",\"n\":2}\n{\"n\":3}\n{\"n\":4}\n"), "remote logs: the line completes (\(content(cp)))")
    // A rewritten (shorter) file starts over.
    fake.files["box:\(claudeLog)"] = "{\"new\":1}\n"
    check(await waitFor(cp, "{\"new\":1}\n"), "remote logs: a shorter remote file is copied again (\(content(cp)))")

    // Codex: the log its tree holds open (lsof there, box's home), else the rollout by cwd and start time.
    check(found["hc:box/t8"]?.hasSuffix("rollout-2026-11-02T10-00-00-abc.jsonl") == true, "remote logs: Codex via lsof on box (\(found["hc:box/t8"] ?? "nil"))")
    check(await waitFor(found["hc:box/t8"], "{\"a\":1}\n"), "remote logs: the Codex mirror holds its log")
    check(found["hc:box/t9"]?.hasSuffix("rollout-2026-11-02T10-00-01-def.jsonl") == true, "remote logs: Codex rollout by cwd + start on box (\(found["hc:box/t9"] ?? "nil"))")

    // A claimed log is not handed out again.
    let again = await HCClient(session: HCSession(socketPath: fake.path)).resolveRemote([needs[0]], claimed: [sid])
    check(again.isEmpty, "remote logs: a claimed session is not used twice (\(again))")

    // private copies: owner-only, removed when their pane goes away and when mirroring stops.
    let fm = FileManager.default
    func mode(_ p: String) -> Int { ((try? fm.attributesOfItem(atPath: p))?[.posixPermissions] as? NSNumber)?.intValue ?? -1 }
    check(!fm.fileExists(atPath: root + "/999999/0123456789abcdef-stale.jsonl"), "remote logs: an earlier run's copies are removed")
    if let cp {
        let dir = (cp as NSString).deletingLastPathComponent
        check(mode(cp) == 0o600 && mode(dir) == 0o700,
              "remote logs: mirror file 0600, its folder 0700 (\(String(mode(cp), radix: 8)) \(String(mode(dir), radix: 8)))")
    }
    _ = await hc.resolveRemote(Array(needs.dropFirst()), claimed: [])
    check(cp.map { !fm.fileExists(atPath: $0) } == true && found["hc:box/t8"].map { fm.fileExists(atPath: $0) } == true,
          "remote logs: a gone pane's mirror is deleted, the others stay")

    // a line too long for one read is skipped and reported; the copy goes on after it. A file that is not
    // UTF-8 cannot be read at all (hc refuses it from any line): reported, the copy keeps what it has.
    fake.fileByteCap = 64
    let big = "{\"big\":\"" + String(repeating: "x", count: 100) + "\"}\n"
    fake.files["box:\(codexLog)"]! += big + "{\"b\":2}\n"
    let cx = found["hc:box/t8"]
    check(await waitFor(cx, "{\"a\":1}\n{\"b\":2}\n"), "remote logs: an oversized line is skipped (\(content(cx)))")
    fake.files["box:\(codexLog)"]! += "{\"bad\":\"\u{0}\"}\n{\"c\":3}\n"
    var probs: [String: String] = [:]
    for _ in 0..<60 { probs = await hc.mirrorProblems(); if probs[cx ?? ""]?.contains("UTF-8") == true { break }; try? await Task.sleep(nanoseconds: 50_000_000) }
    check(probs[cx ?? ""]?.contains("line 2") == true && probs[cx ?? ""]?.contains("UTF-8") == true && content(cx) == "{\"a\":1}\n{\"b\":2}\n",
          "remote logs: the skipped line and the unreadable file are reported (\(probs))")
    await hc.stopMirrors()
    check(found.values.allSatisfy { !fm.fileExists(atPath: $0) }, "remote logs: stopping removes every mirror")

    // The store sends only remote panes there; local ones stay with the local resolver.
    check(HerdrStore.isRemotePane("hc:box/t7") && !HerdrStore.isRemotePane("hc:t1") && !HerdrStore.isRemotePane("w1:p2"),
          "remote logs: the store tells remote panes apart")
}

/// another machine may be a day ahead or behind this one: its rollout day folder can be tomorrow here.
@MainActor func checksHCRemoteDayFolders() {
    let cal = Calendar.current
    let now = Date()
    func folder(_ d: Date) -> String {
        let c = cal.dateComponents([.year, .month, .day], from: d)
        return String(format: "/s/%04d/%02d/%02d", c.year ?? 0, c.month ?? 0, c.day ?? 0)
    }
    let f = SessionResolver.codexDayFolders(dir: "/s", after: now, now: now, slackDays: 1)
    let yesterday = cal.date(byAdding: .day, value: -1, to: now)!, tomorrow = cal.date(byAdding: .day, value: 1, to: now)!
    check(f == [folder(yesterday), folder(now), folder(tomorrow)], "remote logs: day folders a day before and after here (\(f))")
    let local = SessionResolver.codexDayFolders(dir: "/s", after: now, now: now)
    check(local == [folder(now)], "remote logs: no slack, today only (\(local))")
}

/// mirror names cannot collide or leave the mirror folder, whatever the machine and path are.
@MainActor func checksHCMirrorPath() async {
    let hc = HCClient(session: HCSession(socketPath: "/nonexistent"))
    await hc.setMirror(root: "/r", poll: 1)
    let a = await hc.mirrorPath("a_b", "/c/x.jsonl"), b = await hc.mirrorPath("a", "/b_c/x.jsonl")
    let c = await hc.mirrorPath("a/b", "/c/x.jsonl"), d = await hc.mirrorPath("a", "/b/c/x.jsonl")
    check(Set([a, b, c, d]).count == 4, "mirror path: distinct machine + path pairs never share a file (\([a, b, c, d]))")
    let dots = await hc.mirrorPath("..", "/../../etc/x.jsonl")
    let mine = "/r/\(getpid())/"
    check(dots.hasPrefix(mine) && !dots.dropFirst(mine.count).contains("/") && dots.hasSuffix("x.jsonl"),
          "mirror path: one file right in the mirror folder, ending in the log's name (\(dots))")
    let long = await hc.mirrorPath("m", "/p/" + String(repeating: "é", count: 300) + ".jsonl")
    check(((long as NSString).lastPathComponent).utf8.count <= 200, "mirror path: the name fits a file name in bytes")
}

/// starting mirrors never wipes the root. Each process mirrors into `<root>/<pid>/`; at start only copies
/// (`<16 hex>-name`) in the folders of processes that are gone are deleted. Other files, other folders and a live
/// process's copies stay.
@MainActor func checksHCMirrorRootSafe() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("box/t8", argv: ["codex"], pid: 800, cwd: "/home/b/w")]
    let log = "/home/b/.codex/sessions/2026/11/02/rollout-2026-11-02T10-00-00-abc.jsonl"
    fake.execReplies = [("ps ", "/home/b\n  800     1 Mon Nov  2 10:00:00 2026 codex\n"), ("lsof", "p800\nn\(log)\n")]
    fake.files = ["box:\(log)": "{\"a\":1}\n"]
    let fm = FileManager.default
    let root = NSTemporaryDirectory() + "hkroot-\(UUID().uuidString)"
    defer { try? fm.removeItem(atPath: root) }
    let live = String(getppid()), dead = "999999"   // a running process; a pid no process has (above macOS's pid_max)
    let copy = "0123456789abcdef-x.jsonl"
    for d in ["notes", live, dead] { try fm.createDirectory(atPath: root + "/" + d, withIntermediateDirectories: true) }
    for f in ["notes/keep.txt", "\(live)/\(copy)", "\(dead)/\(copy)", "\(dead)/mine.txt", "readme.txt"] {
        fm.createFile(atPath: root + "/" + f, contents: Data("x\n".utf8))
    }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setMirror(root: root, poll: 0.05)
    let found = await hc.resolveRemote([SessionResolver.Need(paneId: "hc:box/t8", kind: "codex", cwd: "/home/b/w")], claimed: [])
    let p = found["hc:box/t8"] ?? ""
    check(p.hasPrefix(root + "/\(getpid())/"), "root safe: this process mirrors into its own folder (\(p))")
    check(fm.fileExists(atPath: root + "/notes/keep.txt") && fm.fileExists(atPath: root + "/readme.txt"),
          "root safe: files and folders that are not copies stay")
    check(fm.fileExists(atPath: root + "/\(live)/\(copy)"), "root safe: a running process's copies stay")
    check(!fm.fileExists(atPath: root + "/\(dead)/\(copy)") && fm.fileExists(atPath: root + "/\(dead)/mine.txt"),
          "root safe: a gone process's copies go; other files in its folder stay")
    await hc.stopMirrors()
    check(!fm.fileExists(atPath: root + "/\(getpid())"), "root safe: this process's empty folder goes when mirroring stops")
}
