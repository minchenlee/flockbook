import Foundation
@testable import HerdrKit

// omp (oh-my-pi, a pi fork): pi's log under `~/.omp/agent/sessions`, folders named by omp's session-paths.ts, a
// `title` slot before the header, more entry types and `activeLeafId`. The Rust helper has the same rules and tests
// (xplat/herdr-core: hc/mapper.rs omp_kind, resolver/tests.rs omp_*, tests/conversation.rs omp_*).

private let ompTitle = #"{"type":"title","v":1,"title":"Fix the build","source":"auto","updatedAt":"2026-11-10T07:00:00.000Z","pad":"    "}"#

private func ompHead(_ id: String, _ started: Date, _ cwd: String) -> String {
    #"{"type":"session","version":3,"id":"\#(id)","timestamp":"\#(ISO8601DateFormatter.withFraction.string(from: started))","cwd":"\#(cwd)"}"#
}

/// omp by its program (alone, or through bun), or by its 7501 app name when a launcher shows a shell in front.
@MainActor func checksOmpKind() {
    func term(_ o: [String: Any]) -> HCMapper.Term { HCMapper.Term(o)! }
    var t = term(["address": "t5", "argv": ["/usr/local/bin/omp"], "pid": 20, "created_ms": 7])
    check(HCMapper.kind(t) == "omp" && HCMapper.process(t, kind: "omp") == "20@7", "omp kind: its program")
    t = term(["address": "t6", "argv": ["bun", "/opt/omp/bin/omp"], "pid": 21])
    check(HCMapper.kind(t) == "omp", "omp kind: through an interpreter")
    t = term(["address": "t7", "argv": ["launch"], "pid": 22, "foreground": ["name": "fish", "pid": 23], "status": ["state": "idle", "app": "omp"]])
    check(HCMapper.kind(t) == "omp", "omp kind: a shell in front, omp by its 7501 app name")
    check(HCMapper.process(t, kind: "omp") == nil, "omp kind: keys still need the omp process itself")
    check(SessionResolver.logPath(openFiles: ["/home/u/.omp/agent/sessions/-src-app/a.jsonl"], kind: "omp", home: "/home/u")
          == "/home/u/.omp/agent/sessions/-src-app/a.jsonl", "omp: an open log under ~/.omp")
}

@MainActor func checksOmpFolderNames() {
    func d(_ cwd: String, _ home: String, _ tmp: String) -> String { SessionResolver.ompSessionDir(cwd: cwd, sessions: "/s", home: home, tmp: tmp) }
    check(d("/home/u/src/app", "/home/u", "/tmp") == "/s/-src-app", "omp folder: under home, '-' + the path from home")
    check(d("/home/u", "/home/u", "/tmp") == "/s/-", "omp folder: home itself")
    check(d("/tmp/x/y", "/home/u", "/tmp") == "/s/-tmp-x-y", "omp folder: under the temp root, '-tmp-' + the path from it")
    check(d("/tmp", "/home/u", "/tmp") == "/s/-tmp", "omp folder: the temp root itself")
    check(d("/home/u/.t/a", "/home/u", "/home/u/.t") == "/s/-tmp-a", "omp folder: the temp root wins when it is inside home")
    check(d("/srv/app", "/home/u", "/tmp") == "/s/--srv-app--", "omp folder: elsewhere, the absolute path in '--'")
    check(d("/private/tmp/app", "/home/u", "/private/var/folders/ab/T") == "/s/--private-tmp-app--", "omp folder: /private/tmp is outside TMPDIR")
    check(d("/home/u/a:b\\c", "/home/u", "/tmp") == "/s/-a-b-c", "omp folder: ':' and '\\' become '-'")
    check(d("/home/user2/x", "/home/u", "/tmp") == "/s/--home-user2-x--", "omp folder: a sibling of home is not under home")
    check(d("/home/u/src/app", "/home/u/", "/tmp/") == "/s/-src-app", "omp folder: roots with a slash at the end")

    let line = "omp --x PI_CODING_AGENT_DIR=/data/omp profiles/work TMPDIR=/t/my tmp HC_TERMINAL_ID=t1"
    check(SessionResolver.envValue(line, "PI_CODING_AGENT_DIR") == "/data/omp profiles/work", "omp env: a value runs to the next NAME=")
    check(SessionResolver.envValue(line, "TMPDIR") == "/t/my tmp", "omp env: a value with a space")
    check(SessionResolver.envValue(line, "HC_TERMINAL_ID") == "t1", "omp env: the last value runs to the end")
    check(SessionResolver.envValue("omp PI_CODING_AGENT_DIR= HOME=/h", "PI_CODING_AGENT_DIR") == nil, "omp env: empty is unset")
    check(SessionResolver.envValue("omp XPI_CODING_AGENT_DIR=/a", "PI_CODING_AGENT_DIR") == nil, "omp env: whole names only")
}

@MainActor func checksOmpSession() async throws {
    let fm = FileManager.default
    let base = NSTemporaryDirectory() + "hkomp-\(UUID().uuidString)"
    let work = NSTemporaryDirectory() + "hkompw-\(UUID().uuidString)"
    defer { try? fm.removeItem(atPath: base); try? fm.removeItem(atPath: work) }
    try fm.createDirectory(atPath: work, withIntermediateDirectories: true)
    let start = Date().addingTimeInterval(-600)
    func write(_ dir: String, _ name: String, cwd: String, title: Bool = true, started: Date? = nil, modified: Date? = nil) -> String {
        try? fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
        let p = dir + "/" + name + ".jsonl"
        let text = (title ? ompTitle + "\n" : "") + ompHead(name, started ?? start.addingTimeInterval(3), cwd) + "\n"
        fm.createFile(atPath: p, contents: Data(text.utf8))
        try? fm.setAttributes([.modificationDate: modified ?? start.addingTimeInterval(60)], ofItemAtPath: p)
        return p
    }
    func resolver(_ root: String) -> SessionResolver { SessionResolver(socketPath: "/nonexistent", ompSessionsDir: root, home: "/home/u") }

    // The header after a title slot.
    let lines: [(String, Date, Data)] = [
        ("/r/a.jsonl", start.addingTimeInterval(50), Data((ompTitle + "\n" + ompHead("a", start.addingTimeInterval(2), "/r/w") + "\n").utf8)),
        ("/r/b.jsonl", start.addingTimeInterval(80), Data((ompTitle + "\n").utf8)),
    ]
    check(SessionResolver.piPick(lines, cwd: "/r/w", after: start, claimed: { _ in false }) == "/r/a.jsonl", "omp: the session header after a title slot")

    // A cwd outside home and temp, sessions under the default root.
    let cwd = "/w/omp-proj", root = base + "/default/sessions"
    let mine = write(root + "/--w-omp-proj--", "2026-11-10T07-35-44-761Z_a", cwd: cwd)
    _ = write(base, "decoy", cwd: cwd)
    var procs = [SessionResolver.Proc(pid: 940_001, ppid: 1, start: start, line: "omp HC_TERMINAL_ID=t8")]
    var got = await resolver(root).resolve([.init(paneId: "hc:t8", kind: "omp", cwd: cwd, pids: [940_001])], claimed: [], procs: procs)
    check(got["hc:t8"] == mine, "omp: its file in its cwd's folder (\(got))")

    // PI_CODING_AGENT_DIR in the process env moves the root; a cwd in the temp root gets the -tmp- name.
    let real = SessionResolver.realPath(work)
    let moved = base + "/moved"
    let other = write("\(moved)/sessions/-tmp-\((real as NSString).lastPathComponent)", "2026-11-10T07-35-44-761Z_b", cwd: work)
    procs = [SessionResolver.Proc(pid: 940_002, ppid: 1, start: start, line: "bun /usr/local/bin/omp PI_CODING_AGENT_DIR=\(moved) HC_TERMINAL_ID=t9")]
    got = await resolver(root).resolve([.init(paneId: "hc:t9", kind: "omp", cwd: work, pids: [940_002])], claimed: [], procs: procs)
    check(got["hc:t9"] == other, "omp: agent dir from the process env, folder from the real cwd (\(got), real cwd \(real))")

    // PI_CODING_AGENT_DIR=./.omp-agent is resolved from omp's cwd.
    let rel = write("\(work)/.omp-agent/sessions/-tmp-\((real as NSString).lastPathComponent)", "2026-11-10T07-35-44-761Z_c", cwd: work, title: false)
    procs = [SessionResolver.Proc(pid: 970_001, ppid: 1, start: start, line: "omp PI_CODING_AGENT_DIR=./.omp-agent HC_TERMINAL_ID=t3")]
    got = await resolver("/nonexistent/sessions").resolve([.init(paneId: "hc:t3", kind: "omp", cwd: work, pids: [970_001])], claimed: [], procs: procs)
    check(got["hc:t3"] == rel, "omp: a relative agent dir under the agent's cwd (\(got))")

    // Two omp in one cwd with their own PI_CODING_AGENT_DIR: different folders, so no rivals. A's /new after B
    // started: a header later than B's start, in A's own folder.
    let two = "/w/omp-two", a0 = Date().addingTimeInterval(-900), b0 = a0.addingTimeInterval(300)
    let fa = write("\(base)/a/sessions/--w-omp-two--", "a", cwd: two, title: false, started: b0.addingTimeInterval(100), modified: b0.addingTimeInterval(110))
    let fb = write("\(base)/b/sessions/--w-omp-two--", "b", cwd: two, title: false, started: b0.addingTimeInterval(1), modified: b0.addingTimeInterval(11))
    procs = [SessionResolver.Proc(pid: 950_001, ppid: 1, start: a0, line: "omp PI_CODING_AGENT_DIR=\(base)/a HC_TERMINAL_ID=ta"),
             SessionResolver.Proc(pid: 950_002, ppid: 1, start: b0, line: "omp PI_CODING_AGENT_DIR=\(base)/b HC_TERMINAL_ID=tb")]
    got = await resolver(root).resolve([.init(paneId: "hc:ta", kind: "omp", cwd: two, pids: [950_001]),
                                        .init(paneId: "hc:tb", kind: "omp", cwd: two, pids: [950_002])], claimed: [], procs: procs)
    check(got["hc:ta"] == fa && got["hc:tb"] == fb, "omp two: each omp reads its own folder (\(got))")
}

/// /w/a-b and /w/a/b both name pi's folder --w-a-b--: the two pi are not rivals.
@MainActor func checksPiFolderSharedByTwoCwds() async throws {
    let fm = FileManager.default
    let agentDir = NSTemporaryDirectory() + "hkpi3-\(UUID().uuidString)"
    defer { try? fm.removeItem(atPath: agentDir) }
    let dir = SessionResolver.piSessionDir(cwd: "/w/a-b", agentDir: agentDir)
    check(dir == SessionResolver.piSessionDir(cwd: "/w/a/b", agentDir: agentDir), "pi shared folder: two cwds, one folder")
    try fm.createDirectory(atPath: dir, withIntermediateDirectories: true)
    let a0 = Date().addingTimeInterval(-900), b0 = a0.addingTimeInterval(300)
    func file(_ id: String, _ started: Date, _ cwd: String) -> String {
        let p = "\(dir)/\(id).jsonl"
        fm.createFile(atPath: p, contents: Data((ompHead(id, started, cwd) + "\n").utf8))
        try? fm.setAttributes([.modificationDate: started.addingTimeInterval(10)], ofItemAtPath: p)
        return p
    }
    let fa = file("a", b0.addingTimeInterval(100), "/w/a-b"), fb = file("b", b0.addingTimeInterval(1), "/w/a/b")
    let procs = [SessionResolver.Proc(pid: 960_001, ppid: 1, start: a0, line: "node pi"),
                 SessionResolver.Proc(pid: 960_002, ppid: 1, start: b0, line: "node pi")]
    let got = await SessionResolver(socketPath: "/nonexistent", piAgentDir: agentDir)
        .resolve([.init(paneId: "hc:ta", kind: "pi", cwd: "/w/a-b", pids: [960_001]),
                  .init(paneId: "hc:tb", kind: "pi", cwd: "/w/a/b", pids: [960_002])], claimed: [], procs: procs)
    check(got["hc:ta"] == fa && got["hc:tb"] == fb, "pi shared folder: each pi keeps its own cwd's file (\(got))")
}

/// omp on another machine: its default root there, the folder named from the real cwd, home and temp root.
@MainActor func checksOmpRemote() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("box/t6", argv: ["bun", "/usr/bin/omp"], pid: 600, cwd: "/home/b/src/app")]
    let path = "/home/b/.omp/agent/sessions/-src-app/2026-11-10T10-00-03-000Z_01aa.jsonl"
    let head = #"{"type":"session","version":3,"id":"01aa","timestamp":"2026-11-10T10:00:03.000Z","cwd":"/home/b/src/app"}"#
    fake.execReplies = [
        ("ps ", "/home/b\n  600     1 Tue Nov 10 10:00:00 2026 bun /usr/bin/omp\n"),
        ("pwd -P", "/home/b/src/app\n/home/b\n/tmp\n"),
        ("/.omp/agent/sessions/-src-app", "\(path)\t1794304900\t\(head)\n"),
    ]
    fake.files = ["box:\(path)": ompTitle + "\n" + head + "\n"]
    let root = NSTemporaryDirectory() + "hkompm-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setMirror(root: root, poll: 0.05)
    let found = await hc.resolveRemote([SessionResolver.Need(paneId: "hc:box/t6", kind: "omp", cwd: "/home/b/src/app")], claimed: [])
    check(found["hc:box/t6"]?.hasSuffix("_01aa.jsonl") == true, "omp remote: found in its cwd's session folder on box (\(found))")
    let ls = fake.calls.filter { $0.method == "machine.exec" }.compactMap { $0.params["line"] as? String }.first { $0.contains(".omp/agent") }
    check(ls?.contains("'/home/b/.omp/agent/sessions/-src-app'/*.jsonl") == true && ls?.contains(#""type":"title""#) == true,
          "omp remote: box's home, quoted folder, the line after the title slot (\(ls ?? "nil"))")
    await hc.stopMirrors()
}

/// A synthetic omp log: pi's tree, with a title slot first, `activeLeafId` on off-branch entries and the newer
/// entry types.
private func ompLines() -> [String] {
    func m(_ id: String, _ parent: String?, _ role: String, _ text: String, _ extra: String = "") -> String {
        let p = parent.map { "\"\($0)\"" } ?? "null"
        return #"{"type":"message","id":"\#(id)","parentId":\#(p),"timestamp":"2026-11-10T07:00:00.000Z"\#(extra),"message":{"role":"\#(role)","content":[{"type":"text","text":"\#(text)"}],"timestamp":1}}"#
    }
    func e(_ type: String, _ id: String, _ parent: String?, _ extra: String) -> String {
        let p = parent.map { "\"\($0)\"" } ?? "null"
        return #"{"type":"\#(type)","id":"\#(id)","parentId":\#(p),"timestamp":"2026-11-10T07:00:00.000Z"\#(extra)}"#
    }
    return [
        ompTitle,
        #"{"type":"session","version":3,"id":"s0","title":"Fix the build","timestamp":"2026-11-10T07:00:00.000Z","cwd":"/home/u/src/app"}"#,
        e("session_init", "i1", nil, #","systemPrompt":"x""#),
        m("u1", "i1", "user", "Fix the build."),
        m("a1", "u1", "assistant", "On it."),
        m("u2", "a1", "user", "First try."),
        m("a2", "u2", "assistant", "Answer one."),
        m("u3", "a1", "user", "Second try."),
        m("a3", "u3", "assistant", "Answer two."),
        // 9: late usage and a background result written on the first branch while the second is shown.
        e("model_usage", "k1", "a2", #","activeLeafId":"a3","purpose":"title","api":"x","provider":"x","model":"x","usage":{},"stopReason":"stop""#),
        m("b1", "a2", "assistant", "Late result.", #","activeLeafId":"a3""#),
        // 11: newer entry types in the chain.
        e("mode_change", "mc1", "a3", #","mode":"plan""#),
        e("label", "l1", "mc1", #","targetId":"a3","label":"good""#),
        e("custom_message", "n1", "l1", #","customType":"bg","content":"<system-notice>\nBackground job done.\n</system-notice>","display":true"#),
        e("custom_message", "n2", "n1", #","customType":"ext","content":"Subagent finished.","display":true"#),
        m("u4", "n2", "user", "Third."),
        m("u5", "n2", "user", "Third again."),
    ]
}

@MainActor func checksOmpTree() {
    let ol = ompLines()
    func shown(_ n: Int) -> [String] { describe(SessionLog.parse(lines: Array(ol.prefix(n)))) }
    let second = ["user:Fix the build.", "asst:On it.", "user:Second try.", "asst:Answer two."]
    check(shown(9) == second, "omp: title slot and session_init skipped; newest branch \(shown(9))")
    check(shown(10) == second, "omp: late usage keeps the shown branch (activeLeafId) \(shown(10))")
    check(shown(11) == second, "omp: an off-branch result is not shown and does not switch the branch \(shown(11))")
    check(shown(ol.count) == second + ["status", "user:Third again."],
          "omp: newer entry types keep the chain; a <system-notice> custom_message is no status row \(shown(ol.count))")
    let items = SessionLog.parse(lines: ol)
    check(items.count > 4 && { if case .status(let s) = items[4].kind { return s == "Subagent finished." }; return false }(), "omp: other custom_message rows stay")
    var p = ConversationParser()
    for l in ol { p.feed(line: l) }
    check(describe(p.items()) == describe(items), "omp: incremental feed = batch")
}

@MainActor func checksOmpClearedLeaf() {
    let ol = ompLines()
    func msg(_ id: String, _ parent: String?, _ role: String, _ content: String, leaf: String = "") -> String {
        let p = parent.map { "\"\($0)\"" } ?? "null"
        return #"{"type":"message","id":"\#(id)","parentId":\#(p),"timestamp":"2026-11-10T07:00:00.000Z"\#(leaf),"message":{"role":"\#(role)","content":[\#(content)],"timestamp":1}}"#
    }
    func text(_ t: String) -> String { #"{"type":"text","text":"\#(t)"}"# }
    func parsed(_ l: [String]) -> [String] { describe(SessionLog.parse(lines: l)) }
    let second = ["user:Fix the build.", "asst:On it.", "user:Second try.", "asst:Answer two."]

    // activeLeafId null: the leaf was cleared; the entry itself is not shown.
    var lines = Array(ol.prefix(9))
    lines.append(msg("b2", "a3", "assistant", text("Hidden."), leaf: #","activeLeafId":null"#))
    check(!parsed(lines).contains { $0.contains("Hidden") }, "omp: an entry with a cleared leaf is not shown \(parsed(lines))")

    // The shown branch goes too, with an open tool call; the next root starts a new list.
    lines = Array(ol.prefix(9))
    lines.append(msg("a4", "a3", "assistant", #"{"type":"toolCall","id":"c9","name":"bash","arguments":{"command":"make"}}"#))
    check(parsed(lines).last == "perm:make", "omp: the open call before the clear \(parsed(lines))")
    lines.append(#"{"type":"label","id":"z1","parentId":"a4","timestamp":"2026-11-10T07:00:00.000Z","activeLeafId":null,"targetId":"a4"}"#)
    var p = ConversationParser()
    for l in lines { p.feed(line: l) }
    check(p.items().isEmpty && !p.hasOpenToolUse, "omp: a cleared leaf clears the shown items \(describe(p.items()))")
    lines.append(msg("u9", nil, "user", text("Fresh start.")))
    check(parsed(lines) == ["user:Fresh start."], "omp: the next root is shown alone \(parsed(lines))")
    // The tree is kept: an entry on the old branch shows it again.
    lines.append(msg("u8", "a3", "user", text("Back.")))
    check(parsed(lines) == second + ["user:Back."], "omp: the old branch can be rebuilt \(parsed(lines))")

    // Cleared, then the old branch goes on at once (no new root first): its history comes back.
    lines = Array(ol.prefix(9))
    lines.append(#"{"type":"label","id":"z2","parentId":"a3","timestamp":"2026-11-10T07:00:00.000Z","activeLeafId":null,"targetId":"a3"}"#)
    lines.append(msg("u7", "a3", "user", text("Go on.")))
    check(parsed(lines) == second + ["user:Go on."], "omp: an entry on the old branch right after a clear rebuilds it \(parsed(lines))")
}
