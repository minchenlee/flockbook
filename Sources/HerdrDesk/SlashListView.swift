import HerdrKit
import SwiftUI
import AppKit

/// The "/" list above the reply box (`SlashList`): at most `SlashList.rows` rows, the picked one marked; a row shows
/// the command (or value), its argument hint, its description (or the value's label), and "needs a value" for a
/// command that opens a screen when sent bare. Under the rows, "4–11 of 20" when there are more. A click completes
/// the row; the wheel or trackpad over the list moves its window.
struct SlashListView: View {
    let list: SlashList
    var onPick: (SlashList.Item) -> Void
    var onWheel: (Double) -> Void
    @State private var hover: Int?

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(list.shown.enumerated()), id: \.element) { j, item in
                let i = list.top + j
                Button { onPick(item) } label: { row(item, marked: i == list.pick || i == hover) }
                    .buttonStyle(.plain)
                    .onHover { hover = $0 ? i : (hover == i ? nil : hover) }
                    .accessibilityLabel(list.title(item))
            }
            if let count = list.count {
                Text(count).font(.subheadline).foregroundStyle(.tertiary).padding(.horizontal, 10).padding(.top, 2)
            }
        }
        .padding(4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .fixedSize(horizontal: false, vertical: true)
        .glassCard(radius: Radius.card)
        .compositingGroup()   // one shadow for the card, none under each text
        .shadow(color: .black.opacity(0.08), radius: 10, y: 4)
        .background { WheelCatcher(onWheel: onWheel) }
        .onChange(of: list.top) { hover = nil }   // the rows moved under the pointer
    }

    private func row(_ item: SlashList.Item, marked: Bool) -> some View {
        HStack(spacing: 10) {
            Text(list.title(item)).font(.body.weight(.semibold)).lineLimit(1).fixedSize()
            if !item.hint.isEmpty { Text(item.hint).font(.callout).foregroundStyle(.tertiary).lineLimit(1).fixedSize() }
            Text(item.description).font(.callout).foregroundStyle(.secondary).lineLimit(1).truncationMode(.tail)
                .frame(maxWidth: .infinity, alignment: .leading)
            // Sent bare, it opens a screen in the terminal (a picker, settings): it goes out only with a value.
            if item.screen { Text("needs a value").font(.subheadline).foregroundStyle(.tertiary).lineLimit(1).fixedSize() }
        }
        .padding(.horizontal, 10)
        .frame(height: SlashList.rowHeight)
        .background(Color.primary.opacity(marked ? 0.08 : 0), in: RoundedRectangle(cornerRadius: Radius.control + 2))
        .contentShape(Rectangle())
    }
}

/// Takes the wheel over the list (SwiftUI has no wheel event): a window-level monitor, only for events over this
/// view. It never takes a click (the rows are SwiftUI's).
private struct WheelCatcher: NSViewRepresentable {
    var onWheel: (Double) -> Void
    func makeNSView(context: Context) -> Catcher { Catcher() }
    func updateNSView(_ v: Catcher, context: Context) { v.onWheel = onWheel }

    final class Catcher: NSView {
        var onWheel: ((Double) -> Void)?
        nonisolated(unsafe) private var monitor: Any?
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let m = monitor { NSEvent.removeMonitor(m); monitor = nil }
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] e in
                guard let self, !self.isHiddenOrHasHiddenAncestor, !SettingsFocus.covered(self), let p = eventPoint(e, in: self), self.bounds.contains(p) else { return e }
                // A mouse wheel counts lines: a row each.
                if e.scrollingDeltaY != 0 { self.onWheel?(e.hasPreciseScrollingDeltas ? e.scrollingDeltaY : e.scrollingDeltaY * SlashList.rowHeight) }
                return nil
            }
        }
        deinit { if let m = monitor { NSEvent.removeMonitor(m) } }
    }
}
