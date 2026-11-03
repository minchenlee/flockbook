import AppKit
import HerdrKit
import SwiftUI

/// Offscreen checks for herdr-core (hc) behaviour in the app. Same window rules as `ContentTests` (offscreen, no focus).
@MainActor enum HCAppTests {
    static let flags: Set<String> = ["--test-send-unconfirmed"]   // listed in LaunchGuard.headless

    static func run(_ args: [String]) {
        if args.contains("--test-send-unconfirmed") { Task { await sendUnconfirmed() } }
    }

    private static func offscreen(_ view: some View, _ size: NSSize) -> (NSWindow, NSView) {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = NSWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        return (win, host)
    }
    private static func textView(in v: NSView) -> NSTextView? {
        if let t = v as? NSTextView { return t }
        for c in v.subviews { if let t = textView(in: c) { return t } }
        return nil
    }

    /// A send past its timeout (`unconfirmed`) keeps the text out of the composer and says
    /// "sent, not confirmed"; typing clears the note. `failed` still gives the text back.
    static func sendUnconfirmed() async {
        let probe = PlaceholderProbe()
        var outcome = DeliveryOutcome.unconfirmed
        var delivered: [String] = []
        let (win, host) = offscreen(ComposerView(agentName: "testagent", onSend: { _ in }, probe: probe, deliverResult: { msg in
            try? await Task.sleep(for: .milliseconds(150))
            delivered.append(msg)
            return outcome
        }), NSSize(width: 700, height: 220))
        _ = win
        try? await Task.sleep(for: .seconds(1))
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("unconfirmed-test: \(n) \(ok ? "ok" : "FAIL")") }
        guard let tv = textView(in: host) else { print("unconfirmed-test: no text view"); NSApp.terminate(nil); return }
        win.makeFirstResponder(tv)
        tv.insertText("hello hc", replacementRange: NSRange(location: NSNotFound, length: 0))
        try? await Task.sleep(for: .milliseconds(300))
        tv.insertNewline(nil)
        try? await Task.sleep(for: .milliseconds(600))
        step("unconfirmed: text stays out of the composer (\"\(tv.string)\")", tv.string.isEmpty && delivered == ["hello hc"])
        step("unconfirmed: note says sent, not confirmed (\(probe.error ?? "none"))", probe.error?.hasPrefix("Sent, not confirmed") == true)
        tv.insertText("next", replacementRange: NSRange(location: NSNotFound, length: 0))
        try? await Task.sleep(for: .milliseconds(300))
        step("typing clears the note (\(probe.error ?? "none"))", probe.error == nil)
        outcome = .failed
        tv.insertNewline(nil)
        try? await Task.sleep(for: .milliseconds(600))
        step("failed: text comes back (\"\(tv.string)\")", tv.string == "next" && probe.error?.hasPrefix("Not sent") == true)
        print("unconfirmed-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }
}
