import Foundation
import UniformTypeIdentifiers
@testable import HerdrKit

// Remote herdr (Log): agent logs on a host, read through a local mirror (`RemoteLogMirror`) that holds every byte at
// the host's offset. `Via.local` plays the host with /bin/sh on this Mac; the "host" files are temp files.
@MainActor func checksRemoteLog() async throws {
    let host = RemoteHost(via: .local)
    let fm = FileManager.default
    let root = NSTemporaryDirectory() + "hkrl-\(UUID().uuidString)"
    let cache = URL(fileURLWithPath: root + "/cache", isDirectory: true)
    let savedDir = RemoteLogMirror.directory
    RemoteLogMirror.directory = cache
    defer { RemoteLogMirror.directory = savedDir; try? fm.removeItem(atPath: root) }
    func put(_ path: String, _ s: String) throws {
        try fm.createDirectory(atPath: (path as NSString).deletingLastPathComponent, withIntermediateDirectories: true)
        try s.write(toFile: path, atomically: false, encoding: .utf8)
    }
    func bytes(_ url: URL) -> Data { (try? Data(contentsOf: url)) ?? Data() }

    // Locating a log on the host: the same rules as on this Mac.
    do {
        let claude = root + "/claude", codex = root + "/codex", cursor = root + "/cursor"
        try put(claude + "/-proj-a/s-claude.jsonl", "{}\n")
        try put(claude + "/-proj-b/other.jsonl", "{}\n")
        try put(codex + "/2026/10/21/rollout-2026-10-21T01-00-00-c1.jsonl", "{}\n")
        try put(codex + "/2026/10/22/rollout-2026-10-22T01-00-00-c1.jsonl", "{}\n")
        try put(codex + "/2026/10/22/notarollout-c2.jsonl", "{}\n")
        try put(cursor + "/proj/agent-transcripts/k1/k1.jsonl", "{}\n")
        try put(root + "/pi/run.jsonl", "{}\n")
        try put(root + "/pi/run.txt", "x\n")
        func at(_ agent: String, _ id: String, _ dir: String?) async -> String? {
            await RemoteLogMirror.locate(agent: agent, sessionId: id, host: host, dir: dir)
        }
        check(await at("claude", "s-claude", claude) == claude + "/-proj-a/s-claude.jsonl", "remote log: Claude log found on the host")
        check(await at("claude", "nope", claude) == nil, "remote log: unknown Claude id -> nil")
        let trick1 = await at("claude", "-proj-a/s-claude", claude), trick2 = await at("claude", "..", claude)
        let trick3 = await at("claude", "", claude)
        check(trick1 == nil && trick2 == nil && trick3 == nil, "remote log: ids with path tricks -> nil")
        check(await at("claude", "*", claude) == nil, "remote log: a glob in the id matches nothing")
        check(await at("codex", "c1", codex) == codex + "/2026/10/22/rollout-2026-10-22T01-00-00-c1.jsonl", "remote log: Codex rollout found, newest day")
        check(await at("codex", "c2", codex) == nil, "remote log: Codex file without the rollout- prefix -> nil")
        check(await at("cursor", "k1", cursor) == cursor + "/proj/agent-transcripts/k1/k1.jsonl", "remote log: Cursor transcript found")
        check(await at("pi", root + "/pi/run.jsonl", nil) == root + "/pi/run.jsonl", "remote log: absolute .jsonl path used as is")
        check(await at("pi", root + "/pi/run.txt", nil) == nil, "remote log: absolute path not ending in .jsonl -> nil")
        check(await at("pi", root + "/pi/../pi/run.jsonl", nil) == nil, "remote log: absolute path with /../ -> nil")
        check(await at("pi", root + "/pi/gone.jsonl", nil) == nil, "remote log: missing absolute path -> nil")
        // Grok Build: sessions/<cwd folder>/<id>/updates.jsonl (not events.jsonl or chat_history.jsonl).
        let grok = root + "/grok"
        try put(grok + "/%2Fproj/g1/events.jsonl", "{}\n")
        try put(grok + "/%2Fproj/g1/updates.jsonl", "{}\n")
        try put(grok + "/%2Fproj/g2/chat_history.jsonl", "{}\n")
        check(await at("grok", "g1", grok) == grok + "/%2Fproj/g1/updates.jsonl", "remote log: Grok updates.jsonl found on the host")
        check(await at("grok", "g2", grok) == nil, "remote log: Grok session without updates.jsonl -> nil")
        check(RemoteLogMirror.locateCommand(agent: "grok", sessionId: "g1", dir: nil)?.contains("\"${GROK_HOME:-$HOME/.grok}\"/sessions") == true,
              "remote log: Grok without a folder: the host's GROK_HOME, else ~/.grok")
        check(RemoteLogMirror.locateCommand(agent: "claude", sessionId: "s1", dir: nil)?.contains("\"$HOME\"/.claude/projects") == true,
              "remote log: without a folder the host's own home is used")
    }

    // The mirror: the tail first at the host's offsets (holes before it), then growth, earlier ranges on demand,
    // and a reset when the host file shrinks.
    do {
        let path = root + "/m/log.jsonl"
        var body = ""
        for i in 0..<2000 { body += #"{"type":"user","uuid":"m\#(i)","message":{"content":"line \#(i) ............................................"}}"# + "\n" }
        try put(path, body)
        let full = Data(body.utf8)
        let m = try RemoteLogMirror(host: host, path: path)
        check(m.url.path.hasPrefix(cache.path + "/") && m.url.pathExtension == "jsonl", "remote log: mirror file in the cache folder")
        let mode = (try? fm.attributesOfItem(atPath: m.url.path))?[.posixPermissions] as? Int
        let dmode = (try? fm.attributesOfItem(atPath: cache.path))?[.posixPermissions] as? Int
        check(mode == 0o600 && dmode == 0o700, "remote log: mirror file 600, folder 700")
        let r1 = await m.sync(tail: 50_000)
        var local = bytes(m.url)
        let lo = full.count - 50_000
        check(r1 == .grew && local.count == full.count, "remote log: mirror has the host's size (\(local.count) vs \(full.count))")
        check(local.suffix(50_000) == full.suffix(50_000), "remote log: tail bytes identical at identical offsets")
        check(local.prefix(lo).allSatisfy { $0 == 0 }, "remote log: bytes before the tail are holes (zeros)")
        check(await m.sync(tail: 50_000) == .unchanged, "remote log: no growth -> unchanged")
        let more = #"{"type":"assistant","uuid":"grow","message":{"content":[{"type":"text","text":"grown"}]}}"# + "\n"
        let fh = try FileHandle(forWritingTo: URL(fileURLWithPath: path)); try fh.seekToEnd(); try fh.write(contentsOf: Data(more.utf8)); try fh.close()
        let r2 = await m.sync(tail: 50_000)
        local = bytes(m.url)
        check(r2 == .grew && local.count == full.count + more.utf8.count && local.suffix(more.utf8.count) == Data(more.utf8), "remote log: growth appended")
        check(await m.ensure(from: 1000, to: UInt64(lo)), "remote log: ensure fetches an earlier range")
        local = bytes(m.url)
        check(local[1000..<lo] == full[1000..<lo] && local.prefix(1000).allSatisfy { $0 == 0 }, "remote log: earlier range at its offset, nothing more")
        try put(path, "{\"type\":\"user\",\"uuid\":\"new\",\"message\":{\"content\":\"new file\"}}\n")
        let r3 = await m.sync(tail: 50_000)
        check(r3 == .reset && bytes(m.url) == bytes(URL(fileURLWithPath: path)), "remote log: shrunk host file resets the mirror")
        // Replaced by another file that is not smaller (rotation, a rename over it): reset too, not a spliced tail.
        try put(path, String(repeating: "{\"type\":\"user\",\"uuid\":\"rot\",\"message\":{\"content\":\"rotated\"}}\n", count: 5))
        let r4 = await m.sync(tail: 50_000)
        check(r4 == .reset && bytes(m.url) == bytes(URL(fileURLWithPath: path)), "remote log: a larger replacement file resets the mirror (\(r4))")
        try? fm.removeItem(atPath: path)
        check(await m.sync(tail: 50_000) == .gone, "remote log: gone host file -> gone (not unreachable)")
        check(bytes(m.url).count > 0, "remote log: a gone host file keeps the mirror as it was")
        m.remove()
        check(!fm.fileExists(atPath: m.url.path), "remote log: remove deletes the mirror file")
    }

    // SessionLog on a host: same items as a local SessionLog over the same file; loadEarlier fetches from the host.
    do {
        let dir = root + "/sl"
        var ls: [String] = []
        for i in 0..<1500 {
            ls.append(#"{"type":"user","uuid":"u\#(i)","message":{"content":"question \#(i) ......................................"}}"#)
            ls.append(#"{"type":"assistant","uuid":"a\#(i)","message":{"content":[{"type":"tool_use","id":"t\#(i)","name":"Bash","input":{"command":"echo \#(i)"}}]}}"#)
            ls.append(#"{"type":"user","uuid":"r\#(i)","message":{"content":[{"type":"tool_result","tool_use_id":"t\#(i)","content":"out\#(i)"}]}}"#)
        }
        let path = dir + "/p/rl1.jsonl"
        try put(path, ls.joined(separator: "\n") + "\n")
        let size = (try? fm.attributesOfItem(atPath: path))?[.size] as? Int ?? 0
        check(size > Int(RemoteLogMirror.slack) * 4, "remote log: fixture larger than the mirror's first fetch (\(size))")
        let remote = SessionLog(sessionId: "rl1", agent: "claude", remote: host)
        let local = SessionLog(sessionId: "rl1", agent: "claude")
        let rf = LogFeed(remote.snapshots(projectsDir: dir, pollInterval: .milliseconds(20), initialTailBytes: 3000))
        let lf = LogFeed(local.snapshots(projectsDir: dir, pollInterval: .milliseconds(20), initialTailBytes: 3000))
        defer { rf.stop(); lf.stop() }
        let r0 = await rf.wait { !$0.isEmpty }
        let l0 = await lf.wait { !$0.isEmpty }
        check(!r0.isEmpty && r0 == l0, "remote log: first view matches the local one (\(r0.count) vs \(l0.count))")
        let murl = remote.url
        check(murl != nil && murl?.path.hasPrefix(cache.path + "/") == true, "remote log: url is the local mirror")
        if let murl { check(bytes(murl).prefix(1000).allSatisfy { $0 == 0 }, "remote log: only the tail fetched at first") }
        // A failed host range fetch must not claim that an older page was loaded.
        // Make only this synthetic host file unreadable; the already mirrored tail stays readable. Not a move: a host
        // file that is gone for 5 polls drops the mirror, and on a slow runner this window can be longer than that.
        try fm.setAttributes([.posixPermissions: 0o000], ofItemAtPath: path)
        let unavailable = await remote.loadEarlier(bytes: 100_000)
        check(unavailable == .failed, "remote log: failed earlier fetch is not reported as more content")
        check(rf.items.get() == r0, "remote log: failed earlier fetch preserves the visible tail")
        try fm.setAttributes([.posixPermissions: 0o644], ofItemAtPath: path)
        var r = r0, l = l0
        for (log, feed) in [(remote, rf), (local, lf)] {
            var more = true, n = 0
            while more && n < 100 {
                let c = feed.items.get().count
                more = await log.loadEarlier(bytes: 40_000) == .more
                // the last call (.end) also loads a page: wait for it too, long enough for a slow runner
                _ = await feed.wait(timeout: 10) { $0.count > c }
                n += 1
            }
        }
        r = rf.items.get(); l = lf.items.get()
        check(r.count == 3000 && r == l, "remote log: loadEarlier to the start, items match local (\(r.count) vs \(l.count))")
        if let murl { check(bytes(murl) == bytes(URL(fileURLWithPath: path)), "remote log: mirror equals the host file after loading all") }

        // Growth on the host shows up.
        let add = #"{"type":"assistant","uuid":"late","message":{"content":[{"type":"text","text":"late line"}]}}"# + "\n"
        let fh = try FileHandle(forWritingTo: URL(fileURLWithPath: path)); try fh.seekToEnd(); try fh.write(contentsOf: Data(add.utf8)); try fh.close()
        r = await rf.wait { $0.contains { $0.id.hasPrefix("late-") } }
        check(r.contains { $0.id.hasPrefix("late-") }, "remote log: growth on the host appears")

        // Truncation on the host starts over.
        try put(path, #"{"type":"user","uuid":"fresh","message":{"content":"fresh start"}}"# + "\n")
        r = await rf.wait { $0.count == 1 && $0.first?.id.hasPrefix("fresh-") == true }
        check(r.count == 1 && r.first?.id.hasPrefix("fresh-") == true, "remote log: truncated host log resets the items (\(r.count))")
    }

    // A picture in a mirrored log reads back through LogImage.read.
    do {
        let dir = root + "/pic"
        try fm.createDirectory(atPath: dir + "/p", withIntermediateDirectories: true)
        let shot = testImage(.png, w: 40, h: 20)!
        _ = try pictureLog(URL(fileURLWithPath: dir + "/p/pic1.jsonl"), user: [shot], tool: [], tail: 3)
        let log = SessionLog(sessionId: "pic1", agent: "claude", remote: host)
        let feed = LogFeed(log.snapshots(projectsDir: dir, pollInterval: .milliseconds(20)))
        let items = await feed.wait { $0.contains { !$0.images.isEmpty } }
        let ref = items.first { !$0.images.isEmpty }?.images.first
        let url = log.url
        check(ref != nil && url != nil && url.flatMap { u in ref.map { LogImage.read($0, from: u) } } == shot, "remote log: picture read back from the mirror")
        feed.stop()
        if let url {
            var gone = false
            for _ in 0..<100 { if !fm.fileExists(atPath: url.path) { gone = true; break }; try? await Task.sleep(for: .milliseconds(20)) }
            check(gone, "remote log: mirror file removed when the log is no longer watched")
        }
    }

    // Old mirror files go on the next start; the cache stays under its cap.
    do {
        let d = URL(fileURLWithPath: root + "/sweep", isDirectory: true)
        try fm.createDirectory(at: d, withIntermediateDirectories: true)
        let old = d.appendingPathComponent("old.jsonl"), fresh = d.appendingPathComponent("fresh.jsonl"), mid = d.appendingPathComponent("mid.jsonl")
        try Data(repeating: 1, count: 10_000).write(to: old)
        try Data(repeating: 2, count: 10_000).write(to: mid)
        try Data(repeating: 3, count: 10_000).write(to: fresh)
        try fm.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -2 * 86400)], ofItemAtPath: old.path)
        try fm.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -3600)], ofItemAtPath: mid.path)
        // A mirror another app run has open holds a shared lock: it stays, however old.
        let held = d.appendingPathComponent("held.jsonl")
        try Data(repeating: 4, count: 10).write(to: held)
        try fm.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -2 * 86400)], ofItemAtPath: held.path)
        let hfd = open(held.path, O_RDONLY)
        flock(hfd, LOCK_SH)
        RemoteLogMirror.sweep(dir: d, maxTotal: 1 << 30)
        check(fm.fileExists(atPath: held.path), "remote log: a mirror another run has open is not removed")
        close(hfd)
        check(!fm.fileExists(atPath: old.path) && fm.fileExists(atPath: mid.path) && fm.fileExists(atPath: fresh.path), "remote log: mirror older than a day removed")
        RemoteLogMirror.sweep(dir: d, maxTotal: 15_000)
        check(!fm.fileExists(atPath: mid.path) && fm.fileExists(atPath: fresh.path), "remote log: over the cap the oldest mirror goes first")
    }

    // The ps/lsof fallback cannot see host processes: off for a remote session; herdr's session is shown as named.
    do {
        let res = SessionResolver(socketPath: RemoteHost.runtimeDir + "/api-x.sock", remote: host)
        let found = await res.resolve([SessionResolver.Need(paneId: "w1:p1", kind: "codex", cwd: "/")], claimed: [])
        let pass = await res.leads([SessionResolver.LeadAsk(paneId: "w1:p2", sessionId: "s-lead")], claimed: [])
        check(found.isEmpty && pass.swaps.isEmpty && pass.leads == ["s-lead"], "remote log: resolver off remotely, herdr's session kept")
    }
}

/// Consumes a log's snapshots in a task; the latest items are kept for polling.
private final class LogFeed: Sendable {
    let items: Locked<[ConversationItem]>
    private let task: Task<Void, Never>
    init(_ stream: AsyncStream<SessionLog.Snapshot>) {
        let box = Locked<[ConversationItem]>([])
        items = box
        task = Task { for await s in stream { box.set(s.items) } }
    }
    /// The items once `ok` holds, or the last ones after `timeout` seconds.
    func wait(timeout: Double = 5, _ ok: @Sendable ([ConversationItem]) -> Bool) async -> [ConversationItem] {
        let end = Date(timeIntervalSinceNow: timeout)
        while Date() < end {
            let cur = items.get()
            if ok(cur) { return cur }
            try? await Task.sleep(for: .milliseconds(10))
        }
        return items.get()
    }
    func stop() { task.cancel() }
}
