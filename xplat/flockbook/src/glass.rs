//! Window glass per platform (DESIGN.md section 9: glass for navigation and bars, content stays legible).
//!
//! - macOS: an AppKit `NSVisualEffectView` (sidebar material, behind the window) under iced's layer.
//! - Windows 11: Mica (`window-vibrancy`).
//! - Linux and the rest: opaque. Blur there belongs to the compositor; there is no API.
//!
//! Glass is an enhancement. The UI is complete when opaque, and every failure (no backdrop, Reduce Transparency, an
//! older Windows) falls back to opaque. The window must be created transparent for glass, so `--glass` and
//! `FLOCKBOOK_GLASS` apply at launch.
//!
//! Under glass the root paints nothing. The sidebar paints `glass_side`, the top row and the Needs you list
//! `glass_bar`, and the main area `content` (almost opaque). Floating surfaces (composer, cards, menus, tooltips) stay
//! opaque: iced cannot blur the content under them, and a thin tint over text is unreadable.

use std::collections::HashMap;

use iced::{window, Color, Subscription, Task};

use crate::{App, Message};

/// What sits behind the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Backdrop {
    #[default]
    Opaque,
    /// macOS `NSVisualEffectView`.
    Vibrancy,
    /// Windows 11 Mica.
    Mica,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Other
        }
    }
}

/// `--glass on|off`, else `FLOCKBOOK_GLASS=on|off`, else on. "On" is the platform's backdrop; Linux has none.
/// Any other value counts as unset.
pub fn choose(os: Os, args: &[String], env: &HashMap<String, String>) -> Backdrop {
    choose_with(os, args, env, None)
}

/// [`choose`] with the saved setting between the env and the default.
pub fn choose_with(os: Os, args: &[String], env: &HashMap<String, String>, setting: Option<bool>) -> Backdrop {
    let parse = |v: &str| match v.trim().to_ascii_lowercase().as_str() {
        "on" | "1" | "true" => Some(true),
        "off" | "0" | "false" => Some(false),
        _ => None,
    };
    let flag = args.iter().position(|a| a == "--glass").and_then(|i| args.get(i + 1)).and_then(|v| parse(v));
    let wanted = flag.or_else(|| env.get("FLOCKBOOK_GLASS").and_then(|v| parse(v))).or(setting).unwrap_or(true);
    match (wanted, os) {
        (true, Os::MacOs) => Backdrop::Vibrancy,
        (true, Os::Windows) => Backdrop::Mica,
        _ => Backdrop::Opaque,
    }
}

#[derive(Debug, Default)]
pub struct GlassState {
    pub backdrop: Backdrop,
    /// The panels paint the glass tints and the root paints nothing. False until `install`, and after a failure.
    pub on: bool,
    pub reduced_transparency: bool,
    pub window: Option<window::Id>,
    /// The dark flag last pushed to the window.
    applied: Option<bool>,
    /// Why glass fell back to opaque.
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum GlassMsg {
    WindowOpened(window::Id),
    CheckAccessibility,
    Applied(Result<(), String>),
}

/// Turns glass on for a window created transparent (`main.rs`). Tests and snapshots set `on` directly.
pub fn install(app: &mut App, backdrop: Backdrop) {
    app.state.glass.backdrop = backdrop;
    app.state.glass.on = backdrop != Backdrop::Opaque;
}

/// True when the window must be created transparent.
pub fn launch_transparent(backdrop: Backdrop) -> bool {
    backdrop != Backdrop::Opaque
}

pub fn update(app: &mut App, msg: GlassMsg) -> Task<Message> {
    match msg {
        GlassMsg::CheckAccessibility => {
            let reduce = macos::reduce_transparency();
            if reduce != app.state.glass.reduced_transparency {
                let wanted = app.state.settings.prefs.glass.unwrap_or(true);
                let g = &mut app.state.glass;
                g.reduced_transparency = reduce;
                g.applied = None;
                if reduce {
                    g.on = false;
                    g.error = Some("Reduce Transparency is on".into());
                } else if g.error.as_deref() == Some("Reduce Transparency is on") {
                    g.error = None;
                    g.on = wanted && g.backdrop != Backdrop::Opaque;
                }
            }
        }
        GlassMsg::WindowOpened(id) => {
            if app.state.glass.window.is_none() {
                app.state.glass.window = Some(id);
            }
        }
        GlassMsg::Applied(Ok(())) => {}
        GlassMsg::Applied(Err(e)) => {
            eprintln!("flockbook: window glass unavailable, using opaque: {e}");
            app.state.glass.on = false;
            app.state.glass.reduced_transparency = e == "Reduce Transparency is on";
            app.state.glass.error = Some(e);
        }
    }
    Task::none()
}

/// Pushes the backdrop to the window when it is new or the theme changed. Called after every update.
pub fn sync(app: &mut App) -> Option<Task<Message>> {
    let g = &mut app.state.glass;
    let id = g.window?;
    if !g.on {
        return None;
    }
    let dark = app.state.mode.is_dark();
    if g.applied == Some(dark) {
        return None;
    }
    g.applied = Some(dark);
    let backdrop = g.backdrop;
    Some(window::run(id, move |w| apply(backdrop, w, dark)).map(|r| Message::Glass(GlassMsg::Applied(r))))
}

/// The first window's id, until it is known. Only while a backdrop waits for it.
pub fn subscription(app: &App) -> Subscription<Message> {
    let opened = if app.state.glass.on && app.state.glass.window.is_none() {
        window::open_events().map(|id| Message::Glass(GlassMsg::WindowOpened(id)))
    } else {
        Subscription::none()
    };
    if cfg!(target_os = "macos") && app.state.glass.backdrop != Backdrop::Opaque {
        Subscription::batch([opened, iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::Glass(GlassMsg::CheckAccessibility))])
    } else { opened }
}

/// The window's clear color: nothing under glass, the content color otherwise.
pub fn root_color(app: &App) -> Color {
    if app.state.glass.on {
        Color::TRANSPARENT
    } else {
        app.tokens().bg
    }
}

/// Runs on the main thread (iced `window::run`).
fn apply(backdrop: Backdrop, window: &dyn window::Window, dark: bool) -> Result<(), String> {
    match backdrop {
        Backdrop::Opaque => Ok(()),
        Backdrop::Vibrancy => macos::show(window, dark),
        // Windows 11 only; older Windows returns an error and the app goes opaque.
        Backdrop::Mica => window_vibrancy::apply_mica(window, Some(dark)).map_err(|e| e.to_string()),
    }
}

#[cfg(not(target_os = "macos"))]
mod macos {
    pub fn reduce_transparency() -> bool { false }
    pub fn show(_window: &dyn iced::window::Window, _dark: bool) -> Result<(), String> {
        Err("window vibrancy is macOS only".into())
    }
}

/// Port of rmdv's `macos_vibrancy` (MIT): the blur view sits beside iced's content view in the window
/// frame view, ordered below it, so the Metal layer composites on top.
#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;

    use iced::window::raw_window_handle::RawWindowHandle;
    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
        NSAutoresizingMaskOptions, NSColor, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode, NSWorkspace,
    };

    thread_local! {
        /// The installed blur view. AppKit objects live on the main thread.
        static BLUR: RefCell<Option<Retained<NSVisualEffectView>>> = const { RefCell::new(None) };
    }

    pub fn reduce_transparency() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
    }

    pub fn show(window: &dyn iced::window::Window, dark: bool) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("not on the main thread")?;
        BLUR.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                // Reduce Transparency: the tints would still be translucent, so stay opaque.
                if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency() {
                    return Err("Reduce Transparency is on".to_string());
                }
                *slot = Some(install(window, mtm)?);
            }
            let blur = slot.as_ref().expect("installed above");
            // The material follows the app theme, not the system appearance.
            let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
            blur.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
            Ok(())
        })
    }

    fn install(window: &dyn iced::window::Window, mtm: MainThreadMarker) -> Result<Retained<NSVisualEffectView>, String> {
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return Err("not an AppKit window".into());
        };
        // SAFETY: winit's AppKit handle points at the window's live content view, and we are on the main thread.
        let content: &NSView = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
        let frame_view = unsafe { content.superview() }.ok_or("content view has no superview")?;
        let ns_window = content.window().ok_or("content view has no window")?;
        let blur = NSVisualEffectView::initWithFrame(mtm.alloc(), frame_view.bounds());
        blur.setMaterial(NSVisualEffectMaterial::Sidebar);
        blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        blur.setState(NSVisualEffectState::FollowsWindowActiveState);
        blur.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        frame_view.addSubview_positioned_relativeTo(&blur, NSWindowOrderingMode::Below, Some(content));
        ns_window.setOpaque(false);
        ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
        Ok(blur)
    }
}
