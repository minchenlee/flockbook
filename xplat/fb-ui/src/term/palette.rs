//! Cell colors to iced colors, light and dark.
//!
//! The default foreground, the cursor and the selection come from the theme tokens. The default background is
//! transparent: a terminal sits on the same surface as the window (Swift `applyColors`). The 16 ANSI colors are
//! tuned for each surface (the same hue family as the syntax tokens, readable on white and on the dark content color).

use super::grid::TermColor;
use crate::theme::{hex, Mode, Tokens};
use iced::Color;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TermPalette {
    pub mode: Mode,
    pub fg: Color,
    /// The window surface. Drawn only for an inverse cell with the default colors.
    pub bg: Color,
    pub cursor: Color,
    pub on_cursor: Color,
    /// Selected cells: the accent, translucent, drawn under the text.
    pub selection: Color,
    /// Ghostty can choose selected text separately; None preserves each cell color.
    pub selection_fg: Option<Color>,
    pub ansi: [Color; 16],
}

/// Swift `applyColors`: Flockbook's own selection is this teal with black text (never the accent).
const TEAL_SELECTION: Color = Color { r: 0.0, g: 166.0 / 255.0, b: 178.0 / 255.0, a: 1.0 };

const LIGHT_ANSI: [u32; 16] = [
    0x1D1D1F, 0xC41A16, 0x1E7F34, 0x8A6A00, 0x0A5FD0, 0x9B2393, 0x1B7A86, 0x8E8E93, // normal
    0x4A4A4F, 0xE0352B, 0x2A9D43, 0xA67C00, 0x007AFF, 0xB5379F, 0x2799A6, 0x3C3C43, // bright
];

const DARK_ANSI: [u32; 16] = [
    0x3A3A3C, 0xFF453A, 0x32D74B, 0xE5C07B, 0x0A84FF, 0xFC5FA3, 0x67B7A4, 0xD1D1D6, // normal
    0x636366, 0xFF6961, 0x5DE06E, 0xFFD60A, 0x5AAEFF, 0xFF85B8, 0x8FD6C4, 0xFFFFFF, // bright
];

impl TermPalette {
    pub fn of(t: &Tokens) -> TermPalette {
        let table = if t.mode.is_dark() { &DARK_ANSI } else { &LIGHT_ANSI };
        TermPalette { mode: t.mode, fg: over(t.label, t.bg), bg: t.bg, cursor: t.accent, on_cursor: t.on_accent, selection: TEAL_SELECTION, selection_fg: Some(Color::BLACK), ansi: table.map(hex) }
    }

    /// The color of an indexed or true color. `Default` gives `None`: the caller picks the default fg or bg.
    pub fn resolve(&self, c: TermColor) -> Option<Color> {
        match c {
            TermColor::Default => None,
            TermColor::Rgb(r, g, b) => Some(Color::from_rgb8(r, g, b)),
            TermColor::Indexed(i) => Some(self.indexed(i)),
        }
    }

    pub fn indexed(&self, i: u8) -> Color {
        match i {
            0..=15 => self.ansi[i as usize],
            16..=231 => {
                let n = i - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
                Color::from_rgb8(level(n / 36), level((n / 6) % 6), level(n % 6))
            }
            232..=255 => {
                let v = 8 + 10 * (i - 232);
                Color::from_rgb8(v, v, v)
            }
        }
    }
}

/// Blends `c` over `surface`. The text token is a translucent black or white; a terminal needs it solid.
fn over(c: Color, surface: Color) -> Color {
    let a = c.a;
    Color { r: c.r * a + surface.r * (1.0 - a), g: c.g * a + surface.g * (1.0 - a), b: c.b * a + surface.b * (1.0 - a), a: 1.0 }
}

/// The colors one cell is drawn with: text color and an optional background (`None` = transparent).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellColors {
    pub fg: Color,
    pub bg: Option<Color>,
}

pub fn cell_colors(p: &TermPalette, cell: &super::grid::CellView, on_block_cursor: bool) -> CellColors {
    if on_block_cursor {
        return CellColors { fg: p.on_cursor, bg: Some(p.cursor) };
    }
    let mut fg = p.resolve(cell.fg);
    let mut bg = p.resolve(cell.bg);
    if cell.inverse {
        // An inverse cell swaps; a default side becomes the surface or the text color.
        let (f, b) = (bg.unwrap_or(p.bg), fg.unwrap_or(p.fg));
        fg = Some(f);
        bg = Some(b);
    }
    let mut fg = fg.unwrap_or(p.fg);
    if cell.dim {
        fg.a *= 0.6;
    }
    CellColors { fg, bg }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::grid::CellView;

    #[test]
    fn the_default_fg_is_solid_and_the_bg_is_transparent() {
        for t in [Tokens::LIGHT, Tokens::DARK] {
            let p = TermPalette::of(&t);
            assert_eq!(p.fg.a, 1.0);
            assert_eq!(p.resolve(TermColor::Default), None);
            let c = cell_colors(&p, &CellView::default(), false);
            assert_eq!(c.bg, None, "no fill for the default background");
        }
    }

    #[test]
    fn the_16_ansi_colors_differ_per_mode() {
        let (l, d) = (TermPalette::of(&Tokens::LIGHT), TermPalette::of(&Tokens::DARK));
        assert_ne!(l.indexed(1), d.indexed(1));
        assert_eq!(l.indexed(9), hex(0xE0352B));
        assert_eq!(d.indexed(15), hex(0xFFFFFF));
    }

    #[test]
    fn the_256_cube_and_gray_ramp() {
        let p = TermPalette::of(&Tokens::LIGHT);
        assert_eq!(p.indexed(16), Color::from_rgb8(0, 0, 0));
        assert_eq!(p.indexed(231), Color::from_rgb8(255, 255, 255));
        assert_eq!(p.indexed(196), Color::from_rgb8(255, 0, 0));
        assert_eq!(p.indexed(232), Color::from_rgb8(8, 8, 8));
        assert_eq!(p.indexed(255), Color::from_rgb8(238, 238, 238));
        assert_eq!(p.resolve(TermColor::Rgb(1, 2, 3)), Some(Color::from_rgb8(1, 2, 3)));
    }

    #[test]
    fn inverse_swaps_and_fills() {
        let p = TermPalette::of(&Tokens::LIGHT);
        let c = cell_colors(&p, &CellView { inverse: true, ..Default::default() }, false);
        assert_eq!(c.fg, p.bg);
        assert_eq!(c.bg, Some(p.fg));
        let red = cell_colors(&p, &CellView { inverse: true, fg: TermColor::Indexed(1), ..Default::default() }, false);
        assert_eq!(red.bg, Some(p.indexed(1)));
    }

    #[test]
    fn the_block_cursor_uses_the_accent() {
        let p = TermPalette::of(&Tokens::DARK);
        let c = cell_colors(&p, &CellView::default(), true);
        assert_eq!((c.fg, c.bg), (p.on_cursor, Some(p.cursor)));
    }

    #[test]
    fn dim_lowers_the_alpha() {
        let p = TermPalette::of(&Tokens::LIGHT);
        let c = cell_colors(&p, &CellView { dim: true, ..Default::default() }, false);
        assert!(c.fg.a < 1.0);
    }
}
