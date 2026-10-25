//! The Keyboard Shortcuts panel and the Settings sheet: the table, the key dispatch, the entry points, the saved
//! values and what each setting changes. Headless (iced_test Simulator, tiny-skia). Settings are kept in memory or in
//! a temp folder: the user's real settings file is never touched.
//! Set `FB_WRITE_EVIDENCE=1` to copy the PNGs to `evidence/`.

use std::collections::HashMap;

use fb_ui::Mode;
use flockbook::glass::{self, Backdrop, Os};
use flockbook::settings::{self, Prefs, ReadingWidth, SettingsMsg};
use flockbook::shortcuts::{self, Action, Combo, Keys, K, TABLE};
use flockbook::snapshot::{render_app, write_png};
use flockbook::state::Screen;
use flockbook::{App, Message};
use iced::keyboard::{key::Named, Key, Modifiers};
use iced::{Point, Settings, Size};
use iced_test::{simulator, Simulator};

const SIZE: (f32, f32) = (1280.0, 852.0);

fn sim(app: &App) -> Simulator<'_, Message> {
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
    Simulator::with_size(settings, Size::new(SIZE.0, SIZE.1), app.view())
}

fn apply(app: &mut App, msgs: Vec<Message>) {
    for m in msgs {
        let _ = app.update(m);
    }
}

fn key(app: &mut App, combo: Combo) {
    if let Some(m) = shortcuts::on_key(combo.key(), combo.modifiers()) {
        let _ = app.update(m);
    }
}

/// Every module's key handler, as `main.rs` subscribes them (search not on its own screen).
fn handlers(key: &Key, mods: Modifiers) -> Vec<Message> {
    [flockbook::toprow::on_key(key.clone(), mods), flockbook::search::on_key(key.clone(), mods, false), shortcuts::on_key(key.clone(), mods)].into_iter().flatten().collect()
}

fn combo(k: &'static str, cmd: bool, shift: bool) -> Combo {
    Combo { key: K::Char(k, k), cmd, ctrl: false, shift, alt: false }
}

// ------------------------------------------------------------ the table

#[test]
fn every_listed_shortcut_reaches_exactly_one_handler() {
    for s in TABLE {
        let Keys::Combo(c) = s.keys else { continue };
        let got = handlers(&c.key(), c.modifiers());
        assert_eq!(got.len(), 1, "{} ({}): {got:?}", s.label, c.text(cfg!(target_os = "macos")));
        if let Some(a) = s.action {
            assert!(matches!(got[0], Message::Shortcut(x) if x == a), "{}: {got:?}", s.label);
        }
    }
}

#[test]
fn tab_numbers_and_escape_work_as_listed() {
    let cmd = if cfg!(target_os = "macos") { Modifiers::LOGO } else { Modifiers::CTRL };
    for n in 1..=9 {
        assert_eq!(handlers(&Key::Character(n.to_string().into()), cmd).len(), 1, "Cmd+{n}");
    }
    assert!(handlers(&Key::Named(Named::Escape), Modifiers::empty()).iter().any(|m| matches!(m, Message::Shortcut(Action::Dismiss))));
}

/// Ctrl+` with the command key held too. On Linux and Windows Ctrl IS the command key, so this is plain Ctrl+`
/// there; it used to fall into the Cmd+digit arm, which returned early on a non-digit.
#[test]
fn ctrl_backtick_toggles_the_terminal_when_ctrl_is_the_command_key() {
    let m = flockbook::toprow::on_key(Key::Character("`".into()), Modifiers::CTRL | Modifiers::COMMAND);
    assert!(matches!(m, Some(Message::TopRow(flockbook::toprow::TopRowMsg::ToggleTerminal))), "{m:?}");
    // A command-key letter the top row does not own still reaches nothing here (and Cmd+1 still selects a tab).
    assert!(flockbook::toprow::on_key(Key::Character("k".into()), Modifiers::COMMAND).is_none());
    assert!(flockbook::toprow::on_key(Key::Character("1".into()), Modifiers::COMMAND).is_some());
}

#[test]
fn keys_read_the_platform_way() {
    let new_ws = TABLE.iter().find(|s| s.label == "New Workspace").unwrap();
    assert_eq!(shortcuts::keys_text(new_ws, true), "⇧⌘N");
    assert_eq!(shortcuts::keys_text(new_ws, false), "Ctrl+Shift+N");
    let terminal = TABLE.iter().find(|s| s.label == "Show Terminal").unwrap();
    assert_eq!(shortcuts::keys_text(terminal, true), "⌃`");
    assert_eq!(shortcuts::keys_text(terminal, false), "Ctrl+`");
    let settings_row = TABLE.iter().find(|s| s.action == Some(Action::Settings)).unwrap();
    assert_eq!(shortcuts::keys_text(settings_row, true), "⌘,");
    assert_eq!(shortcuts::keys_text(settings_row, false), "Ctrl+,");
}

#[test]
fn shifted_brackets_switch_tabs() {
    // Shift turns "]" into "}" on a US layout: both match.
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let before = app.state.selected_tab.clone();
    let mods = combo("]", true, true).modifiers();
    let m = shortcuts::on_key(Key::Character("}".into()), mods).expect("Shift+Cmd+]");
    assert!(matches!(m, Message::Shortcut(Action::NextTab)));
    let _ = app.update(m);
    assert_ne!(app.state.selected_tab, before);
}

#[test]
fn search_overview_and_needs_you_keys_navigate() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    key(&mut app, combo("k", true, false));
    assert_eq!(app.state.screen, Screen::Search);
    key(&mut app, combo("\\", true, true));
    assert_eq!(app.state.screen, Screen::Overview);
    key(&mut app, combo("i", true, true));
    assert_eq!(app.state.screen, Screen::NeedsYou);
}

// ------------------------------------------------------------ the panel

#[test]
fn cmd_slash_toggles_the_panel_and_escape_closes_it() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    key(&mut app, combo("/", true, false));
    assert!(app.state.shortcuts.open);
    key(&mut app, combo("/", true, false));
    assert!(!app.state.shortcuts.open);
    key(&mut app, combo("/", true, false));
    let _ = app.update(shortcuts::on_key(Key::Named(Named::Escape), Modifiers::empty()).unwrap());
    assert!(!app.state.shortcuts.open);
}

#[test]
fn the_panel_lists_every_row() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let _ = app.update(Message::Shortcut(Action::Shortcuts));
    let mut s = sim(&app);
    assert!(s.find("Keyboard Shortcuts").is_ok());
    let mac = cfg!(target_os = "macos");
    for row in TABLE {
        assert!(s.find(row.label).is_ok(), "label {}", row.label);
        assert!(s.find(shortcuts::keys_text(row, mac).as_str()).is_ok(), "keys of {}", row.label);
    }
}

#[test]
fn a_click_outside_closes_the_panel_and_inside_does_not() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let _ = app.update(Message::Shortcut(Action::Shortcuts));
    let mut s = sim(&app);
    s.click("New Workspace").expect("a row inside the card");
    let inside: Vec<Message> = s.into_messages().collect();
    apply(&mut app, inside);
    assert!(app.state.shortcuts.open, "a click on the card keeps it");
    let mut s = sim(&app);
    s.point_at(Point::new(20.0, SIZE.1 - 20.0));
    s.simulate(simulator::click());
    let outside: Vec<Message> = s.into_messages().collect();
    apply(&mut app, outside);
    assert!(!app.state.shortcuts.open);
}

#[test]
fn the_sidebar_buttons_open_settings_and_the_panel() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    // The two buttons sit at the bottom left of the sidebar: gear first, then the keyboard.
    let y = SIZE.1 - 8.0 - 14.0;
    for (x, check) in [(12.0 + 14.0, 0), (12.0 + 28.0 + 4.0 + 14.0, 1)] {
        let mut s = sim(&app);
        s.point_at(Point::new(x, y));
        s.simulate(simulator::click());
        let msgs: Vec<Message> = s.into_messages().collect();
        apply(&mut app, msgs);
        if check == 0 {
            assert!(app.state.settings.open, "the gear opens Settings");
            let _ = app.update(Message::Settings(SettingsMsg::Close));
        } else {
            assert!(app.state.shortcuts.open, "the keyboard opens the panel");
        }
    }
}

// ------------------------------------------------------------ settings

#[test]
fn cmd_comma_toggles_settings_and_closes_the_panel() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    key(&mut app, combo("/", true, false));
    key(&mut app, combo(",", true, false));
    assert!(app.state.settings.open);
    assert!(!app.state.shortcuts.open, "one sheet at a time");
    let mut s = sim(&app);
    for label in ["Settings", "Appearance", "Window", "Reading", "Width", "Input", "Allow sending input to agents"] {
        assert!(s.find(label).is_ok(), "{label}");
    }
    drop(s);
    key(&mut app, combo(",", true, false));
    assert!(!app.state.settings.open);
}

#[test]
fn the_reading_width_moves_the_column_now() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    assert!(sim(&app).find("Text column up to 760 pt.").is_err(), "closed: nothing shows");
    let _ = app.update(Message::Settings(SettingsMsg::Open));
    let mut s = sim(&app);
    s.click("Narrow").expect("Narrow");
    let msgs: Vec<Message> = s.into_messages().collect();
    apply(&mut app, msgs);
    assert_eq!(app.state.settings.reading_width(), ReadingWidth::Narrow);
    assert!(sim(&app).find("Text column up to 600 pt.").is_ok());
    let _ = app.update(Message::Settings(SettingsMsg::SetWidth(ReadingWidth::Full)));
    assert!(sim(&app).find("The text fills the window.").is_ok());
    assert_eq!(ReadingWidth::ALL.map(|w| w.points()), [600.0, 760.0, 920.0, 2400.0]);
}

#[test]
fn turning_input_off_removes_the_write_paths_now_and_on_waits_for_a_launch() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    assert!(app.state.cards.actions.is_some(), "the demo records actions");
    let _ = app.update(Message::Settings(SettingsMsg::SetAllowInput(false)));
    assert!(app.state.cards.actions.is_none() && app.state.composer.sender.is_none());
    assert!(!app.state.terminal.writable);
    assert_eq!(app.state.settings.prefs.allow_input, Some(false));
    let _ = app.update(Message::Settings(SettingsMsg::SetAllowInput(true)));
    assert_eq!(app.state.settings.prefs.allow_input, Some(true));
    assert!(app.state.cards.actions.is_none(), "still read-only until the next launch");
    let _ = app.update(Message::Settings(SettingsMsg::Open));
    assert!(sim(&app).find(settings::INPUT_NEXT_LAUNCH).is_ok(), "the sheet says this window is still read-only");
}

#[test]
fn the_input_row_never_contradicts_the_composer() {
    // The demo has no sender: the composer reads "Read-only: input is off".
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    assert!(!flockbook::access::writes_live(&app));
    let _ = app.update(Message::Settings(SettingsMsg::Open));
    let mut s = sim(&app);
    assert!(s.find(settings::INPUT_NEXT_LAUNCH).is_ok());
    assert!(s.find(settings::INPUT_LIVE).is_err(), "never 'types into your agents' while read-only");
    drop(s);
    // Saved off: plain read-only note.
    let _ = app.update(Message::Settings(SettingsMsg::SetAllowInput(false)));
    assert!(sim(&app).find(settings::INPUT_OFF).is_ok());
}

#[test]
fn the_toggle_knob_is_white_in_every_state() {
    use iced::widget::toggler::Status;
    for mode in [Mode::Light, Mode::Dark] {
        let t = fb_ui::theme::Tokens::of(mode);
        for is_toggled in [true, false] {
            for st in [Status::Active { is_toggled }, Status::Hovered { is_toggled }, Status::Disabled { is_toggled }] {
                let style = settings::toggle_style(t, st);
                assert_eq!(style.foreground, iced::Background::Color(iced::Color::WHITE), "{mode:?} {st:?}");
            }
        }
        assert_eq!(settings::toggle_style(t, Status::Active { is_toggled: true }).background, iced::Background::Color(t.accent));
    }
}

#[test]
fn read_only_launch_keeps_the_toggle_off() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    app.state.settings.read_only_launch = true;
    let _ = app.update(Message::Settings(SettingsMsg::SetAllowInput(true)));
    assert_eq!(app.state.settings.prefs.allow_input, None, "ignored");
    let _ = app.update(Message::Settings(SettingsMsg::Open));
    assert!(sim(&app).find("Launched with --read-only: the app only reads, whatever this says.").is_ok());
}

#[test]
fn glass_off_applies_now_and_on_applies_now_only_in_a_glass_window() {
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    glass::install(&mut app, if cfg!(target_os = "windows") { Backdrop::Mica } else { Backdrop::Vibrancy });
    let _ = app.update(Message::Settings(SettingsMsg::SetGlass(false)));
    if Os::current() == Os::Other {
        assert_eq!(app.state.settings.prefs.glass, None, "Linux: the picker is off");
        return;
    }
    assert!(!app.state.glass.on, "Opaque applies now");
    let _ = app.update(Message::Settings(SettingsMsg::SetGlass(true)));
    assert!(app.state.glass.on, "the window was made transparent at launch: glass returns now");
    // A window made opaque at launch: glass waits for the next launch.
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let _ = app.update(Message::Settings(SettingsMsg::SetGlass(true)));
    assert!(!app.state.glass.on);
    assert_eq!(app.state.settings.pending, Some(settings::NEXT_LAUNCH_GLASS));
}

#[test]
fn the_saved_glass_setting_sits_between_the_env_and_the_default() {
    let env = |v: Option<&str>| v.map(|v| HashMap::from([("FLOCKBOOK_GLASS".to_string(), v.to_string())])).unwrap_or_default();
    let flag = vec!["--glass".to_string(), "on".to_string()];
    assert_eq!(glass::choose_with(Os::MacOs, &[], &env(None), Some(false)), Backdrop::Opaque);
    assert_eq!(glass::choose_with(Os::MacOs, &[], &env(Some("on")), Some(false)), Backdrop::Vibrancy, "env beats setting");
    assert_eq!(glass::choose_with(Os::MacOs, &flag, &env(None), Some(false)), Backdrop::Vibrancy, "flag beats setting");
    assert_eq!(glass::choose_with(Os::Windows, &[], &env(None), None), Backdrop::Mica);
}

// ------------------------------------------------------------ the file

fn temp_file(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("settings-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("flockbook").join("settings.json")
}

#[test]
fn settings_save_and_load_back() {
    let path = temp_file("roundtrip");
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    app.state.settings.path = Some(path.clone());
    let _ = app.update(Message::Settings(SettingsMsg::SetWidth(ReadingWidth::Wide)));
    let _ = app.update(Message::Settings(SettingsMsg::SetAllowInput(false)));
    assert!(app.state.settings.error.is_none(), "{:?}", app.state.settings.error);
    let loaded = Prefs::load(&path);
    assert_eq!(loaded, Prefs { glass: None, reading_width: Some(ReadingWidth::Wide), allow_input: Some(false), ..Prefs::default() });
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"reading_width\": \"wide\""), "{text}");
}

#[test]
fn a_missing_or_broken_file_reads_as_nothing_set() {
    let path = temp_file("broken");
    assert_eq!(Prefs::load(&path), Prefs::default());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ not json").unwrap();
    assert_eq!(Prefs::load(&path), Prefs::default());
    std::fs::write(&path, r#"{"reading_width":"narrow","future_key":1}"#).unwrap();
    assert_eq!(Prefs::load(&path).reading_width, Some(ReadingWidth::Narrow), "unknown keys are ignored");
}

#[test]
fn tests_and_demo_never_write_a_settings_file() {
    let app = App::demo(Mode::Light, Screen::Conversation);
    assert!(app.state.settings.path.is_none());
}

// ------------------------------------------------------------ look

fn shot(app: &App, name: &str) {
    let img = render_app(app, (SIZE.0 as u32, SIZE.1 as u32));
    write_png(&img, &std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"))).unwrap();
    if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
        write_png(&img, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence").join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn snapshots_of_the_panel_and_the_sheet() {
    for mode in [Mode::Light, Mode::Dark] {
        let suffix = if mode.is_dark() { "dark" } else { "light" };
        let mut app = App::demo(mode, Screen::Conversation);
        let _ = app.update(Message::Shortcut(Action::Shortcuts));
        shot(&app, &format!("shortcuts-panel-{suffix}"));
        let _ = app.update(Message::Settings(SettingsMsg::Open));
        assert!(!app.state.shortcuts.open, "opening Settings closes the panel");
        shot(&app, &format!("settings-{suffix}"));
        app.state.settings.read_only_launch = true;
        shot(&app, &format!("settings-read-only-launch-{suffix}"));
    }
}
