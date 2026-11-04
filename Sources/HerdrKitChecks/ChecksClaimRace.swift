import Foundation
import ModFeedKit

// Two sidecars that press Allow at the same moment: only one claim wins, and a claim file that another run is still
// writing is never removed (review of PR #1, finding 3). Rust: xplat/flockbook-modfeed/tests/claim.rs.
func checksClaimRace() throws {
    let fm = FileManager.default
    let t = "t1", id = "box-1"
    func setup() throws -> (dir: URL, cache: URL, folder: URL) {
        let dir = fm.temporaryDirectory.appendingPathComponent("fb-claim-\(UUID().uuidString)")
        let folder = dir.appendingPathComponent("locate/answered/\(fnv1a(t))", isDirectory: true)
        try fm.createDirectory(at: folder, withIntermediateDirectories: true)
        return (dir, dir.appendingPathComponent("locate/cache.json"), folder)
    }
    func names(_ folder: URL) -> [String] { ((try? fm.contentsOfDirectory(atPath: folder.path)) ?? []).sorted() }
    func age(_ url: URL, _ by: TimeInterval) { try? fm.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -by)], ofItemAtPath: url.path) }

    // Many runs at once: one claim wins, and no lock or temp file stays.
    do {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        let n = 16
        let wins = UnsafeMutablePointer<Int>.allocate(capacity: n)
        defer { wins.deallocate() }
        let cache = s.cache
        DispatchQueue.concurrentPerform(iterations: n) { i in
            wins[i] = FeedAnswered(cache: cache).claim(terminal: t, id: id, tool: "Bash", command: "rm -rf build", blocked: true) ? 1 : 0
        }
        let total = (0..<n).reduce(0) { $0 + wins[$1] }
        check(total == 1, "claim race: one claim wins of \(n) at the same moment (\(total))")
        check(names(s.folder) == [fnv1a(id)], "claim race: only the claim stays (\(names(s.folder)))")
    }

    // A claim mid-write (an older sidecar made it, not written yet) is not removed; an old unreadable file goes.
    do {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        let url = s.folder.appendingPathComponent(fnv1a(id))
        fm.createFile(atPath: url.path, contents: Data())
        let a = FeedAnswered(cache: s.cache)
        check(!a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true), "claim race: a fresh empty claim counts as taken")
        check(fm.fileExists(atPath: url.path), "claim race: and it stays")
        a.seen(terminal: t, box: nil)
        check(fm.fileExists(atPath: url.path), "claim race: seen() leaves a fresh empty claim too")
        age(url, 3)
        check(a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true), "claim race: an old unreadable file is no claim")
        let body = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
        check(body.contains("\"id\":\"box-1\""), "claim race: the new claim has its mark (\(body))")
    }

    // A lock of a live run blocks; an old lock (a run that died mid-claim) goes.
    do {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        let lock = s.folder.appendingPathComponent(".lock")
        fm.createFile(atPath: lock.path, contents: Data())
        let a = FeedAnswered(cache: s.cache)
        check(!a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true), "claim race: another run is claiming: no second claim")
        age(lock, 30)
        check(a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true), "claim race: a lock left by a dead run goes")
        check(names(s.folder) == [fnv1a(id)], "claim race: no lock stays (\(names(s.folder)))")
    }
}
