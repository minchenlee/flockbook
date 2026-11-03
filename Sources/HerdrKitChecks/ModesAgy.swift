import Foundation
@testable import HerdrKit

/// `--agy-resolve-live <socket> <paneId> [seconds]`: one SessionResolver follows a live agy pane of a named herdr
/// session (TUI, found by `HERDR_PANE_ID`) and prints the conversation id each time it changes. Read-only: `ps` and
/// `lsof` only; sends nothing. Use it across `/new` in the same agy process. Prints ids, never log text.
@MainActor func modeAgyResolveLive() async throws {
    let a = CommandLine.arguments
    guard let i = a.firstIndex(of: "--agy-resolve-live"), a.indices.contains(i + 2) else { return }
    let sock = a[i + 1], pane = a[i + 2]
    let secs = a.indices.contains(i + 3) ? Double(a[i + 3]) ?? 60 : 60
    let res = SessionResolver(socketPath: sock, retry: 1, recheck: 1)
    let need = SessionResolver.Need(paneId: pane, kind: "agy", cwd: "")
    var last: String?? = .none
    let t0 = Date()
    while Date().timeIntervalSince(t0) < secs {
        let p = await res.resolve([need], claimed: [])[pane]
        let id = p.map { $0.components(separatedBy: "/brain/").last?.components(separatedBy: "/").first ?? "?" }
        if last == nil || last! != id {
            print("agy-resolve \(Int(Date().timeIntervalSince(t0)))s: \(id ?? "none")")
            fflush(stdout)
            last = .some(id)
        }
        try await Task.sleep(for: .seconds(1))
    }
    exit(0)
}
