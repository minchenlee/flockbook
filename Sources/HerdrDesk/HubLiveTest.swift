import HerdrKit
import Foundation
import AppKit
import ImageIO

/// `--session <named> --test-hub <paneId> [--deny]`: the remote hub over a real WebSocket on 127.0.0.1, with this
/// app's store and input paths. A read-level device must be refused; with an approval open, `send` must be refused
/// (promptOpen), a stale prompt id must be refused, and the real id must answer. Without a prompt, `send` types a
/// short message; with `--image`, it first attaches a JPEG that carries GPS (the stored file must have none) and
/// sends it with the text. Refuses to run on the default session (checked at launch).
@MainActor
enum HubLiveTest {
    // Swift 6.2.1 crashes optimizing this function in release builds (CopyPropagation ownership error); test-only code.
    @_optimize(none)
    /// `other`: answer every open question with this free text instead of its first option.
    static func run(model: AppModel, paneId: String, allow: Bool, image: Bool = false, other: String? = nil) async {
        try? await Task.sleep(for: .seconds(3))
        await model.store.refresh()
        var ok = true
        // The pane's own backend (hc or TUI), as the hub uses.
        let screen: any PaneScreen = model.screen(paneId: paneId) ?? model.store.client
        func step(_ what: String, _ pass: Bool) { print("test-hub: \(pass ? "ok" : "FAIL") \(what)"); ok = ok && pass }

        let devices = MemoryDeviceStore()
        devices.add(token: "tok-input", device: HubDevice(id: "d-in", name: "Test phone", level: .input))
        devices.add(token: "tok-read", device: HubDevice(id: "d-ro", name: "Test tablet", level: .read))
        let server = HubServer(backend: AppHubBackend(model: model), devices: devices, tickInterval: .milliseconds(500))
        guard let port = try? await server.start(host: "127.0.0.1", port: 0) else { step("server start", false); finish(false); return }
        print("test-hub: listening on 127.0.0.1:\(port)")

        let ro = Client(port: port)
        try? await ro.send(#"{"v":1,"type":"hello","id":"h","token":"tok-read","client":"test-hub","clientVersion":"1"}"#)
        let roHello = await ro.untilReply()
        step("read device hello -> \(roHello.map(\.type))", roHello.last.map(isOK) == true)
        try? await ro.send(#"{"v":1,"type":"send","id":"s","pane":"\#(paneId)","text":"must not arrive"}"#)
        let roSend = await ro.untilReply().last
        step("read device send -> readOnly", errorCode(roSend) == .readOnly)
        ro.close()

        let c = Client(port: port)
        try? await c.send(#"{"v":1,"type":"hello","id":"h","token":"tok-input","client":"test-hub","clientVersion":"1"}"#)
        step("input device hello", await c.untilReply().last.map(isOK) == true)
        try? await c.send(#"{"v":1,"type":"watch","id":"w","pane":"\#(paneId)"}"#)
        let watch = await c.untilReply()
        step("watch -> \(watch.map(\.type))", watch.last.map(isOK) == true)
        var prompt = watch.compactMap { if case .prompt(let p) = $0 { p } else { nil } }.last
        // A log the store had not read yet (Codex's question is in its log) arrives in a later push.
        if prompt?.approval == nil, prompt?.question == nil, model.store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == paneId })?.agent?.status == .blocked {
            for _ in 0..<6 {
                guard let m = await c.next(1) else { continue }
                if case .prompt(let p) = m { prompt = p; break }
            }
        }
        print("test-hub: prompt = approval \(prompt?.approval.map { "\($0.id) \($0.tool) | \($0.command)" } ?? "none"), question \(prompt?.question?.id ?? "none")")

        if let a = prompt?.approval {
            try? await c.send(#"{"v":1,"type":"send","id":"s","pane":"\#(paneId)","text":"must not land in the picker"}"#)
            step("send while the approval is open -> promptOpen", errorCode(await c.untilReply().last) == .promptOpen)
            try? await c.send(#"{"v":1,"type":"approve","id":"a0","pane":"\#(paneId)","prompt":"toolu_stale","allow":\#(allow)}"#)
            step("approve with a stale id -> stale", errorCode(await c.untilReply().last) == .stale)
            let still = await screen.checkPrompt(paneId: paneId)
            step("prompt still on screen after the refusals", still)
            try? await c.send(#"{"v":1,"type":"approve","id":"a1","pane":"\#(paneId)","prompt":"\#(a.id)","allow":\#(allow)}"#)
            step("\(allow ? "allow" : "deny") with the current id -> ok", await c.untilReply(8).last.map(isOK) == true)
            var cleared = false
            for _ in 0..<20 {
                guard let m = await c.next(1) else { continue }
                if case .prompt(let p) = m, p.approval == nil { cleared = true; break }
            }
            step("prompt push without the approval", cleared)
        } else if let q = prompt?.question {
            // A question: a stale id is refused, the open one is answered with the first option of each question.
            try? await c.send(#"{"v":1,"type":"answer","id":"q0","pane":"\#(paneId)","question":"toolu_stale","answers":[{"picks":[0]}]}"#)
            step("answer with a stale id -> stale", errorCode(await c.untilReply().last) == .stale)
            // JSONSerialization escapes the typed text (quotes, backslashes, controls).
            let one = other.flatMap { t in (try? JSONSerialization.data(withJSONObject: ["picks": [Int](), "other": t] as [String: Any], options: [.sortedKeys]))
                .flatMap { String(data: $0, encoding: .utf8) } } ?? #"{"picks":[0]}"#
            let answers = Array(repeating: one, count: max(1, q.headers.count, q.questions.count)).joined(separator: ",")
            try? await c.send(#"{"v":1,"type":"answer","id":"q1","pane":"\#(paneId)","question":"\#(q.id)","answers":[\#(answers)]}"#)
            let r = await c.untilReply(20).last
            if q.onMac == true {
                // Read from the screen only (Claude): the Mac answers it; the hub sends nothing and the picker stays.
                step("screen-read question: marked onMac, answer -> \(r.map { isOK($0) ? "ok" : "\(String(describing: errorCode($0)))" } ?? "nothing")",
                     r.map(isOK) == false && errorCode(r) == .failed)
                try? await Task.sleep(for: .seconds(1))
                let shown = (try? await screen.visibleText(paneId: paneId)) ?? ""
                step("picker still open, nothing chosen", QuestionAdapter.claudeScreen(shown)?.question.text == q.questions.first?.text)
            } else {
                if case .reply(let rr)? = r, let e = rr.error { print("test-hub: answer error: \(e.code) \(e.message)") }
                step("answer with the open id -> \(r.map { isOK($0) ? "ok" : "\(String(describing: errorCode($0)))" } ?? "nothing")", r.map(isOK) == true)
                var cleared = false
                for _ in 0..<20 {
                    guard let m = await c.next(1) else { continue }
                    if case .prompt(let p) = m, p.question == nil { cleared = true; break }
                }
                step("prompt push without the question", cleared)
            }
        } else if prompt?.question == nil {
            let word = "hub-ok-\(Int.random(in: 1000...9999))"
            var attachments = ""
            if image, let jpeg = gpsJPEG() {
                try? await c.send(#"{"v":1,"type":"attach","id":"i","upload":"live-1","part":0,"parts":1,"data":"\#(jpeg.base64EncodedString())"}"#)
                let up = await c.untilReply(8)
                let at = up.compactMap { if case .attached(let a) = $0 { a } else { nil } }.first
                step("attach -> \(up.map(\.type)) \(at.map { "\($0.width)x\($0.height) \($0.bytes) B" } ?? "")", up.last.map(isOK) == true && at != nil)
                let newest = ((try? FileManager.default.contentsOfDirectory(at: PasteImages.directory, includingPropertiesForKeys: [.creationDateKey])) ?? [])
                    .filter { $0.lastPathComponent.hasPrefix("remote-") }
                    .max { ((try? $0.resourceValues(forKeys: [.creationDateKey]).creationDate) ?? .distantPast) < ((try? $1.resourceValues(forKeys: [.creationDateKey]).creationDate) ?? .distantPast) }
                let props = newest.flatMap { CGImageSourceCreateWithURL($0 as CFURL, nil) }.flatMap { CGImageSourceCopyPropertiesAtIndex($0, 0, nil) as? [String: Any] } ?? [:]
                let mode = newest.flatMap { try? FileManager.default.attributesOfItem(atPath: $0.path)[.posixPermissions] as? Int } ?? 0
                step("stored \(newest?.lastPathComponent ?? "nothing"): no GPS, mode \(String(mode, radix: 8))",
                     newest != nil && props[kCGImagePropertyGPSDictionary as String] == nil && mode == 0o600)
                attachments = #","attachments":["live-1"]"#
            }
            try? await c.send(#"{"v":1,"type":"send","id":"s","pane":"\#(paneId)","text":"Reply with just: \#(word)\#(image ? "\\nline two" : "")"\#(attachments)}"#)
            step("send -> ok", await c.untilReply(8).last.map(isOK) == true)
            // A pane without a transcript (CLAUDE_CODE_CHILD_SESSION) has no conversation to push; its screen still changes.
            var pushed: String?
            for _ in 0..<30 {
                guard let m = await c.next(1) else { continue }
                if case .conversation(let conv) = m, conv.mode == .append, !conv.items.isEmpty { pushed = "conversation"; break }
                if case .screen(let sc) = m, sc.text.components(separatedBy: word).count > 2 { pushed = "screen"; break }
            }
            step("\(pushed ?? "no") push after the send", pushed != nil)
            if image {
                let text = (screen is HerdrClient ? try? await model.store.client.readScrollback(paneId: paneId, lines: 40, ansi: false).text : try? await screen.visibleText(paneId: paneId)) ?? ""
                step("the agent took the image as an attachment ([Image #n] on screen)", text.contains("[Image #"))
            }
        }
        c.close()
        server.stop()
        finish(ok)
    }

    /// A small JPEG with GPS and a camera name in its metadata.
    static func gpsJPEG() -> Data? {
        guard let ctx = CGContext(data: nil, width: 64, height: 48, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                                  bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { return nil }
        ctx.setFillColor(CGColor(srgbRed: 0.2, green: 0.6, blue: 0.3, alpha: 1)); ctx.fill(CGRect(x: 0, y: 0, width: 64, height: 48))
        let out = NSMutableData()
        guard let img = ctx.makeImage(), let dest = CGImageDestinationCreateWithData(out, "public.jpeg" as CFString, 1, nil) else { return nil }
        let props: [CFString: Any] = [kCGImagePropertyGPSDictionary: [kCGImagePropertyGPSLatitude: 47.37, kCGImagePropertyGPSLatitudeRef: "N",
                                                                     kCGImagePropertyGPSLongitude: 8.54, kCGImagePropertyGPSLongitudeRef: "E"],
                                      kCGImagePropertyTIFFDictionary: [kCGImagePropertyTIFFMake: "TestCam"]]
        CGImageDestinationAddImage(dest, img, props as CFDictionary)
        return CGImageDestinationFinalize(dest) ? out as Data : nil
    }

    /// `--session <named> --test-remote`: the app's RemoteHub on 127.0.0.1 with its files in a temp folder: a new
    /// self-signed certificate, TLS with the pinned fingerprint, pairing with the 6 words, hello with the new token,
    /// read level by default, revoke. Reads only.
    static func remote(model: AppModel) async {
        try? await Task.sleep(for: .seconds(2))
        var ok = true
        func step(_ what: String, _ pass: Bool) { print("test-remote: \(pass ? "ok" : "FAIL") \(what)"); ok = ok && pass }
        let dir = URL(fileURLWithPath: NSTemporaryDirectory() + "hkremote-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        AppDefaults.store.set("127.0.0.1", forKey: RemoteHub.hostKey)
        AppDefaults.store.set(0, forKey: RemoteHub.portKey)
        let hub = RemoteHub(model: model, devices: FileDeviceStore(url: dir.appendingPathComponent("devices.json")), dir: dir)
        step("off by default", !hub.enabled && !hub.running)
        hub.enabled = true
        for _ in 0..<50 where !hub.running && hub.problem == nil { try? await Task.sleep(for: .milliseconds(100)) }
        step("on: \(hub.url ?? hub.problem ?? "nothing")", hub.running && hub.url?.hasPrefix("https://127.0.0.1:") == true)
        guard let page = hub.url.flatMap(URL.init(string:)), let url = URL(string: page.absoluteString.replacingOccurrences(of: "https://", with: "wss://") + "/ws"),
              let fp = hub.fingerprint else { finish(false); return }
        let offer = hub.startPairing()
        step("QR link carries the fingerprint", hub.pairLink?.absoluteString.hasSuffix("&fp=\(fp)") == true)

        final class Pin: NSObject, URLSessionDelegate, @unchecked Sendable {
            let pin: String
            init(_ p: String) { pin = p }
            func urlSession(_ s: URLSession, didReceive c: URLAuthenticationChallenge,
                            completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
                guard let t = c.protectionSpace.serverTrust, let leaf = (SecTrustCopyCertificateChain(t) as? [SecCertificate])?.first,
                      HubTLS.fingerprint(der: SecCertificateCopyData(leaf) as Data) == pin else { completionHandler(.cancelAuthenticationChallenge, nil); return }
                completionHandler(.useCredential, URLCredential(trust: t))
            }
        }
        let session = URLSession(configuration: .ephemeral, delegate: Pin(fp), delegateQueue: nil)
        let index = try? await session.data(from: page)
        let body = index.map { String(decoding: $0.0, as: UTF8.self) } ?? ""
        step("page over pinned https: \((index?.1 as? HTTPURLResponse)?.statusCode ?? 0)",
             (index?.1 as? HTTPURLResponse)?.statusCode == 200 && body.contains("app.js"))
        let c = Client(task: session.webSocketTask(with: url))
        try? await c.send(#"{"v":1,"type":"pair","id":"p","secret":"\#(offer.code)","name":"Test phone"}"#)
        let paired = await c.untilReply()
        guard case .paired(let pd)? = paired.first else { step("pair -> paired (\(paired.map(\.type)))", false); finish(false); return }
        step("paired as \(pd.device.name), level \(pd.device.level.rawValue)", pd.device.level == .read)
        try? await c.send(#"{"v":1,"type":"hello","id":"h","token":"\#(pd.token)","client":"test-remote","clientVersion":"1"}"#)
        let hello = await c.untilReply()
        let ws = hello.compactMap { if case .snapshot(let w) = $0 { w } else { nil } }.first
        step("hello -> \(hello.map(\.type)), \(ws?.count ?? 0) workspaces of the named session", hello.last.map(isOK) == true && (ws?.isEmpty == false))
        hub.checkPairing()
        step("offer used up, device listed", hub.offer == nil && hub.deviceList.map(\.device.name) == ["Test phone"])
        let onDisk = (try? String(contentsOf: dir.appendingPathComponent("devices.json"), encoding: .utf8)) ?? ""
        step("device file has the hash, not the token", onDisk.contains(HubToken.hash(pd.token)) && !onDisk.contains(pd.token))
        hub.revoke(device: pd.device.id)
        try? await c.send(#"{"v":1,"type":"ping","id":"x"}"#)
        let after = await c.untilReply().last
        if case .reply(let r)? = after { step("after revoke -> \(r.error?.code.rawValue ?? "ok")", r.error?.code == .unauthorized) } else { step("after revoke -> a reply", false) }
        c.close()
        hub.enabled = false
        try? await Task.sleep(for: .milliseconds(200))
        step("off again: nothing listens", !hub.running && hub.url == nil)
        session.invalidateAndCancel()
        finish(ok)
    }

    /// `--session <named> --serve-remote`: the app's RemoteHub on 127.0.0.1 (temp folder, new certificate) with one
    /// read-level test device, for the browser smoke run (`scripts/web-test.mjs --live`). Prints one JSON line, serves
    /// until stdin closes. A read device cannot send input. `SERVE_REMOTE_LEVEL=input` (the phone stills,
    /// scripts/demo/phone-stills.mjs): an input device instead, only on a demo session; the stills send nothing.
    /// The device is revoked before the hub stops.
    static func serve(model: AppModel) async {
        try? await Task.sleep(for: .seconds(2))
        let dir = URL(fileURLWithPath: NSTemporaryDirectory() + "hkserve-\(UUID().uuidString)")
        AppDefaults.store.set("127.0.0.1", forKey: RemoteHub.hostKey)
        AppDefaults.store.set(0, forKey: RemoteHub.portKey)
        // Direct mode: its own port, so a running Flockbook with remote access on (Tailscale's local port) is no clash.
        AppDefaults.store.set(RemoteHub.Mode.direct.rawValue, forKey: RemoteHub.modeKey)
        let store = FileDeviceStore(url: dir.appendingPathComponent("devices.json"))
        let token = HubToken.random()
        let input = ProcessInfo.processInfo.environment["SERVE_REMOTE_LEVEL"] == "input"
        guard !input || model.store.session.name?.contains("demo") == true else {
            print("serve-remote: FAIL an input device only on a demo session"); NSApp.terminate(nil); return
        }
        let device = input ? HubDevice(id: "live-input", name: "Demo stills", level: .input) : HubDevice(id: "live-read", name: "Live read test", level: .read)
        store.add(token: token, device: device, paired: Date())
        let hub = RemoteHub(model: model, devices: store, dir: dir)
        hub.enabled = true
        for _ in 0..<50 where !hub.running && hub.problem == nil { try? await Task.sleep(for: .milliseconds(100)) }
        guard hub.running, let port = hub.port else { print("serve-remote: FAIL \(hub.problem ?? "not running")"); hub.enabled = false; NSApp.terminate(nil); return }
        let info: [String: Any] = ["port": Int(port), "cert": dir.appendingPathComponent("tls/cert.pem").path, "token": token]
        if let d = try? JSONSerialization.data(withJSONObject: info, options: [.sortedKeys]) { print(String(decoding: d, as: UTF8.self)) }
        fflush(stdout)
        await Task.detached { while FileHandle.standardInput.availableData.count > 0 {} }.value
        store.revoke(device: device.id)
        print("serve-remote: revoked \(device.id) (\(store.device(id: device.id) == nil ? "gone" : "still there"))"); fflush(stdout)
        hub.enabled = false
        try? await Task.sleep(for: .milliseconds(200))
        try? FileManager.default.removeItem(at: dir)
        NSApp.terminate(nil)
    }

    private static func finish(_ ok: Bool) {
        print("test-hub/remote: \(ok ? "PASS" : "FAIL")")
        NSApp.terminate(nil)
    }
    private static func isOK(_ m: HubMessage) -> Bool { if case .reply(let r) = m { r.ok } else { false } }
    private static func errorCode(_ m: HubMessage?) -> HubError.Code? { if case .reply(let r)? = m { r.error?.code } else { nil } }

    @MainActor final class Client {
        let task: URLSessionWebSocketTask
        convenience init(port: UInt16) { self.init(task: URLSession.shared.webSocketTask(with: URL(string: "ws://127.0.0.1:\(port)/ws")!)) }
        init(task: URLSessionWebSocketTask) {
            self.task = task
            task.resume()
            read()
        }
        func send(_ json: String) async throws { try await task.send(.string(json)) }
        /// `receive()` ignores task cancellation, so one reader loop fills an inbox and `next` polls it.
        private var inbox: [HubMessage] = []
        private var ended = false
        private func read() {
            Task { @MainActor [weak self, task] in
                while true {
                    guard case .string(let s)? = try? await task.receive() else { self?.ended = true; return }
                    if let m = try? Hub.decodeHub(Data(s.utf8)) { self?.inbox.append(m) }
                }
            }
        }
        /// The next message, or nil after `seconds` or when the connection is closed.
        func next(_ seconds: Double = 3) async -> HubMessage? {
            let deadline = ContinuousClock.now.advanced(by: .seconds(seconds))
            while inbox.isEmpty, !ended, ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(20)) }
            return inbox.isEmpty ? nil : inbox.removeFirst()
        }
        func untilReply(_ seconds: Double = 3) async -> [HubMessage] {
            var out: [HubMessage] = []
            while let m = await next(seconds) { out.append(m); if case .reply = m { break } }
            return out
        }
        func close() { task.cancel(with: .goingAway, reason: nil) }
    }
}
