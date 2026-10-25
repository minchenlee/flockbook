//! Sidebar: New tab / Needs you / Overview / Search, then one row per workspace (DESIGN.md section 7).
//!
//! Skeleton version. The sidebar builder owns this file: hover states, the "+" workspace action, keyboard
//! navigation, tooltips, scrolling of long lists.

use crate::state::{monogram, status_kind, Screen, State};
use crate::{style, App, Message};
use fb_ui::icons::{icon, Icon, SIZE_DEFAULT};
use fb_ui::status::{self, StatusKind};
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
/// Settings and Keyboard Shortcuts, bottom left (Swift: the app menu and the Help menu).
fn footer(app: &App) -> Element<'_, Message> {
    let t = app.bar_tokens();
    let mac = cfg!(target_os = "macos");
    let item = |ic: Icon, tip: String, msg: Message| -> Element<'_, Message> {
        let b = button(container(icon(ic, SIZE_DEFAULT, t.label_2)).center(Length::Fixed(28.0))).padding(0).on_press(msg).style(move |_, st| button::Style {
            background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_2)),
            border: Border { radius: R_ROW.into(), ..Default::default() },
            ..Default::default()
        });
        iced::widget::tooltip(b, container(CALLOUT.text(tip).color(t.label)).padding([5, 9]).style(style::float_e2(app.tokens(), R_ROW)), iced::widget::tooltip::Position::Top).gap(6).into()
    };
    let key = |a| crate::shortcuts::TABLE.iter().find(|s| s.action == Some(a)).map(|s| crate::shortcuts::keys_text(s, mac)).unwrap_or_default();
    row![
        item(Icon::Settings2, format!("Settings ({})", key(crate::shortcuts::Action::Settings)), Message::Settings(crate::settings::SettingsMsg::Toggle)),
        item(Icon::Keyboard, format!("Keyboard shortcuts ({})", key(crate::shortcuts::Action::Shortcuts)), Message::Shortcut(crate::shortcuts::Action::Shortcuts)),
    ]
    .spacing(SP_4)
    .padding([SP_8, 0.0])
    .into()
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

/// Status summary, right-aligned, in the fixed order of the spec: done/idle dots (max 4, then +n), working arc
/// and count, blocked badge.
fn summary<'a>(t: Tokens, spin: f32, ws: &herdr_core::Workspace) -> Element<'a, Message> {
    let c = State::counts(ws);
    let mut r = row![].spacing(4).align_y(Alignment::Center);
    let mut dots: Vec<StatusKind> = Vec::new();
    dots.extend(std::iter::repeat(StatusKind::Done).take(c.done));
    dots.extend(std::iter::repeat(StatusKind::Idle).take(c.idle));
    for k in dots.iter().take(4) {
        r = r.push(status::dot(*k, &t));
    }
    if dots.len() > 4 {
        r = r.push(CAPTION.text(format!("+{}", dots.len() - 4)).color(t.label_3));
    }
    if c.working > 0 {
        r = r.push(Space::new().width(2)).push(status::symbol_sized(StatusKind::Working, &t, 11.0, spin)).push(CAPTION.text(c.working.to_string()).color(t.label_2));
    }
    if c.blocked > 0 {
        r = r.push(Space::new().width(2)).push(count_badge(t, c.blocked));
    }
    r.into()
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

    let header = row![
        CAPTION.text("Workspaces").font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(t.label_2),
        Space::new().width(Length::Fill),
        button(icon(Icon::Plus, SIZE_DEFAULT, t.label_2)).padding(2).style(button::text).on_press(Message::Sidebar(SidebarMsg::AddWorkspace)),
    ]
    .align_y(Alignment::Center)
    .padding([0, 8]);

    let mut list = column![].spacing(2);
    for ws in &s.workspaces {
        let selected = s.screen == Screen::Conversation && s.selected_workspace.as_deref() == Some(ws.id.as_str());
        let c = State::counts(ws);
        let quiet = c.blocked == 0 && c.working == 0;
        let name_color = if quiet && !selected { t.label_2 } else { t.label };
        let name = if c.blocked > 0 { HEADLINE.text(ws.label.clone()) } else { BODY.text(ws.label.clone()) };
        let content = row![monogram_chip(t, &ws.label, selected), name.color(name_color), Space::new().width(Length::Fill), summary(t, s.toprow.spin, ws)]
            .spacing(SP_8)
            .align_y(Alignment::Center);
        let tip = format!("{} \u{00B7} {} tab{}{}{}", ws.label, ws.tabs.len(), if ws.tabs.len() == 1 { "" } else { "s" }, if c.working > 0 { format!(" \u{00B7} {} working", c.working) } else { String::new() }, if c.blocked > 0 { format!(" \u{00B7} {} need you", c.blocked) } else { String::new() });
        let rowb = mouse_area(row_button(t, selected, content.into(), Message::SelectWorkspace(ws.id.clone()), WORKSPACE_ROW_H))
            .on_right_press(Message::Workspace(crate::workspace_actions::WorkspaceMsg::Menu(crate::workspace_actions::Target::Workspace(ws.id.clone()))));
        let rowb = crate::toprow::drag_target(rowb.into(), crate::workspace_actions::Target::Workspace(ws.id.clone()));
        list = list.push(iced::widget::tooltip(rowb, container(CALLOUT.text(tip).color(t.label)).padding([5, 9]).style(style::float_e2(t, R_ROW)), iced::widget::tooltip::Position::Right).gap(6).delay(std::time::Duration::from_millis(600)));
    }

    // Keep the status mapping in one place for builders.
    let _ = status_kind;

    container(column![crate::chrome::drag_area(app, Space::new().width(Length::Fill).height(52)), nav, Space::new().height(SP_24), header, Space::new().height(SP_4), scrollable(list).height(Length::Fill), footer(app)])
        .padding([0, 12])
        .height(Length::Fill)
        .into()
}
