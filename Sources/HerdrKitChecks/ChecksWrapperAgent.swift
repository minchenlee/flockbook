import Foundation
@testable import HerdrKit

// Twins of herdr-core's wrapper-agent tests (issue #15): an agent started through a wrapper program
// (`secretspec run -- claude`) gets its kind from 7501 `status.app`, and a key identity `<fg_pid>/<agent_pid>@<created>`
// only when the foreground's process tree really runs that agent. Test numbers follow the design's list.

private typealias R = SessionResolver

/// Counts the asks; it never answers. Twin of the Rust `PanicCheck`: a path that must not ask asserts a count of 0.
private final class CountingCheck: HCAgentCheck, @unchecked Sendable {
    private let lock = NSLock()
    private var n = 0
    var asks: Int { lock.withLock { n } }
    func agentUnder(_ ask: HCWrapperAsk) -> Int32? { lock.withLock { n += 1 }; return nil }
}

/// A process list the checks change; counts the reads.
private final class Procs: @unchecked Sendable {
    private let lock = NSLock()
    private var list: [R.Proc]
    private var n = 0
    init(_ list: [R.Proc]) { self.list = list }
    var reads: Int { lock.withLock { n } }
    func set(_ l: [R.Proc]) { lock.withLock { list = l } }
    func read() -> [R.Proc] { lock.withLock { n += 1; return list } }
    func has(_ pid: Int32) -> Bool { lock.withLock { list.contains { $0.pid == pid } } }
}

/// A clock the checks move (seconds).
private final class Clock: @unchecked Sendable {
    private let lock = NSLock()
    private var t: TimeInterval = 1000
    var now: TimeInterval { lock.withLock { t } }
    func set(_ s: TimeInterval) { lock.withLock { t = 1000 + s } }
    func add(_ s: TimeInterval) { lock.withLock { t += s } }
}

private let baseMs = 1_794_500_000_000

private func proc(_ pid: Int32, _ ppid: Int32, _ ms: Int, _ line: String) -> R.Proc {
    R.Proc(pid: pid, ppid: ppid, start: Date(timeIntervalSince1970: TimeInterval(ms) / 1000), line: line)
}

private func tree(_ procs: [R.Proc]) -> HCTreeCheck {
    HCTreeCheck(source: { procs }, now: { 0 }, alive: { _ in true })
}

private func term(_ addr: String = "t1", argv: [String] = ["zsh"], pid: Int? = 400, created: Int = 7,
                  fg: [String: Any]?, app: String?, state: String = "working", statusMs: Int? = baseMs + 2000) -> HCMapper.Term {
    var o: [String: Any] = ["address": addr, "argv": argv, "created_ms": created]
    if let pid { o["pid"] = pid }
    if let fg { o["foreground"] = fg }
    var st: [String: Any] = ["state": state]
    if let app { st["app"] = app }
    o["status"] = st
    if let statusMs { o["status_ms"] = statusMs }
    return HCMapper.Term(o)!
}

private var secretspec: [String: Any] { ["name": "secretspec", "argv": ["secretspec", "run", "--profile", "claude", "--", "claude"], "pid": 500] }

@MainActor func checksWrapperAgent() async throws {
    checksWrapperKind()
    checksWrapperIdentity()
    checksWrapperTreeCache()
    checksWrapperResolver()
    try await checksWrapperGate()
    await checksWrapperLogCache()
    checksRealExecutable()
}

/// Twins of herdr-core `runs_checks_the_real_executable` and `direct_identity_needs_the_real_executable` (Codex P1): a
/// process runs the agent only when its program is the agent, or an interpreter whose script is the agent.
@MainActor private func checksRealExecutable() {
    func one(_ line: String, _ kind: String) -> Int32? { R.agentInTree(root: 1, kind: kind, procs: [proc(10, 1, baseMs, line)], tieNewest: false)?.pid }
    for (line, kind) in [("claude --resume", "claude"), ("/usr/local/bin/claude HC_TERMINAL_ID=t1", "claude"),
                         ("node /opt/x/bin/codex HC_TERMINAL_ID=t1", "codex"), ("node22 /opt/x/bin/claude", "claude"),
                         ("python3.12 /opt/x/bin/pi", "pi"), ("bun /usr/local/bin/omp", "omp"), ("/usr/local/bin/cursor-agent", "cursor")] {
        check(one(line, kind) == 10, "real exec: \(line) runs \(kind)")
    }
    for (line, kind) in [("script claude", "claude"), ("vim claude", "claude"), ("cat codex", "codex"), ("less /tmp/claude", "claude"),
                         ("node --inspect /opt/x/bin/codex", "codex"), ("nodemon codex", "codex"), ("node", "claude"), ("", "claude")] {
        check(one(line, kind) == nil, "real exec: \(line) does not run \(kind)")
    }
    check(R.agentInTree(root: 1, kind: "claude", procs: [proc(10, 1, baseMs, "secretspec run -- claude"), proc(11, 10, baseMs, "claude")], tieNewest: false)?.pid == 11,
          "real exec: a wrapper's child is the agent")
    check(R.agentInTree(root: 1, kind: "claude", procs: [proc(10, 1, baseMs, "script claude"), proc(11, 10, baseMs, "-zsh")], tieNewest: false) == nil,
          "real exec: script with a shell under it: no agent")

    func fg(_ argv: [String]) -> [String: Any] { ["argv": argv, "pid": 600] }
    func direct(_ t: HCMapper.Term, _ k: String) -> String? { HCMapper.process(t, kind: k) }
    check(direct(term(fg: fg(["claude", "--resume"]), app: nil), "claude") == "600@7", "real exec: direct claude")
    check(direct(term(fg: fg(["node", "/opt/x/bin/codex"]), app: nil), "codex") == "600@7", "real exec: direct node codex")
    for argv in [["script", "claude"], ["vim", "claude"], ["node", "--inspect", "/opt/x/bin/claude"]] {
        check(direct(term(fg: fg(argv), app: nil), "claude") == nil, "real exec: \(argv) names claude, does not run it")
    }
    check(direct(term(argv: ["claude"], fg: nil, app: nil), "claude") == "400@7", "real exec: the terminal's own claude")
    check(direct(term(argv: ["script", "claude"], fg: nil, app: nil), "claude") == nil, "real exec: script claude as the terminal's program")
    let chk = tree([proc(600, 400, baseMs, "script claude"), proc(601, 600, baseMs + 1000, "claude")])
    check(HCMapper.process(term(fg: fg(["script", "claude"]), app: "claude-code"), kind: "claude", check: chk) == "600/601@7",
          "real exec: script in front of a real claude: the tree proves it")
}

/// Tests 1 and 2: the kind from `status.app`, after the processes.
@MainActor private func checksWrapperKind() {
    check(HCMapper.kind(term(fg: secretspec, app: "claude-code")) == "claude", "wrapper 1: claude-code -> claude with secretspec in front")
    check(HCMapper.kind(term(fg: ["name": "secretspec", "argv": ["secretspec", "run", "--", "codex"], "pid": 501], app: "codex")) == "codex",
          "wrapper 1: codex -> codex with secretspec in front")
    for app in ["pi", "claude", "zsh"] {
        check(HCMapper.kind(term(fg: ["name": "secretspec", "argv": ["secretspec", "run", "--", "x"], "pid": 502], app: app)) == nil,
              "wrapper 1: app \(app) is no kind")
    }
    check(HCMapper.kind(term(argv: ["/bin/zsh", "-l"], fg: ["name": "pi", "argv": ["pi"], "pid": 503], app: "claude-code", state: "done")) == "pi",
          "wrapper 2: the foreground wins over status.app")
}

/// Tests 3 to 13: the wrapper identity, and every way it fails closed.
@MainActor private func checksWrapperIdentity() {
    let wrap = term(fg: secretspec, app: "claude-code")
    func id(_ t: HCMapper.Term, _ c: HCAgentCheck) -> String? { HCMapper.process(t, kind: "claude", check: c) }

    // 3: wrapper + one agent child + status newer than its start.
    let one = tree([proc(500, 400, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs + 1000, "claude")])
    check(id(wrap, one) == "500/501@7", "wrapper 3: wrapper + agent child -> <fg_pid>/<agent_pid>@<created> (\(id(wrap, one) ?? "nil"))")
    // 4: no agent process under the wrapper.
    let none = tree([proc(500, 400, baseMs, "secretspec run -- claude")])
    check(id(wrap, none) == nil, "wrapper 4: wrapper, agent gone -> no identity")
    // 5: a shell in front with an old status: no identity, the check is never asked.
    for (name, argv) in [("zsh", ["zsh"]), ("-zsh", ["-zsh"]), ("bash", ["/bin/bash", "-l"]), ("wrapper.sh", ["sh", "wrapper.sh"])] {
        let c = CountingCheck()
        let t = term("t-sh", fg: ["name": name, "argv": argv, "pid": 500], app: "claude-code", state: "done")
        check(id(t, c) == nil && c.asks == 0, "wrapper 5: shell \(name) in front -> no identity, no process read (\(c.asks))")
    }
    // 6: a viewer in front with an old status, no agent below.
    for viewer in ["vim", "less"] {
        let t = term("t-view", fg: ["name": viewer, "argv": [viewer, "notes.md"], "pid": 500], app: "claude-code", state: "done")
        check(id(t, none) == nil, "wrapper 6: \(viewer) in front, no agent below -> no identity")
    }
    // 7: two agents at the same depth: none; an agent and its deeper helper of the same kind: the nearer one.
    let tie = tree([proc(500, 400, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs + 1000, "claude"),
                    proc(502, 500, baseMs + 1500, "claude")])
    check(id(wrap, tie) == nil, "wrapper 7: two agents at the same depth -> no identity")
    let helper = tree([proc(500, 400, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs + 1000, "claude"),
                       proc(502, 501, baseMs + 2000, "claude -p helper")])
    check(id(wrap, helper) == "500/501@7", "wrapper 7: the nearer agent wins over its helper (\(id(wrap, helper) ?? "nil"))")
    // 8: the agent restarted under the same wrapper: another identity.
    let restart = tree([proc(500, 400, baseMs, "secretspec run -- claude"), proc(509, 500, baseMs + 1500, "claude")])
    check(id(wrap, restart) == "500/509@7" && id(wrap, restart) != id(wrap, one), "wrapper 8: a restarted agent -> another identity")
    // 9: a direct agent (foreground or no foreground) as before, with no process read.
    let c9 = CountingCheck()
    let direct = term("t-direct", fg: ["name": "2.1.322", "argv": ["claude"], "pid": 500], app: "claude-code", statusMs: baseMs)
    check(id(direct, c9) == "500@7", "wrapper 9: direct claude in front -> <fg_pid>@<created>")
    let start = term("t-direct2", argv: ["claude"], fg: nil, app: "claude-code", statusMs: baseMs)
    check(id(start, c9) == "400@7", "wrapper 9: no foreground, the terminal runs claude -> <pid>@<created>")
    check(c9.asks == 0, "wrapper 9: a direct agent never asks the process check (\(c9.asks))")
    // 10: W6, the status changed at or after the agent started (1 s slack); no status_ms: no identity, no read.
    let late = tree([proc(500, 400, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs + 4000, "claude")])
    check(id(wrap, late) == nil, "wrapper 10: agent started > 1 s after the status -> no identity")
    let slack = tree([proc(500, 400, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs + 3000, "claude")])
    check(id(wrap, slack) == "500/501@7", "wrapper 10: 1 s lstart slack is allowed")
    let c10 = CountingCheck()
    check(id(term(fg: secretspec, app: "claude-code", statusMs: nil), c10) == nil && c10.asks == 0,
          "wrapper 10: no status_ms -> no identity, no process read")
    // 11: W2, a remote terminal: no identity, no read.
    let c11 = CountingCheck()
    check(id(term("box/t1", fg: secretspec, app: "claude-code"), c11) == nil && c11.asks == 0, "wrapper 11: remote wrapper -> no identity, no read")
    // 12: the mapper without a check: no identity.
    var o: [String: Any] = ["address": "t1", "argv": ["zsh"], "pid": 400, "created_ms": 7, "foreground": secretspec,
                            "status": ["state": "working", "app": "claude-code"], "status_ms": baseMs + 2000]
    let plain = HCMapper.workspaces(terminals: [o], appDoc: nil).first?.tabs.first?.panes.first?.agent
    check(plain?.kind == "claude" && plain?.process == nil, "wrapper 12: workspaces with no check -> kind claude, no identity")
    o["address"] = "t2"
    let checked = HCMapper.workspaces(terminals: [o], appDoc: nil, check: one).first?.tabs.first?.panes.first?.agent
    check(checked?.process == "500/501@7", "wrapper 12: workspaces passes the check to the mapper (\(checked?.process ?? "nil"))")
    // 13: an empty process list (ps failed).
    check(id(wrap, tree([])) == nil, "wrapper 13: empty process list -> no identity")
}

/// Tests 16 to 20 and the positive-cache rule: one table read per snapshot, and no entry outlives its agent.
@MainActor private func checksWrapperTreeCache() {
    let s = baseMs + 2000
    func ask(_ ms: Int, _ addr: String = "t1", _ fg: Int32 = 500) -> HCWrapperAsk {
        HCWrapperAsk(address: addr, fgPid: fg, created: 1, kind: "claude", statusMs: ms)
    }
    let src = Procs([proc(500, 1, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs, "claude")])
    let clock = Clock()
    final class Flag: @unchecked Sendable { let l = NSLock(); var v = true; func get() -> Bool { l.withLock { v } }; func set(_ x: Bool) { l.withLock { v = x } } }
    let up = Flag()
    let c = HCTreeCheck(source: { src.read() }, now: { clock.now }, alive: { _ in up.get() })

    // 16: two asks inside 10 s, agent alive: one read.
    check(c.agentUnder(ask(s)) == 501 && src.reads == 1, "wrapper 16: first ask reads the table")
    clock.set(5)
    check(c.agentUnder(ask(s)) == 501 && src.reads == 1, "wrapper 16: inside 10 s and alive -> cached (\(src.reads))")
    // 18: a Yes entry older than 10 s: read again.
    clock.set(16)
    check(c.agentUnder(ask(s)) == 501 && src.reads == 2, "wrapper 18: older than 10 s -> reads again (\(src.reads))")
    // 17: the agent is gone: read again at once, no identity.
    up.set(false)
    src.set([proc(500, 1, baseMs, "secretspec run -- claude")])
    check(c.agentUnder(ask(s)) == nil && src.reads == 3, "wrapper 17: agent not alive -> reads again, none (\(src.reads))")
    // 19: a No entry holds 30 s for the same status_ms; a new status_ms reads at once.
    clock.add(10)
    check(c.agentUnder(ask(s)) == nil && src.reads == 3, "wrapper 19: No entry reused inside 30 s (\(src.reads))")
    src.set([proc(500, 1, baseMs, "secretspec run -- claude"), proc(502, 500, baseMs, "claude")])
    check(c.agentUnder(ask(s + 5000)) == 502 && src.reads == 4, "wrapper 19: new status_ms -> reads at once (\(src.reads))")

    // Positive cache: a second agent at the same depth and a new status inside 10 s: read again, fail closed.
    let src2 = Procs([proc(500, 1, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs, "claude")])
    let clock2 = Clock()
    let c2 = HCTreeCheck(source: { src2.read() }, now: { clock2.now }, alive: { _ in true })
    check(c2.agentUnder(ask(s)) == 501, "wrapper cache: first ask")
    clock2.set(2)
    src2.set([proc(500, 1, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs, "claude"), proc(502, 500, baseMs, "claude")])
    check(c2.agentUnder(ask(s + 500)) == nil && src2.reads == 2, "wrapper cache: a new status_ms re-checks the tree, a tie -> none")

    // 20: three wrapper terminals in one snapshot: one table read. Dropped addresses are pruned.
    let batch = Procs([proc(100, 1, baseMs, "secretspec run -- claude"), proc(101, 100, baseMs, "claude"),
                       proc(200, 1, baseMs, "secretspec run -- claude"), proc(201, 200, baseMs, "claude"),
                       proc(300, 1, baseMs, "secretspec run -- claude"), proc(301, 300, baseMs, "claude")])
    let bc = HCTreeCheck(source: { batch.read() }, now: { 0 }, alive: { _ in true })
    func t(_ addr: String, _ fg: Int) -> [String: Any] {
        ["address": addr, "argv": ["zsh"], "created_ms": 9, "status_ms": s, "status": ["state": "working", "app": "claude-code"],
         "foreground": ["name": "secretspec", "argv": ["secretspec", "run", "--", "claude"], "pid": fg]]
    }
    let ws = HCMapper.workspaces(terminals: [t("a", 100), t("b", 200), t("c", 300)], appDoc: nil, check: bc)
    let ids = ws.flatMap(\.tabs).flatMap(\.panes).map { $0.agent?.process ?? "nil" }
    check(batch.reads == 1 && ids == ["100/101@9", "200/201@9", "300/301@9"], "wrapper 20: three wrappers, one table read (\(batch.reads), \(ids))")
    _ = HCMapper.workspaces(terminals: [t("a", 100)], appDoc: nil, check: bc)
    check(batch.reads == 1, "wrapper 20: a cached terminal reads nothing")
    _ = HCMapper.workspaces(terminals: [t("a", 100), t("b", 200)], appDoc: nil, check: bc)
    check(batch.reads == 2, "wrapper 20: a pruned terminal reads again when it comes back (\(batch.reads))")
}

/// Tests 21 to 23: the nearest agent in a tree (shared by the gate and the feed).
@MainActor private func checksWrapperResolver() {
    let p = [proc(400, 1, baseMs, "-zsh"), proc(500, 400, baseMs, "secretspec run -- claude"),
             proc(501, 500, baseMs + 1000, "claude"), proc(502, 501, baseMs + 2000, "claude -p helper")]
    check(R.agentInTree(root: 500, kind: "claude", procs: p, tieNewest: false)?.pid == 501, "wrapper 21: nearest agent wins")
    let tie = p + [proc(503, 500, baseMs + 3000, "claude")]
    check(R.agentInTree(root: 500, kind: "claude", procs: tie, tieNewest: false) == nil, "wrapper 21: a tie -> none for the gate")
    check(R.agentInTree(root: 500, kind: "claude", procs: tie, tieNewest: true)?.pid == 503, "wrapper 21: a tie -> newest for the feed")
    check(R.agentInTree(root: 500, kind: "claude", procs: [proc(500, 400, baseMs, "claude")], tieNewest: false) == nil,
          "wrapper 21: the root itself does not count (depth >= 1)")
    // 22: zsh -> secretspec -> claude, pids as the mapper gives them (foreground first): the feed finds claude, not the helper.
    check(R.hcAgentProcess(pids: [500, 400], kind: "claude", procs: p)?.pid == 501, "wrapper 22: the feed finds the wrapped agent")
    // 23: a shell in front, no agent process: nothing.
    check(R.hcAgentProcess(pids: [400], kind: "claude", procs: [proc(400, 1, baseMs, "-zsh"), proc(410, 400, baseMs, "vim")]) == nil,
          "wrapper 23: no agent process -> no feed process")
}

/// Tests 14 and 15 (and the client twin): the gate bound to a wrapper identity, through a real snapshot.
@MainActor private func checksWrapperGate() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    let procs = Procs([proc(500, 400, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs + 1000, "claude")])
    await hc.setWrapperCheck(HCTreeCheck(source: { procs.read() }, now: { ProcessInfo.processInfo.systemUptime }, alive: { procs.has($0) }))
    func wrapTerm(fg: Int) -> [String: Any] {
        var t = FakeHC.terminal("t1", argv: ["zsh"], pid: 400, status: "blocked", kind: "permission",
                                fg: ["name": "secretspec", "argv": ["secretspec", "run", "--", "claude"], "pid": fg])
        var st = t["status"] as! [String: Any]; st["app"] = "claude-code"; t["status"] = st
        t["status_ms"] = baseMs + 2000
        return t
    }
    fake.terminals = [wrapTerm(fg: 500)]
    let seen = try await hc.snapshot().flatMap(\.tabs).flatMap(\.panes).first { $0.id == "hc:t1" }?.agent
    check(seen?.kind == "claude" && seen?.process == "500/501@1", "wrapper gate: the snapshot binds the wrapped agent (\(seen?.process ?? "nil"))")
    check(seen.map(KeyGate.hasIdentity) == true, "wrapper gate: the wrapped agent has an identity")
    let gate = KeyGate(agent: seen, allowed: { true })
    check(await hc.passes(gate, paneId: "hc:t1"), "wrapper 14: the same wrapper and agent -> keys may go")
    // 14: a new foreground pid.
    procs.set([proc(600, 400, baseMs, "secretspec run -- claude"), proc(601, 600, baseMs + 1000, "claude")])
    fake.terminals = [wrapTerm(fg: 600)]
    check(await !hc.passes(gate, paneId: "hc:t1"), "wrapper 14: a new foreground pid -> no")
    // The agent restarted under the same wrapper.
    procs.set([proc(500, 400, baseMs, "secretspec run -- claude"), proc(509, 500, baseMs + 1500, "claude")])
    fake.terminals = [wrapTerm(fg: 500)]
    check(await !hc.passes(gate, paneId: "hc:t1"), "wrapper 14: a restarted agent under the same wrapper -> no")
    // 15: the agent exits after the first key, the wrapper stays: the keys stop there.
    procs.set([proc(500, 400, baseMs, "secretspec run -- claude"), proc(501, 500, baseMs + 1000, "claude")])
    check(await hc.passes(gate, paneId: "hc:t1"), "wrapper 15: back to the bound agent -> keys may go")
    fake.onKeys = { _, _ in procs.set([proc(500, 400, baseMs, "secretspec run -- claude")]) }
    defer { fake.onKeys = nil }
    let before = fake.calls.filter { $0.method == "terminal.keys" }.count
    var err: Error?
    do { try await hc.gatedKeys(["Down", "Enter"], gate: gate, paneId: "hc:t1", screen: nil) } catch { err = error }
    let sent = fake.calls.filter { $0.method == "terminal.keys" }.count - before
    check(err is KeyGateStop && sent == 1, "wrapper 15: the agent gone after the first key -> stopped before Enter (\(sent) sent, \(String(describing: err)))")
}

/// Codex P1 twin: the log cache is bound to the resolved agent child, not only to the pane's pids.
private func checksWrapperLogCache() async {
    let a = "/tmp/hkcs-wrap/.codex/sessions/2026/11/13/rollout-2026-11-13T10-00-00-a.jsonl"
    let b = "/tmp/hkcs-wrap/.codex/sessions/2026/11/13/rollout-2026-11-13T10-05-00-b.jsonl"
    final class Open: @unchecked Sendable { let l = NSLock(); var m: [Int32: String] = [:]
        func set(_ x: [Int32: String]) { l.withLock { m = x } }
        func files(_ pids: [Int32]) -> [String] { l.withLock { pids.compactMap { m[$0] } } } }
    let open = Open()
    open.set([501: a])
    let r = SessionResolver(socketPath: "/nonexistent", home: "/tmp/hkcs-wrap", openFiles: { open.files($0) })
    var procs = [proc(400, 1, baseMs, "-zsh"), proc(500, 400, baseMs, "secretspec run -- codex"), proc(501, 500, baseMs + 1000, "codex")]
    let need1 = R.Need(paneId: "hc:t1", kind: "codex", cwd: "/tmp", pids: [500, 400], process: "500/501@1")
    let first = await r.resolve([need1], claimed: [], procs: procs)
    check(first["hc:t1"] == a, "wrapper log: the wrapped codex's open rollout (\(first["hc:t1"] ?? "nil"))")
    // Codex restarted under the same wrapper: same pane pids, another child. The old log must not stay.
    procs = [proc(400, 1, baseMs, "-zsh"), proc(500, 400, baseMs, "secretspec run -- codex"), proc(509, 500, baseMs + 5000, "codex")]
    open.set([509: b])
    let need2 = R.Need(paneId: "hc:t1", kind: "codex", cwd: "/tmp", pids: [500, 400], process: "500/509@1")
    let second = await r.resolve([need2], claimed: [], procs: procs)
    check(second["hc:t1"] == b, "wrapper log: a restarted child -> the cache is due at once, its own log (\(second["hc:t1"] ?? "nil"))")
}
