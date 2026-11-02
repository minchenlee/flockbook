import Foundation
import CoreGraphics

/// The app's flags, and the launch guard that refuses a -- argument it does not know. A flag the app does not match
/// starts a normal launch: a real window, a Dock icon, a focus change. A misspelt flag, or a flag and its value passed as
/// one argument (zsh does not split an unquoted variable), must stop before any of that.
public enum LaunchGuard {
    /// Checks that never show a window, a Dock icon, or take focus (HerdrDeskApp).
    public static let headless: Set<String> = [
        "--headless", "--snapshot", "--snapshot-reading", "--snapshot-log", "--snapshot-queued", "--snapshot-statusline", "--snapshot-live", "--snapshot-demo", "--snapshot-settings",
        "--snapshot-remote", "--stress", "--hittest", "--test-approve", "--test-notice", "--test-hub", "--test-remote", "--test-remote-live",
        "--serve-remote", "--test-terminal", "--test-scroll", "--test-bottom", "--test-table", "--test-closetab",
        "--test-menus", "--test-tabdrag", "--test-chrome", "--test-sheet", "--test-panemenu", "--test-wheel",
        "--test-send-image", "--test-composer", "--fuzz", "--test-sidebar", "--test-sidebar-layout", "--test-root-width",
        "--test-newtab-live", "--test-settings-migration", "--test-island-height", "--test-tab-overflow", "--test-tabkeys", "--test-update",
        "--test-settings-page", "--test-send-unconfirmed",
    ]
    /// Content-area checks (ContentTests), also headless.
    public static let content: Set<String> = [
        "--test-earlier-failure", "--test-sendfail", "--test-loadstate", "--test-termfail", "--test-termtheme", "--test-glyph", "--test-nerdglyph", "--test-panebars", "--test-surfaces", "--test-search",
        "--test-textsize", "--test-scrolltop", "--test-shortcuts", "--test-question", "--test-permission", "--test-screencolumn",
        "--test-question-live", "--bench-conversation", "--test-conversation-churn", "--test-busy-tab",
        "--test-open-latest", "--test-open-latest-live", "--test-open-latest-log",
        "--test-legacy-look", "--test-pictures", "--test-dragroute",
    ]
    /// The checks with a real window, run only with the owner's leave.
    public static let realWindow: Set<String> = ["--test-fullscreen-real", "--test-scroll-real"]
    /// Options that change a launch or a check: the session, the look, the preview stores, the fuzz seed, live-check modes.
    public static let options: Set<String> = [
        "--session", "--read-only", "--light", "--dark", "--save-window", "--preview", "--preview-dense", "--seed",
        "--deny", "--image", "--warm", "--other", "--legacy-look", "--remote", "--remote-local",
        "--hc-socket", "--no-hc",
    ]
    public static let known: Set<String> = headless.union(content).union(realWindow).union(options)

    /// Nil: launch. Otherwise the usage line to print before exit(2). `known` adds flags to the app's own.
    public static func problem(_ arguments: [String], known extra: Set<String> = []) -> String? {
        for arg in arguments.dropFirst() {
            let flag = arg.hasPrefix("--")
            if flag, arg.contains(where: \.isWhitespace) {
                return usage("“\(arg)” has whitespace in it: pass a flag and its value as separate arguments")
            }
            if flag, !known.contains(arg), !extra.contains(arg) {
                return usage("unknown flag “\(arg)”")
            }
        }
        return nil
    }

    /// The real-window checks take the owner's screen and focus: refused unless FLOCKBOOK_VISIBLE_OK=1 (the owner's leave).
    public static func realWindowRefused(_ arguments: [String], environment: [String: String]) -> String? {
        guard let flag = arguments.first(where: realWindow.contains), environment["FLOCKBOOK_VISIBLE_OK"] != "1" else { return nil }
        return "\(flag) opens a real window on the owner's screen: run it only with the owner's leave, FLOCKBOOK_VISIBLE_OK=1. Nothing was opened."
    }

    /// Where a check's window may be: a frame that touches a screen moves left of every screen (size kept), so no check
    /// window ever shows over what the owner is looking at.
    public static func offscreen(_ r: CGRect, screens: [CGRect]) -> CGRect {
        guard screens.contains(where: { $0.intersects(r) }) else { return r }
        let left = screens.map(\.minX).min() ?? 0
        return CGRect(x: left - r.width - 3000, y: -3000, width: r.width, height: r.height)
    }

    /// The pre-26 look forced on (macOS 15 support: `--legacy-look` or FLOCKBOOK_LEGACY_LOOK=1), to see it on 26.
    public static func legacyLookForced(_ arguments: [String], environment: [String: String]) -> Bool {
        arguments.contains("--legacy-look") || environment["FLOCKBOOK_LEGACY_LOOK"] == "1"
    }

    private static func usage(_ what: String) -> String {
        "usage: HerdrDesk [--session <name>] [<flag> [<value>]]: \(what). Nothing was opened."
    }
}
