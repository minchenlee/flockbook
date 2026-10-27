//! Top row: one 52 pt row holding the centred tab island and, right after it, the action capsule (DESIGN.md section 7).
//!
//! Centre: a horizontally scrolling strip of pill tabs in a glass island, then `ACTION_GAP` and a small glass capsule
//! with the + and Show terminal buttons (Swift TopRow + TopRowActions). The All tabs and Split menus have no button
//! there, as on the Mac; they open from their keys and hang off that capsule. The empty space on either side is the
//! window drag area.
//!
//! The strip is a custom widget (`Shift`): it lays its content out at full width, moves it left by the scroll
//! offset and clips it. Hit tests are masked to the strip bounds, so a half-hidden tab never steals a click from
//! the pinned buttons. The offset lives in `TopRowState` behind a `Cell`: `view` knows the real width and
//! corrects the offset (clamp, and scroll the selected tab into view), `update` adds wheel deltas.
//!
//! Keys (`on_key`, wired by `subscription`): Ctrl+Tab / Ctrl+Shift+Tab next/previous tab, Cmd+1..9 select a tab,
//! Cmd+Alt+Down/Up next/previous workspace, Ctrl+` toggles the terminal, Esc closes a menu. "Cmd" is the logo key
//! on macOS and Ctrl elsewhere (`Modifiers::command`).

use crate::state::{status_kind, Screen, State};
use crate::{style, App, Message};
use fb_ui::icons::{icon, Icon, SIZE_INLINE};
use fb_ui::status;
use fb_ui::theme::*;
use herdr_core::{AgentStatus, Tab};
use iced::advanced::{layout, mouse, overlay, renderer, widget, Clipboard, Layout, Renderer as _, Shell, Widget};
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::{button, column, container, mouse_area, row, stack, text, tooltip, Space};
use iced::font::Weight;
use iced::{gradient, Alignment, Background, Border, Color, Element, Event, Length, Padding, Point, Radians, Rectangle, Size, Subscription, Task, Vector};
use std::cell::{Cell, RefCell};
use std::time::Duration;

/// Gap between two tabs (Swift TopRow.pillSpacing).
pub const GAP: f32 = 2.0;
const MIN_TAB_W: f32 = 72.0;
const MAX_TAB_W: f32 = 200.0;
const FADE_L: f32 = 36.0;
const FADE_R: f32 = 44.0;
/// Pills are capsules and the island insets them by 3 on every side (Swift TabPill.height / TopRow.inset).
const PILL_R: f32 = TAB_H / 2.0;
const ISLAND_INSET: f32 = 3.0;
/// Height of the action capsule next to the island: the island's height (Swift TopRow.height, 36).
const CAPSULE_H: f32 = TAB_H + 2.0 * ISLAND_INSET;
/// One action button inside that capsule (Swift FullScreenTopRow buttons: 36 wide, 36 tall).
const ACTION_BTN: f32 = 36.0;
/// The buttons' own inset. None: with no capsule around them they sit as the Mac toolbar's do (the full-screen copy's
/// `.padding(.horizontal, 2)` belongs to its glass capsule).
const CAPSULE_PAD: f32 = 0.0;
/// Space between the island and the action capsule (Swift FullScreenTopRow `HStack(spacing: 10)`).
const ACTION_GAP: f32 = 10.0;
/// The action icons: the Mac's 15 pt medium symbols draw a 14 pt plus and a 20 pt terminal with a thin stroke. Lucide's
/// stroke grows with the box, so 20 pt is the closest match in both size and weight.
const ACTION_ICON: f32 = 20.0;
/// Width of the action buttons: + and terminal.
const CAPSULE_W: f32 = 2.0 * ACTION_BTN + 2.0 * CAPSULE_PAD;
/// Pill label: Swift TabPill `.callout`, medium, semibold when selected or blocked (TopRow.swift:213). Callout is 12 pt.
const PILL_BODY: TextStyle = TextStyle { size: 12.0, line: 1.3, weight: Weight::Medium, mono: false };
const PILL_BOLD: TextStyle = TextStyle { size: 12.0, line: 1.3, weight: Weight::Semibold, mono: false };
/// The pill's status glyph (Swift TabPill: `StatusGlyph(size: 13)`).
const PILL_GLYPH: f32 = 13.0;
/// The slot a still status symbol takes: SF Symbols at 13 pt are about 16 pt wide with the glyph centred (measured on
/// the Mac's screenshot); the working arc keeps its 13 pt frame (StatusGlyph `SpinnerArc ... .frame(width: size)`).
const PILL_SLOT_STILL: f32 = 16.0;

fn glyph_slot(status: AgentStatus) -> f32 {
    if status == AgentStatus::Working { PILL_GLYPH } else { PILL_SLOT_STILL }
}
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
    /// Working arcs turn (a real window, set by `main.rs`). Off in snapshots and tests: the arcs stand still. The
    /// sidebar and Needs you read it too.
    pub animate: bool,
    /// The window is not the key window (another app is in front, or it is minimized): the arcs turn at about 10 fps.
    pub background: bool,
    /// "Show terminal" pressed state. `terminal.rs` sets it when the sheet opens or closes.
    pub terminal_on: bool,
    pub dragging: Option<(String, Point)>,
}

impl TopRowState {
    /// Time between frames of a working arc: `None` while the arcs stand still.
    pub fn spin_frame(&self) -> Option<std::time::Duration> {
        self.animate.then_some(if self.background { status::SPIN_FRAME_BACKGROUND } else { status::SPIN_FRAME })
    }
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
    /// The window became the key window (true) or lost it (false).
    WindowFocus(bool),
    ToggleMenu(MenuKind),
    CloseMenu,
    /// Pick a tab in the All tabs menu.
    PickTab { workspace: String, tab: String },
    ShowOverview,
    Split(SplitDir),
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
        TopRowMsg::WindowFocus(focused) => {
            app.state.toprow.background = !focused;
            Task::none()
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

/// Subscriptions of this module: keys. Working arcs turn by themselves (`status::symbol_live`), with no tick.
pub fn subscription(_app: &App) -> Subscription<Message> {
    let keys = keyboard::listen().filter_map(|e| match e {
        keyboard::Event::KeyPressed { key, modifiers, .. } => on_key(key, modifiers),
        _ => None,
    });
    let focus = iced::event::listen_with(|e, _, _| match e {
        iced::Event::Window(iced::window::Event::Focused) => Some(Message::TopRow(TopRowMsg::WindowFocus(true))),
        iced::Event::Window(iced::window::Event::Unfocused) => Some(Message::TopRow(TopRowMsg::WindowFocus(false))),
        _ => None,
    });
    Subscription::batch([keys, focus])
}

// ---------------------------------------------------------------------------------------------------------------
// Tab geometry

/// Label width at the pill's 12 pt, shaped with the pill's own font (an estimate from a glyph table came out a few
/// points short, so labels ran into the next pill). Kept per label: `view` asks for every tab on every frame.
fn text_w(s: &str, bold: bool) -> f32 {
    use iced::advanced::graphics::text::Paragraph;
    use iced::advanced::text::{self as atext, paragraph::Paragraph as _};
    thread_local! {
        static WIDTHS: RefCell<std::collections::HashMap<(String, bool), f32>> = RefCell::new(std::collections::HashMap::new());
    }
    if let Some(w) = WIDTHS.with(|m| m.borrow().get(&(s.to_string(), bold)).copied()) {
        return w;
    }
    // A width taken before the app's fonts are in would be the fallback font's and stay in the cache: load them first.
    static FONTS: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        let mut system = iced::advanced::graphics::text::font_system().write().expect("font system lock");
        for bytes in fb_ui::fonts::bytes() {
            system.load_font(bytes);
        }
    });
    let style = if bold { PILL_BOLD } else { PILL_BODY };
    let p = Paragraph::with_text(atext::Text {
        content: s,
        bounds: Size::INFINITE,
        size: iced::Pixels(style.size),
        line_height: atext::LineHeight::Relative(style.line),
        font: fb_ui::fonts::inter(style.weight),
        align_x: atext::Alignment::Default,
        align_y: iced::alignment::Vertical::Top,
        shaping: atext::Shaping::Advanced,
        wrapping: atext::Wrapping::None,
    });
    let w = p.min_bounds().width.ceil();
    WIDTHS.with(|m| m.borrow_mut().insert((s.to_string(), bold), w));
    w
}

/// Fixed chrome around the label: padding, symbol, gaps, split glyph, close button (Swift TabPill: leading 10, glyph
/// 13, spacing 6; closable: spacing 4, the 20 pt ×, trailing 5; else trailing 10).
fn chrome(status: AgentStatus, is_split: bool, selected: bool) -> f32 {
    10.0 + glyph_slot(status) + 6.0 + if is_split { 6.0 + 11.0 } else { 0.0 } + if selected { 4.0 + 20.0 + 5.0 } else { 10.0 }
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
    let ch = chrome(tab.status, split, selected);
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

/// A button next to the island: a round 36 pt hit area, a plain icon at the Mac's glyph size (Swift FullScreenTopRow:
/// `.frame(width: 36, height: 36)`, 15 pt medium symbols, `.foregroundStyle(.secondary)`, which on the toolbar's glass
/// reads as light as our tertiary label).
fn action_button<'a>(t: Tokens, ic: Icon, active: bool, msg: Message) -> Element<'a, Message> {
    button(container(icon(ic, ACTION_ICON, if active { t.label } else { t.label_3 })).center(Length::Fixed(ACTION_BTN)))
        .padding(0)
        .on_press(msg)
        .style(move |_, st| button::Style {
            background: (active || matches!(st, button::Status::Hovered)).then_some(Background::Color(t.fill_2)),
            border: Border { radius: (ACTION_BTN / 2.0).into(), ..Default::default() },
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

pub(crate) fn tip<'a>(t: Tokens, content: Element<'a, Message>, label: String, pos: tooltip::Position) -> Element<'a, Message> {
    tooltip(content, container(CALLOUT.text(label).color(t.label)).padding([5, 9]).style(style::float_e2(t, R_ROW)), pos).gap(6).delay(Duration::from_millis(450)).into()
}

fn tab_pill<'a>(t: Tokens, ws_id: &str, tab: &'a Tab, g: &TabGeom, selected: bool, position: usize, animate: Option<std::time::Duration>) -> Element<'a, Message> {
    let blocked = tab.status == AgentStatus::Blocked;
    let split = tab.panes.len() > 1;
    let kind = status_kind(tab.status);
    let sym = status::symbol_live(kind, &t, PILL_GLYPH, animate);
    let sym = container(sym).center_x(Length::Fixed(glyph_slot(tab.status)));
    let label = if g.bold { PILL_BOLD.text(g.label.clone()) } else { PILL_BODY.text(g.label.clone()) };
    let label = label.color(if selected || blocked { t.label } else { t.label_2 }).wrapping(text::Wrapping::None);
    let mut inner = row![sym, label].spacing(6).align_y(Alignment::Center);
    if split {
        // Swift `rectangle.split.1x2` at caption size, tertiary.
        inner = inner.push(icon(Icon::Rows2, 11.0, t.label_3));
    }
    let main = button(container(inner).height(Length::Fill).align_y(Alignment::Center).padding(Padding { left: 10.0, right: if selected { 4.0 } else { 10.0 }, top: 0.0, bottom: 0.0 }))
        .on_press(Message::SelectTab { workspace: ws_id.to_string(), tab: tab.id.clone() })
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(0)
        .style(move |_, st| button::Style {
            background: (!selected && matches!(st, button::Status::Hovered)).then_some(Background::Color(Color { a: 0.05, ..t.label })),
            border: Border { radius: PILL_R.into(), ..Default::default() },
            ..Default::default()
        });
    let mut r = row![main].align_y(Alignment::Center);
    if selected {
        r = r.push(
            // Swift TabCloseButton: a 20 pt circle, its fill only on hover.
            button(container(icon(Icon::X, SIZE_INLINE - 2.0, t.label_3)).center(Length::Fixed(20.0)))
                .padding(0)
                .on_press(Message::TopRow(TopRowMsg::CloseTab(tab.id.clone())))
                .style(move |_, st| button::Style {
                    background: matches!(st, button::Status::Hovered).then_some(Background::Color(Color { a: 0.12, ..t.label })),
                    border: Border { radius: 10.0.into(), ..Default::default() },
                    ..Default::default()
                }),
        )
        .push(Space::new().width(5));
    }
    let pill = container(r).width(Length::Fixed(g.width)).height(Length::Fixed(TAB_H));
    // The island is the glass; a selected pill is only a faint fill, a capsule (Swift TabPill).
    let pill: Element<'a, Message> = if selected { pill.style(style::fill(Color { a: 0.10, ..t.label }, PILL_R)).into() } else { pill.into() };
    let pill = mouse_area(pill)
        .on_right_press(Message::Workspace(crate::workspace_actions::WorkspaceMsg::Menu(crate::workspace_actions::Target::Tab(tab.id.clone()))));
    let pill = drag_target(pill.into(), crate::workspace_actions::Target::Tab(tab.id.clone()));
    tip(t, pill, tab_tooltip(tab, position), tooltip::Position::Bottom)
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

/// Same look as the sidebar's Needs-you count, with the direction to scroll (Swift TopRow.marker: caption semibold,
/// white on orange, a capsule at least 18 pt, chevron.left / chevron.right).
fn marker<'a>(t: Tokens, edge: Edge, n: usize) -> Element<'a, Message> {
    let chevron = |ic| icon(ic, 9.0, t.on_accent);
    let num = text(n.to_string()).size(10).font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(t.on_accent);
    let count = match edge {
        Edge::Left => row![chevron(Icon::ChevronLeft), num],
        Edge::Right => row![num, chevron(Icon::ChevronRight)],
    }
    .spacing(2)
    .align_y(Alignment::Center);
    let pill = button(container(count).padding([0, 6]).height(Length::Fixed(18.0)).align_y(Alignment::Center))
        .padding(0)
        .on_press(Message::TopRow(TopRowMsg::RevealBlocked(edge)))
        .style(move |_, st| button::Style {
            background: Some(Background::Color(if matches!(st, button::Status::Hovered) { mix(t.blocked, t.label, 0.12) } else { t.blocked })),
            border: Border { radius: 100.0.into(), ..Default::default() },
            ..Default::default()
        });
    let pill = container(pill).id(marker_id(edge));
    tip(t, pill.into(), format!("{n} tab{} need you: click to show", if n == 1 { "" } else { "s" }), tooltip::Position::Bottom)
}

/// Widget id of the edge marker, for tests.
pub fn marker_id(edge: Edge) -> iced::widget::Id {
    iced::widget::Id::new(match edge {
        Edge::Left => "toprow-marker-left",
        Edge::Right => "toprow-marker-right",
    })
}

/// Review diffstat of the selected tab as (added, removed), only when the working tree has changes.
/// Hook for the Review feature; the model has no git data yet, so it is always `None`.
pub fn review_diffstat(_state: &State) -> Option<(usize, usize)> {
    None
}

/// (visible strip width, available width) for a top row of `width` points.
fn strip_metrics(s: &State, width: f32, total: f32) -> (f32, f32) {
    let review_w = if review_diffstat(s).is_some() { 96.0 } else { 0.0 };
    // On the Search screen the field takes the trailing end; the group stays centred, so it gives up both sides.
    let field_w = if s.screen == Screen::Search { 2.0 * (crate::search::FIELD_W + SP_8) } else { 0.0 };
    let avail = (width - 2.0 * SP_16 - CAPSULE_W - ACTION_GAP - review_w - SP_16 - field_w).max(DEFAULT_STRIP_W.min(width * 0.4));
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
        r = r.push(tab_pill(t, &ws_id, tab, g, sel, i + 1, ts.spin_frame()));
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
    // The tab island keeps the Swift geometry (TopRow.height / inset) but no fill of its own: only the selected pill
    // has a ground (the owner's call over the Mac's island capsule).
    let island = container(strip_box)
        .width(Length::Fixed(strip_w + 2.0 * ISLAND_INSET))
        .height(Length::Fixed(TAB_H + 2.0 * ISLAND_INSET))
        .padding(ISLAND_INSET);

    let new_tab = tip(t, action_button(t, Icon::Plus, false, Message::TopRow(TopRowMsg::Plus)), "New tab (\u{2318}T)".into(), tooltip::Position::Bottom);
    let terminal = tip(t, action_button(t, Icon::SquareTerminal, ts.terminal_on, Message::TopRow(TopRowMsg::ToggleTerminal)), "Show terminal (\u{2303}`)".into(), tooltip::Position::Bottom);

    // The actions share one small capsule right after the island (Swift TopRowActions / FullScreenTopRow), not the far right.
    let mut capsule = row![new_tab].spacing(0).align_y(Alignment::Center);
    if let Some((add, del)) = review {
        capsule = capsule.push(
            row![icon(Icon::FileDiff, SIZE_INLINE, t.label_2), CAPTION.text(format!("+{add}")).color(t.diff_add), CAPTION.text(format!("\u{2212}{del}")).color(t.diff_del)]
                .spacing(4)
                .align_y(Alignment::Center),
        );
    }
    capsule = capsule.push(terminal);
    // Plain icons with no capsule of their own, as on the Mac (the owner's call, as for the island).
    let actions = container(capsule).height(Length::Fixed(CAPSULE_H)).padding([0.0, CAPSULE_PAD]);
    // A short hairline in the gap before them, as the Mac toolbar draws between the island and its buttons.
    let rule = container(container(Space::new()).width(Length::Fixed(ACTION_GAP - 2.0)).height(Length::Fixed(1.0)).style(style::fill(t.sep, 0.0)))
        .width(Length::Fixed(ACTION_GAP))
        .center_x(Length::Fixed(ACTION_GAP));

    // Island and actions centred in the content toolbar, the buttons `ACTION_GAP` after the island.
    let group = row![island, rule, actions].align_y(Alignment::Center);
    let gap_l = crate::chrome::drag_area(app, Space::new().width(Length::Fill).height(Length::Fixed(TOPROW_H)));
    let gap_r = crate::chrome::drag_area(app, Space::new().width(Length::Fill).height(Length::Fixed(TOPROW_H)));
    let bar: Element<Message> = container(row![gap_l, group, gap_r].align_y(Alignment::Center).padding([0.0, SP_16]))
        .height(Length::Fixed(TOPROW_H))
        .width(Length::Fill)
        .align_y(Alignment::Center)
        .into();
    if s.screen != Screen::Search {
        return bar;
    }
    // Swift `.searchable(placement: .toolbar)`: the field at the toolbar's trailing end, over the centred group.
    let field = container(crate::search::field(app, CAPSULE_H)).width(Length::Fill).height(Length::Fixed(TOPROW_H)).align_x(Alignment::End).align_y(Alignment::Center).padding([0.0, SP_8]);
    stack![bar, field].into()
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
    // Both menus hang off the centred island's action capsule.
    let island_w = strip_w + 2.0 * ISLAND_INSET;
    let group_w = island_w + ACTION_GAP + CAPSULE_W;
    let group_left = (row_width - group_w).max(0.0) / 2.0;
    let capsule_left = group_left + island_w + ACTION_GAP;
    let (ax, pad) = match kind {
        MenuKind::AllTabs => (
            Alignment::Start,
            Padding {
                top: TOPROW_H - 8.0,
                left: (SIDEBAR_W + capsule_left).min(SIDEBAR_W + row_width - w - SP_8),
                right: 0.0,
                bottom: 0.0,
            },
        ),
        MenuKind::Split => (
            Alignment::Start,
            Padding { top: TOPROW_H - 8.0, left: (SIDEBAR_W + capsule_left + CAPSULE_W / 2.0 - w / 2.0).max(SIDEBAR_W + SP_8), right: 0.0, bottom: 0.0 },
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_turn_slower_while_the_window_is_in_the_background() {
        let mut app = App::demo(crate::Mode::Light, crate::Screen::Conversation);
        assert_eq!(app.state.toprow.spin_frame(), None, "snapshots and tests draw the arcs still");
        app.state.toprow.animate = true;
        assert_eq!(app.state.toprow.spin_frame(), Some(status::SPIN_FRAME));
        let _ = app.update(Message::TopRow(TopRowMsg::WindowFocus(false)));
        assert_eq!(app.state.toprow.spin_frame(), Some(status::SPIN_FRAME_BACKGROUND));
        let _ = app.update(Message::TopRow(TopRowMsg::WindowFocus(true)));
        assert_eq!(app.state.toprow.spin_frame(), Some(status::SPIN_FRAME));
    }
}
