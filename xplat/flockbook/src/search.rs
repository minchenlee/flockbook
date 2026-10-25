//! Search screen (`Screen::Search`): one field, results in two groups, Tabs and Messages.
//!
//! * Tabs: the query matches a tab's name, its workspace's name, or an agent's name (cheap, on the UI thread).
//!   An empty query lists every tab.
//! * Messages: the query matches the text of the loaded conversations (`state.conversations`: user and agent
//!   text, tool chip summaries, questions). Matching is case-insensitive substring over chars, so CJK works.
//!   A small corpus is searched inline. A large one runs on a worker thread; the result carries the query's
//!   sequence number and is dropped when a newer query started in the meantime (`SearchMsg::Results`).
//!
//! Keys: Cmd+F (Ctrl+F elsewhere) opens the screen from anywhere and focuses the field. On the screen, Up/Down move
//! the selection, Enter opens it, Esc clears the field (then leaves). Opening a tab selects it; opening a message
//! also selects its pane and scrolls the conversation to the message (`reveal`).
//!
//! This module never sends anything to a pane. It only reads state and navigates through `Message`.

mod matcher;

pub use matcher::{Parts, Role};

use crate::state::status_kind;
use crate::{style, App, Message};
use fb_ui::icons::{icon, Icon, SIZE_DEFAULT, SIZE_INLINE};
use fb_ui::selectable::flow::row_id;
use fb_ui::status;
use fb_ui::theme::*;
use herdr_core::AgentStatus;
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::{button, column, container, row, rich_text, scrollable, span, text, text_input, Space};
use iced::{Alignment, Background, Border, Element, Length, Subscription, Task};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Widest the search column gets (the app's screen column: 960 wide, rows inset 8 inside it).
pub const COLUMN_MAX: f32 = 960.0;
/// Space between the list and the detail area's edges (Swift `.listStyle(.sidebar)` row inset).
const LIST_INSET: f32 = 10.0;
/// Width of the toolbar search field (Swift `.searchable(placement: .toolbar)`).
pub const FIELD_W: f32 = 250.0;
/// Gap that separates the tab rows from the message rows (there are no section headers).
const GROUP_GAP: f32 = 12.0;
/// Message hits per pane, and in all.
pub const PER_PANE: usize = 6;
pub const MAX_MESSAGE_HITS: usize = 40;
/// Corpora up to this many chars are searched inline; bigger ones on a worker thread.
pub const INLINE_CHARS: usize = 150_000;
/// Swift sidebar-style list row: 32 tall.
const TAB_ROW_H: f32 = 32.0;
const MSG_ROW_H: f32 = 58.0;

fn field_id() -> iced::widget::Id {
    iced::widget::Id::new("flockbook-search-field")
}

fn list_id() -> iced::widget::Id {
    iced::widget::Id::new("flockbook-search-list")
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabHit {
    pub workspace: String,
    pub workspace_label: String,
    pub tab: String,
    pub tab_label: String,
    /// The pane to select: the one whose agent matched, else `None` (the tab's own choice).
    pub pane: Option<String>,
    pub agent: Option<String>,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MsgHit {
    pub workspace: String,
    pub workspace_label: String,
    pub tab: String,
    pub tab_label: String,
    pub pane: String,
    pub agent: Option<String>,
    pub status: AgentStatus,
    pub item: String,
    pub role: Role,
    pub parts: Parts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Tabs,
    Messages,
}

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub tabs: Vec<TabHit>,
    pub messages: Vec<MsgHit>,
    /// Index into tabs then messages.
    pub selected: usize,
    /// Number of the latest query. A message result with another number is stale.
    pub seq: u64,
    /// A worker thread is searching messages for the latest query.
    pub searching: bool,
    /// How many message searches ran on a worker (tests).
    pub async_runs: u64,
    /// How many times the corpus was built on the UI thread (tests: a big corpus must never be).
    pub ui_corpus_builds: u64,
    /// The corpus of message texts, with the signature of the conversations it was built from.
    corpus: Option<(u64, Arc<Vec<matcher::Entry>>, usize)>,
}

impl SearchState {
    pub fn len(&self) -> usize {
        self.tabs.len() + self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn group_of(&self, i: usize) -> Group {
        if i < self.tabs.len() {
            Group::Tabs
        } else {
            Group::Messages
        }
    }
}

#[derive(Debug, Clone)]
pub enum SearchMsg {
    /// Show the screen and focus the field (Cmd+F).
    Open,
    Input(String),
    Clear,
    Move(i32),
    /// Open the selected result (Enter).
    Activate,
    /// Open result number `i` (click).
    Click(usize),
    Escape,
    /// The messages found for query number `seq`.
    Results { seq: u64, hits: Vec<MsgHit> },
    /// A worker's raw answer ([`Job::run`]): the hits not yet resolved, and the corpus when the worker had to build it.
    /// `App::update` never sees this one: the task turns it into `Done`.
    Worked { seq: u64, sig: u64, corpus: Option<(Arc<Vec<matcher::Entry>>, usize)>, raws: Vec<matcher::Raw> },
    /// A worker's answer with resolved hits. The corpus it built is kept (for the signature `sig`), even when the answer
    /// itself is stale.
    Done { seq: u64, sig: u64, corpus: Option<(Arc<Vec<matcher::Entry>>, usize)>, hits: Vec<MsgHit> },
}

// ---------------------------------------------------------------------------------------------------- keys

/// Maps a key to a search message. `on_screen`: the search screen is showing.
pub fn on_key(key: Key, mods: Modifiers, on_screen: bool) -> Option<Message> {
    if mods.command() && !mods.shift() && !mods.alt() && matches!(key.as_ref(), Key::Character("f") | Key::Character("F")) {
        return Some(Message::Search(SearchMsg::Open));
    }
    if !on_screen || mods.command() || mods.alt() {
        return None;
    }
    match key.as_ref() {
        Key::Named(Named::ArrowDown) => Some(Message::Search(SearchMsg::Move(1))),
        Key::Named(Named::ArrowUp) => Some(Message::Search(SearchMsg::Move(-1))),
        Key::Named(Named::Escape) => Some(Message::Search(SearchMsg::Escape)),
        _ => None,
    }
}

/// Cmd+F always; Up/Down/Esc only while the screen shows.
pub fn subscription(app: &App) -> Subscription<Message> {
    if app.state.screen == crate::state::Screen::Search {
        keyboard::listen().filter_map(|e| match e {
            keyboard::Event::KeyPressed { key, modifiers, .. } => on_key(key, modifiers, true),
            _ => None,
        })
    } else {
        keyboard::listen().filter_map(|e| match e {
            keyboard::Event::KeyPressed { key, modifiers, .. } => on_key(key, modifiers, false),
            _ => None,
        })
    }
}

// ---------------------------------------------------------------------------------------------------- matching

fn agent_of(p: &herdr_core::Pane) -> Option<&str> {
    p.agent.as_ref().map(|a| a.name.as_str())
}

/// Tabs that match `q`, in workspace order. An empty query matches every tab.
pub fn tab_hits(app: &App, q: &str) -> Vec<TabHit> {
    let needle = matcher::fold_query(q);
    let mut out = Vec::new();
    for ws in &app.state.workspaces {
        for tab in &ws.tabs {
            let mut pane: Option<String> = None;
            let hit = if needle.is_empty() {
                true
            } else if matcher::contains(&tab.label, &needle) || matcher::contains(&ws.label, &needle) {
                true
            } else if let Some(p) = tab.panes.iter().find(|p| agent_of(p).is_some_and(|n| matcher::contains(n, &needle)) || p.label.as_deref().is_some_and(|n| matcher::contains(n, &needle))) {
                pane = Some(p.id.clone());
                true
            } else {
                false
            };
            if hit {
                let agent = pane
                    .as_ref()
                    .and_then(|id| tab.panes.iter().find(|p| &p.id == id))
                    .and_then(agent_of)
                    .or_else(|| tab.panes.iter().find_map(agent_of))
                    .map(str::to_string);
                out.push(TabHit { workspace: ws.id.clone(), workspace_label: ws.label.clone(), tab: tab.id.clone(), tab_label: tab.label.clone(), pane, agent, status: tab.status });
            }
        }
    }
    out
}

/// Where a pane lives: (workspace id and label, tab id and label, agent name, status).
fn pane_info(app: &App) -> HashMap<String, (String, String, String, String, Option<String>, AgentStatus)> {
    let mut m = HashMap::new();
    for ws in &app.state.workspaces {
        for tab in &ws.tabs {
            for p in &tab.panes {
                m.insert(p.id.clone(), (ws.id.clone(), ws.label.clone(), tab.id.clone(), tab.label.clone(), agent_of(p).map(str::to_string), tab.status));
            }
        }
    }
    m
}

/// Cheap signature of the loaded conversations: per pane its id, item count, first and last item id, and the size of the
/// last item's text (a streaming reply grows under one id). No formatting, no walk over all items.
fn corpus_signature(app: &App) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut ids: Vec<&String> = app.state.conversations.keys().collect();
    ids.sort();
    for id in ids {
        let items = &app.state.conversations[id];
        id.hash(&mut h);
        items.len().hash(&mut h);
        items.first().map(|i| &i.id).hash(&mut h);
        if let Some(l) = items.last() {
            l.id.hash(&mut h);
            matcher::text_len(&l.kind).hash(&mut h);
        }
    }
    h.finish()
}

/// The conversations of all panes in workspace order (so results group the way the sidebar lists them), as shared
/// snapshots: cloning an `Arc` per pane, no text is copied.
fn pane_snapshot(app: &App) -> Vec<(String, Arc<Vec<herdr_core::ConversationItem>>)> {
    let mut out = Vec::new();
    for ws in &app.state.workspaces {
        for tab in &ws.tabs {
            for p in &tab.panes {
                if let Some(items) = app.state.conversations.get(&p.id) {
                    out.push((p.id.clone(), items.clone()));
                }
            }
        }
    }
    out
}

/// The searchable texts of a snapshot, and their size in bytes.
fn build_corpus(panes: &[(String, Arc<Vec<herdr_core::ConversationItem>>)]) -> (Arc<Vec<matcher::Entry>>, usize) {
    let mut entries = Vec::new();
    for (id, items) in panes {
        for it in items.iter() {
            entries.extend(matcher::texts_of(id, it));
        }
    }
    let chars: usize = entries.iter().map(|e| e.text.len()).sum();
    (Arc::new(entries), chars)
}

/// What a worker thread needs to answer a query: the snapshot, the needle, and the corpus if one is cached for it.
pub struct Job {
    sig: u64,
    panes: Vec<(String, Arc<Vec<herdr_core::ConversationItem>>)>,
    cached: Option<(Arc<Vec<matcher::Entry>>, usize)>,
    needle: Vec<char>,
    seq: u64,
}

impl Job {
    /// The blocking work: builds the corpus if it is not cached, searches it. Runs on a worker thread.
    pub fn run(self) -> SearchMsg {
        let (corpus, chars, built) = match self.cached {
            Some((c, n)) => (c, n, false),
            None => {
                let (c, n) = build_corpus(&self.panes);
                (c, n, true)
            }
        };
        let raws = matcher::search(&corpus, &self.needle, PER_PANE, MAX_MESSAGE_HITS);
        SearchMsg::Worked { seq: self.seq, sig: self.sig, corpus: built.then_some((corpus, chars)), raws }
    }
}

/// The job for the current query on the current conversations (tests run it without a runtime).
pub fn job(app: &App, seq: u64, needle: Vec<char>) -> Job {
    let sig = corpus_signature(app);
    let cached = app.state.search.corpus.as_ref().filter(|(s, _, _)| *s == sig).map(|(_, c, n)| (c.clone(), *n));
    Job { sig, panes: pane_snapshot(app), cached, needle, seq }
}

fn resolve(info: &HashMap<String, (String, String, String, String, Option<String>, AgentStatus)>, raw: matcher::Raw) -> Option<MsgHit> {
    let (ws, wsl, tab, tabl, agent, status) = info.get(&raw.pane)?.clone();
    Some(MsgHit { workspace: ws, workspace_label: wsl, tab, tab_label: tabl, pane: raw.pane, agent, status, item: raw.item, role: raw.role, parts: raw.parts })
}

/// Recomputes the hits for the current query. The tab hits are ready on return; the message hits are too when the
/// corpus is small, else a task delivers them as `SearchMsg::Results`.
fn run_query(app: &mut App) -> Task<Message> {
    let q = app.state.search.query.clone();
    let hits = tab_hits(app, &q);
    let seq = app.state.search.seq + 1;
    let s = &mut app.state.search;
    s.seq = seq;
    s.tabs = hits;
    s.messages.clear();
    s.searching = false;
    s.selected = 0;
    let needle = matcher::fold_query(&q);
    if needle.is_empty() {
        return Task::none();
    }
    let info = pane_info(app);
    let sig = corpus_signature(app);
    let cached = app.state.search.corpus.as_ref().filter(|(s, _, _)| *s == sig).map(|(_, c, n)| (c.clone(), *n));
    // Small corpora are searched inline: the cached one, or one built now when the texts are few (the estimate is a walk
    // without copying). A big one is built AND searched on a worker thread from an `Arc` snapshot.
    let small = cached.as_ref().map(|(_, n)| *n <= INLINE_CHARS).unwrap_or_else(|| matcher::estimate_chars(app.state.conversations.values().flat_map(|v| v.iter())) <= INLINE_CHARS);
    if small {
        let (corpus, _) = match cached {
            Some(c) => c,
            None => {
                app.state.search.ui_corpus_builds += 1;
                let (c, n) = build_corpus(&pane_snapshot(app));
                app.state.search.corpus = Some((sig, c.clone(), n));
                (c, n)
            }
        };
        let raws = matcher::search(&corpus, &needle, PER_PANE, MAX_MESSAGE_HITS);
        app.state.search.messages = raws.into_iter().filter_map(|r| resolve(&info, r)).collect();
        return Task::none();
    }
    app.state.search.searching = true;
    app.state.search.async_runs += 1;
    let job = job(app, seq, needle);
    Task::perform(
        async move {
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(job.run());
            });
            rx.await.ok()
        },
        move |done| match done {
            Some(SearchMsg::Worked { seq, sig, corpus, raws }) => {
                let hits = raws.into_iter().filter_map(|r| resolve(&info, r)).collect();
                Message::Search(SearchMsg::Done { seq, sig, corpus, hits })
            }
            _ => Message::Search(SearchMsg::Results { seq, hits: Vec::new() }),
        },
    )
}

// ---------------------------------------------------------------------------------------------------- update

pub fn update(app: &mut App, msg: SearchMsg) -> Task<Message> {
    match msg {
        SearchMsg::Open => {
            app.state.screen = crate::state::Screen::Search;
            let q = run_query(app);
            Task::batch([q, iced::widget::operation::focus(field_id())])
        }
        SearchMsg::Input(s) => {
            app.state.search.query = s;
            run_query(app)
        }
        SearchMsg::Clear => {
            app.state.search.query.clear();
            let q = run_query(app);
            Task::batch([q, iced::widget::operation::focus(field_id())])
        }
        SearchMsg::Escape => {
            if app.state.search.query.is_empty() {
                app.state.screen = crate::state::Screen::Conversation;
                Task::none()
            } else {
                update(app, SearchMsg::Clear)
            }
        }
        SearchMsg::Move(d) => {
            let s = &mut app.state.search;
            if s.is_empty() {
                return Task::none();
            }
            s.selected = (s.selected as i64 + d as i64).clamp(0, s.len() as i64 - 1) as usize;
            scroll_to_selected(&app.state.search)
        }
        SearchMsg::Activate => {
            let i = app.state.search.selected;
            open(app, i)
        }
        SearchMsg::Click(i) => {
            app.state.search.selected = i;
            open(app, i)
        }
        SearchMsg::Worked { .. } => Task::none(),
        SearchMsg::Done { seq, sig, corpus, hits } => {
            if let Some((c, n)) = corpus {
                app.state.search.corpus = Some((sig, c, n));
            }
            update(app, SearchMsg::Results { seq, hits })
        }
        SearchMsg::Results { seq, hits } => {
            let s = &mut app.state.search;
            if seq != s.seq {
                return Task::none(); // stale: the query changed while the worker ran
            }
            s.searching = false;
            s.messages = hits;
            s.selected = s.selected.min(s.len().saturating_sub(1));
            Task::none()
        }
    }
}

fn scroll_to_selected(s: &SearchState) -> Task<Message> {
    let tabs_h = s.tabs.len() as f32 * TAB_ROW_H + if s.tabs.is_empty() { 0.0 } else { GROUP_GAP };
    let y = if s.selected < s.tabs.len() { s.selected as f32 * TAB_ROW_H } else { tabs_h + (s.selected - s.tabs.len()) as f32 * MSG_ROW_H };
    let y = (y - 120.0).max(0.0);
    iced::widget::operation::scroll_to(list_id(), scrollable::AbsoluteOffset { x: None, y: Some(y) })
}

/// Opens result `i`: selects its tab (and pane), shows the conversation, and for a message scrolls to it.
fn open(app: &mut App, i: usize) -> Task<Message> {
    let s = &app.state.search;
    if i < s.tabs.len() {
        let h = s.tabs[i].clone();
        let t = app.update(Message::SelectTab { workspace: h.workspace, tab: h.tab });
        if let Some(p) = h.pane {
            let t2 = app.update(Message::SelectPane(p));
            return Task::batch([t, t2]);
        }
        t
    } else if let Some(h) = s.messages.get(i - s.tabs.len()).cloned() {
        let t = app.update(Message::SelectTab { workspace: h.workspace, tab: h.tab });
        let t2 = app.update(Message::SelectPane(h.pane));
        reveal(app, &h.item);
        Task::batch([t, t2])
    } else {
        Task::none()
    }
}

/// Scrolls the shown conversation so the row of item `item` sits near the top. An item that has no row of its own
/// (tool steps merge into the step before) falls back to the closest earlier item that has one. Returns false when
/// nothing of the conversation is built.
pub fn reveal(app: &mut App, item: &str) -> bool {
    let ids: Vec<String> = app.state.conversation_items().iter().map(|i| i.id.clone()).collect();
    let Some(pos) = ids.iter().position(|i| i == item) else { return false };
    let cs = &mut app.state.conversation;
    for id in ids[..=pos].iter().rev() {
        if let Some(&idx) = cs.doc.index.get(&row_id(&[id, "0"])) {
            let top = cs.doc.window.block_top(idx);
            let max = (cs.doc.total_height() - cs.viewport.height).max(0.0);
            cs.offset = (top - 64.0).clamp(0.0, max);
            cs.pinned = cs.offset >= max - 2.0;
            let off = if cs.pinned { f32::MAX } else { (cs.offset - fb_ui::selectable::flow::PAD_TOP).max(0.0) };
            cs.doc.window.rebuild_at(off, cs.viewport.height);
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------------- view

fn highlighted<'a>(t: Tokens, p: &Parts, base: TextStyle, color: iced::Color) -> Element<'a, Message> {
    let hit_bg = Background::Color(iced::Color { a: if t.mode.is_dark() { 0.32 } else { 0.22 }, ..t.accent });
    let mut spans: Vec<iced::widget::text::Span<'a, ()>> = Vec::new();
    if !p.before.is_empty() {
        spans.push(span(p.before.clone()).color(color).font(base.font()));
    }
    if !p.hit.is_empty() {
        spans.push(span(p.hit.clone()).color(t.label).font(HEADLINE.font()).background(hit_bg).padding([0, 1]));
    }
    if !p.after.is_empty() {
        spans.push(span(p.after.clone()).color(color).font(base.font()));
    }
    rich_text(spans).size(base.size).font(base.font()).wrapping(text::Wrapping::None).into()
}

fn row_button<'a>(t: Tokens, selected: bool, h: f32, pad: f32, content: Element<'a, Message>, msg: Message) -> Element<'a, Message> {
    button(container(content).height(Length::Fill).align_y(Alignment::Center).padding([0.0, pad]))
        .on_press(msg)
        .width(Length::Fill)
        .height(Length::Fixed(h))
        .padding(0)
        .style(move |_, st| {
            let bg = if selected {
                Some(Background::Color(t.sidebar_sel))
            } else if matches!(st, button::Status::Hovered) {
                Some(Background::Color(t.fill_1))
            } else {
                None
            };
            button::Style { background: bg, text_color: t.label, border: Border { radius: R_ROW.into(), ..Default::default() }, ..Default::default() }
        })
        .into()
}

fn tab_row<'a>(app: &App, i: usize, h: &TabHit, needle: &[char], pad: f32) -> Element<'a, Message> {
    let t = app.tokens();
    let sel = app.state.search.selected == i;
    let label = matcher::split(&h.tab_label, needle);
    // The row: status, the tab's name, its agent, then the workspace at the trailing edge (no workspace chip).
    let mut r = row![status::symbol(status_kind(h.status), &t), highlighted(t, &label, BODY, t.label)].spacing(SP_8).align_y(Alignment::Center);
    if let Some(a) = &h.agent {
        r = r.push(highlighted(t, &matcher::split(a, needle), BODY, t.label_2));
    }
    r = r.push(Space::new().width(Length::Fill));
    r = r.push(highlighted(t, &matcher::split(&h.workspace_label, needle), BODY, t.label_2));
    row_button(t, sel, TAB_ROW_H, pad, r.into(), Message::Search(SearchMsg::Click(i)))
}

fn msg_row<'a>(app: &App, i: usize, h: &MsgHit, pad: f32) -> Element<'a, Message> {
    let t = app.tokens();
    let sel = app.state.search.selected == i;
    let who = match h.role {
        Role::You => "You".to_string(),
        Role::Agent => "Agent".to_string(),
        Role::Tool => "Tool".to_string(),
        Role::Other => "Note".to_string(),
    };
    let head = row![
        HEADLINE.text(h.agent.clone().unwrap_or_else(|| h.tab_label.clone())).color(t.label).wrapping(text::Wrapping::None),
        CAPTION.text(format!("{} \u{b7} {} \u{b7} {}", h.workspace_label, h.tab_label, who)).color(t.label_2).wrapping(text::Wrapping::None),
    ]
    .spacing(SP_8)
    .align_y(Alignment::Center);
    let body = column![head, container(highlighted(t, &h.parts, BODY, t.label_2)).clip(true).width(Length::Fill)].spacing(3);
    let r = row![container(status::symbol(status_kind(h.status), &t)).padding([2, 0]), body.width(Length::Fill)].spacing(SP_8).align_y(Alignment::Center);
    row_button(t, sel, MSG_ROW_H, pad, r.into(), Message::Search(SearchMsg::Click(i)))
}

/// The search field in the toolbar's trailing end, shown on the Search screen (Swift `SearchView`:
/// `.searchable(text:, placement: .toolbar)`): a capsule as tall as the tab island.
pub fn field<'a>(app: &App, height: f32) -> Element<'a, Message> {
    let t = app.tokens();
    let q = &app.state.search.query;
    let input = text_input("Search tabs, agents and messages", q)
        .id(field_id())
        .on_input(|s| Message::Search(SearchMsg::Input(s)))
        .on_submit(Message::Search(SearchMsg::Activate))
        .size(13)
        .font(BODY.font())
        .padding(0)
        .style(move |_, _| text_input::Style {
            background: Background::Color(iced::Color::TRANSPARENT),
            border: Border::default(),
            icon: t.label_2,
            placeholder: t.label_3,
            value: t.label,
            selection: iced::Color { a: 0.28, ..t.accent },
        });
    let mut r = row![icon(Icon::Search, SIZE_DEFAULT, t.label_2), input].spacing(SP_8).align_y(Alignment::Center);
    if !q.is_empty() {
        r = r.push(button(icon(Icon::X, SIZE_INLINE, t.label_2)).padding(2).style(button::text).on_press(Message::Search(SearchMsg::Clear)));
    }
    container(r)
        .padding([0, 12])
        .width(Length::Fixed(FIELD_W))
        .height(Length::Fixed(height))
        .align_y(Alignment::Center)
        .style(style::fill(t.fill_2, height / 2.0))
        .into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    iced::widget::responsive(move |size| view_sized(app, size.width)).into()
}

/// The list spans the detail area (inset 10); the rows' content sits in the centred 960 column, 8 in from it
/// (Swift `SearchView`: `listRowInsets(leading: ScreenColumn.margin(area.width) + 8)`).
fn view_sized(app: &App, width: f32) -> Element<'_, Message> {
    let t = app.tokens();
    let pad = ((width - COLUMN_MAX) / 2.0).max(0.0) + 8.0 - LIST_INSET;
    let pad = pad.max(8.0);
    let s = &app.state.search;
    let needle = matcher::fold_query(&s.query);
    // One flat list of rows, as the screen's list is: no section headers, only a gap between the two groups.
    let mut list = column![].spacing(0);
    for (i, h) in s.tabs.iter().enumerate() {
        list = list.push(tab_row(app, i, h, &needle, pad));
    }
    if !s.messages.is_empty() {
        if !s.tabs.is_empty() {
            list = list.push(Space::new().height(GROUP_GAP));
        }
        for (k, h) in s.messages.iter().enumerate() {
            list = list.push(msg_row(app, s.tabs.len() + k, h, pad));
        }
    }
    let body: Element<'_, Message> = if s.is_empty() && !s.searching {
        container(
            column![icon(Icon::Search, 28.0, t.label_3), Space::new().height(SP_8), TITLE_3.text(format!("No Results for \u{201c}{}\u{201d}", s.query.trim())).color(t.label_2), Space::new().height(SP_4), BODY.text("Check the spelling or try a new search.").color(t.label_3)]
                .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else {
        let mut c = column![scrollable(container(list)).id(list_id()).height(Length::Fill)];
        if s.searching {
            c = c.push(container(CAPTION.text("Searching messages\u{2026}").color(t.label_3)).padding([6, 12]));
        }
        c.into()
    };
    container(body).padding(iced::Padding { top: 7.0, right: LIST_INSET, bottom: 16.0, left: LIST_INSET }).width(Length::Fill).height(Length::Fill).into()
}

/// Loads the conversation of every pane into `state.conversations` so message search covers all tabs. The app loads
/// only the selected pane's conversation by itself. Tests and snapshots use this with the demo source; a live
/// window would start a log reader for every pane, so it does not call it.
pub fn preload_all(app: &mut App) {
    let panes: Vec<herdr_core::Pane> = app.state.workspaces.iter().flat_map(|w| w.tabs.iter().flat_map(|t| t.panes.iter().cloned())).collect();
    for p in panes {
        let items = app.source.conversation(&p);
        app.state.conversations.insert(p.id.clone(), Arc::new(items));
    }
}
