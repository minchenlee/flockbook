//! Small container styles shared by the modules. Add to it freely; keep each style a pure function of `Tokens`.

use fb_ui::theme::*;
use iced::widget::{column, container, stack, Space};
use iced::{Background, Border, Color, Element, Length, Radians, Theme};

pub fn solid(c: Color) -> container::Style {
    container::Style { background: Some(Background::Color(c)), ..Default::default() }
}

/// The opaque content color of the main area.
pub fn content(t: Tokens) -> impl Fn(&Theme) -> container::Style {
    move |_| solid(t.bg)
}

/// The sidebar: the content/window surface, `sidebar` (equal to `bg`), or the `glass_side` tint (equal to the `content`
/// tint under window glass) so sidebar and content are one tone. No hairline: Swift's sidebar column draws the
/// window surface again instead of a lighter panel with a divider.
pub fn sidebar(t: Tokens, glass: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(if glass { t.glass_side } else { t.sidebar })),
        border: Border { color: t.sep, width: 0.0, radius: 0.0.into() },
        ..Default::default()
    }
}

/// A filled rounded box (chips, selected rows, user bubble).
pub fn fill(color: Color, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style { background: Some(Background::Color(color)), border: Border { radius: radius.into(), ..Default::default() }, ..Default::default() }
}

/// e1 card: content color, hairline, soft shadow.
pub fn card_e1(t: Tokens, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(t.bg)),
        border: Border { color: t.sep, width: 1.0, radius: radius.into() },
        shadow: shadow_e1(&t),
        ..Default::default()
    }
}

/// e2 floating surface (composer, approval card, menus).
pub fn float_e2(t: Tokens, radius: f32) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(if t.mode.is_dark() { rgba(48, 48, 52, 1.0) } else { rgba(255, 255, 255, 1.0) })),
        border: Border { color: t.sep, width: 1.0, radius: radius.into() },
        shadow: shadow_e2(&t),
        ..Default::default()
    }
}

/// Height of the soft fade under the top row.
pub const FADE_H: f32 = 24.0;

/// A vertical gradient from the content color `c` to the same color with no alpha (top to bottom).
pub fn fade_down(c: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let g = iced::gradient::Linear::new(Radians(std::f32::consts::PI)).add_stop(0.0, c).add_stop(1.0, Color { a: 0.0, ..c });
        container::Style { background: Some(Background::Gradient(iced::Gradient::Linear(g))), ..Default::default() }
    }
}

/// Puts a `FADE_H` soft fade over the top of `content`, so rows that scroll under the top row fade out instead of
/// being cut. The content keeps its own top inset (24 pt in the conversation, 36 pt in the overview), so the first
/// row at rest sits below the fade. The fade ignores the pointer.
/// `ground` is the color under the content (`bg`, or `content` under window glass).
pub fn with_top_fade<'a, M: 'a>(ground: Color, content: Element<'a, M>) -> Element<'a, M> {
    let fade = container(Space::new()).width(Length::Fill).height(Length::Fixed(FADE_H)).style(fade_down(ground));
    stack![content, column![fade, Space::new().height(Length::Fill)]].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_runs_from_the_content_color_to_transparent() {
        let t = Tokens::LIGHT;
        let st = fade_down(t.bg)(&Theme::Light);
        let Some(Background::Gradient(iced::Gradient::Linear(g))) = st.background else { panic!("gradient") };
        let stops: Vec<_> = g.stops.iter().flatten().collect();
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].color, t.bg);
        assert_eq!(stops[1].color.a, 0.0);
        assert_eq!((stops[1].color.r, stops[1].color.g, stops[1].color.b), (t.bg.r, t.bg.g, t.bg.b), "no gray fringe");
        assert_eq!(FADE_H, 24.0);
    }
}
