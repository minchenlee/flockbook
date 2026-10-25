//! Keyboard shortcuts: one table for the Keyboard Shortcuts panel and for the keys this module handles
//! (port of Swift `ShortcutsPanel` and the menu key equivalents).
//!
//! The table lists only shortcuts that work in this app. Some rows are handled by other modules (the top row's tab
//! keys, the terminal key, Cmd+F for search); `action` is `None` for those. A test checks that every row with keys
//! reaches exactly one handler. The panel opens with Cmd+/ (Ctrl+/ on Windows and Linux) and from the keyboard
//! button at the bottom of the sidebar; Esc, the close button or a click outside closes it.
//!
//! Cmd means the platform's command key: Cmd on macOS, Ctrl elsewhere (`Modifiers::command`). The panel shows the
//! macOS symbols on macOS and "Ctrl+Shift+K" style text elsewhere.
//!
//! Pane zoom and tab move routes belong to workspace actions.

use fb_ui::icons::{icon, Icon, SIZE_INLINE};
use fb_ui::theme::*;
use iced::keyboard::{self, key::Named, Key, Modifiers};
use iced::widget::{button, column, container, mouse_area, responsive, row, scrollable, stack, text, Space};
use iced::{Alignment, Background, Border, Element, Length, Subscription, Task};

use crate::settings::SettingsMsg;
use crate::state::Screen;
use crate::toprow::{SplitDir, TopRowMsg};
use crate::{style, App, Message};

/// What a shortcut of this module does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ToggleSidebar,
    LargerText,
    SmallerText,
    ActualTextSize,
    NewTab,
    NewWorkspace,
    CloseTab,
    Search,
    Overview,
    NeedsYou,
    Shortcuts,
    Settings,
    SplitRight,
    SplitDown,
    NextTab,
    PrevTab,
    /// Esc: closes the panel and the settings sheet. Not a table row (Esc is listed under Other Keys).
    Dismiss,
}

/// A key and its modifiers. `cmd` is the platform command key; `ctrl` is the Control key itself (macOS ⌃).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combo {
    pub key: K,
    pub cmd: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum K {
    /// A character, and what Shift makes of it on a US layout ("]" and "}").
    Char(&'static str, &'static str),
    Named(Named),
}

const fn cmd(c: &'static str) -> Combo {
    Combo { key: K::Char(c, c), cmd: true, ctrl: false, shift: false, alt: false }
}

const fn cmd_shift(c: &'static str, shifted: &'static str) -> Combo {
    Combo { key: K::Char(c, shifted), cmd: true, ctrl: false, shift: true, alt: false }
}

impl Combo {
    pub fn matches(&self, key: &Key, mods: Modifiers) -> bool {
        let key_ok = match (self.key, key.as_ref()) {
            (K::Char(a, b), Key::Character(c)) => c.eq_ignore_ascii_case(a) || c.eq_ignore_ascii_case(b),
            (K::Named(n), Key::Named(m)) => n == m,
            _ => false,
        };
        if !key_ok || mods.shift() != self.shift || mods.alt() != self.alt {
            return false;
        }
        if cfg!(target_os = "macos") {
            mods.logo() == self.cmd && mods.control() == self.ctrl
        } else {
            // Ctrl is the command key: a row wants it when it has either.
            mods.control() == (self.cmd || self.ctrl) && !mods.logo()
        }
    }

    /// The modifiers a user holds for this combo on this platform (for tests and the panel).
    pub fn modifiers(&self) -> Modifiers {
        let mut m = Modifiers::empty();
        if self.cmd {
            m |= if cfg!(target_os = "macos") { Modifiers::LOGO } else { Modifiers::CTRL };
        }
        if self.ctrl {
            m |= Modifiers::CTRL;
        }
        if self.shift {
            m |= Modifiers::SHIFT;
        }
        if self.alt {
            m |= Modifiers::ALT;
        }
        m
    }

    /// The key event a user makes: with Shift, the shifted character.
    pub fn key(&self) -> Key {
        match self.key {
            K::Char(c, s) => Key::Character((if self.shift { s } else { c }).into()),
            K::Named(n) => Key::Named(n),
        }
    }

    /// "⇧⌘N" on macOS, "Ctrl+Shift+N" elsewhere.
    pub fn text(&self, mac: bool) -> String {
        let key = match self.key {
            K::Char(c, _) => c.to_uppercase(),
            K::Named(Named::Tab) => if mac { "⇥" } else { "Tab" }.into(),
            K::Named(Named::ArrowDown) => "↓".into(),
            K::Named(Named::ArrowUp) => "↑".into(),
            K::Named(Named::Enter) => if mac { "↩" } else { "Enter" }.into(),
            K::Named(Named::Escape) => if mac { "⎋" } else { "Esc" }.into(),
            K::Named(n) => format!("{n:?}"),
        };
        if mac {
            let mut s = String::new();
            if self.ctrl {
                s.push('⌃');
            }
            if self.alt {
                s.push('⌥');
            }
            if self.shift {
                s.push('⇧');
            }
            if self.cmd {
                s.push('⌘');
            }
            s + &key
        } else {
            let mut parts = Vec::new();
            if self.cmd || self.ctrl {
                parts.push("Ctrl".to_string());
            }
            if self.alt {
                parts.push("Alt".into());
            }
            if self.shift {
                parts.push("Shift".into());
            }
            parts.push(key);
            parts.join("+")
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Keys {
    Combo(Combo),
    /// Shown only (a range, or keys another widget owns): macOS text, other text.
    Text(&'static str, &'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Shortcut {
    pub group: &'static str,
    pub label: &'static str,
    pub keys: Keys,
    /// Handled here. `None`: another module handles the keys, or the row is only shown.
    pub action: Option<Action>,
}

const fn row_of(group: &'static str, label: &'static str, keys: Combo, action: Option<Action>) -> Shortcut {
    Shortcut { group, label, keys: Keys::Combo(keys), action }
}

const fn shown(group: &'static str, label: &'static str, mac: &'static str, other: &'static str) -> Shortcut {
    Shortcut { group, label, keys: Keys::Text(mac, other), action: None }
}

const CTRL_TAB: Combo = Combo { key: K::Named(Named::Tab), cmd: false, ctrl: true, shift: false, alt: false };
const CTRL_SHIFT_TAB: Combo = Combo { key: K::Named(Named::Tab), cmd: false, ctrl: true, shift: true, alt: false };
const ALT_CMD_DOWN: Combo = Combo { key: K::Named(Named::ArrowDown), cmd: true, ctrl: false, shift: false, alt: true };
const ALT_CMD_UP: Combo = Combo { key: K::Named(Named::ArrowUp), cmd: true, ctrl: false, shift: false, alt: true };
const CTRL_BACKTICK: Combo = Combo { key: K::Char("`", "~"), cmd: false, ctrl: true, shift: false, alt: false };

/// Every shortcut, in panel order. Groups follow the Swift menus.
pub const TABLE: &[Shortcut] = &[
    row_of("File", "New Tab", cmd("t"), Some(Action::NewTab)),
    row_of("File", "New Workspace", cmd_shift("n", "N"), Some(Action::NewWorkspace)),
    row_of("File", "Close Tab", cmd("w"), Some(Action::CloseTab)),
    row_of("View", "Toggle Sidebar", cmd("b"), Some(Action::ToggleSidebar)),
    row_of("View", "Larger Text", cmd("="), Some(Action::LargerText)),
    row_of("View", "Smaller Text", cmd("-"), Some(Action::SmallerText)),
    row_of("View", "Actual Text Size", cmd("0"), Some(Action::ActualTextSize)),
    row_of("View", "Show Terminal", CTRL_BACKTICK, None),
    row_of("View", "Show in Overview", cmd_shift("\\", "|"), Some(Action::Overview)),
    row_of("View", "Search", cmd("k"), Some(Action::Search)),
    row_of("View", "Search", cmd("f"), None),
    row_of("View", "Needs You", cmd_shift("i", "I"), Some(Action::NeedsYou)),
    row_of("View", "Keyboard Shortcuts", cmd("/"), Some(Action::Shortcuts)),
    row_of("View", "Settings", cmd(","), Some(Action::Settings)),
    row_of("Pane", "Split Right", cmd("d"), Some(Action::SplitRight)),
    row_of("Pane", "Split Down", cmd_shift("d", "D"), Some(Action::SplitDown)),
    row_of("Window", "Next Tab", CTRL_TAB, None),
    row_of("Window", "Previous Tab", CTRL_SHIFT_TAB, None),
    row_of("Window", "Next Tab", cmd_shift("]", "}"), Some(Action::NextTab)),
    row_of("Window", "Previous Tab", cmd_shift("[", "{"), Some(Action::PrevTab)),
    row_of("Window", "Next Workspace", ALT_CMD_DOWN, None),
    row_of("Window", "Previous Workspace", ALT_CMD_UP, None),
    shown("Window", "Tab 1 to 9", "⌘1 … ⌘9", "Ctrl+1 … Ctrl+9"),
    shown("Other Keys", "Send message", "↩", "Enter"),
    shown("Other Keys", "New line", "⇧↩", "Shift+Enter"),
    shown("Other Keys", "Open selected search result", "↩", "Enter"),
    shown("Other Keys", "Close sheet, terminal or panel", "⎋", "Esc"),
];

/// The panel's groups in order, with their rows.
pub fn groups() -> Vec<(&'static str, Vec<&'static Shortcut>)> {
    let mut out: Vec<(&'static str, Vec<&'static Shortcut>)> = Vec::new();
    for s in TABLE {
        match out.iter_mut().find(|(g, _)| *g == s.group) {
            Some((_, rows)) => rows.push(s),
            None => out.push((s.group, vec![s])),
        }
    }
    out
}

pub fn keys_text(s: &Shortcut, mac: bool) -> String {
    match s.keys {
        Keys::Combo(c) => c.text(mac),
        Keys::Text(m, o) => if mac { m } else { o }.to_string(),
    }
}

/// The keys this module handles. Esc dismisses the panel and the settings sheet.
pub fn on_key(key: Key, mods: Modifiers) -> Option<Message> {
    if key == Key::Named(Named::Escape) && mods.is_empty() {
        return Some(Message::Shortcut(Action::Dismiss));
    }
    TABLE.iter().find_map(|s| match (s.keys, s.action) {
        (Keys::Combo(c), Some(a)) if c.matches(&key, mods) => Some(Message::Shortcut(a)),
        _ => None,
    })
}

pub fn subscription(_app: &App) -> Subscription<Message> {
    keyboard::listen().filter_map(|e| match e {
        keyboard::Event::KeyPressed { key, modifiers, .. } => on_key(key, modifiers),
        _ => None,
    })
}

#[derive(Debug, Default)]
pub struct ShortcutsState {
    pub open: bool,
}

/// Runs an action through the same messages the buttons send.
pub fn run(app: &mut App, action: Action) -> Task<Message> {
    if app.state.settings.open && !matches!(action, Action::Dismiss | Action::Settings | Action::Shortcuts) {
        return Task::none();
    }
    let msg = match action {
        Action::ToggleSidebar => Message::Settings(SettingsMsg::ToggleSidebar),
        Action::LargerText => Message::Settings(SettingsMsg::SetSize(app.state.settings.reading_size().larger())),
        Action::SmallerText => Message::Settings(SettingsMsg::SetSize(app.state.settings.reading_size().smaller())),
        Action::ActualTextSize => Message::Settings(SettingsMsg::SetSize(crate::settings::ReadingSize::M)),
        Action::NewTab => Message::TopRow(TopRowMsg::NewTab),
        Action::NewWorkspace => Message::Sidebar(crate::sidebar::SidebarMsg::AddWorkspace),
        Action::CloseTab => match app.state.selected_tab.clone() {
            Some(id) => Message::TopRow(TopRowMsg::CloseTab(id)),
            None => return Task::none(),
        },
        Action::Search => Message::Search(crate::search::SearchMsg::Open),
        Action::Overview => Message::Navigate(Screen::Overview),
        Action::NeedsYou => Message::Navigate(Screen::NeedsYou),
        Action::Shortcuts => {
            app.state.shortcuts.open = !app.state.shortcuts.open;
            if app.state.shortcuts.open {
                app.state.settings.open = false;
            }
            return Task::none();
        }
        Action::Settings => {
            app.state.shortcuts.open = false;
            Message::Settings(SettingsMsg::Toggle)
        }
        Action::SplitRight => Message::TopRow(TopRowMsg::Split(SplitDir::Right)),
        Action::SplitDown => Message::TopRow(TopRowMsg::Split(SplitDir::Down)),
        Action::NextTab => Message::TopRow(TopRowMsg::NextTab),
        Action::PrevTab => Message::TopRow(TopRowMsg::PrevTab),
        Action::Dismiss => {
            app.state.shortcuts.open = false;
            app.state.settings.open = false;
            return Task::none();
        }
    };
    app.update(msg)
}

// ---------------------------------------------------------------------------------------------------------- view

pub const PANEL_W: f32 = 680.0;
/// Keycap corner radius (Swift `Radius.key`).
const R_KEY: f32 = 5.0;

/// One keycap (Swift `Kbd`): 11pt secondary text, a subtle fill, a hairline edge, 20pt square.
fn keycap(t: Tokens, cap: &str) -> Element<'static, Message> {
    container(text(cap.to_string()).size(11.0).color(t.label_2))
        .center(Length::Fixed(20.0))
        .padding([0, if cap.chars().count() > 1 { 3 } else { 5 }])
        .style(move |_| iced::widget::container::Style {
            background: Some(Background::Color(iced::Color { a: 0.06, ..t.label })),
            border: Border { color: iced::Color { a: 0.14, ..t.label }, width: 1.0, radius: R_KEY.into() },
            ..Default::default()
        })
        .into()
}

/// The caps one at a time; a range dash stays plain text between them (Swift `KbdGroup`).
fn caps_view(t: Tokens, keys: &str) -> Element<'static, Message> {
    // A range ("⌘1 … ⌘9") reads better as one line than as caps.
    if keys.contains('…') {
        return CALLOUT.text(keys.to_string()).color(t.label_2).into();
    }
    let mut r = row![].spacing(4).align_y(Alignment::Center);
    for cap in split_caps(keys) {
        if cap == "–" {
            r = r.push(text(cap).size(11.0).color(t.label_3));
        } else {
            r = r.push(keycap(t, &cap));
        }
    }
    r.into()
}

/// "⇧⌘N" -> ["⇧", "⌘", "N"]: each modifier glyph is a cap, the rest is one.
pub fn split_caps(keys: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in keys.chars() {
        if matches!(c, '⌃' | '⌥' | '⇧' | '⌘') {
            out.push(c.to_string());
        } else if let Some(last) = out.last_mut().filter(|s| c.is_ascii_alphanumeric() && s.chars().next().is_some_and(|f| f.is_ascii_alphanumeric())) {
            last.push(c);
        } else {
            out.push(c.to_string());
        }
    }
    out
}

fn group_view<'a>(t: Tokens, name: &'a str, rows: &[&'a Shortcut], mac: bool) -> Element<'a, Message> {
    let mut col = column![container(CAPTION.text(name).color(t.label_2)).padding([0, 4])].spacing(4);
    for s in rows {
        let keys: Element<'_, Message> = if mac { caps_view(t, &keys_text(s, mac)) } else { CALLOUT.text(keys_text(s, mac)).color(t.label_2).into() };
        col = col.push(
            row![BODY.text(s.label).color(t.label), Space::new().width(Length::Fill), keys]
                .align_y(Alignment::Center)
                .padding([2, 4]),
        );
    }
    col.into()
}

/// The panel, or nothing when closed.
pub fn layer(app: &App) -> Element<'_, Message> {
    if !app.state.shortcuts.open {
        return Space::new().into();
    }
    responsive(move |size| panel(app, size.width, size.height)).into()
}

fn panel(app: &App, width: f32, height: f32) -> Element<'_, Message> {
    let t = app.tokens();
    let mac = cfg!(target_os = "macos");
    // Two columns, the groups split where the row counts come closest to even.
    let gs = groups();
    let total: usize = gs.iter().map(|(_, r)| r.len() + 1).sum();
    let (mut left, mut right) = (column![].spacing(SP_16).width(Length::Fill), column![].spacing(SP_16).width(Length::Fill));
    let mut seen = 0;
    for (name, rows) in &gs {
        let el = group_view(t, name, rows, mac);
        if seen * 2 < total {
            left = left.push(el);
        } else {
            right = right.push(el);
        }
        seen += rows.len() + 1;
    }
    let close = button(container(icon(Icon::X, SIZE_INLINE, t.label_2)).center(Length::Fixed(28.0))).padding(0).on_press(Message::Shortcut(Action::Dismiss)).style(move |_, st| button::Style {
        background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_2)),
        border: Border { radius: 14.0.into(), ..Default::default() },
        ..Default::default()
    });
    let header = row![HEADLINE.text("Keyboard Shortcuts").color(t.label), Space::new().width(Length::Fill), close].align_y(Alignment::Center);
    let groups: Element<'_, Message> = if width < 600.0 { column![left, right].spacing(SP_16).into() } else { row![left, right].spacing(SP_24).into() };
    let card = container(column![header, scrollable(groups).height(Length::Fill)].spacing(SP_12)).padding(SP_16)
        .width(Length::Fill).max_width(PANEL_W).height(Length::Fixed((height - TOPROW_H - 24.0).max(1.0)))
        .style(style::float_e2(t, R_CARD));
    let card = mouse_area(card).on_press(Message::Noop);
    let scrim = mouse_area(container(Space::new()).width(Length::Fill).height(Length::Fill).style(style::fill(iced::Color { a: 0.18, ..iced::Color::BLACK }, 0.0))).on_press(Message::Shortcut(Action::Dismiss));
    let placed = container(card).width(Length::Fill).height(Length::Fill).align_x(Alignment::Center).padding(iced::Padding { top: TOPROW_H + 24.0, ..iced::Padding::ZERO });
    stack![scrim, placed].into()
}
