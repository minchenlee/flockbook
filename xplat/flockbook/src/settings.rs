//! Settings (port of Swift `SettingsView`): window glass, reading width, and "Allow sending input to agents".
//!
//! Swift opens a separate fixed-size window; here it is a sheet over the app (one window). It opens with Cmd+,
//! (Ctrl+, on Windows and Linux) and from the gear at the bottom of the sidebar. Esc, the close button or a click
//! outside closes it.
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

use fb_ui::icons::{icon, Icon, SIZE_INLINE};
use fb_ui::theme::*;
use iced::widget::{button, column, container, mouse_area, responsive, row, scrollable, stack, text, toggler, Space};
use iced::{Alignment, Background, Border, Element, Length, Task};
use serde::{Deserialize, Serialize};

#[path = "ghostty_theme.rs"]
pub mod ghostty_theme;
#[path = "status_tags.rs"]
pub mod status_tags;
#[path = "settings_cover.rs"]
mod settings_cover;
pub use settings_cover::cover;

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
}

impl SettingsState {
    pub fn reading_width(&self) -> ReadingWidth {
        self.prefs.reading_width.unwrap_or_default()
    }

    pub fn reading_size(&self) -> ReadingSize { self.prefs.reading_size.unwrap_or_default() }
    pub fn column_width(&self) -> f32 { self.reading_size().column(self.reading_width()) }
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

pub const SHEET_W: f32 = 520.0;

fn segmented<'a, T: Copy + PartialEq + 'a>(t: Tokens, items: &[(T, &'static str)], selected: Option<T>, on: Option<fn(T) -> Message>) -> Element<'a, Message> {
    let mut r = row![].spacing(2);
    for &(value, label) in items {
        let sel = selected == Some(value);
        let color = if on.is_none() { t.label_3 } else { t.label };
        let mut b = button(container(text(label).size(CALLOUT.size).color(color)).center_x(Length::Fill)).width(Length::Fill).padding([3, 6]).style(move |_, st| button::Style {
            background: if sel {
                Some(Background::Color(t.bg))
            } else {
                matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_2))
            },
            border: Border { radius: 5.0.into(), width: if sel { 0.5 } else { 0.0 }, color: t.sep_strong },
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

fn setting_row<'a>(t: Tokens, title: &'a str, note: String, control: Element<'a, Message>) -> Element<'a, Message> {
    container(
        column![BODY.text(title).color(t.label), CAPTION.text(note).color(t.label_2), control]
            .spacing(6).width(Length::Fill),
    )
    .padding([10, 12])
    .into()
}

fn section<'a>(t: Tokens, title: &'a str, rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut col = column![].spacing(0);
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            col = col.push(container(Space::new()).width(Length::Fill).height(Length::Fixed(1.0)).style(style::fill(t.sep, 0.0)));
        }
        col = col.push(r);
    }
    column![container(CAPTION.text(title).color(t.label_2)).padding([0, 12]), container(col).style(style::fill(t.fill_1, R_ROW + 4.0))].spacing(6).into()
}

/// The sheet, or nothing when closed.
pub fn layer(app: &App) -> Element<'_, Message> {
    if !app.state.settings.open {
        return Space::new().into();
    }
    responsive(move |size| sheet(app, size.height)).into()
}

fn sheet(app: &App, available_height: f32) -> Element<'_, Message> {
    let t = app.tokens();
    let s = &app.state.settings;

    let (note, glass_off) = glass_note(app);
    let glass = segmented(t, &[(true, "Glass"), (false, "Opaque")], Some(glass_selected(app)), (!glass_off).then_some(|v| Message::Settings(SettingsMsg::SetGlass(v))));
    let width = s.reading_width();
    let width_note = if width == ReadingWidth::Full { "The text fills the window.".to_string() } else { format!("Text column up to {} pt.", width.points() as u32) };
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
    let footer_row = row![icon(if live { Icon::Shield } else { Icon::Eye }, SIZE_INLINE, footer_color), CAPTION.text(footer).color(t.label_2)].spacing(6).align_y(Alignment::Center);

    let sizes: Vec<_> = ReadingSize::ALL.iter().map(|v| (*v, v.label())).collect();
    let size_picker = segmented(t, &sizes, Some(s.reading_size()), Some(|v| Message::Settings(SettingsMsg::SetSize(v))));
    let colors = segmented(t, &[(TerminalColors::Flockbook, "Flockbook"), (TerminalColors::Ghostty, "Ghostty")], Some(s.prefs.terminal_colors.unwrap_or_default()), Some(|v| Message::Settings(SettingsMsg::SetTerminalColors(v))));
    let plus = segmented(t, &[(PlusAction::NewTab, "New tab"), (PlusAction::SplitRight, "Split right"), (PlusAction::SplitDown, "Split down")], Some(s.prefs.plus_action.unwrap_or_default()), Some(|v| Message::Settings(SettingsMsg::SetPlusAction(v))));
    let theme_note = match &s.ghostty {
        Some(Ok(themes)) => format!("Light: {} · Dark: {}. {} skipped color lines.", themes.light.name, themes.dark.name, themes.skipped.len()),
        Some(Err(reason)) => format!("{reason} Using Flockbook colors."),
        None => "Read colors from your Ghostty config. Reload after changing it.".into(),
    };
    let mut body = column![
        section(t, "Appearance", vec![setting_row(t, "Window", note.to_string(), glass)]),
        section(t, "Reading", vec![setting_row(t, "Text size", "Larger text also widens the reading column.".into(), size_picker), setting_row(t, "Width", width_note, width_picker)]),
        section(t, "Terminal", vec![setting_row(t, "Colors", theme_note, colors), button(CALLOUT.text("Reload Ghostty colors")).on_press(Message::Settings(SettingsMsg::ReloadGhostty)).into()]),
        section(t, "Toolbar", vec![setting_row(t, "+ button", "Keyboard New Tab always opens a tab.".into(), plus)]),
    ].spacing(SP_16);
    if s.tab == SettingsTab::Agents {
        body = column![
            section(t, "Input", vec![setting_row(t, "Allow sending input to agents", "Typing, Allow and Deny, new tab, close tab and split.".into(), input_toggle.into())]),
            container(footer_row).padding([0, 12]),
            section(t, "Status line", vec![
                setting_row(t, "Claude Code", "Capture status tags; Off restores the original command.".into(), segmented(t, &[(true, "On"), (false, "Off")], s.status_enabled.get("claude").copied(), Some(|on| Message::Settings(SettingsMsg::SetStatusLine { agent: "claude".into(), on })))),
                setting_row(t, "Grok Build", "Unsafe or complex TOML is refused; any error appears below.".into(), segmented(t, &[(true, "On"), (false, "Off")], s.status_enabled.get("grok").copied(), Some(|on| Message::Settings(SettingsMsg::SetStatusLine { agent: "grok".into(), on })))),
            ]),
            CAPTION.text("Choose On or Off to apply setup. Highlighted choices show successful actions in this window.").color(t.label_2),
        ].spacing(SP_16);
    } else if s.tab == SettingsTab::Remote {
        body = column![BODY.text("Remote access is outside this desktop parity release.").color(t.label), CAPTION.text("This build does not start a Hub or expose a remote input endpoint.").color(t.label_2)].spacing(SP_12);
    } else if s.tab == SettingsTab::Updates {
        body = column![BODY.text("Updates are installed manually in this build.").color(t.label), CAPTION.text("Automatic update checks are outside this desktop parity release.").color(t.label_2)].spacing(SP_12);
    }
    for line in [s.pending.map(str::to_string), s.error.clone()].into_iter().flatten() {
        body = body.push(container(CAPTION.text(line).color(t.label_2)).padding([0, 12]));
    }

    let close = button(container(icon(Icon::X, SIZE_INLINE, t.label_2)).center(Length::Fixed(28.0))).padding(0).on_press(Message::Settings(SettingsMsg::Close)).style(move |_, st| button::Style {
        background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_2)),
        border: Border { radius: 14.0.into(), ..Default::default() },
        ..Default::default()
    });
    let header = row![HEADLINE.text("Settings").color(t.label), Space::new().width(Length::Fill), close].align_y(Alignment::Center);
    let tabs: Vec<_> = SettingsTab::ALL.iter().map(|v| (*v, v.label())).collect();
    let tabs = segmented(t, &tabs, Some(s.tab), Some(|v| Message::Settings(SettingsMsg::SelectTab(v))));
    let card = container(column![header, tabs, scrollable(body).height(Length::Fill)].spacing(SP_12))
        .padding(SP_16).width(Length::Fill).max_width(SHEET_W)
        .height(Length::Fixed((available_height - TOPROW_H - 24.0).max(1.0)))
        .style(style::float_e2(t, R_CARD));
    // The card swallows its own clicks; the scrim around it closes the sheet.
    let card = mouse_area(card).on_press(Message::Noop);
    let scrim = mouse_area(container(Space::new()).width(Length::Fill).height(Length::Fill).style(style::fill(iced::Color { a: 0.18, ..iced::Color::BLACK }, 0.0))).on_press(Message::Settings(SettingsMsg::Close));
    let placed = container(card).width(Length::Fill).height(Length::Fill).align_x(Alignment::Center).padding(iced::Padding { top: TOPROW_H + 24.0, ..iced::Padding::ZERO });
    stack![scrim, placed].into()
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
