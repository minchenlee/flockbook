//! Sidebar: New tab / Needs you / Overview / Search, then one row per workspace (DESIGN.md section 7).
//!
//! Skeleton version. The sidebar builder owns this file: hover states, the "+" workspace action, keyboard
//! navigation, tooltips, scrolling of long lists.

use crate::state::{monogram, status_kind, Screen, State};
use crate::{style, App, Message};
use fb_ui::icons::{icon, Icon, SIZE_DEFAULT};
use fb_ui::status;
use fb_ui::theme::*;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, Space};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};

#[derive(Debug, Clone, Default)]
pub struct SidebarState {}

#[derive(Debug, Clone)]
pub enum SidebarMsg {
    NewTab,
    AddWorkspace,
    /// Cmd+Alt+Down / Up: the next / previous workspace (wraps).
    NextWorkspace,
    PrevWorkspace,
}

pub fn update(app: &mut App, msg: SidebarMsg) -> Task<Message> {
    match msg {
        SidebarMsg::NewTab => crate::workspace_actions::update(app, crate::workspace_actions::WorkspaceMsg::NewTab),
        SidebarMsg::AddWorkspace => crate::workspace_actions::update(app, crate::workspace_actions::WorkspaceMsg::Edit(crate::workspace_actions::Editor::CreateWorkspace)),
        SidebarMsg::NextWorkspace => step_workspace(app, 1),
        SidebarMsg::PrevWorkspace => step_workspace(app, -1),
    }
}

fn step_workspace(app: &mut App, delta: isize) -> Task<Message> {
    let n = app.state.workspaces.len() as isize;
    if n == 0 {
        return Task::none();
    }
    let cur = app.state.selected_workspace.as_deref().and_then(|id| app.state.workspaces.iter().position(|w| w.id == id)).map(|i| i as isize).unwrap_or(if delta > 0 { -1 } else { 0 });
    let id = app.state.workspaces[(cur + delta).rem_euclid(n) as usize].id.clone();
    app.update(Message::SelectWorkspace(id))
}

/// A full-width sidebar row button with the selected/hover background.
/// Settings, pinned at the foot of the sidebar (Swift SidebarView.swift:59-67: one `navRow`
/// "Settings" with the gearshape symbol). Keyboard shortcuts have no row there: they stay on ⌘/.
fn footer(app: &App) -> Element<'_, Message> {
    let t = app.bar_tokens();
    let mac = cfg!(target_os = "macos");
    let key = |a| crate::shortcuts::TABLE.iter().find(|s| s.action == Some(a)).map(|s| crate::shortcuts::keys_text(s, mac)).unwrap_or_default();
    let tip = format!("Settings ({})", key(crate::shortcuts::Action::Settings));
    // The symbol sits in a 22 pt box, like the nav rows and the avatars (Swift SidebarLine.box).
    let content = row![
        container(crate::settings::gear(SIZE_DEFAULT, t.label_2)).center(Length::Fixed(22.0)),
        BODY.text("Settings").color(t.label),
        Space::new().width(Length::Fill),
    ]
    .spacing(SP_8)
    .align_y(Alignment::Center);
    let b = row_button(t, app.state.settings.open, content.into(), Message::Settings(crate::settings::SettingsMsg::Toggle), SIDEBAR_ROW_H + 4.0);
    let b = iced::widget::tooltip(b, container(CALLOUT.text(tip).color(t.label)).padding([5, 9]).style(style::float_e2(app.tokens(), R_ROW)), iced::widget::tooltip::Position::Top).gap(6);
    container(b).padding([SP_8, 0.0]).into()
}

fn row_button<'a>(t: Tokens, selected: bool, content: Element<'a, Message>, on_press: Message, height: f32) -> Element<'a, Message> {
    button(container(content).height(Length::Fill).align_y(Alignment::Center).padding([0, 8]))
        .on_press(on_press)
        .width(Length::Fill)
        .height(Length::Fixed(height))
        .padding(0)
        .style(move |_, status| {
            let bg = if selected {
                Some(Background::Color(t.sidebar_sel))
            } else if matches!(status, button::Status::Hovered) {
                Some(Background::Color(t.fill_1))
            } else {
                None
            };
            button::Style { background: bg, text_color: t.label, border: Border { radius: R_ROW.into(), ..Default::default() }, ..Default::default() }
        })
        .into()
}

fn nav_row<'a>(app: &App, ic: Icon, label: &'a str, screen: Option<Screen>, badge: usize, msg: Message) -> Element<'a, Message> {
    let t = app.bar_tokens();
    let selected = screen.is_some_and(|s| s == app.state.screen);
    // The symbol sits in a box as wide as a workspace avatar, at the same spacing: nav titles and
    // workspace names share one leading edge (Swift SidebarLine.box / spacing 8).
    let sym = container(icon(ic, SIZE_DEFAULT, t.label_2)).center(Length::Fixed(22.0));
    let mut r = row![sym, BODY.text(label).color(t.label), Space::new().width(Length::Fill)].spacing(SP_8).align_y(Alignment::Center);
    if badge > 0 {
        r = r.push(count_badge(t, badge));
    }
    row_button(t, selected, r.into(), msg, SIDEBAR_ROW_H + 4.0)
}

/// The orange blocked badge with a count.
pub fn count_badge<'a, M: 'a>(t: Tokens, n: usize) -> Element<'a, M> {
    container(CAPTION.text(n.to_string()).color(t.on_accent).font(fb_ui::fonts::inter(iced::font::Weight::Bold)))
        .padding([1, 7])
        .style(style::fill(t.blocked, 100.0))
        .into()
}

fn monogram_chip<'a>(t: Tokens, label: &str, selected: bool) -> Element<'a, Message> {
    // primary 0.1 chip with primary 0.7 letters; inverted when selected (Swift Monogram).
    let (bg, fg) = if selected { (Color { a: 0.8, ..t.label }, t.bg) } else { (Color { a: 0.1, ..t.label }, Color { a: 0.7, ..t.label }) };
    container(text(monogram(label)).size(10).font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(fg))
        .width(Length::Fixed(22.0))
        .height(Length::Fixed(22.0))
        .center(Length::Fixed(22.0))
        .style(style::fill(bg, 6.0))   // Radius.control
        .into()
}

/// Per-tab status glyphs at the end of a workspace row (Swift StatusColumn, SidebarView.swift:189-221):
/// one glyph per tab in tab order, 12 pt with 3 pt gaps, in one right-aligned column as wide as the
/// busiest workspace needs (at most five slots), so the glyphs line up and the name gets the rest.
const STATUS_GLYPH: f32 = 12.0;
const STATUS_SPACING: f32 = 3.0;
const STATUS_SLOTS: usize = 5;

/// Slots of the shared status column: the busiest workspace, at most five.
fn status_slots(ws: &[herdr_core::Workspace]) -> usize {
    ws.iter().map(|w| w.tabs.len()).max().unwrap_or(0).min(STATUS_SLOTS)
}

fn status_column_width(slots: usize) -> f32 {
    slots as f32 * STATUS_GLYPH + slots.saturating_sub(1) as f32 * STATUS_SPACING
}

/// More tabs than slots: the most urgent (blocked, working, done, idle; earlier tab first on a tie) and
/// "+N" in the last two slots (Swift StatusColumn.shown).
fn status_shown(tabs: &[herdr_core::Tab]) -> (Vec<&herdr_core::Tab>, usize) {
    if tabs.len() <= STATUS_SLOTS {
        return (tabs.iter().collect(), 0);
    }
    fn urgency(s: herdr_core::AgentStatus) -> u8 {
        match s {
            herdr_core::AgentStatus::Blocked => 4,
            herdr_core::AgentStatus::Working => 3,
            herdr_core::AgentStatus::Done => 2,
            herdr_core::AgentStatus::Idle => 1,
            herdr_core::AgentStatus::Unknown => 0,
        }
    }
    let keep = STATUS_SLOTS - 2;
    let mut order: Vec<usize> = (0..tabs.len()).collect();
    order.sort_by(|a, b| urgency(tabs[*b].status).cmp(&urgency(tabs[*a].status)).then(a.cmp(b)));
    order.truncate(keep);
    (tabs.iter().enumerate().filter(|(i, _)| order.contains(i)).map(|(_, tab)| tab).collect(), tabs.len() - keep)
}

fn status_column<'a>(t: Tokens, spin: f32, tabs: &[herdr_core::Tab], slots: usize) -> Element<'a, Message> {
    let (shown, more) = status_shown(tabs);
    let mut r = row![].spacing(STATUS_SPACING).align_y(Alignment::Center);
    for tab in shown {
        let phase = if tab.status == herdr_core::AgentStatus::Working { spin } else { 0.0 };
        r = r.push(status::symbol_sized(status_kind(tab.status), &t, STATUS_GLYPH, phase));
    }
    if more > 0 {
        r = r.push(container(CAPTION.text(format!("+{more}")).color(t.label_3)).width(Length::Fixed(status_column_width(2))).align_x(Alignment::End));
    }
    r.width(Length::Fixed(status_column_width(slots))).into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let t = app.bar_tokens();
    let s = &app.state;
    let blocked = s.blocked_tabs().len();

    let nav = column![
        nav_row(app, Icon::SquarePen, "New tab", None, 0, Message::Sidebar(SidebarMsg::NewTab)),
        nav_row(app, Icon::Inbox, "Needs you", Some(Screen::NeedsYou), blocked, Message::Navigate(Screen::NeedsYou)),
        nav_row(app, Icon::LayoutGrid, "Overview", Some(Screen::Overview), 0, Message::Navigate(Screen::Overview)),
        nav_row(app, Icon::Search, "Search", Some(Screen::Search), 0, Message::Search(crate::search::SearchMsg::Open)),
    ]
    .spacing(2);

    // "Workspaces" subheadline/semibold (Swift SidebarView.swift:127) with the folder.badge.plus button
    // (SidebarView.swift:132); the closest Lucide pair is folder + plus, the plus reading as the badge.
    let header = row![
        BODY.text("Workspaces").font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(t.label_2),
        Space::new().width(Length::Fill),
        button(row![icon(Icon::Folder, SIZE_DEFAULT, t.label_2), icon(Icon::Plus, 11.0, t.label_2)].spacing(1).align_y(Alignment::Center))
            .padding(2)
            .style(button::text)
            .on_press(Message::Sidebar(SidebarMsg::AddWorkspace)),
    ]
    .align_y(Alignment::Center)
    .padding([6, 8]);

    let slots = status_slots(&s.workspaces);
    let mut list = column![].spacing(2);
    for ws in &s.workspaces {
        let selected = s.screen == Screen::Conversation && s.selected_workspace.as_deref() == Some(ws.id.as_str());
        let c = State::counts(ws);
        let quiet = c.blocked == 0 && c.working == 0;
        let name_color = if quiet && !selected { t.label_2 } else { t.label };
        let name = if c.blocked > 0 { HEADLINE.text(ws.label.clone()) } else { BODY.text(ws.label.clone()) };
        let content = row![monogram_chip(t, &ws.label, selected), name.color(name_color), Space::new().width(Length::Fill), status_column(t, s.toprow.spin, &ws.tabs, slots)]
            .spacing(SP_8)
            .align_y(Alignment::Center);
        let tip = format!("{} \u{00B7} {} tab{}{}{}", ws.label, ws.tabs.len(), if ws.tabs.len() == 1 { "" } else { "s" }, if c.working > 0 { format!(" \u{00B7} {} working", c.working) } else { String::new() }, if c.blocked > 0 { format!(" \u{00B7} {} need you", c.blocked) } else { String::new() });
        let rowb = mouse_area(row_button(t, selected, content.into(), Message::SelectWorkspace(ws.id.clone()), WORKSPACE_ROW_H))
            .on_right_press(Message::Workspace(crate::workspace_actions::WorkspaceMsg::Menu(crate::workspace_actions::Target::Workspace(ws.id.clone()))));
        let rowb = crate::toprow::drag_target(rowb.into(), crate::workspace_actions::Target::Workspace(ws.id.clone()));
        list = list.push(iced::widget::tooltip(rowb, container(CALLOUT.text(tip).color(t.label)).padding([5, 9]).style(style::float_e2(t, R_ROW)), iced::widget::tooltip::Position::Right).gap(6).delay(std::time::Duration::from_millis(600)));
    }

    container(column![crate::chrome::drag_area(app, Space::new().width(Length::Fill).height(52)), nav, Space::new().height(SP_24), header, Space::new().height(SP_4), scrollable(list).height(Length::Fill), footer(app)])
        .padding([0, 12])
        .height(Length::Fill)
        .into()
}
