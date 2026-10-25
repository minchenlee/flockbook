//! Overview screen (plus the Needs you and Search stubs, which share this file's `view`).
//!
//! Port of the Swift `OverviewView`: a large greeting, a one-line summary and the Group by pop-up, then cards
//! grouped by state (Needs you, Working, Done and resting). A second grouping shows one section per workspace.
//!
//! The clock is injectable (`OverviewState::clock`) so snapshots and tests are deterministic.

use crate::state::{monogram, status_kind, Screen};
use crate::{App, Message};
use fb_ui::status;
use fb_ui::theme::*;
use herdr_core::{AgentStatus, ItemKind, Pane, Tab, Workspace};
use iced::widget::{button, column, container, responsive, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Minimum card width. Columns = how many fit in the content column (at most `COLUMN_MAX` wide, its 36 pt side
/// padding inside, as Swift pads before `screenColumn`).
pub const CARD_MIN_W: f32 = 260.0;
pub const GRID_GAP: f32 = 10.0;
pub const COLUMN_MAX: f32 = 960.0;
const SIDE_PAD: f32 = 36.0;
const PREVIEW_TTL: Duration = Duration::from_secs(2);

/// The clock the screen reads: hour of day (0-23, local) and Unix seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clock {
    pub hour: u32,
    pub now: f64,
}

impl Clock {
    pub fn system() -> Clock {
        use chrono::Timelike;
        let now = chrono::Local::now();
        Clock { hour: now.hour(), now: now.timestamp() as f64 }
    }

    /// The fixed clock of the demo snapshots: 15:00, a little after the demo conversations end.
    pub fn demo() -> Clock {
        Clock { hour: 15, now: 1_790_000_000.0 + 1500.0 }
    }

    pub fn greeting(&self) -> &'static str {
        match self.hour {
            0..=11 => "Good morning",
            12..=17 => "Good afternoon",
            _ => "Good evening",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupBy {
    #[default]
    Status,
    Workspace,
}

impl GroupBy {
    const ALL: [GroupBy; 2] = [GroupBy::Status, GroupBy::Workspace];
}

impl std::fmt::Display for GroupBy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            GroupBy::Status => "Status",
            GroupBy::Workspace => "Workspace",
        })
    }
}

/// The last thing the agent said or asked, and when. A card shows only the when, as its age.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Preview {
    pub text: String,
    pub timestamp: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct OverviewState {
    /// `None` = the system clock. Tests and demo snapshots inject one.
    pub clock: Option<Clock>,
    pub group: GroupBy,
    /// Preview per pane id, refreshed at most every `PREVIEW_TTL` (live conversations can be large).
    cache: RefCell<HashMap<String, (Instant, Preview)>>,
}

impl OverviewState {
    pub fn clock(&self) -> Clock {
        self.clock.unwrap_or_else(Clock::system)
    }
}

#[derive(Debug, Clone)]
pub enum OverviewMsg {
    Open { workspace: String, tab: String },
    SetGroup(GroupBy),
}

pub fn update(app: &mut App, msg: OverviewMsg) -> Task<Message> {
    match msg {
        OverviewMsg::Open { workspace, tab } => app.update(Message::SelectTab { workspace, tab }),
        OverviewMsg::SetGroup(g) => {
            app.state.overview.group = g;
            Task::none()
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Pure helpers (tested without a window)

/// One tab with its workspace.
#[derive(Debug, Clone, Copy)]
pub struct Entry<'a> {
    pub ws: &'a Workspace,
    pub tab: &'a Tab,
}

/// Tabs split by state, in workspace order. `Unknown` counts as idle.
#[derive(Debug, Default)]
pub struct Groups<'a> {
    pub blocked: Vec<Entry<'a>>,
    pub working: Vec<Entry<'a>>,
    pub done: Vec<Entry<'a>>,
    pub idle: Vec<Entry<'a>>,
}

pub fn groups(workspaces: &[Workspace]) -> Groups<'_> {
    let mut g = Groups::default();
    for ws in workspaces {
        for tab in &ws.tabs {
            let e = Entry { ws, tab };
            match tab.status {
                AgentStatus::Blocked => g.blocked.push(e),
                AgentStatus::Working => g.working.push(e),
                AgentStatus::Done => g.done.push(e),
                AgentStatus::Idle | AgentStatus::Unknown => g.idle.push(e),
            }
        }
    }
    g
}

/// Columns for a content area `width` wide: as many `CARD_MIN_W` cards as fit, at least one.
pub fn columns_for(width: f32) -> usize {
    let w = (width.min(COLUMN_MAX) - 2.0 * SIDE_PAD).max(0.0);
    (((w + GRID_GAP) / (CARD_MIN_W + GRID_GAP)).floor() as usize).max(1)
}

/// "now", "5m", "3h", "2d".
pub fn age_label(now: f64, ts: f64) -> String {
    let s = (now - ts).max(0.0);
    if s < 45.0 {
        "now".into()
    } else if s < 3600.0 {
        format!("{}m", (s / 60.0).round().max(1.0) as u64)
    } else if s < 86_400.0 {
        format!("{}h", (s / 3600.0).floor() as u64)
    } else {
        format!("{}d", (s / 86_400.0).floor() as u64)
    }
}

/// First readable line of a markdown text: markers and fences stripped.
fn first_line(s: &str) -> String {
    for line in s.lines() {
        let l = line.trim().trim_start_matches(['#', '>', '-', '*', '|', ' ']).replace(['`', '*'], "");
        let l = l.trim();
        if !l.is_empty() && !l.chars().all(|c| c == '-' || c == '|' || c == ':') {
            return l.to_string();
        }
    }
    String::new()
}

/// The preview of a conversation: the newest item that has something to show.
pub fn preview_of(items: &[herdr_core::ConversationItem]) -> Preview {
    for it in items.iter().rev() {
        let text = match &it.kind {
            ItemKind::PermissionRequest(r) => format!("Asks to run: {}", if r.command.is_empty() { &r.title } else { &r.command }),
            ItemKind::Question(q) => q.questions.first().map(|q| format!("Asks: {}", q.text)).unwrap_or_default(),
            ItemKind::AssistantText(s) => first_line(s),
            ItemKind::UserText(s) => format!("You: {}", first_line(s)),
            ItemKind::ToolChips(chips) => chips.last().map(|c| c.text.clone()).unwrap_or_default(),
            ItemKind::Status(s) => first_line(s),
            ItemKind::Images(refs) => if refs.len() == 1 { "Image".to_string() } else { format!("{} images", refs.len()) },
        };
        if !text.is_empty() {
            return Preview { text, timestamp: it.timestamp };
        }
    }
    Preview::default()
}

fn agent_pane(tab: &Tab) -> Option<&Pane> {
    tab.panes.iter().find(|p| p.agent.is_some()).or_else(|| tab.panes.first())
}

fn agent_name(tab: &Tab) -> String {
    tab.panes.iter().find_map(|p| p.agent.as_ref()).map(|a| a.name.clone()).unwrap_or_else(|| tab.label.clone())
}

fn preview_for(app: &App, tab: &Tab) -> Preview {
    let Some(p) = agent_pane(tab) else { return Preview::default() };
    let cache = &app.state.overview.cache;
    if let Some((at, v)) = cache.borrow().get(&p.id) {
        if at.elapsed() < PREVIEW_TTL {
            return v.clone();
        }
    }
    let v = preview_of(&app.source.conversation(p));
    cache.borrow_mut().insert(p.id.clone(), (Instant::now(), v.clone()));
    v
}

// ---------------------------------------------------------------------------------------------------------------
// Views

fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

/// Swift Monogram (StatusViews.swift:98-109): caption2 semibold, primary 0.7 on primary 0.1, 22 pt, Radius.control.
fn monogram_chip<'a>(t: Tokens, label: &str) -> Element<'a, Message> {
    container(text(monogram(label)).size(10).font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(with_alpha(t.label, 0.7)))
        .center(Length::Fixed(22.0))
        .style(crate::style::fill(with_alpha(t.label, 0.1), 6.0))
        .into()
}

fn card<'a>(app: &App, e: Entry<'a>, clock: Clock) -> Element<'a, Message> {
    let t = app.tokens();
    let st = e.tab.status;
    // The age at the name's trailing edge is the iced app's own (the Swift card has none); tests keep it.
    let pv = preview_for(app, e.tab);
    let age = pv.timestamp.map(|ts| age_label(clock.now, ts)).unwrap_or_default();
    let title_row = row![container(HEADLINE.text(agent_name(e.tab)).color(t.label).wrapping(text::Wrapping::None)).clip(true).width(Length::Fill), CAPTION.text(age).color(t.label_3)]
        .spacing(SP_8)
        .align_y(Alignment::Center);

    // The card subtitle is plain text (workspace · tab), like Swift's `Text("\(ws.label) · \(t.label)")`;
    // the workspace monogram belongs to the workspace section header only.
    let sub = container(CAPTION.text(format!("{} \u{b7} {}", e.ws.label, e.tab.label)).color(t.label_2).wrapping(text::Wrapping::None))
        .clip(true)
        .width(Length::Fill);

    // Two lines, as Swift's card (`Screens.swift:216-221`): the name and the place, nothing else.
    let info = column![title_row, sub].spacing(2);

    let body = row![container(status::symbol_sized(status_kind(st), &t, 16.0, 0.0)), info].spacing(10.0).align_y(Alignment::Center);
    let msg = Message::Overview(OverviewMsg::Open { workspace: e.ws.id.clone(), tab: e.tab.id.clone() });
    button(container(body).padding(12).width(Length::Fill))
        .on_press(msg)
        .padding(0)
        .width(Length::Fill)
        .style(move |_, status| {
            let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
            let (bg, edge) = card_colors(&t, st, hover);
            button::Style {
                background: Some(Background::Color(bg)),
                border: Border { color: edge, width: 1.0, radius: R_CARD.into() },
                text_color: t.label,
                ..Default::default()
            }
        })
        .into()
}

/// Card fill and hairline (Swift Screens.swift:225-226): one fill for every state, the control background (white in
/// light mode, a step above the window in dark), and a primary 0.09 hairline, orange at 0.5 for Needs you.
pub fn card_colors(t: &Tokens, st: AgentStatus, hover: bool) -> (Color, Color) {
    let fill = match (t.mode.is_dark(), hover) {
        (false, false) => t.bg,
        (false, true) | (true, false) => t.fill_1,
        (true, true) => t.fill_2,
    };
    (fill, if st == AgentStatus::Blocked { with_alpha(t.blocked, 0.5) } else { t.sep })
}

fn grid<'a>(app: &App, entries: &[Entry<'a>], cols: usize, clock: Clock) -> Element<'a, Message> {
    let mut rows = column![].spacing(GRID_GAP);
    for chunk in entries.chunks(cols) {
        let mut r = row![].spacing(GRID_GAP);
        for e in chunk {
            r = r.push(card(app, *e, clock));
        }
        for _ in chunk.len()..cols {
            r = r.push(Space::new().width(Length::Fill));
        }
        rows = rows.push(r);
    }
    rows.into()
}

fn section<'a>(app: &App, title: &'a str, entries: &[Entry<'a>], cols: usize, clock: Clock) -> Option<Element<'a, Message>> {
    if entries.is_empty() {
        return None;
    }
    let t = app.tokens();
    Some(column![HEADLINE.text(title).color(t.label), grid(app, entries, cols, clock)].spacing(SP_8).into())
}

/// "Group by" with a pop-up button (Swift Screens.swift:155-159: a `.menu` Picker, fixed size): the label, then the
/// chosen value and an up-down chevron in a quiet rounded field.
fn group_toggle<'a>(app: &App) -> Element<'a, Message> {
    use iced::widget::{pick_list, stack};
    let t = app.tokens();
    let cur = app.state.overview.group;
    let field = pick_list(GroupBy::ALL, Some(cur), |g| Message::Overview(OverviewMsg::SetGroup(g)))
        .handle(pick_list::Handle::None)
        .text_size(BODY.size)
        .font(BODY.font())
        .padding(iced::Padding { top: 3.0, bottom: 3.0, left: 12.0, right: 28.0 })
        .width(Length::Fixed(115.0))
        .style(move |_, st| pick_list::Style {
            text_color: t.label,
            placeholder_color: t.label_2,
            handle_color: t.label,
            background: Background::Color(if matches!(st, pick_list::Status::Hovered | pick_list::Status::Opened { .. }) { t.fill_3 } else { t.fill_2 }),
            border: Border { radius: R_ROW.into(), ..Default::default() },
        })
        .menu_style(move |_| iced::overlay::menu::Style {
            background: Background::Color(t.bg),
            border: Border { color: t.sep, width: 1.0, radius: R_ROW.into() },
            text_color: t.label,
            selected_text_color: t.on_accent,
            selected_background: Background::Color(t.accent),
            shadow: shadow_e2(&t),
        });
    let chevron = container(fb_ui::icons::icon(fb_ui::icons::Icon::ChevronsUpDown, 12.0, t.label)).height(Length::Fill).width(Length::Fixed(115.0)).align_x(Alignment::End).align_y(Alignment::Center).padding(iced::Padding { right: 8.0, ..iced::Padding::ZERO });
    row![BODY.text("Group by").color(t.label), stack![field, chevron]]
        .spacing(SP_8)
        .align_y(Alignment::Center)
        .into()
}

fn workspace_sections<'a>(app: &'a App, cols: usize, clock: Clock) -> Element<'a, Message> {
    let t = app.tokens();
    let mut order: Vec<&Workspace> = app.state.workspaces.iter().filter(|w| !w.tabs.is_empty()).collect();
    let key = |w: &Workspace| {
        let c = crate::state::State::counts(w);
        if c.blocked > 0 {
            0
        } else if c.working > 0 {
            1
        } else {
            2
        }
    };
    order.sort_by_key(|w| key(w)); // stable: workspace order inside a rank
    let rank = |s: AgentStatus| match s {
        AgentStatus::Blocked => 0,
        AgentStatus::Working => 1,
        AgentStatus::Done => 2,
        _ => 3,
    };
    let mut col = column![].spacing(SP_24);
    for ws in order {
        let c = crate::state::State::counts(ws);
        let mut parts = Vec::new();
        if c.blocked > 0 {
            parts.push(format!("{} need you", c.blocked));
        }
        if c.working > 0 {
            parts.push(format!("{} working", c.working));
        }
        let calm = c.done + c.idle;
        if calm > 0 {
            parts.push(format!("{calm} resting"));
        }
        let mut entries: Vec<Entry> = ws.tabs.iter().map(|tab| Entry { ws, tab }).collect();
        entries.sort_by_key(|e| rank(e.tab.status));
        let head = row![monogram_chip(t, &ws.label), HEADLINE.text(ws.label.clone()).color(t.label), CAPTION.text(parts.join(" \u{b7} ")).color(t.label_2)]
            .spacing(9)
            .align_y(Alignment::Center);
        col = col.push(column![head, grid(app, &entries, cols, clock)].spacing(SP_8));
    }
    col.into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    match app.state.screen {
        Screen::NeedsYou => return crate::needs_you::view(app),
        Screen::Search => return crate::search::view(app),
        _ => {}
    }
    responsive(move |size| {
        let t = app.tokens();
        let s = &app.state;
        let clock = s.overview.clock();
        let cols = columns_for(size.width);
        let g = groups(&s.workspaces);
        let total = s.tab_count();
        let sub = format!("{} tabs \u{b7} {} working \u{b7} {} need you", total, g.working.len(), g.blocked.len());

        let head = row![
            column![LARGE_TITLE.text(clock.greeting()).color(t.label), BODY.text(sub).color(t.label_2)].spacing(6).width(Length::Fill),
            container(group_toggle(app)),
        ]
        .align_y(Alignment::Start);

        let mut body = column![].spacing(SP_24);
        match s.overview.group {
            GroupBy::Status => {
                if let Some(x) = section(app, "Needs you", &g.blocked, cols, clock) {
                    body = body.push(x);
                }
                if let Some(x) = section(app, "Working", &g.working, cols, clock) {
                    body = body.push(x);
                }
                // Swift: done and idle tabs are ordinary cards under their own heading (`Screens.swift:191`).
                let calm: Vec<Entry> = g.done.iter().chain(g.idle.iter()).copied().collect();
                if let Some(x) = section(app, "Done and resting", &calm, cols, clock) {
                    body = body.push(x);
                }
            }
            GroupBy::Workspace => body = body.push(workspace_sections(app, cols, clock)),
        }

        let col = column![head, body].spacing(SP_24).max_width(COLUMN_MAX - 2.0 * SIDE_PAD);
        let centered = container(col).padding(iced::Padding { top: 36.0, right: SIDE_PAD, bottom: 36.0, left: SIDE_PAD }).center_x(Length::Fill);
        scrollable(centered).height(Length::Fill).into()
    })
    .into()
}

#[cfg(test)]
mod card_tests {
    use super::*;

    #[test]
    fn card_fills_follow_the_design() {
        // Swift Screens.swift:225-226: one fill for every state, an orange 0.5 hairline only for Needs you.
        for t in [Tokens::LIGHT, Tokens::DARK] {
            let (bg, edge) = card_colors(&t, AgentStatus::Blocked, false);
            assert!((edge.a - 0.5).abs() < 1e-6 && edge.g == t.blocked.g, "orange hairline");
            assert_eq!(card_colors(&t, AgentStatus::Working, false), (bg, t.sep));
            assert_eq!(card_colors(&t, AgentStatus::Done, false).0, bg);
            assert_ne!(card_colors(&t, AgentStatus::Working, true).0, bg, "hover is stronger");
        }
        assert_eq!(card_colors(&Tokens::LIGHT, AgentStatus::Idle, false).0, Tokens::LIGHT.bg, "white, as controlBackgroundColor");
    }
}
