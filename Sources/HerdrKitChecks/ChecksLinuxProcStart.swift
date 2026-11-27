import Foundation
@testable import HerdrKit

// Claude Code on Linux writes `procStart` in `<pid>.json` as `/proc/<pid>/stat` starttime (clock ticks after boot),
// not `ps lstart` text. On another machine the app reads it with that machine's boot time and CLK_TCK.

func checksLinuxProcStart() {
    typealias R = SessionResolver
    let utc = DateFormatter(); utc.locale = Locale(identifier: "en_US_POSIX"); utc.timeZone = TimeZone(identifier: "UTC"); utc.dateFormat = "EEE MMM d HH:mm:ss yyyy"

    // The clock line.
    check(R.ProcClock.parse("1788008817 100") == R.ProcClock(bootTime: 1788008817, ticksPerSecond: 100), "procclock: btime and CLK_TCK")
    check(R.ProcClock.parse(" 100") == nil && R.ProcClock.parse("") == nil, "procclock: no btime (not Linux) is no clock")
    check(R.ProcClock.parse("1788008817 0") == nil && R.ProcClock.parse("x 100") == nil && R.ProcClock.parse("1 2 3") == nil,
          "procclock: zero, junk or extra words are no clock")

    // procStart: text, Linux ticks, Windows FILETIME.
    let clock = R.ProcClock(bootTime: 1788008817, ticksPerSecond: 100)
    // A real pair from a Linux machine: btime 1788008817, starttime 779125042 -> ps lstart "Fri Nov 27 17:21:07 2026" (UTC).
    let real = utc.date(from: "Fri Nov 27 17:21:07 2026")!
    check(R.claudeProcStart("779125042", clock: clock) == real, "procStart: Linux ticks after boot (\(String(describing: R.claudeProcStart("779125042", clock: clock))))")
    check(R.claudeProcStart("779125042", clock: nil) == nil, "procStart: Linux ticks without that machine's clock give no date")
    check(R.claudeProcStart("Fri Nov 27 17:21:07 2026", clock: clock) == real
          && R.claudeProcStart("Mon Nov  2 10:00:00 2026", clock: nil) == utc.date(from: "Mon Nov 2 10:00:00 2026"),
          "procStart: ps lstart text, with or without a clock")
    check(R.claudeProcStart("134396888779841927", clock: nil) == Date(timeIntervalSince1970: 13439688877 - 11_644_473_600),
          "procStart: a Windows FILETIME")
    check(R.claudeProcStart("", clock: clock) == nil && R.claudeProcStart("12a", clock: clock) == nil
          && R.claudeProcStart("99999999999999999999999", clock: clock) == nil, "procStart: empty, junk or overflow give no date")

    // The remote ps output, with and without the clock line.
    let ps = "  700     1 Fri Nov 27 17:21:07 2026 claude\n  701   700 Fri Nov 27 17:22:00 2026 claude -p x\n"
    let linux = R.parseRemoteProcesses("/home/dev\nprocclock 1788008817 100\n" + ps)
    check(linux?.home == "/home/dev" && linux?.clock == clock && linux?.procs.map(\.pid) == [700, 701]
          && linux?.procs.first?.start == real, "remote ps: home, clock, then processes")
    let mac = R.parseRemoteProcesses("/Users/b\nprocclock  100\n" + ps)
    check(mac?.clock == nil && mac?.procs.count == 2, "remote ps: a Mac there prints no btime: no clock, processes still read")
    let old = R.parseRemoteProcesses("/home/b\n" + ps)
    check(old?.clock == nil && old?.procs.count == 2, "remote ps: no clock line at all still reads")
    check(R.parseRemoteProcesses("procclock 1 100\n" + ps) == nil, "remote ps: no home, no result")
    check(R.remoteProcessesLine.contains("ps -A -ww -o pid=,ppid=,lstart=,args=") && R.remoteProcessesLine.contains("/proc/stat")
          && R.remoteProcessesLine.contains("getconf CLK_TCK"), "remote ps: the line asks for home, clock and ps")

    // The pid file with Linux ticks.
    let procs = linux!.procs
    func file(_ pid: Int, _ sid: String, _ start: String) -> Data {
        try! JSONSerialization.data(withJSONObject: ["pid": pid, "sessionId": sid, "procStart": start])
    }
    let files: [Int32: Data] = [700: file(700, "sid-linux", "779125042")]
    check(R.claudeSessionSource(pids: [700], procs: procs, clock: clock, read: { files[$0] })?.sid == "sid-linux",
          "claude: a Linux pid file matches with that machine's clock")
    check(R.claudeSessionSource(pids: [700], procs: procs, clock: nil, read: { files[$0] }) == nil,
          "claude: a Linux pid file without a clock does not count (fail closed)")
    let stale: [Int32: Data] = [700: file(700, "sid-stale", "779100000")]   // about 4 minutes earlier: pid reused
    check(R.claudeSessionSource(pids: [700], procs: procs, clock: clock, read: { stale[$0] }) == nil,
          "claude: a Linux pid file from another process start is ignored")
    let child: [Int32: Data] = [700: file(700, "sid-linux", "779125042"), 701: file(701, "sid-child", "779130342")]
    check(R.claudeSessionSource(pids: [700], procs: procs, clock: clock, read: { child[$0] }) == nil,
          "claude: two sessions in one tree is no guess, with Linux ticks too")
}

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
