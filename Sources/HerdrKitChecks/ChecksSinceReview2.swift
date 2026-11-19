import Foundation
@testable import HerdrKit

// The second review of the Since you left card and Summaries (each failed before its fix).

private let turn = SessionLog.parse(lines: [
    #"{"type":"user","uuid":"u1","timestamp":"2026-11-03T01:40:00.000Z","message":{"role":"user","content":"Fix the parser"}}"#,
    #"{"type":"assistant","uuid":"a3","timestamp":"2026-11-03T01:40:05.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Fixed it. Tests pass."}]}}"#,
])

private func scratch<T>(_ body: (URL) async throws -> T) async rethrows -> T {
    let root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hksincerev2-\(UUID().uuidString)")
    try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    return try await body(root)
}

private final class Flag: @unchecked Sendable {
    private let lock = NSLock()
    private var v: [Int] = []
    func add(_ n: Int) { lock.withLock { v.append(n) } }
    var all: [Int] { lock.withLock { v } }
}

/// A run through herdr-core asks once more, with the connection open and right before the request goes, whether it
/// may still run (Summaries on, the same writer, the agent stopped): no, and nothing is written.
func checksHCExecGate() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.execReplies = [("claude", "STATE: Done.\n")]
    let run = HCExecRunner(client: HCClient(session: HCSession(socketPath: fake.path)))
    let seen = Flag()
    let no = await run.run(argv: ["claude", "-p"], input: "T", cwd: NSTemporaryDirectory(), timeout: 10, go: {
        // The fake takes the connection a moment after connect returns: wait for it (none comes if not connected).
        for _ in 0..<40 where fake.openCount == 0 { try? await Task.sleep(for: .milliseconds(25)) }
        seen.add(fake.openCount); return false
    })
    let execs = fake.calls.filter { $0.method == "machine.exec" }.count
    check(!no.sent && execs == 0, "hc exec gate: no: nothing goes to herdr-core (sent \(no.sent), \(execs) runs)")
    check(seen.all.count == 1 && (seen.all.first ?? 0) >= 1, "hc exec gate: asked once, with the connection open (\(seen.all))")
    let yes = await run.run(argv: ["claude", "-p"], input: "T", cwd: NSTemporaryDirectory(), timeout: 10, go: { true })
    check(yes.sent && yes.code == 0 && fake.calls.filter { $0.method == "machine.exec" }.count == 1, "hc exec gate: yes: the run goes (\(yes))")
}

/// A run through herdr-core that is called off (the app quits, Summaries goes off) closes its connection: herdr-core
/// ends the program when the caller goes away (`machine.exec`).
func checksHCExecCancel() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.execHold = 8
    let run = HCExecRunner(client: HCClient(session: HCSession(socketPath: fake.path)))
    let t = Task { await run.run(argv: ["claude", "-p"], input: "T", cwd: NSTemporaryDirectory(), timeout: 10) }
    for _ in 0..<200 where !fake.calls.contains(where: { $0.method == "machine.exec" }) { try? await Task.sleep(for: .milliseconds(25)) }
    let began = Date()
    t.cancel()
    let r = await t.value
    let took = Date().timeIntervalSince(began)
    for _ in 0..<40 where fake.execGone == 0 { try? await Task.sleep(for: .milliseconds(25)) }
    check(took < 3 && r.code == nil, "hc exec cancel: the run ends at once (\(String(format: "%.1f", took)) s)")
    check(fake.execGone == 1, "hc exec cancel: the connection closes, so herdr-core ends the program (\(fake.execGone))")
}

/// herdr-core cannot be reached for a moment (not started yet, restarting): nothing ran, so the turn is not marked
/// failed and is asked again once herdr-core is back.
func checksSummaryNotSent() async throws {
    try await scratch { dir in
        let fake = FakeHC(path: fakeHCPath())
        let store = await MainActor.run { SinceStore(dir: dir.appendingPathComponent("app")) }
        let run = HCExecRunner(client: HCClient(session: HCSession(socketPath: fake.path)))
        let s = Summary.Settings(on: true)
        let down = await Summary.ask(items: turn, kind: "claude", settings: s, runner: run, store: store, cwd: dir.path)
        var notSent = false
        if case .failure(.notSent) = down { notSent = true }
        let u = await MainActor.run { store.usage(since: [0], now: Int64(Date().timeIntervalSince1970 * 1000) + 1) }
        check(notSent, "summary not sent: herdr-core down: the run did not go (\(down))")
        check(Summary.keep(down, id: "a3", by: s.writer) == nil, "summary not sent: nothing is kept, so the turn is asked again")
        check((u.first?.runs ?? 0) == 0, "summary not sent: no model run is counted (\(u))")
        try fake.start()
        defer { fake.kill() }
        fake.execReplies = [("claude", "STATE: Done.\n")]
        let up = await Summary.ask(items: turn, kind: "claude", settings: s, runner: run, store: store, cwd: dir.path)
        var ok = false
        if case .success(let a) = up { ok = a.state == "Done." }
        check(ok, "summary not sent: herdr-core back: the same turn is asked (\(up))")
        if case .success = up { check(Summary.keep(up, id: "a3", by: s.writer)?.state == "Done.", "summary keep: an answer is kept") }
        let failed = Summary.keep(.failure(.failed("x")), id: "a3", by: s.writer)
        check(failed?.failed == true && failed?.id == "a3", "summary keep: a run that failed is marked, not asked again")
    }
}
