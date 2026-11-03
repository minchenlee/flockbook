import HerdrKit
import SwiftUI
import AppKit
import UniformTypeIdentifiers

/// Multi-line input. Return sends, Shift-Return inserts a newline.
/// Never sends while IME marked text is active; system dictation works because this is a real NSTextView.
struct ComposerTextView: NSViewRepresentable {
    @Binding var text: String
    @Binding var height: CGFloat
    @Binding var composing: Bool
    var pasteboard: NSPasteboard = .general
    var onPasteImages: ([URL]) -> Void = { _ in }
    /// Files dropped on the text (an image becomes a chip; see `ComposerView.attachFiles`).
    var onDropFiles: ([URL]) -> Void = { _ in }
    /// A file drag is over the text (the composer shows its drop outline).
    var onDropTarget: (Bool) -> Void = { _ in }
    var onSubmit: () -> Void
    var canSubmit = true
    var maxHeight: CGFloat = 160
    var handle: ComposerHandle? = nil

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = false
        let tv = SubmitTextView()
        tv.delegate = context.coordinator
        tv.pasteboard = pasteboard
        tv.onPasteImages = { [weak c = context.coordinator] in c?.parent.onPasteImages($0) }
        tv.drop.onFiles = { [weak c = context.coordinator] in c?.parent.onDropFiles($0) }
        tv.drop.onTarget = { [weak c = context.coordinator] in c?.parent.onDropTarget($0) }
        tv.onMarkedChange = { [weak c = context.coordinator] in c?.markedChanged() }
        tv.isRichText = false
        tv.allowsUndo = true
        tv.drawsBackground = false
        tv.font = .preferredFont(forTextStyle: .title3)   // same style as the placeholder
        tv.textContainerInset = NSSize(width: 0, height: 4)
        tv.textContainer?.lineFragmentPadding = 0
        tv.isAutomaticQuoteSubstitutionEnabled = false
        tv.isAutomaticDashSubstitutionEnabled = false
        tv.autoresizingMask = [.width]
        tv.isVerticallyResizable = true
        scroll.documentView = tv
        context.coordinator.textView = tv
        handle?.textView = tv
        DispatchQueue.main.async { context.coordinator.recalc() }
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.parent = self
        SettingsFocus.cover(scroll, context.environment.underSettings)
        guard let tv = context.coordinator.textView else { return }
        if tv.string != text, !tv.hasMarkedText() { tv.string = text; context.coordinator.recalc() }
    }

    final class SubmitTextView: NSTextView {
        /// Called after the marked-text state may have changed (IME start / update / commit).
        var onMarkedChange: (() -> Void)?
        var pasteboard: NSPasteboard = .general
        var onPasteImages: (([URL]) -> Void)?
        let drop = FileDrop()
        /// A screenshot on the clipboard holds only image types, which a plain-text view does not read, so the
        /// system would grey out Edit > Paste (and ⌘V) and never call `paste`. Enable it for images.
        override func validateUserInterfaceItem(_ item: any NSValidatedUserInterfaceItem) -> Bool {
            if item.action == #selector(NSText.paste(_:)), PasteImages.hasImage(pasteboard) { return true }
            return super.validateUserInterfaceItem(item)
        }
        override func paste(_ sender: Any?) {
            if let urls = PasteImages.capture(from: pasteboard), !urls.isEmpty { onPasteImages?(urls); return }
            if pasteboard === NSPasteboard.general { super.paste(sender); return }
            // Injected pasteboard (tests): NSTextView would read the system one.
            if let t = pasteboard.string(forType: .string) { insertText(t, replacementRange: selectedRange()) }
        }
        // When a drag reaches the text (a composer without a glass card; see `FileDropArea`): files and image promises
        // go to the composer as on its drop area. A plain-text view would insert the raw path, and take a file promise
        // (a screenshot's thumbnail) but insert nothing. Anything else (text) keeps NSTextView's own drop.
        override func draggingEntered(_ info: NSDraggingInfo) -> NSDragOperation { drop.over(info) ?? super.draggingEntered(info) }
        override func draggingUpdated(_ info: NSDraggingInfo) -> NSDragOperation { drop.over(info) ?? super.draggingUpdated(info) }
        override func draggingExited(_ info: NSDraggingInfo?) { drop.left(); super.draggingExited(info) }
        override func concludeDragOperation(_ info: NSDraggingInfo?) { drop.left(); super.concludeDragOperation(info) }
        override func performDragOperation(_ info: NSDraggingInfo) -> Bool { drop.perform(info) ?? super.performDragOperation(info) }
        override func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
            super.setMarkedText(string, selectedRange: selectedRange, replacementRange: replacementRange)
            onMarkedChange?()
        }
        override func unmarkText() { super.unmarkText(); onMarkedChange?() }
        override func insertNewline(_ sender: Any?) {
            if hasMarkedText() { super.insertNewline(sender); return }
            if NSApp.currentEvent?.modifierFlags.contains(.shift) == true {
                insertNewlineIgnoringFieldEditor(sender)
            } else {
                (delegate as? Coordinator)?.submit()
            }
        }
    }

    @MainActor final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: ComposerTextView
        weak var textView: NSTextView?
        init(_ p: ComposerTextView) { parent = p }

        func textDidChange(_ n: Notification) {
            guard let tv = textView else { return }
            if !tv.hasMarkedText() { parent.text = tv.string }
            let marked = tv.hasMarkedText()
            if parent.composing != marked { parent.composing = marked }
            recalc()
        }
        func markedChanged() {
            guard let tv = textView else { return }
            let marked = tv.hasMarkedText()
            if parent.composing != marked { parent.composing = marked }
        }
        func submit() {
            guard let tv = textView, !tv.hasMarkedText(), parent.canSubmit else { return }
            parent.text = tv.string
            parent.onSubmit()
            tv.string = ""
            parent.text = ""
            recalc()
        }
        func recalc() {
            guard let tv = textView, let lm = tv.layoutManager, let tc = tv.textContainer else { return }
            lm.ensureLayout(for: tc)
            let h = lm.usedRect(for: tc).height + tv.textContainerInset.height * 2
            let clamped = min(max(h, 26), parent.maxHeight)
            if abs(parent.height - clamped) > 0.5 { DispatchQueue.main.async { self.parent.height = clamped } }
        }
    }
}

/// Lets the toolbar buttons reach the NSTextView (insert text, take focus).
@MainActor final class ComposerHandle {
    weak var textView: NSTextView?
    func insert(_ s: String) {
        guard let tv = textView else { return }
        tv.window?.makeFirstResponder(tv)
        tv.insertText(s, replacementRange: tv.selectedRange())
    }
    func focus() { if let tv = textView { tv.window?.makeFirstResponder(tv) } }
    /// A file's path for the message: as is when plain, else single-quoted (`ShellQuote`, per scalar).
    static func quoted(_ url: URL) -> String { ShellQuote.quotedIfNeeded(url.path) }
}

/// Plain icon button with a hover background.
struct HoverIconStyle: ButtonStyle {
    @State private var hover = false
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(enabled ? 1 : 0.35)   // the system's disabled look; a plain style does not dim by itself
            .frame(width: 28, height: 28)
            .background(Color.primary.opacity(!enabled ? 0 : configuration.isPressed ? 0.14 : hover ? 0.08 : 0), in: Circle())
            .contentShape(Circle())
            .onHover { hover = $0 }
    }
}

/// Unsent composer content for one pane.
struct ComposerDraft: Equatable {
    var text = ""
    var images: [URL] = []
    var isEmpty: Bool { text.isEmpty && images.isEmpty }
}

struct ComposerView: View {
    @Environment(\.underSettings) private var underSettings
    let agentName: String
    var readOnly = false
    var branch: String? = nil
    var onSend: (String) -> Void
    var onDraft: (ComposerDraft) -> Void = { _ in }
    /// When set, sends through this instead of `onSend` and learns whether herdr took the message. On failure the
    /// text and images come back into the composer (unless the owner already typed something new) with a note.
    var deliver: ((String) async -> Bool)? = nil
    /// Like `deliver`, with a third outcome: `unconfirmed` (the request went out, no answer in time) keeps the text out
    /// of the composer and says "sent, not confirmed". Takes precedence over `deliver`.
    var deliverResult: ((String) async -> DeliveryOutcome)? = nil
    /// Overrides the "Reply to <agent>" placeholder.
    var placeholder: String? = nil
    /// Changes when the owner asks for the field (an unfocused pane's composer was clicked): take keyboard focus.
    var focusRequest = 0
    /// The agent's host when it is not this Mac: attached and dropped files are copied there first (`RemoteUpload`).
    var remote: RemoteHost? = nil
    /// The host folder for attached files (checks use a temp folder).
    var uploadFolder = RemoteUpload.defaultFolder
    @State private var text: String
    @State private var images: [URL]
    var pasteboard: NSPasteboard = .general
    init(agentName: String, readOnly: Bool = false, branch: String? = nil, draft: ComposerDraft = ComposerDraft(),
         onSend: @escaping (String) -> Void, onDraft: @escaping (ComposerDraft) -> Void = { _ in }, probe: PlaceholderProbe? = nil, pasteboard: NSPasteboard = .general,
         deliver: ((String) async -> Bool)? = nil, deliverResult: ((String) async -> DeliveryOutcome)? = nil, placeholder: String? = nil, focusRequest: Int = 0, remote: RemoteHost? = nil,
         uploadFolder: String = RemoteUpload.defaultFolder) {
        self.pasteboard = pasteboard; self.deliver = deliver; self.deliverResult = deliverResult; self.placeholder = placeholder; self.focusRequest = focusRequest
        self.remote = remote; self.uploadFolder = uploadFolder
        self.agentName = agentName; self.readOnly = readOnly; self.branch = branch
        self.onSend = onSend; self.onDraft = onDraft; self.probe = probe
        _text = State(initialValue: draft.text); _images = State(initialValue: draft.images)
    }
    @State private var height: CGFloat = 26
    @State private var composing = false
    /// Test hook: tells whether the placeholder view is currently in the hierarchy.
    var probe: PlaceholderProbe? = nil
    @State private var handle = ComposerHandle()
    @State private var dropping = false
    @State private var sendError: String?
    /// Attached files being copied to the host.
    @State private var uploading = 0
    /// Text the failed send gave back; the note goes once the owner edits it.
    @State private var returnedText = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !images.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(images, id: \.self) { url in PasteChip(url: url) { removeImage(url) } }
                    }
                }
            }
            ZStack(alignment: .topLeading) {
                if text.isEmpty && !composing {
                    Text(readOnly ? "Read-only: input is off" : placeholder ?? "Reply to \(agentName)").font(.title3).foregroundStyle(Color(nsColor: .placeholderTextColor))
                        .padding(.top, 4).allowsHitTesting(false)
                        .onAppear { probe?.shown = true }.onDisappear { probe?.shown = false }
                }
                ComposerTextView(text: $text, height: $height, composing: $composing, pasteboard: pasteboard,
                                 onPasteImages: { images.append(contentsOf: $0) }, onDropFiles: { attachFiles($0) }, onDropTarget: { dropping = $0 },
                                 onSubmit: { send() }, canSubmit: !readOnly && !underSettings, handle: handle)
                    .frame(height: height)
            }
            if uploading > 0, let remote {
                Label("Copying to \(remote.label)…", systemImage: "arrow.up.circle")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let sendError {
                Label(sendError, systemImage: "exclamationmark.circle")
                    .font(.caption).foregroundStyle(.red)
                    .transition(.opacity)
            }
            HStack(spacing: 10) {
                Button { attach() } label: { Image(systemName: "plus").foregroundStyle(.secondary) }
                    .buttonStyle(HoverIconStyle()).help(remote.map { "Attach file (an image attaches, another file is copied to \($0.label) and its path inserted there)" } ?? "Attach file (an image attaches, another file inserts its path)").disabled(readOnly)
                if let branch {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.triangle.branch").font(.subheadline)
                        Text(branch).font(.caption).lineLimit(1).truncationMode(.tail)
                    }
                    .foregroundStyle(.secondary).help(branch)
                }
                Spacer(minLength: 8)
                Button {
                    handle.focus()
                    NSApp.sendAction(Selector(("startDictation:")), to: nil, from: nil)
                } label: { Image(systemName: "mic").foregroundStyle(.secondary) }
                    .buttonStyle(HoverIconStyle()).help("Dictate").disabled(readOnly)
                Button(action: send) {
                    Image(systemName: "arrow.up")
                        .font(.body.weight(.bold))
                        .foregroundStyle(canSend ? AnyShapeStyle(.white) : AnyShapeStyle(.tertiary))
                        .frame(width: 28, height: 28)
                        .background(Circle().fill(canSend ? Color.accentColor : Color.primary.opacity(0.08)))
                }
                .buttonStyle(.plain).disabled(!canSend).help("Send (Return)")
            }
        }
        .onChange(of: dropping) { probe?.dropping = dropping }
        .onChange(of: text) {
            onDraft(ComposerDraft(text: text, images: images))
            if sendError != nil, !text.isEmpty, text != returnedText { withAnimation(.easeOut(duration: 0.2)) { sendError = nil } }
            probe?.text = text
        }
        .onChange(of: sendError) { probe?.error = sendError }
        .onChange(of: focusRequest) { DispatchQueue.main.async { handle.focus() } }
        .onChange(of: images) { onDraft(ComposerDraft(text: text, images: images)); probe?.images = images }
        .onAppear {
            probe?.images = images; probe?.removeFirst = { if let u = images.first { removeImage(u) } }
            probe?.drop = { attachFiles($0) }
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .glassCard(radius: Radius.bubble)
        // The drop outline is the card's own edge (same corners), over everything in it (the chips' labels too).
        .overlay {
            if dropping {
                RoundedRectangle(cornerRadius: Radius.bubble).strokeBorder(Color.accentColor, lineWidth: 2)
                    .background { GeometryReader { g in Color.clear.onAppear { DropOutlineProbe.outline = g.frame(in: .global) } } }
            }
        }
        .background { GeometryReader { g in Color.clear.onAppear { DropOutlineProbe.card = g.frame(in: .global) }.onChange(of: g.frame(in: .global)) { DropOutlineProbe.card = $1 } } }
        .background { FileDropArea(onFiles: readOnly ? nil : { attachFiles($0) }, onTarget: { dropping = $0 }) }
        .shadow(color: .black.opacity(0.08), radius: 10, y: 4)
    }

    private func attach() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = true
        guard panel.runModal() == .OK else { return }
        attachFiles(panel.urls)
    }
    /// Image files become chips (`PasteImages.take`; for an agent on a host the send copies them there, see
    /// `MessageSender`); other files and folders insert their path (`insertFiles`).
    private func attachFiles(_ urls: [URL]) {
        let (taken, rest) = PasteImages.take(urls)
        images.append(contentsOf: taken)
        if !rest.isEmpty { insertFiles(rest) }
    }
    /// Types the paths of attached or dropped files. For an agent on a host each file is copied there first and its
    /// host path is typed; a folder is not copied (the note says so).
    private func insertFiles(_ urls: [URL]) {
        guard let remote else {
            handle.insert(urls.map { ComposerHandle.quoted($0) }.joined(separator: " ") + " ")
            return
        }
        sendError = nil
        uploading += 1
        Task {
            defer { uploading -= 1 }
            var paths: [String] = []
            for u in urls {
                do { paths.append(try await RemoteUpload(host: remote, folder: uploadFolder).uploadFile(u)) } catch {
                    withAnimation(.easeOut(duration: 0.2)) { sendError = "\(error)" }
                }
            }
            if !paths.isEmpty {
                handle.insert(paths.map { ComposerHandle.quoted(URL(fileURLWithPath: $0)) }.joined(separator: " ") + " ")
            }
        }
    }
    /// Not while attached files are still being copied to the host: their paths are not in the text yet.
    private var canSend: Bool { !readOnly && uploading == 0 && !(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && images.isEmpty) }
    /// Message = text, then the absolute image paths (agents read an image path as an attachment).
    private func send() {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !readOnly, !underSettings, uploading == 0, !(t.isEmpty && images.isEmpty) else { return }   // the draft stays
        let paths = images.map { ComposerHandle.quoted($0) }.joined(separator: " ")
        let message = [t, paths].filter { !$0.isEmpty }.joined(separator: " ")
        let sent = (text: text, images: images)
        text = ""; images = []   // files stay: the agent reads them; they are purged after 7 days
        sendError = nil
        let deliverResult = self.deliverResult ?? deliver.map { d in { (m: String) async -> DeliveryOutcome in await d(m) ? .sent : .failed } }
        guard let deliverResult else { onSend(message); return }
        Task {
            switch await deliverResult(message) {
            case .sent: return
            case .unconfirmed:
                returnedText = ""
                withAnimation(.easeOut(duration: 0.2)) { sendError = Self.unconfirmedNote }
                return
            case .failed: break
            }
            if text.isEmpty && images.isEmpty { text = sent.text; images = sent.images }
            returnedText = text
            let why = MessageSender.takeFailure(message).map { "Not sent. \($0) Press Return to try again." }
            withAnimation(.easeOut(duration: 0.2)) { sendError = why ?? "Not sent: herdr did not take the message. Press Return to try again." }
        }
    }
    static let unconfirmedNote = "Sent, not confirmed: no answer in time. Check the terminal before you send it again."

    private func removeImage(_ url: URL) {
        PasteImages.remove(url)
        images.removeAll { $0 == url }
    }
}

/// Checks: where the composer's drop outline was drawn, and the card's frame (window coordinates, from SwiftUI).
@MainActor enum DropOutlineProbe { static var outline = CGRect.zero; static var card = CGRect.zero }

@MainActor final class PlaceholderProbe { var shown = false; var images: [URL] = []; var text = ""; var error: String?; var removeFirst: (() -> Void)?; var drop: (([URL]) -> Void)?; var dropping = false }
