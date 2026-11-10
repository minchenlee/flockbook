import HerdrKit
import SwiftUI

extension AgentStatus {
    var label: String {
        switch self {
        case .idle: "Idle"
        case .working: "Working"
        case .blocked: "Needs you"
        case .done: "Done"
        case .unknown: "Unknown"
        }
    }
}

/// One shape + one color per state (DESIGN §6).
struct StatusGlyph: View {
    let status: AgentStatus
    var size: CGFloat = 14
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        switch status {
        case .blocked:
            Image(systemName: "exclamationmark.circle.fill")
                .font(.system(size: size))
                .foregroundStyle(.orange)
        case .done:
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: size))
                .foregroundStyle(.green)
        case .working:
            SpinnerArc(reduceMotion: reduceMotion).frame(width: size - 2, height: size - 2).frame(width: size, height: size)
        case .idle, .unknown:
            Image(systemName: "circle")
                .font(.system(size: size - 1))
                .foregroundStyle(.tertiary)
        }
    }
}

/// The spinner: a quarter arc that turns over a faint fixed ring of the same colour, by a Core Animation rotation. The
/// render server runs it, so the app does no per-frame work, and it stops by itself while the window is not drawn.
/// Reduce Motion: static. The same drawing and speed as the Herdr app mod's (`icons/spinner.svg`, `SPIN_MS`).
struct SpinnerArc: NSViewRepresentable {
    var reduceMotion = false
    /// The arc's colour: the working blue, or the secondary text colour where something is waited for (`Spinner`).
    var tint = NSColor.systemBlue

    /// One turn, in seconds.
    static let turn = 0.9
    /// The arc's part of the ring, and the ring's opacity against the arc.
    static let arc = 0.25
    static let ringOpacity = 0.25
    /// The line for a spinner `side` points wide (1.8 on 16), not thinner than 1.25.
    static func lineWidth(side: CGFloat) -> CGFloat { max(1.25, side * 1.8 / 16) }

    func makeNSView(context: Context) -> ArcView { ArcView() }
    func updateNSView(_ v: ArcView, context: Context) { v.spinning = !reduceMotion; v.tint = tint }

    final class ArcView: NSView {
        let ring = CAShapeLayer(), arc = CAShapeLayer()
        var spinning = true { didSet { if spinning != oldValue { updateAnimation() } } }
        var tint = NSColor.systemBlue { didSet { if tint != oldValue { updateColors() } } }
        override init(frame: NSRect) {
            super.init(frame: frame)
            wantsLayer = true
            for l in [ring, arc] { l.fillColor = nil; l.lineCap = .round; layer?.addSublayer(l) }
            arc.strokeEnd = SpinnerArc.arc
        }
        required init?(coder: NSCoder) { fatalError() }
        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }
        override func layout() {
            super.layout()
            let width = SpinnerArc.lineWidth(side: min(bounds.width, bounds.height))
            let r = bounds.insetBy(dx: width / 2, dy: width / 2)
            for l in [ring, arc] { l.frame = bounds; l.lineWidth = width; l.path = CGPath(ellipseIn: r, transform: nil) }
            arc.anchorPoint = CGPoint(x: 0.5, y: 0.5); arc.position = CGPoint(x: bounds.midX, y: bounds.midY)
            updateColors(); updateAnimation()
        }
        override func viewDidChangeEffectiveAppearance() { super.viewDidChangeEffectiveAppearance(); updateColors() }
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); updateAnimation() }
        private func updateColors() {
            effectiveAppearance.performAsCurrentDrawingAppearance {
                let c = tint.cgColor
                arc.strokeColor = c
                ring.strokeColor = c.copy(alpha: c.alpha * SpinnerArc.ringOpacity)
            }
        }
        private func updateAnimation() {
            guard spinning, window != nil else { arc.removeAnimation(forKey: "spin"); return }
            guard arc.animation(forKey: "spin") == nil else { return }
            let a = CABasicAnimation(keyPath: "transform.rotation.z")
            a.fromValue = 0; a.toValue = -2 * Double.pi   // clockwise on screen (layer y is up)
            a.duration = SpinnerArc.turn; a.repeatCount = .infinity; a.isRemovedOnCompletion = false
            arc.add(a, forKey: "spin")
        }
    }
}

/// The spinner beside a text or a button while something is waited for (loading, a test, a send), in place of the
/// system's: the working glyph's arc in the secondary text colour, the size of the system's small or mini one.
struct Spinner: View {
    enum Size: CGFloat { case mini = 12, small = 16 }
    var size = Size.small
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    var body: some View {
        SpinnerArc(reduceMotion: reduceMotion, tint: .secondaryLabelColor)
            .frame(width: size.rawValue, height: size.rawValue)
            .accessibilityElement().accessibilityLabel("In progress")
    }
}

struct BlockedBadge: View {
    let count: Int
    var body: some View {
        Text("\(count)")
            .font(.caption.weight(.semibold)).monospacedDigit()
            .foregroundStyle(.white)
            .padding(.horizontal, 6).frame(minWidth: 18, minHeight: 18)
            .background(Color.orange, in: Capsule())
    }
}

struct Monogram: View {
    let text: String
    var inverted = false
    var body: some View {
        let letters = Monogram.letters(text)
        Text(letters)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(inverted ? Color(nsColor: .windowBackgroundColor) : Color.primary.opacity(0.7))
            .frame(width: 22, height: 22)
            .background(inverted ? Color.primary.opacity(0.8) : Color.primary.opacity(0.1),
                        in: RoundedRectangle(cornerRadius: Radius.control))
    }
    /// Name without a leading "[TAG] ". Falls back to the raw name when nothing remains.
    static func displayName(_ raw: String) -> String {
        guard raw.hasPrefix("["), let e = raw.firstIndex(of: "]") else { return raw }
        let rest = String(raw[raw.index(after: e)...]).trimmingCharacters(in: .whitespaces)
        return rest.isEmpty ? raw : rest
    }
    static func letters(_ raw: String) -> String { WorkspaceMonogram.letters(raw) }
}

extension Tab {
    var isSplit: Bool { panes.count > 1 }
    var primaryAgent: Agent? { panes.compactMap(\.agent).first }
}

extension Workspace {
    var blockedCount: Int { tabs.filter { $0.status == .blocked }.count }
    var workingCount: Int { tabs.filter { $0.status == .working }.count }
}

func role(for agentName: String) -> (symbol: String, name: String) {
    switch agentName.split(separator: "-").last.map(String.init) ?? "" {
    case "orch": ("sparkles", "Orchestrator")
    case "dev": ("chevron.left.forwardslash.chevron.right", "Developer")
    case "rev": ("eye", "Reviewer")
    case "res": ("book", "Researcher")
    case "des": ("pencil.and.outline", "Designer")
    default: ("person.crop.circle", "Agent")
    }
}
