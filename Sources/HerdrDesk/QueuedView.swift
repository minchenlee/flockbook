import HerdrKit
import SwiftUI

/// The agents' own queues (QueuedInput): read while the agent works, shown faded over the composer, pulled back into it.
extension AppModel {
    /// Pull back reads herdr's pane layout (`pane.layout`), which herdr-core has no match for: refused on hc, no keys sent.
    static let hcPullNote = "Pull back works on herdr server panes only. Nothing was sent."

    /// READ-ONLY: the pane's queue from its screen and log. Nothing unless the agent works and has a queue Flockbook reads.
    func checkQueue(for pane: Pane) async {
        deliverPulled(pane: pane.id)
        guard let a = pane.agent, a.status == .working, QueuedInput.reads(kind: a.kind) else {
            if queued[pane.id] != nil { queued[pane.id] = nil }
            return
        }
        let read: QueuedInput?
        if let q = QueueProbe.injected[pane.id] { read = q }   // offscreen shots only (empty in the app)
        else {
            let screen = try? await screen(for: pane)?.visibleText(paneId: pane.id)
            read = QueuedInput.read(agent: a, screen: screen, log: store.queuedInLog(for: a.sessionId))
        }
        let q = read.flatMap { $0.items.isEmpty ? nil : $0 }
        if queued[pane.id] != q { queued[pane.id] = q }
    }

    /// User action only (Pull back): QueuePull takes the agent's queue back (Up or Shift-Left, then Ctrl-U; never Escape)
    /// and the text goes into this pane's composer. It reads the pane before and after the keys; when the turn stopped
    /// or the keys were refused, a notice says so and the log keeps it.
    func pullQueued(_ pane: Pane) {
        guard writesEnabled, let a = pane.agent else { return }
        guard pane.backend == .tui else { notice = Self.hcPullNote; return }
        let id = pane.id
        // The pane's lock for the whole pull: no send (Mac or phone) types into the box meanwhile.
        guard claim(id) else { notice = "A message to this pane is still going out. Pull back again in a moment."; return }
        pulling.insert(id)
        Task {
            defer { pulling.remove(id); release(id) }
            let io = HerdrQueuePullIO(client: store.client, paneId: id, allowed: { [weak self] in self?.writesEnabled == true })
            // Kept before the first Ctrl-U, until this composer sends it (a quit before: `deliverPulled`).
            let entry = UUID().uuidString, texts = pulledTexts
            let r = await QueuePull.run(expect: a, log: store.queuedInLog(for: a.sessionId), io: io) { text in
                texts.keep(PulledText(id: entry, pane: id, device: PulledText.mac, text: text, note: PulledText.unfinished))
            }
            if let t = r.text {
                if texts.all.contains(where: { $0.id == entry }) {
                    delivered.insert(entry)
                    texts.finish(id: entry, note: r.partialReason, interrupted: r.isInterrupted)
                }
                putInComposer(t, pane: id)
            }
            switch r {
            case .pulled(let t):
                noticeLog.info("queue pull pane \(id, privacy: .public): pulled \(t.count) chars")
            case .interrupted:
                noticeLog.error("queue pull pane \(id, privacy: .public): the turn stopped after the keys")
                notice = "The agent's turn stopped after the pull. Your text is in the composer."
            case .partial(_, let why):
                noticeLog.error("queue pull pane \(id, privacy: .public): stopped early: \(why, privacy: .public)")
                notice = "Your text is in the composer. " + why
            case .refused(let why):
                noticeLog.info("queue pull pane \(id, privacy: .public): refused: \(why, privacy: .public)")
                notice = why
            case .failed(let why):
                noticeLog.error("queue pull pane \(id, privacy: .public): failed: \(why, privacy: .public)")
                notice = why
            }
            QueueProbe.report?(r)
            await store.refresh()
            if let now = store.workspaces.flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == id }) { await checkQueue(for: now) }
        }
    }

    /// Pulled texts of this pane the Mac's composer does not have (the app quit before that composer sent them), into
    /// the composer, once per run. Not while a pull or a send of the pane runs (a pull delivers its own).
    func deliverPulled(pane id: String) {
        guard !pulling.contains(id), !busy.contains(id) else { return }
        for p in pulledTexts.pending(device: PulledText.mac) where p.pane == id && !delivered.contains(p.id) {
            delivered.insert(p.id)
            putInComposer(p.text, pane: id)
            if let why = p.note { notice = "Pulled-back text is in the composer again. " + why }
        }
    }

    /// The pane's composer sent its draft: the kept texts this run put there went with it.
    func composerSent(pane id: String) {
        for p in pulledTexts.pending(device: PulledText.mac) where p.pane == id && delivered.contains(p.id) {
            _ = pulledTexts.take(id: p.id, device: PulledText.mac)
            delivered.remove(p.id)
        }
    }

    /// Where the app keeps pulled texts: one file per herdr session (pane ids are per session).
    static func pulledTextsFile(session: String?) -> URL {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        let name = session.map { "pulled-" + $0.unicodeScalars.map { CharacterSet.alphanumerics.contains($0) || $0 == "-" ? String($0) : "_" }.joined() } ?? "pulled"
        return support.appendingPathComponent("Flockbook/\(name).json")
    }

    /// Adds `text` after what the composer holds and starts the composer again from that draft.
    func putInComposer(_ text: String, pane id: String) {
        var d = drafts[id] ?? ComposerDraft()
        d.text = d.text.isEmpty ? text : d.text + "\n" + text
        drafts[id] = d
        composerResets[id, default: 0] += 1
    }
}

/// Test hook for the live check (`--test-queue-live`): what the last pull returned. nil in the app.
/// `injected`: queues by pane id for offscreen shots, read instead of the screen. Empty in the app.
@MainActor enum QueueProbe {
    static var report: ((QueuePull.Outcome) -> Void)?
    static var injected: [String: QueuedInput] = [:]
    /// Each queued bubble's frame in global coordinates, for the alignment check. nil in the app.
    static var bubble: ((CGRect) -> Void)?
}

/// The agent's queued messages, faded user bubbles over the composer, with when they go in and Pull back.
struct QueuedStrip: View {
    let queue: QueuedInput
    let canPull: Bool
    let pulling: Bool
    let onPull: () -> Void
    @AppStorage(ReadingSize.key, store: AppDefaults.store) private var sizeRaw = ReadingSize.m.rawValue
    @AppStorage(ReadingWidth.key, store: AppDefaults.store) private var widthRaw = ReadingWidth.medium.rawValue   // re-renders when the width changes
    private var size: ReadingSize { ReadingSize(rawValue: sizeRaw) ?? .m }

    private var when: String {
        queue.agent == "claude" ? "Goes in after the current step or when the turn ends." : "Goes in when this turn ends."
    }
    private var pullLabel: String { queue.pullsAll ? "Pull back to edit" : "Pull back the newest" }
    /// What the pull does to the queue, next to the button.
    private var pullNote: String {
        if queue.pullsAll { return queue.items.count > 1 ? "All \(queue.items.count) come back as one message." : "It comes back to the composer." }
        return "Sending it again adds it to the turn now running."
    }
    private var blocked: String? {
        if !queue.pullsAll { return [when, QueuedInput.editHint(agent: queue.agent)].compactMap { $0 }.joined(separator: " ") }
        if !queue.pullable { return "The agent's input box has text. Clear it in the terminal to pull back." }
        if queue.items.contains(where: { $0 == nil }) { return "The queued text cannot be read yet." }
        return nil
    }

    var body: some View {
        let width = ReadingWidth(rawValue: widthRaw) ?? .medium
        ZeroWidthAs(width: size.column + 2 * width.sidePadding) { strip(width) }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("Queued messages")
    }

    private func strip(_ width: ReadingWidth) -> some View {
        VStack(alignment: .trailing, spacing: 6) {
            ForEach(Array(queue.items.enumerated()), id: \.offset) { _, item in
                HStack {
                    Spacer(minLength: min(size.column * 0.2, 120))
                    bubble(item)
                }
            }
            HStack(spacing: 8) {
                Text(blocked ?? "\(when) \(pullNote)").font(.caption).foregroundStyle(.secondary)
                    .multilineTextAlignment(.trailing)
                if pulling { ProgressView().controlSize(.small) }
                // Only Claude Code's queue can be taken back exactly (its log); Codex's and Grok's are edited in their terminals.
                if queue.pullsAll {
                    Button(pullLabel, action: onPull)
                        .controlSize(.small)
                        .disabled(!canPull || blocked != nil || pulling)
                        .help("Takes the queue back from the agent into this composer. The turn keeps running.")
                }
            }
        }
        // The conversation's own column, so a queued bubble ends where the user's bubbles end.
        .readingColumn(size, width, alignment: .trailing)
        .padding(.top, 6)
    }

    private func bubble(_ text: String?) -> some View {
        VStack(alignment: .trailing, spacing: 4) {
            Text("Queued").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
            Text(text ?? "1 queued message").font(text == nil ? .body.italic() : .body)
                .foregroundStyle(.secondary).textSelection(.enabled)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .leading)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 14).padding(.vertical, 8)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: Radius.bubble))
        .overlay(RoundedRectangle(cornerRadius: Radius.bubble).strokeBorder(Color.primary.opacity(0.22), style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
        .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { QueueProbe.bubble?($0) }
        .frame(maxWidth: size.column * 0.8, alignment: .trailing)
        .fixedSize(horizontal: false, vertical: true)
    }
}

/// SwiftUI asks a pane's minimum height with a zero width (the split view's minimum). The strip's texts would wrap a
/// letter per line there and push the composer off the window, so a zero width is measured at `width` (the reading
/// column) instead. Every other proposal lays the strip out as it is.
struct ZeroWidthAs: Layout {
    let width: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let strip = subviews.first else { return .zero }
        guard proposal.width == 0 else { return strip.sizeThatFits(proposal) }
        return CGSize(width: 0, height: strip.sizeThatFits(ProposedViewSize(width: width, height: proposal.height)).height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
    }
}
