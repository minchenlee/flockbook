//! Flockbook binary.
//!
//! Normal run: opens a real window (for the owner only).
//! Headless: `flockbook --snapshot out.png --screen conversation|overview [--theme light|dark] [--tab LABEL] [--query TEXT] [--terminal] [--glass on] [--size 1280x852] [--demo]`
//! renders one demo frame offscreen and exits. It never opens a window.
//! `--glass on|off` (or env `FLOCKBOOK_GLASS`): window glass, on by default on macOS and Windows 11, always off on Linux.
//! `--terminal` opens the Show terminal sheet on demo frames (fake backend, no process). `--read-only` (or env `FLOCKBOOK_READ_ONLY=1`): nothing is ever sent to a pane; the composer and the cards only read.

use fb_ui::Mode;
use flockbook::state::Screen;
use flockbook::{snapshot, App, Message};
use iced::{time, Size, Subscription, Task, Theme};
use std::time::Duration;

fn arg_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn main() -> iced::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(agent) = arg_value(&args, "--flockbook-statusline") {
        std::process::exit(herdr_core::status_line::wrapper(&agent));
    }

    if let Some(out) = arg_value(&args, "--snapshot") {
        let screen = arg_value(&args, "--screen").and_then(|s| Screen::parse(&s)).unwrap_or(Screen::Conversation);
        let mode = arg_value(&args, "--theme").and_then(|s| Mode::parse(&s)).unwrap_or(Mode::Light);
        let size = arg_value(&args, "--size").and_then(|s| snapshot::parse_size(&s)).unwrap_or((1280, 852));
        // Snapshots always use the demo source: they never touch a live herdr.
        // `--tab LABEL` shows that demo tab (e.g. `deploy` or `getting started`: the two blocked ones show a card).
        // `--query TEXT` types TEXT into Search (all demo conversations are loaded first).
        // `--terminal` opens the sheet over the conversation, on the demo shell screen. With `--read-only` it is the Observe look.
        let terminal = args.iter().any(|a| a == "--terminal");
        // `--glass on` draws the window-glass look over a stand-in wallpaper.
        let glass = arg_value(&args, "--glass").is_some_and(|v| v == "on");
        let read_only = args.iter().any(|a| a == "--read-only");
        let mut app = if terminal {
            flockbook::terminal::demo_app(mode, screen, !read_only)
        } else if let Some(q) = arg_value(&args, "--query") {
            let mut app = App::demo(mode, Screen::Search);
            flockbook::search::preload_all(&mut app);
            let _ = app.update(Message::Search(flockbook::search::SearchMsg::Open));
            let _ = app.update(Message::Search(flockbook::search::SearchMsg::Input(q)));
            app
        } else {
            match arg_value(&args, "--tab") {
                Some(label) => flockbook::cards::demo_app_on_tab(mode, screen, &label),
                None => App::demo(mode, screen),
            }
        };
        app.state.glass.on = glass;
        let img = snapshot::render_app(&app, size);
        match snapshot::write_png(&img, std::path::Path::new(&out)) {
            Ok(()) => {
                println!("wrote {out} ({}x{})", img.width, img.height);
                return Ok(());
            }
            Err(e) => {
                eprintln!("cannot write {out}: {e}");
                std::process::exit(1);
            }
        }
    }

    let demo = args.iter().any(|a| a == "--demo");
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let settings_path = flockbook::settings::default_path();
    let prefs = settings_path.as_deref().map(flockbook::settings::Prefs::load).unwrap_or_default();
    let backdrop = flockbook::glass::choose_with(flockbook::glass::Os::current(), &args, &env, prefs.glass);
    // Before the first frame: CJK text uses a good system face at every weight.
    fb_ui::cjk::install();
    iced::application(move || boot(demo, &args, backdrop, settings_path.clone(), prefs.clone()), App::update, App::view)
        .title(App::title)
        .theme(|app: &App| -> Theme { app.theme() })
        .style(|app: &App, theme: &Theme| iced::theme::Style { background_color: flockbook::glass::root_color(app), text_color: theme.palette().text })
        .subscription(|app: &App| -> Subscription<Message> { Subscription::batch([time::every(Duration::from_millis(800)).map(|_| Message::Tick), flockbook::toprow::subscription(app), flockbook::needs_you::subscription(app), flockbook::conversation::subscription(app), flockbook::search::subscription(app), flockbook::terminal::subscription(app), flockbook::glass::subscription(app), flockbook::shortcuts::subscription(app), flockbook::composer::subscription(app)]) })
        .font(fb_ui::fonts::BUNDLED[0])
        .font(fb_ui::fonts::BUNDLED[1])
        .font(fb_ui::fonts::BUNDLED[2])
        .font(fb_ui::fonts::BUNDLED[3])
        .font(fb_ui::fonts::BUNDLED[4])
        .default_font(fb_ui::fonts::default_font())
        .window(flockbook::chrome::window_settings(
            flockbook::chrome::choose(flockbook::glass::Os::current()),
            iced::window::Settings {
                size: Size::new(1280.0, 852.0),
                min_size: Some(Size::new(900.0, 560.0)),
                transparent: flockbook::glass::launch_transparent(backdrop),
                ..Default::default()
            },
        ))
        .run()
}

fn boot(demo: bool, args: &[String], backdrop: flockbook::glass::Backdrop, settings_path: Option<std::path::PathBuf>, prefs: flockbook::settings::Prefs) -> (App, Task<Message>) {
    let allow_input = prefs.allow_input;
    let mut app = if demo {
        App::demo(Mode::Light, Screen::Conversation)
    } else {
        live_app(args, allow_input)
    };
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    app.state.settings.read_only_launch = !flockbook::access::writes_enabled(args, &env, Some(true));
    app.state.settings.prefs = prefs;
    if app.state.settings.prefs.terminal_colors == Some(flockbook::settings::TerminalColors::Ghostty) {
        app.state.settings.ghostty = Some(flockbook::settings::ghostty_theme::load_current());
    }
    app.state.settings.path = settings_path;
    // A real window opens links in the system browser. Snapshots and tests keep the recording opener.
    app.opener = Box::new(flockbook::links::SystemOpener);
    // Read-only also strips the demo's recording actions, so the demo shows the read-only look.
    if !writes_enabled(args, allow_input) {
        flockbook::access::make_read_only(&mut app);
    }
    flockbook::glass::install(&mut app, backdrop);
    app.state.chrome.title_bar = flockbook::chrome::choose(flockbook::glass::Os::current());
    (app, Task::none())
}

/// Swift `writesEnabled`: `--read-only`, `FLOCKBOOK_READ_ONLY=1` or the "Allow sending input" setting turn it off.
fn writes_enabled(args: &[String], allow_input: Option<bool>) -> bool {
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    flockbook::access::writes_enabled(args, &env, allow_input)
}

#[cfg(unix)]
fn live_app(args: &[String], allow_input: Option<bool>) -> App {
    // Read-only: the live source only subscribes to snapshots and events.
    let source = flockbook::source::LiveSource::from_args(args);
    let mut app = App::new(Box::new(source), Mode::Light, Screen::Conversation);
    let env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let session = herdr_core::HerdrSession::resolve(args, &env, &home);
    // The terminal sheet is always available. It opens in Observe mode (no input path) when writes are off.
    flockbook::terminal::install_live(&mut app, &session);
    flockbook::composer::install_observer(&mut app, &session);
    // Read-only: no sender and no pane actions are installed at all.
    if !writes_enabled(args, allow_input) {
        return app;
    }
    // The composer is the only mutating path (sends text to a pane). It is wired to the live source only.
    flockbook::composer::install_live(&mut app, &session);
    // Allow, Deny and question answers send keys to a pane: live path only.
    flockbook::cards::install_live(&mut app, &session);
    flockbook::workspace_actions::install_live(&mut app, &session);
    app
}

#[cfg(not(unix))]
fn live_app(_args: &[String], _allow_input: Option<bool>) -> App {
    eprintln!("live herdr source is not available on this platform yet; showing demo data");
    App::demo(Mode::Light, Screen::Conversation)
}
