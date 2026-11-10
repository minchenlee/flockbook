import AppKit
import HerdrKit
import SwiftUI

/// The "/" list above the reply box, and the two send guards (a bare screen command; text already in the agent's
/// box). Offscreen; the agent's files come from a temp home, the pane's screen from a hook, and every send goes to
/// a hook: nothing reaches herdr or an agent.
extension ContentTests {
    private static func field(in v: NSView) -> NSTextView? { all(NSTextView.self, in: v).first { $0.isEditable } }
    private static func press(_ win: NSWindow, _ ch: String, _ code: UInt16, _ flags: NSEvent.ModifierFlags = []) {
        for t in [NSEvent.EventType.keyDown, .keyUp] {
            if let e = NSEvent.keyEvent(with: t, location: .zero, modifierFlags: flags, timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: win.windowNumber,
                                        context: nil, characters: ch, charactersIgnoringModifiers: ch, isARepeat: false, keyCode: code) { win.sendEvent(e) }
        }
    }
    private static func put(_ p: URL, _ s: String) {
        try? FileManager.default.createDirectory(at: p.deletingLastPathComponent(), withIntermediateDirectories: true)
        try? Data(s.utf8).write(to: p)
    }

    static func slash() async {
        var failures = 0
        func step(_ n: String, _ ok: Bool) { if !ok { failures += 1 }; print("slash-test: \(n) \(ok ? "ok" : "FAIL")") }
        func wait(_ ms: Int = 250) async { try? await Task.sleep(for: .milliseconds(ms)) }
        let up = ("\u{F700}", UInt16(126)), down = ("\u{F701}", UInt16(125)), tab = ("\t", UInt16(48)), esc = ("\u{1b}", UInt16(53)), ret = ("\r", UInt16(36))
        let arrows: NSEvent.ModifierFlags = [.function, .numericPad]

        let root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("slash-test-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let home = root.appendingPathComponent("home"), proj = root.appendingPathComponent("proj")
        put(proj.appendingPathComponent(".claude/commands/deploy.md"), "---\ndescription: Deploy the app to staging\nargument-hint: \"[target]\"\n---\n")
        put(home.appendingPathComponent(".claude/skills/notes/SKILL.md"), "---\nname: take-notes\ndescription: Write the meeting notes\n---\n")
        let want = SlashCommands.listing("claude", home: home, cwd: proj)

        // 1. The composer alone.
        let probe = PlaceholderProbe()
        var delivered: [String] = []
        let (win, host) = offscreen(VStack { Spacer(); ComposerView(agentName: "testagent", onSend: { _ in }, probe: probe, deliverResult: { msg in
            delivered.append(msg); return .sent
        }, commands: { SlashCommands.listing("claude", home: home, cwd: proj) }).padding(16) }, NSSize(width: 720, height: 460))
        await wait(900)
        guard let tv = field(in: host) else { print("slash-test: no text view"); NSApp.terminate(nil); return }
        win.makeFirstResponder(tv)
        func type(_ s: String) async { tv.insertText(s, replacementRange: tv.selectedRange()); await wait() }
        func clear() async { tv.selectAll(nil); tv.delete(nil); await wait() }

        await type("/")
        step("\"/\" shows the agent's commands (\(probe.slash?.items.count ?? -1) of \(want.commands.count)), 8 rows (\(probe.slash?.shown.count ?? -1))",
             probe.slash?.items.map(\.name) == want.commands.map(\.name) && probe.slash?.shown.count == 8 && want.commands.count == 12)
        step("the count says there are more (\(probe.slash?.count ?? "none"))", probe.slash?.count == "1–8 of 12")
        step("its own commands and skills are in it", probe.slash?.items.contains { $0.name == "deploy" && $0.hint == "[target]" } == true && probe.slash?.items.contains { $0.name == "take-notes" } == true)
        capture(win, to: "\(dir)/slash-list.png")
        press(win, down.0, down.1, arrows); press(win, down.0, down.1, arrows); await wait()
        step("↓ ↓ moves the pick (\(probe.slash?.pick ?? -1))", probe.slash?.pick == 2)
        press(win, up.0, up.1, arrows); await wait()
        step("↑ moves it back (\(probe.slash?.pick ?? -1))", probe.slash?.pick == 1)
        for _ in 0..<9 { press(win, down.0, down.1, arrows) }
        await wait()
        step("↓ past the last row moves the window (\(probe.slash?.count ?? "none"), pick \(probe.slash?.pick ?? -1))", probe.slash?.count == "4–11 of 12" && probe.slash?.pick == 10)
        capture(win, to: "\(dir)/slash-list-scrolled.png")
        for _ in 0..<9 { press(win, down.0, down.1, arrows) }
        await wait()
        step("↓ stops at the last match (\(probe.slash?.pick ?? -1))", probe.slash?.pick == 11 && tv.string == "/")

        // The wheel over the list: a real scroll event through the app's event path.
        await clear(); await type("/")
        let catcher = all(NSView.self, in: host).first { String(describing: Swift.type(of: $0)) == "Catcher" }
        func wheel(_ dy: Int32) {
            guard let v = catcher ?? host as NSView?, let w = v.window else { return }
            let inWin = v.convert(NSPoint(x: v.bounds.midX, y: v.bounds.midY), to: nil), screenPt = w.convertPoint(toScreen: inWin)
            let h = NSScreen.screens.first?.frame.height ?? 0
            guard let cg = CGEvent(scrollWheelEvent2Source: nil, units: .pixel, wheelCount: 1, wheel1: dy, wheel2: 0, wheel3: 0) else { return }
            cg.location = CGPoint(x: screenPt.x, y: h - screenPt.y)
            cg.setIntegerValueField(.mouseEventWindowUnderMousePointer, value: Int64(w.windowNumber))
            cg.setIntegerValueField(.scrollWheelEventIsContinuous, value: 1)
            if let ev = NSEvent(cgEvent: cg) { NSApp.sendEvent(ev) }
        }
        step("the list has a wheel area", catcher != nil)
        wheel(-90); await wait()
        step("wheel down 90 pt: three rows (\(probe.slash?.count ?? "none")), the pick stays on a shown row (\(probe.slash?.pick ?? -1))", probe.slash?.top == 3 && probe.slash?.pick == 3)
        wheel(2000); await wait()
        step("wheel up stops at the top (\(probe.slash?.top ?? -1))", probe.slash?.top == 0)

        // Filter, Tab, values, Return.
        await clear(); await type("/mo")
        step("typing filters by prefix (\(probe.slash?.items.map(\.name) ?? []))", probe.slash?.items.map(\.name) == ["model"])
        step("a screen command says it needs a value", probe.slash?.items.first?.screen == true && probe.slash?.items.first?.hint == "<model>")
        press(win, tab.0, tab.1); await wait()
        step("Tab completes the name and a space (\"\(tv.string)\"), nothing sent", tv.string == "/model " && delivered.isEmpty)
        step("then the list offers its values with labels (\(probe.slash?.items.first.map { "\($0.name) \($0.description)" } ?? "none"))",
             probe.slash?.command == "model" && probe.slash?.items.first == SlashList.Item(name: "claude-fable-5-2", description: "Fable 5.2"))
        step("the cursor is at the end (\(tv.selectedRange().location))", tv.selectedRange().location == 7)
        capture(win, to: "\(dir)/slash-values.png")
        await type("so")
        step("a part of a value or its label finds it (\(probe.slash?.items.map(\.name) ?? []))", probe.slash?.items.map(\.name) == ["claude-sonnet-5-5", "sonnet"])
        press(win, ret.0, ret.1); await wait()
        step("Return on a part completes it (\"\(tv.string)\"), nothing sent", tv.string == "/model claude-sonnet-5-5" && delivered.isEmpty)
        press(win, ret.0, ret.1); await wait()
        step("Return on a whole value sends it (\(delivered))", delivered == ["/model claude-sonnet-5-5"] && tv.string.isEmpty)
        await type("/cle"); press(win, ret.0, ret.1); await wait()
        step("Return on a part of a name completes it (\"\(tv.string)\")", tv.string == "/clear " && delivered.count == 1)
        press(win, ret.0, ret.1); await wait()
        step("Return on the whole name sends it (\(delivered.last ?? ""))", delivered.last == "/clear" && tv.string.isEmpty)

        // A bare screen command.
        await type("/model"); press(win, ret.0, ret.1); await wait(400)
        step("\"/model\" alone is not sent, the text stays (\"\(tv.string)\")", tv.string == "/model" && delivered.count == 2)
        step("the note says why (\(probe.error ?? "none"))", probe.error == "/model opens a menu in the terminal. Add a value, like /model claude-fable-5-2.")
        capture(win, to: "\(dir)/slash-screen-note.png")
        await clear(); await type("/status"); press(win, ret.0, ret.1); await wait(400)
        step("\"/status\" alone is not sent (\(probe.error ?? "none"))", tv.string == "/status" && delivered.count == 2 && probe.error == "/status opens a screen in the terminal. Use it there.")
        step("a screen with no value is not in the list", probe.slash == nil)

        // Esc, and the keys with no list.
        await clear(); await type("/c")
        step("the list is open (\(probe.slash?.items.count ?? 0))", (probe.slash?.items.count ?? 0) > 1)
        press(win, esc.0, esc.1); await wait()
        step("Esc closes it, the text stays (\"\(tv.string)\")", probe.slash == nil && tv.string == "/c")
        press(win, tab.0, tab.1); await wait()
        step("closed: Tab is the text's own (\"\(tv.string.replacingOccurrences(of: "\t", with: "\\t"))\")", tv.string == "/c\t")
        await clear(); await type("/cl")
        step("typing opens it again", probe.slash?.items.map(\.name) == ["clear"])
        await clear(); await type("first line")
        tv.insertNewlineIgnoringFieldEditor(nil); await type("second")
        press(win, up.0, up.1, arrows); await wait()
        step("no list: ↑ moves the cursor (to \(tv.selectedRange().location))", tv.selectedRange().location < 11 && tv.string == "first line\nsecond")
        press(win, down.0, down.1, arrows); await wait()
        step("no list: ↓ moves it back (to \(tv.selectedRange().location))", tv.selectedRange().location > 10)

        // A click on a row (pressed the way VoiceOver does: an offscreen window swallows a synthetic first click).
        await clear(); await type("/co")
        var row: NSObject?
        func walk(_ e: Any, _ depth: Int) {
            guard row == nil, depth < 40, let el = e as? NSAccessibilityElementProtocol & NSObject else { return }
            if (el.value(forKey: "accessibilityLabel") as? String) == "/context" { row = el; return }
            for c in (el.value(forKey: "accessibilityChildren") as? [Any]) ?? [] { walk(c, depth + 1) }
        }
        var axWindows: CFTypeRef?
        _ = AXUIElementCopyAttributeValue(AXUIElementCreateApplication(getpid()), kAXWindowsAttribute as CFString, &axWindows)
        await wait(500)
        walk(host, 0)
        step("a row is a button in the accessibility tree", row != nil)
        _ = row?.perform(#selector(NSAccessibilityButton.accessibilityPerformPress))
        await wait()
        step("a click on a row completes it (\"\(tv.string)\"), the field keeps the keys", tv.string == "/context " && win.firstResponder === tv)
        await clear(); await type("/zz")
        step("no match: no list", probe.slash == nil)
        await clear(); await type("hello /model")
        step("a \"/\" later in the text: no list", probe.slash == nil)
        win.orderOut(nil)

        // 2. The send guards on the app's send path (AppModel.deliverResult): the screen and the send are hooks.
        AppDefaults.store.set(true, forKey: AppSettings.allowInputKey)
        defer { AppDefaults.store.removeObject(forKey: AppSettings.allowInputKey) }
        let model = AppModel(store: .preview)
        model.agentHome = home
        var typed: [String] = []
        model.sendHook = { _, text in typed.append(text); return true }
        let rule = String(repeating: "─", count: 60)
        func claude(_ row: String) -> String { "● Done.\n\n\(rule)\n\(row)\n\(rule)\n  ~/proj  ⎇main\n" }
        var screen: String? = claude("❯\u{a0}/sta")
        model.screenHook = { _ in screen }
        guard let ws = model.store.workspaces.first, let tabItem = ws.tabs.first(where: { !$0.isSplit && $0.panes.first?.agent?.kind == "claude" }), let pane = tabItem.panes.first, let agent = pane.agent else {
            print("slash-test: preview has no single Claude pane"); NSApp.terminate(nil); return
        }
        var r = await model.deliverResult("hello", pane: pane.id, kind: "claude")
        step("text in the agent's box: not sent (\(r), typed \(typed))", r == .failed && typed.isEmpty)
        step("the note is the agent-box note", MessageSender.takeRefusal("hello") == AgentBox.notEmpty)
        screen = claude("❯\u{a0}run the tests")
        r = await model.deliverResult("hello", pane: pane.id, kind: "claude")
        step("Claude Code with prompt suggestions on: other text may be a suggestion, so it is sent (\(r))", r == .sent && typed == ["hello"])
        put(home.appendingPathComponent(".claude/settings.json"), #"{"promptSuggestionEnabled": false}"#)
        r = await model.deliverResult("again", pane: pane.id, kind: "claude")
        step("suggestions off in the settings: the same text holds the send back (\(r))", r == .failed && typed == ["hello"] && MessageSender.takeRefusal("again") == AgentBox.notEmpty)
        screen = claude("❯\u{a0}Try \"fix typecheck errors\"")
        r = await model.deliverResult("again", pane: pane.id, kind: "claude")
        step("an empty box (its placeholder): sent (\(r))", r == .sent && typed == ["hello", "again"])
        screen = claude(" Do you want to proceed?\n ❯ 1. Yes")
        r = await model.deliverResult("yes please", pane: pane.id, kind: "claude")
        step("a box that cannot be read for sure: sent as before (\(r))", r == .sent && typed.count == 3)
        screen = nil
        r = await model.deliverResult("no screen", pane: pane.id, kind: "claude")
        step("no screen read: sent as before (\(r))", r == .sent && typed.count == 4)
        r = await model.deliverResult(" /status\n", pane: pane.id, kind: "claude")
        step("a bare screen command on the send path: not sent (\(r))", r == .failed && typed.count == 4
             && MessageSender.takeRefusal(" /status\n") == "/status opens a screen in the terminal. Use it there.")
        r = await model.deliverResult("/model sonnet", pane: pane.id, kind: "claude")
        step("with a value it goes out (\(r))", r == .sent && typed.last == "/model sonnet")
        r = await model.deliverResult("/model", pane: pane.id, kind: "cursor-agent")
        step("an agent with no list: sent as before (\(r))", r == .sent && typed.last == "/model")
        step("the preview pane's agent is on this Mac", model.agentIsLocal(pane: pane.id) && model.slashHome(pane: pane.id) == home)
        step("a pane on another machine reads no local files", !model.agentIsLocal(pane: "hc:far/t1") && model.slashHome(pane: "hc:far/t1") == SlashCommands.noFiles)

        // 3. In the pane (the real layout): the list floats over the conversation; the box note comes back with the text.
        model.select(tab: tabItem, in: ws)
        // Flat cards (the checks' usual look), then the window's glass, where the cards are see-through.
        for (name, look, glass) in [("", NSAppearance.Name.aqua, WindowGlass.off), ("-dark", .darkAqua, .off), ("-glass", .darkAqua, .full)] {
            NSApp.appearance = NSAppearance(named: look)
            let (w2, h2) = offscreen(ZStack { WindowSurface(glass: glass); TabScreen() }.environment(model).environment(\.flatGlass, glass == .off), NSSize(width: 900, height: 640))
            await wait(900)
            guard let field = field(in: h2) else { step("the pane has a reply box", false); continue }
            let convo = all(NSScrollView.self, in: h2).max { $0.bounds.height < $1.bounds.height }
            let inset = convo?.contentInsets.bottom ?? -1, boxY = field.convert(field.bounds, to: nil).minY
            w2.makeFirstResponder(field)
            field.insertText("/", replacementRange: field.selectedRange()); await wait(500)
            let listed = all(NSView.self, in: h2).first { String(describing: Swift.type(of: $0)) == "Catcher" }
            step("pane\(name): \"/\" opens the list for \(agent.name)", listed != nil)
            step("pane\(name): the bar keeps its height (inset \(Int(inset)) → \(Int(convo?.contentInsets.bottom ?? -1))), the box does not move",
                 convo?.contentInsets.bottom == inset && field.convert(field.bounds, to: nil).minY == boxY)
            if let listed {
                let l = listed.convert(listed.bounds, to: nil), f = field.convert(field.bounds, to: nil)
                step("pane\(name): the list is above the box (list \(Int(l.minY))…\(Int(l.maxY)), box top \(Int(f.maxY))), as wide as its card", l.minY >= f.maxY && l.width > 300)
            }
            capture(w2, to: "\(dir)/slash-pane\(name).png")
            field.selectAll(nil); field.delete(nil); await wait()
            screen = claude("❯\u{a0}/sta")
            let before = typed.count
            field.insertText("hello from the pane", replacementRange: field.selectedRange()); await wait()
            press(w2, ret.0, ret.1); await wait(700)
            step("pane\(name): text in the agent's box: nothing sent, the text comes back (\"\(field.string)\")", typed.count == before && field.string == "hello from the pane")
            capture(w2, to: "\(dir)/slash-box-note\(name).png")
            screen = claude("❯\u{a0}")
            press(w2, ret.0, ret.1); await wait(700)
            step("pane\(name): the box is empty now: Return sends it", typed.last == "hello from the pane" && field.string.isEmpty)
            w2.orderOut(nil)
        }
        print("slash-test: \(failures == 0 ? "PASS" : "FAIL x\(failures)")")
        NSApp.terminate(nil)
    }
}
