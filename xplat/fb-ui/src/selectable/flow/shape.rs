//! Turns a `Row` into shaped paragraphs and drawing primitives at one column width. Pure data: the widget draws
//! it, hit-tests it and measures it; tests probe it.

use super::model::{Chip, Metrics, Row, RowKind};
use crate::fonts;
use crate::icons::Icon;
use crate::theme::Tokens;
use iced::advanced::graphics::text::Paragraph;
use iced::advanced::text::paragraph::Paragraph as _;
use iced::advanced::text::{self, Shaping, Span, Wrapping};
use iced::font::Weight;
use iced::{alignment, border, Background, Border, Color, Pixels, Point, Rectangle, Size};
use std::sync::Arc;

pub type Spans = Arc<[Span<'static, String>]>;

/// A drawing primitive behind the text.
pub enum Prim {
    Quad {
        rect: Rectangle,
        fill: Color,
        border: Color,
        bw: f32,
        radius: border::Radius,
    },
    Text {
        para: Paragraph,
        pos: Point,
        color: Color,
    },
    Icon {
        icon: Icon,
        rect: Rectangle,
        color: Color,
    },
}

/// A selectable piece of text. Normal rows have one; a table has one per cell.
pub struct Part {
    pub para: Paragraph,
    pub spans: Spans,
    /// Top-left, relative to the row origin (column left, content top).
    pub pos: Point,
    /// Width available to the text (for hit testing).
    pub w: f32,
    /// Where this part starts in `Row::plain`.
    pub plain_start: usize,
    pub plain_len: usize,
    /// Byte offset of each logical line start inside this part's text.
    pub line_starts: Vec<usize>,
    pub color: Color,
}

pub struct ChipGeo {
    pub id: String,
    pub rect: Rectangle,
    pub expandable: bool,
    pub label: Paragraph,
    pub label_pos: Point,
    pub label_color: Color,
    pub icon: Icon,
    pub icon_rect: Rectangle,
    pub chevron: Option<(Icon, Rectangle)>,
}

pub struct Shaped {
    pub sig: u64,
    pub colw: f32,
    /// Content height (without the slot lead).
    pub h: f32,
    pub prims: Vec<Prim>,
    pub parts: Vec<Part>,
    pub chips: Vec<ChipGeo>,
    /// Clickable rect of a "Show earlier" row.
    pub load_earlier: Option<Rectangle>,
}

pub fn signature(row: &Row, colw: f32, t: &Tokens, m: &Metrics) -> u64 {
    row.sig
        ^ (colw.to_bits() as u64).rotate_left(17)
        ^ m.hash().rotate_left(29)
        ^ if t.mode.is_dark() {
            0x9E37_79B9_7F4A_7C15
        } else {
            0
        }
}

pub fn line_starts(plain: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(plain.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// Styled spans of one paragraph. CJK runs get their own span so they never ask for italic.
pub fn spans_of(
    para: &super::model::Para,
    size: f32,
    t: &Tokens,
    color: Color,
    weight: Weight,
    bold_all: bool,
    mono_all: bool,
) -> Vec<Span<'static, String>> {
    let mut out = Vec::new();
    for inl in &para.inlines {
        if inl.text.is_empty() {
            continue;
        }
        fonts::for_text_runs(&inl.text, |run, cjk| {
            let mut font =
                fonts::span_font(inl.code || mono_all, inl.italic, inl.bold || bold_all, cjk);
            if !inl.code && !mono_all && !inl.bold && !bold_all {
                font.weight = weight;
            }
            let run_color = if mono_all && inl.tone != crate::highlight::Tone::Plain {
                t.syntax(inl.tone)
            } else {
                color
            };
            let mut sp = Span::new(run.to_string()).font(font).color(run_color);
            if inl.code && !mono_all {
                sp = sp
                    .color(t.label)
                    .size(Pixels(size * 0.875))
                    .background(Background::Color(t.fill_2))
                    .border(Border {
                        radius: 4.0.into(),
                        ..Default::default()
                    })
                    .padding(iced::Padding {
                        top: 1.0,
                        bottom: 1.0,
                        left: 2.0,
                        right: 2.0,
                    });
            }
            if let Some(l) = &inl.link {
                sp = sp.color(t.accent).link(l.clone());
            }
            if inl.strike {
                sp = sp.strikethrough(true);
            }
            out.push(sp);
        });
    }
    out
}

fn make(
    spans: &[Span<'static, String>],
    width: f32,
    size: f32,
    line: f32,
    wrap: Wrapping,
) -> Paragraph {
    Paragraph::with_spans(text::Text {
        content: spans,
        bounds: Size::new(width, f32::INFINITY),
        size: Pixels(size),
        line_height: text::LineHeight::Relative(line),
        font: fonts::default_font(),
        align_x: text::Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: wrap,
    })
}

fn label(
    text: &str,
    size: f32,
    weight: Weight,
    mono: bool,
    color: Color,
    t: &Tokens,
) -> (Paragraph, Size) {
    let mut font = if mono {
        fonts::mono()
    } else {
        fonts::inter(weight)
    };
    if !mono {
        font.weight = weight;
    }
    let sp = [Span::new(text.to_string()).font(font).color(color)];
    let _ = t;
    let p = make(&sp, f32::INFINITY, size, 1.3, Wrapping::None);
    let s = p.min_bounds();
    (p, s)
}

#[allow(clippy::too_many_arguments)]
fn part(
    para: &super::model::Para,
    size: f32,
    line: f32,
    width: f32,
    wrap: Wrapping,
    pos: Point,
    plain_start: usize,
    color: Color,
    weight: Weight,
    bold_all: bool,
    mono: bool,
    t: &Tokens,
) -> (Part, f32) {
    let spans = spans_of(para, size, t, color, weight, bold_all, mono);
    let p = make(&spans, width, size, line, wrap);
    let h = p.min_bounds().height;
    let plain = para.plain();
    (
        Part {
            para: p,
            spans: spans.into(),
            pos,
            w: width,
            plain_start,
            plain_len: plain.len(),
            line_starts: line_starts(&plain),
            color,
        },
        h,
    )
}

fn quad(rect: Rectangle, fill: Color, bc: Color, bw: f32, radius: f32) -> Prim {
    Prim::Quad {
        rect,
        fill,
        border: bc,
        bw,
        radius: radius.into(),
    }
}

pub fn shape(row: &Row, colw: f32, t: &Tokens, m: &Metrics) -> Shaped {
    let mut s = Shaped {
        sig: signature(row, colw, t, m),
        colw,
        h: 0.0,
        prims: Vec::new(),
        parts: Vec::new(),
        chips: Vec::new(),
        load_earlier: None,
    };
    let empty = super::model::Para::default();
    let first = row.paras.first().unwrap_or(&empty);
    match &row.kind {
        RowKind::Picture { width, height, .. } => {
            let k = 1f32
                .min(colw.min(480.) / (*width).max(1) as f32)
                .min(640. / (*height).max(1) as f32);
            let size = Size::new(
                (*width as f32 * k).round().max(1.),
                (*height as f32 * k).round().max(1.),
            );
            s.h = size.height;
            s.prims.push(quad(
                Rectangle::new(Point::ORIGIN, size),
                t.fill_2,
                t.sep,
                1.,
                8.,
            ));
        }
        RowKind::Prose => {
            let (p, h) = part(
                first,
                m.body,
                m.rel(),
                colw,
                Wrapping::WordOrGlyph,
                Point::ORIGIN,
                0,
                t.label,
                Weight::Normal,
                false,
                false,
                t,
            );
            s.parts.push(p);
            s.h = h;
        }
        RowKind::Heading(l) => {
            // Swift MarkdownView: h1 bold, the rest semibold; level 4 is secondary; line 1.36 em.
            let size = m.heading[(*l as usize).clamp(1, 4) - 1];
            let (p, h) = part(
                first,
                size,
                1.36,
                colw,
                Wrapping::WordOrGlyph,
                Point::ORIGIN,
                0,
                if *l >= 4 { t.label_2 } else { t.label },
                if *l == 1 { Weight::Bold } else { Weight::Semibold },
                false,
                false,
                t,
            );
            s.parts.push(p);
            s.h = h;
        }
        RowKind::ListItem { marker, depth } => {
            // Swift MarkdownView `item`: a marker column of max(24, 1.6 em) per level, the marker centred in it.
            let mw = (m.body * 1.6).max(24.0);
            let base = mw * (*depth as f32).clamp(1.0, 4.0) - mw;
            let indent = base + mw;
            let (p, h) = part(
                first,
                m.body,
                m.rel(),
                colw - indent,
                Wrapping::WordOrGlyph,
                Point::new(indent, 0.0),
                0,
                t.label,
                Weight::Normal,
                false,
                false,
                t,
            );
            s.parts.push(p);
            s.h = h;
            let (mp, ms) = label(marker, m.body, Weight::Normal, false, t.label_2, t);
            // The marker's line box is shorter than the text's line, so centre it in the first line.
            let dy = (m.line - ms.height) / 2.0;
            let ordered = marker.ends_with('.');
            let mx = if ordered {
                base + mw - 6.0 - ms.width
            } else {
                base + (mw - ms.width) / 2.0
            };
            s.prims.push(Prim::Text {
                para: mp,
                pos: Point::new(mx, dy),
                color: t.label_2,
            });
        }
        RowKind::Quote => {
            // Swift: a 3 pt bar and 14 pt of space.
            let (p, h) = part(
                first,
                m.body,
                m.rel(),
                colw - 17.0,
                Wrapping::WordOrGlyph,
                Point::new(17.0, 0.0),
                0,
                t.label_2,
                Weight::Normal,
                false,
                false,
                t,
            );
            s.parts.push(p);
            s.h = h;
            s.prims.push(quad(
                Rectangle::new(Point::new(2.0, 1.0), Size::new(3.0, (h - 2.0).max(0.0))),
                t.fill_3,
                Color::TRANSPARENT,
                0.0,
                1.5,
            ));
        }
        RowKind::Code { lang } => {
            let pad_x = 14.0;
            let mut y = 12.0;
            if let Some(l) = lang.as_ref().filter(|l| !l.is_empty()) {
                let (lp, ls) = label(l, 11.0, Weight::Medium, false, t.label_2, t);
                s.prims.push(Prim::Text {
                    para: lp,
                    pos: Point::new(pad_x, y),
                    color: t.label_2,
                });
                y += ls.height + 6.0;
            }
            let (p, h) = part(
                first,
                m.code,
                m.code_line / m.code,
                colw - 2.0 * pad_x,
                Wrapping::WordOrGlyph,
                Point::new(pad_x, y),
                0,
                t.label,
                Weight::Normal,
                false,
                true,
                t,
            );
            s.parts.push(p);
            s.h = y + h + 12.0;
            s.prims.insert(
                0,
                quad(
                    Rectangle::new(Point::ORIGIN, Size::new(colw, s.h)),
                    t.fill_1,
                    t.sep,
                    1.0,
                    12.0,
                ),
            );
        }
        RowKind::Table { cols } => shape_table(&mut s, row, *cols, colw, t, m),
        RowKind::Bubble => {
            let maxw = (colw * 0.8 - 28.0).max(80.0);
            let (mut p, h) = part(
                first,
                m.body,
                (m.line - 2.0) / m.body,
                maxw,
                Wrapping::WordOrGlyph,
                Point::ORIGIN,
                0,
                t.label,
                Weight::Normal,
                false,
                false,
                t,
            );
            let tw = p.para.min_bounds().width.ceil();
            let bw = (tw + 28.0).min(colw);
            // Swift UserBubble: padding 14 across, 10 down.
            let bh = h + 20.0;
            p.pos = Point::new(colw - bw + 14.0, 10.0);
            p.w = tw + 1.0;
            s.prims.push(quad(
                Rectangle::new(Point::new(colw - bw, 0.0), Size::new(bw, bh)),
                t.fill_2,
                Color::TRANSPARENT,
                0.0,
                (bh / 2.0).min(18.0),
            ));
            s.parts.push(p);
            s.h = bh;
        }
        RowKind::Caption { center, rule } => {
            let text = first.plain();
            let (cp, cs) = label(
                &text,
                12.0,
                Weight::Normal,
                false,
                Color {
                    a: 0.4,
                    ..t.label_3
                },
                t,
            );
            let x = if *center {
                (colw - cs.width) / 2.0
            } else {
                0.0
            };
            s.prims.push(Prim::Text {
                para: cp,
                pos: Point::new(x, 0.0),
                color: Color {
                    a: 0.4,
                    ..t.label_3
                },
            });
            if *rule {
                let y = cs.height / 2.0;
                s.prims.push(quad(
                    Rectangle::new(Point::new(x - 58.0, y), Size::new(48.0, 0.5)),
                    t.sep_strong,
                    Color::TRANSPARENT,
                    0.0,
                    0.0,
                ));
                s.prims.push(quad(
                    Rectangle::new(Point::new(x + cs.width + 10.0, y), Size::new(48.0, 0.5)),
                    t.sep_strong,
                    Color::TRANSPARENT,
                    0.0,
                    0.0,
                ));
            }
            s.h = cs.height;
        }
        RowKind::Rule => {
            s.prims.push(quad(
                Rectangle::new(Point::ORIGIN, Size::new(colw, 1.0)),
                t.sep_strong,
                Color::TRANSPARENT,
                0.0,
                0.0,
            ));
            s.h = 1.0;
        }
        RowKind::Chips(chips) => shape_chips(&mut s, chips, colw, t),
        RowKind::LoadEarlier { loading } => {
            let (lp, ls) = label(
                if *loading {
                    "Loading earlier\u{2026}"
                } else {
                    "Show earlier"
                },
                11.0,
                Weight::Medium,
                false,
                t.label_2,
                t,
            );
            let x = (colw - ls.width) / 2.0;
            s.prims.push(Prim::Text {
                para: lp,
                pos: Point::new(x, 3.0),
                color: t.label_2,
            });
            s.load_earlier = Some(Rectangle::new(
                Point::new(x - 10.0, 0.0),
                Size::new(ls.width + 20.0, ls.height + 6.0),
            ));
            s.h = ls.height + 6.0;
        }
    }
    s
}

fn shape_table(s: &mut Shaped, row: &Row, cols: usize, colw: f32, t: &Tokens, m: &Metrics) {
    let cols = cols.max(1);
    let cells = &row.paras;
    let nrows = cells.len().div_ceil(cols);
    let (px, py) = (12.0, 5.0);
    // Natural widths.
    let mut natural = vec![0.0f32; cols];
    for (i, c) in cells.iter().enumerate() {
        let spans = spans_of(c, m.table, t, t.label, Weight::Normal, i < cols, false);
        let p = make(&spans, f32::INFINITY, m.table, 1.5, Wrapping::None);
        let w = p.min_bounds().width + 2.0 * px;
        natural[i % cols] = natural[i % cols].max(w);
    }
    let sum: f32 = natural.iter().sum::<f32>().max(1.0);
    let widths: Vec<f32> = natural.iter().map(|n| (colw * n / sum).max(56.0)).collect();
    let scale = colw / widths.iter().sum::<f32>();
    let widths: Vec<f32> = widths.iter().map(|w| w * scale).collect();
    let mut xs = vec![0.0f32; cols];
    for i in 1..cols {
        xs[i] = xs[i - 1] + widths[i - 1];
    }
    let mut y = 0.0;
    let mut plain_start = 0usize;
    let mut row_tops = Vec::new();
    for r in 0..nrows {
        let mut built = Vec::new();
        let mut rh = 0.0f32;
        for c in 0..cols {
            let i = r * cols + c;
            let Some(cell) = cells.get(i) else { continue };
            let (p, h) = part(
                cell,
                m.table,
                1.5,
                (widths[c] - 2.0 * px).max(20.0),
                Wrapping::WordOrGlyph,
                Point::new(xs[c] + px, y + py),
                plain_start,
                t.label,
                Weight::Normal,
                r == 0,
                false,
                t,
            );
            plain_start += p.plain_len + 1;
            rh = rh.max(h);
            built.push(p);
        }
        let rh = rh + 2.0 * py;
        row_tops.push((y, rh));
        s.parts.extend(built);
        y += rh;
    }
    s.h = y;
    let radius = 12.0;
    // Header fill (top corners rounded), then row rules, then the border on top.
    if let Some(&(_, hh)) = row_tops.first() {
        s.prims.push(Prim::Quad {
            rect: Rectangle::new(Point::ORIGIN, Size::new(colw, hh)),
            fill: t.fill_1,
            border: Color::TRANSPARENT,
            bw: 0.0,
            radius: border::Radius {
                top_left: radius,
                top_right: radius,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
        });
    }
    for &(top, _) in row_tops.iter().skip(1) {
        s.prims.push(quad(
            Rectangle::new(Point::new(0.5, top), Size::new(colw - 1.0, 1.0)),
            t.sep,
            Color::TRANSPARENT,
            0.0,
            0.0,
        ));
    }
    s.prims.push(quad(
        Rectangle::new(Point::ORIGIN, Size::new(colw, s.h)),
        Color::TRANSPARENT,
        t.sep,
        1.0,
        radius,
    ));
}

fn shape_chips(s: &mut Shaped, chips: &[Chip], colw: f32, t: &Tokens) {
    let (h, gap) = (26.0, 6.0);
    let (mut x, mut y) = (0.0f32, 0.0f32);
    for c in chips {
        // Swift ChipView: padding 10, spacing 5, an 11 pt symbol, 12 pt medium text, a small chevron.
        let color = if c.error { t.label } else { t.label_2 };
        let (lp, ls) = label(&c.text, 12.0, Weight::Medium, false, color, t);
        let w = 10.0 + 13.0 + 5.0 + ls.width + if c.expandable { 5.0 + 8.0 } else { 0.0 } + 10.0;
        if x > 0.0 && x + w > colw {
            x = 0.0;
            y += h + gap;
        }
        let rect = Rectangle::new(Point::new(x, y), Size::new(w, h));
        s.chips.push(ChipGeo {
            id: c.id.clone(),
            rect,
            expandable: c.expandable,
            label: lp,
            label_pos: Point::new(x + 10.0 + 13.0 + 5.0, y + (h - ls.height) / 2.0),
            label_color: color,
            icon: c.icon,
            icon_rect: Rectangle::new(Point::new(x + 10.0, y + 6.5), Size::new(13.0, 13.0)),
            chevron: c.expandable.then(|| {
                (
                    if c.expanded {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    Rectangle::new(
                        Point::new(x + w - 10.0 - 8.0, y + 9.0),
                        Size::new(8.0, 8.0),
                    ),
                )
            }),
        });
        x += w + gap;
    }
    s.h = y + h;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selectable::flow::model::{Inline, Para};
    use crate::theme::Tokens;

    fn width(para: &Para) -> f32 {
        let t = Tokens::LIGHT;
        let spans = spans_of(para, 15.0, &t, t.label, Weight::Normal, false, false);
        make(&spans, f32::INFINITY, 15.0, 1.55, Wrapping::None)
            .min_bounds()
            .width
    }

    fn inl(text: &str, italic: bool) -> Inline {
        Inline {
            text: text.into(),
            italic,
            ..Default::default()
        }
    }

    // Splitting CJK text into styled spans must not add any advance at the span boundaries.
    #[test]
    fn cjk_span_boundaries_add_no_gap() {
        let one = width(&Para {
            inlines: vec![inl("受到衰減影響", false)],
        });
        let split = width(&Para {
            inlines: vec![inl("受到", false), inl("衰減", true), inl("影響", false)],
        });
        assert!(one > 0.0);
        assert!(
            (one - split).abs() < 0.5,
            "one span {one} vs three spans {split}"
        );
        let spaced = width(&Para {
            inlines: vec![inl("受到 ", false), inl("衰減", true), inl(" 影響", false)],
        });
        assert!(
            spaced > one + 2.0,
            "the check can see a real space: {spaced} vs {one}"
        );
    }
}
