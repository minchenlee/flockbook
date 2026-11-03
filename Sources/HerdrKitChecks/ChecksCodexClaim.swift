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
