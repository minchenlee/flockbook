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
            wins[i] = claimed(FeedAnswered(cache: cache).claim(terminal: t, id: id, tool: "Bash", command: "rm -rf build", blocked: true)) ? 1 : 0
        }
        let total = (0..<n).reduce(0) { $0 + wins[$1] }
        check(total == 1, "claim race: one claim wins of \(n) at the same moment (\(total))")
        check(names(s.folder) == [".lock", fnv1a(id)], "claim race: only the claim and the lock file stay (\(names(s.folder)))")
    }

    // A claim mid-write (an older sidecar made it, not written yet) is not removed; an old unreadable file goes.
    do {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        let url = s.folder.appendingPathComponent(fnv1a(id))
        fm.createFile(atPath: url.path, contents: Data())
        let a = FeedAnswered(cache: s.cache)
        check(a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true) == .taken, "claim race: a fresh empty claim counts as taken")
        check(fm.fileExists(atPath: url.path), "claim race: and it stays")
        a.seen(terminal: t, box: nil)
        check(fm.fileExists(atPath: url.path), "claim race: seen() leaves a fresh empty claim too")
        age(url, 3)
        check(claimed(a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true)), "claim race: an old unreadable file is no claim")
        let body = (try? String(contentsOf: url, encoding: .utf8)) ?? ""
        check(body.contains("\"id\":\"box-1\""), "claim race: the new claim has its mark (\(body))")
    }

    // A lock held by a live run gives busy (not answered); once that run ends, the claim goes ahead.
    do {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        let fd = open(s.folder.appendingPathComponent(".lock").path, O_RDWR | O_CREAT, 0o600)
        check(fd >= 0 && flock(fd, LOCK_EX) == 0, "claim race: setup, another run holds the lock")
        let a = FeedAnswered(cache: s.cache)
        check(a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true) == .busy, "claim race: busy, nothing is decided")
        check(!fm.fileExists(atPath: s.folder.appendingPathComponent(fnv1a(id)).path), "claim race: no claim while busy")
        close(fd)
        check(claimed(a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true)), "claim race: that run ended: the claim goes")
        check(a.claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true) == .taken, "claim race: and now it is answered")
    }

    // Re-review 1 and 2: a lock left by a dead run never blocks, whatever its time; breaking it is atomic.
    func future(_ url: URL, _ by: TimeInterval) { try? fm.setAttributes([.modificationDate: Date(timeIntervalSinceNow: by)], ofItemAtPath: url.path) }
    do {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        fm.createFile(atPath: s.folder.appendingPathComponent(".lock").path, contents: Data())   // its run was killed a moment ago
        check(claimed(FeedAnswered(cache: s.cache).claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true)), "claim race: a lock nobody holds does not block")
    }
    do {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        let lock = s.folder.appendingPathComponent(".lock")
        fm.createFile(atPath: lock.path, contents: Data())
        future(lock, 3600)   // the clock was set back
        check(claimed(FeedAnswered(cache: s.cache).claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true)), "claim race: a lock time in the future blocks nothing")
    }
    do {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        let url = s.folder.appendingPathComponent(fnv1a(id))
        fm.createFile(atPath: url.path, contents: Data())
        future(url, 3600)
        check(claimed(FeedAnswered(cache: s.cache).claim(terminal: t, id: id, tool: "Bash", command: "ls", blocked: true)), "claim race: an unreadable claim from the future is no claim")
    }
    for _ in 0..<20 {
        let s = try setup()
        defer { try? fm.removeItem(at: s.dir) }
        let lock = s.folder.appendingPathComponent(".lock")
        fm.createFile(atPath: lock.path, contents: Data())
        age(lock, 30)
        let n = 16
        let wins = UnsafeMutablePointer<Int>.allocate(capacity: n)
        defer { wins.deallocate() }
        let cache = s.cache
        DispatchQueue.concurrentPerform(iterations: n) { i in
            wins[i] = claimed(FeedAnswered(cache: cache).claim(terminal: t, id: id, tool: "Bash", command: "rm -rf build", blocked: true)) ? 1 : 0
        }
        let total = (0..<n).reduce(0) { $0 + wins[$1] }
        check(total == 1, "claim race: one claim wins of \(n), after a dead run's lock (\(total))")
        if total != 1 { break }
    }
}

private func claimed(_ c: FeedAnswered.Claim) -> Bool { c == .got }
