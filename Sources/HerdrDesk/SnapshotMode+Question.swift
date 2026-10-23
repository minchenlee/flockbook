import AppKit
import HerdrKit
import SwiftUI

/// Offscreen check for the AskUserQuestion card (`--test-question`, `--light` for the light pictures). Nothing is sent to
/// herdr: the card's send path is only driven with a local closure.
extension ContentTests {
    private static let agent = "orchard-web-dev"

    private static func single(_ id: String = "q-single") -> QuestionRequest {
        QuestionRequest(toolUseId: id, questions: [
            .init(header: "Autosave", text: "Where should drafts be saved?", options: [
                .init(label: "Local file", detail: "Written every 2 s to Application Support. Survives a crash.", recommended: true),
                .init(label: "UserDefaults", detail: "Simplest, but slow for long drafts."),
                .init(label: "Memory only", detail: "Gone when the app quits."),
            ]),
        ])
    }
    private static let restore = QuestionRequest.Question(header: "Restore", text: "After a restart, what should come back?", options: [
        .init(label: "Unsent replies", detail: "Half-typed text in the composer."),
        .init(label: "Scroll position", detail: "Back to where you were reading."),
        .init(label: "Open tabs", detail: "The agent tabs that were open."),
        .init(label: "Sidebar state", detail: "Which workspaces were expanded."),
    ], multiSelect: true)
    private static func multi() -> QuestionRequest { QuestionRequest(toolUseId: "q-multi", questions: [restore]) }
    private static func three() -> QuestionRequest {
        QuestionRequest(toolUseId: "q-three", questions: [
            .init(header: "Sign-in", text: "How should people sign in?", options: [
                .init(label: "Passkey", detail: "No password. Needs macOS 14 or later.", recommended: true),
                .init(label: "Email link", detail: "A one-time link by mail."),
                .init(label: "Password", detail: "Familiar, but needs a reset flow."),
            ]),
            restore,
            .init(header: "Ship", text: "When should it ship?", options: [.init(label: "This week"), .init(label: "Next sprint")]),
        ])
    }
    private static func items(_ q: QuestionRequest?) -> [ConversationItem] {
        var out = [ConversationItem(kind: .userText("Finish draft autosave and tell me when it's done.")),
                   ConversationItem(kind: .assistantText("The autosave skeleton is in and all 18 tests pass. One decision is yours: where drafts are stored."))]
        if let q { out.append(ConversationItem(kind: .question(q))) }
        return out
    }

    /// Claude Code's picker as the owner's pane showed it (user-shot-55): one question, four options with a description line.
    static let tallScreen = """
    ❯ 模擬選項，我想看一下你的選項介面
    ───────────────────────────────────────────────────────────────────────
     ☐ 筆記格式

    這是模擬選項介面。您想先用哪一種方式整理一份新筆記？

    ❯ 1. Markdown 純文字 (推薦)
         適合版本控制與後續編輯，最省事。
      2. HTML 頁面
         適合互動瀏覽、比較多個方案，可分享給別人看。
      3. 流程圖
         適合說明步驟、流程或元件關係，一眼看出順序。
      4. 表格
         適合需要逐項核對、引用或更新的資料。
      5. Type something.
    ───────────────────────────────────────────────────────────────────────
      6. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    """
    /// A long session like the owner's: many turns (so "Show earlier" shows), CJK markdown, then the question.
    private static func longItems() -> [ConversationItem] {
        var out: [ConversationItem] = []
        for k in 0..<200 {
            out.append(ConversationItem(kind: .userText("第 \(k) 輪：給我幾個選項吧")))
            out.append(ConversationItem(kind: .assistantText("目前對話中沒有可以接續的主題，我不知道您要哪方面的選項。請告訴我：\n\n1. **要做什麼？** 例如：某個專案的下一步、一段程式的實作方式。\n2. **在哪裡？** 例如：某個 repo、某個資料夾。\n3. **選項要用來做什麼決定？** 例如：選技術、排優先順序。\n\n您只要回答一句，我就會列出幾個選項。")))
        }
        out.append(ConversationItem(kind: .userText("模擬選項，我想看一下你的選項介面")))
        return out
    }

    /// PaneView's agent layout: conversation, and the composer slot holding the card while a question waits.
    private static func scene(_ items: [ConversationItem], status: AgentStatus = .blocked, canSend: Bool = false,
                              onFlow: @escaping (QuestionFlow) -> Void = { _ in }, onSend: @escaping ([QuestionAnswer]) async throws -> Void = { _ in },
                              onLoadAll: @escaping () async throws -> QuestionRequest? = { nil },
                              screen: QuestionRequest? = nil,
                              onFrame: @escaping (CGRect) -> Void = { _ in },
                              onConvo: @escaping (CGRect) -> Void = { _ in }) -> some View {
        ZStack {
            WindowSurface(glass: .off)
            ConversationView(items: items, agentName: agent)
                .paneBar(.bottom) {
                    if let q = screen ?? PaneView.liveQuestion(items, status: status) {
                        QuestionCard(model: q, agentName: agent, canSend: canSend, onSend: onSend, onLoadAll: onLoadAll, onFlow: onFlow)
                            .id(q.toolUseId)
                            .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { onFrame($0) }
                            .inputColumn()
                    } else {
                        Text("Reply to \(agent)").foregroundStyle(.secondary).frame(maxWidth: .infinity).padding(.vertical, 18)
                    }
                }
                .softScrollEdges([.top, .bottom])
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { onConvo($0) }
        }
    }

    private static func window(_ view: some View, _ size: NSSize) -> (NSWindow, NSView) {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        host.frame = NSRect(origin: .zero, size: size)
        let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled], backing: .buffered, defer: false)
        win.contentView = host; win.setFrameOrigin(NSPoint(x: -3000, y: -3000)); win.orderFrontRegardless()
        return (win, host)
    }

    private static func press(_ win: NSWindow, _ ch: String, _ code: UInt16) async {
        for t in [NSEvent.EventType.keyDown, .keyUp] {
            if let e = NSEvent.keyEvent(with: t, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: win.windowNumber,
                                        context: nil, characters: ch, charactersIgnoringModifiers: ch, isARepeat: false, keyCode: code) { win.sendEvent(e) }
        }
        try? await Task.sleep(for: .milliseconds(250))
    }
    private static func digit(_ win: NSWindow, _ n: Int) async { await press(win, "\(n)", [0, 18, 19, 20, 21, 23, 22, 26, 28, 25][n]) }
    private static func returnKey(_ win: NSWindow) async { await press(win, "\r", 36) }
    private static func escKey(_ win: NSWindow) async { await press(win, "\u{1b}", 53) }

    private static func fields(in v: NSView) -> [NSTextField] {
        ((v as? NSTextField).flatMap { $0.isEditable ? [$0] : nil } ?? []) + v.subviews.flatMap { fields(in: $0) }
    }
    private static func axLabels(_ host: NSView) async -> [String] {
        var axWindows: CFTypeRef?
        _ = AXUIElementCopyAttributeValue(AXUIElementCreateApplication(getpid()), kAXWindowsAttribute as CFString, &axWindows)
        try? await Task.sleep(for: .milliseconds(300))
        var out: [String] = []
        func walk(_ e: Any, _ depth: Int) {
            guard depth < 40, let el = e as? NSAccessibilityElementProtocol & NSObject else { return }
            if let l = el.value(forKey: "accessibilityLabel") as? String, !l.isEmpty { out.append(l) }
            if let v = el.value(forKey: "accessibilityValue") as? String, !v.isEmpty { out.append(v) }   // static text
            for c in (el.value(forKey: "accessibilityChildren") as? [Any]) ?? [] { walk(c, depth + 1) }
        }
        walk(host, 0)
        return out
    }

    static func question() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("question-test: \(n) \(ok ? "ok" : "FAIL")") }

        // A tall card (owner's shot 55): four options with descriptions in a long CJK session. The card fits the window, the
        // conversation keeps room above it, and the newest message stays visible.
        let parsed = QuestionAdapter.claudeScreen(tallScreen)
        step("tall fixture parses: 4 options with details, header 筆記格式 (\(parsed?.question.options.count ?? 0) options)",
             parsed?.question.options.count == 4 && parsed?.question.options.allSatisfy { !$0.detail.isEmpty } == true && parsed?.headers == ["筆記格式"])
        if let parsed {
            let tall = QuestionRequest(toolUseId: "screen-tall", questions: [parsed.question], headers: parsed.headers)
            for size in [NSSize(width: 1470, height: 956), NSSize(width: 960, height: 600)] {
                var frame = CGRect.zero, convo = CGRect.zero
                let (win, _) = window(scene(longItems(), screen: tall, onFrame: { frame = $0 }, onConvo: { convo = $0 }), size)
                try? await Task.sleep(for: .seconds(1.5))
                let tag = "\(Int(size.width))x\(Int(size.height))"
                let h = win.frame.height   // global coordinates include the title bar
                step("tall \(tag): card inside the window (y \(Int(frame.minY))..\(Int(frame.maxY)) of \(Int(h)), \(Int(frame.height)) tall)",
                     frame.height > 100 && frame.minY >= 0 && frame.maxY <= h + 0.5)
                step("tall \(tag): conversation keeps at least 160 pt above the card (\(Int(frame.minY - convo.minY)))", frame.minY - convo.minY >= 160)
                capture(win, to: "\(dir)/question-tall-\(tag).png")
                win.orderOut(nil)
            }
            // The real window: RootView, the blocked preview tab (Mobile view), full screen like the owner's shot.
            QuestionProbe.injected = tall
            var cardFrame = CGRect.zero
            for (size, full) in [(NSSize(width: 1470, height: 956), true), (NSSize(width: 960, height: 600), false), (NSSize(width: 900, height: 480), false)] {
                let model = AppModel(store: .preview)
                if let ws = model.store.workspaces.first(where: { $0.tabs.contains { $0.status == .blocked } }),
                   let t = ws.tabs.first(where: { $0.status == .blocked }) { model.select(tab: t, in: ws) }
                let host = NSHostingView(rootView: RootView(flat: false).environment(model).frame(width: size.width, height: size.height))
                host.frame = NSRect(origin: .zero, size: size)
                let win = OffscreenWindow(contentRect: host.frame, styleMask: [.titled, .closable, .resizable], backing: .buffered, defer: false)
                WindowChrome.apply(win)
                win.contentView = host; win.setFrameOrigin(NSPoint(x: -4000, y: -3000)); win.orderFrontRegardless()
                if full { ChromeTests.emulateFullScreen(win, model, true) }
                try? await Task.sleep(for: .seconds(2))
                cardFrame = QuestionProbe.cardFrame
                let tag = "\(Int(size.width))x\(Int(size.height))\(full ? "-full" : "")"
                step("real window \(tag): card shown inside the window (y \(Int(cardFrame.minY))..\(Int(cardFrame.maxY)) of \(Int(win.frame.height)), \(Int(cardFrame.height)) tall)",
                     cardFrame.height > 100 && cardFrame.minY >= 0 && cardFrame.maxY <= win.frame.height + 0.5)
                let pane = QuestionProbe.paneFrame
                step("real window \(tag): pane fits the window (y \(Int(pane.minY))..\(Int(pane.maxY)))", pane.minY >= 0 && pane.maxY <= win.frame.height + 0.5)
                step("real window \(tag): conversation keeps at least \(Int(PaneView.conversationMin)) pt above the card (\(Int(cardFrame.minY - pane.minY)))",
                     cardFrame.minY - pane.minY >= PaneView.conversationMin)
                capture(win, to: "\(dir)/question-tall-real-\(tag).png")
                if full { ChromeTests.emulateFullScreen(win, model, false) }
                win.orderOut(nil)
            }
            QuestionProbe.injected = nil; QuestionProbe.report = nil
        }

        // Which item becomes the live card.
        let pend = items(single())
        step("pending last item, agent blocked: the card", PaneView.liveQuestion(pend, status: .blocked)?.toolUseId == "q-single")
        step("agent working: no card (the question is gone)", PaneView.liveQuestion(pend, status: .working) == nil)
        step("pending but not the last item: no card", PaneView.liveQuestion(pend + [ConversationItem(kind: .assistantText("later"))], status: .idle) == nil)
        var answered = single("q-ans"); answered.state = .answered(["Local file (Recommended)"])
        step("answered: no card", PaneView.liveQuestion(items(answered), status: .idle) == nil)
        step("answer text without the raw (Recommended) suffix: \(QuestionText.shown("integration (Recommended), Local file"))",
             QuestionText.shown("integration (Recommended), Local file") == "integration, Local file")

        // The answer logic, without a view.
        var f = QuestionFlow(three().questions)
        step("1–n picks, n+1 is Other in single choice", f.digit(2) == .picked && f.picks[0] == [1] && f.digit(4) == .other && f.digit(9) == .ignored)
        f.setOther("Magic link"); step("typing Other replaces the single pick", f.picks[0].isEmpty && f.valid)
        _ = f.advance(); _ = f.digit(1); _ = f.digit(3); _ = f.digit(1)
        step("multiSelect toggles: 1, 3, 1 leaves option 3 (\(f.picks[1].sorted()))", f.picks[1] == [2])
        step("multiSelect: no Other key", f.digit(5) == .ignored)
        _ = f.digit(4); f.setOther("Window size")
        step("multiSelect keeps picks with Other", f.picks[1] == [2, 3])
        _ = f.advance(); _ = f.digit(2)
        step("last question: Return means send", f.isLast && f.advance())
        step("answers as HerdrKit reports them: \(f.answers())", f.answers() == ["Magic link", "Open tabs, Sidebar state, Window size", "Next sprint"])
        step("values for answerQuestion: \(f.values().map { "\($0.picks)/\($0.other ?? "-")" })",
             f.values() == [QuestionAnswer(picks: [], other: "Magic link"), QuestionAnswer(picks: [2, 3], other: "Window size"), QuestionAnswer(picks: [1])])
        f.back(); step("back keeps the answers", f.step == 1 && f.picks[1] == [2, 3])

        // Claude's screen read: one question read, the others only as tab-bar headers.
        var first = three().questions[0]; first.header = ""
        var u = QuestionFlow([first], headers: ["Sign-in", "Restore", "Ship"])
        step("screen read: 3 in all, header chip from the tab bar, not last", u.total == 3 && u.header == "Sign-in" && !u.isLast)
        _ = u.digit(1)
        step("going on needs the rest read first; Return alone stays", u.needsLoad && !u.advance() && u.step == 0)
        u.load(three().questions)
        step("after load: pick kept, Return goes to question 2", !u.advance() && u.step == 1 && u.picks[0] == [0])
        step("send only as picker keys with writes on",
             PaneView.canAnswer(single(), writes: true) && !PaneView.canAnswer(single(), writes: false)
             && !PaneView.canAnswer(QuestionRequest(toolUseId: "q-async", agent: "codex", delivery: .message, questions: single().questions), writes: true))
        step("problem words: nothing sent for notOnScreen / unsupported",
             QuestionCard.problemText(QuestionAdapter.Failure.notOnScreen).contains("Nothing was sent")
             && QuestionCard.problemText(QuestionAdapter.Failure.unsupported("x")).contains("Nothing was sent"))

        AppDefaults.store.set(ReadingSize.m.rawValue, forKey: ReadingSize.key)
        AppDefaults.store.set(ReadingWidth.medium.rawValue, forKey: ReadingWidth.key)
        for width in [1200, 960] as [CGFloat] {
            let size = NSSize(width: width, height: 760), w = Int(width)
            // Single choice: keys, recommended tag, send stays off.
            var flow = QuestionFlow([]), frame = CGRect.zero, sent: [QuestionAnswer]?
            let (win, host) = window(scene(items(single()), onFlow: { flow = $0 }, onSend: { sent = $0 }, onFrame: { frame = $0 }), size)
            try? await Task.sleep(for: .seconds(1.2))
            // The conversation's reading column (size M, Medium): as wide as the user's bubbles reach, centred.
            let column = min(ReadingSize.m.column, width - 2 * ReadingWidth.medium.sidePadding)
            step("\(w): card in the composer slot, in the reading column and inside the window (x \(Int(frame.minX))..\(Int(frame.maxX)), y \(Int(frame.minY))..\(Int(frame.maxY)), column \(Int(column)), window \(Int(win.frame.height)) with title bar)",
                 abs(frame.width - column) < 0.5 && abs(frame.midX - width / 2) < 0.5 && frame.maxY <= win.frame.height - 11.5)
            let labels = await axLabels(host)
            step("\(w): Recommended tag on the stripped label", labels.contains("Local file, recommended") && !labels.contains { $0.contains("(Recommended)") })
            await digit(win, 2)
            step("\(w): key 2 picks option 2 (\(flow.picks.first.map { $0.sorted() } ?? []))", flow.picks.first == [1])
            capture(win, to: "\(dir)/question-single-\(w).png")
            await returnKey(win)
            step("\(w): Return with sending off sends nothing", sent == nil)
            win.orderOut(nil)

            // multiSelect: keys toggle.
            var mflow = QuestionFlow([])
            let (mwin, _) = window(scene(items(multi()), onFlow: { mflow = $0 }), size)
            try? await Task.sleep(for: .seconds(1.2))
            await digit(mwin, 1); await digit(mwin, 3)
            step("\(w): multiSelect keys 1 and 3 check two boxes (\(mflow.picks.first.map { $0.sorted() } ?? []))", mflow.picks.first == [0, 2])
            capture(mwin, to: "\(dir)/question-multi-\(w).png")
            await digit(mwin, 1)
            step("\(w): key 1 again unchecks it", mflow.picks.first == [2])
            mwin.orderOut(nil)

            // Three questions: step dots, Return goes on, Esc goes back; Other with marked text.
            var tflow = QuestionFlow([])
            let (twin, thost) = window(scene(items(three()), onFlow: { tflow = $0 }), size)
            try? await Task.sleep(for: .seconds(1.2))
            await digit(twin, 1); await returnKey(twin)
            step("\(w): Return goes to question 2", tflow.step == 1)
            await escKey(twin)
            step("\(w): Esc goes back to question 1, pick kept", tflow.step == 0 && tflow.picks[0] == [0])
            if let field = fields(in: thost).first(where: { $0.accessibilityLabel() == "Other answer" }) {
                twin.makeFirstResponder(field)
                try? await Task.sleep(for: .milliseconds(200))
                if let tv = field.currentEditor() as? NSTextView {
                    tv.setMarkedText("ㄘㄜ", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
                    try? await Task.sleep(for: .milliseconds(200))
                    capture(twin, to: "\(dir)/question-other-marked-\(w).png")
                    tv.doCommand(by: #selector(NSResponder.insertNewline(_:)))
                    try? await Task.sleep(for: .milliseconds(300))
                    step("\(w): Return while composing commits the text and stays (step \(tflow.step), other '\(tflow.other[0])')", tflow.step == 0 && !tv.hasMarkedText() && tflow.other[0] == "ㄘㄜ")
                    step("\(w): Other replaces the pick in single choice", tflow.picks[0].isEmpty)
                    tv.insertText("2", replacementRange: tv.selectedRange())
                    try? await Task.sleep(for: .milliseconds(200))
                    step("\(w): a digit typed in Other is text, not a pick ('\(tflow.other[0])')", tflow.other[0] == "ㄘㄜ2" && tflow.picks[0].isEmpty)
                    tv.doCommand(by: #selector(NSResponder.insertNewline(_:)))
                    try? await Task.sleep(for: .milliseconds(400))
                    step("\(w): Return after composing goes to question 2", tflow.step == 1)
                } else { step("\(w): Other field editor", false) }
            } else { step("\(w): Other field found", false) }
            await digit(twin, 2); await returnKey(twin)
            step("\(w): question 3 shows its step (\(tflow.step + 1) of 3)", tflow.step == 2)
            capture(twin, to: "\(dir)/question-steps-\(w).png")
            twin.orderOut(nil)
        }

        // Screen read with unread questions: the rest are read only when the user goes on (Return), never on appear.
        var loads = 0, uflow = QuestionFlow([])
        let screenQ = QuestionRequest(toolUseId: "screen-1", questions: [first], headers: ["Sign-in", "Restore", "Ship"])
        let loaded = QuestionRequest(toolUseId: "screen-1", questions: three().questions, headers: ["Sign-in", "Restore", "Ship"])
        let (uwin, _) = window(scene(items(nil), canSend: true, onFlow: { uflow = $0 }, onLoadAll: { loads += 1; return loaded }, screen: screenQ),
                               NSSize(width: 1200, height: 760))
        try? await Task.sleep(for: .seconds(1.2))
        capture(uwin, to: "\(dir)/question-unread.png")
        await digit(uwin, 1)
        step("unread: nothing read on appear or on a pick (\(loads) loads)", loads == 0 && uflow.picks.first == [0])
        await returnKey(uwin)
        try? await Task.sleep(for: .milliseconds(300))
        step("unread: Return reads the rest once and goes to question 2 (\(loads) loads, step \(uflow.step + 1))", loads == 1 && uflow.step == 1 && uflow.questions.count == 3)
        uwin.orderOut(nil)

        // A failed send shows why and keeps the answers; the next try goes through.
        var tries = 0, eflow = QuestionFlow([])
        let (ewin, ehost) = window(scene(items(single("q-err")), canSend: true, onFlow: { eflow = $0 },
                                         onSend: { _ in tries += 1; if tries == 1 { throw QuestionAdapter.Failure.notOnScreen } }), NSSize(width: 1200, height: 760))
        try? await Task.sleep(for: .seconds(1.2))
        await digit(ewin, 2); await returnKey(ewin)
        try? await Task.sleep(for: .milliseconds(300))
        let elabels = await axLabels(ehost)
        step("failed send: the card says why, answers kept (\(tries) try)", tries == 1 && elabels.contains { $0.contains("no longer on screen") } && eflow.picks[0] == [1])
        capture(ewin, to: "\(dir)/question-error.png")
        await returnKey(ewin)
        try? await Task.sleep(for: .milliseconds(300))
        let elabels2 = await axLabels(ehost)
        step("second try sends and the card says Sent (\(tries) tries)", tries == 2 && elabels2.contains { $0.hasPrefix("Sent.") })
        ewin.orderOut(nil)

        // Sending on: the card hands over the values and steps aside.
        var sent: [QuestionAnswer]?
        let (swin, _) = window(scene(items(single("q-send")), canSend: true, onSend: { sent = $0 }), NSSize(width: 1200, height: 760))
        try? await Task.sleep(for: .seconds(1.2))
        await digit(swin, 1); await returnKey(swin)
        step("send on: Return sends picks [0] (\(sent.map { "\($0.map(\.picks))" } ?? "nothing"))", sent == [QuestionAnswer(picks: [0])])
        capture(swin, to: "\(dir)/question-sent.png")
        swin.orderOut(nil)

        // Under Settings (the screen kept hidden and inert under the page): the keys do not answer, by the card's own
        // guard too (no `.disabled` here). A card that came while Settings showed answers the keys after Back.
        var coveredSent: [QuestionAnswer]?
        let (cwin, _) = window(scene(items(single("q-covered")), canSend: true, onSend: { coveredSent = $0 })
                                   .environment(\.underSettings, true), NSSize(width: 1200, height: 760))
        try? await Task.sleep(for: .seconds(1.2))
        await digit(cwin, 1); await returnKey(cwin)
        step("under Settings: 1 and Return send nothing (\(coveredSent.map { "\($0.map(\.picks))" } ?? "nothing"))", coveredSent == nil)
        cwin.orderOut(nil)
        let cover = Cover()
        var backSent: [QuestionAnswer]?
        let (bwin, _) = window(Covered(cover: cover) { scene(items(single("q-back")), canSend: true, onSend: { backSent = $0 }) }, NSSize(width: 1200, height: 760))
        try? await Task.sleep(for: .seconds(1.2))
        cover.on = false; try? await Task.sleep(for: .milliseconds(600))
        await digit(bwin, 1); await returnKey(bwin)
        step("came under Settings, after Back: 1 and Return answer (\(backSent.map { "\($0.map(\.picks))" } ?? "nothing"))", backSent == [QuestionAnswer(picks: [0])])
        bwin.orderOut(nil)
        // A send held back under the page (the screen's answer gate) is not "Sent": the card says so and keeps the picks.
        var reports: [String] = []
        QuestionProbe.report = { if case .sent = $0 { reports.append("sent") } else if case .failed(let why) = $0 { reports.append(why) } }
        let (hwin, _) = window(scene(items(single("q-held")), canSend: true, onSend: { _ in throw QuestionCard.HeldUnderSettings() }), NSSize(width: 1200, height: 760))
        try? await Task.sleep(for: .seconds(1.2))
        await digit(hwin, 1); await returnKey(hwin)
        QuestionProbe.report = nil
        step("a send held under Settings is not Sent (\(reports))", reports.count == 1 && reports[0].hasPrefix("Not sent while Settings"))
        hwin.orderOut(nil)

        // Answered and declined rows in the transcript; the composer is back.
        var multiAnswered = three(); multiAnswered.toolUseId = "q-three-ans"
        multiAnswered.state = .answered(["Passkey (Recommended)", "Open tabs, Sidebar state", ""])
        var declined = single("q-dec"); declined.state = .declined
        var unknown = multi(); unknown.toolUseId = "q-unknown"; unknown.state = .answered(nil)
        let history = items(answered) + [ConversationItem(kind: .question(multiAnswered)), ConversationItem(kind: .question(declined)), ConversationItem(kind: .question(unknown))]
        for width in [1200, 960] as [CGFloat] {
            let (awin, _) = window(scene(history, status: .idle), NSSize(width: width, height: 760))
            try? await Task.sleep(for: .seconds(1.2))
            step("\(Int(width)): answered, declined and unknown rows; no card", PaneView.liveQuestion(history, status: .idle) == nil)
            capture(awin, to: "\(dir)/question-answered-\(Int(width)).png")
            awin.orderOut(nil)
        }

        print("question-test: pictures in \(dir)")
        print("question-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }
}

/// Checks: a screen under the Settings page, as `DetailView` keeps it, until `on` turns false (Back).
@Observable private final class Cover { var on = true }
private struct Covered<Content: View>: View {
    let cover: Cover
    @ViewBuilder let content: () -> Content
    var body: some View { content().environment(\.underSettings, cover.on).disabled(cover.on) }
}
