import AppKit
import Darwin
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
        f.setOther("Magic link")
        step("Claude Other wins over single pick while kept underneath", f.picks[0] == [1] && f.values().first == QuestionAnswer(picks: [], other: "Magic link"))
        f.setOther("")
        step("clearing Claude Other brings the single pick back", f.picks[0] == [1] && f.values().first == QuestionAnswer(picks: [1]))
        f.setOther("Magic link")
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
                    step("\(w): Other wins over the Claude single pick while kept underneath", tflow.values().first == QuestionAnswer(picks: [], other: "ㄘㄜ") && tflow.picks[0] == [0])
                    tv.insertText("2", replacementRange: tv.selectedRange())
                    try? await Task.sleep(for: .milliseconds(200))
                    step("\(w): a digit typed in Other is text, not a pick ('\(tflow.other[0])')", tflow.other[0] == "ㄘㄜ2" && tflow.values().first == QuestionAnswer(picks: [], other: "ㄘㄜ2") && tflow.picks[0] == [0])
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

        // Codex keeps pick + notes on single-choice questions; Claude single choice lets non-empty Other win (old pick
        // does not render selected, clearing Other restores it).
        var cxSent: [QuestionAnswer]?, cxFlow = QuestionFlow([])
        let codexSingle = QuestionRequest(toolUseId: "q-cx-single", agent: "codex", questions: single().questions)
        let (cxWin, cxHost) = window(
            ZStack {
                WindowSurface(glass: .off)
                QuestionCard(model: codexSingle, agentName: "codex", canSend: true, onSend: { cxSent = $0 }, onFlow: { cxFlow = $0 }).inputColumn()
            }, NSSize(width: 1200, height: 520))
        try? await Task.sleep(for: .seconds(1.2))
        await digit(cxWin, 2)
        if let field = fields(in: cxHost).first(where: { $0.accessibilityLabel() == "Other answer" }) {
            cxWin.makeFirstResponder(field)
            try? await Task.sleep(for: .milliseconds(200))
            if let tv = field.currentEditor() as? NSTextView {
                tv.insertText("my note", replacementRange: tv.selectedRange())
                try? await Task.sleep(for: .milliseconds(200))
                tv.doCommand(by: #selector(NSResponder.insertNewline(_:)))
                try? await Task.sleep(for: .milliseconds(300))
            }
        }
        step("Codex single choice keeps pick + notes on Send (\(cxSent?.map { "\($0.picks)/\($0.other ?? "-")" } ?? []))",
             cxFlow.values() == [QuestionAnswer(picks: [1], other: "my note")]
             && cxSent == [QuestionAnswer(picks: [1], other: "my note")])
        cxWin.orderOut(nil)

        // Multi-choice with a non-colliding comma label now sends cleanly, while truly colliding labels (e.g. two
        // identical labels) block from the start with the shared exact copy and Return sends nothing.
        var cleanCommaSent: [QuestionAnswer]?
        let cleanCommaQ = QuestionRequest(toolUseId: "toolu_cc", questions: [
            .init(header: "Colors", text: "Which colors?", options: [.init(label: "Red, green"), .init(label: "Blue")], multiSelect: true),
        ])
        let (ccWin, _) = window(
            ZStack {
                WindowSurface(glass: .off)
                QuestionCard(model: cleanCommaQ, agentName: agent, canSend: true, onSend: { cleanCommaSent = $0 }).inputColumn()
            }, NSSize(width: 1200, height: 420))
        try? await Task.sleep(for: .seconds(1.0))
        await digit(ccWin, 1); await returnKey(ccWin)
        step("multi-choice unique comma option now sends (\(cleanCommaSent.map { "\($0.map(\.picks))" } ?? "nothing"))",
             cleanCommaSent == [QuestionAnswer(picks: [0])])
        ccWin.orderOut(nil)

        var blockedSent: [QuestionAnswer]?, opened = 0
        let commaQ = QuestionRequest(toolUseId: "toolu_rb", questions: [
            .init(header: "Colors", text: "Which colors?", options: [.init(label: "Yes"), .init(label: "Yes")], multiSelect: true),
        ])
        let (rwin, rhost) = window(
            ZStack {
                WindowSurface(glass: .off)
                QuestionCard(model: commaQ, agentName: agent, canSend: true, onSend: { blockedSent = $0 }, onOpenTerminal: { opened += 1 }).inputColumn()
            }, NSSize(width: 1200, height: 420))
        try? await Task.sleep(for: .seconds(1.2))
        let rlabels = await axLabels(rhost)
        step("review blocked: the reason and Answer in terminal from the start",
             rlabels.contains { $0.hasPrefix("Two answers here would look identical") } && rlabels.contains("Answer in terminal"))
        capture(rwin, to: "\(dir)/question-review-blocked.png")
        await digit(rwin, 2); await returnKey(rwin)
        step("review blocked: 2 and Return send nothing (\(blockedSent.map { "\($0.map(\.picks))" } ?? "nothing"))", blockedSent == nil)
        rwin.orderOut(nil)

        // TabScreen / PaneView: failing on a screen- question when the terminal moved to Review your answers keeps a
        // read-only stop card (error shown, picks & Send off, Open terminal active) instead of dropping to composer.
        do {
            let fakeSock = FakeQuestionSocket(path: NSTemporaryDirectory() + "hd-stop-\(getpid()).sock")
            fakeSock.agents = #"[{"pane_id":"w1:p1","agent":"claude","name":"claude","agent_status":"blocked","agent_session":{"agent":"claude","kind":"id","source":"herdr:claude","value":"s"}}]"#
            let picker = """
            ────────────────────────────────────────────────────────
             ☐ Color

            Pick a color?

            ❯ 1. Red
              2. Blue
              3. Type something.
            ────────────────────────────────────────────────────────
              4. Chat about this

            Enter to select · ↑/↓ to navigate · Esc to cancel
            """
            let review = """
            ←  ☒ Color  ✔ Submit  →

            Review your answers

             ● Pick a color?
               → Red

            Ready to submit your answers?

            ❯ 1. Submit answers
              2. Cancel
            """
            fakeSock.screen = picker
            fakeSock.failSendWithScreen = review
            try fakeSock.start()
            defer { fakeSock.kill() }
            let stopStore = HerdrStore(session: HerdrSession(name: "fake-stop", socketPath: fakeSock.path))
            let stopModel = AppModel(store: stopStore)
            stopStore.start()
            let t0 = Date()
            while (stopStore.workspaces.first?.tabs.first?.panes.first == nil) && Date().timeIntervalSince(t0) < 3 {
                await stopStore.refresh()
                try? await Task.sleep(for: .milliseconds(50))
            }
            var appeared: QuestionRequest?
            QuestionProbe.report = { if case .appeared(let q) = $0 { appeared = q } }
            defer { QuestionProbe.report = nil }
            let (pwin, phost) = window(
                ZStack {
                    WindowSurface(glass: .off)
                    if let p = stopStore.workspaces.first?.tabs.first?.panes.first {
                        PaneView(pane: p, showLabel: false, focused: true)
                    }
                }.environment(stopModel),
                NSSize(width: 960, height: 600))
            let t1 = Date()
            while appeared == nil && Date().timeIntervalSince(t1) < 3 {
                try? await Task.sleep(for: .milliseconds(50))
            }
            step("TabScreen screen- card loaded (\(appeared?.toolUseId ?? "nil"))", appeared?.toolUseId.hasPrefix("screen-") == true)
            await digit(pwin, 1)
            await returnKey(pwin)
            try? await Task.sleep(for: .milliseconds(800))
            let stopLabels = await axLabels(phost)
            step("TabScreen failed screen- send keeps read-only stop card with error and Open terminal (labels: \(stopLabels))",
                 stopLabels.contains { $0.contains("Could not reach the terminal") || $0.contains("Nothing was sent") || $0.contains("Check the terminal") }
                 && stopLabels.contains("Open terminal")
                 && !stopLabels.contains("Send")
                 && !stopLabels.contains("Red")
                 && !stopLabels.contains("Reply to claude"))
            pwin.orderOut(nil)
        } catch {
            step("TabScreen stop card setup failed: \(error)", false)
        }

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

        // One answer per question pane: the Mac card and the phone take the pane's lock, so their key sequences never
        // interleave (on hc each key may be held a while). A second answer while one is on its way sends nothing.
        let qm = AppModel(store: HerdrStore.preview)
        let qp = Pane(id: "hc:tq", terminalId: "tq", cwd: "/tmp", agent: nil, backend: .hc)
        _ = qm.claim(qp.id)
        var busyErr: Error?
        do { try await qm.answerQuestion(pane: qp.id, request: single(), answers: [QuestionAnswer(picks: [0])]) } catch { busyErr = error }
        step("a second answer while one is on its way: refused, nothing sent (\(busyErr.map { "\($0)" } ?? "no error"))",
             (busyErr as? QuestionAdapter.Failure) == .unsupported(AppModel.answerBusy))
        qm.release(qp.id)
        do { try await qm.answerQuestion(pane: qp.id, request: single(), answers: [QuestionAnswer(picks: [0])]) } catch { busyErr = error }
        let free = qm.claim(qp.id)
        step("the answer releases the pane afterwards", free)
        // Reading the rest of the questions moves the picker (Left/Right): it takes the same lock, on the Mac card and
        // on the phone (there in one hold with the answer keys), so no navigation key lands between another's answers.
        var unreadQ = single(); unreadQ.headers = ["Sign-in", "Restore", "Ship"]   // 1 read, 2 not yet
        var loadErr: Error?
        do { _ = try await qm.loadAllQuestions(pane: qp.id) } catch { loadErr = error }
        step("Mac 'load all' while the pane is held: refused before any key (\(loadErr.map { "\($0)" } ?? "no error"))",
             (loadErr as? QuestionAdapter.Failure) == .unsupported(AppModel.answerBusy))
        let hubAgent = Agent(name: "claude", kind: "claude", status: .blocked, sessionId: nil, title: "", backend: .hc)
        var hubErr: Error?
        do { try await AppHubBackend(model: qm).answer(pane: qp.id, question: unreadQ, answers: [], agent: hubAgent, allowed: { true }) } catch { hubErr = error }
        step("phone answer with unread questions while the pane is held: refused before any key (\(hubErr.map { "\($0)" } ?? "no error"))",
             (hubErr as? QuestionAdapter.Failure) == .unsupported(AppModel.answerBusy))
        qm.release(qp.id)
        do { try await AppHubBackend(model: qm).answer(pane: qp.id, question: unreadQ, answers: [], agent: hubAgent, allowed: { true }) } catch { hubErr = error }
        let freeAgain = qm.claim(qp.id)
        step("the phone's answer releases the pane afterwards", freeAgain)
        qm.release(qp.id)

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

private final class FakeQuestionSocket: @unchecked Sendable {
    let path: String
    private let lock = NSLock()
    private var listenFD: Int32 = -1
    private var open: [Int32] = []
    private var _screen = ""
    private var _agents = "[]"
    var failSendWithScreen: String?

    init(path: String) { self.path = path }
    var screen: String { get { lock.withLock { _screen } } set { lock.withLock { _screen = newValue } } }
    var agents: String { get { lock.withLock { _agents } } set { lock.withLock { _agents = newValue } } }

    func start() throws {
        unlink(path)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        var addr = sockaddr_un(); addr.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            for (i, b) in path.utf8.enumerated() { raw[i] = b }; raw[path.utf8.count] = 0
        }
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard rc == 0, listen(fd, 16) == 0 else { close(fd); throw HerdrError.socket("fake bind \(errno)") }
        lock.withLock { listenFD = fd }
        Thread.detachNewThread { [self] in
            while true {
                let c = accept(fd, nil, nil)
                if c < 0 { return }
                var one: Int32 = 1
                setsockopt(c, SOL_SOCKET, SO_NOSIGPIPE, &one, socklen_t(MemoryLayout<Int32>.size))
                lock.withLock { open.append(c) }
                Thread.detachNewThread { self.serve(c) }
            }
        }
    }

    func kill() {
        lock.withLock {
            if listenFD >= 0 { shutdown(listenFD, SHUT_RDWR); close(listenFD); listenFD = -1 }
            for c in open { shutdown(c, SHUT_RDWR); close(c) }
            open = []
        }
        unlink(path)
    }

    private func serve(_ c: Int32) {
        var buf = [UInt8]()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(c, &chunk, chunk.count)
            if n <= 0 { return }
            buf += chunk[..<n]
            while let nl = buf.firstIndex(of: 0x0A) {
                let line = Data(buf[..<nl]); buf.removeSubrange(...nl)
                guard let o = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
                let id = o["id"] as? String ?? ""
                let reply: String
                switch o["method"] as? String {
                case "session.snapshot":
                    let a = agents
                    reply = #"{"id":"\#(id)","result":{"type":"snapshot","snapshot":{"workspaces":[{"workspace_id":"w1","label":"v0","number":1}],"tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1}],"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","terminal_id":"term_1","cwd":"/tmp"}],"agents":\#(a)}}}"#
                case "events.subscribe":
                    reply = #"{"id":"\#(id)","result":{"type":"subscription_started"}}"#
                case "pane.read":
                    let text = String(decoding: try! JSONSerialization.data(withJSONObject: [screen]), as: UTF8.self).dropFirst().dropLast()
                    reply = #"{"id":"\#(id)","result":{"type":"pane_read","read":{"text":\#(text)}}}"#
                case "pane.send_text", "pane.send_keys", "pane.send_input":
                    if let s = failSendWithScreen {
                        screen = s
                        reply = #"{"id":"\#(id)","error":{"code":"failed","message":"review screen"}}"#
                    } else {
                        reply = #"{"id":"\#(id)","result":{"type":"ok"}}"#
                    }
                default:
                    reply = #"{"id":"\#(id)","error":{"code":"unknown","message":"fake"}}"#
                }
                let out = Array(reply.utf8) + [0x0A]
                _ = out.withUnsafeBytes { write(c, $0.baseAddress, $0.count) }
            }
        }
    }
}
