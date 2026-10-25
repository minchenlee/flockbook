//! Top row: one 52 pt row with the tab strip on the left and the actions on the right (DESIGN.md section 7).
//!
//! Left to right: a horizontally scrolling strip of pill tabs, the pinned All tabs and + buttons, flexible space,
//! then the actions (Review diffstat when there is data, Split menu, Show terminal).
//!
//! The strip is a custom widget (`Shift`): it lays its content out at full width, moves it left by the scroll
//! offset and clips it. Hit tests are masked to the strip bounds, so a half-hidden tab never steals a click from
//! the pinned buttons. The offset lives in `TopRowState` behind a `Cell`: `view` knows the real width and
//! corrects the offset (clamp, and scroll the selected tab into view), `update` adds wheel deltas.
//!
//! Keys (`on_key`, wired by `subscription`): Ctrl+Tab / Ctrl+Shift+Tab next/previous tab, Cmd+1..9 select a tab,
//! Cmd+Alt+Down/Up next/previous workspace, Ctrl+` toggles the terminal, Esc closes a menu. "Cmd" is the logo key
//! on macOS and Ctrl elsewhere (`Modifiers::command`).

use crate::state::{status_kind, State};
use crate::{style, App, Message};
use fb_ui::fonts::is_cjk_char;
use fb_ui::icons::{icon, Icon, SIZE_DEFAULT, SIZE_INLINE};
use fb_ui::status;
use fb_ui::theme::*;
use herdr_core::{AgentStatus, Tab};
use iced::advanced::{layout, mouse, overlay, renderer, widget, Clipboard, Layout, Renderer as _, Shell, Widget};
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::{button, column, container, mouse_area, row, stack, text, tooltip, Space};
use iced::{gradient, time, Alignment, Background, Border, Color, Element, Event, Length, Padding, Point, Radians, Rectangle, Size, Subscription, Task, Vector};
use std::cell::{Cell, RefCell};
use std::time::Duration;

/// Gap between two tabs (holds the hairline divider).
const GAP: f32 = 4.0;
const MIN_TAB_W: f32 = 72.0;
const MAX_TAB_W: f32 = 200.0;
const FADE_L: f32 = 36.0;
const FADE_R: f32 = 44.0;
const BTN: f32 = 30.0;
/// Horizontal room the right-hand actions need: split (44) + terminal (30) + gap.
const ACTIONS_W: f32 = 44.0 + BTN + SP_4;
/// Window width assumed before the first `view` has measured the real one.
const DEFAULT_STRIP_W: f32 = 480.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    AllTabs,
    Split,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    Right,
    Down,
}

#[derive(Debug, Clone, Default)]
pub struct TopRowState {
    /// Scroll offset of the strip, in points. Corrected by `view`.
    pub offset: Cell<f32>,
    /// Visible width of the strip, measured by the last `view`.
    pub strip_w: Cell<f32>,
    /// Full width of all tabs, measured by the last `view`.
    pub content_w: Cell<f32>,
    /// (workspace, tab) the strip last scrolled into view. A different selection triggers a reveal.
    seen: RefCell<Option<(String, String)>>,
    /// The open menu, if any.
    pub menu: Option<MenuKind>,
    /// Working-arc rotation in radians, advanced by `TopRowMsg::Spin`. The sidebar reads it too.
    pub spin: f32,
    /// "Show terminal" pressed state. `terminal.rs` sets it when the sheet opens or closes.
    pub terminal_on: bool,
    pub dragging: Option<(String, Point)>,
}

#[derive(Debug, Clone)]
pub enum TopRowMsg {
    Plus,
    DragStart { tab: String, at: Point },
    Drop { target: crate::workspace_actions::Target, at: Point },
    NewTab,
    ToggleTerminal,
    CloseTab(String),
    /// Wheel / trackpad delta over the strip, in points (positive scrolls the strip to later tabs).
    Scroll(f32),
    /// Click on a blocked edge marker: bring the nearest hidden blocked tab on that side into view.
    RevealBlocked(Edge),
    NextTab,
    PrevTab,
    /// 0-based index into the selected workspace's tabs (Cmd+1..9).
    SelectIndex(usize),
    ToggleMenu(MenuKind),
    CloseMenu,
    /// Pick a tab in the All tabs menu.
    PickTab { workspace: String, tab: String },
    ShowOverview,
    Split(SplitDir),
    /// Advances the working-arc animation.
    Spin,
}

pub fn update(app: &mut App, msg: TopRowMsg) -> Task<Message> {
    match msg {
        TopRowMsg::DragStart { tab, at } => {
            app.state.toprow.dragging = Some((tab, at));
            Task::none()
        }
        TopRowMsg::Drop { target, at } => {
            let Some((id, start)) = app.state.toprow.dragging.take() else { return Task::none() };
            if (at.x - start.x).hypot(at.y - start.y) < 6.0 { return Task::none() }
            let destination = match target {
                crate::workspace_actions::Target::Tab(tab) if tab != id => app.state.workspaces.iter().find_map(|w| w.tabs.iter().position(|t| t.id == tab).map(|i| (w.id.clone(), i))),
                crate::workspace_actions::Target::Workspace(ws) => app.state.workspace(&ws).map(|w| (ws, w.tabs.len())),
                _ => None,
            };
            let Some((ws, index)) = destination else { return Task::none() };
            let same = app.state.workspaces.iter().any(|w| w.id == ws && w.tabs.iter().any(|t| t.id == id));
            crate::workspace_actions::update(app, crate::workspace_actions::WorkspaceMsg::Run(herdr_core::client::WorkspaceAction::MoveTab { id, workspace: (!same).then_some(ws), index }))
        }
        TopRowMsg::NewTab => {
            app.state.toprow.menu = None;
            crate::workspace_actions::update(app, crate::workspace_actions::WorkspaceMsg::NewTab)
        }
        TopRowMsg::Plus => {
            app.state.toprow.menu = None;
            crate::workspace_actions::update(app, crate::workspace_actions::WorkspaceMsg::Plus)
        }
        TopRowMsg::CloseTab(id) => {
            app.state.toprow.menu = None;
            crate::workspace_actions::update(app, crate::workspace_actions::WorkspaceMsg::Run(herdr_core::client::WorkspaceAction::CloseTab { id }))
        }
        TopRowMsg::Split(dir) => {
            app.state.toprow.menu = None;
            let Some(pane) = app.state.selected_pane() else { return Task::none() };
            crate::workspace_actions::update(app, crate::workspace_actions::WorkspaceMsg::Run(herdr_core::client::WorkspaceAction::Split { pane: pane.id.clone(), down: dir == SplitDir::Down }))
        }
        TopRowMsg::ToggleTerminal => {
            // The sheet is the state: the button shows it. Without a terminal backend nothing opens.
            crate::terminal::toggle(app);
            Task::none()
        }
        TopRowMsg::Scroll(dx) => {
            let s = &app.state.toprow;
            let max = (s.content_w.get() - s.strip_w.get()).max(0.0);
            s.offset.set((s.offset.get() + dx).clamp(0.0, max));
            Task::none()
        }
        TopRowMsg::RevealBlocked(edge) => {
            reveal_blocked(app, edge);
            Task::none()
        }
        TopRowMsg::NextTab => step_tab(app, 1),
        TopRowMsg::PrevTab => step_tab(app, -1),
        TopRowMsg::SelectIndex(i) => {
            let pick = app.state.selected_workspace().and_then(|ws| ws.tabs.get(i).map(|t| (ws.id.clone(), t.id.clone())));
            match pick {
                Some((workspace, tab)) => app.update(Message::SelectTab { workspace, tab }),
                None => Task::none(),
            }
        }
        TopRowMsg::ToggleMenu(kind) => {
            let s = &mut app.state.toprow;
            s.menu = if s.menu == Some(kind) { None } else { Some(kind) };
            Task::none()
        }
        TopRowMsg::CloseMenu => {
            app.state.toprow.dragging = None;
            app.state.toprow.menu = None;
            if crate::workspace_actions::modal(app) {
                return crate::workspace_actions::update(app, crate::workspace_actions::WorkspaceMsg::Dismiss);
            }
            Task::none()
        }
        TopRowMsg::PickTab { workspace, tab } => {
            app.state.toprow.menu = None;
            app.update(Message::SelectTab { workspace, tab })
        }
        TopRowMsg::ShowOverview => {
            app.state.toprow.menu = None;
            app.update(Message::Navigate(crate::state::Screen::Overview))
        }
        TopRowMsg::Spin => {
            let s = &mut app.state.toprow;
            s.spin = (s.spin + 0.42) % std::f32::consts::TAU;
            Task::none()
        }
    }
}

fn step_tab(app: &mut App, delta: isize) -> Task<Message> {
    let pick = app.state.selected_workspace().and_then(|ws| {
        let n = ws.tabs.len() as isize;
        if n == 0 {
            return None;
        }
        let cur = app.state.selected_tab.as_deref().and_then(|id| ws.tabs.iter().position(|t| t.id == id)).map(|i| i as isize).unwrap_or(if delta > 0 { -1 } else { 0 });
        let next = (cur + delta).rem_euclid(n) as usize;
        Some((ws.id.clone(), ws.tabs[next].id.clone()))
    });
    match pick {
        Some((workspace, tab)) => app.update(Message::SelectTab { workspace, tab }),
        None => Task::none(),
    }
}

/// Keyboard shortcuts of the top row and the sidebar. Pure, so tests can call it directly.
pub fn on_key(key: Key, mods: Modifiers) -> Option<Message> {
    use crate::sidebar::SidebarMsg;
    match key.as_ref() {
        Key::Named(Named::Tab) if mods.control() && !mods.alt() => Some(Message::TopRow(if mods.shift() { TopRowMsg::PrevTab } else { TopRowMsg::NextTab })),
        Key::Named(Named::ArrowDown) if mods.command() && mods.alt() => Some(Message::Sidebar(SidebarMsg::NextWorkspace)),
        Key::Named(Named::ArrowUp) if mods.command() && mods.alt() => Some(Message::Sidebar(SidebarMsg::PrevWorkspace)),
        Key::Named(Named::Escape) => Some(Message::TopRow(TopRowMsg::CloseMenu)),
        // Only digits: an early return here would also swallow Ctrl+` on Linux and Windows, where Ctrl is the
        // command key.
        Key::Character(c) if mods.command() && !mods.alt() && !mods.shift() && tab_digit(c).is_some() => {
            tab_digit(c).map(|n| Message::TopRow(TopRowMsg::SelectIndex(n - 1)))
        }
        Key::Character("`") if mods.control() => Some(Message::TopRow(TopRowMsg::ToggleTerminal)),
        _ => None,
    }
}

/// "1" to "9" as 1 to 9.
fn tab_digit(c: &str) -> Option<usize> {
    let mut chars = c.chars();
    let n = chars.next()?.to_digit(10)? as usize;
    (chars.next().is_none() && (1..=9).contains(&n)).then_some(n)
}

/// Subscriptions of this module: keys, and the working-arc tick while any tab works.
pub fn subscription(app: &App) -> Subscription<Message> {
    let keys = keyboard::listen().filter_map(|e| match e {
        keyboard::Event::KeyPressed { key, modifiers, .. } => on_key(key, modifiers),
        _ => None,
    });
    let working = app.state.workspaces.iter().any(|w| w.tabs.iter().any(|t| t.status == AgentStatus::Working));
    if working {
        Subscription::batch([keys, time::every(Duration::from_millis(60)).map(|_| Message::TopRow(TopRowMsg::Spin))])
    } else {
        keys
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tab geometry

fn text_w(s: &str, bold: bool) -> f32 {
    let w: f32 = s
        .chars()
        .map(|c| match c {
            c if is_cjk_char(c) => 13.0,
            'i' | 'l' | 'j' | 't' | 'f' | 'r' | 'I' | '.' | ',' | '\'' | ';' | ':' | ' ' | '!' | '|' | '-' => 4.4,
            'm' | 'w' | 'M' | 'W' => 10.6,
            c if c.is_uppercase() => 8.2,
            _ => 6.9,
        })
        .sum();
    if bold {
        w * 1.04
    } else {
        w
    }
}

/// Fixed chrome around the label: padding, symbol, gaps, split glyph, close button.
fn chrome(is_split: bool, selected: bool) -> f32 {
    10.0 + 14.0 + 6.0 + if is_split { 6.0 + 12.0 } else { 0.0 } + if selected { 28.0 } else { 10.0 }
}

struct TabGeom {
    width: f32,
    /// Label after truncation.
    label: String,
    bold: bool,
}

/// Width of a tab in the strip (public for tests and for the menu placement).
pub fn tab_width(tab: &Tab, selected: bool) -> f32 {
    tab_geom(tab, selected).width
}

fn tab_geom(tab: &Tab, selected: bool) -> TabGeom {
    let blocked = tab.status == AgentStatus::Blocked;
    let bold = selected || blocked;
    let split = tab.panes.len() > 1;
    let ch = chrome(split, selected);
    let natural = ch + text_w(&tab.label, bold);
    if blocked || natural <= MAX_TAB_W {
        return TabGeom { width: natural.max(MIN_TAB_W), label: tab.label.clone(), bold };
    }
    // Truncate with an ellipsis until it fits in the maximum width.
    let room = MAX_TAB_W - ch - text_w("\u{2026}", bold);
    let mut label = String::new();
    for c in tab.label.chars() {
        if text_w(&label, bold) + text_w(&c.to_string(), bold) > room {
            break;
        }
        label.push(c);
    }
    label.push('\u{2026}');
    TabGeom { width: MAX_TAB_W, label, bold }
}

/// (x, geometry) of every tab of the selected workspace.
fn strip_layout<'a>(tabs: &'a [Tab], selected: Option<&str>) -> (Vec<(f32, TabGeom)>, f32) {
    let mut x = 0.0;
    let mut out = Vec::with_capacity(tabs.len());
    for tab in tabs {
        let g = tab_geom(tab, selected == Some(tab.id.as_str()));
        let w = g.width;
        out.push((x, g));
        x += w + GAP;
    }
    let total = (x - GAP).max(0.0);
    (out, total)
}

fn reveal_blocked(app: &mut App, edge: Edge) {
    let Some(ws) = app.state.selected_workspace() else { return };
    let (lay, total) = strip_layout(&ws.tabs, app.state.selected_tab.as_deref());
    let s = &app.state.toprow;
    let (off, w) = (s.offset.get(), s.strip_w.get());
    let target = match edge {
        Edge::Left => ws.tabs.iter().zip(&lay).filter(|(t, (x, g))| t.status == AgentStatus::Blocked && x + g.width <= off + 1.0).last(),
        Edge::Right => ws.tabs.iter().zip(&lay).find(|(t, (x, _))| t.status == AgentStatus::Blocked && *x >= off + w - 1.0),
    };
    if let Some((_, (x, g))) = target {
        let max = (total - w).max(0.0);
        s.offset.set((x + g.width / 2.0 - w / 2.0).clamp(0.0, max));
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Shift widget: content moved left by `offset`, clipped, with masked hit testing.

struct Shift<'a> {
    content: Element<'a, Message>,
    offset: f32,
    width: f32,
    height: f32,
}

impl<'a> Widget<Message, iced::Theme, iced::Renderer> for Shift<'a> {
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }
    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }
    fn children(&self) -> Vec<widget::Tree> {
        self.content.as_widget().children()
    }
    fn diff(&self, tree: &mut widget::Tree) {
        self.content.as_widget().diff(tree);
    }
    fn size(&self) -> Size<Length> {
        Size { width: Length::Fixed(self.width), height: Length::Fixed(self.height) }
    }
    fn layout(&mut self, tree: &mut widget::Tree, renderer: &iced::Renderer, _limits: &layout::Limits) -> layout::Node {
        let inner = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, self.height));
        let node = self.content.as_widget_mut().layout(tree, renderer, &inner).move_to(Point::new(-self.offset, 0.0));
        layout::Node::with_children(Size::new(self.width, self.height), vec![node])
    }
    fn operate(&mut self, tree: &mut widget::Tree, layout: Layout<'_>, renderer: &iced::Renderer, op: &mut dyn widget::Operation) {
        self.content.as_widget_mut().operate(tree, layout.children().next().unwrap(), renderer, op);
    }
    fn update(&mut self, tree: &mut widget::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &iced::Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        let bounds = layout.bounds();
        let cursor = if cursor.is_over(bounds) { cursor } else { mouse::Cursor::Unavailable };
        let Some(vp) = bounds.intersection(viewport) else { return };
        self.content.as_widget_mut().update(tree, event, layout.children().next().unwrap(), cursor, renderer, clipboard, shell, &vp);
    }
    fn mouse_interaction(&self, tree: &widget::Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &iced::Renderer) -> mouse::Interaction {
        let bounds = layout.bounds();
        if !cursor.is_over(bounds) {
            return mouse::Interaction::None;
        }
        let Some(vp) = bounds.intersection(viewport) else { return mouse::Interaction::None };
        self.content.as_widget().mouse_interaction(tree, layout.children().next().unwrap(), cursor, &vp, renderer)
    }
    fn draw(&self, tree: &widget::Tree, renderer: &mut iced::Renderer, theme: &iced::Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        let bounds = layout.bounds();
        let Some(vp) = bounds.intersection(viewport) else { return };
        let cursor = if cursor.is_over(bounds) { cursor } else { mouse::Cursor::Unavailable };
        renderer.with_layer(bounds, |renderer| {
            self.content.as_widget().draw(tree, renderer, theme, style, layout.children().next().unwrap(), cursor, &vp);
        });
    }
    fn overlay<'b>(&'b mut self, tree: &'b mut widget::Tree, layout: Layout<'b>, renderer: &iced::Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(tree, layout.children().next().unwrap(), renderer, viewport, translation)
    }
}

impl<'a> From<Shift<'a>> for Element<'a, Message> {
    fn from(s: Shift<'a>) -> Self {
        Element::new(s)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Views

fn icon_button<'a>(t: Tokens, ic: Icon, active: bool, msg: Message) -> Element<'a, Message> {
    button(container(icon(ic, SIZE_DEFAULT, if active { t.label } else { t.label_2 })).center(Length::Fixed(BTN)))
        .padding(0)
        .on_press(msg)
        .style(move |_, st| button::Style {
            background: (active || matches!(st, button::Status::Hovered)).then_some(Background::Color(t.fill_2)),
            border: Border { radius: R_ROW.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

fn menu_button<'a>(t: Tokens, ic: Icon, active: bool, msg: Message) -> Element<'a, Message> {
    button(container(row![icon(ic, SIZE_DEFAULT, t.label_2), icon(Icon::ChevronDown, SIZE_INLINE - 4.0, t.label_3)].spacing(2).align_y(Alignment::Center)).center_y(Length::Fixed(BTN)).padding([0, 8]))
        .padding(0)
        .on_press(msg)
        .style(move |_, st| button::Style {
            background: (active || matches!(st, button::Status::Hovered)).then_some(Background::Color(t.fill_2)),
            border: Border { radius: R_ROW.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

/// Tooltip text of a tab: agent, state, tab number, pane count (the model has no timestamps yet, so no age).
pub fn tab_tooltip(tab: &Tab, position: usize) -> String {
    let agent = tab.panes.iter().find_map(|p| p.agent.as_ref());
    let state = match tab.status {
        AgentStatus::Working => "Working",
        AgentStatus::Blocked => "Needs you",
        AgentStatus::Done => "Done",
        AgentStatus::Idle | AgentStatus::Unknown => "Idle",
    };
    let panes = format!("{} pane{}", tab.panes.len(), if tab.panes.len() == 1 { "" } else { "s" });
    let mut parts: Vec<String> = Vec::new();
    if let Some(a) = agent {
        parts.push(a.name.clone());
    }
    parts.push(state.to_string());
    parts.push(format!("Tab {position} \u{00B7} {panes}"));
    parts.join(" \u{00B7} ")
}

fn tip<'a>(t: Tokens, content: Element<'a, Message>, label: String, pos: tooltip::Position) -> Element<'a, Message> {
    tooltip(content, container(CALLOUT.text(label).color(t.label)).padding([5, 9]).style(style::float_e2(t, R_ROW)), pos).gap(6).delay(Duration::from_millis(450)).into()
}

fn tab_pill<'a>(t: Tokens, ws_id: &str, tab: &'a Tab, g: &TabGeom, selected: bool, position: usize, spin: f32) -> Element<'a, Message> {
    let blocked = tab.status == AgentStatus::Blocked;
    let split = tab.panes.len() > 1;
    let kind = status_kind(tab.status);
    let sym: Element<'a, Message> = if kind == fb_ui::StatusKind::Working { status::symbol_sized(kind, &t, 14.0, spin) } else { status::symbol(kind, &t) };
    let label = if g.bold { HEADLINE.text(g.label.clone()) } else { BODY.text(g.label.clone()) };
    let label = label.color(if selected || blocked { t.label } else { t.label_2 }).wrapping(text::Wrapping::None);
    let mut inner = row![sym, label].spacing(6).align_y(Alignment::Center);
    if split {
        inner = inner.push(icon(Icon::Rows2, 12.0, t.label_3));
    }
    let main = button(container(inner).height(Length::Fill).align_y(Alignment::Center).padding(Padding { left: 10.0, right: if selected { 2.0 } else { 10.0 }, top: 0.0, bottom: 0.0 }))
        .on_press(Message::SelectTab { workspace: ws_id.to_string(), tab: tab.id.clone() })
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(0)
        .style(move |_, st| button::Style {
            background: (!selected && matches!(st, button::Status::Hovered)).then_some(Background::Color(t.fill_1)),
            border: Border { radius: TAB_RADIUS.into(), ..Default::default() },
            ..Default::default()
        });
    let mut r = row![main].align_y(Alignment::Center);
    if selected {
        r = r.push(
            button(container(icon(Icon::X, SIZE_INLINE - 2.0, t.label_3)).center(Length::Fixed(22.0)))
                .padding(0)
                .on_press(Message::TopRow(TopRowMsg::CloseTab(tab.id.clone())))
                .style(move |_, st| button::Style {
                    background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_2)),
                    border: Border { radius: 6.0.into(), ..Default::default() },
                    ..Default::default()
                }),
        )
        .push(Space::new().width(4));
    }
    let pill = container(r).width(Length::Fixed(g.width)).height(Length::Fixed(TAB_H));
    let pill: Element<'a, Message> = if selected { pill.style(style::card_e1(t, TAB_RADIUS)).into() } else { pill.into() };
    let pill = mouse_area(pill)
        .on_right_press(Message::Workspace(crate::workspace_actions::WorkspaceMsg::Menu(crate::workspace_actions::Target::Tab(tab.id.clone()))));
    let pill = drag_target(pill.into(), crate::workspace_actions::Target::Tab(tab.id.clone()));
    tip(t, pill, tab_tooltip(tab, position), tooltip::Position::Bottom)
}

fn divider<'a>(t: Tokens, show: bool) -> Element<'a, Message> {
    let line: Element<'a, Message> = if show { container(Space::new()).width(1).height(14).style(style::fill(t.sep_strong, 0.0)).into() } else { Space::new().into() };
    container(line).width(Length::Fixed(GAP)).height(Length::Fixed(TAB_H)).center(Length::Fixed(GAP)).into()
}

fn fade<'a>(t: Tokens, left: bool) -> Element<'a, Message> {
    let (from, to) = (t.bg, Color { a: 0.0, ..t.bg });
    // Angle PI/2 runs left to right in iced's gradient space.
    let (a, b) = if left { (from, to) } else { (to, from) };
    let g = gradient::Linear::new(Radians(std::f32::consts::FRAC_PI_2)).add_stop(0.0, a).add_stop(1.0, b);
    container(Space::new()).width(Length::Fixed(if left { FADE_L } else { FADE_R })).height(Length::Fill).style(move |_| container::Style { background: Some(Background::Gradient(g.into())), ..Default::default() }).into()
}

/// Opaque blend of `over` onto `base`.
fn mix(base: Color, over: Color, a: f32) -> Color {
    Color::from_rgb(base.r + (over.r - base.r) * a, base.g + (over.g - base.g) * a, base.b + (over.b - base.b) * a)
}

fn marker<'a>(t: Tokens, edge: Edge, n: usize) -> Element<'a, Message> {
    let count = row![CAPTION.text("!").color(t.blocked).font(fb_ui::fonts::inter(iced::font::Weight::Bold)), CAPTION.text(n.to_string()).color(t.blocked).font(fb_ui::fonts::inter(iced::font::Weight::Bold))].spacing(3).align_y(Alignment::Center);
    let pill = button(container(count).padding([2, 8]))
        .padding(0)
        .on_press(Message::TopRow(TopRowMsg::RevealBlocked(edge)))
        .style(move |_, st| button::Style {
            background: Some(Background::Color(if matches!(st, button::Status::Hovered) { mix(t.bg, t.blocked, 0.30) } else { mix(t.bg, t.blocked, 0.18) })),
            border: Border { radius: 100.0.into(), ..Default::default() },
            ..Default::default()
        });
    tip(t, pill.into(), format!("{n} tab{} out of view need{} you: click to show", if n == 1 { "" } else { "s" }, if n == 1 { "s" } else { "" }), tooltip::Position::Bottom)
}

/// Review diffstat of the selected tab as (added, removed), only when the working tree has changes.
/// Hook for the Review feature; the model has no git data yet, so it is always `None`.
pub fn review_diffstat(_state: &State) -> Option<(usize, usize)> {
    None
}

/// (visible strip width, available width) for a top row of `width` points.
fn strip_metrics(s: &State, width: f32, total: f32) -> (f32, f32) {
    let review_w = if review_diffstat(s).is_some() { 96.0 } else { 0.0 };
    let avail = (width - 2.0 * SP_16 - 2.0 * BTN - 2.0 * SP_4 - ACTIONS_W - review_w - SP_16).max(DEFAULT_STRIP_W.min(width * 0.4));
    (total.min(avail), avail)
}

pub fn view(app: &App) -> Element<'_, Message> {
    iced::widget::responsive(move |size| view_sized(app, size.width)).into()
}

fn view_sized(app: &App, width: f32) -> Element<'_, Message> {
    let t = app.bar_tokens();
    let s = &app.state;
    let ts = &s.toprow;
    let tabs: &[Tab] = s.selected_workspace().map(|w| w.tabs.as_slice()).unwrap_or(&[]);
    let ws_id = s.selected_workspace.clone().unwrap_or_default();
    let (lay, total) = strip_layout(tabs, s.selected_tab.as_deref());

    let (strip_w, _) = strip_metrics(s, width, total);
    let max_off = (total - strip_w).max(0.0);
    let review = review_diffstat(s);

    // Offset: reset on a workspace change, reveal the selected tab when it is not fully visible, clamp.
    let key = s.selected_workspace.clone().zip(s.selected_tab.clone());
    let mut off = ts.offset.get();
    if *ts.seen.borrow() != key {
        if ts.seen.borrow().as_ref().map(|k| &k.0) != key.as_ref().map(|k| &k.0) {
            off = 0.0;
        }
        if let Some(i) = s.selected_tab.as_deref().and_then(|id| tabs.iter().position(|t| t.id == id)) {
            let (x, g) = (lay[i].0, &lay[i].1);
            if x < off || x + g.width > off + strip_w {
                off = x + g.width / 2.0 - strip_w / 2.0;
            }
        }
        *ts.seen.borrow_mut() = key;
    }
    let off = off.clamp(0.0, max_off);
    ts.offset.set(off);
    ts.strip_w.set(strip_w);
    ts.content_w.set(total);

    // Tabs row with hairline dividers between neighbours that are not selected.
    let mut r = row![].align_y(Alignment::Center);
    for (i, (tab, (_, g))) in tabs.iter().zip(&lay).enumerate() {
        let sel = s.selected_tab.as_deref() == Some(tab.id.as_str());
        if i > 0 {
            let prev_sel = s.selected_tab.as_deref() == Some(tabs[i - 1].id.as_str());
            r = r.push(divider(t, !sel && !prev_sel));
        }
        r = r.push(tab_pill(t, &ws_id, tab, g, sel, i + 1, ts.spin));
    }
    let shifted: Element<Message> = Shift { content: r.into(), offset: off, width: strip_w, height: TAB_H }.into();
    let strip = mouse_area(shifted).on_scroll(|d| {
        let (x, y) = match d {
            mouse::ScrollDelta::Lines { x, y } => (x * 40.0, y * 40.0),
            mouse::ScrollDelta::Pixels { x, y } => (x, y),
        };
        // Horizontal wheel or trackpad swipe, else the vertical wheel (Shift + wheel arrives as x on most systems).
        Message::TopRow(TopRowMsg::Scroll(-(if x.abs() > 0.0 { x } else { y }) ))
    });

    // Fades on clipped edges, then the blocked edge markers above them.
    let clipped_l = off > 0.5;
    let clipped_r = off < max_off - 0.5;
    let hidden_l = tabs.iter().zip(&lay).filter(|(t, (x, g))| t.status == AgentStatus::Blocked && x + g.width <= off + 1.0).count();
    let hidden_r = tabs.iter().zip(&lay).filter(|(t, (x, _))| t.status == AgentStatus::Blocked && *x >= off + strip_w - 1.0).count();
    let mut layers: Vec<Element<Message>> = vec![strip.into()];
    if clipped_l {
        layers.push(container(fade(t, true)).width(Length::Fill).align_x(Alignment::Start).into());
    }
    if clipped_r {
        layers.push(container(fade(t, false)).width(Length::Fill).align_x(Alignment::End).into());
    }
    if hidden_l > 0 {
        layers.push(container(marker(t, Edge::Left, hidden_l)).width(Length::Fill).height(Length::Fill).align_x(Alignment::Start).align_y(Alignment::Center).padding([0, 4]).into());
    }
    if hidden_r > 0 {
        layers.push(container(marker(t, Edge::Right, hidden_r)).width(Length::Fill).height(Length::Fill).align_x(Alignment::End).align_y(Alignment::Center).padding([0, 4]).into());
    }
    let strip_box = container(stack(layers)).width(Length::Fixed(strip_w)).height(Length::Fixed(TAB_H));

    let all_tabs = tip(t, icon_button(t, Icon::ChevronDown, ts.menu == Some(MenuKind::AllTabs), Message::TopRow(TopRowMsg::ToggleMenu(MenuKind::AllTabs))), "All tabs".into(), tooltip::Position::Bottom);
    let new_tab = tip(t, icon_button(t, Icon::Plus, false, Message::TopRow(TopRowMsg::Plus)), "New tab (\u{2318}T)".into(), tooltip::Position::Bottom);
    let left = row![strip_box, all_tabs, new_tab].spacing(SP_4).align_y(Alignment::Center);

    let mut right = row![].spacing(SP_4).align_y(Alignment::Center);
    if let Some((add, del)) = review {
        right = right.push(
            row![icon(Icon::FileDiff, SIZE_INLINE, t.label_2), CAPTION.text(format!("+{add}")).color(t.diff_add), CAPTION.text(format!("\u{2212}{del}")).color(t.diff_del)]
                .spacing(4)
                .align_y(Alignment::Center),
        );
    }
    right = right
        .push(tip(t, menu_button(t, Icon::Columns2, ts.menu == Some(MenuKind::Split), Message::TopRow(TopRowMsg::ToggleMenu(MenuKind::Split))), "Split".into(), tooltip::Position::Bottom))
        .push(tip(t, icon_button(t, Icon::SquareTerminal, ts.terminal_on, Message::TopRow(TopRowMsg::ToggleTerminal)), "Show terminal (\u{2303}`)".into(), tooltip::Position::Bottom));

    let gap = crate::chrome::drag_area(app, Space::new().width(Length::Fill).height(Length::Fixed(TOPROW_H)));
    container(row![left, gap, right].align_y(Alignment::Center).padding([0.0, SP_16]))
        .height(Length::Fixed(TOPROW_H))
        .width(Length::Fill)
        .align_y(Alignment::Center)
        .into()
}

// ---------------------------------------------------------------------------------------------------------------
// Menus (drawn as a layer above the whole window; `App::view` stacks `menu_layer`).

fn menu_item<'a>(t: Tokens, lead: Element<'a, Message>, label: String, hint: Option<String>, msg: Message) -> Element<'a, Message> {
    let mut r = row![lead, BODY.text(label).color(t.label).wrapping(text::Wrapping::None), Space::new().width(Length::Fill)].spacing(SP_8).align_y(Alignment::Center);
    if let Some(h) = hint {
        r = r.push(CAPTION.text(h).color(t.label_3));
    }
    button(container(r).padding([0, 8]).height(Length::Fill).align_y(Alignment::Center))
        .on_press(msg)
        .width(Length::Fill)
        .height(Length::Fixed(SIDEBAR_ROW_H))
        .padding(0)
        .style(move |_, st| button::Style {
            background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.accent_soft)),
            text_color: t.label,
            border: Border { radius: R_ROW.into(), ..Default::default() },
            ..Default::default()
        })
        .into()
}

/// The open menu above everything, or an empty element. Clicking outside closes it.
pub fn menu_layer(app: &App) -> Element<'_, Message> {
    if app.state.toprow.menu.is_none() {
        return Space::new().into();
    }
    iced::widget::responsive(move |size| menu_sized(app, size.width - SIDEBAR_W)).into()
}

fn menu_sized(app: &App, row_width: f32) -> Element<'_, Message> {
    let Some(kind) = app.state.toprow.menu else { return Space::new().into() };
    let strip_w = {
        let tabs: &[Tab] = app.state.selected_workspace().map(|w| w.tabs.as_slice()).unwrap_or(&[]);
        let (_, total) = strip_layout(tabs, app.state.selected_tab.as_deref());
        strip_metrics(&app.state, row_width, total).0
    };
    let t = app.tokens();
    let s = &app.state;
    let (card, w): (Element<Message>, f32) = match kind {
        MenuKind::AllTabs => {
            let mut col = column![].spacing(2);
            if let Some(ws) = s.selected_workspace() {
                for (i, tab) in ws.tabs.iter().enumerate() {
                    let cur = s.selected_tab.as_deref() == Some(tab.id.as_str());
                    let check: Element<Message> = if cur { icon(Icon::Check, SIZE_INLINE, t.label) } else { Space::new().width(SIZE_INLINE).into() };
                    let lead = row![check, status::symbol(status_kind(tab.status), &t)].spacing(SP_8).align_y(Alignment::Center);
                    let hint = (i < 9).then(|| format!("\u{2318}{}", i + 1));
                    col = col.push(menu_item(t, lead.into(), { let mut l: String = tab.label.chars().take(26).collect(); if tab.label.chars().count() > 26 { l.push('\u{2026}'); } l }, hint, Message::TopRow(TopRowMsg::PickTab { workspace: ws.id.clone(), tab: tab.id.clone() })));
                }
            }
            col = col.push(container(Space::new()).width(Length::Fill).height(1).style(style::fill(t.sep, 0.0)));
            col = col.push(menu_item(t, icon(Icon::LayoutGrid, SIZE_INLINE, t.label_2), "Show in Overview".into(), Some("\u{21E7}\u{2318}\\".into()), Message::TopRow(TopRowMsg::ShowOverview)));
            (col.into(), 280.0)
        }
        MenuKind::Split => (
            column![
                menu_item(t, icon(Icon::Columns2, SIZE_INLINE, t.label_2), "Split right".into(), Some("\u{2318}D".into()), Message::TopRow(TopRowMsg::Split(SplitDir::Right))),
                menu_item(t, icon(Icon::Rows2, SIZE_INLINE, t.label_2), "Split down".into(), Some("\u{21E7}\u{2318}D".into()), Message::TopRow(TopRowMsg::Split(SplitDir::Down))),
            ]
            .spacing(2)
            .into(),
            200.0,
        ),
    };
    let card = container(card).width(Length::Fixed(w)).padding(6).style(style::float_e2(t, R_MENU));
    let (ax, pad) = match kind {
        MenuKind::AllTabs => (Alignment::Start, Padding { top: TOPROW_H - 8.0, left: (SIDEBAR_W + SP_16 + strip_w + SP_4).min(SIDEBAR_W + row_width - w - SP_8), right: 0.0, bottom: 0.0 }),
        MenuKind::Split => (Alignment::End, Padding { top: TOPROW_H - 8.0, left: 0.0, right: SP_16 + BTN + SP_4, bottom: 0.0 }),
    };
    let placed = container(card).width(Length::Fill).height(Length::Fill).align_x(ax).padding(pad);
    stack![mouse_area(Space::new().width(Length::Fill).height(Length::Fill)).on_press(Message::TopRow(TopRowMsg::CloseMenu)), placed].into()
}

/// Observe pointer boundaries before child buttons capture them. This is also used for workspace drop targets.
pub fn drag_target(content: Element<'_, Message>, target: crate::workspace_actions::Target) -> Element<'_, Message> {
    Element::new(DragTarget { content, target })
}
struct DragTarget<'a> { content: Element<'a, Message>, target: crate::workspace_actions::Target }
impl Widget<Message, iced::Theme, iced::Renderer> for DragTarget<'_> {
    fn tag(&self) -> widget::tree::Tag { self.content.as_widget().tag() }
    fn state(&self) -> widget::tree::State { self.content.as_widget().state() }
    fn children(&self) -> Vec<widget::Tree> { self.content.as_widget().children() }
    fn diff(&self, tree: &mut widget::Tree) { self.content.as_widget().diff(tree); }
    fn size(&self) -> Size<Length> { self.content.as_widget().size() }
    fn layout(&mut self, tree: &mut widget::Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }
    fn operate(&mut self, tree: &mut widget::Tree, layout: Layout<'_>, renderer: &iced::Renderer, op: &mut dyn widget::Operation) {
        self.content.as_widget_mut().operate(tree, layout, renderer, op);
    }
    fn update(&mut self, tree: &mut widget::Tree, event: &Event, layout: Layout<'_>, cursor: mouse::Cursor, renderer: &iced::Renderer, clipboard: &mut dyn Clipboard, shell: &mut Shell<'_, Message>, viewport: &Rectangle) {
        let over = layout.bounds().intersection(viewport).is_some_and(|r| cursor.is_over(r));
        if over {
            if let Some(at) = cursor.position() {
                match event {
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                        if let crate::workspace_actions::Target::Tab(tab) = &self.target { shell.publish(Message::TopRow(TopRowMsg::DragStart { tab: tab.clone(), at })); }
                    }
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                        shell.publish(Message::TopRow(TopRowMsg::Drop { target: self.target.clone(), at }));
                    }
                    _ => {}
                }
            }
        }
        self.content.as_widget_mut().update(tree, event, layout, cursor, renderer, clipboard, shell, viewport);
    }
    fn mouse_interaction(&self, tree: &widget::Tree, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle, renderer: &iced::Renderer) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
    fn draw(&self, tree: &widget::Tree, renderer: &mut iced::Renderer, theme: &iced::Theme, style: &renderer::Style, layout: Layout<'_>, cursor: mouse::Cursor, viewport: &Rectangle) {
        self.content.as_widget().draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
    fn overlay<'b>(&'b mut self, tree: &'b mut widget::Tree, layout: Layout<'b>, renderer: &iced::Renderer, viewport: &Rectangle, translation: Vector) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(tree, layout, renderer, viewport, translation)
    }
}
