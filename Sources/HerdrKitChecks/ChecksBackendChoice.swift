import Foundation
@testable import HerdrKit

/// Settings > General > Backend: which loops run for each choice and launch flags, and when a backend that does not
/// answer is reported.
func checksBackendPlan() {
    func plan(_ c: BackendChoice, _ args: [String] = [], headless: Bool = false, remote: Bool = false) -> BackendPlan {
        BackendPlan.make(choice: c, arguments: args, environment: [:], headless: headless, remote: remote)
    }
    check(BackendChoice(rawValue: "") == nil && BackendChoice.read(nil) == .automatic, "backend: no setting is Automatic")
    check(BackendChoice.read("hc") == .hcOnly && BackendChoice.read("junk") == .automatic, "backend: stored values read back, junk is Automatic")
    let a = plan(.automatic)
    check(a.tui && a.hc && a.quiet, "backend: Automatic runs both, a silent side is hidden")
    let h = plan(.hcOnly)
    check(!h.tui && h.hc, "backend: herdr-core only never runs the TUI")
    let t = plan(.tuiOnly)
    check(t.tui && !t.hc, "backend: Herdr TUI only never runs hc")
    let b = plan(.both)
    check(b.tui && b.hc && !b.quiet, "backend: Both runs both and reports both")
    // Launch flags win over the setting.
    let noHC = plan(.hcOnly, ["--no-hc"])
    check(noHC.tui && !noHC.hc && noHC.flags, "backend: --no-hc wins over herdr-core only")
    let sess = plan(.hcOnly, ["--session", "x"])
    check(sess.tui && sess.hc && sess.flags, "backend: --session keeps the TUI under herdr-core only")
    let sock = plan(.tuiOnly, ["--hc-socket", "/tmp/x.sock"])
    check(sock.tui && sock.hc && sock.flags, "backend: --hc-socket keeps hc under Herdr TUI only")
    check(!plan(.automatic).flags, "backend: no flags, the setting decides")
    // Another machine's herdr is a TUI: herdr-core only must not leave it blank.
    let rem = plan(.hcOnly, remote: true)
    check(rem.tui && rem.hc && rem.remote, "backend: a remote machine keeps its TUI under herdr-core only")
    // Headless runs follow hc only with an explicit socket (as before).
    let hl = plan(.automatic, headless: true)
    check(hl.tui && !hl.hc, "backend: headless without --hc-socket: no hc")
    let hlh = plan(.hcOnly, headless: true)
    check(hlh.tui && !hlh.hc, "backend: nothing left to run falls back to the TUI")

    // A failed TUI snapshot.
    check(a.tuiDown(hcConnected: true) == .hide, "Automatic: hc answers, a silent TUI is hidden (no diagnose)")
    check(a.tuiDown(hcConnected: false) == .report, "Automatic: neither answers, the TUI problem shows")
    check(b.tuiDown(hcConnected: true) == .report && t.tuiDown(hcConnected: false) == .report, "Both and TUI only report the TUI")
    check(a.hcDown(tuiConnected: true) == .hide && a.hcDown(tuiConnected: false) == .report, "Automatic: a silent hc is hidden while the TUI answers")
    check(h.hcDown(tuiConnected: false) == .report && b.hcDown(tuiConnected: true) == .report, "herdr-core only and Both report hc")
    check(!a.showsHCProblem && h.showsHCProblem && b.showsHCProblem, "hc's own problem: herdr-core only and Both; Automatic shows the TUI's")
    // Retry spacing while hidden: backs off to 30 s; reporting keeps the short reconnect.
    check(BackendPlan.retry(after: .milliseconds(100), hidden: true) == .seconds(2), "hidden: first retry after 2 s")
    check(BackendPlan.retry(after: .seconds(20), hidden: true) == .seconds(30), "hidden: retries back off to 30 s")
    check(BackendPlan.retry(after: .milliseconds(100), hidden: false, cap: .seconds(1)) == .milliseconds(200), "shown: the usual doubling")
    check(BackendPlan.retry(after: .seconds(1), hidden: false, cap: .seconds(1)) == .seconds(1), "shown: the usual 1 s cap")
}

/// The store under each plan, on fake sockets only.
@MainActor func checksBackendStore() async throws {
    // herdr-core only: the TUI socket is never opened, no diagnose, no TUI problem.
    let tui = FakeHerdr(path: NSTemporaryDirectory() + "hkbk-\(getpid()).sock")
    try tui.start()
    let hc = FakeHC(path: fakeHCPath())
    try hc.start()
    hc.terminals = [FakeHC.terminal("t1", argv: ["claude"], status: "idle")]
    let st = HerdrStore(session: HerdrSession(name: "fake", socketPath: tui.path), hc: HCSession(socketPath: hc.path),
                        plan: BackendPlan.make(choice: .hcOnly, arguments: [], environment: [:], headless: false, remote: false))
    st.start()
    check(await eventually(3) { st.hcConnected && st.workspaces.count == 1 }, "hc only: hc panes shown")
    await st.refresh()
    try? await Task.sleep(for: .milliseconds(500))
    check(tui.acceptCount == 0 && tui.requests.isEmpty, "hc only: the TUI client never connected (\(tui.acceptCount))")
    check(st.diagnoses == 0 && st.problem == nil && !st.tuiConnected, "hc only: no diagnose, no problem")
    hc.kill()
    check(await eventually(5) { st.problem == .hcNotRunning(socket: hc.path) }, "hc only: hc gone, its own problem shows: \(String(describing: st.problem))")
    check(tui.acceptCount == 0 && st.diagnoses == 0, "hc only: still no TUI call after hc left")
    st.stop()

    // Automatic, TUI silent while hc answers: no problem, no diagnose, slow retries.
    tui.kill()
    let hc2 = FakeHC(path: fakeHCPath())
    try hc2.start()
    hc2.terminals = [FakeHC.terminal("t1", argv: ["claude"], status: "idle")]
    let st2 = HerdrStore(session: HerdrSession(name: "fake", socketPath: tui.path), hc: HCSession(socketPath: hc2.path),
                         plan: BackendPlan.make(choice: .automatic, arguments: [], environment: [:], headless: false, remote: false))
    st2.start()
    check(await eventually(3) { st2.hcConnected }, "automatic: hc connects")
    try? await Task.sleep(for: .milliseconds(300))
    let tries0 = st2.tuiTries
    try? await Task.sleep(for: .seconds(4))
    check(st2.problem == nil && st2.diagnoses == 0, "automatic: a silent TUI shows no problem and is not diagnosed (\(st2.diagnoses))")
    // The usual reconnect would try about 5 times a second; hidden: at most every 2 s.
    check(st2.tuiTries - tries0 <= 3, "automatic: a silent TUI is retried slowly (\(st2.tuiTries - tries0) tries in 4 s)")
    // Neither answers: the TUI problem, as before.
    hc2.kill()
    check(await eventually(8) { st2.problem != nil && st2.problem != .hcNotRunning(socket: hc2.path) }, "automatic: neither answers, the TUI problem shows: \(String(describing: st2.problem))")
    st2.stop()

    // A new choice while running: the TUI side leaves at once, nothing is relaunched.
    let tui3 = FakeHerdr(path: NSTemporaryDirectory() + "hkbk3-\(getpid()).sock")
    try tui3.start()
    let hc3 = FakeHC(path: fakeHCPath())
    try hc3.start()
    hc3.terminals = [FakeHC.terminal("t1", argv: ["claude"], status: "idle")]
    let st3 = HerdrStore(session: HerdrSession(name: "fake", socketPath: tui3.path), hc: HCSession(socketPath: hc3.path),
                         plan: BackendPlan.make(choice: .both, arguments: [], environment: [:], headless: false, remote: false))
    st3.start()
    check(await eventually(3) { st3.workspaces.count == 2 }, "both: TUI and hc panes")
    check(st3.use(BackendPlan.make(choice: .hcOnly, arguments: [], environment: [:], headless: false, remote: false)), "switch to hc only without a relaunch")
    check(await eventually(3) { st3.workspaces.map(\.backend) == [.hc] && st3.hcConnected && !st3.tuiConnected }, "hc only after the switch: only hc panes")
    let before = tui3.acceptCount
    try? await Task.sleep(for: .milliseconds(2500))
    check(tui3.acceptCount == before, "after the switch the TUI is not polled")
    check(st3.use(BackendPlan.make(choice: .automatic, arguments: [], environment: [:], headless: false, remote: false)), "switch back")
    check(await eventually(3) { st3.workspaces.count == 2 }, "automatic after the switch: both again")
    st3.stop()
    let st4 = HerdrStore(session: HerdrSession(name: "fake", socketPath: tui3.path))
    check(!st4.use(BackendPlan.make(choice: .hcOnly, arguments: [], environment: [:], headless: false, remote: false)), "no hc client: the switch needs a relaunch")
    tui3.kill(); hc3.kill()
}
