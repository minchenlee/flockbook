import AppKit
import HerdrKit
import SwiftUI

/// macOS 15 support. The macOS 26 chrome (Liquid Glass, soft scroll edges, safe-area bars) and the look drawn before
/// 26: material for glass, and the window surface under the bars and the toolbar, so no text shows through them.
/// The views here pick one; the rest of the app uses them and never asks for the system version itself.
enum LegacyLook {
    /// Forced on 26 to see the pre-26 look there (`--legacy-look`, FLOCKBOOK_LEGACY_LOOK=1). Off on a normal launch.
    static let forced = LaunchGuard.legacyLookForced(ProcessInfo.processInfo.arguments, environment: ProcessInfo.processInfo.environment)
    /// The pre-26 look: before macOS 26, or forced.
    static var on: Bool {
        if #available(macOS 26, *) { return forced }
        return true
    }
}

/// The system sidebar's place, which the system decides, not the look: 26 floats the panel 8 pt in from the window's
/// edges with rounded corners; before 26 it is docked at the edges, square. Our peek panel and full-screen toggle match it.
enum SystemSidebar {
    static var inset: CGFloat {
        if #available(macOS 26, *) { return 8 }
        return 0
    }
    static var cornerRadius: CGFloat { inset > 0 ? 22 : 0 }
}

private struct LegacyLookKey: EnvironmentKey { static let defaultValue = LegacyLook.on }
extension EnvironmentValues {
    /// The pre-26 look. Checks set it on one view to draw both looks in one run.
    var legacyLook: Bool { get { self[LegacyLookKey.self] } set { self[LegacyLookKey.self] = newValue } }
}

/// Glass in `shape` on 26; before, a material fill with a hairline edge (and the tint over it).
struct GlassSurface<S: Shape>: ViewModifier {
    @Environment(\.legacyLook) private var legacy
    let shape: S
    var tint: Color?
    var interactive = false
    func body(content: Content) -> some View {
        if #available(macOS 26, *), !legacy {
            content.glassEffect(glass, in: shape)
        } else {
            content
                .background {
                    ZStack {
                        shape.fill(.regularMaterial)
                        if let tint { shape.fill(tint.opacity(0.3)) }
                    }
                }
                .overlay(shape.stroke(Color.primary.opacity(0.1)))
        }
    }
    @available(macOS 26, *) private var glass: Glass {
        var g = Glass.regular
        if let tint { g = g.tint(tint) }
        if interactive { g = g.interactive() }
        return g
    }
}

/// `GlassEffectContainer` on 26 (glass shapes in it blend); before, the content alone.
struct GlassGroup<Content: View>: View {
    @Environment(\.legacyLook) private var legacy
    @ViewBuilder let content: Content
    var body: some View {
        if #available(macOS 26, *), !legacy { GlassEffectContainer { content } } else { content }
    }
}

/// A toolbar item at the centre with no system background behind it (26 draws one; before 26 there is none).
/// Toolbar items are hosted outside the content's environment: the process-wide `LegacyLook.on` decides.
struct PrincipalToolbarItem<V: View>: ToolbarContent {
    let content: () -> V
    var body: some ToolbarContent {
        if #available(macOS 26, *), !LegacyLook.on {
            ToolbarItem(placement: .principal, content: content).sharedBackgroundVisibility(.hidden)
        } else {
            ToolbarItem(placement: .principal, content: content)
        }
    }
}

/// A bar over a scroll view's edge (pane label, composer): the text scrolls under it. 26: a system safe-area bar,
/// faded by the soft scroll edge. Before: a safe-area inset on the window surface (`BarSurface`).
struct PaneBar<B: View>: ViewModifier {
    @Environment(\.legacyLook) private var legacy
    let edge: VerticalEdge
    let bar: B
    func body(content: Content) -> some View {
        if #available(macOS 26, *), !legacy {
            content.safeAreaBar(edge: edge, spacing: 0) { bar }
        } else {
            content.safeAreaInset(edge: edge, spacing: 0) { bar.background { BarSurface(fadeEdge: edge == .top ? .bottom : .top) } }
        }
    }
}

/// Soft scroll edges on 26. Before 26 there are none: the bars draw the surface under themselves (`PaneBar`).
struct SoftScrollEdges: ViewModifier {
    @Environment(\.legacyLook) private var legacy
    let edges: Edge.Set
    func body(content: Content) -> some View {
        if #available(macOS 26, *), !legacy { content.scrollEdgeEffectStyle(.soft, for: edges) } else { content }
    }
}

/// Content under the transparent toolbar: on 26 it fades out at the soft top edge. Before: the window surface over
/// the toolbar band (the top safe area), so the text does not show through between the tab island and the buttons.
struct ToolbarFade: ViewModifier {
    @Environment(\.legacyLook) private var legacy
    @State private var band: CGFloat = 0
    func body(content: Content) -> some View {
        if #available(macOS 26, *), !legacy {
            content.scrollEdgeEffectStyle(.soft, for: .top)
        } else {
            content
                .onGeometryChange(for: CGFloat.self) { $0.safeAreaInsets.top } action: { band = $0 }
                .overlay(alignment: .top) {
                    if band > 0 { BarSurface(fadeEdge: .bottom).frame(height: band).ignoresSafeArea(.container, edges: .top) }
                }
        }
    }
}

/// The window surface under a bar (the same view as the window's, so the same pixels), running out in a short fade
/// past the bar's edge toward the content.
struct BarSurface: View {
    let fadeEdge: VerticalEdge
    static let fade: CGFloat = 12
    var body: some View {
        GlassReader { glass in
            let tone = Color(nsColor: .textBackgroundColor).opacity(glass.tint)
            VStack(spacing: 0) {
                if fadeEdge == .top { LinearGradient(colors: [tone.opacity(0), tone], startPoint: .top, endPoint: .bottom).frame(height: Self.fade) }
                WindowSurface(glass: glass)
                if fadeEdge == .bottom { LinearGradient(colors: [tone, tone.opacity(0)], startPoint: .top, endPoint: .bottom).frame(height: Self.fade) }
            }
            .padding(fadeEdge == .top ? .top : .bottom, -Self.fade)
        }
        .allowsHitTesting(false)
    }
}

extension View {
    func glassSurface<S: Shape>(_ shape: S, tint: Color? = nil, interactive: Bool = false) -> some View {
        modifier(GlassSurface(shape: shape, tint: tint, interactive: interactive))
    }
    func paneBar<B: View>(_ edge: VerticalEdge, @ViewBuilder _ bar: () -> B) -> some View { modifier(PaneBar(edge: edge, bar: bar())) }
    func softScrollEdges(_ edges: Edge.Set) -> some View { modifier(SoftScrollEdges(edges: edges)) }
    func toolbarFade() -> some View { modifier(ToolbarFade()) }
}
