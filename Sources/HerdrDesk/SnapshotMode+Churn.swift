import AppKit
import HerdrKit
import SwiftUI

/// Offscreen check of a live conversation under churn (`--test-conversation-churn`): items appended at the bottom, the
/// front trimmed and grown back (short/full log depth), the composer bar changing height, the window width changing, a
/// reader scrolled up while items arrive. After each change: is the viewport drawn (not blank)? After each scenario:
/// does the scroll view come to rest (no scroll or layout loop)? And how long did the main thread stall at most?
/// Synthetic items only. Under 60 s.
extension ContentTests {
    /// The longest main-thread stall an autoload may cause (a hitch the reader sees mid-scroll). Half a page at once, every
    /// row of the window rendered again, stalled about 100 ms here; a few rows per frame, only new rows rendered, 35–50 ms
    /// (this check runs a debug build; a quiet scroll alone is about 20 ms). Each batch still lays out the whole stack once
    /// (about 34 ms here, whatever the batch size); on a CI runner that frame took 61–64 ms on macOS 26 and 158–202 ms on
    /// macOS 15. The budgets keep a margin over those; a lazy stack is the fix that would bring them down.
    static var autoloadBudget: Double {
        if #available(macOS 26, *) { return 90 }
        return 250
    }
    static func conversationChurn(_ args: [String]) async {
        // `--test-conversation-churn <rows>`: rows on screen (default 150, one page); a long session read far back has more.
        if let i = args.firstIndex(of: "--test-conversation-churn"), args.indices.contains(i + 1), let n = Int(args[i + 1]) { ConversationView.page = n }
        print("churn-test: \(ConversationView.page) rows shown of a 3000-item session")
        if args.contains("stall-meter-only") { exit(await stallMeterCheck() ? 0 : 1) }
        let placementOK = anchorPlacement()
        if args.contains("anchor-placement-only") { exit(placementOK ? 0 : 1) }
        let meterOK = await stallMeterCheck()
        var failures = (placementOK ? 0 : 1) + (meterOK ? 0 : 1)
        let t0 = Date()
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("churn-test: \(n) \(ok ? "ok" : "FAIL")") }
        // On a shared CI runner, stall times and the short-lived notice measure how busy the machine is: there a miss is
        // a warning, not a failure. A run on a Mac still counts it.
        let shared = ProcessInfo.processInfo.environment["CI"] == "true"
        func timing(_ n: String, _ ok: Bool) {
            if ok || !shared { step(n, ok) } else { print("churn-test: \(n) WARN (CI runner, not counted)") }
        }
        // The probe actions: none outside the checks, and none after the view left.
        let probe = ProbeHolder()
        let probeWin = OffscreenWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 400), styleMask: [.titled], backing: .buffered, defer: false)
        probeWin.contentView = NSHostingView(rootView: ProbeView(holder: probe)); probeWin.setFrameOrigin(NSPoint(x: -3000, y: -3000))
        probeWin.orderFrontRegardless()
        try? await Task.sleep(for: .milliseconds(500))
        step("probe: no action kept outside the checks", ConversationProbe.pressEarlier == nil)
        ConversationProbe.enabled = true
        probe.generation += 1
        try? await Task.sleep(for: .milliseconds(500))
        step("probe: a check gets the Show earlier action", ConversationProbe.pressEarlier != nil)
        probe.shown = false
        try? await Task.sleep(for: .milliseconds(500))
        step("probe: the action goes with the view", ConversationProbe.pressEarlier == nil)
        probeWin.orderOut(nil)

        let all = ChurnData.items(3000)
        let holder = ChurnHolder(items: Array(all.prefix(2800)))
        let size = NSSize(width: 1100, height: 760)
        let opened = Date()
        let host = NSHostingView(rootView: ChurnView(holder: holder))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        // CHURN_DARK=1: the whole run in dark mode (the loading notice's shot gets a -dark name).
        let dark = ProcessInfo.processInfo.environment["CHURN_DARK"] == "1"
        win.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)   // not the system's: a light run stays light
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        host.layoutSubtreeIfNeeded(); win.displayIfNeeded()
        print("churn-test: first view: laid out and drawn in \(Int(Date().timeIntervalSince(opened) * 1000)) ms (\(ConversationView.page) rows)")
        try? await Task.sleep(for: .seconds(1))

        // Selectable text: SwiftUI puts an AppKit overlay on each selectable Text and re-sets its font on every update,
        // which asks for another layout: with every row selectable a window could lay out forever. The conversation has
        // no SwiftUI selectable text: each message is one text view.
        func overlays(_ v: NSView) -> Int {
            (String(describing: type(of: v)).contains("SelectionTextField") ? 1 : 0) + v.subviews.reduce(0) { $0 + overlays($1) }
        }
        let n = overlays(host)
        step("selectable text: \(n) SwiftUI selection overlays in the conversation (want 0)", n == 0)
        // One message is one text view: selecting all of it takes every paragraph, list and code block.
        let multi = Self.all(NSTextView.self, in: host).first { !$0.isEditable && $0.string.hasPrefix("Finished step") && $0.string.contains("Paragraph 2:") }
        if let tv = multi {
            tv.setSelectedRange(NSRange(location: 0, length: (tv.string as NSString).length))
            let pb = NSPasteboard(name: NSPasteboard.Name("churn-test-\(UUID().uuidString)"))
            pb.clearContents()
            // The types Cmd-C and the menu's Copy write: plain text only, so a rich paste cannot bring back code labels or
            // raw list markers.
            _ = tv.writeSelection(to: pb, types: tv.writablePasteboardTypes)
            let copied = pb.string(forType: .string) ?? ""
            let types = pb.types ?? []
            pb.releaseGlobally()
            step("copy: plain text only (\(types.map(\.rawValue)))", !types.isEmpty && Set(types).isSubset(of: [.string, NSPasteboard.PasteboardType("NSStringPboardType")]))
            let last = tv.string.components(separatedBy: "\n").last { $0.hasPrefix("Paragraph ") }.map { String($0.prefix(13)) } ?? "?"
            step("selection: one message is one text view; all of it copies the first and the last paragraph (\(last))",
                 copied.hasPrefix("Finished step") && copied.contains(last) && copied.contains("let value = parse(line)"))
        } else { step("selection: a multi-paragraph message is one selectable text view", false) }
        copyMessage(in: host, window: win, step: step)
        // Older long replies fold to 14 lines: the text view is cut there.
        let line = (ReadingSize(rawValue: AppDefaults.store.string(forKey: ReadingSize.key) ?? "") ?? .m).line
        let folded = Self.all(MessageTextView.self, in: host).filter { $0.fold != nil }
        step("fold: \(folded.count) folded replies, each \(Int(line * 14)) pt tall (\(folded.map { Int($0.frame.height) }.prefix(4)))",
             !folded.isEmpty && folded.allSatisfy { abs($0.frame.height - line * 14) < 1 })

        // Main-thread stalls: a 5 ms timer; the longest gap between two ticks, not counting the check's own captures.
        let stall = StallMeter()
        stall.start()
        func scrollView() -> NSScrollView? {
            func views(_ v: NSView) -> [NSView] { [v] + v.subviews.flatMap(views) }
            return views(host).compactMap { $0 as? NSScrollView }.max { $0.bounds.height < $1.bounds.height }
        }
        guard let sv = scrollView() else { step("scroll view found", false); exit(1) }
        // A text view never scrolls the conversation itself. When SwiftUI resizes a text view, NSTextView's setFrameSize
        // asks the enclosing scroll view to show a rect of it (scrollToVisible). SwiftUI does not see that scroll: a lazy
        // stack can keep the rows of the old range and leave the viewport blank until the next change.
        let clip = sv.contentView.bounds
        if let tv = Self.all(MessageTextView.self, in: host).last, clip.minY > 2000 {
            // A rect well above the viewport, in the text view's coordinates.
            let asked = tv.scrollToVisible(tv.convert(NSRect(x: 0, y: clip.minY - 2000, width: 10, height: 10), from: sv.contentView))
            let moved = sv.contentView.bounds.origin.y - clip.origin.y
            if abs(moved) > 0.5 { sv.contentView.scroll(to: clip.origin); sv.reflectScrolledClipView(sv.contentView) }
            step("text view: does not scroll the conversation (moved \(Int(moved)) pt, scrolled \(asked))", abs(moved) < 0.5 && !asked)
        } else { step("text view: one in the conversation, scrolled down", false) }
        // Scroll moves: the clip view's bounds origin. Counted to see whether the view comes to rest.
        var moves = 0
        sv.contentView.postsBoundsChangedNotifications = true
        let obs = NotificationCenter.default.addObserver(forName: NSView.boundsDidChangeNotification, object: sv.contentView, queue: .main) { _ in
            MainActor.assumeIsolated { moves += 1 }
        }
        defer { NotificationCenter.default.removeObserver(obs) }

        /// Ink in the viewport between the bars: pixels away from their row's median tone, on a 3 px grid.
        func ink() -> Int {
            stall.pause(); defer { stall.resume() }   // the check's own work (capture, pixel scan) is not an app stall
            typealias Fn = @convention(c) (CGRect, UInt32, UInt32, UInt32) -> Unmanaged<CGImage>?
            guard let sym = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "CGWindowListCreateImage"),
                  let img = unsafeBitCast(sym, to: Fn.self)(.null, 8, UInt32(win.windowNumber), 1)?.takeRetainedValue(),
                  let data = img.dataProvider?.data, let p = CFDataGetBytePtr(data) else { return -1 }
            let k = CGFloat(img.width) / win.frame.width, bpr = img.bytesPerRow, bpp = img.bitsPerPixel / 8
            let titleBar = win.frame.height - (win.contentView?.frame.height ?? win.frame.height)
            let y0 = Int((titleBar + holder.topBar + 30) * k), y1 = Int((win.frame.height - holder.bottomBar - 30) * k)
            guard y1 > y0 else { return -1 }
            var n = 0
            for y in stride(from: y0, to: y1, by: 3) {
                var row: [Int] = []
                for x in stride(from: Int(40 * k), to: img.width - Int(40 * k), by: 3) {
                    let o = y * bpr + x * bpp
                    row.append(Int(p[o]) + Int(p[o + 1]) + Int(p[o + 2]))
                }
                let mid = row.sorted()[row.count / 2]
                n += row.filter { abs($0 - mid) > 90 }.count
            }
            return n
        }
        // The meter: a stall just before a capture counts; the capture itself does not.
        try? await Task.sleep(for: .milliseconds(100))
        _ = stall.take()
        usleep(300_000)   // an app stall, then a capture before the timer ticks again
        _ = ink()
        let before = stall.take()
        step("stall meter: a stall before a capture counts (\(Int(before * 1000)) ms)", before >= 0.29)
        try? await Task.sleep(for: .milliseconds(100))
        _ = stall.take()
        let c0 = Date()
        _ = ink()
        let span = Date().timeIntervalSince(c0)
        try? await Task.sleep(for: .milliseconds(50))
        let own = stall.take()
        step("stall meter: the capture's own \(Int(span * 1000)) ms does not count (\(Int(own * 1000)) ms)", own < 0.05)

        func atBottom() -> Bool {
            let b = sv.contentView.bounds, doc = sv.documentView?.frame.height ?? 0
            return doc - b.maxY <= 40 + sv.contentInsets.bottom
        }
        var blanks: [String] = []
        var passes: [String: [Double]] = [:]
        /// One change, then two checks: right after one layout pass, and 300 ms later (at rest).
        func change(_ name: String, _ i: Int, _ f: () -> Void) async {
            f()
            let t = Date()
            host.layoutSubtreeIfNeeded()
            passes[name, default: []].append(Date().timeIntervalSince(t) * 1000)
            try? await Task.sleep(for: .milliseconds(250))
            let rest = ink()
            if rest >= 0, rest < 40 { blanks.append("\(name)#\(i) rest \(rest)") }
        }
        /// The main thread's time in texts since the last scenario (markdown parse, text build, text measure) and the rows
        /// it made or sized: a stall with little of these was the stack's own layout.
        var spent = (MarkdownCache.parseTime, MessageContent.buildTime, MessageContent.measureTime, MarkdownView.bodies, MessageText.makes, MessageText.fits)
        func ms(_ d: Duration) -> Int { Int(d.components.seconds * 1000) + Int(d.components.attoseconds / 1_000_000_000_000_000) }
        /// After a scenario: the view must come to rest within 1.5 s, then stay still for 2 s.
        func settle(_ name: String, budget: Double = 250) async {
            try? await Task.sleep(for: .milliseconds(1000))
            moves = 0
            let before = sv.contentView.bounds.origin.y
            try? await Task.sleep(for: .milliseconds(1500))
            step("\(name): comes to rest (\(moves) scroll moves in 1.5 s, y \(Int(before)) -> \(Int(sv.contentView.bounds.origin.y)))", moves == 0)
            let n = ink()
            step("\(name): viewport drawn at rest (\(n) ink)", n >= 40)
            if n < 40 { stall.pause(); capture(win, to: "\(dir)/churn-\(ConversationView.page)-\(name)-blank.png"); stall.resume() }
            let worst = stall.take() * 1000, p = (passes[name] ?? []).sorted()
            let p95 = p.isEmpty ? 0 : p[min(p.count - 1, p.count * 95 / 100)]
            print("churn-test: \(name): at \(Int(Date().timeIntervalSince(t0))) s; layout pass after a change: median \(Int(p.isEmpty ? 0 : p[p.count / 2])) ms, p95 \(Int(p95)) ms")
            let now = (MarkdownCache.parseTime, MessageContent.buildTime, MessageContent.measureTime, MarkdownView.bodies, MessageText.makes, MessageText.fits)
            print("churn-test: \(name): main thread in texts: parse \(ms(now.0 - spent.0)) ms, build \(ms(now.1 - spent.1)) ms, measure \(ms(now.2 - spent.2)) ms; \(now.3 - spent.3) markdown bodies, \(now.4 - spent.4) texts made, \(now.5 - spent.5) sized")
            spent = now
            timing("\(name): longest main-thread stall \(Int(worst)) ms (under \(Int(budget)))", worst < budget)
        }

        // 1. Appends at the bottom (each new reply folds the one before it).
        var next = 2800
        for i in 0..<14 { await change("append", i) { holder.items.append(all[next]); next += 1 } }
        await settle("append")
        step("append: follows the bottom", atBottom())

        // 2. Front trimmed and grown back (log depth short <-> full), at the bottom.
        for i in 0..<8 {
            await change("depth", i) {
                if i % 2 == 0 { holder.items.removeFirst(60) } else { holder.items.insert(contentsOf: all[(next - holder.items.count - 60)..<(next - holder.items.count)], at: 0) }
            }
        }
        await settle("depth")

        // 3. Composer bar height changes (typing, an image, a question card), at the bottom.
        for i in 0..<10 { await change("composer", i) { holder.bottomBar = i % 2 == 0 ? 180 : 64 } }
        holder.bottomBar = 64
        await settle("composer")
        step("composer: still at the bottom", atBottom())

        // 4. Window width changes (sidebar, split, resize).
        for i in 0..<6 {
            await change("width", i) { win.setContentSize(NSSize(width: i % 2 == 0 ? 760 : 1100, height: size.height)) }
        }
        win.setContentSize(size)
        await settle("width")

        // 5. Reader scrolled up while items arrive and the front trims.
        let doc = sv.documentView?.frame.height ?? 0
        sv.contentView.scroll(to: NSPoint(x: 0, y: max(0, doc / 2))); sv.reflectScrolledClipView(sv.contentView)
        try? await Task.sleep(for: .milliseconds(500))
        for i in 0..<12 {
            await change("scrolled-up", i) {
                if i % 5 == 4 { holder.items.removeFirst(20) } else { holder.items.append(all[next % all.count]); next += 1 }
            }
        }
        await settle("scrolled-up")
        step("scrolled-up: the reader stays up", !atBottom())

        // 6. Everything at once, fast: appends, composer, width, depth (a busy agent while typing).
        for i in 0..<24 {
            holder.items.append(all[next % all.count]); next += 1
            if i % 3 == 0 { holder.bottomBar = holder.bottomBar == 64 ? 120 : 64 }
            if i % 7 == 0 { holder.items.removeFirst(min(30, holder.items.count - 50)) }
            if i % 11 == 0 { win.setContentSize(NSSize(width: i % 2 == 0 ? 900 : 1100, height: size.height)) }
            try? await Task.sleep(for: .milliseconds(25))
        }
        holder.bottomBar = 64; win.setContentSize(size)
        await settle("burst")

        // 7. The row window: Show earlier past the cap drops the newest rows and offers Jump to latest, which goes back.
        // Scrolled to the top for Show earlier, to the bottom for Jump to latest.
        func scroll(top: Bool) async {
            let doc = sv.documentView?.frame.height ?? 0
            let y = top ? -sv.contentInsets.top : doc + sv.contentInsets.bottom - sv.contentView.bounds.height
            sv.contentView.scroll(to: NSPoint(x: 0, y: y)); sv.reflectScrolledClipView(sv.contentView)
            try? await Task.sleep(for: .milliseconds(500))
        }
        if ConversationView.page >= ConversationWindow.maxRows {
            await scroll(top: true); await scroll(top: true)   // twice: the first pass realizes rows and refines the height
            let earlier = ConversationProbe.earlier
            step("window: Show earlier on screen at the top (\(earlier))", earlier.height > 0 && earlier.minY >= 0)
            step("window: Show earlier is pressable", ConversationProbe.pressEarlier != nil)
            ConversationProbe.pressEarlier?()
            try? await Task.sleep(for: .milliseconds(600))
            await scroll(top: false); await scroll(top: false)
            let latest = ConversationProbe.latest
            step("window: Show earlier past \(ConversationWindow.maxRows) rows offers Jump to latest (\(latest))", latest.height > 0)
            let y = sv.contentView.bounds.origin.y
            holder.items.append(all[next % all.count]); next += 1
            try? await Task.sleep(for: .milliseconds(500))
            step("window: a new row leaves the older rows in place (y \(Int(y)) -> \(Int(sv.contentView.bounds.origin.y)))", abs(sv.contentView.bounds.origin.y - y) < 1)
            ConversationProbe.pressLatest?()
            try? await Task.sleep(for: .milliseconds(600))
            step("window: Jump to latest goes back to the newest rows, at the bottom (button \(ConversationProbe.latest))", ConversationProbe.latest == .zero && atBottom())
            let n = ink()
            step("window: viewport drawn after Jump to latest (\(n) ink)", n >= 40)
        }

        // 8. The reader scrolls up to the top: older rows load by themselves (no button press), and the message that was at
        // the top of the view stays where it was (the rows come in above it, out of view). At the app's page size: a page as
        // big as the window would replace every row on screen, the one on top too.
        if ConversationView.page < ConversationWindow.maxRows {
            /// The topmost message in view: its text and its distance from the top of the view.
            func topMessage() -> (text: String, y: CGFloat)? {
                let b = sv.contentView.bounds, top = b.minY + sv.contentInsets.top
                return Self.all(MessageTextView.self, in: host)
                    .map { (String($0.string.prefix(80)), $0.convert($0.bounds, to: sv.contentView)) }
                    .filter { $0.1.maxY > top + 10 && $0.1.minY < b.maxY }
                    .min { $0.1.minY < $1.1.minY }.map { ($0.0, $0.1.minY - top) }
            }
            func y(of text: String) -> CGFloat? {
                let top = sv.contentView.bounds.minY + sv.contentInsets.top
                return Self.all(MessageTextView.self, in: host).first { String($0.string.prefix(80)) == text }.map { $0.convert($0.bounds, to: sv.contentView).minY - top }
            }
            // Just outside the zone that loads, then into it in one move: the top message is read before the load runs.
            let outside = NSPoint(x: 0, y: -sv.contentInsets.top + ConversationView.nearTop + 400)
            for _ in 0..<2 { sv.contentView.scroll(to: outside); sv.reflectScrolledClipView(sv.contentView); try? await Task.sleep(for: .milliseconds(500)) }
            _ = stall.take()
            let noticeBefore = ConversationProbe.loadingShows
            sv.contentView.scroll(to: NSPoint(x: 0, y: -sv.contentInsets.top)); sv.reflectScrolledClipView(sv.contentView)
            let before = topMessage(), count = Self.all(MessageTextView.self, in: host).count
            // Feedback: a "Loading earlier" notice over the top of the rows (it moves none of them), gone when they are in.
            // Between run-loop turns (when a frame is committed), the top message stays where it was all through the load.
            var worst: CGFloat = 0, looks = 0
            func look() { if let b = before, let d = y(of: b.text) { worst = max(worst, abs(d - b.y)); looks += 1 } else { worst = .infinity } }
            for _ in 0..<15 { try? await Task.sleep(for: .milliseconds(8)); look() }
            let pill = ConversationProbe.lastLoadingFrame, viewport = sv.convert(sv.bounds, to: nil)
            step("autoload: a Loading earlier notice was observed at the top while rows loaded (\(pill))",
                 ConversationProbe.loadingShows > noticeBefore && pill.height > 0 && pill.minY >= viewport.minY && pill.maxY <= viewport.maxY)
            stall.pause(); capture(win, to: "\(dir)/churn-\(ConversationView.page)-autoload-loading\(dark ? "-dark" : "").png"); stall.resume()
            for _ in 0..<60 { try? await Task.sleep(for: .milliseconds(8)); look() }
            step("autoload: while rows come in, the top message never moves (worst \(Int(worst)) pt over \(looks) looks)",
                 before != nil && worst < 3)
            try? await Task.sleep(for: .milliseconds(900))
            let now = Self.all(MessageTextView.self, in: host).count, loaded = now > count
            let after = before.flatMap { y(of: $0.text) }
            step("autoload: at the top, older rows load without a press (\(count) -> \(now) messages)", loaded)
            step("autoload: after the load, the top message stays in place (\(before.map { Int($0.y) } ?? -1) -> \(after.map { Int($0) } ?? -1) pt)",
                 loaded && before != nil && after != nil && abs(after! - before!.y) < 3)
            step("autoload: the notice goes when the rows are in (\(ConversationProbe.loading))", ConversationProbe.loading == .zero)
            stall.pause(); capture(win, to: "\(dir)/churn-\(ConversationView.page)-autoload.png"); stall.resume()
            await settle("autoload", budget: Self.autoloadBudget)
            ConversationProbe.pressLatest?()
            try? await Task.sleep(for: .milliseconds(600))

            /// Just outside the zone that loads, then into it in one move.
            func intoTop() async {
                let outside = NSPoint(x: 0, y: -sv.contentInsets.top + ConversationView.nearTop + 400)
                for _ in 0..<2 { sv.contentView.scroll(to: outside); sv.reflectScrolledClipView(sv.contentView); try? await Task.sleep(for: .milliseconds(500)) }
                sv.contentView.scroll(to: NSPoint(x: 0, y: -sv.contentInsets.top)); sv.reflectScrolledClipView(sv.contentView)
            }
            /// The numbers of the "Even reply" rows on hand, oldest first.
            func evens() -> [Int] {
                Self.all(MessageTextView.self, in: host).compactMap { Int($0.string.dropFirst("Even reply ".count).prefix { $0.isNumber }) }.sorted()
            }

            // 8b. A window at the cap, scrolled back (Show earlier past the cap), takes a page above and drops one below. Rows
            // of one height: the content height stays the same. Groups of five rows, a tool row at each end: the row on top of
            // the window is a tool row that merges with the older tool row above it. The top message stays in place all the same.
            holder.items = ChurnData.even(3000)
            try? await Task.sleep(for: .milliseconds(600))
            await scroll(top: false)
            ConversationProbe.pressEarlier?()
            try? await Task.sleep(for: .milliseconds(600))
            let latest = ConversationProbe.latest
            await intoTop()
            let capBefore = topMessage(), capOld = evens()
            _ = stall.take()
            try? await Task.sleep(for: .milliseconds(1500))
            let capNew = evens()
            let capLoaded = !capOld.isEmpty && !capNew.isEmpty && capNew[0] < capOld[0] && capNew.last! < capOld.last!
            let capAfter = capBefore.flatMap { y(of: $0.text) }
            step("autoload-cap: at the cap, older rows come in and newer go (Jump to latest \(latest.height > 0), rows \(capOld.first ?? -1)…\(capOld.last ?? -1) -> \(capNew.first ?? -1)…\(capNew.last ?? -1))",
                 latest.height > 0 && capLoaded)
            step("autoload-cap: the top message stays in place (\(capBefore.map { Int($0.y) } ?? -1) -> \(capAfter.map { Int($0) } ?? -1) pt)",
                 capLoaded && capBefore != nil && capAfter != nil && abs(capAfter! - capBefore!.y) < 3)
            await settle("autoload-cap", budget: Self.autoloadBudget)
            ConversationProbe.pressLatest?()
            try? await Task.sleep(for: .milliseconds(600))

            // 8b2. A send while a page of older rows is still going in: the view goes to the newest rows and stays there
            // (the rest of the page does not go in and take the window off the newest end again).
            ConversationProbe.pressEarlier?()
            try? await Task.sleep(for: .milliseconds(60))
            holder.sent += 1
            try? await Task.sleep(for: .milliseconds(1500))
            let sentRows = evens()
            step("autoload-send: a send mid-load stays at the newest rows (Jump to latest \(ConversationProbe.latest.height > 0), at bottom \(atBottom()), last row \(sentRows.last ?? -1))",
                 ConversationProbe.latest == .zero && atBottom() && sentRows.last == 2998)

            // 8e. The reader's case: the rows at hand run out, so the top reads the log, and the older rows come a moment
            // later in one go. While the reader still scrolls (a trackpad's momentum, the bounce at the top) they wait: put
            // in mid-bounce, the bounce ends at the new top and the message being read is gone. A check cannot drive a
            // trackpad here (an offscreen window gets no gesture events): it holds the view's "reader scrolls" state, which
            // the scroll phase sets in the app. Then page after page, to the cap and past it: the message on top when the
            // rows come stays where it was (2 pt at most), all through the batches and after.
            do {
                let pool = ChurnData.items(3000)
                var first = 2900
                holder.items = Array(pool[first...])
                holder.load = { @MainActor [holder] in
                    holder.reads += 1
                    Task { @MainActor in
                        try? await Task.sleep(for: .milliseconds(300))
                        let from = max(0, first - 60)
                        holder.items.insert(contentsOf: pool[from..<first], at: 0); first = from
                    }
                    return first > 60 ? .more : .end
                }
                try? await Task.sleep(for: .milliseconds(600))
                await scroll(top: false)
                func messages() -> Int { Self.all(MessageTextView.self, in: host).count }

                ConversationProbe.scrolling = true
                print("churn-trace: scrolling check starts")
                await intoTop()
                let held = topMessage(), held0 = messages()
                try? await Task.sleep(for: .milliseconds(900))
                let heldMid = held.flatMap { y(of: $0.text) }, heldN = messages(), heldPill = ConversationProbe.loading
                stall.pause(); capture(win, to: "\(dir)/churn-\(ConversationView.page)-autoload-held.png"); stall.resume()
                step("autoload-scrolling: while the reader scrolls, older rows wait and the top message stays (\(held0) -> \(heldN) messages, \(held.map { Int($0.y) } ?? -1) -> \(heldMid.map { Int($0) } ?? -1) pt, notice \(heldPill.height > 0))",
                     holder.reads == 1 && heldN == held0 && held != nil && heldMid != nil && abs(heldMid! - held!.y) <= 2 && heldPill.height > 0)
                ConversationProbe.scrolling = false
                print("churn-trace: scrolling check released")
                try? await Task.sleep(for: .milliseconds(1500))
                let heldAfter = held.flatMap { y(of: $0.text) }
                stall.pause(); capture(win, to: "\(dir)/churn-\(ConversationView.page)-autoload-held-in.png"); stall.resume()
                step("autoload-scrolling: when the scroll stops they go in, and the top message stays (\(held0) -> \(messages()) messages, \(held.map { Int($0.y) } ?? -1) -> \(heldAfter.map { Int($0) } ?? -1) pt)",
                     messages() > held0 && held != nil && heldAfter != nil && abs(heldAfter! - held!.y) <= 2)

                // Page after page from the top: read from the log, from the rows at hand, at the cap and past it.
                ConversationProbe.pressLatest?()
                try? await Task.sleep(for: .milliseconds(600))
                first = 2900; holder.items = Array(pool[first...]); holder.reads = 0
                try? await Task.sleep(for: .milliseconds(600))
                await scroll(top: false)
                for page in 1...4 {
                    await intoTop()
                    let before = topMessage()
                    try? await Task.sleep(for: .milliseconds(2000))
                    let after = before.flatMap { y(of: $0.text) }
                    step("autoload-log: page \(page) (\(holder.reads) log reads, Jump to latest \(ConversationProbe.latest.height > 0)): the top message stays (\(before.map { Int($0.y) } ?? -1) -> \(after.map { Int($0) } ?? -1) pt)",
                         before != nil && after != nil && abs(after! - before!.y) <= 2)
                }
                step("autoload-log: the pages read the log and went past the cap (\(holder.reads) reads, Jump to latest \(ConversationProbe.latest.height > 0))",
                     holder.reads >= 2 && ConversationProbe.latest.height > 0)

                // 8i. The same pages on a busy main thread: every 100 ms it is held for 200-500 ms (a busy Mac, a long
                // layout pass elsewhere), while the rows come in batch by batch. The message on top stays where it was.
                ConversationProbe.pressLatest?()
                try? await Task.sleep(for: .milliseconds(600))
                first = 2900; holder.items = Array(pool[first...]); holder.reads = 0
                try? await Task.sleep(for: .milliseconds(600))
                await scroll(top: false)
                for (page, hold) in [200, 300, 400, 500].enumerated() {
                    await intoTop()
                    let before = topMessage()
                    let busy = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { _ in Thread.sleep(forTimeInterval: Double(hold) / 1000) }
                    // And the other cores busy too, as on a loaded Mac: threads that spin until the page is in.
                    let spin = SpinLoad(threads: ProcessInfo.processInfo.activeProcessorCount * 2)
                    try? await Task.sleep(for: .milliseconds(2500))
                    busy.invalidate(); spin.stop()
                    try? await Task.sleep(for: .milliseconds(1000))
                    let after = before.flatMap { y(of: $0.text) }
                    step("autoload-busy: page \(page + 1), main thread held \(hold) ms each 100 ms: the top message stays (\(before.map { Int($0.y) } ?? -1) -> \(after.map { Int($0) } ?? -1) pt)",
                         before != nil && after != nil && abs(after! - before!.y) <= 2)
                }
                _ = stall.take()

                // 8f. A fast fling up, from 3000 pt below the top at 6000 pt/s, slowing down at 6000 pt/s² (it would stop at
                // the top), the rows at hand all shown: the read of the log (300 ms here, the slowest of a 30 MB log) starts
                // early enough that the rows go in while the view still glides, far from the top, and the glide goes on over
                // them. The rows in view move only with the glide (2 pt at most). The glide moves from where the view is at
                // each step, as a trackpad's momentum does; the view's "reader scrolls" state is held all the while.
                // The same fling with every core busy (a loaded Mac, a CI runner): rows can be placed in a frame where the
                // anchor row's own view gets no frame change (a parent moves), so the fix must not wait for one.
                for busy in [false, true] {
                    let name = busy ? "-busy" : ""
                    ConversationProbe.pressLatest?()
                    try? await Task.sleep(for: .milliseconds(600))
                    first = 2900; holder.items = Array(pool[first...]); holder.reads = 0
                    try? await Task.sleep(for: .milliseconds(600))
                    await scroll(top: false)
                    let top0 = -sv.contentInsets.top
                    for _ in 0..<2 { sv.contentView.scroll(to: NSPoint(x: 0, y: top0 + 3000)); sv.reflectScrolledClipView(sv.contentView); try? await Task.sleep(for: .milliseconds(500)) }
                    func docHeight() -> CGFloat { sv.documentView?.frame.height ?? 0 }
                    ConversationProbe.scrolling = true
                    let spin = busy ? SpinLoad(threads: ProcessInfo.processInfo.activeProcessorCount * 2) : nil
                    ConversationProbe.lateFixes = 0
                    let fAt = topMessage(), h0 = docHeight(), t0 = Date()
                    var v: CGFloat = 6000, glided: CGFloat = 0, last = t0, asked: Date?, from: CGFloat = -1, loadMs = -1, inAt: CGFloat = -1
                    var hitTop = false, hitBeforeIn = false, drift: CGFloat = 0, longest: TimeInterval = 0, prevY: CGFloat = 3000, sinceIn = 0
                    while v > 0, !hitTop, let at = fAt {
                        try? await Task.sleep(for: .milliseconds(16))
                        let now = Date(), dt = min(now.timeIntervalSince(last), 0.05)
                        longest = max(longest, now.timeIntervalSince(last)); last = now
                        let y0 = sv.contentView.bounds.minY
                        defer { prevY = sv.contentView.bounds.minY - top0 }
                        if asked == nil, holder.reads > 0 { asked = now; from = y0 - top0 }
                        if inAt < 0, docHeight() > h0 + 1 { inAt = prevY; loadMs = Int(now.timeIntervalSince(asked ?? t0) * 1000) }
                        let dy = min(v * CGFloat(dt), y0 - top0)
                        sv.contentView.scroll(to: NSPoint(x: 0, y: y0 - dy)); sv.reflectScrolledClipView(sv.contentView)
                        glided += y0 - sv.contentView.bounds.minY
                        if let d = y(of: at.text) { drift = max(drift, abs(d - (at.y + glided))) }
                        if sv.contentView.bounds.minY <= top0 + 0.5 { hitTop = true; if inAt < 0 { hitBeforeIn = true } }
                        v -= 6000 * CGFloat(dt)
                        // A few frames after the rows went in, the view still gliding: the capture's own time is not a step.
                        if inAt >= 0 { sinceIn += 1 }
                        if sinceIn == 3, v > 0, !hitTop {
                            stall.pause(); capture(win, to: "\(dir)/churn-\(ConversationView.page)-autoload-fling\(name)-mid.png"); stall.resume(); last = Date()
                        }
                    }
                    stall.pause(); capture(win, to: "\(dir)/churn-\(ConversationView.page)-autoload-fling\(name).png"); stall.resume()
                    let late = ConversationProbe.lateFixes
                    spin?.stop()
                    ConversationProbe.scrolling = false
                    try? await Task.sleep(for: .milliseconds(1500))
                    stall.pause(); capture(win, to: "\(dir)/churn-\(ConversationView.page)-autoload-fling\(name)-rest.png"); stall.resume()
                    step("autoload-fling\(name): a fast fling up\(busy ? ", the cores busy" : ""): older rows go in before the top and the glide goes on over them (read from \(Int(from)) pt, rows in after \(loadMs) ms, \(Int(inAt)) pt from the top just before, top met \(hitTop)\(hitBeforeIn ? " before the rows" : ""), drift \(Int(drift.rounded(.up))) pt, \(late) moves a frame late, longest step \(Int(longest * 1000)) ms)",
                         fAt != nil && from > 0 && inAt > 0 && !hitBeforeIn && !hitTop && drift <= 2 && late == 0)
                }
            }
            holder.load = nil; holder.reads = 0
            ConversationProbe.pressLatest?()
            try? await Task.sleep(for: .milliseconds(600))

            // 8g. A window at the cap (Show earlier went past it: the newest rows are out), the reader at its bottom, and a
            // short flick up so fast the load starts at once (800 pt, then it stops). Each batch at the cap drops as many
            // rows from the bottom, faster than this glide moves: rows still in view must not go. A message in view stays
            // until the glide takes it out (it moves only with the glide, 2 pt at most). Near the top later, the older rows
            // go in.
            holder.items = ChurnData.even(3000)
            try? await Task.sleep(for: .milliseconds(600))
            ConversationProbe.pressLatest?()
            try? await Task.sleep(for: .milliseconds(600))
            ConversationProbe.pressEarlier?()
            try? await Task.sleep(for: .milliseconds(1500))
            await scroll(top: false)
            try? await Task.sleep(for: .milliseconds(600))
            do {
                let top0 = -sv.contentInsets.top
                func inView() -> [String: CGFloat] {
                    let b = sv.contentView.bounds, top = b.minY + sv.contentInsets.top, bottom = b.maxY - sv.contentInsets.bottom
                    var out: [String: CGFloat] = [:]
                    for v in Self.all(MessageTextView.self, in: host) {
                        let r = v.convert(v.bounds, to: sv.contentView)
                        if r.minY >= top && r.maxY <= bottom { out[String(v.string.prefix(80))] = r.minY - top }
                    }
                    return out
                }
                let dist = sv.contentView.bounds.minY - top0, oldest0 = evens().first ?? -1
                let v0: CGFloat = 12000, decel = v0 * v0 / (2 * 800)
                let viewH = sv.contentView.bounds.height - sv.contentInsets.top - sv.contentInsets.bottom
                ConversationProbe.scrolling = true
                var v = v0, last = Date(), seen = inView(), gone = 0, worst: CGFloat = 0, hitTop = false, stopped: Date?
                // The glide, then 600 ms at rest (the view still in the reader's hands): the batches go on meanwhile.
                while !hitTop, stopped.map({ Date().timeIntervalSince($0) < 0.6 }) ?? true {
                    try? await Task.sleep(for: .milliseconds(16))
                    let now = Date(), dt = min(now.timeIntervalSince(last), 0.05); last = now
                    // What was in view at the last step, where the glide alone puts it now (the rows came in, if any).
                    let y0 = sv.contentView.bounds.minY
                    let dy = min(v * CGFloat(dt), y0 - top0)
                    sv.contentView.scroll(to: NSPoint(x: 0, y: y0 - dy)); sv.reflectScrolledClipView(sv.contentView)
                    let moved = y0 - sv.contentView.bounds.minY
                    try? await Task.sleep(for: .milliseconds(1))
                    let now2 = inView(), all = Dictionary(Self.all(MessageTextView.self, in: host).map {
                        (String($0.string.prefix(80)), $0.convert($0.bounds, to: sv.contentView).minY - sv.contentView.bounds.minY - sv.contentInsets.top)
                    }, uniquingKeysWith: { a, _ in a })
                    for (text, y) in seen {
                        let want = y + moved
                        guard want >= 0, want <= viewH - 40 else { continue }   // the glide takes it out
                        if let got = all[text] { worst = max(worst, abs(got - want)) } else { gone += 1 }
                    }
                    seen = now2
                    if sv.contentView.bounds.minY <= top0 + 0.5 { hitTop = true }
                    v = max(0, v - decel * CGFloat(dt))
                    if v == 0, stopped == nil { stopped = Date() }
                }
                ConversationProbe.scrolling = false
                try? await Task.sleep(for: .milliseconds(1500))
                let oldest1 = evens().first ?? -1
                step("autoload-cap-fling: at the cap, from its bottom, a flick up at \(Int(v0)) pt/s (\(Int(dist)) pt to the top): no message in view goes or moves past the glide (\(gone) gone, worst \(Int(worst.rounded(.up))) pt)",
                     gone == 0 && worst <= 2)
                await intoTop()
                try? await Task.sleep(for: .milliseconds(1500))
                let oldest2 = evens().first ?? -1
                step("autoload-cap-fling: near the top later, the older rows go in (oldest \(oldest0) -> \(oldest1) after the flick -> \(oldest2) at the top)",
                     oldest2 >= 0 && oldest2 < oldest0)
            }
            ConversationProbe.pressLatest?()
            try? await Task.sleep(for: .milliseconds(600))

            // 8h. A window at the cap whose rows all fit in view: tool rows merge into a few short groups, so each batch at
            // the cap drops rows in view. The reader is at rest and presses Show earlier: the older rows go in all the same.
            // The notes before 2800 are long: the content then outgrows the view. The message on top stays in place, or, when
            // the rows that stay below it (newer ones go at the cap) cannot fill the view, the view goes no farther than its end.
            holder.items = ChurnData.chips(3000, longBefore: 2800)
            try? await Task.sleep(for: .milliseconds(600))
            ConversationProbe.pressLatest?()
            try? await Task.sleep(for: .milliseconds(600))
            do {
                func notes() -> [Int] {
                    Self.all(MessageTextView.self, in: host).compactMap { Int($0.string.dropFirst("Chip note ".count).prefix { $0.isNumber }) }.sorted()
                }
                ConversationProbe.pressEarlier?()
                try? await Task.sleep(for: .milliseconds(1500))
                let fits = (sv.documentView?.frame.height ?? .infinity) <= sv.contentView.bounds.height
                let oldest1 = notes().first ?? -1, before = topMessage()
                ConversationProbe.pressEarlier?()
                try? await Task.sleep(for: .milliseconds(1500))
                let oldest2 = notes().first ?? -1, after = before.flatMap { y(of: $0.text) }
                let doc = sv.documentView?.frame.height ?? 0, clip = sv.contentView.bounds
                let grew = doc > clip.height
                // Where the view would have to be for the message to stay, and where it can go at most.
                let need = clip.minY + (after ?? 0) - (before?.y ?? 0), end = doc + sv.contentInsets.bottom - clip.height
                let stays = before != nil && after != nil && (abs(after! - before!.y) < 3 || after! > before!.y && abs(clip.minY - end) < 1 && need > end)
                step("autoload-cap-short: at the cap, every row in view, at rest: Show earlier puts older rows in (fits \(fits), oldest note \(oldest1) -> \(oldest2))",
                     fits && oldest1 >= 0 && oldest2 >= 0 && oldest2 < oldest1)
                step("autoload-cap-short: long older notes outgrow the view, and the message on top stays in place or the view is at its end (grew \(grew), \(before?.text.prefix(16) ?? "none") \(before.map { Int($0.y) } ?? -1) -> \(after.map { Int($0) } ?? -1) pt, view at \(Int(clip.minY)), needs \(Int(need)), ends at \(Int(end)))",
                     grew && stays)
            }
            ConversationProbe.pressLatest?()
            try? await Task.sleep(for: .milliseconds(600))

            // 8c. Past the oldest row of the log, near the top again: no more reads of the log.
            holder.items = ChurnData.even(120)
            holder.load = { @MainActor [holder] in holder.reads += 1; return .end }
            try? await Task.sleep(for: .milliseconds(600))
            for _ in 0..<3 { await intoTop(); try? await Task.sleep(for: .milliseconds(900)) }
            step("autoload-end: after the log said nothing older, no more reads (\(holder.reads), want 1)", holder.reads == 1)

            // 8d. Another session in the same pane (the view stays): its older rows read again. It starts with no rows (the
            // empty view, not the list, sees the switch), then its rows come. Fewer rows than a page, so "Show earlier" is a
            // read of the log.
            holder.items = []
            holder.session = "churn-b"
            try? await Task.sleep(for: .milliseconds(600))
            holder.items = ChurnData.even(30)
            holder.reads = 0
            holder.load = { @MainActor [holder] in holder.reads += 1; return .more }
            try? await Task.sleep(for: .milliseconds(600))
            ConversationProbe.pressEarlier?()
            try? await Task.sleep(for: .milliseconds(600))
            step("autoload-session: another session in the pane reads its older rows (\(holder.reads), want 1)", holder.reads == 1)
            holder.load = nil
            holder.session = "churn-a"
        }

        step("no blank viewport at rest after a change (\(blanks.count): \(blanks.prefix(6)))", blanks.isEmpty)
        stall.stop()
        win.orderOut(nil)
        print("churn-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        exit(failures == 0 ? 0 : 1)
    }
}

@MainActor @Observable private final class ChurnHolder {
    var items: [ConversationItem]
    var bottomBar: CGFloat = 64
    var topBar: CGFloat = 28
    /// The view's read of older log rows (nil: rows in memory only).
    var load: (() async -> EarlierLoadResult)?
    var reads = 0
    var session: String? = "churn-a"
    /// Counts the reader's sends (the view goes back to the newest rows on each).
    var sent = 0
    init(items: [ConversationItem]) { self.items = items }
}

/// The pane's shape: ConversationView with a composer-like bar under it (PaneView uses paneBar the same way).
private struct ChurnView: View {
    let holder: ChurnHolder
    var body: some View {
        ConversationView(items: holder.items, agentName: "churn", loadState: .ready, scrollToken: holder.sent, session: holder.session, onLoadEarlier: holder.load)
            .paneBar(.top) { Text("churn · label").font(.caption).frame(maxWidth: .infinity).frame(height: 28) }
            .paneBar(.bottom) {
                // The real composer (a text view that grows with its text), plus room for a taller bar.
                VStack(spacing: 0) {
                    ComposerView(agentName: "churn", draft: ComposerDraft(text: "A draft reply"), onSend: { _ in })
                    Color.clear.frame(height: max(0, holder.bottomBar - 64))
                }
            }
            .softScrollEdges([.top, .bottom])
    }
}

@MainActor @Observable private final class ProbeHolder { var shown = true; var generation = 0 }

/// A short conversation with a Show earlier button on screen.
private struct ProbeView: View {
    let holder: ProbeHolder
    var body: some View {
        if holder.shown {
            ConversationView(items: Array(ChurnData.items(3)), agentName: "probe", loadState: .ready, onLoadEarlier: { .end })
                .id(holder.generation)
        }
    }
}

/// Longest gap between ticks of a 5 ms main-thread timer.
@MainActor final class StallMeter {
    private var timer: Timer?
    private var last = Date()
    private var worst: TimeInterval = 0
    private var paused = false
    func start() {
        paused = false
        last = Date()
        timer = Timer.scheduledTimer(withTimeInterval: 0.005, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let now = Date()
                if !self.paused { self.worst = max(self.worst, now.timeIntervalSince(self.last)) }
                self.last = now
            }
        }
        RunLoop.main.add(timer!, forMode: .common)
    }
    func take() -> TimeInterval { defer { worst = 0 }; return worst }
    /// The check's own work starts: the gap so far (an app stall before it) counts, the work's own span does not.
    func pause() {
        if !paused { worst = max(worst, Date().timeIntervalSince(last)) }
        paused = true
    }
    func resume() { last = Date(); paused = false }
    func stop() { timer?.invalidate() }
}

/// Synthetic conversation: user turns, replies of mixed length (paragraphs, lists, code), tool chips between.
enum ChurnData {
    /// Rows of one height, no times: groups of five (a tool row, three one-line replies, a tool row), so two tool rows
    /// meet at each group's edge and merge into one.
    static func even(_ n: Int) -> [ConversationItem] {
        (0..<n).map { i in
            i % 5 == 0 || i % 5 == 4
                ? ConversationItem(id: "et\(i)", kind: .toolChips([ToolChip(symbol: "doc.text", text: "Read a file",
                                                                       id: UUID(uuidString: String(format: "00000000-0000-0000-0001-%012d", i))!)]))
                : ConversationItem(id: "ea\(i)", kind: .assistantText("Even reply \(i)."))
        }
    }

    /// Tool rows with a note every 50th: the tool rows between two notes merge into one row, so a window at the cap is a
    /// few short rows. The notes before `longBefore` are long.
    static func chips(_ n: Int, longBefore: Int = 0) -> [ConversationItem] {
        (0..<n).map { i in
            i % 50 == 0
                ? ConversationItem(id: "cn\(i)", kind: .assistantText("Chip note \(i)." + (i < longBefore
                    ? (1...40).map { "\n\nOlder line \($0) of a long note." }.joined() : "")))
                : ConversationItem(id: "ct\(i)", kind: .toolChips([ToolChip(symbol: "doc.text", text: "Read a file",
                                                                       id: UUID(uuidString: String(format: "00000000-0000-0000-0002-%012d", i))!)]))
        }
    }

    static func items(_ n: Int) -> [ConversationItem] {
        var out: [ConversationItem] = []
        var t = Date(timeIntervalSince1970: 1_800_000_000)
        for i in 0..<n {
            t += 37
            switch i % 4 {
            case 0: out.append(ConversationItem(id: "u\(i)", kind: .userText("Step \(i): check the parser and report what changed."), timestamp: t))
            case 1, 2:
                let calls = (0..<(1 + i % 3)).map { ToolCall(id: "c\(i)-\($0)", toolName: "Read", summary: "Sources/File\($0).swift", input: "{}", output: "ok") }
                out.append(ConversationItem(id: "t\(i)", kind: .toolChips([ToolChip(symbol: "doc.text", text: "Read \(calls.count) files", calls: calls,
                                                                                     id: UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", i))!)]), timestamp: t))
            default:
                let paras = 1 + (i * 7) % 9
                var s = "Finished step \(i). "
                for p in 0..<paras {
                    s += "\n\nParagraph \(p): the reader shows this text wrapped to the column width, with **bold** and `code` spans, long enough to take several lines in the reading column."
                    if p % 3 == 1 { s += "\n\n- first point\n- second point with more words\n- third" }
                    if p % 4 == 2 { s += "\n\n```swift\nlet value = parse(line)\nprint(value)\n```" }
                }
                out.append(ConversationItem(id: "a\(i)", kind: .assistantText(s), timestamp: t))
            }
        }
        return out
    }
}

extension ContentTests {
    /// The right-click menu of a message: "Copy Message" after Copy (and Copy Code on a code block), and what it writes:
    /// the message's markdown as it was written, with nothing selected. On a named pasteboard: a check never writes
    /// the owner's clipboard.
    static func copyMessage(in host: NSView, window win: NSWindow, step: (String, Bool) -> Void) {
        func menu(_ tv: MessageTextView, at p: NSPoint) -> [String] {
            let e = NSEvent.mouseEvent(with: .rightMouseDown, location: tv.convert(p, to: nil), modifierFlags: [], timestamp: 0,
                                       windowNumber: win.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)
            return e.flatMap { tv.menu(for: $0) }?.items.map { $0.isSeparatorItem ? "-" : $0.title } ?? []
        }
        func written(_ tv: MessageTextView) -> String? {
            let pb = NSPasteboard(name: NSPasteboard.Name("copy-message-test-\(UUID().uuidString)"))
            defer { pb.releaseGlobally() }
            return tv.writeMessage(to: pb) ? pb.string(forType: .string) : nil
        }
        /// A point in the middle of the first line that has `text`.
        func point(_ tv: MessageTextView, of text: String) -> NSPoint? {
            let r = (tv.string as NSString).range(of: text)
            guard r.location != NSNotFound, let lm = tv.layoutManager, let c = tv.textContainer else { return nil }
            let box = lm.boundingRect(forGlyphRange: lm.glyphRange(forCharacterRange: r, actualCharacterRange: nil), in: c)
            return NSPoint(x: box.midX + tv.textContainerOrigin.x, y: box.midY + tv.textContainerOrigin.y)
        }
        let views = all(MessageTextView.self, in: host)
        // An assistant reply with a list and a code block.
        if let tv = views.first(where: { $0.string.hasPrefix("Finished step") && $0.string.contains("let value = parse(line)") }),
           let prose = point(tv, of: "Finished step"), let code = point(tv, of: "let value = parse(line)") {
            tv.setSelectedRange(NSRange(location: 0, length: 0))
            let onProse = menu(tv, at: prose), onCode = menu(tv, at: code)
            step("copy message: the menu on a reply's text (\(onProse))", onProse == ["Copy", "Copy Message", "-", "Select All"])
            step("copy message: on a code block it comes after Copy Code (\(onCode))", onCode == ["Copy", "Copy Code", "Copy Message", "-", "Select All"])
            let md = written(tv) ?? ""
            step("copy message: the reply's markdown as written, with nothing selected (\(md.count) characters)",
                 md.hasPrefix("Finished step") && md.contains("with **bold** and `code` spans") && md.contains("\n- first point\n")
                 && md.contains("```swift\nlet value = parse(line)\nprint(value)\n```"))
            // The plain Copy of the whole text has no markdown marks: the two are not the same text.
            step("copy message: not the plain text of Copy", md != tv.plainText(NSRange(location: 0, length: (tv.string as NSString).length)))
        } else { step("copy message: a reply with a code block is on screen", false) }
        // The owner's own message.
        if let tv = views.first(where: { $0.string.hasPrefix("Step ") && $0.string.hasSuffix("report what changed.") }) {
            let titles = menu(tv, at: NSPoint(x: 4, y: 4)), md = written(tv) ?? ""
            step("copy message: a user message has it too (\(titles)) and gives its text (\(md))",
                 titles == ["Copy", "Copy Message", "-", "Select All"] && md == tv.string)
        } else { step("copy message: a user message is on screen", false) }
        // Plain text (tool output, a command) is no message.
        let plain = MessageTextView.make()
        plain.load(.plain("ls -la", monoSize: 12))
        plain.frame = NSRect(x: 0, y: 0, width: 300, height: 40)
        step("copy message: none on tool output (\(menu(plain, at: NSPoint(x: 4, y: 4))))",
             menu(plain, at: NSPoint(x: 4, y: 4)) == ["Copy", "-", "Select All"] && written(plain) == nil)
    }
}

/// Threads that keep the cores busy until stopped.
final class SpinLoad: @unchecked Sendable {
    private let lock = NSLock()
    private var on = true
    init(threads: Int) {
        for _ in 0..<threads {
            Thread.detachNewThread { [self] in
                var x = 0.0
                while self.running { for i in 0..<10_000 { x += sin(Double(i)) } }
                _ = x
            }
        }
    }
    private var running: Bool { lock.lock(); defer { lock.unlock() }; return on }
    func stop() { lock.lock(); on = false; lock.unlock() }
}
