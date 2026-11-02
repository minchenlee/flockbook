import Foundation
import CoreGraphics
@testable import HerdrKit

/// The launch guard: a check flag the app does not know, or a flag with its value in one argument, never reaches a normal
/// launch (real window, Dock icon). And every check flag the app's sources and scripts use is a known one.
func checksLaunchGuard() async throws {
    let app = "/x/HerdrDesk"
    func ok(_ args: [String]) -> Bool { LaunchGuard.problem([app] + args) == nil }
    check(ok([]), "launch guard: a plain launch")
    check(ok(["--session", "c9-input-test", "--read-only"]), "launch guard: ordinary options")
    check(ok(["--snapshot-settings", "/tmp/a b/dir"]), "launch guard: a value with a space, in its own argument")
    check(ok(["--test-sidebar"]) && ok(["--snapshot", "build/x"]) && ok(["--test-update", "http://127.0.0.1:8765/appcast.xml"]),
          "launch guard: known check flags")
    check(ok(["--test-screencolumn", "--light"]), "launch guard: a content check (ContentTests)")
    check(ok(["--test-fullscreen-real"]) && ok(["--test-scroll-real"]), "launch guard: the real-window checks are known")
    // macOS 15 support: the pre-26 look only with its flag or variable; a normal launch on 26 draws the 26 look.
    check(ok(["--snapshot", "build/x", "--legacy-look"]) && ok(["--test-legacy-look"]), "launch guard: the legacy look flag and its check are known")
    check(!LaunchGuard.legacyLookForced([app], environment: [:])
          && !LaunchGuard.legacyLookForced([app, "--snapshot", "build/x"], environment: ["FLOCKBOOK_LEGACY_LOOK": "0"]),
          "legacy look: off by default")
    check(LaunchGuard.legacyLookForced([app, "--snapshot", "build/x", "--legacy-look"], environment: [:])
          && LaunchGuard.legacyLookForced([app], environment: ["FLOCKBOOK_LEGACY_LOOK": "1"]),
          "legacy look: forced by --legacy-look or FLOCKBOOK_LEGACY_LOOK=1")
    check(!LaunchGuard.legacyLookForced([app, "--session", "legacy-look"], environment: ["LEGACY_LOOK": "1"]),
          "legacy look: not by a value or another variable")
    // No check window on the owner's screen: the real-window checks need the owner's leave; frames stay off every screen.
    check(LaunchGuard.realWindowRefused([app, "--test-fullscreen-real"], environment: [:]) != nil
          && LaunchGuard.realWindowRefused([app, "--test-scroll-real"], environment: ["FLOCKBOOK_VISIBLE_OK": "0"]) != nil,
          "launch guard: a real-window check without the owner's leave is refused")
    check(LaunchGuard.realWindowRefused([app, "--test-scroll-real"], environment: ["FLOCKBOOK_VISIBLE_OK": "1"]) == nil
          && LaunchGuard.realWindowRefused([app, "--test-chrome"], environment: [:]) == nil,
          "launch guard: with leave, and other checks, run")
    let screens = [CGRect(x: 0, y: 0, width: 1470, height: 956), CGRect(x: -1920, y: 0, width: 1920, height: 1080)]
    let away = LaunchGuard.offscreen(CGRect(x: -3000, y: -3000, width: 1440, height: 952), screens: screens)
    check(away == CGRect(x: -3000, y: -3000, width: 1440, height: 952), "offscreen: a frame off every screen stays")
    let moved = LaunchGuard.offscreen(CGRect(x: 221, y: 63, width: 1249, height: 820), screens: screens)
    check(!screens.contains(where: { $0.intersects(moved) }) && moved.size == CGSize(width: 1249, height: 820),
          "offscreen: a frame on a screen moves off all screens, size kept (\(moved))")
    let left = LaunchGuard.offscreen(CGRect(x: -1500, y: 10, width: 400, height: 300), screens: screens)
    check(!screens.contains(where: { $0.intersects(left) }),
          "offscreen: also off a screen left of the main one")
    check(ok(["-NSDocumentRevisionsDebugMode", "YES"]) && ok(["-psn_0_12345"]), "launch guard: system arguments (one dash)")
    check(!ok(["--snapshot-settings /tmp/dir"]), "launch guard: flag and value in one argument (zsh, unquoted variable)")
    check(!ok(["--test-sidebar "]), "launch guard: trailing space")
    check(!ok(["--session c9-input-test"]), "launch guard: any -- argument with whitespace")
    check(!ok(["--test-sidebr"]) && !ok(["--snapshot-setings", "/tmp/x"]), "launch guard: misspelt check flags")
    check(!ok(["--test"]) && !ok(["--snapshot-"]), "launch guard: bare prefixes are not flags")
    check(!ok(["--test-sidebar", "--test-nope"]), "launch guard: one bad flag among good ones")
    // Every -- argument, not only checks: a misspelt --stress, --fuzz, --hittest or --preview launches a real window too.
    check(!ok(["--stres"]) && !ok(["--hitest"]) && !ok(["--fuz", "5"]) && !ok(["--previw"]) && !ok(["--readonly"]),
          "launch guard: misspelt options")
    check(!ok(["--"]) && !ok(["--session=c9-input-test"]), "launch guard: a bare -- and the = form are not options")
    check(ok(["--stress"]) && ok(["--hittest"]) && ok(["--fuzz", "5", "--seed", "7"]) && ok(["--preview"]) && ok(["--preview-dense"]),
          "launch guard: known options")
    check(ok(["--session", "flockbook-demo", "--snapshot-demo", "docs/media", "--dark"]) && ok(["--snapshot", "x", "--light", "--save-window"]),
          "launch guard: look and window options")
    check(ok(["--session", "s", "--test-hub", "p1", "--deny", "--image", "--warm"]), "launch guard: live-check options")
    let why = LaunchGuard.problem([app, "--snapshot-settings /tmp/dir"]) ?? ""
    check(why.contains("--snapshot-settings /tmp/dir") && why.lowercased().contains("usage"), "launch guard: the usage line names the argument (\(why))")
    check(LaunchGuard.problem([app, "--test-local-only"], known: ["--test-local-only"]) == nil, "launch guard: extra known flags from the caller")
    check(LaunchGuard.problem([app, "--hc-socket", "/tmp/hc.sock", "--test-send-unconfirmed"]) == nil && LaunchGuard.problem([app, "--no-hc"]) == nil,
          "launch guard: the hc flags launch")

    // Every flag the app reads, and every check flag the scripts pass, is known: a new flag fails here until it is listed.
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
    let dirs = ["Sources/HerdrDesk", "Sources/HerdrKit", "scripts"]
    let scan = try launchFlagScan(root: root, dirs: dirs)
    let used = scan.used
    check(scan.problems.isEmpty, "launch guard: every source directory and file was read (\(scan.problems))")
    check((scan.files["Sources/HerdrDesk"] ?? 0) > 20 && (scan.files["Sources/HerdrKit"] ?? 0) > 10 && (scan.files["scripts"] ?? 0) > 3,
          "launch guard: each directory was scanned (\(scan.files))")
    check(["--snapshot-demo", "--test-update"].allSatisfy { scan.flags["scripts"]?.contains($0) == true },
          "launch guard: the scripts' check flags were found (\(scan.flags["scripts"]?.sorted() ?? []))")
    // A directory or a file the scan cannot read is a problem, not a skip.
    let missing = try launchFlagScan(root: root, dirs: ["scripts", "no-such-dir"])
    check(missing.problems.contains { $0.contains("no-such-dir") }, "launch guard: a missing directory is a problem (\(missing.problems))")
    let tmp = FileManager.default.temporaryDirectory.appendingPathComponent("launchguard-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: tmp.appendingPathComponent("src"), withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: tmp) }
    try Data([0xFF, 0xFE, 0x22]).write(to: tmp.appendingPathComponent("src/bad.swift"))
    let unreadable = try launchFlagScan(root: tmp, dirs: ["src"])
    check(unreadable.problems.contains { $0.contains("bad.swift") }, "launch guard: an unreadable file is a problem (\(unreadable.problems))")
    let prefixes: Set<String> = ["--test", "--snapshot"]   // hasPrefix tests, not flags
    // Arguments the app passes to other tools (herdr, tailscale, git, log), not its own.
    let otherTools: Set<String> = ["--abbrev-ref", "--bg", "--check", "--cols", "--json", "--last", "--predicate", "--rows", "--style", "--takeover", "--show-toplevel"]
    let unknown = used.subtracting(prefixes).subtracting(otherTools).subtracting(LaunchGuard.known).sorted()
    check(used.count > 40, "launch guard: scanned the app's flags (\(used.count) flags)")
    check(unknown.isEmpty, "launch guard: every flag the app uses is known (unknown: \(unknown))")
}

/// The flags in `dirs` under `root`. Swift: every "--x" string literal (the app reads its arguments in HerdrDesk and
/// HerdrKit). Scripts (.sh, .mjs): check flags as words. A directory or file that cannot be read is a problem.
func launchFlagScan(root: URL, dirs: [String]) throws -> (used: Set<String>, files: [String: Int], flags: [String: Set<String>], problems: [String]) {
    let swiftFlag = try NSRegularExpression(pattern: #"--[A-Za-z][A-Za-z0-9-]*"#)
    let scriptFlag = try NSRegularExpression(pattern: #"--(?:test|snapshot)[A-Za-z0-9-]*"#)
    var used = Set<String>(), files: [String: Int] = [:], flags: [String: Set<String>] = [:], problems: [String] = []
    for dir in dirs {
        let url = root.appendingPathComponent(dir)
        let names: [String]
        do { names = try FileManager.default.contentsOfDirectory(atPath: url.path) } catch {
            problems.append("\(dir): \(error.localizedDescription)"); continue
        }
        for name in names.sorted() where name.hasSuffix(".swift") || name.hasSuffix(".sh") || name.hasSuffix(".mjs") {
            let text: String
            do { text = try String(contentsOf: url.appendingPathComponent(name), encoding: .utf8) } catch {
                problems.append("\(dir)/\(name): \(error.localizedDescription)"); continue
            }
            files[dir, default: 0] += 1
            let swift = name.hasSuffix(".swift")
            for m in (swift ? swiftFlag : scriptFlag).matches(in: text, range: NSRange(text.startIndex..., in: text)) {
                let r = Range(m.range, in: text)!
                if swift {
                    guard r.lowerBound > text.startIndex, text[text.index(before: r.lowerBound)] == "\"",
                          r.upperBound < text.endIndex, text[r.upperBound] == "\"" else { continue }
                }
                used.insert(String(text[r]))
                flags[dir, default: []].insert(String(text[r]))
            }
        }
    }
    return (used, files, flags, problems)
}
