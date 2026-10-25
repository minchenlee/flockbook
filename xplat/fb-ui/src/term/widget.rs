//! The terminal widget. It draws a [`GridSnapshot`] and reports what the user does. It owns no terminal state and
//! sends nothing itself: every callback is optional, and a widget without `on_input` has no input path at all
//! (that is how read-only works).
//!
//! - Text: JetBrains Mono, one fixed cell (`CELL_W` x `CELL_H`). ASCII runs are drawn as one string; every other
//!   character is drawn alone at its own cell, so a CJK fallback font cannot shift the columns.
//! - The default background is not drawn (transparent). Cells with their own background, inverse cells and the block
//!   cursor are drawn as quads.
//! - Keys reach `on_input` only while `focused` is true. A key press that encodes to nothing (Cmd combinations,
//!   Ctrl+`) is left for the app.

use super::grid::{CursorShape, GridSnapshot};
use super::keys;
use super::palette::{cell_colors, TermPalette};
use super::selection::{Cell, Selection};
use crate::fonts;
use iced::advanced::clipboard::{self, Clipboard};
use iced::advanced::input_method::{InputMethod, Preedit};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::text::{self, Renderer as _, Shaping, Wrapping};
use iced::advanced::widget::{tree, Tree, Widget};
use iced::advanced::{mouse, Shell};
use iced::{alignment, keyboard, window, Border, Color, Element, Event, Length, Pixels, Point, Rectangle, Size, Theme};
use std::ops::Range;
use std::time::{Duration, Instant};

pub const FONT_SIZE: f32 = 13.0;
/// JetBrains Mono advances 0.6 em.
pub const CELL_W: f32 = 7.8;
pub const CELL_H: f32 = 17.0;
/// Inner padding of the grid inside the widget. SwiftTerm draws from the view edge (Swift `TerminalSheet`), so 0.
pub const PAD: f32 = 0.0;

/// Columns and rows that fit a widget of `w` x `h` points.
pub fn grid_size(w: f32, h: f32) -> (usize, usize) {
    let cols = ((w - 2.0 * PAD) / CELL_W).floor().max(1.0) as usize;
    let rows = ((h - 2.0 * PAD) / CELL_H).floor().max(1.0) as usize;
    (cols, rows)
}

/// Rows that intersect the clip (a widget inside a scrollable draws only what shows). `oy` is the y of row 0.
pub(crate) fn visible_rows(oy: f32, clip_y: f32, clip_h: f32, rows: usize) -> Range<usize> {
    let first = (((clip_y - oy) / CELL_H).floor().max(0.0) as usize).min(rows);
    let last = (((clip_y + clip_h - oy) / CELL_H).ceil().max(0.0) as usize).min(rows);
    first..last.max(first)
}

/// True when the block cursor covers this cell. A wide character's spacer cell is covered with it.
pub(crate) fn on_block(snap: &GridSnapshot, block: Option<(usize, usize)>, col: usize, row: usize) -> bool {
    let Some(b) = block else { return false };
    b == (col, row) || (col > 0 && b == (col - 1, row) && snap.cell(col, row).is_some_and(|c| c.spacer) && snap.cell(col - 1, row).is_some_and(|c| c.wide))
}

/// The copy chord: Cmd+C on macOS, Ctrl+Shift+C elsewhere (plain Ctrl+C is the program's interrupt).
pub(crate) fn is_copy_chord(key: &keyboard::Key, mods: keyboard::Modifiers, mac: bool) -> bool {
    let is_c = matches!(key.as_ref(), keyboard::Key::Character(c) if c.eq_ignore_ascii_case("c"));
    if mac {
        is_c && mods.logo() && !mods.control() && !mods.alt() && !mods.shift()
    } else {
        is_c && mods.control() && mods.shift() && !mods.alt() && !mods.logo()
    }
}

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

type Callback<'a, M, T> = Option<Box<dyn Fn(T) -> M + 'a>>;

pub struct Terminal<'a, M> {
    snap: &'a GridSnapshot,
    palette: TermPalette,
    height: Length,
    focused: bool,
    app_cursor: bool,
    on_input: Callback<'a, M, Vec<u8>>,
    on_paste: Callback<'a, M, String>,
    on_resize: Callback<'a, M, (usize, usize)>,
    /// `(dy, precise)`: `dy` > 0 scrolls toward older content. `precise` is a trackpad (pixels).
    on_wheel: Callback<'a, M, (f32, bool)>,
    on_focus: Callback<'a, M, bool>,
    on_copy: Callback<'a, M, String>,
    /// Like `on_wheel`, but the event is not captured: a scrollable around the widget still scrolls.
    on_wheel_passive: Callback<'a, M, (f32, bool)>,
}

impl<'a, M> Terminal<'a, M> {
    pub fn new(snap: &'a GridSnapshot, palette: TermPalette) -> Self {
        Terminal { snap, palette, height: Length::Fill, focused: false, app_cursor: false, on_input: None, on_paste: None, on_resize: None, on_wheel: None, on_focus: None, on_copy: None, on_wheel_passive: None }
    }

    /// Fixed height for a terminal inside a scrollable (the default fills the space).
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// The program set application cursor keys (arrows send `ESC O x`).
    pub fn app_cursor(mut self, on: bool) -> Self {
        self.app_cursor = on;
        self
    }

    /// Bytes for the program: keys and IME text. Without this, the widget never reports a key.
    pub fn on_input(mut self, f: impl Fn(Vec<u8>) -> M + 'a) -> Self {
        self.on_input = Some(Box::new(f));
        self
    }

    /// Cmd+V with text on the clipboard. Only reported when `on_input` is set.
    pub fn on_paste(mut self, f: impl Fn(String) -> M + 'a) -> Self {
        self.on_paste = Some(Box::new(f));
        self
    }

    /// The grid size (cols, rows) the widget can show. Reported when it changes, and once on the first draw.
    pub fn on_resize(mut self, f: impl Fn((usize, usize)) -> M + 'a) -> Self {
        self.on_resize = Some(Box::new(f));
        self
    }

    pub fn on_wheel(mut self, f: impl Fn((f32, bool)) -> M + 'a) -> Self {
        self.on_wheel = Some(Box::new(f));
        self
    }

    /// Wheel over the widget, reported without taking the event (a surrounding scrollable keeps scrolling).
    pub fn on_wheel_passive(mut self, f: impl Fn((f32, bool)) -> M + 'a) -> Self {
        self.on_wheel_passive = Some(Box::new(f));
        self
    }

    /// The selected text, when the copy chord is pressed. Without this the widget writes it to the clipboard itself.
    /// Copying never sends anything to the program.
    pub fn on_copy(mut self, f: impl Fn(String) -> M + 'a) -> Self {
        self.on_copy = Some(Box::new(f));
        self
    }

    /// A click inside asks for focus (`true`), a click outside gives it up (`false`).
    pub fn on_focus(mut self, f: impl Fn(bool) -> M + 'a) -> Self {
        self.on_focus = Some(Box::new(f));
        self
    }
}

#[derive(Default)]
struct State {
    reported: Option<(usize, usize)>,
    preedit: String,
    /// The selection is local to the widget: it lives here and nowhere else.
    sel: Option<Selection>,
    dragging: bool,
    last_click: Option<(Instant, Cell)>,
    /// Grid size the selection belongs to.
    dims: (usize, usize),
}

/// The cell under `p`, clamped to the grid.
fn cell_at(bounds: Rectangle, p: Point, cols: usize, rows: usize) -> Cell {
    let col = ((p.x - bounds.x - PAD) / CELL_W).floor().max(0.0) as usize;
    let row = ((p.y - bounds.y - PAD) / CELL_H).floor().max(0.0) as usize;
    (col.min(cols.saturating_sub(1)), row.min(rows.saturating_sub(1)))
}

impl<'a, M: 'a> Widget<M, Theme, iced::Renderer> for Terminal<'a, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, self.height)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, Length::Fill, self.height)
    }

    fn update(&mut self, tree: &mut Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, _renderer: &iced::Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, M>, _viewport: &Rectangle) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        let (cols, rows) = (self.snap.cols, self.snap.rows);
        if state.dims != (cols, rows) {
            // A different grid: the old cells mean something else.
            state.dims = (cols, rows);
            state.sel = None;
            state.dragging = false;
        }
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let inside = cursor.position_over(bounds).is_some();
                if let Some(f) = &self.on_focus {
                    if inside != self.focused {
                        shell.publish(f(inside));
                    }
                }
                if inside {
                    if let Some(p) = cursor.position() {
                        let cell = cell_at(bounds, p, cols, rows);
                        let now = Instant::now();
                        let double = state.last_click.is_some_and(|(at, c)| c == cell && now.duration_since(at) < DOUBLE_CLICK);
                        if double {
                            state.sel = Some(Selection::word_at(self.snap, cell));
                            state.last_click = None;
                        } else {
                            state.sel = Some(Selection::at(cell));
                            state.last_click = Some((now, cell));
                        }
                        state.dragging = true;
                        shell.request_redraw();
                    }
                    shell.capture_event();
                } else {
                    state.sel = None;
                    state.dragging = false;
                    state.last_click = None;
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if state.dragging => {
                if let (Some(p), Some(sel)) = (cursor.position(), state.sel.as_mut()) {
                    sel.extend(cell_at(bounds, p, cols, rows));
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                // A click without a drag selects nothing.
                if state.sel.is_some_and(|s| !s.has_extent()) {
                    state.sel = None;
                    shell.request_redraw();
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if is_copy_chord(key, *modifiers, cfg!(target_os = "macos")) => {
                // Local only: the text goes to the clipboard, never to the program.
                let copied = state.sel.map(|s| s.text(self.snap)).filter(|t| !t.is_empty());
                let has_text = copied.is_some();
                if let Some(text) = copied {
                    match &self.on_copy {
                        Some(f) => shell.publish(f(text)),
                        None => clipboard.write(clipboard::Kind::Standard, text),
                    }
                }
                // Cmd+C with nothing selected stays free for the app. Ctrl+Shift+C is never a program key.
                if has_text || !cfg!(target_os = "macos") {
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if cursor.position_over(bounds).is_none() {
                    return;
                }
                let (dy, precise) = match *delta {
                    mouse::ScrollDelta::Lines { y, .. } => (y, false),
                    mouse::ScrollDelta::Pixels { y, .. } => (y, true),
                };
                if let (Some(f), true) = (&self.on_wheel, dy != 0.0) {
                    shell.publish(f((dy, precise)));
                    shell.capture_event();
                } else if let (Some(f), true) = (&self.on_wheel_passive, dy != 0.0) {
                    shell.publish(f((dy, precise)));
                }
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, text, .. }) if self.focused => {
                let Some(on_input) = &self.on_input else { return };
                if modifiers.logo() {
                    // Cmd+V is the paste path. Every other Cmd key is the app's.
                    if matches!(key.as_ref(), keyboard::Key::Character("v")) {
                        if let (Some(f), Some(t)) = (&self.on_paste, clipboard.read(clipboard::Kind::Standard)) {
                            if !t.is_empty() {
                                shell.publish(f(t));
                                shell.capture_event();
                            }
                        }
                    }
                    return;
                }
                if let Some(bytes) = keys::encode_key(key, *modifiers, text.as_deref(), self.app_cursor) {
                    // Typing ends a selection (the program redraws under it).
                    state.sel = None;
                    shell.publish(on_input(bytes));
                    shell.capture_event();
                }
            }
            Event::InputMethod(im) if self.focused && self.on_input.is_some() => match im {
                iced::advanced::input_method::Event::Preedit(s, _) => {
                    state.preedit = s.clone();
                    shell.request_redraw();
                }
                iced::advanced::input_method::Event::Commit(s) => {
                    state.preedit.clear();
                    if let Some(f) = &self.on_input {
                        shell.publish(f(keys::commit_bytes(s)));
                    }
                    shell.capture_event();
                }
                _ => {}
            },
            Event::Window(window::Event::RedrawRequested(_)) => {
                let size = grid_size(bounds.width, bounds.height);
                if state.reported != Some(size) {
                    state.reported = Some(size);
                    if let Some(f) = &self.on_resize {
                        shell.publish(f(size));
                    }
                }
                if self.focused && self.on_input.is_some() {
                    let (col, row) = self.snap.cursor.map(|c| (c.col, c.row)).unwrap_or((0, 0));
                    let rect = Rectangle { x: bounds.x + PAD + col as f32 * CELL_W, y: bounds.y + PAD + row as f32 * CELL_H, width: CELL_W, height: CELL_H };
                    let preedit = (!state.preedit.is_empty()).then(|| Preedit { content: state.preedit.clone(), selection: None, text_size: Some(Pixels(FONT_SIZE)) });
                    shell.request_input_method(&InputMethod::Enabled { cursor: rect, purpose: iced::advanced::input_method::Purpose::Terminal, preedit });
                }
            }
            _ => {}
        }
    }

    fn draw(&self, tree: &Tree, renderer: &mut iced::Renderer, _theme: &Theme, _defaults: &renderer::Style, layout: Layout<'_>, _cursor: mouse::Cursor, viewport: &Rectangle) {
        let bounds = layout.bounds();
        let Some(clip) = bounds.intersection(viewport) else { return };
        let (ox, oy) = (bounds.x + PAD, bounds.y + PAD);
        let (cols, rows) = (self.snap.cols, self.snap.rows);
        let block_at = self.snap.cursor.filter(|c| c.shape == CursorShape::Block && self.focused).map(|c| (c.col, c.row));
        let sel = tree.state.downcast_ref::<State>().sel;
        // Only the rows that show: a long history sits inside a scrollable and most rows are out of view.
        let shown = visible_rows(oy, clip.y, clip.height, rows);
        let quad = |renderer: &mut iced::Renderer, r: Rectangle, color: Color| {
            renderer.fill_quad(renderer::Quad { bounds: r, ..Default::default() }, color);
        };

        renderer.with_layer(clip, |renderer| {
            // Backgrounds.
            for row in shown.clone() {
                let y = oy + row as f32 * CELL_H;
                let mut c = 0;
                while c < cols {
                    let cell = &self.snap.cells[row * cols + c];
                    let colors = cell_colors(&self.palette, cell, on_block(self.snap, block_at, c, row));
                    let Some(bg) = colors.bg else {
                        c += 1;
                        continue;
                    };
                    let mut end = c + 1;
                    while end < cols {
                        let next = &self.snap.cells[row * cols + end];
                        if cell_colors(&self.palette, next, on_block(self.snap, block_at, end, row)).bg != Some(bg) {
                            break;
                        }
                        end += 1;
                    }
                    quad(renderer, Rectangle { x: ox + c as f32 * CELL_W, y, width: (end - c) as f32 * CELL_W, height: CELL_H }, bg);
                    c = end;
                }
            }

            // The selection, under the text.
            if let Some(sel) = sel {
                for row in shown.clone() {
                    if let Some((a, b)) = sel.row_span(row, cols) {
                        quad(renderer, Rectangle { x: ox + a as f32 * CELL_W, y: oy + row as f32 * CELL_H, width: (b - a + 1) as f32 * CELL_W, height: CELL_H }, self.palette.selection);
                    }
                }
            }

            // Cursor shapes other than the filled block.
            if let Some(cur) = self.snap.cursor {
                let w = if cur.wide { 2.0 * CELL_W } else { CELL_W };
                let (x, y) = (ox + cur.col as f32 * CELL_W, oy + cur.row as f32 * CELL_H);
                let color = self.palette.cursor;
                if !self.focused {
                    renderer.fill_quad(renderer::Quad { bounds: Rectangle { x, y, width: w, height: CELL_H }, border: Border { color, width: 1.0, radius: 0.0.into() }, ..Default::default() }, Color::TRANSPARENT);
                } else {
                    match cur.shape {
                        CursorShape::Block => {}
                        CursorShape::Underline => quad(renderer, Rectangle { x, y: y + CELL_H - 2.0, width: w, height: 2.0 }, color),
                        CursorShape::Beam => quad(renderer, Rectangle { x, y, width: 2.0, height: CELL_H }, color),
                    }
                }
            }

            // Text, underline, strike.
            for row in shown {
                let y = oy + row as f32 * CELL_H;
                let mut run = String::new();
                let mut run_start = 0usize;
                let mut run_style: Option<(Color, bool)> = None;
                let flush = |renderer: &mut iced::Renderer, run: &mut String, start: usize, style: Option<(Color, bool)>| {
                    if run.is_empty() {
                        return;
                    }
                    if let Some((color, bold)) = style {
                        let width = run.chars().count() as f32 * CELL_W;
                        draw_text(renderer, run.clone(), Shaping::Basic, Point::new(ox + start as f32 * CELL_W, y), width, color, bold, clip);
                    }
                    run.clear();
                };
                for c in 0..cols {
                    let cell = &self.snap.cells[row * cols + c];
                    if cell.spacer {
                        continue;
                    }
                    let mut colors = cell_colors(&self.palette, cell, on_block(self.snap, block_at, c, row));
                    if sel.and_then(|s| s.row_span(row, cols)).is_some_and(|(a, b)| c >= a && c <= b) {
                        if let Some(fg) = self.palette.selection_fg { colors.fg = fg; }
                    }
                    let cell_w = if cell.wide { 2.0 * CELL_W } else { CELL_W };
                    let x = ox + c as f32 * CELL_W;
                    if cell.underline {
                        quad(renderer, Rectangle { x, y: y + CELL_H - 2.0, width: cell_w, height: 1.0 }, colors.fg);
                    }
                    if cell.strike {
                        quad(renderer, Rectangle { x, y: y + CELL_H / 2.0, width: cell_w, height: 1.0 }, colors.fg);
                    }
                    let blank = cell.hidden || cell.ch == ' ' || cell.ch == '\0';
                    if blank {
                        // A blank breaks the run (the next run starts at its own column).
                        flush(renderer, &mut run, run_start, run_style);
                        run_style = None;
                        continue;
                    }
                    if cell.ch.is_ascii() && !cell.wide {
                        let style = (colors.fg, cell.bold);
                        if run_style != Some(style) {
                            flush(renderer, &mut run, run_start, run_style);
                            run_style = Some(style);
                            run_start = c;
                        }
                        run.push(cell.ch);
                    } else {
                        flush(renderer, &mut run, run_start, run_style);
                        run_style = None;
                        draw_text(renderer, cell.ch.to_string(), Shaping::Advanced, Point::new(x, y), cell_w, colors.fg, cell.bold, clip);
                    }
                }
                flush(renderer, &mut run, run_start, run_style);
            }
        });
    }

    fn mouse_interaction(&self, _tree: &Tree, layout: Layout<'_>, cursor: mouse::Cursor, _viewport: &Rectangle, _renderer: &iced::Renderer) -> mouse::Interaction {
        if cursor.position_over(layout.bounds()).is_some() {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(renderer: &mut iced::Renderer, content: String, shaping: Shaping, at: Point, width: f32, color: Color, bold: bool, clip: Rectangle) {
    let t = text::Text {
        content,
        bounds: Size::new(width + 1.0, CELL_H),
        size: Pixels(FONT_SIZE),
        line_height: text::LineHeight::Absolute(Pixels(CELL_H)),
        font: fonts::mono(),
        align_x: text::Alignment::Left,
        align_y: alignment::Vertical::Top,
        shaping,
        wrapping: Wrapping::None,
    };
    renderer.fill_text(t.clone(), at, color, clip);
    if bold {
        // Only the Regular face is bundled: a second pass half a point to the right reads as bold.
        renderer.fill_text(t, Point::new(at.x + 0.5, at.y), color, clip);
    }
}

impl<'a, M: 'a> From<Terminal<'a, M>> for Element<'a, M> {
    fn from(t: Terminal<'a, M>) -> Self {
        Element::new(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{GridSnapshot, TermGrid};
    use iced::keyboard::{Key, Modifiers};
    use iced_test::Simulator;

    #[test]
    fn grid_size_follows_the_widget_size() {
        // The padding is `PAD` on each side (0 since the grid draws from the edge, as SwiftTerm).
        assert_eq!(grid_size(600.0, 360.0), (((600.0 - 2.0 * PAD) / CELL_W).floor() as usize, ((360.0 - 2.0 * PAD) / CELL_H).floor() as usize));
        assert_eq!(grid_size(10.0, 10.0), (1, 1), "never below one cell");
        let (c, r) = grid_size(880.0, 560.0);
        assert!(c >= 100 && r >= 30, "{c}x{r}");
    }

    #[test]
    fn only_rows_in_the_viewport_are_drawn() {
        // 1000 rows, the clip shows 10 rows starting 500 rows down.
        let oy = 8.0;
        let top = oy + 500.0 * CELL_H;
        let r = visible_rows(oy, top, 10.0 * CELL_H, 1000);
        assert_eq!(r, 500..510);
        // A clip that starts mid-row includes that row; one that ends mid-row too.
        assert_eq!(visible_rows(oy, top + 3.0, 10.0 * CELL_H, 1000), 500..511);
        // Before the first row and past the last: clamped.
        assert_eq!(visible_rows(oy, 0.0, 4.0 * CELL_H, 3), 0..3);
        assert_eq!(visible_rows(oy, 5000.0, 100.0, 3), 3..3);
        assert!(visible_rows(oy, top, 0.0, 1000).is_empty());
    }

    #[test]
    fn the_block_cursor_on_a_wide_char_fills_both_cells() {
        let mut g = TermGrid::new(1, 1);
        g.feed(true, 10, 1, "\u{4f60}x".as_bytes());
        let snap = g.snapshot();
        assert!(snap.cell(0, 0).unwrap().wide && snap.cell(1, 0).unwrap().spacer);
        let block = Some((0, 0));
        assert!(on_block(&snap, block, 0, 0));
        assert!(on_block(&snap, block, 1, 0), "the spacer cell is under the cursor too");
        assert!(!on_block(&snap, block, 2, 0));
        assert!(!on_block(&snap, None, 0, 0));
    }

    #[test]
    fn the_copy_chord_is_cmd_c_on_mac_and_ctrl_shift_c_elsewhere() {
        let c = Key::Character("c".into());
        let big_c = Key::Character("C".into());
        assert!(is_copy_chord(&c, Modifiers::LOGO, true));
        assert!(!is_copy_chord(&c, Modifiers::CTRL, true), "Ctrl+C is the program's interrupt");
        assert!(!is_copy_chord(&c, Modifiers::LOGO | Modifiers::SHIFT, true));
        assert!(is_copy_chord(&big_c, Modifiers::CTRL | Modifiers::SHIFT, false));
        assert!(is_copy_chord(&c, Modifiers::CTRL | Modifiers::SHIFT, false));
        assert!(!is_copy_chord(&c, Modifiers::CTRL, false), "Ctrl+C stays the interrupt");
        assert!(!is_copy_chord(&c, Modifiers::LOGO, false));
    }

    // ---- selection through the widget (headless)

    #[derive(Debug, Clone, PartialEq)]
    enum M {
        Copy(String),
        Input(Vec<u8>),
        Focus(bool),
    }

    fn grid(s: &str) -> GridSnapshot {
        let mut g = TermGrid::new(1, 1);
        g.feed(true, 40, 5, s.as_bytes());
        g.snapshot()
    }

    fn at(col: usize, row: usize) -> Point {
        Point::new(PAD + col as f32 * CELL_W + CELL_W / 2.0, PAD + row as f32 * CELL_H + CELL_H / 2.0)
    }

    fn mouse_move(p: Point) -> Event {
        Event::Mouse(mouse::Event::CursorMoved { position: p })
    }
    fn press() -> Event {
        Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
    }
    fn release() -> Event {
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
    }
    fn key(k: &str, m: Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Character(k.into()),
            modified_key: Key::Character(k.into()),
            physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: m,
            text: Some(k.into()),
            repeat: false,
        })
    }
    fn copy_key() -> Event {
        if cfg!(target_os = "macos") {
            key("c", Modifiers::LOGO)
        } else {
            key("C", Modifiers::CTRL | Modifiers::SHIFT)
        }
    }

    fn run(snap: &GridSnapshot, input: bool, focused: bool, steps: Vec<(Point, Vec<Event>)>) -> Vec<M> {
        std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
        let palette = TermPalette::of(&crate::theme::Tokens::of(crate::theme::Mode::Light));
        let mut t = Terminal::new(snap, palette).focused(focused).on_copy(M::Copy).on_focus(M::Focus);
        if input {
            t = t.on_input(M::Input);
        }
        let mut sim = Simulator::with_size(iced::Settings::default(), Size::new(600.0, 200.0), Element::from(t));
        for (p, evs) in steps {
            sim.point_at(p);
            let _ = sim.simulate(evs);
        }
        sim.into_messages().collect()
    }

    #[test]
    fn a_drag_selects_and_the_copy_key_copies_without_input() {
        let snap = grid("hello world\r\nsecond line");
        let msgs = run(&snap, true, true, vec![(at(0, 0), vec![mouse_move(at(0, 0)), press()]), (at(4, 0), vec![mouse_move(at(4, 0)), release()]), (at(4, 0), vec![copy_key()])]);
        assert!(msgs.contains(&M::Copy("hello".into())), "{msgs:?}");
        assert!(!msgs.iter().any(|m| matches!(m, M::Input(_))), "copying never reaches the program: {msgs:?}");
    }

    #[test]
    fn a_selection_across_rows_copies_both() {
        let snap = grid("hello world\r\nsecond line");
        let msgs = run(&snap, false, false, vec![(at(6, 0), vec![mouse_move(at(6, 0)), press()]), (at(5, 1), vec![mouse_move(at(5, 1)), release()]), (at(5, 1), vec![copy_key()])]);
        assert!(msgs.contains(&M::Copy("world\nsecond".into())), "works without input or focus (history view): {msgs:?}");
    }

    #[test]
    fn a_double_click_selects_a_word() {
        let snap = grid("hello world");
        let msgs = run(&snap, false, false, vec![(at(7, 0), vec![mouse_move(at(7, 0)), press(), release(), press(), release(), copy_key()])]);
        assert!(msgs.contains(&M::Copy("world".into())), "{msgs:?}");
    }

    #[test]
    fn copy_with_nothing_selected_does_nothing_and_a_click_selects_nothing() {
        let snap = grid("hello");
        let msgs = run(&snap, true, true, vec![(at(2, 0), vec![mouse_move(at(2, 0)), press(), release(), copy_key()])]);
        assert!(!msgs.iter().any(|m| matches!(m, M::Copy(_) | M::Input(_))), "{msgs:?}");
    }

    #[test]
    fn a_click_outside_clears_the_selection() {
        let snap = grid("hello world");
        let outside = Point::new(700.0, 300.0);
        let msgs = run(&snap, false, false, vec![(at(0, 0), vec![mouse_move(at(0, 0)), press()]), (at(4, 0), vec![mouse_move(at(4, 0)), release()]), (outside, vec![press(), release()]), (outside, vec![copy_key()])]);
        assert!(!msgs.iter().any(|m| matches!(m, M::Copy(_))), "{msgs:?}");
    }

    #[test]
    fn ctrl_c_still_goes_to_the_program_with_a_selection() {
        let snap = grid("hello world");
        let msgs = run(&snap, true, true, vec![(at(0, 0), vec![mouse_move(at(0, 0)), press()]), (at(4, 0), vec![mouse_move(at(4, 0)), release()]), (at(4, 0), vec![key("c", Modifiers::CTRL)])]);
        assert!(msgs.contains(&M::Input(vec![3])), "{msgs:?}");
        assert!(!msgs.iter().any(|m| matches!(m, M::Copy(_))));
    }
}
