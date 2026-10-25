//! The four status symbols, drawn on a canvas so they stay crisp at any size.
//!
//! working = gray track + blue arc, blocked = orange disc + `!`, done = green disc + check, idle = gray hollow ring.
//! `symbol` is the 14 pt form (sidebar rows, tabs, cards). `dot` is the 6 pt mini form for status summaries.

use crate::theme::Tokens;
use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Theme};
use std::f32::consts::{FRAC_PI_2, TAU};

pub const SYMBOL_SIZE: f32 = 14.0;
pub const DOT_SIZE: f32 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusKind {
    Working,
    Blocked,
    Done,
    Idle,
}

impl StatusKind {
    pub fn color(self, t: &Tokens) -> Color {
        match self {
            StatusKind::Working => t.working,
            StatusKind::Blocked => t.blocked,
            StatusKind::Done => t.done,
            StatusKind::Idle => t.idle,
        }
    }
}

struct Symbol {
    kind: StatusKind,
    tokens: Tokens,
    /// Arc rotation in radians (0 = starts at 12 o'clock). The app may animate it.
    phase: f32,
}

impl<M> canvas::Program<M> for Symbol {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        draw_symbol(&mut f, self.kind, &self.tokens, bounds.width.min(bounds.height), self.phase);
        vec![f.into_geometry()]
    }
}

/// Draws one symbol into a square of side `size` at the frame origin.
pub fn draw_symbol(f: &mut Frame, kind: StatusKind, t: &Tokens, size: f32, phase: f32) {
    let c = Point::new(size / 2.0, size / 2.0);
    let col = kind.color(t);
    match kind {
        StatusKind::Working => {
            // 2 pt at the 14 pt symbol, like the Swift arc's `lineWidth 2`.
            let w = (size * 0.143).max(1.0);
            let r = size / 2.0 - w / 2.0;
            let track = Color { a: 0.15, ..t.label };
            f.stroke(&Path::circle(c, r), Stroke::default().with_width(w).with_color(track));
            let start = phase - FRAC_PI_2;
            let arc = Path::new(|b| {
                b.arc(canvas::path::Arc { center: c, radius: r, start_angle: start.into(), end_angle: (start + TAU * 0.28).into() });
            });
            f.stroke(&arc, Stroke::default().with_width(w).with_color(col).with_line_cap(canvas::LineCap::Round));
        }
        StatusKind::Idle => {
            // The Swift hollow circle is drawn at `size - 1` in a `size` frame.
            let d = (size - 1.0).max(1.0);
            let w = (d * 0.09).max(1.0);
            f.stroke(&Path::circle(c, d / 2.0 - w / 2.0), Stroke::default().with_width(w).with_color(col));
        }
        StatusKind::Blocked => {
            f.fill(&Path::circle(c, size / 2.0), col);
            let w = (size * 0.13).max(1.0);
            let s = Stroke::default().with_width(w).with_color(t.on_accent).with_line_cap(canvas::LineCap::Round);
            f.stroke(&Path::line(Point::new(c.x, size * 0.26), Point::new(c.x, size * 0.58)), s);
            f.fill(&Path::circle(Point::new(c.x, size * 0.74), w * 0.62), t.on_accent);
        }
        StatusKind::Done => {
            f.fill(&Path::circle(c, size / 2.0), col);
            let w = (size * 0.13).max(1.0);
            let s = Stroke::default()
                .with_width(w)
                .with_color(t.on_accent)
                .with_line_cap(canvas::LineCap::Round)
                .with_line_join(canvas::LineJoin::Round);
            let p = Path::new(|b| {
                b.move_to(Point::new(size * 0.28, size * 0.52));
                b.line_to(Point::new(size * 0.43, size * 0.67));
                b.line_to(Point::new(size * 0.73, size * 0.34));
            });
            f.stroke(&p, s);
        }
    }
}

/// The 14 pt status symbol.
pub fn symbol<'a, M: 'a>(kind: StatusKind, t: &Tokens) -> Element<'a, M> {
    symbol_sized(kind, t, SYMBOL_SIZE, 0.0)
}

/// A status symbol of any size, with the arc rotated by `phase` radians.
pub fn symbol_sized<'a, M: 'a>(kind: StatusKind, t: &Tokens, size: f32, phase: f32) -> Element<'a, M> {
    Canvas::new(Symbol { kind, tokens: *t, phase }).width(Length::Fixed(size)).height(Length::Fixed(size)).into()
}

struct Dot {
    kind: StatusKind,
    tokens: Tokens,
}

impl<M> canvas::Program<M> for Dot {
    type State = ();
    fn draw(&self, _: &(), renderer: &Renderer, _: &Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut f = Frame::new(renderer, bounds.size());
        let c = Point::new(bounds.width / 2.0, bounds.height / 2.0);
        let col = self.kind.color(&self.tokens);
        match self.kind {
            StatusKind::Done | StatusKind::Blocked => f.fill(&Path::circle(c, bounds.width / 2.0), col),
            StatusKind::Idle => f.stroke(&Path::circle(c, bounds.width / 2.0 - 0.6), Stroke::default().with_width(1.2).with_color(col)),
            StatusKind::Working => {
                f.stroke(&Path::circle(c, bounds.width / 2.0 - 0.6), Stroke::default().with_width(1.2).with_color(self.tokens.fill_3));
                let arc = Path::new(|b| {
                    b.arc(canvas::path::Arc { center: c, radius: bounds.width / 2.0 - 0.6, start_angle: (-FRAC_PI_2).into(), end_angle: (-FRAC_PI_2 + TAU * 0.28).into() });
                });
                f.stroke(&arc, Stroke::default().with_width(1.2).with_color(col));
            }
        }
        vec![f.into_geometry()]
    }
}

/// The 6 pt mini dot: green filled = done, gray ring = idle, orange filled = blocked, blue arc = working.
pub fn dot<'a, M: 'a>(kind: StatusKind, t: &Tokens) -> Element<'a, M> {
    Canvas::new(Dot { kind, tokens: *t }).width(Length::Fixed(DOT_SIZE)).height(Length::Fixed(DOT_SIZE)).into()
}
