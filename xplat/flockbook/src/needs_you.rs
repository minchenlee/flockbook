//! Needs you screen: a list of blocked tabs on the left, the selected request on the right.
//!
//! Port of the Swift `InboxView`. The list column shows the blocked items (agent, workspace chip, request
//! one-liner, age), then a quiet "Working, no action needed" section and one "n agents resting" pill. The detail
//! shows the request as an approval card (or a question card). With nothing blocked the whole area says
//! "All caught up".
//!
//! Keyboard: Up/Down moves the selection, Enter opens the selected pane's conversation (see [`key_message`] and
//! [`subscription`]).
//!
//! # Acting on a request
//!
//! The detail is the real approval or question card from `cards.rs`. Selecting an item points the app's selected
//! pane at that item's blocked pane (see [`sync_target`]), so the card, its screen read and its keys are the ones
//! the conversation uses. Every send goes through `actions::PaneActions` (installed only on the live path in
//! `main.rs`; tests use a recording fake). With a question card shown, Up/Down/Enter belong to the card.

use crate::state::{monogram, Screen};
use crate::{style, App, Message};
use fb_ui::icons::{icon, Icon};
use fb_ui::status;
use fb_ui::theme::*;
use fb_ui::StatusKind;
use herdr_core::{AgentStatus, ItemKind, PermState, PermissionRequest, QuestionRequest, QuestionState, Tab, Workspace};
use iced::widget::{button, column, container, responsive, row, scrollable, stack, text, Space};
use iced::{Alignment, Background, Border, Element, Length, Subscription, Task};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const LIST_W: f32 = 340.0;
const CARD_MAX_W: f32 = 640.0;
const REQUEST_TTL: Duration = Duration::from_secs(2);

/// What a blocked tab asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Permission(PermissionRequest),
    Question(QuestionRequest),
}

impl Request {
    /// One line for the list.
    pub fn one_liner(&self) -> String {
        match self {
            Request::Permission(p) => p.command.lines().next().unwrap_or("").trim().to_string(),
            Request::Question(q) => q.questions.first().map(|x| x.text.clone()).unwrap_or_else(|| "Asks a question".into()),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct Found {
    request: Option<Request>,
    timestamp: Option<f64>,
}

#[derive(Default)]
pub struct NeedsYouState {
    /// Selected tab id. `None` or a tab that is no longer blocked = the first item.
    pub selected: Option<String>,
    cache: RefCell<HashMap<String, (Instant, Found)>>,
}

#[derive(Debug, Clone)]
pub enum NeedsYouMsg {
    Select(String),
    /// Moves the selection by this many rows (clamped).
    Move(i32),
    /// Opens the selected item's conversation.
    OpenSelected,
}

/// One blocked tab with its workspace.
#[derive(Debug, Clone, Copy)]
pub struct Item<'a> {
    pub ws: &'a Workspace,
    pub tab: &'a Tab,
}

impl Item<'_> {
    pub fn agent_name(&self) -> String {
        self.tab.panes.iter().find_map(|p| p.agent.as_ref().map(|a| a.name.clone())).unwrap_or_else(|| self.tab.label.clone())
    }
    pub fn blocked_pane(&self) -> Option<&herdr_core::Pane> {
        self.tab.panes.iter().find(|p| p.agent.as_ref().is_some_and(|a| a.status == AgentStatus::Blocked)).or_else(|| self.tab.panes.iter().find(|p| p.agent.is_some()))
    }
}

/// The blocked tabs in workspace order. Its length is the sidebar badge count.
pub fn items(app: &App) -> Vec<Item<'_>> {
    app.state.blocked_tabs().into_iter().map(|(ws, tab)| Item { ws, tab }).collect()
}

/// Index of the selected item (the first when the stored id is gone).
pub fn selected_index(app: &App) -> Option<usize> {
    let list = items(app);
    if list.is_empty() {
        return None;
    }
    Some(app.state.needs_you.selected.as_deref().and_then(|id| list.iter().position(|i| i.tab.id == id)).unwrap_or(0))
}

fn find_request(app: &App, item: &Item) -> Found {
    let Some(pane) = item.blocked_pane() else { return Found::default() };
    let ny = &app.state.needs_you;
    if let Some((at, f)) = ny.cache.borrow().get(&pane.id) {
        if at.elapsed() < REQUEST_TTL {
            return f.clone();
        }
    }
    let found = app
        .source
        .conversation(pane)
        .into_iter()
        .rev()
        .find_map(|it| match it.kind {
            ItemKind::PermissionRequest(p) if p.state == PermState::Pending => Some(Found { request: Some(Request::Permission(p)), timestamp: it.timestamp }),
            ItemKind::Question(q) if q.state == QuestionState::Pending => Some(Found { request: Some(Request::Question(q)), timestamp: it.timestamp }),
            _ => None,
        })
        .unwrap_or_default();
    ny.cache.borrow_mut().insert(pane.id.clone(), (Instant::now(), found.clone()));
    found
}

/// Points the app's selected tab and pane at the selected item, so `cards` works on its pane. Returns true when the
/// selection changed. Called by `App::refresh` while the screen is shown, and after each list selection.
pub fn sync_target(app: &mut App) -> bool {
    let changed = retarget(app);
    if changed {
        app.refresh();
    }
    changed
}

/// The selection part of [`sync_target`], without the refresh.
pub fn retarget(app: &mut App) -> bool {
    if app.state.screen != Screen::NeedsYou {
        return false;
    }
    let Some(i) = selected_index(app) else { return false };
    let (ws, tab, pane) = {
        let it = items(app)[i];
        (it.ws.id.clone(), it.tab.id.clone(), it.blocked_pane().map(|p| p.id.clone()))
    };
    let st = &mut app.state;
    if st.selected_workspace.as_deref() == Some(ws.as_str()) && st.selected_tab.as_deref() == Some(tab.as_str()) && (pane.is_none() || st.selected_pane == pane) {
        return false;
    }
    st.selected_workspace = Some(ws);
    st.selected_tab = Some(tab);
    st.selected_pane = pane;
    true
}

pub fn update(app: &mut App, msg: NeedsYouMsg) -> Task<Message> {
    match msg {
        NeedsYouMsg::Select(id) => {
            app.state.needs_you.selected = Some(id);
            sync_target(app);
            Task::none()
        }
        NeedsYouMsg::Move(delta) => {
            let ids: Vec<String> = items(app).iter().map(|i| i.tab.id.clone()).collect();
            if let Some(cur) = selected_index(app) {
                let next = (cur as i64 + delta as i64).clamp(0, ids.len() as i64 - 1) as usize;
                app.state.needs_you.selected = Some(ids[next].clone());
            }
            sync_target(app);
            Task::none()
        }
        NeedsYouMsg::OpenSelected => {
            // A question card owns Enter (it sends the answer).
            if matches!(app.state.cards.card, Some(crate::cards::Card::Question { .. })) {
                return Task::none();
            }
            let Some(i) = selected_index(app) else { return Task::none() };
            let (ws, tab) = {
                let it = items(app)[i];
                (it.ws.id.clone(), it.tab.id.clone())
            };
            app.update(Message::SelectTab { workspace: ws, tab })
        }
    }
}

/// Maps a key to a message: Up/Down move, Enter opens. Pure, so tests can call it.
pub fn key_message(event: iced::keyboard::Event) -> Option<Message> {
    use iced::keyboard::{key::Named, Event, Key};
    let Event::KeyPressed { key, modifiers, .. } = event else { return None };
    if modifiers.command() || modifiers.alt() || modifiers.control() {
        return None;
    }
    match key.as_ref() {
        Key::Named(Named::ArrowDown) => Some(Message::NeedsYou(NeedsYouMsg::Move(1))),
        Key::Named(Named::ArrowUp) => Some(Message::NeedsYou(NeedsYouMsg::Move(-1))),
        Key::Named(Named::Enter) => Some(Message::NeedsYou(NeedsYouMsg::OpenSelected)),
        _ => None,
    }
}

/// Keys, only while the screen is shown.
pub fn subscription(app: &App) -> Subscription<Message> {
    if !app.state.settings.open && !app.state.shortcuts.open && app.state.screen == Screen::NeedsYou && !items(app).is_empty() && !matches!(app.state.cards.card, Some(crate::cards::Card::Question { .. })) {
        iced::keyboard::listen().filter_map(key_message)
    } else {
        Subscription::none()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Views

/// Under window glass the list column is a glass bar and the detail the content tint, so the screen paints its own
/// ground instead of the main area's one fill.
pub fn paints_own_ground(app: &App) -> bool {
    app.state.screen == Screen::NeedsYou && !items(app).is_empty()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let list = items(app);
    if list.is_empty() {
        return empty(app);
    }
    responsive(move |size| layout(app, size.width, size.height)).into()
}

fn layout(app: &App, width: f32, height: f32) -> Element<'_, Message> {
    let list = items(app);
    if list.is_empty() { return empty(app); }
    let cur = selected_index(app).unwrap_or(0);
    let t = app.tokens();
    if width < LIST_W + 380.0 {
        return column![container(list_column(app, &list, cur)).height(Length::Fixed((height * 0.3).clamp(80.0, 200.0))), detail(app, list[cur])].into();
    }
    let mut list_box = container(list_column(app, &list, cur)).width(Length::Fixed(LIST_W)).height(Length::Fill);
    let mut detail_box = container(detail(app, list[cur])).width(Length::Fill).height(Length::Fill);
    if app.state.glass.on {
        list_box = list_box.style(style::fill(t.glass_bar, 0.0));
        detail_box = detail_box.style(style::fill(app.ground(), 0.0));
    }
    row![list_box, container(Space::new().width(1).height(Length::Fill)).style(style::fill(t.sep, 0.0)), detail_box].into()
}

fn empty(app: &App) -> Element<'_, Message> {
    let t = app.tokens();
    container(
        column![
            status::symbol_sized(StatusKind::Done, &t, 34.0, 0.0),
            Space::new().height(6),
            TITLE_3.text("All caught up").color(t.label),
            BODY.text("No agent is waiting for you.").color(t.label_2),
        ]
        .align_x(Alignment::Center)
        .spacing(4),
    )
    .center(Length::Fill)
    .into()
}

fn avatar<'a>(t: Tokens, ws: &str, size: f32, badge: f32, selected: bool, ring: bool) -> Element<'a, Message> {
    let (bg, fg) = if selected { (t.on_accent.scale_alpha(0.22), t.on_accent) } else { (t.fill_3, t.label_2) };
    let circle = container(text(monogram(ws)).size(size * 0.32).font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(fg))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .center(Length::Fixed(size))
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { color: if ring { t.blocked } else { iced::Color::TRANSPARENT }, width: if ring { 2.0 } else { 0.0 }, radius: (size / 2.0).into() },
            ..Default::default()
        });
    let pad = 1.0;
    let b = container(status::symbol_sized(StatusKind::Blocked, &t, badge, 0.0)).padding(pad).style(style::fill(if selected { t.accent } else { t.bg }, 100.0));
    container(stack![circle, container(b).align_right(Length::Fill).align_bottom(Length::Fill)]).width(Length::Fixed(size + 4.0)).height(Length::Fixed(size + 4.0)).into()
}

fn list_column<'a>(app: &'a App, list: &[Item<'a>], cur: usize) -> Element<'a, Message> {
    let t = app.bar_tokens();
    let now = app.state.overview.clock().now;
    let head = row![TITLE_3.text("Needs you").color(t.label), TITLE_3.text(list.len().to_string()).color(t.label_3)].spacing(8).padding(iced::Padding { top: 20.0, right: 16.0, bottom: 8.0, left: 16.0 });
    let mut col = column![head];
    for (i, it) in list.iter().enumerate() {
        let sel = i == cur;
        let found = find_request(app, it);
        let (fg, fg2) = if sel { (t.on_accent, t.on_accent.scale_alpha(0.8)) } else { (t.label, t.label_2) };
        let age = found.timestamp.map(|ts| crate::overview::age_label(now, ts)).unwrap_or_default();
        let mut texts = column![
            row![HEADLINE.text(it.agent_name()).color(fg).width(Length::Fill), CAPTION.text(age).color(fg2)].align_y(Alignment::Center),
            CAPTION.text(format!("{} \u{b7} {}", it.ws.label, it.tab.label)).color(fg2),
        ]
        .spacing(1)
        .width(Length::Fill);
        if let Some(r) = &found.request {
            texts = texts.push(CAPTION.text(r.one_liner()).color(fg2).wrapping(iced::widget::text::Wrapping::None));
        }
        let content = row![avatar(t, &it.ws.label, 32.0, 11.0, sel, false), texts].spacing(10).align_y(Alignment::Center);
        col = col.push(
            container(
                button(content)
                    .padding([9, 12])
                    .width(Length::Fill)
                    .on_press(Message::NeedsYou(NeedsYouMsg::Select(it.tab.id.clone())))
                    .style(move |_, status| {
                        let hover = matches!(status, button::Status::Hovered);
                        let bg = if sel { Some(Background::Color(t.accent)) } else if hover { Some(Background::Color(t.fill_1)) } else { None };
                        button::Style { background: bg, text_color: fg, border: Border { radius: R_ROW.into(), ..Default::default() }, ..Default::default() }
                    }),
            )
            .padding([0, 8]),
        );
    }
    // Working tabs: quiet rows, one click opens the conversation.
    let working: Vec<(&Workspace, &Tab)> = app.state.workspaces.iter().flat_map(|w| w.tabs.iter().filter(|x| x.status == AgentStatus::Working).map(move |x| (w, x))).collect();
    if !working.is_empty() {
        col = col.push(container(CAPTION.text("WORKING \u{b7} NO ACTION NEEDED").color(t.label_3)).padding(iced::Padding { top: 18.0, right: 16.0, bottom: 4.0, left: 16.0 }));
        for (ws, tab) in working {
            let name = Item { ws, tab }.agent_name();
            col = col.push(
                button(row![status::symbol(StatusKind::Working, &t), column![BODY.text(name).color(t.label), CAPTION.text(tab.label.clone()).color(t.label_2)]].spacing(10).align_y(Alignment::Center))
                    .padding([4, 16])
                    .width(Length::Fill)
                    .on_press(Message::SelectTab { workspace: ws.id.clone(), tab: tab.id.clone() })
                    .style(move |_, _| button::Style { background: None, text_color: t.label, ..Default::default() }),
            );
        }
    }
    let resting = app.state.workspaces.iter().flat_map(|w| w.tabs.iter()).filter(|x| matches!(x.status, AgentStatus::Done | AgentStatus::Idle | AgentStatus::Unknown)).count();
    if resting > 0 {
        col = col.push(
            container(
                container(row![status::symbol(StatusKind::Done, &t), BODY.text(format!("{resting} agents resting")).color(t.label_2)].spacing(8).align_y(Alignment::Center))
                    .padding([10, 12])
                    .width(Length::Fill)
                    .style(style::fill(t.fill_1, R_CARD)),
            )
            .padding(iced::Padding { top: 12.0, right: 16.0, bottom: 16.0, left: 16.0 }),
        );
    }
    scrollable(col).height(Length::Fill).into()
}

fn detail<'a>(app: &'a App, it: Item<'a>) -> Element<'a, Message> {
    let t = app.tokens();
    let head = row![
        avatar(t, &it.ws.label, 40.0, 18.0, false, true),
        column![TITLE.text(it.agent_name()).color(t.label), BODY.text(format!("{} \u{b7} {} \u{b7} Needs you", it.ws.label, it.tab.label)).color(t.label_2)].spacing(3),
    ]
    .spacing(14)
    .align_y(Alignment::Center);
    let open = Message::SelectTab { workspace: it.ws.id.clone(), tab: it.tab.id.clone() };
    let body: Element<Message> = match crate::cards::detail(app) {
        Some(card) => column![card, row![plain_button(t, "Open tab", Some(Icon::SquareTerminal), Some(open), false)]].spacing(12).into(),
        None => column![BODY.text("The request is not on screen. Open the tab to see it.").color(t.label_2), plain_button(t, "Open tab", Some(Icon::SquareTerminal), Some(open), false)].spacing(12).into(),
    };
    let col = column![head, body].spacing(18).max_width(CARD_MAX_W + 40.0);
    scrollable(container(col).padding(iced::Padding { top: 44.0, right: 12.0, bottom: 16.0, left: 12.0 })).height(Length::Fill).width(Length::Fill).into()
}

fn plain_button<'a>(t: Tokens, label: &'a str, ic: Option<Icon>, on: Option<Message>, strong: bool) -> Element<'a, Message> {
    let off = on.is_none();
    let fg = if off { t.label_3 } else { t.label };
    let mut r = row![].spacing(8).align_y(Alignment::Center);
    if let Some(i) = ic {
        r = r.push(icon(i, 15.0, fg));
    }
    r = r.push(if strong { HEADLINE.text(label) } else { BODY.text(label) }.size(14).color(fg));
    let mut b = button(r).padding([6, 14]).style(move |_, status| {
        let bg = if matches!(status, button::Status::Hovered) { t.fill_3 } else { t.fill_2 };
        button::Style { background: Some(Background::Color(bg)), text_color: fg, border: Border { radius: 8.0.into(), ..Default::default() }, ..Default::default() }
    });
    if let Some(m) = on {
        b = b.on_press(m);
    }
    b.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{self, key::Named, Key, Modifiers};

    fn press(k: Named, m: Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed { key: Key::Named(k), modified_key: Key::Named(k), physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified), location: keyboard::Location::Standard, modifiers: m, text: None, repeat: false }
    }

    #[test]
    fn keys_map() {
        assert!(matches!(key_message(press(Named::ArrowDown, Modifiers::empty())), Some(Message::NeedsYou(NeedsYouMsg::Move(1)))));
        assert!(matches!(key_message(press(Named::ArrowUp, Modifiers::empty())), Some(Message::NeedsYou(NeedsYouMsg::Move(-1)))));
        assert!(matches!(key_message(press(Named::Enter, Modifiers::empty())), Some(Message::NeedsYou(NeedsYouMsg::OpenSelected))));
        assert!(key_message(press(Named::Enter, Modifiers::COMMAND)).is_none());
    }
}
