//! Data model of the flowing conversation: rows, selection anchors and the document (rows plus the virtual
//! window). No iced widget code here, so the logic is unit-testable.
//!
//! A conversation is a flat list of rows. Each row has a stable `RowId` (derived from the source item id), so a
//! selection `(RowId, byte offset)` survives virtualization, appends and prepends. Copy is built from the rows,
//! never from mounted widgets.

use super::super::model::{clamp_boundary, Pos};
use super::super::words;
use crate::icons::Icon;
use crate::virt::{VirtWindow, BLOCK_GAP_PX};
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

pub type RowId = u64;

/// Gap the virtual window puts between slots. A row's own `gap_before` is folded into its slot as `lead`.
pub const GAP: f32 = BLOCK_GAP_PX;
/// Space above the first row and below the last row.
pub const PAD_TOP: f32 = 24.0;
pub const PAD_BOTTOM: f32 = 24.0;
/// Minimum space left and right of the column.
pub const GUTTER: f32 = 24.0;

/// A run of text with one style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
    /// Syntax class (code rows only).
    pub tone: crate::highlight::Tone,
}

impl Inline {
    pub fn plain(text: impl Into<String>) -> Inline {
        Inline {
            text: text.into(),
            ..Default::default()
        }
    }
}

/// One paragraph of styled text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Para {
    pub inlines: Vec<Inline>,
}

impl Para {
    pub fn text(s: impl Into<String>) -> Para {
        Para {
            inlines: vec![Inline::plain(s)],
        }
    }
    pub fn plain(&self) -> String {
        self.inlines.iter().map(|i| i.text.as_str()).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.inlines.iter().all(|i| i.text.is_empty())
    }
}

/// A tool step chip.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub id: String,
    pub icon: Icon,
    pub text: String,
    pub expandable: bool,
    pub expanded: bool,
    pub running: bool,
    pub error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    /// Agent prose.
    Prose,
    /// Fixed-size picture placeholder. Decoded handles live only in the viewport owner.
    Picture {
        width: u32,
        height: u32,
        digest: u64,
    },
    Heading(u8),
    /// A list item. The marker is drawn in the gutter and is not part of the copied text.
    ListItem {
        marker: String,
        depth: u8,
    },
    Quote,
    /// Code card. `lang` is the label above the code.
    Code {
        lang: Option<String>,
    },
    /// A table: `paras` are the cells, row-major, the first `cols` are the header.
    Table {
        cols: usize,
    },
    /// The user's right-aligned bubble.
    Bubble,
    /// Footer, status line or time separator. Not selectable.
    Caption {
        center: bool,
        rule: bool,
    },
    Rule,
    Chips(Vec<Chip>),
    /// "Show earlier" at the top of the list.
    LoadEarlier {
        loading: bool,
    },
}

#[derive(Debug, Clone)]
pub struct Row {
    pub id: RowId,
    pub kind: RowKind,
    pub paras: Vec<Para>,
    /// The text copy returns for this row (no markup, no list marker). Empty for rows that are not selectable.
    pub plain: String,
    /// Text put between the previous selectable row and this one when copying.
    pub separator: &'static str,
    /// Space between the previous row's content and this one.
    pub gap_before: f32,
    /// Hash of everything that decides how the row looks. Equal signatures mean equal pixels.
    pub sig: u64,
}

impl Row {
    pub fn new(
        id: RowId,
        kind: RowKind,
        paras: Vec<Para>,
        separator: &'static str,
        gap_before: f32,
    ) -> Row {
        let selectable = !matches!(
            kind,
            RowKind::Picture { .. }
                | RowKind::Caption { .. }
                | RowKind::Rule
                | RowKind::Chips(_)
                | RowKind::LoadEarlier { .. }
        );
        let plain = if !selectable {
            String::new()
        } else if let RowKind::Table { cols } = &kind {
            let mut s = String::new();
            for (i, p) in paras.iter().enumerate() {
                if i > 0 {
                    s.push(if i % cols.max(&1) == 0 { '\n' } else { '\t' });
                }
                s.push_str(&p.plain());
            }
            s
        } else {
            paras.iter().map(Para::plain).collect::<Vec<_>>().join("")
        };
        let mut h = DefaultHasher::new();
        format!("{kind:?}{paras:?}{gap_before}").hash(&mut h);
        Row {
            id,
            kind,
            paras,
            plain,
            separator,
            gap_before,
            sig: h.finish(),
        }
    }

    pub fn selectable(&self) -> bool {
        !self.plain.is_empty()
    }

    /// Slot lead: the row's own gap minus the gap the window already puts between slots. Negative for tight rows.
    pub fn lead(&self, index: usize) -> f32 {
        if index == 0 {
            0.0
        } else {
            self.gap_before - GAP
        }
    }
}

/// Stable id from text parts.
pub fn row_id(parts: &[&str]) -> RowId {
    let mut h = DefaultHasher::new();
    for p in parts {
        p.hash(&mut h);
        0xFFu8.hash(&mut h);
    }
    h.finish()
}

/// Height estimate of a row's slot (lead included), for rows that are not laid out yet.
pub fn estimate(row: &Row, index: usize, colw: f32) -> f32 {
    let width = |s: &str| {
        s.chars()
            .map(|c| {
                if crate::fonts::is_cjk_char(c) {
                    2.0
                } else {
                    1.0
                }
            })
            .sum::<f32>()
    };
    let lines = |s: &str, cpl: f32| {
        s.split('\n')
            .map(|l| (width(l) / cpl.max(8.0)).ceil().max(1.0))
            .sum::<f32>()
    };
    let cpl = colw / 7.6;
    let body = 15.0 * 1.55;
    let content = match &row.kind {
        RowKind::Prose | RowKind::Quote => lines(&row.plain, cpl) * body,
        RowKind::ListItem { depth, .. } => lines(&row.plain, cpl - 3.0 * *depth as f32) * body,
        RowKind::Heading(l) => {
            lines(&row.plain, cpl * 0.8) * (HEADING_SIZES[(*l as usize).clamp(1, 4) - 1] * 1.3)
        }
        RowKind::Bubble => lines(&row.plain, cpl * 0.78) * body + 14.0,
        RowKind::Code { lang } => {
            lines(&row.plain, colw / 7.4) * 20.2 + 28.0 + if lang.is_some() { 20.0 } else { 0.0 }
        }
        RowKind::Table { cols } => ((row.paras.len() / (*cols).max(1)) as f32) * 31.0 + 2.0,
        RowKind::Caption { .. } => 14.3,
        RowKind::Rule => 1.0,
        RowKind::Picture { width, height, .. } => {
            let k = 1f32
                .min(colw.min(480.) / (*width).max(1) as f32)
                .min(640. / (*height).max(1) as f32);
            (*height as f32 * k).round().max(1.)
        }
        RowKind::Chips(c) => {
            ((c.len() as f32 * 130.0 / colw).ceil().max(1.0)) * 26.0
                + 6.0 * ((c.len() as f32 * 130.0 / colw).ceil() - 1.0).max(0.0)
        }
        RowKind::LoadEarlier { .. } => 22.0,
    };
    row.lead(index) + content
}

pub const HEADING_SIZES: [f32; 4] = [22.0, 19.0, 17.0, 15.0];

/// Where the user clicked or dragged: a row (by stable id) and a byte offset in its plain text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Anchor {
    pub id: RowId,
    pub offset: usize,
}

/// A selection across rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlowSel {
    pub anchor: Anchor,
    pub focus: Anchor,
}

impl FlowSel {
    pub fn is_empty(&self) -> bool {
        self.anchor == self.focus
    }

    /// `(start, end)` in document order, as row indexes. `None` when a row is gone from the document.
    pub fn resolve(&self, index: &HashMap<RowId, usize>) -> Option<(Pos, Pos)> {
        let a = Pos {
            block: *index.get(&self.anchor.id)?,
            offset: self.anchor.offset,
        };
        let f = Pos {
            block: *index.get(&self.focus.id)?,
            offset: self.focus.offset,
        };
        Some(if a <= f { (a, f) } else { (f, a) })
    }

    /// The selected plain text, built from the model. Rows join by their separators; rows without text are skipped.
    pub fn text(&self, rows: &[Arc<Row>], index: &HashMap<RowId, usize>) -> String {
        let Some((start, end)) = self.resolve(index) else {
            return String::new();
        };
        let mut out = String::new();
        let mut first = true;
        for i in start.block..=end.block.min(rows.len().saturating_sub(1)) {
            let row = &rows[i];
            if !row.selectable() {
                continue;
            }
            let from = clamp_boundary(&row.plain, if i == start.block { start.offset } else { 0 });
            let to = clamp_boundary(
                &row.plain,
                if i == end.block {
                    end.offset
                } else {
                    row.plain.len()
                },
            );
            if !first {
                out.push_str(row.separator);
            }
            first = false;
            if from < to {
                out.push_str(&row.plain[from..to]);
            }
        }
        out
    }
}

/// What the widget measured for one mounted row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    pub id: RowId,
    /// Slot height (lead included).
    pub h: f32,
    pub colw: f32,
}

/// What the widget reports after layout: measured row heights to apply, plus the scroll offset it really used.
#[derive(Debug, Default)]
pub struct SinkData {
    pub measures: Vec<Measure>,
    /// The effective scroll offset of the last layout (differs from the app's offset while pinned to the bottom).
    pub eff: f32,
    /// The real total height of the last layout.
    pub total: f32,
}

/// The widget writes here during layout; the app drains `measures` in `update`.
pub type MeasureSink = Rc<RefCell<SinkData>>;

/// How the rows changed in `Doc::set_rows`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Same,
    Appended(usize),
    /// `count` rows were added above the old first row.
    Prepended(usize),
    Replaced,
}

/// Rows plus everything the virtual list needs. Held in app state.
pub struct Doc {
    pub rows: Vec<Arc<Row>>,
    pub index: HashMap<RowId, usize>,
    pub window: VirtWindow,
    /// Column width the estimates were made for.
    pub colw: f32,
    measured: HashMap<RowId, f32>,
}

impl Default for Doc {
    fn default() -> Self {
        Doc {
            rows: Vec::new(),
            index: HashMap::new(),
            window: VirtWindow::new(),
            colw: 760.0,
            measured: HashMap::new(),
        }
    }
}

impl Doc {
    fn reindex(&mut self) {
        self.index = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id, i))
            .collect();
    }

    fn rebuild_window(&mut self) {
        let mut w = VirtWindow::new();
        for (i, r) in self.rows.iter().enumerate() {
            w.push(estimate(r, i, self.colw));
        }
        for (i, r) in self.rows.iter().enumerate() {
            if let Some(m) = self.measured.get(&r.id) {
                let cur = w.block_height(i);
                w.apply_height_delta(i, m - cur);
            }
        }
        self.window = w;
    }

    /// Replaces the rows. Appends keep the window and its measurements; a prepend or a replacement rebuilds the
    /// window from estimates plus the heights measured so far.
    pub fn set_rows(&mut self, rows: Vec<Arc<Row>>) -> Change {
        let old = self.rows.len();
        let same_ids = rows.len() == old && rows.iter().zip(&self.rows).all(|(a, b)| a.id == b.id);
        let append = rows.len() > old && rows.iter().zip(&self.rows).all(|(a, b)| a.id == b.id);
        if same_ids {
            let changed = rows.iter().zip(&self.rows).any(|(a, b)| a.sig != b.sig);
            self.rows = rows;
            return if changed {
                Change::Replaced
            } else {
                Change::Same
            };
        }
        if append {
            for (i, r) in rows.iter().enumerate().skip(old) {
                self.window.push(estimate(r, i, self.colw));
            }
            self.rows = rows;
            self.reindex();
            return Change::Appended(self.rows.len() - old);
        }
        // Prepend: the old first row sits at some index k and the old list is the tail.
        let prepended =
            old > 0 && rows.first().map(|f| f.id) != self.rows.first().map(|f| f.id) && {
                let k = rows.iter().position(|r| r.id == self.rows[0].id);
                k.is_some_and(|k| {
                    k > 0
                        && rows.len() - k >= old
                        && rows[k..k + old]
                            .iter()
                            .zip(&self.rows)
                            .all(|(a, b)| a.id == b.id)
                })
            };
        let count = if prepended {
            rows.iter().position(|r| r.id == self.rows[0].id).unwrap()
        } else {
            0
        };
        self.rows = rows;
        self.reindex();
        self.measured.retain(|id, _| self.index.contains_key(id));
        self.rebuild_window();
        if prepended {
            Change::Prepended(count)
        } else {
            Change::Replaced
        }
    }

    /// Applies one measurement. Returns `(row index, delta)` when the slot height changed.
    pub fn apply_measure(&mut self, m: Measure) -> Option<(usize, f32)> {
        if (m.colw - self.colw).abs() > 0.5 {
            self.colw = m.colw;
            self.measured.clear();
            self.rebuild_window();
        }
        let idx = *self.index.get(&m.id)?;
        let cur = self.window.block_height(idx);
        let delta = m.h - cur;
        if delta.abs() <= 0.5 {
            return None;
        }
        self.window.apply_height_delta(idx, delta);
        self.measured.insert(m.id, m.h);
        Some((idx, delta))
    }

    /// Total scroll height including the top and bottom padding.
    pub fn total_height(&self) -> f32 {
        PAD_TOP + self.window.total_height() + PAD_BOTTOM
    }

    /// Top of the slot of row `index` in document coordinates.
    pub fn slot_top(&self, index: usize) -> f32 {
        PAD_TOP + self.window.block_top(index)
    }
}

/// The unit a gesture selects by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Char,
    Word,
    Para,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    lo: Anchor,
    hi: Anchor,
    unit: Unit,
}

/// Selection state machine: clicks, drags, select all. Pure: the app feeds it widget events and a clock.
#[derive(Debug, Clone, Default)]
pub struct SelState {
    pub sel: Option<FlowSel>,
    drag: Option<Drag>,
    /// True after a click in the conversation; gates keyboard shortcuts so they do not steal the composer's.
    pub focused: bool,
    last_click: Option<(std::time::Instant, Anchor, iced::Point, u8)>,
    clock: Clock,
}

/// A second press within this time (and `MULTI_CLICK_PX`) counts as a double click.
pub const MULTI_CLICK: std::time::Duration = std::time::Duration::from_millis(500);
/// Largest pointer move between two presses of one multi-click, in px.
pub const MULTI_CLICK_PX: f32 = 4.0;

/// Where `SelState` reads the time. The system clock by default; tests inject their own.
#[derive(Clone)]
pub struct Clock(Rc<dyn Fn() -> std::time::Instant>);

impl Clock {
    pub fn new(f: impl Fn() -> std::time::Instant + 'static) -> Clock {
        Clock(Rc::new(f))
    }
    pub fn now(&self) -> std::time::Instant {
        (self.0)()
    }
}

impl Default for Clock {
    fn default() -> Clock {
        Clock::new(std::time::Instant::now)
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Clock")
    }
}

impl SelState {
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn clear(&mut self) {
        self.sel = None;
        self.drag = None;
    }

    pub fn blur(&mut self) {
        self.clear();
        self.focused = false;
        self.last_click = None;
    }

    /// Range of `unit` around `at`.
    fn unit_range(doc: &Doc, at: Anchor, unit: Unit) -> (Anchor, Anchor) {
        let Some(&i) = doc.index.get(&at.id) else {
            return (at, at);
        };
        let row = &doc.rows[i];
        let off = clamp_boundary(&row.plain, at.offset);
        let (s, e) = match unit {
            Unit::Char => (off, off),
            Unit::Word => words::word_range(&row.plain, off),
            Unit::Para => match row.kind {
                RowKind::Code { .. } => words::line_range(&row.plain, off),
                RowKind::Table { .. } => words::cell_range(&row.plain, off),
                _ => (0, row.plain.len()),
            },
        };
        (
            Anchor {
                id: at.id,
                offset: s,
            },
            Anchor {
                id: at.id,
                offset: e,
            },
        )
    }

    /// A state that reads the time from `clock` (tests).
    pub fn with_clock(clock: Clock) -> SelState {
        SelState {
            clock,
            ..Default::default()
        }
    }

    /// The clock this state reads, to keep it when the state is replaced.
    pub fn clock(&self) -> Clock {
        self.clock.clone()
    }

    /// Mouse press at `at`, pointer at `point` (widget coordinates). Reads the injected clock.
    pub fn press(&mut self, doc: &Doc, at: Anchor, point: iced::Point, shift: bool) {
        let now = self.clock.now();
        self.press_at(doc, at, point, shift, now);
    }

    /// Mouse press at time `now`. A second press within `MULTI_CLICK` and `MULTI_CLICK_PX` of the first, on the same
    /// row, selects a word; a third the paragraph. Shift extends.
    pub fn press_at(
        &mut self,
        doc: &Doc,
        at: Anchor,
        point: iced::Point,
        shift: bool,
        now: std::time::Instant,
    ) {
        self.focused = true;
        let count = match self.last_click {
            Some((t, prev, p, n))
                if now.saturating_duration_since(t) <= MULTI_CLICK
                    && prev.id == at.id
                    && p.distance(point) <= MULTI_CLICK_PX =>
            {
                n % 3 + 1
            }
            _ => 1,
        };
        self.last_click = Some((now, at, point, count));
        if shift && count == 1 {
            if let Some(sel) = self.sel {
                self.sel = Some(FlowSel {
                    anchor: sel.anchor,
                    focus: at,
                });
                self.drag = Some(Drag {
                    lo: sel.anchor,
                    hi: sel.anchor,
                    unit: Unit::Char,
                });
                return;
            }
        }
        let unit = match count {
            2 => Unit::Word,
            3 => Unit::Para,
            _ => Unit::Char,
        };
        let (lo, hi) = Self::unit_range(doc, at, unit);
        self.drag = Some(Drag { lo, hi, unit });
        self.sel = if lo == hi {
            None
        } else {
            Some(FlowSel {
                anchor: lo,
                focus: hi,
            })
        };
    }

    /// The pointer moved to `at` with the button down.
    pub fn drag_to(&mut self, doc: &Doc, at: Anchor) {
        let Some(d) = self.drag else { return };
        let (Some(&li), Some(&ai)) = (doc.index.get(&at.id), doc.index.get(&d.lo.id)) else {
            return;
        };
        let _ = (li, ai);
        let before = |a: Anchor, b: Anchor| -> bool {
            let ia = doc.index.get(&a.id).copied().unwrap_or(0);
            let ib = doc.index.get(&b.id).copied().unwrap_or(0);
            (ia, a.offset) < (ib, b.offset)
        };
        let next = if d.unit == Unit::Char {
            FlowSel {
                anchor: d.lo,
                focus: at,
            }
        } else {
            let (s, e) = Self::unit_range(doc, at, d.unit);
            if before(s, d.lo) {
                FlowSel {
                    anchor: d.hi,
                    focus: s,
                }
            } else {
                FlowSel {
                    anchor: d.lo,
                    focus: e,
                }
            }
        };
        self.sel = if next.is_empty() { None } else { Some(next) };
    }

    pub fn release(&mut self) {
        self.drag = None;
    }

    pub fn select_all(&mut self, doc: &Doc) {
        let (Some(first), Some(last)) = (
            doc.rows.iter().find(|r| r.selectable()),
            doc.rows.iter().rev().find(|r| r.selectable()),
        ) else {
            return;
        };
        self.sel = Some(FlowSel {
            anchor: Anchor {
                id: first.id,
                offset: 0,
            },
            focus: Anchor {
                id: last.id,
                offset: last.plain.len(),
            },
        });
        self.focused = true;
    }

    /// Copy text of the current selection, built from the model.
    pub fn text(&self, doc: &Doc) -> Option<String> {
        let sel = self.sel.filter(|s| !s.is_empty())?;
        let t = sel.text(&doc.rows, &doc.index);
        (!t.is_empty()).then_some(t)
    }

    /// Drops a selection whose rows are gone.
    pub fn validate(&mut self, doc: &Doc) {
        if let Some(s) = self.sel {
            if s.resolve(&doc.index).is_none() {
                self.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const P0: iced::Point = iced::Point::ORIGIN;

    fn prose(id: u64, text: &str) -> Arc<Row> {
        Arc::new(Row::new(
            id,
            RowKind::Prose,
            vec![Para::text(text)],
            "\n\n",
            12.0,
        ))
    }

    fn doc_of(rows: Vec<Arc<Row>>) -> Doc {
        let mut d = Doc::default();
        d.set_rows(rows);
        d
    }

    #[test]
    fn double_click_selects_a_word_and_triple_the_paragraph() {
        let doc = doc_of(vec![
            prose(1, "alpha beta gamma"),
            prose(2, "second paragraph"),
        ]);
        let mut s = SelState::default();
        let t0 = Instant::now();
        let at = Anchor { id: 1, offset: 7 };
        s.press_at(&doc, at, P0, false, t0);
        s.release();
        assert!(s.sel.is_none());
        s.press_at(&doc, at, P0, false, t0 + Duration::from_millis(100));
        s.release();
        assert_eq!(s.text(&doc).as_deref(), Some("beta"));
        s.press_at(&doc, at, P0, false, t0 + Duration::from_millis(200));
        s.release();
        assert_eq!(s.text(&doc).as_deref(), Some("alpha beta gamma"));
        // A slow click starts over.
        s.press_at(&doc, at, P0, false, t0 + Duration::from_secs(5));
        assert!(s.sel.is_none());
    }

    fn clocked() -> (SelState, Rc<std::cell::Cell<Instant>>) {
        let now = Rc::new(std::cell::Cell::new(Instant::now()));
        let c = now.clone();
        (SelState::with_clock(Clock::new(move || c.get())), now)
    }

    fn click(
        s: &mut SelState,
        now: &Rc<std::cell::Cell<Instant>>,
        doc: &Doc,
        after_ms: u64,
        at: Anchor,
        p: iced::Point,
    ) {
        now.set(now.get() + Duration::from_millis(after_ms));
        s.press(doc, at, p, false);
        s.release();
    }

    #[test]
    fn injected_clock_decides_double_click_timing() {
        let doc = doc_of(vec![prose(1, "alpha beta gamma")]);
        let at = Anchor { id: 1, offset: 7 };
        // Fast second click, 2 px away: word.
        let (mut s, now) = clocked();
        click(&mut s, &now, &doc, 0, at, iced::Point::new(100.0, 50.0));
        click(&mut s, &now, &doc, 200, at, iced::Point::new(102.0, 50.0));
        assert_eq!(s.text(&doc).as_deref(), Some("beta"));
        // Slow second click: single click, no selection.
        let (mut s, now) = clocked();
        click(&mut s, &now, &doc, 0, at, iced::Point::new(100.0, 50.0));
        click(&mut s, &now, &doc, 900, at, iced::Point::new(100.0, 50.0));
        assert!(s.sel.is_none());
        // Fast but far away (40 px): single click.
        let (mut s, now) = clocked();
        click(&mut s, &now, &doc, 0, at, iced::Point::new(100.0, 50.0));
        click(&mut s, &now, &doc, 100, at, iced::Point::new(140.0, 50.0));
        assert!(s.sel.is_none());
        // Exactly at the interval still counts.
        let (mut s, now) = clocked();
        click(&mut s, &now, &doc, 0, at, iced::Point::ORIGIN);
        click(
            &mut s,
            &now,
            &doc,
            MULTI_CLICK.as_millis() as u64,
            at,
            iced::Point::ORIGIN,
        );
        assert_eq!(s.text(&doc).as_deref(), Some("beta"));
    }

    #[test]
    fn shift_click_extends_from_the_anchor() {
        let doc = doc_of(vec![
            prose(1, "alpha beta gamma"),
            prose(2, "second paragraph"),
        ]);
        let mut s = SelState::default();
        let t0 = Instant::now();
        s.press_at(&doc, Anchor { id: 1, offset: 6 }, P0, false, t0);
        s.drag_to(&doc, Anchor { id: 1, offset: 10 });
        s.release();
        assert_eq!(s.text(&doc).as_deref(), Some("beta"));
        s.press_at(
            &doc,
            Anchor { id: 2, offset: 6 },
            P0,
            true,
            t0 + Duration::from_secs(3),
        );
        s.release();
        assert_eq!(s.text(&doc).as_deref(), Some("beta gamma\n\nsecond"));
    }

    #[test]
    fn word_drag_extends_by_words_in_both_directions() {
        let doc = doc_of(vec![prose(1, "alpha beta gamma delta")]);
        let mut s = SelState::default();
        let t0 = Instant::now();
        let at = Anchor { id: 1, offset: 8 }; // inside "beta"
        s.press_at(&doc, at, P0, false, t0);
        s.press_at(&doc, at, P0, false, t0 + Duration::from_millis(50));
        s.drag_to(&doc, Anchor { id: 1, offset: 13 }); // inside "gamma"
        assert_eq!(s.text(&doc).as_deref(), Some("beta gamma"));
        s.drag_to(&doc, Anchor { id: 1, offset: 1 }); // back into "alpha"
        assert_eq!(s.text(&doc).as_deref(), Some("alpha beta"));
    }

    #[test]
    fn select_all_and_copy_skip_rows_without_text() {
        let chips = Arc::new(Row::new(9, RowKind::Chips(vec![]), vec![], "", 8.0));
        let doc = doc_of(vec![prose(1, "one"), chips, prose(2, "two")]);
        let mut s = SelState::default();
        s.select_all(&doc);
        assert_eq!(s.text(&doc).as_deref(), Some("one\n\ntwo"));
        s.clear();
        assert!(s.text(&doc).is_none());
    }

    #[test]
    fn selection_survives_a_prepend_because_it_keys_on_ids() {
        let mut doc = doc_of(vec![prose(2, "second"), prose(3, "third")]);
        let sel = FlowSel {
            anchor: Anchor { id: 2, offset: 0 },
            focus: Anchor { id: 3, offset: 5 },
        };
        assert_eq!(sel.text(&doc.rows, &doc.index), "second\n\nthird");
        let change = doc.set_rows(vec![
            prose(1, "first"),
            prose(2, "second"),
            prose(3, "third"),
        ]);
        assert_eq!(change, Change::Prepended(1));
        assert_eq!(sel.text(&doc.rows, &doc.index), "second\n\nthird");
    }

    #[test]
    fn append_keeps_measurements() {
        let mut doc = doc_of(vec![prose(1, "a"), prose(2, "b")]);
        let colw = doc.colw;
        assert!(doc
            .apply_measure(Measure {
                id: 1,
                h: 100.0,
                colw
            })
            .is_some());
        let h = doc.window.block_height(0);
        assert_eq!(
            doc.set_rows(vec![prose(1, "a"), prose(2, "b"), prose(3, "c")]),
            Change::Appended(1)
        );
        assert_eq!(doc.window.block_height(0), h);
        assert_eq!(doc.window.len(), 3);
    }

    #[test]
    fn table_plain_is_tsv() {
        let cells = ["a", "b", "c", "d"]
            .iter()
            .map(|s| Para::text(*s))
            .collect();
        let r = Row::new(1, RowKind::Table { cols: 2 }, cells, "\n\n", 12.0);
        assert_eq!(r.plain, "a\tb\nc\td");
    }
}

#[cfg(test)]
mod empty_tests {
    use super::*;

    #[test]
    fn empty_doc_never_panics_in_the_selection_state() {
        let doc = Doc::default();
        let mut s = SelState::default();
        s.press(
            &doc,
            Anchor { id: 1, offset: 0 },
            iced::Point::ORIGIN,
            false,
        );
        s.drag_to(&doc, Anchor { id: 1, offset: 5 });
        s.release();
        s.select_all(&doc);
        assert!(s.text(&doc).is_none());
        s.validate(&doc);
        assert!(doc.rows.is_empty());
    }
}
