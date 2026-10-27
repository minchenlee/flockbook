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

use crate::state::Screen;
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
const CARD_MAX_W: f32 = 560.0;
/// The centred content column of the full-width screens (Swift `ScreenColumn.width`).
const SCREEN_COL_W: f32 = 960.0;
const REQUEST_TTL: Duration = Duration::from_secs(2);

/// What a blocked tab asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Permission(PermissionRequest),
    Question(QuestionRequest),
}

impl Request {
    /// One line for the list (kept for the request summary; the list rows show two lines like the Mac app).
    #[allow(dead_code)]
    pub fn one_liner(&self) -> String {
        match self {
            Request::Permission(p) => p.command.lines().next().unwrap_or("").trim().to_string(),
            Request::Question(q) => q.questions.first().map(|x| x.text.clone()).unwrap_or_else(|| "Asks a question".into()),
        }
    }
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
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

#[allow(dead_code)]
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

/// The agent's role symbol, from the last part of its name (Swift `role(for:)`, StatusViews.swift:129-138).
fn role_icon(agent: &str) -> Icon {
    match agent.rsplit('-').next().unwrap_or("") {
        "orch" => Icon::Sparkles,
        "dev" => Icon::Code,
        "rev" => Icon::Eye,
        "res" => Icon::BookOpen,
        "des" => Icon::PenTool,
        _ => Icon::CircleUserRound,
    }
}

/// The role symbol in a circle with the blocked glyph at its foot (Swift InboxView: list 30 pt, symbol at body size,
/// white 0.22 when selected else primary 0.08, the glyph on a window-coloured disc; detail 56 pt, symbol at title size,
/// an orange 2 pt ring, the glyph 18 pt).
fn avatar<'a>(t: Tokens, agent: &str, size: f32, badge: f32, selected: bool, ring: bool) -> Element<'a, Message> {
    let (bg, fg) = if selected { (t.on_accent.scale_alpha(0.22), t.on_accent) } else { (t.label.scale_alpha(0.08), t.label) };
    let glyph = if ring { 24.0 } else { 15.0 };
    let circle = container(icon(role_icon(agent), glyph, fg))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .center(Length::Fixed(size))
        .style(move |_| container::Style {
            background: Some(Background::Color(bg)),
            border: Border { color: if ring { t.blocked } else { iced::Color::TRANSPARENT }, width: if ring { 2.0 } else { 0.0 }, radius: (size / 2.0).into() },
            ..Default::default()
        });
    let b: Element<'a, Message> = if ring {
        status::symbol_sized(StatusKind::Blocked, &t, badge, 0.0)
    } else {
        container(status::symbol_sized(StatusKind::Blocked, &t, badge, 0.0)).padding(1).style(style::fill(if selected { t.accent } else { t.bg }, 100.0)).into()
    };
    container(stack![circle, container(b).align_right(Length::Fill).align_bottom(Length::Fill)]).width(Length::Fixed(size + 4.0)).height(Length::Fixed(size + 4.0)).into()
}

fn list_column<'a>(app: &'a App, list: &[Item<'a>], cur: usize) -> Element<'a, Message> {
    let t = app.bar_tokens();
    // title3 semibold (15 pt), the count at body size, tertiary (Screens.swift:31-35).
    let head = row![text("Needs you").size(15).font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(t.label), BODY.text(list.len().to_string()).color(t.label_3)]
        .spacing(8)
        .align_y(Alignment::Center)
        .padding(iced::Padding { top: 44.0, right: 16.0, bottom: 8.0, left: 16.0 });
    let mut col = column![head];
    for (i, it) in list.iter().enumerate() {
        let sel = i == cur;
        let (fg, fg2) = if sel { (t.on_accent, t.on_accent.scale_alpha(0.8)) } else { (t.label, t.label_2) };
        let texts = column![
            HEADLINE.text(it.agent_name()).color(fg).width(Length::Fill),
            CAPTION.text(format!("{} \u{b7} {}", it.ws.label, it.tab.label)).color(fg2),
        ]
        .spacing(1)
        .width(Length::Fill);
        let content = row![avatar(t, &it.agent_name(), 30.0, 13.0, sel, false), texts].spacing(10).align_y(Alignment::Center);
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
        // caption2 semibold, tertiary (Screens.swift:60).
        col = col.push(container(text("WORKING \u{b7} NO ACTION NEEDED").size(10).font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(t.label_3)).padding(iced::Padding { top: 18.0, right: 16.0, bottom: 4.0, left: 16.0 }));
        for (ws, tab) in working {
            let name = Item { ws, tab }.agent_name();
            col = col.push(
                button(row![status::symbol_live(StatusKind::Working, &t, 14.0, app.state.toprow.animate), column![BODY.text(name).size(12.0).color(t.label), CAPTION.text(tab.label.clone()).color(t.label_2)]].spacing(10).align_y(Alignment::Center))
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
                // Callout for the glyph and the text (Screens.swift:76-80).
                container(row![status::symbol_sized(StatusKind::Done, &t, 13.0, 0.0), BODY.text(format!("{resting} agents resting")).size(12.0).color(t.label_2)].spacing(8).align_y(Alignment::Center))
                    .padding([10, 12])
                    .width(Length::Fill)
                    .style(style::fill(t.fill_1, R_CARD)),
            )
            .padding(iced::Padding { top: 12.0, right: 16.0, bottom: 0.0, left: 16.0 }),
        );
    }
    scrollable(col).height(Length::Fill).into()
}

fn detail<'a>(app: &'a App, it: Item<'a>) -> Element<'a, Message> {
    let t = app.tokens();
    let head = row![
        avatar(t, &it.agent_name(), 56.0, 18.0, false, true),
        column![TITLE.text(it.agent_name()).color(t.label), BODY.text(format!("{} \u{b7} {} \u{b7} Needs you", it.ws.label, it.tab.label)).color(t.label_2)].spacing(3),
    ]
    .spacing(14)
    .align_y(Alignment::Center);
    let open = Message::SelectTab { workspace: it.ws.id.clone(), tab: it.tab.id.clone() };
    // The card alone when there is one; Open tab only with the fallback text (Screens.swift:120-129).
    let body: Element<Message> = match crate::cards::detail(app) {
        Some(card) => container(card).max_width(CARD_MAX_W).into(),
        None => column![BODY.text("The request text is not available from the session log. Open the tab to see it.").color(t.label_2), row![prominent_button(t, "Open tab", open)]].spacing(18).into(),
    };
    let col = column![head, body].spacing(18);
    // At most 960 wide and centred in the detail area, like the Swift `screenColumn`.
    let column = container(col).padding(iced::Padding { top: 48.0, right: 32.0, bottom: 32.0, left: 32.0 }).max_width(SCREEN_COL_W);
    scrollable(container(column).width(Length::Fill).center_x(Length::Fill)).height(Length::Fill).width(Length::Fill).into()
}

/// A `.borderedProminent` push button at the regular size: body text, white on the accent, 24 pt tall.
fn prominent_button<'a>(t: Tokens, label: &'a str, on: Message) -> Element<'a, Message> {
    button(BODY.text(label).color(t.on_accent))
        .padding(iced::Padding { top: 3.5, bottom: 3.5, left: 14.0, right: 14.0 })
        .on_press(on)
        .style(move |_, status| {
            let bg = if matches!(status, button::Status::Pressed) { iced::Color { a: 0.85, ..t.accent } } else { t.accent };
            button::Style { background: Some(Background::Color(bg)), text_color: t.on_accent, border: Border { radius: R_ROW.into(), ..Default::default() }, ..Default::default() }
        })
        .into()
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
