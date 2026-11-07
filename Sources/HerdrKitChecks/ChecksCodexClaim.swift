import Foundation
@testable import HerdrKit

/// Owner bug 2026-10-31: herdr's TUI reported the hc Codex's rollout id for a TUI pane in another folder. That claim
/// hid the rollout from the hc pane, whose cwd and start time match it. A claim from a pane in a different cwd than the
/// rollout's own cwd is wrong and must not block the cwd + time match.
@MainActor func checksCodexForeignClaim() async throws {
    let root = NSTemporaryDirectory() + "hkcx-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    let start = Date().addingTimeInterval(-120)
    let t = start.addingTimeInterval(5)
    let c = Calendar.current.dateComponents([.year, .month, .day], from: t)
    let day = String(format: "%@/%04d/%02d/%02d", root, c.year!, c.month!, c.day!)
    try FileManager.default.createDirectory(atPath: day, withIntermediateDirectories: true)
    let f = DateFormatter(); f.locale = Locale(identifier: "en_US_POSIX"); f.dateFormat = "yyyy-MM-dd'T'HH-mm-ss"
    let iso = ISO8601DateFormatter(); iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    let id = "01a192a3-11b5-78a2-9440-4e4d9874a762"
    let path = "\(day)/rollout-\(f.string(from: t))-\(id).jsonl"
    let meta = #"{"timestamp":"x","type":"session_meta","payload":{"id":"\#(id)","timestamp":"\#(iso.string(from: t))","cwd":"/w/desk"}}"# + "\n"
    FileManager.default.createFile(atPath: path, contents: Data(meta.utf8))

    let procs = [SessionResolver.Proc(pid: 900_001, ppid: 1, start: start, line: "node /opt/bin/codex HC_TERMINAL_ID=t3")]
    let need = SessionResolver.Need(paneId: "hc:t3", kind: "codex", cwd: "/w/desk", pids: [900_001])
    func resolve(_ claimedCwd: [String: String]) async -> String? {
        await SessionResolver(socketPath: "/nonexistent", codexDir: root).resolve([need], claimed: [id], claimedCwd: claimedCwd, procs: procs)["hc:t3"]
    }
    check(await resolve([id: "/w/other/in-ac"]) == path, "codex claim: a claim from a pane in another cwd does not hide the matching rollout")
    check(await resolve([id: "/w/desk"]) == nil, "codex claim: a claim from a pane in the rollout's cwd still counts")
    check(await resolve([:]) == nil, "codex claim: a claim without a known cwd still counts")
}

/// Codex writes a rollout for each subagent it starts (the approvals reviewer "guardian", `spawn_agent` helpers) in the
/// same cwd, with `source: {"subagent": ...}`, and keeps it open. It is not the pane's conversation: not a cwd
/// candidate, and not among the pane's open rollouts (herdr-core resolver.rs, the same rules).
@MainActor func checksCodexSubagentRollouts() async throws {
    let home = NSTemporaryDirectory() + "hkcs-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: home) }
    let root = home + "/.codex/sessions"
    let start = Date().addingTimeInterval(-120)
    let c = Calendar.current.dateComponents([.year, .month, .day], from: start)
    let day = String(format: "%@/%04d/%02d/%02d", root, c.year!, c.month!, c.day!)
    try FileManager.default.createDirectory(atPath: day, withIntermediateDirectories: true)
    let f = DateFormatter(); f.locale = Locale(identifier: "en_US_POSIX"); f.dateFormat = "yyyy-MM-dd'T'HH-mm-ss"
    let iso = ISO8601DateFormatter(); iso.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    func roll(_ dt: TimeInterval, _ id: String, source: String) -> String {
        let t = start.addingTimeInterval(dt)
        let p = "\(day)/rollout-\(f.string(from: t))-\(id).jsonl"
        let meta = #"{"timestamp":"x","type":"session_meta","payload":{"id":"\#(id)","timestamp":"\#(iso.string(from: t))","cwd":"/w/lead","source":\#(source)}}"# + "\n"
        FileManager.default.createFile(atPath: p, contents: Data(meta.utf8))
        return p
    }
    let lead = roll(2, "lead", source: #""cli""#)
    let guardian = roll(40, "guardian", source: #"{"subagent":{"other":"guardian"}}"#)
    typealias R = SessionResolver
    check(R.codexMeta(guardian) == nil, "codex subagent: a subagent rollout has no pane meta")
    check(R.codexMeta(lead)?.cwd == "/w/lead", "codex subagent: a cli rollout still reads")
    check(R.codexCandidates(dir: root, cwd: "/w/lead", after: start) == [lead], "codex subagent: the subagent rollout is not a cwd candidate")

    let procs = [R.Proc(pid: 900_004, ppid: 1, start: start, line: "node /opt/bin/codex HC_TERMINAL_ID=t6")]
    let need = R.Need(paneId: "hc:t6", kind: "codex", cwd: "/w/lead", pids: [900_004])
    let viaCwd = await R(socketPath: "/nonexistent", codexDir: root, openFiles: { _ in [] }).resolve([need], claimed: [], procs: procs)["hc:t6"]
    check(viaCwd == lead, "codex subagent: the lead's rollout resolves by cwd although its guardian wrote one there")
    // The open-file route alone (no rollout folder to fall back on): both rollouts open, the pane is the lead's.
    let open = [lead, guardian]
    let viaOpen = await R(socketPath: "/nonexistent", codexDir: home + "/no-rollout-folder", home: home,
                          openFiles: { $0.contains(900_004) ? open : [] }).resolve([need], claimed: [], procs: procs)["hc:t6"]
    check(viaOpen == lead, "codex subagent: the lead's open rollout, not its guardian's (\(viaOpen ?? "none"))")
}
