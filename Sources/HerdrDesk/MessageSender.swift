import HerdrKit
import Foundation

/// The composer's send (`MessageSend`) with the app's image folder.
@MainActor enum MessageSender {
    static func hasImagePath(_ text: String) -> Bool {
        ImagePaste.plan(text, directory: PasteImages.directory.path).contains { if case .paste = $0 { true } else { false } }
    }

    /// Why a send of this message text failed, when there is more to say than "herdr did not take it" (an image that
    /// could not reach the host). Read once by the composer.
    private static var failures: [String: String] = [:]
    static func takeFailure(_ message: String) -> String? { failures.removeValue(forKey: message) }

    /// True when the message was submitted (see `MessageSend.send`). A remote session (`HerdrSession.remote`) copies
    /// each image to the host first (`RemoteUpload`) and pastes the host path.
    @discardableResult
    static func send(_ text: String, to paneId: String, client: HerdrClient, kind: String?, hc: HCClient? = nil,
                     gate: KeyGate? = nil, maxWait: Duration = .seconds(5)) async -> Bool {
        if HCMapper.address(fromPaneId: paneId) != nil { return await deliverHC(text, to: paneId, hc: hc) != .failed }
        // `session` is an actor `let` of another module: read with await.
        guard let host = await client.session.remote else {
            return await MessageSend.send(text, to: paneId, client: client, kind: kind, directory: PasteImages.directory.path, gate: gate, maxWait: maxWait)
        }
        let steps: [ImagePaste.Step]
        do {
            steps = try await RemoteUpload(host: host).hostSteps(ImagePaste.plan(text, directory: PasteImages.directory.path))
        } catch {
            if failures.count > 20 { failures.removeAll() }
            failures[text] = "\(error)"
            return false
        }
        return await MessageSend.send(steps: steps, text: text, to: paneId, client: client, kind: kind, gate: gate, maxWait: maxWait)
    }

    /// An hc pane (`hc:` id) goes through `hc`'s `terminal.send`: it pastes and presses Enter once nobody has typed for
    /// ~3 s, so the image-attachment polling is not needed there. Claude gets a pasted image path inside the text as
    /// text and reads the file itself; only a bare path becomes `[Image #1]`. It may answer late: past its timeout the
    /// outcome is `unconfirmed` (the core may still deliver it; the text must not come back to the composer). Pasted
    /// images go beside a terminal on another machine first (`terminal.put`). No hc client: `failed`.
    static func deliverHC(_ text: String, to paneId: String, hc: HCClient?) async -> DeliveryOutcome {
        guard let hc else { return .failed }
        return await hc.deliver(text: text, to: paneId, files: HCClient.localFiles(in: text, under: PasteImages.directory.path))
    }
}
