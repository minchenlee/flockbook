//! `Flow`: the conversation widget. It lays out the mounted window of a virtual list of rows, draws them, hit-tests
//! them and reports pointer and key gestures as `FlowEvent`s. It holds no selection logic: the app owns the
//! selection (`SelState`) and passes it back in, so a drag survives rows being unmounted.
//!
//! The widget keeps its own scroll geometry inside one widget (no inner `scrollable`), so an app can drive the
//! offset, auto-scroll while dragging, and test everything without a window. Shaped paragraphs are cached per row
//! id, so a frame costs layout of the mounted rows only.
//!
//! Text shaping, hit testing and highlight geometry use the renderer's `Paragraph` (cosmic-text), as in
//! `SelectableDoc`. Adapted from rmdv (MIT): the keyed window idea (the mounted slice sits at the window's prefix
//! position; measured heights flow back through a sink).

use super::model::{Anchor, Doc, FlowSel, Measure, MeasureSink, Metrics, RowId, GAP, PAD_BOTTOM};
use super::shape::{shape, signature, Prim, Shaped};
use crate::selectable::model::{clamp_boundary, Pos};
use crate::theme::Tokens;
use iced::advanced::graphics::text::cosmic_text;
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::svg::{self, Renderer as _};
use iced::advanced::text::paragraph::Paragraph as _;
use iced::advanced::text::Renderer as _;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::widget::Widget;
use iced::advanced::{mouse, Clipboard, Shell};
use iced::keyboard::{self, key};
use iced::{window, Border, Color, Element, Event, Length, Point, Rectangle, Size, Theme, Vector};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowKey {
    Copy,
    SelectAll,
    Escape,
    PageUp,
    PageDown,
    Home,
    End,
    LineUp,
    LineDown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlowEvent {
    /// Left press on text. The app decides char, word or paragraph from its click count.
    Press {
        at: Anchor,
        point: Point,
        shift: bool,
    },
    /// Pointer moved (or a redraw happened) while the button is down. `edge` is the auto-scroll speed in px per
    /// tick when the pointer is beyond the top (negative) or bottom (positive) edge.
    Drag {
        at: Anchor,
        edge: Option<f32>,
    },
    Release,
    /// A press outside the widget: the selection and the keyboard focus go away.
    Blur,
    Link(String),
    Chip(String),
    LoadEarlier,
    Picture(RowId),
    PicturesNear(Vec<RowId>),
    /// Wheel, in px; positive scrolls down.
    Wheel(f32),
    /// Scrollbar drag: the new offset.
    ScrollTo(f32),
    Key(FlowKey),
    /// Measurements are waiting in the sink.
    Settle,
    /// The widget size changed.
    Viewport(Size),
}

/// A laid-out row, in document coordinates.
#[derive(Clone)]
pub struct Placed {
    pub idx: usize,
    /// Top of the content (slot lead applied).
    pub y: f32,
    pub h: f32,
    pub shaped: Arc<Shaped>,
}

/// Layout facts the widget publishes for tests.
#[derive(Default, Clone)]
pub struct Probe {
    pub view: Size,
    /// Where the widget sits in the window.
    pub origin: Point,
    pub col_x: f32,
    pub colw: f32,
    /// The scroll offset actually used.
    pub eff: f32,
    pub total: f32,
    pub placed: Vec<Placed>,
}

impl Probe {
    pub fn mounted(&self) -> Vec<usize> {
        self.placed.iter().map(|p| p.idx).collect()
    }

    /// Widget-local point at the vertical middle of the glyph at `offset` of row `idx`.
    pub fn point_of(&self, idx: usize, offset: usize) -> Option<Point> {
        let pl = self.placed.iter().find(|p| p.idx == idx)?;
        let part = pl
            .shaped
            .parts
            .iter()
            .rev()
            .find(|p| offset >= p.plain_start)
            .or(pl.shaped.parts.first())?;
        let local = offset.saturating_sub(part.plain_start).min(part.plain_len);
        let (line, i) = line_and_index(&part.line_starts, local);
        let cursor = cosmic_text::Cursor::new(line, i);
        part.para
            .buffer()
            .layout_runs()
            .filter(|r| r.line_i == line)
            .find_map(|run| {
                run.highlight(cursor, cursor).map(|(x, _)| {
                    Point::new(
                        self.col_x + part.pos.x + x,
                        pl.y - self.eff + part.pos.y + run.line_top + run.line_height / 2.0,
                    )
                })
            })
    }

    /// Widget-local center of the first chip of row `idx`.
    pub fn chip_center(&self, idx: usize) -> Option<Point> {
        let pl = self.placed.iter().find(|p| p.idx == idx)?;
        let c = pl.shaped.chips.first()?;
        Some(Point::new(
            self.col_x + c.rect.center_x(),
            pl.y - self.eff + c.rect.center_y(),
        ))
    }
}

fn line_and_index(line_starts: &[usize], offset: usize) -> (usize, usize) {
    let line = line_starts
        .partition_point(|&s| s <= offset)
        .saturating_sub(1);
    (line, offset - line_starts[line])
}

pub struct Flow<'a, M> {
    doc: &'a Doc,
    picture_epoch: u64,
    picture_scope: &'a str,
    pictures: Option<&'a HashMap<RowId, iced::widget::image::Handle>>,
    tokens: Tokens,
    col_max: f32,
    metrics: Metrics,
    offset: f32,
    pinned: bool,
    selection: Option<FlowSel>,
    focused: bool,
    dragging: bool,
    viewport: Size,
    sink: Option<MeasureSink>,
    probe: Option<Rc<RefCell<Probe>>>,
    on_event: Box<dyn Fn(FlowEvent) -> M + 'a>,
}

impl<'a, M> Flow<'a, M> {
    pub fn new(doc: &'a Doc, tokens: Tokens, on_event: impl Fn(FlowEvent) -> M + 'a) -> Self {
        Flow {
            doc,
            pictures: None,
            picture_scope: "",
            picture_epoch: 0,
            tokens,
            col_max: 760.0,
            metrics: Metrics::default(),
            offset: 0.0,
            pinned: true,
            selection: None,
            focused: false,
            dragging: false,
            viewport: Size::ZERO,
            sink: None,
            probe: None,
            on_event: Box::new(on_event),
        }
    }
    pub fn picture_epoch(mut self, epoch: u64) -> Self {
        self.picture_epoch = epoch;
        self
    }
    pub fn picture_scope(mut self, scope: &'a str) -> Self {
        self.picture_scope = scope;
        self
    }
    pub fn pictures(mut self, pictures: &'a HashMap<RowId, iced::widget::image::Handle>) -> Self {
        self.pictures = Some(pictures);
        self
    }
    pub fn col_max(mut self, w: f32) -> Self {
        self.col_max = w;
        self
    }
    /// Text sizes, line heights and side space of the reading column.
    pub fn metrics(mut self, m: Metrics) -> Self {
        self.metrics = m;
        self
    }
    pub fn offset(mut self, offset: f32, pinned: bool) -> Self {
        self.offset = offset;
        self.pinned = pinned;
        self
    }
    pub fn selection(mut self, s: Option<FlowSel>) -> Self {
        self.selection = s;
        self
    }
    pub fn focused(mut self, f: bool) -> Self {
        self.focused = f;
        self
    }
    pub fn dragging(mut self, d: bool) -> Self {
        self.dragging = d;
        self
    }
    pub fn viewport(mut self, v: Size) -> Self {
        self.viewport = v;
        self
    }
    pub fn measure_into(mut self, sink: MeasureSink) -> Self {
        self.sink = Some(sink);
        self
    }
    pub fn probe(mut self, p: Rc<RefCell<Probe>>) -> Self {
        self.probe = Some(p);
        self
    }
}

#[derive(Default)]
struct State {
    pictures_near: Vec<(RowId, u64)>,
    picture_scope: String,
    picture_epoch: u64,
    cache: HashMap<RowId, Arc<Shaped>>,
    placed: Vec<Placed>,
    view: Cell<Size>,
    col_x: Cell<f32>,
    colw: f32,
    eff: Cell<f32>,
    total: f32,
    reaches_end: bool,
    pressing: bool,
    press_point: Point,
    press_link: Option<String>,
    moved: bool,
    mods: keyboard::Modifiers,
    sb_drag: Option<f32>,
    last_drag: Option<(Anchor, Option<f32>)>,
}

impl State {
    fn max_off(&self) -> f32 {
        (self.total - self.view.get().height).max(0.0)
    }

    fn thumb(&self) -> Option<Rectangle> {
        if self.total <= self.view.get().height + 1.0 {
            return None;
        }
        let h = (self.view.get().height * self.view.get().height / self.total)
            .clamp(32.0, self.view.get().height);
        let y = self.eff.get() / self.max_off().max(1.0) * (self.view.get().height - h);
        Some(Rectangle::new(
            Point::new(self.view.get().width - 10.0, y),
            Size::new(6.0, h),
        ))
    }

    /// Row index and nearest part under a widget-local point, with the offset inside the row's plain text.
    fn hit(&self, rows: &Doc, p: Point) -> Option<(Anchor, Option<String>, bool)> {
        if self.placed.is_empty() {
            return None;
        }
        let x = p.x - self.col_x.get();
        let y = p.y + self.eff.get();
        let k = self.placed.partition_point(|pl| pl.y + pl.h <= y);
        let k = if k >= self.placed.len() {
            self.placed.len() - 1
        } else if y < self.placed[k].y && k > 0 {
            let prev = &self.placed[k - 1];
            if y - (prev.y + prev.h) < self.placed[k].y - y {
                k - 1
            } else {
                k
            }
        } else {
            k
        };
        let pl = &self.placed[k];
        let row = &rows.rows[pl.idx];
        if !row.selectable() {
            return Some((
                Anchor {
                    id: row.id,
                    offset: 0,
                },
                None,
                false,
            ));
        }
        let ry = y - pl.y;
        // The part nearest to the point.
        let dist = |part: &super::shape::Part| -> f32 {
            let h = part.para.min_bounds().height;
            let dy = if ry < part.pos.y {
                part.pos.y - ry
            } else if ry > part.pos.y + h {
                ry - part.pos.y - h
            } else {
                0.0
            };
            let dx = if x < part.pos.x {
                part.pos.x - x
            } else if x > part.pos.x + part.w {
                x - part.pos.x - part.w
            } else {
                0.0
            };
            dy * 4.0 + dx
        };
        let part = pl
            .shaped
            .parts
            .iter()
            .min_by(|a, b| dist(a).total_cmp(&dist(b)))?;
        let lx = x - part.pos.x;
        let h = part.para.min_bounds().height;
        let ly = (ry - part.pos.y).clamp(0.0, (h - 1.0).max(0.0));
        let local = part
            .para
            .buffer()
            .hit(lx.max(0.0), ly)
            .map(|c| part.line_starts.get(c.line).copied().unwrap_or(0) + c.index)
            .unwrap_or(part.plain_len);
        let offset = part.plain_start + local.min(part.plain_len);
        let inside = dist(part) == 0.0;
        let link = if inside {
            part.para
                .hit_span(Point::new(lx, ly))
                .and_then(|i| part.spans.get(i))
                .and_then(|s| s.link.clone())
        } else {
            None
        };
        Some((
            Anchor {
                id: row.id,
                offset: clamp_boundary(&row.plain, offset),
            },
            link,
            inside,
        ))
    }

    fn chip_at(&self, p: Point) -> Option<(&str, bool)> {
        let x = p.x - self.col_x.get();
        let y = p.y + self.eff.get();
        self.placed
            .iter()
            .filter(|pl| y >= pl.y && y <= pl.y + pl.h)
            .find_map(|pl| {
                pl.shaped
                    .chips
                    .iter()
                    .find(|c| c.rect.contains(Point::new(x, y - pl.y)))
                    .map(|c| (c.id.as_str(), c.expandable))
            })
    }

    fn load_earlier_at(&self, p: Point) -> bool {
        let x = p.x - self.col_x.get();
        let y = p.y + self.eff.get();
        self.placed.iter().any(|pl| {
            pl.shaped
                .load_earlier
                .is_some_and(|r| r.contains(Point::new(x, y - pl.y)))
        })
    }
}

impl<'a, M> Flow<'a, M> {
    /// Applies the widget's real size: the scroll offset actually used, the column position, and the sink and
    /// probe facts. `layout` can run several times with different limits (flex), so `update` and `draw` call this
    /// again with the final bounds.
    fn fit(&self, state: &State, view: Size, origin: Point) {
        let colw = state.colw;
        state.view.set(view);
        state.col_x.set(((view.width - colw) / 2.0).floor());
        let max_off = state.max_off();
        let eff = if self.pinned && state.reaches_end {
            max_off
        } else {
            self.offset.clamp(0.0, max_off)
        };
        state.eff.set(eff);
        if let Some(sink) = &self.sink {
            let mut s = sink.borrow_mut();
            s.eff = eff;
            s.total = state.total;
        }
        if let Some(p) = &self.probe {
            *p.borrow_mut() = Probe {
                view,
                origin,
                col_x: state.col_x.get(),
                colw,
                eff,
                total: state.total,
                placed: state.placed.clone(),
            };
        }
    }

    fn drag_event(&self, state: &State, p: Point) -> Option<FlowEvent> {
        let h = state.view.get().height;
        let above = p.y < 0.0;
        let below = p.y > h;
        // Past the true start or end of the document the focus goes to the first or last character.
        let at_top = state.eff.get() <= 0.5;
        let at_bottom = state.reaches_end && state.eff.get() >= state.max_off() - 0.5;
        let doc = self.doc;
        if above && at_top {
            let r = doc.rows.iter().find(|r| r.selectable())?;
            return Some(FlowEvent::Drag {
                at: Anchor {
                    id: r.id,
                    offset: 0,
                },
                edge: None,
            });
        }
        if below && at_bottom {
            let r = doc.rows.iter().rev().find(|r| r.selectable())?;
            return Some(FlowEvent::Drag {
                at: Anchor {
                    id: r.id,
                    offset: r.plain.len(),
                },
                edge: None,
            });
        }
        let clamped = Point::new(p.x, p.y.clamp(1.0, (h - 1.0).max(1.0)));
        let (at, _, _) = state.hit(doc, clamped)?;
        let edge = if above {
            Some(-(6.0 + (-p.y).min(120.0) * 0.35))
        } else if below {
            Some(6.0 + (p.y - h).min(120.0) * 0.35)
        } else {
            None
        };
        Some(FlowEvent::Drag { at, edge })
    }
}

impl<'a, M> Widget<M, Theme, iced::Renderer> for Flow<'a, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        let doc = self.doc;
        let view = limits.max();
        let view = Size::new(
            if view.width.is_finite() {
                view.width
            } else {
                800.0
            },
            if view.height.is_finite() {
                view.height
            } else {
                600.0
            },
        );
        let colw = (view.width - 2.0 * self.metrics.gutter).min(self.col_max).max(120.0);
        state.colw = colw;

        let len = doc.rows.len();
        let (r0, r1) = if doc.window.active {
            doc.window.range
        } else {
            (0, len)
        };
        let (r0, r1) = (r0.min(len), r1.min(len));
        state.placed.clear();
        let mut slot_top = doc.slot_top(r0);
        let mut measures: Vec<Measure> = Vec::new();
        for i in r0..r1 {
            let row = &doc.rows[i];
            let sig = signature(row, colw, &self.tokens, &self.metrics);
            let shaped = match state.cache.get(&row.id) {
                Some(s) if s.sig == sig => s.clone(),
                _ => {
                    let s = Arc::new(shape(row, colw, &self.tokens, &self.metrics));
                    state.cache.insert(row.id, s.clone());
                    s
                }
            };
            let lead = row.lead(i);
            let slot_h = lead + shaped.h;
            if (slot_h - doc.window.block_height(i)).abs() > 0.5 || (colw - doc.colw).abs() > 0.5 {
                measures.push(Measure {
                    id: row.id,
                    h: slot_h,
                    colw,
                });
            }
            state.placed.push(Placed {
                idx: i,
                y: slot_top + lead,
                h: shaped.h,
                shaped,
            });
            slot_top += slot_h + GAP;
        }
        if state.cache.len() > (r1 - r0) + 400 {
            let keep: std::collections::HashSet<RowId> =
                state.placed.iter().map(|p| doc.rows[p.idx].id).collect();
            state.cache.retain(|k, _| keep.contains(k));
        }
        if let Some(sink) = &self.sink {
            let mut s = sink.borrow_mut();
            for m in measures {
                if let Some(e) = s.measures.iter_mut().find(|e| e.id == m.id) {
                    *e = m;
                } else {
                    s.measures.push(m);
                }
            }
        }
        state.reaches_end = r1 == len;
        state.total = if state.reaches_end {
            (slot_top - GAP).max(doc.slot_top(0)) + PAD_BOTTOM
        } else {
            doc.total_height()
        };
        self.fit(state, view, Point::ORIGIN);
        layout::Node::new(view)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let bounds = layout.bounds();
        self.fit(state, bounds.size(), bounds.position());
        let local = |p: Point| Point::new(p.x - bounds.x, p.y - bounds.y);
        let publish = |shell: &mut Shell<'_, M>, e: FlowEvent| shell.publish((self.on_event)(e));

        match event {
            Event::Window(window::Event::RedrawRequested(_)) => {
                let near: Vec<_> = state
                    .placed
                    .iter()
                    .filter(|p| {
                        p.y + p.h >= state.eff.get() - state.view.get().height
                            && p.y <= state.eff.get() + 2.0 * state.view.get().height
                    })
                    .filter_map(|p| {
                        let row = &self.doc.rows[p.idx];
                        matches!(row.kind, super::model::RowKind::Picture { .. })
                            .then_some((row.id, row.sig))
                    })
                    .collect();
                if near != state.pictures_near
                    || state.picture_scope != self.picture_scope
                    || state.picture_epoch != self.picture_epoch
                {
                    state.pictures_near = near.clone();
                    state.picture_scope = self.picture_scope.to_string();
                    state.picture_epoch = self.picture_epoch;
                    publish(
                        shell,
                        FlowEvent::PicturesNear(near.into_iter().map(|(id, _)| id).collect()),
                    );
                }
                if self
                    .sink
                    .as_ref()
                    .is_some_and(|s| !s.borrow().measures.is_empty())
                {
                    publish(shell, FlowEvent::Settle);
                }
                if (state.view.get().width - self.viewport.width).abs() > 0.5
                    || (state.view.get().height - self.viewport.height).abs() > 0.5
                {
                    publish(shell, FlowEvent::Viewport(state.view.get()));
                }
                if state.pressing || self.dragging {
                    if let Some(p) = cursor.position() {
                        if let Some(FlowEvent::Drag { at, edge }) = self.drag_event(state, local(p))
                        {
                            if state.last_drag != Some((at, edge)) || edge.is_some() {
                                state.last_drag = Some((at, edge));
                                publish(shell, FlowEvent::Drag { at, edge });
                            }
                            if edge.is_some() {
                                shell.request_redraw();
                            }
                        }
                    }
                }
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                let Some(p) = cursor.position() else { return };
                let p = local(p);
                if let Some(grab) = state.sb_drag {
                    if let Some(th) = state.thumb() {
                        let track = (state.view.get().height - th.height).max(1.0);
                        let off = ((p.y - grab) / track).clamp(0.0, 1.0) * state.max_off();
                        publish(shell, FlowEvent::ScrollTo(off));
                    }
                    shell.capture_event();
                    return;
                }
                if state.pressing {
                    if p.distance(state.press_point) > 3.0 {
                        state.moved = true;
                    }
                    if let Some(e) = self.drag_event(state, p) {
                        if let FlowEvent::Drag { at, edge } = &e {
                            state.last_drag = Some((*at, *edge));
                            if edge.is_some() {
                                shell.request_redraw();
                            }
                        }
                        publish(shell, e);
                    }
                    shell.capture_event();
                } else if bounds.contains(Point::new(p.x + bounds.x, p.y + bounds.y)) {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(abs) = cursor.position_over(bounds) else {
                    if self.focused || self.selection.is_some() {
                        publish(shell, FlowEvent::Blur);
                    }
                    return;
                };
                let p = local(abs);
                if p.x >= state.view.get().width - 14.0 {
                    if let Some(th) = state.thumb() {
                        if th.y <= p.y && p.y <= th.y + th.height {
                            state.sb_drag = Some(p.y - th.y);
                        } else {
                            let track = (state.view.get().height - th.height).max(1.0);
                            let off =
                                ((p.y - th.height / 2.0) / track).clamp(0.0, 1.0) * state.max_off();
                            state.sb_drag = Some(th.height / 2.0);
                            publish(shell, FlowEvent::ScrollTo(off));
                        }
                        shell.capture_event();
                        return;
                    }
                }
                if let Some(row) = state
                    .placed
                    .iter()
                    .find(|pl| {
                        p.y + state.eff.get() >= pl.y && p.y + state.eff.get() <= pl.y + pl.h
                    })
                    .map(|pl| &self.doc.rows[pl.idx])
                    .filter(|r| matches!(r.kind, super::model::RowKind::Picture { .. }))
                {
                    if p.x >= state.col_x.get() && p.x <= state.col_x.get() + state.colw {
                        publish(shell, FlowEvent::Picture(row.id));
                        shell.capture_event();
                        return;
                    }
                }
                if let Some((id, _)) = state.chip_at(p) {
                    publish(shell, FlowEvent::Chip(id.to_string()));
                    shell.capture_event();
                    return;
                }
                if state.load_earlier_at(p) {
                    publish(shell, FlowEvent::LoadEarlier);
                    shell.capture_event();
                    return;
                }
                if let Some((at, link, _)) = state.hit(self.doc, p) {
                    state.pressing = true;
                    state.moved = false;
                    state.press_point = p;
                    state.press_link = link;
                    state.last_drag = None;
                    publish(
                        shell,
                        FlowEvent::Press {
                            at,
                            point: p,
                            shift: state.mods.shift(),
                        },
                    );
                    shell.capture_event();
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if state.sb_drag.take().is_some() {
                    shell.capture_event();
                    return;
                }
                if state.pressing || self.dragging {
                    state.pressing = false;
                    if !state.moved {
                        if let Some(l) = state.press_link.take() {
                            publish(shell, FlowEvent::Link(l));
                        }
                    }
                    state.press_link = None;
                    publish(shell, FlowEvent::Release);
                    shell.capture_event();
                }
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.position_over(bounds).is_some() =>
            {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => -y * 48.0,
                    mouse::ScrollDelta::Pixels { y, .. } => -y,
                };
                if dy != 0.0 {
                    publish(shell, FlowEvent::Wheel(dy));
                    shell.capture_event();
                }
            }
            Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => state.mods = *m,
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) if self.focused => {
                state.mods = *modifiers;
                let has_sel = self.selection.is_some_and(|s| !s.is_empty());
                let ev = match key.as_ref() {
                    keyboard::Key::Character(c)
                        if modifiers.command() && c.eq_ignore_ascii_case("c") && has_sel =>
                    {
                        Some(FlowKey::Copy)
                    }
                    keyboard::Key::Character(c)
                        if modifiers.command() && c.eq_ignore_ascii_case("a") =>
                    {
                        Some(FlowKey::SelectAll)
                    }
                    keyboard::Key::Named(key::Named::Escape) if has_sel => Some(FlowKey::Escape),
                    keyboard::Key::Named(key::Named::PageUp) => Some(FlowKey::PageUp),
                    keyboard::Key::Named(key::Named::PageDown) => Some(FlowKey::PageDown),
                    keyboard::Key::Named(key::Named::Home) => Some(FlowKey::Home),
                    keyboard::Key::Named(key::Named::End) => Some(FlowKey::End),
                    keyboard::Key::Named(key::Named::ArrowUp) if !modifiers.command() => {
                        Some(FlowKey::LineUp)
                    }
                    keyboard::Key::Named(key::Named::ArrowDown) if !modifiers.command() => {
                        Some(FlowKey::LineDown)
                    }
                    _ => None,
                };
                if let Some(k) = ev {
                    publish(shell, FlowEvent::Key(k));
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Theme,
        _defaults: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        self.fit(state, bounds.size(), bounds.position());
        let Some(clip) = bounds.intersection(viewport) else {
            return;
        };
        let t = &self.tokens;
        let sel_color = Color {
            a: if t.mode.is_dark() { 0.42 } else { 0.28 },
            ..t.accent
        };
        let selection = self
            .selection
            .filter(|s| !s.is_empty())
            .and_then(|s| s.resolve(&self.doc.index));
        let hover = cursor.position_in(bounds);

        renderer.with_layer(clip, |renderer| {
            for pl in &state.placed {
                let origin = Point::new(
                    bounds.x + state.col_x.get(),
                    bounds.y + pl.y - state.eff.get(),
                );
                let row_rect = Rectangle::new(origin, Size::new(state.colw, pl.h));
                if row_rect.y > bounds.y + bounds.height + 40.0
                    || row_rect.y + row_rect.height < bounds.y - 40.0
                {
                    continue;
                }
                let at = |p: Point| Point::new(origin.x + p.x, origin.y + p.y);
                let rect_at = |r: Rectangle| Rectangle::new(at(r.position()), r.size());
                for prim in &pl.shaped.prims {
                    match prim {
                        Prim::Quad {
                            rect,
                            fill,
                            border,
                            bw,
                            radius,
                        } => renderer.fill_quad(
                            renderer::Quad {
                                bounds: rect_at(*rect),
                                border: Border {
                                    color: *border,
                                    width: *bw,
                                    radius: *radius,
                                },
                                ..Default::default()
                            },
                            *fill,
                        ),
                        Prim::Text { para, pos, color } => {
                            renderer.fill_paragraph(para, at(*pos), *color, clip)
                        }
                        Prim::Icon { icon, rect, color } => renderer.draw_svg(
                            svg::Svg::new(crate::icons::handle(*icon)).color(*color),
                            rect_at(*rect),
                            clip,
                        ),
                    }
                }
                for c in &pl.shaped.chips {
                    let rect = rect_at(c.rect);
                    let hovered = c.expandable
                        && hover.is_some_and(|h| {
                            rect.contains(Point::new(h.x + bounds.x, h.y + bounds.y))
                        });
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: rect,
                            border: Border {
                                radius: (c.rect.height / 2.0).into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        if hovered { t.fill_3 } else { t.fill_2 },
                    );
                    renderer.draw_svg(
                        svg::Svg::new(crate::icons::handle(c.icon)).color(t.label_2),
                        rect_at(c.icon_rect),
                        clip,
                    );
                    renderer.fill_paragraph(&c.label, at(c.label_pos), c.label_color, clip);
                    if let Some((icon, r)) = &c.chevron {
                        renderer.draw_svg(
                            svg::Svg::new(crate::icons::handle(*icon)).color(t.label_3),
                            rect_at(*r),
                            clip,
                        );
                    }
                }
                let row = &self.doc.rows[pl.idx];
                if let super::model::RowKind::Picture { width, height, .. } = &row.kind {
                    if let Some(handle) = self.pictures.and_then(|p| p.get(&row.id)) {
                        use iced::advanced::image::Renderer as _;
                        let k = 1f32
                            .min(state.colw.min(480.) / (*width).max(1) as f32)
                            .min(640. / (*height).max(1) as f32);
                        renderer.draw_image(
                            iced::advanced::image::Image::new(handle.clone()),
                            Rectangle::new(
                                origin,
                                Size::new(
                                    (*width as f32 * k).round().max(1.),
                                    (*height as f32 * k).round().max(1.),
                                ),
                            ),
                            clip,
                        );
                    }
                }
                for part in &pl.shaped.parts {
                    let pos = at(part.pos);
                    if let Some((start, end)) = selection {
                        if pl.idx >= start.block && pl.idx <= end.block {
                            let from = if pl.idx == start.block {
                                start.offset
                            } else {
                                0
                            };
                            let to = if pl.idx == end.block {
                                end.offset
                            } else {
                                row.plain.len()
                            };
                            let (ps, pe) = (part.plain_start, part.plain_start + part.plain_len);
                            let (lo, hi) = (from.max(ps), to.min(pe));
                            let whole_after = pl.idx != end.block || to > pe;
                            if lo < hi || (lo == hi && lo == pe && whole_after && pe > ps && false)
                            {
                                for r in highlight(part, lo - ps, hi - ps) {
                                    renderer.fill_quad(
                                        renderer::Quad {
                                            bounds: Rectangle::new(
                                                Point::new(pos.x + r.x, pos.y + r.y),
                                                r.size(),
                                            ),
                                            ..Default::default()
                                        },
                                        sel_color,
                                    );
                                }
                            }
                        }
                    }
                    // Span backgrounds (inline code), underline of hovered links, strikethrough.
                    for (i, sp) in part.spans.iter().enumerate() {
                        if sp.highlight.is_none() && !sp.strikethrough && sp.link.is_none() {
                            continue;
                        }
                        for b in part.para.span_bounds(i) {
                            if let Some(h) = sp.highlight {
                                let r = Rectangle::new(
                                    Point::new(
                                        pos.x + b.x - sp.padding.left,
                                        pos.y + b.y - sp.padding.top,
                                    ),
                                    Size::new(b.width + sp.padding.x(), b.height + sp.padding.y()),
                                );
                                renderer.fill_quad(
                                    renderer::Quad {
                                        bounds: r,
                                        border: h.border,
                                        ..Default::default()
                                    },
                                    h.background,
                                );
                            }
                            let abs =
                                Rectangle::new(Point::new(pos.x + b.x, pos.y + b.y), b.size());
                            let over = hover.is_some_and(|h| {
                                abs.contains(Point::new(h.x + bounds.x, h.y + bounds.y))
                            });
                            if sp.strikethrough || (sp.link.is_some() && over) {
                                let y = if sp.strikethrough {
                                    abs.y + abs.height / 2.0
                                } else {
                                    abs.y + abs.height - 3.0
                                };
                                renderer.fill_quad(
                                    renderer::Quad {
                                        bounds: Rectangle::new(
                                            Point::new(abs.x, y),
                                            Size::new(abs.width, 1.0),
                                        ),
                                        ..Default::default()
                                    },
                                    sp.color.unwrap_or(part.color),
                                );
                            }
                        }
                    }
                    renderer.fill_paragraph(&part.para, pos, part.color, clip);
                }
            }
            if let Some(th) = state.thumb() {
                let active = state.sb_drag.is_some()
                    || hover.is_some_and(|h| h.x >= state.view.get().width - 14.0);
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle::new(
                            Point::new(bounds.x + th.x, bounds.y + th.y),
                            if active {
                                Size::new(6.0, th.height)
                            } else {
                                Size::new(5.0, th.height)
                            },
                        ),
                        border: Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    if active { t.label_2 } else { t.label_3 },
                );
            }
        });
        let _ = Vector::ZERO;
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<State>();
        self.fit(state, layout.bounds().size(), layout.bounds().position());
        let Some(p) = cursor.position_in(layout.bounds()) else {
            return mouse::Interaction::default();
        };
        if state.pressing {
            return mouse::Interaction::Text;
        }
        if p.x >= state.view.get().width - 14.0 && state.thumb().is_some() {
            return mouse::Interaction::default();
        }
        if state.placed.iter().any(|pl| {
            p.y + state.eff.get() >= pl.y
                && p.y + state.eff.get() <= pl.y + pl.h
                && matches!(
                    self.doc.rows[pl.idx].kind,
                    super::model::RowKind::Picture { .. }
                )
        }) {
            return mouse::Interaction::Pointer;
        }
        if state.chip_at(p).is_some_and(|(_, expandable)| expandable) || state.load_earlier_at(p) {
            return mouse::Interaction::Pointer;
        }
        match state.hit(self.doc, p) {
            Some((_, Some(_), _)) => mouse::Interaction::Pointer,
            Some((_, None, true)) => mouse::Interaction::Text,
            _ => mouse::Interaction::default(),
        }
    }
}

/// Highlight rectangles (part-relative) for `from..to` of one part, from the paragraph's layout runs.
fn highlight(part: &super::shape::Part, from: usize, to: usize) -> Vec<Rectangle> {
    let (from_line, from_idx) = line_and_index(&part.line_starts, from);
    let (to_line, to_idx) = line_and_index(&part.line_starts, to);
    let mut rects = Vec::new();
    for run in part.para.buffer().layout_runs() {
        if run.line_i < from_line || run.line_i > to_line {
            continue;
        }
        let run_from = if run.line_i == from_line { from_idx } else { 0 };
        let run_to = if run.line_i == to_line {
            to_idx
        } else {
            run.text.len()
        };
        let start = cosmic_text::Cursor::new(run.line_i, run_from);
        let end = cosmic_text::Cursor::new(run.line_i, run_to);
        let span = run
            .highlight(start, end)
            .or_else(|| (run.line_i < to_line && run.glyphs.is_empty()).then_some((0.0, 4.0)));
        if let Some((x, width)) = span {
            if width > 0.0 {
                rects.push(Rectangle {
                    x,
                    y: run.line_top,
                    width,
                    height: run.line_height,
                });
            }
        }
    }
    rects
}

impl<'a, M: 'a> From<Flow<'a, M>> for Element<'a, M> {
    fn from(f: Flow<'a, M>) -> Self {
        Element::new(f)
    }
}

/// Convenience: a `Pos` of a selection endpoint resolved against the document.
pub fn resolve(sel: &FlowSel, doc: &Doc) -> Option<(Pos, Pos)> {
    sel.resolve(&doc.index)
}
