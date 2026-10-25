//! Design tokens from DESIGN.md: neutrals, accent, status colors (light + dark), glass label variants, the type
//! scale, spacing, radii and the two elevation levels.
//!
//! `Tokens` is `Copy`. Build one with [`Tokens::of`] and pass it down to views. Colors that are translucent in the
//! spec (`rgba(0,0,0,.85)`) are translucent here, so they compose over whatever is behind them.

use iced::font::{Family, Weight};
use iced::theme::Palette;
use iced::{Color, Font, Shadow, Theme, Vector};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Light,
    Dark,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "light" => Some(Mode::Light),
            "dark" => Some(Mode::Dark),
            _ => None,
        }
    }
    pub fn is_dark(self) -> bool {
        self == Mode::Dark
    }
}

/// `#RRGGBB` as a color.
pub const fn hex(v: u32) -> Color {
    Color { r: ((v >> 16) & 0xFF) as f32 / 255.0, g: ((v >> 8) & 0xFF) as f32 / 255.0, b: (v & 0xFF) as f32 / 255.0, a: 1.0 }
}

/// `rgba(r,g,b,a)` with 0..255 channels and a 0..1 alpha.
pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
}

const fn black(a: f32) -> Color {
    rgba(0, 0, 0, a)
}
const fn white(a: f32) -> Color {
    rgba(255, 255, 255, a)
}

/// Every color token of one mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub mode: Mode,
    // Neutrals.
    pub bg: Color,
    /// The sidebar column's surface: the same as `bg`, because sidebar and content are one tone.
    pub sidebar: Color,
    pub sidebar_sel: Color,
    pub fill_1: Color,
    pub fill_2: Color,
    pub fill_3: Color,
    pub sep: Color,
    pub sep_strong: Color,
    pub label: Color,
    pub label_2: Color,
    pub label_3: Color,
    // Accent.
    pub accent: Color,
    pub accent_soft: Color,
    // Status (state only).
    pub working: Color,
    pub blocked: Color,
    pub done: Color,
    pub idle: Color,
    pub diff_add: Color,
    pub diff_del: Color,
    /// Text on an accent or status fill.
    pub on_accent: Color,
    // Glass variants (section 9). Use on translucent surfaces only.
    /// The sidebar tint under glass: lighter than `content`, so the desktop shows through the sidebar (section 9).
    pub glass_side: Color,
    pub glass_bar: Color,
    pub glass_float: Color,
    pub content: Color,
    pub label_2_glass: Color,
    pub label_3_glass: Color,
    /// Soft orange pill behind the blocked edge marker and the blocked card border.
    pub blocked_soft: Color,
    // Syntax colors of code blocks.
    pub syn_keyword: Color,
    pub syn_string: Color,
    pub syn_number: Color,
    pub syn_comment: Color,
    pub syn_function: Color,
}

impl Tokens {
    pub const LIGHT: Tokens = Tokens {
        mode: Mode::Light,
        bg: hex(0xFFFFFF),
        // Sidebar and content are one tone: the sidebar column draws the window surface again (WindowSurface), so it
        // shows the content colour, not a lighter panel grey.
        sidebar: hex(0xFFFFFF),
        sidebar_sel: black(0.07),
        fill_1: black(0.035),
        fill_2: black(0.06),
        fill_3: black(0.10),
        sep: black(0.09),
        sep_strong: black(0.14),
        label: black(0.85),
        label_2: black(0.50),
        label_3: black(0.28),
        accent: hex(0x007AFF),
        accent_soft: rgba(0x00, 0x7A, 0xFF, 0.10),
        working: hex(0x007AFF),
        blocked: hex(0xFF9500),
        done: hex(0x28CD41),
        idle: black(0.28),
        diff_add: hex(0x28CD41),
        diff_del: hex(0xFF3B30),
        on_accent: hex(0xFFFFFF),
        glass_side: rgba(240, 240, 245, 0.55),
        glass_bar: rgba(248, 248, 251, 0.62),
        glass_float: rgba(255, 255, 255, 0.66),
        content: rgba(255, 255, 255, 0.92),
        label_2_glass: black(0.62),
        label_3_glass: black(0.45),
        blocked_soft: rgba(0xFF, 0x95, 0x00, 0.12),
        syn_keyword: hex(0x9B2393),
        syn_string: hex(0xC41A16),
        syn_number: hex(0x1C00CF),
        syn_comment: black(0.28),
        syn_function: hex(0x326D74),
    };

    pub const DARK: Tokens = Tokens {
        mode: Mode::Dark,
        bg: hex(0x1E1E1F),
        sidebar: hex(0x1E1E1F),
        sidebar_sel: white(0.09),
        fill_1: white(0.04),
        fill_2: white(0.07),
        fill_3: white(0.12),
        sep: white(0.09),
        sep_strong: white(0.15),
        label: white(0.88),
        label_2: white(0.55),
        label_3: white(0.28),
        accent: hex(0x0A84FF),
        accent_soft: rgba(0x0A, 0x84, 0xFF, 0.18),
        working: hex(0x0A84FF),
        blocked: hex(0xFF9F0A),
        done: hex(0x32D74B),
        idle: white(0.28),
        diff_add: hex(0x32D74B),
        diff_del: hex(0xFF453A),
        on_accent: hex(0xFFFFFF),
        glass_side: rgba(30, 30, 31, 0.45),
        glass_bar: rgba(38, 38, 42, 0.60),
        glass_float: rgba(48, 48, 52, 0.62),
        content: rgba(30, 30, 31, 0.90),
        label_2_glass: white(0.60),
        label_3_glass: white(0.42),
        blocked_soft: rgba(0xFF, 0x9F, 0x0A, 0.16),
        syn_keyword: hex(0xFC5FA3),
        syn_string: hex(0xFC6A5D),
        syn_number: hex(0x9686F5),
        syn_comment: white(0.28),
        syn_function: hex(0x67B7A4),
    };

    /// The tokens for text on a glass surface (sidebar, bars): the stronger `label_2` and `label_3` (section 9).
    pub fn on_glass(self) -> Tokens {
        Tokens { label_2: self.label_2_glass, label_3: self.label_3_glass, ..self }
    }

    pub fn of(mode: Mode) -> Tokens {
        match mode {
            Mode::Light => Tokens::LIGHT,
            Mode::Dark => Tokens::DARK,
        }
    }

    /// An iced theme whose palette matches the tokens, so stock widgets (scrollable, text) blend in.
    pub fn iced_theme(&self) -> Theme {
        let name = if self.mode.is_dark() { "Flockbook Dark" } else { "Flockbook Light" };
        Theme::custom(
            name.to_string(),
            Palette {
                background: self.bg,
                // Opaque stand-in for the translucent label token: iced mixes this with the background.
                text: if self.mode.is_dark() { hex(0xE0E0E0) } else { hex(0x262626) },
                primary: self.accent,
                success: self.done,
                warning: self.blocked,
                danger: self.diff_del,
            },
        )
    }
}

// Spacing (4-pt grid).
pub const SP_4: f32 = 4.0;
pub const SP_8: f32 = 8.0;
pub const SP_12: f32 = 12.0;
pub const SP_16: f32 = 16.0;
pub const SP_24: f32 = 24.0;
pub const SP_32: f32 = 32.0;

// Radii.
pub const R_CHIP: f32 = 5.0;
pub const R_ROW: f32 = 6.0;
pub const R_MENU: f32 = 10.0;
pub const R_CARD: f32 = 12.0;
pub const R_COMPOSER: f32 = 16.0;

// Fixed sizes from the spec.
pub const SIDEBAR_ROW_H: f32 = 28.0;
pub const WORKSPACE_ROW_H: f32 = 34.0;
pub const TOPROW_H: f32 = 52.0;
pub const TAB_H: f32 = 30.0;
pub const TAB_RADIUS: f32 = 8.0;
pub const PANE_LABEL_H: f32 = 28.0;
pub const SIDEBAR_W: f32 = 248.0;

/// One entry of the type scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    /// Line height as a multiple of the size.
    pub line: f32,
    pub weight: Weight,
    pub mono: bool,
}

impl TextStyle {
    pub fn font(&self) -> Font {
        if self.mono {
            crate::fonts::mono()
        } else {
            crate::fonts::inter(self.weight)
        }
    }
    pub fn line_height(&self) -> iced::widget::text::LineHeight {
        iced::widget::text::LineHeight::Relative(self.line)
    }
    /// A `text` widget in this style (color left to the caller).
    pub fn text<'a>(&self, s: impl iced::widget::text::IntoFragment<'a>) -> iced::widget::Text<'a> {
        iced::widget::text(s).size(self.size).font(self.font()).line_height(self.line_height())
    }
}

pub const LARGE_TITLE: TextStyle = TextStyle { size: 26.0, line: 1.2, weight: Weight::Bold, mono: false };
pub const TITLE: TextStyle = TextStyle { size: 22.0, line: 1.2, weight: Weight::Bold, mono: false };
pub const TITLE_3: TextStyle = TextStyle { size: 17.0, line: 1.3, weight: Weight::Semibold, mono: false };
pub const READING: TextStyle = TextStyle { size: 15.0, line: 1.55, weight: Weight::Normal, mono: false };
pub const HEADLINE: TextStyle = TextStyle { size: 13.0, line: 1.3, weight: Weight::Semibold, mono: false };
pub const BODY: TextStyle = TextStyle { size: 13.0, line: 1.3, weight: Weight::Normal, mono: false };
pub const CALLOUT: TextStyle = TextStyle { size: 12.0, line: 1.3, weight: Weight::Medium, mono: false };
pub const CAPTION: TextStyle = TextStyle { size: 11.0, line: 1.3, weight: Weight::Medium, mono: false };
pub const MONO: TextStyle = TextStyle { size: 12.0, line: 1.5, weight: Weight::Normal, mono: true };

/// e1: hairline plus a 1 px soft shadow. Cards that sit in the content. Draw the hairline with a border of `sep`.
pub fn shadow_e1(t: &Tokens) -> Shadow {
    let a = if t.mode.is_dark() { 0.30 } else { 0.06 };
    Shadow { color: black(a), offset: Vector::new(0.0, 1.0), blur_radius: 1.0 }
}

/// e2: hairline plus an 8-10 px blur shadow. Things that float or need attention.
pub fn shadow_e2(t: &Tokens) -> Shadow {
    let a = if t.mode.is_dark() { 0.45 } else { 0.12 };
    Shadow { color: black(a), offset: Vector::new(0.0, 4.0), blur_radius: 10.0 }
}

/// `fg` composited over an opaque `bg`. The tiny-skia svg renderer drops the alpha of an icon color, so icons that
/// should look translucent get their color pre-blended with what is behind them.
pub fn over(fg: Color, bg: Color) -> Color {
    Color { r: fg.r * fg.a + bg.r * (1.0 - fg.a), g: fg.g * fg.a + bg.g * (1.0 - fg.a), b: fg.b * fg.a + bg.b * (1.0 - fg.a), a: 1.0 }
}

impl Tokens {
    /// Color of a code run.
    pub fn syntax(&self, tone: crate::highlight::Tone) -> Color {
        use crate::highlight::Tone;
        match tone {
            Tone::Plain => self.label,
            Tone::Keyword => self.syn_keyword,
            Tone::String => self.syn_string,
            Tone::Number => self.syn_number,
            Tone::Comment => self.syn_comment,
            Tone::Function => self.syn_function,
        }
    }
}

/// A weight-only font helper kept here so call sites do not need `iced::font` imports.
pub fn weighted(family: &'static str, weight: Weight) -> Font {
    Font { family: Family::Name(family), weight, ..Font::DEFAULT }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_rgba_convert() {
        let c = hex(0x007AFF);
        assert_eq!(c.into_rgba8(), [0, 0x7A, 0xFF, 255]);
        assert!((black(0.85).a - 0.85).abs() < 1e-6);
    }

    #[test]
    fn dark_tokens_differ_and_status_keeps_shape_colors() {
        assert_ne!(Tokens::LIGHT.bg, Tokens::DARK.bg);
        assert_eq!(Tokens::LIGHT.blocked.into_rgba8()[..3], [0xFF, 0x95, 0x00]);
        assert_eq!(Tokens::DARK.done.into_rgba8()[..3], [0x32, 0xD7, 0x4B]);
        // Glass label tokens are stronger than the plain ones.
        assert!(Tokens::LIGHT.label_2_glass.a > Tokens::LIGHT.label_2.a);
        assert!(Tokens::DARK.label_3_glass.a > Tokens::DARK.label_3.a);
        let g = Tokens::LIGHT.on_glass();
        assert_eq!((g.label_2, g.label_3), (Tokens::LIGHT.label_2_glass, Tokens::LIGHT.label_3_glass));
        assert_eq!((g.label, g.bg, g.sidebar), (Tokens::LIGHT.label, Tokens::LIGHT.bg, Tokens::LIGHT.sidebar), "only the labels change");
    }

    #[test]
    fn syntax_tokens_exist_in_both_modes_and_differ() {
        use crate::highlight::Tone;
        for tone in [Tone::Keyword, Tone::String, Tone::Number, Tone::Comment, Tone::Function] {
            assert_ne!(Tokens::LIGHT.syntax(tone), Tokens::DARK.syntax(tone));
            assert_ne!(Tokens::LIGHT.syntax(tone), Tokens::LIGHT.label);
        }
        assert_eq!(Tokens::LIGHT.syntax(Tone::Plain), Tokens::LIGHT.label);
        assert_eq!(Tokens::LIGHT.syn_comment, Tokens::LIGHT.label_3);
    }

    #[test]
    fn over_blends_alpha() {
        let c = over(black(0.5), hex(0xFFFFFF));
        assert!((c.r - 0.5).abs() < 1e-6 && c.a == 1.0);
    }

    #[test]
    fn scale_matches_the_spec() {
        assert_eq!((READING.size, READING.line), (15.0, 1.55));
        assert_eq!((BODY.size, HEADLINE.size, CALLOUT.size, CAPTION.size, MONO.size), (13.0, 13.0, 12.0, 11.0, 12.0));
        assert_eq!([SP_4, SP_8, SP_12, SP_16, SP_24, SP_32], [4.0, 8.0, 12.0, 16.0, 24.0, 32.0]);
        assert_eq!([R_CHIP, R_ROW, R_MENU, R_CARD, R_COMPOSER], [5.0, 6.0, 10.0, 12.0, 16.0]);
    }
}
