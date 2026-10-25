//! Window chrome: the title bar per platform.
//!
//! - macOS: no visible title bar, like the Swift app. The title bar is transparent, the title is hidden and the
//!   content runs to the top of the window. The sidebar starts at the top; its first 52 pt are empty, and the
//!   traffic lights sit there. With the title bar gone, the empty parts of the top row and of the sidebar top move
//!   the window (drag) and zoom it (double-click), the way a title bar does.
//! - Windows and Linux: the native title bar stays. Our own decorations would need resize borders, snap layouts,
//!   the Wayland decoration protocol and the window menu re-done by hand, and they would break on some compositors.
//!   The native bar drags, resizes and snaps the way users of each system expect. So no drag areas there: a drag
//!   inside the content would be a behavior these systems do not have.

use iced::widget::mouse_area;
use iced::{window, Element, Task};

use crate::glass::Os;
use crate::{App, Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleBar {
    /// The system title bar with the window title.
    #[default]
    Native,
    /// macOS: transparent, no title, the content under it.
    Hidden,
}

pub fn choose(os: Os) -> TitleBar {
    match os {
        Os::MacOs => TitleBar::Hidden,
        Os::Windows | Os::Other => TitleBar::Native,
    }
}

#[derive(Debug, Default)]
pub struct ChromeState {
    pub title_bar: TitleBar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChromeMsg {
    /// A press on an empty part of the top: move the window.
    Drag,
    /// A double-click there: zoom, as on a title bar.
    Zoom,
}

pub fn update(app: &mut App, msg: ChromeMsg) -> Task<Message> {
    if app.state.chrome.title_bar != TitleBar::Hidden {
        return Task::none();
    }
    match msg {
        ChromeMsg::Drag => window::oldest().and_then(window::drag),
        ChromeMsg::Zoom => window::oldest().and_then(window::toggle_maximize),
    }
}

/// The empty band at the top of the sidebar: the traffic lights and a window drag area.
pub const SIDEBAR_TOP_H: f32 = 52.0;

/// Makes `content` (empty space at the top of the window) act as the title bar when the native one is hidden.
pub fn drag_area<'a>(app: &App, content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    if app.state.chrome.title_bar == TitleBar::Hidden {
        mouse_area(content).on_press(Message::Chrome(ChromeMsg::Drag)).on_double_click(Message::Chrome(ChromeMsg::Zoom)).into()
    } else {
        content.into()
    }
}

/// The window settings for the title bar.
pub fn window_settings(title_bar: TitleBar, settings: window::Settings) -> window::Settings {
    #[cfg(target_os = "macos")]
    {
        if title_bar == TitleBar::Hidden {
            return window::Settings {
                platform_specific: window::settings::PlatformSpecific { title_hidden: true, titlebar_transparent: true, fullsize_content_view: true },
                ..settings
            };
        }
    }
    let _ = title_bar;
    settings
}

/// Swift WorkspaceMonogram, including its single-word fallback.
pub fn monogram(raw: &str) -> String {
    let stripped = if raw.starts_with('[') { raw.split_once(']').map(|(_, s)| s.trim()).unwrap_or(raw) } else { raw };
    let s = if stripped.is_empty() { raw } else { stripped };
    let chars: Vec<char> = s.chars().collect();
    let s: String = chars.iter().enumerate().filter_map(|(i, &c)| {
        let inner = matches!(c, '\'' | '’') && i > 0 && i + 1 < chars.len() && chars[i - 1].is_alphanumeric() && chars[i + 1].is_alphanumeric();
        (!inner).then_some(c)
    }).collect();
    let mut words: Vec<String> = s.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()).map(str::to_owned).collect();
    if words.len() == 1 {
        let mut parts = vec![];
        let mut cur = String::new();
        for c in words[0].chars() {
            if c.is_uppercase() && !cur.is_empty() { parts.push(std::mem::take(&mut cur)); }
            cur.push(c);
        }
        parts.push(cur); words = parts;
    }
    if words.len() >= 2 { words.iter().take(2).filter_map(|s| s.chars().next()).collect::<String>().to_uppercase() }
    else { s.chars().take(2).collect::<String>().to_uppercase() }
}

/// A quiet icon button, the one the sidebar footer and the top row use: an icon on the sidebar, a hover fill, never
/// the accent. `tip` is the tooltip, as Swift gives each chrome button ("Hide sidebar" / "Show sidebar").
fn quiet_icon<'a>(t: fb_ui::theme::Tokens, ic: fb_ui::icons::Icon, size: f32, msg: Message, tip: &str) -> Element<'a, Message> {
    use iced::widget::{button, container, tooltip};
    use iced::{Background, Border, Length};
    let b = button(container(fb_ui::icons::icon(ic, size, t.label_2)).center(Length::Fixed(30.0)))
        .padding(0)
        .on_press(msg)
        .style(move |_, st| button::Style {
            background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.fill_2)),
            border: Border { radius: fb_ui::theme::R_ROW.into(), ..Default::default() },
            ..Default::default()
        });
    tooltip(b, container(fb_ui::theme::CALLOUT.text(tip.to_string()).color(t.label)).padding([5, 9]).style(crate::style::float_e2(t, fb_ui::theme::R_ROW)), tooltip::Position::Bottom)
        .gap(6)
        .into()
}

/// Sidebar container for the integration host. A collapsed rail retains the restore button and Settings access.
pub fn sidebar(app: &App) -> Element<'_, Message> {
    use fb_ui::theme::*;
    use fb_ui::icons::{Icon, SIZE_DEFAULT};
    use iced::widget::{container, row, stack, Space};
    use iced::{Alignment, Length};
    let t = app.tokens();
    let hidden = app.state.settings.sidebar_hidden();
    let toggle = quiet_icon(t, Icon::PanelLeft, SIZE_DEFAULT, Message::Settings(crate::settings::SettingsMsg::ToggleSidebar), if hidden { "Show sidebar" } else { "Hide sidebar" });
    let content: Element<'_, Message> = if hidden {
        // The rail: restore, then Settings. Quiet icons, as in Swift's toolbar toggle.
        let settings = quiet_icon(t, Icon::Settings2, SIZE_DEFAULT, Message::Settings(crate::settings::SettingsMsg::Open), "Settings (\u{2318},)");
        iced::widget::column![iced::widget::row![toggle].spacing(SP_4), iced::widget::row![settings].spacing(SP_4)].spacing(SP_4).padding([10, 9]).into()
    } else {
        // The sidebar's own empty top band is the window's title bar on macOS (`sidebar::view` keeps it 52 pt tall and
        // empty). The toggle rides on that band instead of pushing it down, so the empty part stays a drag area.
        let band = container(row![container(toggle).padding([6, 12]), Space::new().width(Length::Fill)]).height(Length::Fixed(SIDEBAR_TOP_H));
        stack![crate::sidebar::view(app), container(band).width(Length::Fill).align_y(Alignment::Start)].into()
    };
    container(content).width(Length::Fixed(if hidden { 48.0 } else { SIDEBAR_W })).height(Length::Fill)
        .style(crate::style::sidebar(t, app.state.glass.on)).into()
}

#[cfg(test)]
mod parity_tests {
    use super::*;
    #[test]
    fn workspace_initials_follow_words_camel_case_and_apostrophes() {
        for (label, want) in [("[TAG] Meadow · Migration", "MM"), ("OpenAtlas", "OA"), ("Brandon's project", "BP"), ("O’Neil", "ON"), ("hello", "HE"), ("研究 工作", "研工"), ("", "")] {
            assert_eq!(monogram(label), want, "{label}");
        }
    }
}
