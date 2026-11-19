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
    check(a.mark("a1") == "a2", "since arrival: the app's copy still shows the old turn: the read's turn is marked (\(a.mark("a1") ?? "nil"))")
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
