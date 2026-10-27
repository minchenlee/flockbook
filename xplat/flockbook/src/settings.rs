//! Settings (port of Swift `SettingsView`): window glass, reading width, and "Allow sending input to agents".
//!
//! Like Swift's `SettingsPage`, it is a page of the main window: the sidebar holds Back and the tabs, the detail column
//! the form, and the tab row is gone. It opens with Cmd+, (Ctrl+, on Windows and Linux) and from the Settings row at the
//! foot of the sidebar. Esc or Back returns to the screen it was opened from.
//!
//! The values live in `settings.json` in the user's config folder. Tests and snapshots keep them in memory only
//! (`path` is `None`), so they never touch the real file.
//!
//! When a change applies:
//! - Window glass: Opaque applies now. Glass needs a transparent window, which is made at launch: if the window was
//!   made opaque, Glass applies at the next launch.
//! - Width: now.
//! - Allow input: off applies now (the write paths are removed). On applies at the next launch: the write paths are
//!   made at launch, and turning them on is the riskier direction. The toggle shows the saved value (so a second
//!   click undoes the first); the line under it tells what this window does now ([`access::writes_live`]), so the
//!   sheet never says "on" while the composer says "Read-only".

use std::path::{Path, PathBuf};

use fb_ui::icons::{icon, Icon, SIZE_DEFAULT, SIZE_INLINE};
use fb_ui::theme::*;
use iced::widget::{button, column, container, row, scrollable, svg, text, toggler, Space};
use iced::{Alignment, Background, Border, Element, Length, Task};
use serde::{Deserialize, Serialize};

#[path = "ghostty_theme.rs"]
pub mod ghostty_theme;
#[path = "status_tags.rs"]
pub mod status_tags;
#[path = "settings_cover.rs"]
mod settings_cover;
pub use settings_cover::{cover, hide};

use crate::access;
use crate::glass::{Backdrop, Os};
use crate::{style, App, Message};

/// The reading column (Swift `ReadingWidth`), in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadingWidth {
    Narrow,
    #[default]
    Medium,
    Wide,
    Full,
}

impl ReadingWidth {
    pub const ALL: [ReadingWidth; 4] = [ReadingWidth::Narrow, ReadingWidth::Medium, ReadingWidth::Wide, ReadingWidth::Full];

    /// Full is "as wide as the pane": the pane clamps it.
    pub fn points(self) -> f32 {
        match self {
            ReadingWidth::Narrow => 600.0,
            ReadingWidth::Medium => 760.0,
            ReadingWidth::Wide => 920.0,
            ReadingWidth::Full => 2400.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ReadingWidth::Narrow => "Narrow",
            ReadingWidth::Medium => "Medium",
            ReadingWidth::Wide => "Wide",
            ReadingWidth::Full => "Full",
        }
    }
}

/// Shared settings contracts; other packages consume these instead of duplicating preference enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReadingSize { S, #[default] M, L, Xl }
impl ReadingSize {
    pub const ALL: [Self; 4] = [Self::S, Self::M, Self::L, Self::Xl];
    pub fn label(self) -> &'static str { match self { Self::S => "S", Self::M => "M", Self::L => "L", Self::Xl => "XL" } }
    pub fn body(self) -> f32 { match self { Self::S => 15.0, Self::M => 16.0, Self::L => 18.0, Self::Xl => 20.0 } }
    pub fn line(self) -> f32 { match self { Self::S => 24.0, Self::M => 26.0, Self::L => 29.0, Self::Xl => 32.0 } }
    pub fn code(self) -> f32 { match self { Self::S => 12.0, Self::M => 13.0, Self::L => 14.0, Self::Xl => 15.0 } }
    pub fn inline_code(self) -> f32 { match self { Self::S => 13.0, Self::M => 14.0, Self::L => 15.5, Self::Xl => 17.5 } }
    pub fn table(self) -> f32 { match self { Self::S => 13.0, Self::M => 14.0, Self::L => 15.0, Self::Xl => 16.0 } }
    pub fn heading(self, level: usize) -> f32 { match level {
        1 => match self { Self::S => 21.0, Self::M => 22.0, Self::L => 24.0, Self::Xl => 26.0 },
        2 => match self { Self::S | Self::M => 19.0, Self::L => 21.0, Self::Xl => 23.0 },
        _ => self.body(),
    } }
    pub fn larger(self) -> Self { Self::ALL[(self as usize + 1).min(3)] }
    pub fn smaller(self) -> Self { Self::ALL[(self as usize).saturating_sub(1)] }
    pub fn column(self, width: ReadingWidth) -> f32 { width.points() * self.body() / 16.0 }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalColors { #[default] Flockbook, Ghostty }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlusAction { #[default] NewTab, SplitRight, SplitDown }
impl PlusAction {
    pub fn resolved(self, has_pane: bool) -> Self { if has_pane { self } else { Self::NewTab } }
    pub fn label(self) -> &'static str { match self { Self::NewTab => "New tab", Self::SplitRight => "Split right", Self::SplitDown => "Split down" } }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab { #[default] General, Agents, Remote, Updates }
impl SettingsTab {
    pub const ALL: [Self; 4] = [Self::General, Self::Agents, Self::Remote, Self::Updates];
    pub fn label(self) -> &'static str { match self { Self::General => "General", Self::Agents => "Agents", Self::Remote => "Remote", Self::Updates => "Updates" } }
}

/// The stored values. `None` is "not set": the default applies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glass: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_width: Option<ReadingWidth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_input: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading_size: Option<ReadingSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_colors: Option<TerminalColors>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plus_action: Option<PlusAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_hidden: Option<bool>,
}

impl Prefs {
    /// A missing or unreadable file reads as "nothing set".
    pub fn load(path: &Path) -> Prefs {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    /// Writes a temp file next to `path`, then renames it over `path`.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, path)
    }
}

/// `<config dir>/flockbook/settings.json`: `~/Library/Application Support` on macOS, `%APPDATA%` on Windows,
/// `$XDG_CONFIG_HOME` or `~/.config` on Linux.
pub fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("flockbook").join("settings.json"))
}

#[derive(Debug, Default)]
pub struct SettingsState {
    pub open: bool,
    pub tab: SettingsTab,
    pub expanded_status: std::collections::HashSet<String>,
    /// Last result of an explicit setup action in this window (not inferred from a backup file).
    pub status_enabled: std::collections::HashMap<String, bool>,
    /// Populated on explicit Ghostty selection/reload, never by snapshots.
    pub ghostty: Option<Result<ghostty_theme::GhosttyThemes, String>>,
    pub prefs: Prefs,
    /// Where the values are saved. `None`: memory only (tests, snapshots, demo).
    pub path: Option<PathBuf>,
    /// Launched with `--read-only` (or `FLOCKBOOK_READ_ONLY`): input stays off whatever the setting says.
    pub read_only_launch: bool,
    /// A change that applies at the next launch, shown under the settings.
    pub pending: Option<&'static str>,
    /// The last save error.
    pub error: Option<String>,
    /// Status tags per pane id, read at most once a second (views run on every message).
    pub status_tags: std::cell::RefCell<std::collections::HashMap<String, (std::time::Instant, Vec<herdr_core::StatusTag>)>>,
}

impl SettingsState {
    pub fn reading_width(&self) -> ReadingWidth {
        self.prefs.reading_width.unwrap_or_default()
    }

    pub fn reading_size(&self) -> ReadingSize { self.prefs.reading_size.unwrap_or_default() }
    pub fn column_width(&self) -> f32 { self.reading_size().column(self.reading_width()) }
    /// The conversation's reading metrics (Swift `ReadingSize`, `ReadingWidth.sidePadding`).
    pub fn flow_metrics(&self) -> fb_ui::selectable::flow::Metrics {
        let size = self.reading_size();
        fb_ui::selectable::flow::Metrics {
            body: size.body(),
            line: size.line(),
            heading: [size.heading(1), size.heading(2), size.heading(3), size.body() - 1.0],
            code: size.code(),
            code_line: match size { ReadingSize::S => 18.0, ReadingSize::M => 20.0, ReadingSize::L => 21.0, ReadingSize::Xl => 23.0 },
            table: size.table(),
            gutter: if self.reading_width() == ReadingWidth::Full { 40.0 } else { 28.0 },
        }
    }
    pub fn sidebar_hidden(&self) -> bool { self.prefs.sidebar_hidden.unwrap_or(false) }
    pub fn terminal_palette(&self, tokens: &Tokens) -> fb_ui::term::TermPalette {
        if self.prefs.terminal_colors == Some(TerminalColors::Ghostty) {
            if let Some(Ok(themes)) = &self.ghostty { return themes.theme(tokens.mode).palette(tokens.mode); }
        }
        fb_ui::term::TermPalette::of(tokens)
    }
    pub fn allow_input(&self) -> bool {
        self.prefs.allow_input.unwrap_or(true)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsMsg {
    Open,
    Close,
    Toggle,
    SetGlass(bool),
    SetWidth(ReadingWidth),
    SetAllowInput(bool),
    SelectTab(SettingsTab),
    ToggleStatus(String),
    SetStatusLine { agent: String, on: bool },
    SetSize(ReadingSize),
    SetTerminalColors(TerminalColors),
    ReloadGhostty,
    SetPlusAction(PlusAction),
    ToggleSidebar,
}

pub const NEXT_LAUNCH_GLASS: &str = "Glass applies the next time Flockbook starts.";
/// The line under the input toggle.
pub const INPUT_LIVE: &str = "Flockbook types into your agents' terminals. Turn this off to only read.";
pub const INPUT_OFF: &str = "Read only: typing, Allow and Deny, new tab, close tab and split are disabled.";
pub const INPUT_NEXT_LAUNCH: &str = "Read only in this window. Input turns on the next time Flockbook starts.";
pub const INPUT_LOCKED: &str = "Launched with --read-only: the app only reads, whatever this says.";

pub fn update(app: &mut App, msg: SettingsMsg) -> Task<Message> {
    match msg {
        SettingsMsg::Open | SettingsMsg::Toggle => {
            app.state.settings.open = msg == SettingsMsg::Open || !app.state.settings.open;
            // One sheet at a time.
            if app.state.settings.open {
                app.state.shortcuts.open = false;
            }
        }
        SettingsMsg::SetStatusLine { agent, on } => {
            if app.state.settings.path.is_none() {
                app.state.settings.error = Some("Status line setup is unavailable in previews.".into());
                return Task::none();
            }
            match herdr_core::status_line::set_enabled(&agent, on) {
                Ok(()) => {
                    app.state.settings.status_enabled.insert(agent, on);
                    app.state.settings.error = None;
                }
                Err(error) => app.state.settings.error = Some(error),
            }
        }
        SettingsMsg::ToggleStatus(pane) => {
            if !app.state.settings.expanded_status.remove(&pane) { app.state.settings.expanded_status.insert(pane); }
        }
        SettingsMsg::SelectTab(tab) => app.state.settings.tab = tab,
        SettingsMsg::SetSize(size) => {
            app.state.settings.prefs.reading_size = Some(size);
            save(app);
        }
        SettingsMsg::SetTerminalColors(colors) => {
            app.state.settings.prefs.terminal_colors = Some(colors);
            if colors == TerminalColors::Ghostty { app.state.settings.ghostty = Some(ghostty_theme::load_current()); }
            save(app);
        }
        SettingsMsg::ReloadGhostty => app.state.settings.ghostty = Some(ghostty_theme::load_current()),
        SettingsMsg::SetPlusAction(action) => {
            app.state.settings.prefs.plus_action = Some(action);
            save(app);
        }
        SettingsMsg::ToggleSidebar => {
            app.state.settings.prefs.sidebar_hidden = Some(!app.state.settings.sidebar_hidden());
            save(app);
        }
        SettingsMsg::Close => app.state.settings.open = false,
        SettingsMsg::SetGlass(on) => {
            if glass_note(app).1 {
                return Task::none();
            }
            app.state.settings.prefs.glass = Some(on);
            let launched_with_glass = app.state.glass.backdrop != Backdrop::Opaque;
            if on && launched_with_glass {
                app.state.glass.on = true;
                app.state.settings.pending = None;
            } else if on {
                app.state.settings.pending = Some(NEXT_LAUNCH_GLASS);
            } else {
                app.state.glass.on = false;
                if app.state.settings.pending == Some(NEXT_LAUNCH_GLASS) {
                    app.state.settings.pending = None;
                }
            }
            save(app);
        }
        SettingsMsg::SetWidth(w) => {
            app.state.settings.prefs.reading_width = Some(w);
            save(app);
        }
        SettingsMsg::SetAllowInput(on) => {
            if app.state.settings.read_only_launch {
                return Task::none();
            }
            app.state.settings.prefs.allow_input = Some(on);
            if !on {
                access::make_read_only(app);
            }
            save(app);
        }
    }
    Task::none()
}

fn save(app: &mut App) {
    let s = &mut app.state.settings;
    if let Some(path) = &s.path {
        s.error = s.prefs.save(path).err().map(|e| format!("Could not save settings: {e}"));
    }
}

/// The note under the Window picker, and whether the picker is off.
fn glass_note(app: &App) -> (&'static str, bool) {
    if Os::current() == Os::Other {
        return ("Linux has no window glass: the window stays opaque.", true);
    }
    if app.state.glass.error.is_some() && app.state.glass.backdrop != Backdrop::Opaque {
        // Reduce Transparency, or an older Windows without Mica.
        return ("Opaque: the system turned window glass off.", true);
    }
    ("Glass lets the desktop show through, blurred.", false)
}

/// Glass is "on" in the picker when set on, or not set and the platform has it.
fn glass_selected(app: &App) -> bool {
    app.state.settings.prefs.glass.unwrap_or(Os::current() != Os::Other) && !glass_note(app).1
}

// ---------------------------------------------------------------------------------------------------------- view

/// A macOS-style switch: accent track when on, a gray track when off, a white knob in every state.
pub fn toggle_style(t: Tokens, status: toggler::Status) -> toggler::Style {
    let (on, disabled) = match status {
        toggler::Status::Active { is_toggled } | toggler::Status::Hovered { is_toggled } => (is_toggled, false),
        toggler::Status::Disabled { is_toggled } => (is_toggled, true),
    };
    let track = match (on, disabled) {
        (true, false) => t.accent,
        (true, true) => iced::Color { a: 0.45, ..t.accent },
        (false, _) => t.fill_3,
    };
    toggler::Style {
        background: Background::Color(track),
        background_border_width: 0.0,
        background_border_color: iced::Color::TRANSPARENT,
        foreground: Background::Color(iced::Color::WHITE),
        foreground_border_width: 0.5,
        foreground_border_color: iced::Color { a: 0.12, ..iced::Color::BLACK },
        text_color: None,
        border_radius: None,
        padding_ratio: 0.1,
    }
}

/// The tab list beside the page, the page's margins and the widest its rows get (Swift `SettingsSplit.listWidth`,
/// `SettingsView.margin`, `SettingsView.column`).
const LIST_W: f32 = 196.0;
const PAGE_MARGIN: f32 = 40.0;
const PAGE_COLUMN: f32 = 760.0;

/// A small secondary button (Swift's `.controlSize(.small)`): quiet fill, a hairline border, never the accent.
fn small_button<'a>(t: Tokens, label: &'a str, on: Option<Message>) -> Element<'a, Message> {
    let enabled = on.is_some();
    let fg = if enabled { t.label } else { t.label_3 };
    button(container(CALLOUT.text(label).color(fg)).padding([2, 10]))
        .padding(0)
        .on_press_maybe(on)
        .style(move |_, st| button::Style {
            background: Some(Background::Color(match st {
                button::Status::Hovered | button::Status::Pressed if enabled => t.fill_3,
                _ => t.bg,
            })),
            border: Border { color: t.sep_strong, width: 1.0, radius: R_ROW.into() },
            text_color: fg,
            ..Default::default()
        })
        .into()
}

/// Lucide 24-grid sources for the two symbols `fb_ui::icons` has no glyph for (Swift uses SF Symbols
/// `chevron.left` and `arrow.down.circle`). Black stroke: the SVG's style color tints it, like `fb_ui::icon`.
const CHEVRON_LEFT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>"##;
const ARROW_DOWN_CIRCLE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M12 8v8"/><path d="m8 12 4 4 4-4"/></svg>"##;

/// Lucide's `settings` gear, for SF Symbols `gear` (Swift `SettingsTab.symbol`, the sidebar's Settings row).
const GEAR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#000000" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>"##;

/// The Settings gear, at `size` points in `color`.
pub fn gear<'a>(size: f32, color: iced::Color) -> Element<'a, Message> {
    local_icon(GEAR, size, color)
}

/// An icon from a local SVG source, at `size` points in `color`.
fn local_icon<'a>(source: &'static str, size: f32, color: iced::Color) -> Element<'a, Message> {
    let handle = svg::Handle::from_memory(source.as_bytes().to_vec());
    svg(handle).width(Length::Fixed(size)).height(Length::Fixed(size)).style(move |_, _| svg::Style { color: Some(color) }).into()
}

/// The tab's symbol (Swift `SettingsTab.symbol`): gear, terminal, phone, arrow down in a circle.
fn tab_symbol<'a>(tab: SettingsTab, size: f32, color: iced::Color) -> Element<'a, Message> {
    match tab {
        SettingsTab::General => gear(size, color),
        SettingsTab::Agents => icon(Icon::SquareTerminal, size, color),
        SettingsTab::Remote => icon(Icon::Smartphone, size, color),
        SettingsTab::Updates => local_icon(ARROW_DOWN_CIRCLE, size, color),
    }
}

fn segmented<'a, T: Copy + PartialEq + 'a>(t: Tokens, items: &[(T, &'static str)], selected: Option<T>, on: Option<fn(T) -> Message>) -> Element<'a, Message> {
    let mut r = row![].spacing(2);
    for &(value, label) in items {
        let sel = selected == Some(value);
        let color = if on.is_none() { t.label_3 } else { t.label };
        let mut b = button(container(text(label).size(CALLOUT.size).color(color)).center_x(Length::Fixed(label_width(label)))).padding([3, 6]).style(move |_, st| button::Style {
            // Swift's `.segmented` picker: the chosen segment is a slightly darker grey on the track, not a raised chip.
            background: if sel {
                Some(Background::Color(t.fill_3))
            } else {
                matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_2))
            },
            border: Border { radius: 5.0.into(), ..Default::default() },
            text_color: color,
            ..Default::default()
        });
        if let Some(f) = on {
            b = b.on_press(f(value));
        }
        r = r.push(b);
    }
    container(r).padding(2).style(style::fill(t.fill_1, 7.0)).into()
}

/// The name on the left, the control on the right on the same line (Swift `SettingsRow`).
fn setting_row<'a>(t: Tokens, title: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    container(
        row![BODY.text(title).color(t.label), Space::new().width(Length::Fill), control]
            .spacing(SP_12).align_y(Alignment::Center).width(Length::Fill),
    )
    .padding([8, 14])
    .into()
}

/// A segmented label's fixed width, so the cap stays the same across choices (Swift `.fixedSize()`).
fn label_width(label: &str) -> f32 {
    (label.chars().count() as f32 * 6.5 + 18.0).max(34.0)
}

/// A group of rows: a small title above, the rows on one rounded fill with a hairline between them, and the line that
/// says what they do in more words *under* the card (Swift `SettingsGroup`).
fn section<'a>(t: Tokens, title: &'a str, note: Option<String>, rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut col = column![].spacing(0);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            // Swift draws the divider inset by the row's leading padding (`Settings.swift:204`).
            col = col.push(container(container(Space::new()).width(Length::Fill).height(Length::Fixed(1.0)).style(style::fill(t.sep, 0.0))).padding(iced::Padding { left: 14.0, ..iced::Padding::ZERO }));
        }
        col = col.push(r);
    }
    let mut group = column![
        container(CALLOUT.text(title).color(t.label_2)).padding([0, 4]),
        container(col).style(style::fill(t.fill_1, R_ROW + 4.0)),
    ]
    .spacing(4);
    if let Some(note) = note {
        group = group.push(container(CALLOUT.text(note).color(t.label_2)).padding([0, 4]));
    }
    group.into()
}

/// The page in the detail column, or nothing when closed. With the window sidebar hidden the page brings the tab list
/// itself, a hairline to its right (Swift `SettingsPage`: `list: own`).
pub fn page(app: &App) -> Element<'_, Message> {
    if !app.state.settings.open {
        return Space::new().into();
    }
    let t = app.tokens();
    let form = container(scrollable(container(form(app)).width(Length::Fill)).height(Length::Fill)).width(Length::Fill).height(Length::Fill);
    if !app.state.settings.sidebar_hidden() {
        return form.into();
    }
    let rule = container(Space::new()).width(Length::Fixed(1.0)).height(Length::Fill).style(style::fill(t.sep, 0.0));
    row![tab_list(t, app.state.settings.tab, 12.0), rule, form].height(Length::Fill).into()
}

/// The window sidebar while Settings shows (Swift `SettingsSidebar`): Back on top, then the tabs, below the title bar band.
pub fn sidebar_list(app: &App, _top: f32) -> Element<'_, Message> {
    use crate::sidebar::{row_button, PAD_LEFT, PAD_RIGHT, ROW_GAP, ROW_H};
    let t = app.bar_tokens();
    let selected = app.state.settings.tab;
    // Swift `SettingsSidebar`: the window sidebar's own List (`.sidebar`), so the rows match the sidebar's: Back in the
    // first row's place (centre 68 pt down), the "Settings" section header, then the tabs at a 32 pt pitch.
    let line = |sym: Element<'static, Message>, label: &'static str| -> Element<'static, Message> {
        row![container(sym).center(Length::Fixed(22.0)), BODY.text(label).color(t.label), Space::new().width(Length::Fill)].spacing(SP_8).align_y(Alignment::Center).into()
    };
    let back = row_button(t, false, line(local_icon(CHEVRON_LEFT, SIZE_DEFAULT, t.label_2), "Back"), Message::Settings(SettingsMsg::Close), ROW_H);
    let header = container(text("Settings").size(11).font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(t.label_2))
        .height(Length::Fixed(20.0))
        .align_y(Alignment::Center)
        .padding(iced::Padding { left: 4.0, ..iced::Padding::ZERO });
    let mut tabs = column![].spacing(ROW_GAP);
    for tab in SettingsTab::ALL {
        tabs = tabs.push(row_button(t, tab == selected, line(tab_symbol(tab, SIZE_DEFAULT, t.label_2), tab.label()), Message::Settings(SettingsMsg::SelectTab(tab)), ROW_H));
    }
    container(column![
        crate::chrome::drag_area(app, Space::new().width(Length::Fill).height(53)),
        back,
        Space::new().height(13),
        header,
        Space::new().height(1),
        tabs
    ])
    .padding(iced::Padding { left: PAD_LEFT, right: PAD_RIGHT, ..iced::Padding::ZERO })
    .height(Length::Fill)
    .into()
}

/// The tab's one-line summary (Swift `SettingsTab.summary`).
fn tab_summary(tab: SettingsTab) -> &'static str {
    match tab {
        SettingsTab::General => "How the window, reading and terminal look; what + does.",
        SettingsTab::Agents => "Sending input to agents, and the status line setup.",
        SettingsTab::Remote => "Remote access to this Mac.",
        SettingsTab::Updates => "Check for and install updates.",
    }
}

/// The tab's icon in a blue rounded square, its title and one line on what it is for (Swift `SettingsView.header`).
fn page_header(t: Tokens, tab: SettingsTab) -> Element<'static, Message> {
    let badge = container(tab_symbol(tab, 15.0, iced::Color::WHITE))
        .center(Length::Fixed(32.0))
        .style(style::fill(t.accent, 8.0));
    column![
        row![
            badge,
            column![TITLE.text(tab.label()).size(20.0).color(t.label), CALLOUT.text(tab_summary(tab)).color(t.label_2)].spacing(1),
        ]
        .spacing(SP_12)
        .align_y(Alignment::Center),
    ]
    .spacing(6)
    .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 4.0, left: 0.0 })
    .into()
}

/// The tabs down the left, like System Settings (Swift `SettingsTabList`): Back, a small "Settings" heading, then the
/// rows on a light grey fill when chosen. Back returns to the screen Settings was opened from.
fn tab_list(t: Tokens, selected: SettingsTab, top: f32) -> Element<'static, Message> {
    let back = button(container(row![local_icon(CHEVRON_LEFT, SIZE_INLINE, t.label_2), BODY.text("Back").color(t.label)].spacing(SP_8).align_y(Alignment::Center)).padding([6, 8]).width(Length::Fill))
        .padding(0)
        .width(Length::Fill)
        .on_press(Message::Settings(SettingsMsg::Close))
        .style(move |_, st| button::Style {
            background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_1)),
            border: Border { radius: 7.0.into(), ..Default::default() },
            ..Default::default()
        });
    // Swift: Back `.padding(.bottom, 6)`, the heading `.subheadline.weight(.semibold)` `.padding(.bottom, 2)`.
    let mut col = column![
        container(back).padding(iced::Padding { bottom: 6.0, ..iced::Padding::ZERO }),
        container(CALLOUT.text("Settings").font(fb_ui::fonts::inter(iced::font::Weight::Semibold)).color(t.label_2)).padding(iced::Padding { top: 0.0, right: 8.0, bottom: 2.0, left: 8.0 }),
    ]
    .spacing(2);
    for tab in SettingsTab::ALL {
        let on = tab == selected;
        col = col.push(
            button(container(row![tab_symbol(tab, SIZE_INLINE, t.label_2), BODY.text(tab.label()).color(t.label)].spacing(SP_8).align_y(Alignment::Center)).padding([6, 8]).width(Length::Fill))
                .padding(0)
                .width(Length::Fill)
                .on_press(Message::Settings(SettingsMsg::SelectTab(tab)))
                .style(move |_, st| button::Style {
                    background: if on {
                        Some(Background::Color(t.fill_2))
                    } else {
                        matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_1))
                    },
                    border: Border { radius: 7.0.into(), ..Default::default() },
                    ..Default::default()
                }),
        );
    }
    // Swift `SettingsTabList`: `.padding(.horizontal, 10).padding(.top, 12)`.
    container(col).width(Length::Fixed(LIST_W)).height(Length::Fill).padding(iced::Padding { top, right: 10.0, bottom: 10.0, left: 10.0 }).into()
}

/// The form: header and groups, at most `PAGE_COLUMN` wide and kept to the left (Swift `SettingsView.page`).
fn form(app: &App) -> Element<'_, Message> {
    let t = app.tokens();
    let s = &app.state.settings;

    let (note, glass_off) = glass_note(app);
    let glass = segmented(t, &[(true, "Glass"), (false, "Opaque")], Some(glass_selected(app)), (!glass_off).then_some(|v| Message::Settings(SettingsMsg::SetGlass(v))));
    let width = s.reading_width();
    let width_note = if width == ReadingWidth::Full { "the text fills the window.".to_string() } else { format!("a text column up to {} pt.", width.points() as u32) };
    let widths: Vec<(ReadingWidth, &'static str)> = ReadingWidth::ALL.iter().map(|w| (*w, w.label())).collect();
    let width_picker = segmented(t, &widths, Some(width), Some(|w| Message::Settings(SettingsMsg::SetWidth(w))));

    // The toggle: the saved value (off and locked for --read-only). The line: what this window does now.
    let live = access::writes_live(app);
    let mut input_toggle = toggler(s.allow_input() && !s.read_only_launch).size(18.0).style(move |_, st| toggle_style(t, st));
    if !s.read_only_launch {
        input_toggle = input_toggle.on_toggle(|v| Message::Settings(SettingsMsg::SetAllowInput(v)));
    }
    let footer = if s.read_only_launch {
        INPUT_LOCKED
    } else if live {
        INPUT_LIVE
    } else if s.allow_input() {
        INPUT_NEXT_LAUNCH
    } else {
        INPUT_OFF
    };
    let footer_color = if live { t.blocked } else { t.label_2 };
    let footer_row = row![icon(if live { Icon::Shield } else { Icon::Eye }, SIZE_INLINE, footer_color), CALLOUT.text(footer).color(t.label_2)].spacing(6).align_y(Alignment::Center);

    let sizes: Vec<_> = ReadingSize::ALL.iter().map(|v| (*v, v.label())).collect();
    let size_picker = segmented(t, &sizes, Some(s.reading_size()), Some(|v| Message::Settings(SettingsMsg::SetSize(v))));
    let colors_on = s.prefs.terminal_colors.unwrap_or_default();
    let colors = segmented(t, &[(TerminalColors::Flockbook, "Flockbook"), (TerminalColors::Ghostty, "Ghostty")], Some(colors_on), Some(|v| Message::Settings(SettingsMsg::SetTerminalColors(v))));
    // Like Swift (`SettingsView.general`): "Reload" is a small button beside the picker, and only while Ghostty supplies
    // the colors it would reload.
    let colors_control: Element<'_, Message> = if colors_on == TerminalColors::Ghostty {
        row![colors, small_button(t, "Reload", Some(Message::Settings(SettingsMsg::ReloadGhostty)))].spacing(SP_8).align_y(Alignment::Center).into()
    } else {
        colors
    };
    let plus = segmented(t, &[(PlusAction::NewTab, "New tab"), (PlusAction::SplitRight, "Split right"), (PlusAction::SplitDown, "Split down")], Some(s.prefs.plus_action.unwrap_or_default()), Some(|v| Message::Settings(SettingsMsg::SetPlusAction(v))));
    let theme_note = match &s.ghostty {
        Some(Ok(themes)) => format!("Light: {} · Dark: {}. {} skipped color lines.", themes.light.name, themes.dark.name, themes.skipped.len()),
        Some(Err(reason)) => format!("{reason} Using Flockbook colors."),
        None => "Read colors from your Ghostty config. Reload after changing it.".into(),
    };
    // Every group's line of small text goes under its card, like Swift's `SettingsGroup` footer.
    let mut body = column![page_header(t, s.tab),
        section(t, "Appearance", Some(note.to_string()), vec![setting_row(t, "Window", glass)]),
        section(t, "Reading", Some(format!("Text size: ⌘= larger, ⌘− smaller, ⌘0 reset. Width: {width_note}")), vec![setting_row(t, "Text size", size_picker), setting_row(t, "Width", width_picker)]),
        section(t, "Terminal", Some(theme_note), vec![setting_row(t, "Colors", colors_control)]),
        section(t, "Toolbar", Some("A split splits the focused pane. ⌘T, the menu's New Tab and the sidebar's New tab always open a tab.".into()), vec![setting_row(t, "+ button", plus)]),
    ].spacing(SP_16);
    if s.tab == SettingsTab::Agents {
        body = column![page_header(t, s.tab),
            // Swift `SettingsGroup(footer:)`: the line under the Input card, 4 pt below it.
            column![
                section(t, "Input", None, vec![setting_row(t, "Allow sending input to agents", input_toggle.into())]),
                container(footer_row).padding([0, 4]),
            ].spacing(4),
            section(t, "Status line", None, vec![
                setting_row(t, "Claude Code", segmented(t, &[(true, "On"), (false, "Off")], s.status_enabled.get("claude").copied(), Some(|on| Message::Settings(SettingsMsg::SetStatusLine { agent: "claude".into(), on })))),
                setting_row(t, "Grok Build", segmented(t, &[(true, "On"), (false, "Off")], s.status_enabled.get("grok").copied(), Some(|on| Message::Settings(SettingsMsg::SetStatusLine { agent: "grok".into(), on })))),
            ]),
            CALLOUT.text("Capture status tags; Off restores the original command. Unsafe or complex TOML is refused; any error appears below. Choose On or Off to apply setup.").color(t.label_2),
        ].spacing(SP_16);
    } else if s.tab == SettingsTab::Remote {
        body = column![page_header(t, s.tab), BODY.text("Remote access is outside this desktop parity release.").color(t.label), CAPTION.text("This build does not start a Hub or expose a remote input endpoint.").color(t.label_2)].spacing(SP_12);
    } else if s.tab == SettingsTab::Updates {
        body = column![page_header(t, s.tab), BODY.text("Updates are installed manually in this build.").color(t.label), CAPTION.text("Automatic update checks are outside this desktop parity release.").color(t.label_2)].spacing(SP_12);
    }
    for line in [s.pending.map(str::to_string), s.error.clone()].into_iter().flatten() {
        body = body.push(container(CAPTION.text(line).color(t.label_2)).padding([0, 12]));
    }

    // Swift: `.frame(maxWidth: column)`, leading and trailing margin 40, top 16, bottom 24.
    container(body).max_width(PAGE_COLUMN + 2.0 * PAGE_MARGIN).padding(iced::Padding { top: 16.0, right: PAGE_MARGIN, bottom: 24.0, left: PAGE_MARGIN }).into()
}

#[cfg(test)]
mod parity_tests {
    use super::*;

    #[test]
    fn old_settings_keep_defaults_and_new_preferences_round_trip() {
        let prefs: Prefs = serde_json::from_str(r#"{"glass":false,"allow_input":false}"#).unwrap();
        assert_eq!(prefs.reading_size.unwrap_or_default(), ReadingSize::M);
        assert_eq!(prefs.terminal_colors.unwrap_or_default(), TerminalColors::Flockbook);
        assert_eq!(prefs.plus_action.unwrap_or_default(), PlusAction::NewTab);
        let prefs = Prefs { reading_size: Some(ReadingSize::Xl), terminal_colors: Some(TerminalColors::Ghostty), sidebar_hidden: Some(true), ..prefs };
        assert_eq!(serde_json::from_str::<Prefs>(&serde_json::to_string(&prefs).unwrap()).unwrap(), prefs);
    }

    #[test]
    fn reading_size_steps_clamp_and_width_scales() {
        assert_eq!(ReadingSize::S.smaller(), ReadingSize::S);
        assert_eq!(ReadingSize::Xl.larger(), ReadingSize::Xl);
        assert_eq!(ReadingSize::M.body(), 16.0);
        assert_eq!(ReadingSize::Xl.column(ReadingWidth::Medium), 950.0);
    }
}

/// Host route guard. Async completion messages still settle sends started before Settings opened.
pub fn blocks_underlying(message: &Message) -> bool {
    use crate::composer::ComposerMsg;
    use crate::terminal::TerminalMsg;
    crate::terminal::blocks_underlying(message) || matches!(message,
        Message::Navigate(_) | Message::SelectWorkspace(_) | Message::SelectTab { .. } | Message::SelectPane(_)
        | Message::Workspace(_) | Message::Sidebar(_) | Message::TopRow(_) | Message::Overview(_) | Message::NeedsYou(_) | Message::Search(_)
        | Message::Composer(ComposerMsg::Attach | ComposerMsg::AttachFolder | ComposerMsg::Paste | ComposerMsg::Drop(_) | ComposerMsg::RemoveImage(_) | ComposerMsg::Pull | ComposerMsg::RecoverFailed)
        | Message::Terminal(TerminalMsg::Toggle | TerminalMsg::Input(_) | TerminalMsg::Paste(_) | TerminalMsg::Resized(..) | TerminalMsg::Wheel { .. } | TerminalMsg::TakeOver | TerminalMsg::Focus(_))
    )
}
