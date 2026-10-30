import Foundation
import HerdrKit

// Claude Code ends a log with a `continued-in` line when the conversation goes on in a new session's log. herdr keeps
// the old session id: locating, the teammate verdict and a running tail follow the chain to the newest log.
// Synthetic logs only.
@MainActor func checksContinuedIn() async throws {
    let fm = FileManager.default
    let root = NSTemporaryDirectory() + "hkcont-\(UUID().uuidString)"
    let projects = root + "/projects", dir = projects + "/-w-proj"
    try? fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
    let mate = #"{"type":"user","uuid":"m1","sessionId":"s","teamName":"team-1","agentName":"helper","message":{"role":"user","content":"teammate start"}}"#
    let lead = #"{"type":"user","uuid":"l1","sessionId":"s","message":{"role":"user","content":"lead start"}}"#
    func next(_ from: String, _ to: String) -> String {
        #"{"type":"continued-in","timestamp":"2026-10-30T06:54:55.989Z","sessionId":"\#(from)","continuedInSessionId":"\#(to)"}"#
    }
    func log(_ id: String, _ ls: [String]) -> String {
        let p = dir + "/" + id + ".jsonl"
        fm.createFile(atPath: p, contents: Data((ls.joined(separator: "\n") + "\n").utf8))
        return p
    }
    _ = log("old1", [mate, next("old1", "mid1")])
    _ = log("mid1", [lead, next("mid1", "new1")])
    let new1 = log("new1", [lead])
    check(SessionLog.locate(sessionId: "old1", projectsDir: projects)?.path == new1, "continued-in: locate follows the chain to the newest log")
    check(SessionLog.locate(agent: "claude", sessionId: "old1", dir: projects)?.path == new1, "continued-in: locate by agent follows it too")
    let gone = log("gone1", [lead, next("gone1", "missing1")])
    check(SessionLog.locate(sessionId: "gone1", projectsDir: projects)?.path == gone, "continued-in: a missing next log keeps the old one")
    _ = log("loopa", [lead, next("loopa", "loopb")]); _ = log("loopb", [lead, next("loopb", "loopa")])
    check(SessionLog.locate(sessionId: "loopa", projectsDir: projects) != nil, "continued-in: a loop ends")
    let later = log("later1", [lead, next("later1", "new1"), lead])
    check(SessionLog.locate(sessionId: "later1", projectsDir: projects)?.path == later, "continued-in: only a last line counts")

    typealias R = SessionResolver
    let r = R(socketPath: "/nonexistent.sock", projectsDir: projects)
    let pass = await r.leads([R.LeadAsk(paneId: "w1:p1", sessionId: "old1")], claimed: ["old1"])
    check(pass.swaps.isEmpty && pass.leads == ["old1"], "continued-in: a teammate's old log continued as a lead's is shown")

    // A running tail moves to the new log when the continued-in line arrives.
    final class Box: @unchecked Sendable { let l = NSLock(); var v: [ConversationItem] = []
        func set(_ x: [ConversationItem]) { l.withLock { v = x } }
        var users: [String] { l.withLock { v.compactMap { if case .userText(let t) = $0.kind { t } else { nil } } } } }
    func waitFor(_ box: Box, _ sec: Double, _ want: [String]) async -> Bool {
        let t = Date()
        while Date().timeIntervalSince(t) < sec { if box.users == want { return true }; try? await Task.sleep(nanoseconds: 10_000_000) }
        return box.users == want
    }
    let liveOld = log("live1", [lead])
    _ = log("live2", [#"{"type":"user","uuid":"n1","sessionId":"s","message":{"role":"user","content":"in the new log"}}"#])
    let box = Box()
    let stream = SessionLog(sessionId: "live1").items(projectsDir: projects, pollInterval: .milliseconds(50))
    let reader = Task { for await x in stream { box.set(x) } }
    check(await waitFor(box, 3, ["lead start"]), "continued-in: tail reads the old log first \(box.users)")
    let h = FileHandle(forWritingAtPath: liveOld)!
    h.seekToEndOfFile(); h.write(Data((next("live1", "live2") + "\n").utf8)); try? h.close()
    check(await waitFor(box, 3, ["in the new log"]), "continued-in: a running tail moves to the new log \(box.users)")
    reader.cancel()
    try? fm.removeItem(atPath: root)
}
