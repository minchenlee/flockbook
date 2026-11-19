import Foundation
@testable import HerdrKit

// The fixes after the review of the Since you left card and Summaries (each failed before its fix).

private func scratch<T>(_ body: (URL) async throws -> T) async rethrows -> T {
    let root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hksincerev-\(UUID().uuidString)")
    try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    return try await body(root)
}

/// Codex rows get their ids from where their line starts in the file: a log read for Summaries gives the same ids
/// as the app's tailer, so a kept summary shows on the card.
func checksLogTailsCodexIds() async {
    await scratch { dir in
        let lines = [
            #"{"timestamp":"2026-11-03T01:40:00.000Z","type":"session_meta","payload":{"id":"s1","cwd":"/w"}}"#,
            #"{"timestamp":"2026-11-03T01:40:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the importer."}]}}"#,
            #"{"timestamp":"2026-11-03T01:40:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done. The importer keeps quoted newlines."}]}}"#,
            "",
            #"{"timestamp":"2026-11-03T01:40:03.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Say if I should add a test."}]}}"#,
        ]
        let url = dir.appendingPathComponent("rollout.jsonl")
        try? Data((lines.joined(separator: "\n") + "\n").utf8).write(to: url)
        // As the tailer reads it: each line with the place where it starts.
        var p = ConversationParser(), at: UInt64 = 0
        for l in lines { p.feed(line: l, offset: at); at += UInt64(l.utf8.count + 1) }
        let tailer = Glance.facts(p.items(), agent: "codex").id
        let t = LogTails()
        let read = Glance.facts(t.items(url) ?? [], agent: "codex").id
        check(tailer != nil && read == tailer, "log tails: a Codex turn has the tailer's id (\(read ?? "nil") vs \(tailer ?? "nil"))")
        let cut = Glance.facts(LogTails().items(url, bytes: lines[4].utf8.count + lines[3].utf8.count + 30) ?? [], agent: "codex").id
        check(cut == tailer, "log tails: a Codex turn read from the middle of the file has the tailer's id (\(cut ?? "nil"))")
    }
}

/// A run of a model ends with the task that asked for it (the app quits, the round is called off).
func checksProcessRunnerCancel() async {
    await scratch { dir in
        let pidFile = dir.appendingPathComponent("pid")
        let p = dir.appendingPathComponent("hang")
        try? Data("#!/bin/sh\necho $$ > '\(pidFile.path)'\nexec sleep 30\n".utf8).write(to: p)
        try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: p.path)
        let run = Task { await ProcessRunner().run(argv: [p.path], input: "", cwd: dir.path, timeout: 8) }
        var pid: Int32 = 0
        for _ in 0..<300 where pid == 0 {
            try? await Task.sleep(for: .milliseconds(50))
            pid = Int32(((try? String(contentsOf: pidFile, encoding: .utf8)) ?? "").trimmingCharacters(in: .whitespacesAndNewlines)) ?? 0
        }
        try? await Task.sleep(for: .milliseconds(200))
        let began = Date()
        run.cancel()
        let r = await run.value
        let took = Date().timeIntervalSince(began)
        check(pid > 0 && took < 5 && r.code == nil, "process runner: a called-off run stops at once (\(String(format: "%.1f", took)) s, \(r))")
        check(pid > 0 && kill(pid, 0) != 0, "process runner: a called-off run leaves no program running (pid \(pid))")
    }
}

/// The Model field keeps the name only on Return, when it loses the keys, or on a click: typing only narrows the list.
func checksSummaryModelField() {
    var f = Summary.ModelField(saved: "opencode-go/glm-5.3-flash")
    check(f.type("q") == nil && f.type("qw") == nil && f.type("qwen") == nil && f.saved == "opencode-go/glm-5.3-flash", "summary model field: typing keeps the saved name (\(f.saved))")
    let all = [(id: "opencode-go/glm-5.3-flash", note: ""), (id: "openrouter/qwen-coder:free", note: "")]
    check(Summary.matches(all, typed: f.typed, saved: f.saved).map(\.id) == ["openrouter/qwen-coder:free"], "summary model field: what is typed narrows the list")
    check(f.commit() == "qwen" && f.saved == "qwen", "summary model field: Return or leaving the field keeps it")
    check(f.commit() == nil, "summary model field: the same name: nothing to keep")
    check(f.pick("openrouter/qwen-coder:free") == "openrouter/qwen-coder:free" && f.typed == "openrouter/qwen-coder:free", "summary model field: a click keeps the name")
    _ = f.type("")
    check(f.saved == "openrouter/qwen-coder:free" && f.commit() == "", "summary model field: an emptied field keeps the default on Return")
    _ = f.type("  pi/x  ")
    check(f.commit() == "pi/x", "summary model field: kept without spaces")
}

/// The way in: the card waits for a read of the log made after the conversation came on screen.
func checksSinceArrival() {
    let now: Int64 = 100 * 60_000, seen = SinceLeft.Seen(id: "a1", at: now - 47 * 60_000)
    var a = SinceLeft.Arrival()
    let none = a.first(nil, shown: "a1", seen: seen, now: now)
    check(!none.decided && none.away == nil && a.mark("a1") == nil, "since arrival: no read of the log yet: no card, no mark")
    let back = a.first("a2", shown: "a1", seen: seen, now: now + 1000)
    check(back.decided && back.away == .init(from: seen.at, back: now + 1000), "since arrival: the first read decides: back after 47 minutes with a new turn")
    check(a.mark("a1") == nil && a.mark(nil) == nil, "since arrival: the app's copy still shows the old turn: nothing is marked yet (\(a.mark("a1") ?? "nil"))")
    check(a.mark("a2") == "a2", "since arrival: once the read's turn shows, it is marked")
    check(a.mark("a3") == "a3", "since arrival: a newer turn in the app's copy is marked")
    let again = a.first("a3", shown: "a3", seen: seen, now: now + 2000)
    check(again.away == nil, "since arrival: decided once")
    var same = SinceLeft.Arrival()
    check(same.first("a1", shown: "a1", seen: seen, now: now).decided && same.mark("a1") == "a1", "since arrival: nothing new: no card, the turn is marked")
    var soon = SinceLeft.Arrival()
    check(soon.first("a2", shown: "a2", seen: seen, now: seen.at + 60_000).away == nil, "since arrival: away less than 5 minutes: no card")
}

/// An approval card in the conversation holds the place too: the Since you left card waits.
func checksSinceWaits() {
    check(!SinceLeft.waits(question: false, stopCard: false, composerCard: false, conversationCard: false), "since waits: no other card: the card shows")
    check(SinceLeft.waits(question: true, stopCard: false, composerCard: false, conversationCard: false)
          && SinceLeft.waits(question: false, stopCard: true, composerCard: false, conversationCard: false)
          && SinceLeft.waits(question: false, stopCard: false, composerCard: true, conversationCard: false), "since waits: a question, stop or approval card in the reply box")
    check(SinceLeft.waits(question: false, stopCard: false, composerCard: false, conversationCard: true), "since waits: an approval card in the conversation")
}

/// Summaries run as the Herdr app's helper runs them: through herdr-core on this Mac, with its environment. Without
/// herdr-core there is no summary (the card shows the line from the log), so a turn never goes out with the app's own
/// environment, which the agent may not share.
func checksSummaryRoute() {
    check(Summary.route(backend: .hc, core: true) == .core, "summary route: an hc pane: through herdr-core")
    check(Summary.route(backend: .tui, core: true) == .core, "summary route: a herdr pane on a Mac with herdr-core: through herdr-core too")
    check(Summary.route(backend: .tui, core: false) == nil && Summary.route(backend: .hc, core: false) == nil, "summary route: no herdr-core: no summary")
    let p = HCExecRunner.payload(argv: ["claude", "-p"], input: "T", cwd: "/tmp/x", timeout: 60)
    check(Set(p.keys) == ["argv", "input", "timeout", "cwd"] && p["input"] as? String == "VA==", "summary route: machine.exec gets the helper's fields and no environment")
}

/// A mirrored log (an agent on another machine, copied here) is read for the way in only after a copy that finished
/// after the conversation came on screen: a copy that is behind does not use the one decision.
func checksSinceArrivalMirror() {
    var local = SinceLeft.Arrival()
    check(local.fresh(syncs: nil), "since arrival mirror: a log on this Mac: read now")
    var m = SinceLeft.Arrival()
    check(!m.fresh(syncs: 3) && !m.fresh(syncs: 3), "since arrival mirror: no copy since it came on screen: wait")
    check(m.fresh(syncs: 4), "since arrival mirror: a copy finished since then: read")
}

/// Each kind of mirror counts its finished copies: herdr over ssh (`RemoteLogMirror`) and hc's other machines.
@MainActor func checksMirrorSyncs() async throws {
    do {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hkmsync-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        // herdr over ssh: `Via.local` plays the host.
        let saved = RemoteLogMirror.directory
        RemoteLogMirror.directory = dir.appendingPathComponent("cache", isDirectory: true)
        defer { RemoteLogMirror.directory = saved }
        let hostFile = dir.appendingPathComponent("host.jsonl").path
        try "{\"a\":1}\n".write(toFile: hostFile, atomically: false, encoding: .utf8)
        let m = try RemoteLogMirror(host: RemoteHost(via: .local), path: hostFile)
        let c0 = MirrorSyncs.count(m.url.path)
        let s1 = await m.sync(tail: 1 << 20)
        let c1 = MirrorSyncs.count(m.url.path)
        try? FileManager.default.removeItem(atPath: hostFile)
        let s2 = await m.sync(tail: 1 << 20)
        let c2 = MirrorSyncs.count(m.url.path)
        check(c0 == 0 && s1 == .grew && c1 == 1, "mirror syncs: ssh: a finished copy counts (\(String(describing: c0)), \(s1), \(String(describing: c1)))")
        check(s2 == .gone && c2 == 1, "mirror syncs: ssh: a copy that did not finish does not count (\(s2), \(String(describing: c2)))")
        check(MirrorSyncs.count(hostFile) == nil, "mirror syncs: a file that is no mirror: nil")

        // hc: an agent on the machine box.
        let fake = FakeHC(path: fakeHCPath())
        try fake.start()
        defer { fake.kill() }
        fake.machines = [FakeHC.machine("box")]
        fake.terminals = [FakeHC.terminal("box/t7", argv: ["claude"], pid: 700, cwd: "/home/b/w")]
        let sid = "0b1c2d3e-aaaa-bbbb-cccc-000000000077"
        let log = "/home/b/.claude/projects/-home-b-w/\(sid).jsonl"
        fake.execReplies = [
            ("ps ", "/home/b\n  700     1 Mon Nov  2 10:00:00 2026 claude --resume\n"),
            (".claude/sessions", "700\t{\"pid\":700,\"sessionId\":\"\(sid)\",\"procStart\":\"Mon Nov  2 10:00:00 2026\"}\n"),
            (".claude/projects", log + "\n"),
        ]
        fake.files = ["box:\(log)": "{\"type\":\"user\",\"n\":1}\n"]
        let hc = HCClient(session: HCSession(socketPath: fake.path))
        await hc.setMirror(root: dir.appendingPathComponent("hcm").path, poll: 0.05)
        let found = await hc.resolveRemote([SessionResolver.Need(paneId: "hc:box/t7", kind: "claude", cwd: "/home/b/w")], claimed: [])
        let p = found["hc:box/t7"] ?? ""
        for _ in 0..<60 where (MirrorSyncs.count(p) ?? 0) == 0 { try? await Task.sleep(for: .milliseconds(50)) }
        let h1 = MirrorSyncs.count(p) ?? 0
        check(!p.isEmpty && h1 >= 1, "mirror syncs: hc: a finished copy counts (\(h1))")
        fake.files = [:]   // reads fail now
        try? await Task.sleep(for: .milliseconds(150))
        let h2 = MirrorSyncs.count(p) ?? 0
        try? await Task.sleep(for: .milliseconds(400))
        check(MirrorSyncs.count(p) == h2, "mirror syncs: hc: failing reads do not count (\(h2) then \(MirrorSyncs.count(p) ?? -1))")
        await hc.stopMirrors()
    }
}
