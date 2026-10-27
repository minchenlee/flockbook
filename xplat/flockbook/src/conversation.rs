//! Conversation view: user bubbles, agent prose, tool chips, code, tables, footers.
//!
//! The list is one `fb_ui::selectable::flow::Flow` widget over a flat list of rows (`Doc`). The window math is
//! `fb_ui::virt::VirtWindow` (estimates, measured heights, full render below 256 rows, anchor to the bottom while
//! pinned). Selection (`SelState`) lives in app state keyed by stable row id plus byte offset, so it survives rows
//! being unmounted. Copy is built from the model. Drag past the top or bottom edge auto-scrolls on a timer
//! subscription and keeps extending the selection.
//!
//! Why no `KeyedBody`: the rows are not child widgets. `Flow` shapes and draws the mounted slice itself (cached per
//! row id), so there is no per-row widget tree to diff when the window slides.

mod build;
mod markdown;

use crate::{App, Message};
use fb_ui::icons::Icon;
use fb_ui::selectable::flow::{
    Doc, Flow, FlowEvent, FlowKey, MeasureSink, Probe, SelState, PAD_TOP,
};
use fb_ui::theme::*;
use iced::widget::{button, column, container, image, stack, Space};
use iced::{Alignment, Element, Length, Padding, Size, Subscription, Task};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

/// Widest the reading column gets, in points.
pub const COLUMN_MAX: f32 = 760.0;
/// Auto-scroll timer period while a drag is past an edge.
pub const AUTOSCROLL_MS: u64 = 16;

pub struct ConversationState {
    pub pictures: crate::pictures::Pictures,
    /// Agent/session plus actual resolved log path. Pane IDs alone are insufficient.
    pub session_key: String,
    pub doc: Doc,
    pub sel: SelState,
    /// Scroll offset in document coordinates (px of the viewport top). Ignored while `pinned`.
    pub offset: f32,
    /// Following the bottom: new rows stay in view.
    pub pinned: bool,
    /// Last known size of the list widget.
    pub viewport: Size,
    /// The pane the rows belong to. A change resets everything.
    pub pane: Option<String>,
    /// Chip ids that are expanded.
    pub expanded: HashSet<String>,
    cache: build::RowCache,
    /// Auto-scroll speed in px per tick while a drag is beyond an edge.
    pub edge: Option<f32>,
    pub sink: MeasureSink,
    pub more_earlier: bool,
    pub loading_earlier: bool,
    /// Number of "load earlier" reads started. A result with an older number is stale.
    pub earlier_seq: u64,
    /// How many times the rows were rebuilt (tests: an idle tick must not rebuild).
    pub rebuilds: u64,
    /// Last text copied (also written to the clipboard).
    pub last_copied: Option<String>,
    /// Last link the user clicked (handed to `App::opener`).
    pub last_link: Option<String>,
    /// Layout facts for tests.
    pub probe: Option<Rc<RefCell<Probe>>>,
}

impl Default for ConversationState {
    fn default() -> Self {
        ConversationState {
            pictures: Default::default(),
            session_key: String::new(),
            doc: Doc::default(),
            sel: SelState::default(),
            offset: 0.0,
            pinned: true,
            viewport: Size::new(1000.0, 640.0),
            pane: None,
            expanded: HashSet::new(),
            cache: Default::default(),
            edge: None,
            sink: Default::default(),
            more_earlier: false,
            loading_earlier: false,
            earlier_seq: 0,
            rebuilds: 0,
            last_copied: None,
            last_link: None,
            probe: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ConversationMsg {
    Flow(FlowEvent),
    ClosePicture,
    JumpLatest,
    PictureLoaded {
        scope: String,
        id: u64,
        reference: herdr_core::ImageRef,
        preview: bool,
        seq: u64,
        data: Option<crate::pictures::Decoded>,
    },
    /// Timer tick of the drag auto-scroll.
    AutoScroll,
    /// A "load earlier" read finished. `more`: older content remains (`None`: the worker died).
    EarlierLoaded {
        pane: String,
        ticket: u64,
        more: Option<bool>,
    },
}

/// Lucide icon for a herdr tool chip symbol (the symbols are SF Symbol names kept from the Swift model).
pub fn icon_for_symbol(symbol: &str) -> Icon {
    match symbol {
        "terminal" => Icon::SquareTerminal,
        "pencil" => Icon::Pencil,
        "doc.text" => Icon::FileText,
        "globe" => Icon::Globe,
        "magnifyingglass" => Icon::Search,
        "square.and.pencil" => Icon::SquarePen,
        "person.2" => Icon::Layers,
        "checklist" => Icon::Check,
        "wrench" => Icon::Hammer,
        _ => Icon::Sparkles,
    }
}

// ---------------------------------------------------------------------------------------------------- scrolling

fn max_offset(cs: &ConversationState) -> f32 {
    let total = {
        let s = cs.sink.borrow().total;
        if s > 0.0 {
            s
        } else {
            cs.doc.total_height()
        }
    };
    (total - cs.viewport.height).max(0.0)
}

/// The offset the widget really shows: while pinned that is the bottom, which only the widget knows exactly.
fn current_offset(cs: &ConversationState) -> f32 {
    if cs.pinned {
        let eff = cs.sink.borrow().eff;
        if eff > 0.0 {
            return eff;
        }
        return max_offset(cs);
    }
    cs.offset
}

/// Re-windows the mounted rows around the offset (or the end while pinned).
fn rewindow(cs: &mut ConversationState) {
    let off = if cs.pinned {
        f32::MAX
    } else {
        (cs.offset - PAD_TOP).max(0.0)
    };
    cs.doc.window.rebuild_at(off, cs.viewport.height);
}

fn set_offset(cs: &mut ConversationState, off: f32) {
    let max = max_offset(cs);
    cs.offset = off.clamp(0.0, max);
    cs.pinned = cs.offset >= max - 2.0;
    rewindow(cs);
}

fn scroll_by(cs: &mut ConversationState, dy: f32) {
    let base = current_offset(cs);
    set_offset(cs, base + dy);
}

/// Applies the heights the widget measured. Rows above the viewport that change size move the offset, so the
/// content under the reader's eyes stays put.
fn drain(cs: &mut ConversationState) {
    let measures = std::mem::take(&mut cs.sink.borrow_mut().measures);
    if measures.is_empty() {
        return;
    }
    let first_visible = cs
        .doc
        .window
        .index_at((cs.offset - PAD_TOP).max(0.0))
        .unwrap_or(0);
    for m in measures {
        if let Some((idx, delta)) = cs.doc.apply_measure(m) {
            if !cs.pinned && idx < first_visible {
                cs.offset += delta;
            }
        }
    }
    rewindow(cs);
}

// ---------------------------------------------------------------------------------------------------- update

pub fn update(app: &mut App, msg: ConversationMsg) -> Task<Message> {
    match msg {
        ConversationMsg::JumpLatest => {
            follow_latest(app);
            Task::none()
        }
        ConversationMsg::ClosePicture => {
            let p = &mut app.state.conversation.pictures;
            p.preview = None;
            p.preview_id = None;
            p.preview_seq = p.preview_seq.wrapping_add(1);
            Task::none()
        }
        ConversationMsg::PictureLoaded {
            scope,
            id,
            reference,
            preview,
            seq,
            data,
        } => {
            let current = app.state.selected_pane().map(|p| {
                format!(
                    "{:?}|{:?}",
                    herdr_core::AgentIdentity::for_pane(p),
                    app.source.log_path(p)
                )
            });
            if current.as_deref() != Some(scope.as_str()) {
                return Task::none();
            }
            let cs = &mut app.state.conversation;
            if cs.session_key != scope || cs.pictures.refs.get(&id) != Some(&reference) {
                return Task::none();
            }
            if preview {
                if cs.pictures.preview_id == Some(id) && cs.pictures.preview_seq == seq {
                    cs.pictures.preview = data.and_then(|d| d.budgeted_handle(true));
                    if cs.pictures.preview.is_none() {
                        cs.pictures.preview_id = None;
                    }
                }
            } else {
                if seq != cs.pictures.load_seq {
                    return Task::none();
                }
                cs.pictures.pending.remove(&id);
                if let Some(d) = data {
                    cs.pictures.insert(id, d)
                } else {
                    cs.pictures.failed.insert(id);
                }
            }
            picture_jobs(app)
        }
        ConversationMsg::AutoScroll => {
            let cs = &mut app.state.conversation;
            drain(cs);
            if let Some(v) = cs.edge {
                scroll_by(cs, v);
            }
            Task::none()
        }
        ConversationMsg::Flow(ev) => flow_event(app, ev),
        ConversationMsg::EarlierLoaded { pane, ticket, more } => {
            earlier_loaded(app, pane, ticket, more);
            Task::none()
        }
    }
}

fn flow_event(app: &mut App, ev: FlowEvent) -> Task<Message> {
    let mut task = Task::none();
    let (mut scrolled, mut resync, mut load) = (false, false, false);
    let mut link: Option<String> = None;
    let mut picture = None;
    let mut pictures_changed = false;
    let mut jump = false;
    {
        let cs = &mut app.state.conversation;
        match ev {
            FlowEvent::Press { at, point, shift } => cs.sel.press(&cs.doc, at, point, shift),
            FlowEvent::Drag { at, edge } => {
                cs.sel.drag_to(&cs.doc, at);
                cs.edge = edge;
            }
            FlowEvent::Release => {
                cs.sel.release();
                cs.edge = None;
            }
            FlowEvent::Blur => {
                cs.sel.blur();
                cs.edge = None;
            }
            FlowEvent::Picture(id) => picture = Some(id),
            FlowEvent::PicturesNear(ids) => {
                cs.pictures.retain_near(ids);
                pictures_changed = true;
            }
            FlowEvent::Link(url) => link = Some(url),
            FlowEvent::Chip(id) => {
                if !cs.expanded.remove(&id) {
                    cs.expanded.insert(id);
                }
                resync = true;
            }
            FlowEvent::LoadEarlier => load = true,
            FlowEvent::Wheel(dy) => {
                drain(cs);
                scroll_by(cs, dy);
                scrolled = true;
            }
            FlowEvent::ScrollTo(off) => {
                set_offset(cs, off);
                scrolled = true;
            }
            FlowEvent::Key(k) => match k {
                FlowKey::Copy => {
                    if let Some(text) = cs.sel.text(&cs.doc) {
                        cs.last_copied = Some(text.clone());
                        task = iced::clipboard::write(text);
                    }
                }
                FlowKey::SelectAll => cs.sel.select_all(&cs.doc),
                FlowKey::Escape => {
                    cs.sel.clear();
                    cs.pictures.preview = None;
                    cs.pictures.preview_id = None;
                    cs.pictures.preview_seq = cs.pictures.preview_seq.wrapping_add(1);
                }
                FlowKey::PageUp => {
                    scroll_by(cs, -cs.viewport.height * 0.9);
                    scrolled = true;
                }
                FlowKey::PageDown => {
                    scroll_by(cs, cs.viewport.height * 0.9);
                    scrolled = true;
                }
                FlowKey::LineUp => {
                    scroll_by(cs, -48.0);
                    scrolled = true;
                }
                FlowKey::LineDown => {
                    scroll_by(cs, 48.0);
                    scrolled = true;
                }
                FlowKey::Home => {
                    set_offset(cs, 0.0);
                    scrolled = true;
                }
                FlowKey::End => {
                    jump = true;
                    let max = max_offset(cs);
                    set_offset(cs, max);
                    scrolled = true;
                }
            },
            FlowEvent::Settle => drain(cs),
            FlowEvent::Viewport(size) => {
                cs.viewport = size;
                rewindow(cs);
            }
        }
        // Scrolled to the top of what is loaded: read an older part of the log.
        if scrolled
            && cs.offset <= 40.0
            && cs.more_earlier
            && !cs.loading_earlier
            && !cs.doc.rows.is_empty()
        {
            load = true;
        }
    }
    if jump {
        follow_latest(app);
    }
    if let Some(id) = picture {
        task = Task::batch([task, open_picture(app, id)]);
    }
    if pictures_changed {
        task = Task::batch([task, picture_jobs(app)]);
    }
    if let Some(url) = link {
        open_link(app, &url);
    }
    if resync {
        sync_markdown(app);
    }
    if load {
        task = Task::batch([task, load_earlier(app)]);
    }
    let cs = &mut app.state.conversation;
    cs.sel.validate(&cs.doc);
    task
}

fn open_link(app: &mut App, url: &str) {
    app.state.conversation.last_link = Some(url.to_string());
    if crate::links::allowed(url) {
        app.opener.open(url);
    }
}

/// Reads an older part of the pane's log. A source with a job runs it on a blocking thread; the result comes back
/// as `EarlierLoaded`, which is dropped when the pane changed or a newer load started in the meantime. A source
/// without a job reads on the UI thread. The reader's place stays: rows added above move the offset by their height.
fn load_earlier(app: &mut App) -> Task<Message> {
    let Some(pane) = app.state.selected_pane().cloned() else {
        return Task::none();
    };
    if app.state.conversation.loading_earlier {
        return Task::none();
    }
    let cs = &mut app.state.conversation;
    cs.loading_earlier = true;
    cs.earlier_seq += 1;
    let ticket = cs.earlier_seq;
    match app.source.load_earlier_job(&pane) {
        Some(job) => {
            sync_markdown(app); // shows "Loading earlier..."
            let pane_id = pane.id.clone();
            Task::perform(
                async move {
                    let (tx, rx) = iced::futures::channel::oneshot::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(job());
                    });
                    rx.await.ok()
                },
                move |more| Message::PaneConversation {
                    pane: pane_id.clone(),
                    message: ConversationMsg::EarlierLoaded {
                        pane: pane_id.clone(),
                        ticket,
                        more,
                    },
                },
            )
        }
        None => {
            let more = app.source.load_earlier(&pane);
            earlier_loaded(app, pane.id, ticket, Some(more));
            Task::none()
        }
    }
}

fn earlier_loaded(app: &mut App, pane: String, ticket: u64, more: Option<bool>) {
    let current = app.state.selected_pane().map(|p| {
        format!(
            "{:?}|{:?}",
            herdr_core::AgentIdentity::for_pane(p),
            app.source.log_path(p)
        )
    });
    if current.as_deref() != Some(app.state.conversation.session_key.as_str()) {
        return;
    }
    let cs = &mut app.state.conversation;
    if cs.pane.as_deref() != Some(pane.as_str()) || cs.earlier_seq != ticket {
        return; // stale: another pane is shown, or a newer load owns the flags
    }
    cs.loading_earlier = false;
    if let Some(m) = more {
        cs.more_earlier = m;
    }
    app.state.invalidate(&pane);
    app.refresh();
    sync_markdown(app);
}

// ---------------------------------------------------------------------------------------------------- sync

/// Rebuilds the rows of the selected pane from `state.conversations`. Call after `App::refresh`. Items that did not
/// change keep their rows (cached per item), so a poll tick costs hashing, not parsing.
pub fn sync_markdown(app: &mut App) {
    let Some(pane) = app.state.selected_pane().cloned() else {
        return;
    };
    let can_more = app.source.can_load_earlier(&pane);
    let log_path = app.source.log_path(&pane);
    let session_key = format!(
        "{:?}|{:?}",
        herdr_core::AgentIdentity::for_pane(&pane),
        log_path
    );
    let st = &mut app.state;
    let cs = &mut st.conversation;
    if cs.pane.as_deref() != Some(pane.id.as_str()) || cs.session_key != session_key {
        *cs = ConversationState {
            probe: cs.probe.take(),
            viewport: cs.viewport,
            sink: cs.sink.clone(),
            pane: Some(pane.id.clone()),
            more_earlier: can_more,
            earlier_seq: cs.earlier_seq + 1,
            rebuilds: cs.rebuilds,
            sel: SelState::with_clock(cs.sel.clock()),
            ..Default::default()
        };
        *cs.sink.borrow_mut() = Default::default();
        cs.session_key = session_key;
        cs.pictures.path = log_path;
    }
    let items: &[herdr_core::ConversationItem] = st
        .conversations
        .get(&pane.id)
        .map(|v| v.as_slice())
        .unwrap_or(&[]);
    let previous_refs = std::mem::take(&mut cs.pictures.refs);
    cs.pictures.refs = items
        .iter()
        .flat_map(|it| match &it.kind {
            herdr_core::ItemKind::Images(rs) => rs
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    (
                        fb_ui::selectable::flow::row_id(&[&it.id, &i.to_string()]),
                        r.clone(),
                    )
                })
                .collect::<Vec<_>>(),
            _ => vec![],
        })
        .collect();
    cs.pictures
        .handles
        .retain(|id, _| cs.pictures.refs.get(id) == previous_refs.get(id));
    cs.pictures
        .pending
        .retain(|id| cs.pictures.refs.get(id) == previous_refs.get(id));
    cs.pictures
        .failed
        .retain(|id| cs.pictures.refs.get(id) == previous_refs.get(id));
    cs.pictures.prune_costs();
    if cs
        .pictures
        .preview_id
        .is_some_and(|id| !cs.pictures.refs.contains_key(&id))
    {
        cs.pictures.preview = None;
        cs.pictures.preview_id = None;
    }
    // `more_earlier` belongs to the load result, as in Swift (`Conversation.swift`: `moreEarlier = more`). A poll
    // tick must not overwrite it with a fresh guess: the source's `can_load_earlier` is the same log the worker
    // just read, but a source that answers from its own cache would flip the button back on after a load said
    // there is nothing older. Only a reader that changes session re-reads the log (below).
    cs.rebuilds += 1;
    let rows = build::build_rows(
        items,
        &cs.expanded,
        &mut cs.cache,
        cs.more_earlier,
        cs.loading_earlier,
    );
    replace_rows(cs, rows);
    cs.sel.validate(&cs.doc);
    rewindow(cs);
}

fn replace_rows(
    cs: &mut ConversationState,
    rows: Vec<std::sync::Arc<fb_ui::selectable::flow::Row>>,
) {
    // Hold a content row by identity even when the load button remains first, chips merge,
    // or a new reply is appended in the same refresh as a prepend.
    drain(cs);
    let anchor = if cs.pinned {
        None
    } else {
        let i = cs
            .doc
            .window
            .index_at((cs.offset - PAD_TOP).max(0.))
            .unwrap_or(0);
        let i =
            if cs.doc.rows.get(i).is_some_and(|r| {
                matches!(r.kind, fb_ui::selectable::flow::RowKind::LoadEarlier { .. })
            }) {
                i + 1
            } else {
                i
            };
        cs.doc
            .rows
            .get(i)
            .map(|r| (r.id, cs.offset - cs.doc.window.prefix_at(i)))
    };
    cs.doc.set_rows(rows);
    if let Some((id, relative)) = anchor {
        if let Some(i) = cs.doc.rows.iter().position(|r| r.id == id) {
            cs.offset = (cs.doc.window.prefix_at(i) + relative).max(0.);
        }
    }
}

pub fn subscription(app: &App) -> Subscription<Message> {
    if app.state.conversation.edge.is_some() {
        iced::time::every(Duration::from_millis(AUTOSCROLL_MS))
            .map(|_| Message::Conversation(ConversationMsg::AutoScroll))
    } else {
        Subscription::none()
    }
}

// Picture workers carry both the session scope and the content reference back to the reader.
fn picture_task(app: &App, id: u64, preview: bool, seq: u64) -> Task<Message> {
    let cs = &app.state.conversation;
    let (Some(path), Some(reference), Some(pane)) = (
        cs.pictures.path.clone(),
        cs.pictures.refs.get(&id).cloned(),
        cs.pane.clone(),
    ) else {
        return Task::none();
    };
    let scope = cs.session_key.clone();
    let work_ref = reference.clone();
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(crate::pictures::load(
                    &path,
                    &work_ref,
                    if preview { 0 } else { 960 },
                ));
            });
            rx.await.ok().flatten()
        },
        move |data| Message::PaneConversation {
            pane: pane.clone(),
            message: ConversationMsg::PictureLoaded {
                scope: scope.clone(),
                id,
                reference: reference.clone(),
                preview,
                seq,
                data,
            },
        },
    )
}

fn picture_jobs(app: &mut App) -> Task<Message> {
    let cs = &mut app.state.conversation;
    if cs.pictures.path.is_none() {
        return Task::none();
    }
    let slots = 2usize.saturating_sub(cs.pictures.pending.len());
    let ids: Vec<_> = cs
        .pictures
        .near
        .iter()
        .copied()
        .filter(|id| {
            cs.pictures.refs.contains_key(id)
                && !cs.pictures.handles.contains_key(id)
                && !cs.pictures.pending.contains(id)
                && !cs.pictures.failed.contains(id)
        })
        .take(slots)
        .collect();
    cs.pictures.pending.extend(ids.iter().copied());
    let seq = cs.pictures.load_seq;
    Task::batch(ids.into_iter().map(|id| picture_task(app, id, false, seq)))
}

fn open_picture(app: &mut App, id: u64) -> Task<Message> {
    let p = &mut app.state.conversation.pictures;
    if !p.refs.contains_key(&id) {
        return Task::none();
    }
    p.preview = None;
    p.preview_id = Some(id);
    p.preview_seq = p.preview_seq.wrapping_add(1);
    let seq = p.preview_seq;
    picture_task(app, id, true, seq)
}

/// Sending or explicitly jumping to latest invalidates a pending history completion's ownership.
pub fn follow_latest(app: &mut App) {
    let cs = &mut app.state.conversation;
    cs.earlier_seq = cs.earlier_seq.wrapping_add(1);
    cs.loading_earlier = false;
    cs.pinned = true;
    cs.edge = None;
    rewindow(cs);
}

// ---------------------------------------------------------------------------------------------------- view

pub fn view(app: &App) -> Element<'_, Message> {
    let t = app.tokens();
    let cs = &app.state.conversation;
    if app.state.conversation_items().is_empty() {
        let state = app
            .state
            .selected_pane()
            .map(|p| app.source.conversation_state(p))
            .unwrap_or(herdr_core::ConversationState::Missing);
        let (title, detail) = match state {
            herdr_core::ConversationState::Loading => (
                "Loading conversation",
                "Reading the session log…".to_string(),
            ),
            herdr_core::ConversationState::Missing => (
                "Session log unavailable",
                "Waiting for the agent to write its session log.".to_string(),
            ),
            herdr_core::ConversationState::Failed(reason) => ("Cannot read conversation", reason),
            herdr_core::ConversationState::WaitingForLead => (
                "Waiting for lead",
                "This teammate has not been connected to a lead conversation yet.".to_string(),
            ),
            herdr_core::ConversationState::ShowingLead(note) => (
                "Lead conversation",
                format!(
                    "Showing {} for {}.",
                    note.lead.as_deref().unwrap_or("the lead"),
                    note.teammate
                ),
            ),
            herdr_core::ConversationState::Ready => (
                "No messages yet",
                "The agent has not written anything yet.".to_string(),
            ),
        };
        return container(
            column![
                TITLE_3.text(title).color(t.label_2),
                Space::new().height(SP_4),
                BODY.text(detail).color(t.label_3)
            ]
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .into();
    }
    let mut flow = Flow::new(&cs.doc, t, |e| {
        Message::Conversation(ConversationMsg::Flow(e))
    })
    .picture_scope(&cs.session_key)
    .pictures(&cs.pictures.handles)
    .col_max(app.state.settings.column_width())
    .metrics(app.state.settings.flow_metrics())
    .offset(cs.offset, cs.pinned)
    .selection(cs.sel.sel)
    .focused(cs.sel.focused)
    .dragging(cs.sel.dragging())
    .viewport(cs.viewport)
    .measure_into(cs.sink.clone());
    if let Some(p) = &cs.probe {
        flow = flow.probe(p.clone());
    }
    // Soft edges at the top row and over the status line: a line an edge cuts fades out instead of ending half drawn.
    let body = crate::style::with_top_fade(app.ground(), crate::style::with_bottom_fade(app.ground(), Element::from(flow)));
    let body = if !cs.pinned {
        // The count of rows below the window and the down arrow, as the Swift button's label (Conversation.swift:246).
        let newer = cs.doc.rows.len().saturating_sub(cs.doc.window.range.1);
        let label = if newer > 0 {
            format!("\u{2193} Jump to latest ({newer} newer)")
        } else {
            "\u{2193} Jump to latest".to_string()
        };
        // Swift `.buttonStyle(.bordered).controlSize(.small)`, centred under the rows (Conversation.swift:244). Its own
        // strip under the conversation, over the status line: floating over the rows it sat on their text.
        let latest = container(
            button(CAPTION.text(label).color(t.label))
                .padding([3.0, 10.0])
                .style(move |_, status| {
                    let down = matches!(status, button::Status::Pressed | button::Status::Hovered);
                    let bg = match (t.mode.is_dark(), down) {
                        (true, false) => iced::Color::from_rgb8(48, 48, 52),
                        (true, true) => iced::Color::from_rgb8(60, 60, 64),
                        (false, false) => iced::Color::WHITE,
                        (false, true) => iced::Color::from_rgb8(242, 242, 242),
                    };
                    button::Style {
                        background: Some(iced::Background::Color(bg)),
                        text_color: t.label,
                        border: iced::Border { color: t.sep, width: 1.0, radius: R_ROW.into() },
                        shadow: shadow_e1(&t),
                        ..Default::default()
                    }
                })
                .on_press(Message::Conversation(ConversationMsg::JumpLatest)),
        )
        .padding(Padding { top: SP_8, bottom: 2.0, left: 0.0, right: 0.0 })
        .width(Length::Fill)
        .align_x(Alignment::Center);
        Element::from(column![body, latest])
    } else {
        body
    };
    let body = if let Some(handle) = &cs.pictures.preview {
        let panel = container(
            column![
                button(BODY.text("Close image"))
                    .on_press(Message::Conversation(ConversationMsg::ClosePicture)),
                image(handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Contain)
            ]
            .spacing(12),
        )
        .padding(24)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(crate::style::card_e1(t, 12.));
        Element::from(stack![body, panel])
    } else {
        body
    };
    if let Some(pane) = app.state.selected_pane() {
        if let herdr_core::ConversationState::ShowingLead(note) =
            app.source.conversation_state(pane)
        {
            return column![
                CAPTION
                    .text(format!(
                        "Showing {} for {}",
                        note.lead.as_deref().unwrap_or("lead"),
                        note.teammate
                    ))
                    .color(t.label_2),
                body
            ]
            .into();
        }
    }
    body
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use fb_ui::selectable::flow::{Para, Row, RowKind};
    use std::sync::Arc;
    fn rows(start: u64, end: u64) -> Vec<Arc<Row>> {
        let mut out = vec![Arc::new(Row::new(
            99,
            RowKind::LoadEarlier { loading: false },
            vec![],
            "",
            0.,
        ))];
        out.extend((start..end).map(|id| {
            Arc::new(Row::new(
                id,
                RowKind::Prose,
                vec![Para::text(format!("Synthetic {id}"))],
                "\n\n",
                14.,
            ))
        }));
        out
    }
    #[test]
    fn prepend_and_append_hold_a_content_anchor_with_the_same_load_button() {
        let mut cs = ConversationState::default();
        cs.pinned = false;
        replace_rows(&mut cs, rows(1000, 6000));
        cs.offset = cs.doc.window.prefix_at(50) + 7.;
        let id = cs.doc.rows[50].id;
        replace_rows(&mut cs, rows(900, 6001));
        let index = cs.doc.rows.iter().position(|r| r.id == id).unwrap();
        assert!((cs.offset - cs.doc.window.prefix_at(index) - 7.).abs() < 0.1);
        assert!(!cs.pinned);
    }
    #[test]
    fn pinned_reader_follows_after_history_and_new_reply_arrive_together() {
        let mut cs = ConversationState::default();
        replace_rows(&mut cs, rows(1000, 6000));
        replace_rows(&mut cs, rows(900, 6001));
        rewindow(&mut cs);
        assert!(cs.pinned);
        assert_eq!(cs.doc.window.range.1, cs.doc.rows.len());
    }
}

#[cfg(test)]
mod stale_result_tests {
    use super::*;
    fn app() -> App {
        App::demo(fb_ui::Mode::Light, crate::state::Screen::Conversation)
    }
    #[test]
    fn send_mid_load_invalidates_the_old_completion() {
        let mut app = app();
        sync_markdown(&mut app);
        let pane = app.state.conversation.pane.clone().unwrap();
        app.state.conversation.loading_earlier = true;
        let old = app.state.conversation.earlier_seq;
        follow_latest(&mut app);
        earlier_loaded(&mut app, pane, old, Some(false));
        assert!(app.state.conversation.pinned);
        assert!(!app.state.conversation.loading_earlier);
        assert_ne!(app.state.conversation.earlier_seq, old);
    }
    #[test]
    fn old_session_picture_cannot_install_a_handle_or_preview() {
        let mut app = app();
        sync_markdown(&mut app);
        let reference = herdr_core::ImageRef {
            format: herdr_core::ImageFormat::Png,
            width: 1,
            height: 1,
            offset: Some(0),
            index: 0,
            digest: 1,
        };
        app.state
            .conversation
            .pictures
            .refs
            .insert(1, reference.clone());
        app.state.conversation.pictures.retain_near(vec![1]);
        let _ = update(
            &mut app,
            ConversationMsg::PictureLoaded {
                scope: "another session".into(),
                id: 1,
                reference,
                preview: false,
                seq: 0,
                data: Some(crate::pictures::Decoded {
                    width: 1,
                    height: 1,
                    rgba: vec![0; 4],
                }),
            },
        );
        assert!(app.state.conversation.pictures.handles.is_empty());
        assert!(app.state.conversation.pictures.preview.is_none());
    }
}
