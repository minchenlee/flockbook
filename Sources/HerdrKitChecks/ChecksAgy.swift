import Foundation
@testable import HerdrKit

// agy (Antigravity CLI): synthetic fixtures shaped like agy 1.4.12 transcripts.

// Transcript (`brain/<id>/.system_generated/logs/transcript_full.jsonl`): one step per line, a tool's result is the
// next GENERIC step (no call ids), thinking and system steps hidden, a call without a result is pending.
@MainActor func checksAgyParser() async throws {
    let al = String(decoding: fixture("agy.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    let items = SessionLog.parse(lines: al)
    check(describe(items) == ["user:List the files.", "asst:Looking.", "chips:Ran a command|Read a file|Edited a file", "asst:Done with a.txt.",
                              "status", "user:Build it.", "asst:Building.", "perm:make"], "agy: items \(describe(items))")
    let calls = chipCalls(items)
    check(calls.map(\.toolName) == ["Bash", "Read", "Edit"] && calls.map(\.summary) == ["ls", "/tmp/demo/a.txt", "/tmp/demo/a.txt"],
          "agy: tool names and summaries \(calls.map(\.toolName)) \(calls.map(\.summary))")
    check(calls.map(\.isError) == [false, false, true] && calls[0].output?.hasSuffix("a.txt") == true
          && calls[1].output?.hasSuffix("hello") == true && calls[2].output == "no match"
          && !(calls[0].output ?? "").contains("Created At"), "agy: results by order, header lines cut \(calls.map { $0.output ?? "nil" })")
    check(!items.contains { if case .assistantText(let t) = $0.kind { return t.contains("Hidden") || t.contains("SYSTEM_MESSAGE") }; return false },
          "agy: thinking and system messages hidden")
    check(items.first?.timestamp == ISO8601DateFormatter().date(from: "2026-11-02T02:00:00Z"), "agy: created_at as time")
    if case .status(let s) = items[4].kind { check(s == "Context compacted", "agy: checkpoint is a compaction marker") }
    if case .permissionRequest(let r) = items.last?.kind { check(r.toolName == "Bash" && r.title == "Wants to run a command", "agy: pending call as a request") }
    var p = ConversationParser()
    for l in al { p.feed(line: l) }
    check(p.hasOpenToolUse && p.items().map(\.id) == items.map(\.id), "agy: incremental feed = batch, open call")
    check(SessionLog.parse(lines: al).map(\.id) == items.map(\.id) && Set(items.map(\.id)).count == items.count, "agy: stable unique ids")
    // A background task ("RUNNING") answers its call: it is no longer pending.
    let bg = #"{"step_index":12,"source":"MODEL","type":"GENERIC","status":"RUNNING","created_at":"2026-11-02T02:00:12Z","content":"Created At: x\nTool is running as a background task with task id: c/task-12"}"#
    check(!SessionLog.parse(lines: al + [bg]).contains { if case .permissionRequest = $0.kind { return true }; return false },
          "agy: background task closes the call")
    // Prompts without the wrapper are shown as typed; tagged injected text is not.
    let plain = #"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-02T02:00:00Z","content":"hi"}"#
    let tagged = #"{"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-02T02:00:00Z","content":"<USER_SETTINGS_CHANGE>x</USER_SETTINGS_CHANGE>"}"#
    check(describe(SessionLog.parse(lines: [plain, tagged])) == ["user:hi"], "agy: plain prompt kept, tagged-only dropped")
}

// Log from the process: agy holds `presence/<conversation id>.lock` open; the transcript sits under `brain/<id>`.
@MainActor func checksAgyResolver() async throws {
    typealias R = SessionResolver
    let root = NSTemporaryDirectory() + "agy-\(UUID().uuidString)"
    let id = "869e05de-dfbe-46df-be0f-832f29cb9fac", id2 = "d981a68a-038b-4745-aea1-6f1672dfbb1f"
    let logs = "\(root)/brain/\(id)/.system_generated/logs"
    try? FileManager.default.createDirectory(atPath: logs, withIntermediateDirectories: true)
    FileManager.default.createFile(atPath: logs + "/transcript_full.jsonl", contents: Data())
    let lock = "\(root)/presence/\(id).lock", lock2 = "\(root)/presence/\(id2).lock"
    let open = ["\(root)/log/cli-20261101_005021.log", "\(root)/settings.json", "\(root)/brain/\(id)", lock]
    check(R.agyLogPath(openFiles: open, dataDir: root) == logs + "/transcript_full.jsonl", "agy: open presence lock names the transcript")
    check(R.agyLogPath(openFiles: open + [lock2], dataDir: root) == nil, "agy: two locks: no guess")
    check(R.agyLogPath(openFiles: [lock2], dataDir: root) == nil, "agy: no transcript yet: nil")
    check(R.agyLogPath(openFiles: Array(open.dropLast()), dataDir: root) == nil, "agy: no lock (no conversation yet): nil")
    check(R.agyLogPath(openFiles: ["\(root)/presence/../x.lock", "\(root)/presence/not-a-uuid.lock"], dataDir: root) == nil, "agy: odd lock names ignored")
    try? FileManager.default.removeItem(atPath: root)
    let procs = R.parsePS("""
      10     1 Sun Nov  1 00:50:00 2026     -zsh TERM=x
      11    10 Sun Nov  1 00:50:20 2026     agy TERM=x
    """)
    check(R.hcAgentProcess(pids: [10], kind: "agy", procs: procs)?.pid == 11, "agy: hc finds the agy process under the shell")
}

