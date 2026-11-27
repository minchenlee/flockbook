import Foundation
@testable import HerdrKit

// Claude Code on Linux writes `procStart` in `<pid>.json` as `/proc/<pid>/stat` starttime (clock ticks after boot),
// not `ps lstart` text. On another machine the app reads it with that machine's boot time and CLK_TCK.

/// End to end on a fake Linux machine: ps with the clock line, a pid file with ticks, the log mirrored here.
@MainActor func checksHCRemoteLinuxClaude() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("box/t1", argv: ["claude"], pid: 900, cwd: "/home/dev/w")]
    let sid = "0b1c2d3e-aaaa-bbbb-cccc-000000000002"
    let log = "/home/dev/.claude/projects/-home-dev-w/\(sid).jsonl"
    fake.execReplies = [
        ("ps ", "/home/dev\nprocclock 1788008817 100\n900 899 Fri Nov 27 16:48:30 2026 claude\n"),
        (".claude/sessions", "900\t{\"pid\":900,\"sessionId\":\"\(sid)\",\"procStart\":\"778929315\",\"version\":\"2.1.332\"}\n"),
        (".claude/projects", log + "\n"),
    ]
    fake.files = ["box:\(log)": "{\"type\":\"user\"}\n"]
    let root = NSTemporaryDirectory() + "hkmirror-\(UUID().uuidString)"
    defer { try? FileManager.default.removeItem(atPath: root) }
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    await hc.setMirror(root: root, poll: 0.05)
    let found = await hc.resolveRemote([SessionResolver.Need(paneId: "hc:box/t1", kind: "claude", cwd: "/home/dev/w")], claimed: [])
    check(found["hc:box/t1"]?.hasSuffix("\(sid).jsonl") == true, "remote logs: Claude on Linux (ticks procStart) mirrors its log (\(found))")
    let line = fake.calls.filter { $0.method == "machine.exec" }.compactMap { $0.params["line"] as? String }.first { $0.contains("ps ") }
    check(line?.contains("procclock") == true, "remote logs: the ps line asks for the machine's clock")
    await hc.stopMirrors()
}
