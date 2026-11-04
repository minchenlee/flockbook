//! The terminal sheet: grid feed, input gates, resize debounce, wheel, release, history, snapshots.
//! Everything runs against `FakeTerminals`: no process, no socket, no herdr. Headless (iced_test Simulator, tiny-skia).

use fb_ui::Mode;
use flockbook::source::DataSource;
use flockbook::snapshot::{distinct_colors, render_app, write_png};
use flockbook::state::Screen;
use flockbook::actions::{PaneActions, RecordingActions};
use flockbook::cards::{CardKey, CardsMsg};
use flockbook::composer::{ComposerMsg, MessageSender};
use flockbook::terminal::{self, Call, FakeTerminals, TerminalMsg};
use flockbook::toprow::TopRowMsg;
use flockbook::{App, Message};
use herdr_core::scrollback::ScrollInfo;
use herdr_core::terminal::{TerminalEvent, TerminalMode};
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::text_editor::{Action, Edit};
use iced::{Settings, Size};
use iced_test::Simulator;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Rig {
    app: App,
    fake: FakeTerminals,
    now: Arc<Mutex<Instant>>,
}

impl Rig {
    fn new(writable: bool) -> Rig {
        let mut app = App::demo(Mode::Light, Screen::Conversation);
        let fake = FakeTerminals::default();
        app.state.terminal.backend = Some(Arc::new(fake.clone()));
        app.state.terminal.writable = writable;
        let now = Arc::new(Mutex::new(Instant::now()));
        let n = now.clone();
        app.state.terminal.clock = Arc::new(move || *n.lock().unwrap());
        Rig { app, fake, now }
    }
    fn open(&mut self) {
        self.msg(TerminalMsg::Toggle);
    }
    fn msg(&mut self, m: TerminalMsg) {
        let _ = self.app.update(Message::Terminal(m));
    }
    fn advance(&mut self, ms: u64) {
        *self.now.lock().unwrap() += Duration::from_millis(ms);
        self.msg(TerminalMsg::Poll);
    }
    fn sheet(&self) -> &terminal::Sheet {
        self.app.state.terminal.sheet.as_ref().expect("sheet open")
    }
    fn set_kind(&self, max: i64, shell_fg: bool, rows: i64) {
        self.fake.set_info(ScrollInfo { max_offset_from_bottom: max, offset_from_bottom: 0, viewport_rows: rows, foreground_name: Some("x".into()), shell_is_foreground: Some(shell_fg) });
    }
}

fn sim(app: &App) -> Simulator<'_, Message> {
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
    Simulator::with_size(settings, Size::new(1280.0, 852.0), app.view())
}

fn inputs(msgs: Vec<Message>) -> Vec<Vec<u8>> {
    msgs.into_iter().filter_map(|m| if let Message::Terminal(TerminalMsg::Input(b)) = m { Some(b) } else { None }).collect()
}

// ------------------------------------------------------------ open, frames

#[test]
fn the_toggle_opens_the_sheet_for_the_selected_pane_in_control_mode() {
    let mut r = Rig::new(true);
    let (pane_terminal, tab) = {
        let p = r.app.state.selected_pane().unwrap();
        (p.terminal_id.clone(), r.app.state.selected_tab().unwrap().label.clone())
    };
    assert!(!r.app.state.toprow.terminal_on);
    r.open();
    assert!(r.app.state.toprow.terminal_on);
    assert_eq!(r.sheet().title, tab);
    assert!(matches!(r.fake.calls()[0], Call::Open { ref terminal_id, mode: TerminalMode::Control { takeover: false }, .. } if *terminal_id == pane_terminal));
    r.msg(TerminalMsg::Toggle);
    assert!(!r.app.state.terminal.is_open() && !r.app.state.toprow.terminal_on);
}

#[test]
fn frames_feed_the_grid_a_full_frame_resets_it() {
    let mut r = Rig::new(true);
    r.open();
    r.fake.push_frame(true, 40, 10, b"hello \x1b[31mred\x1b[0m\r\n\xe4\xbd\xa0\xe5\xa5\xbd");
    r.msg(TerminalMsg::Poll);
    let s = &r.sheet().snap;
    assert_eq!((s.cols, s.rows), (40, 10), "the size follows the frame");
    assert_eq!(s.row_text(0), "hello red");
    assert_eq!(s.row_text(1), "\u{4f60}\u{597d}");
    r.fake.push_frame(false, 40, 10, b"\x1b[3;1Hmore");
    r.msg(TerminalMsg::Poll);
    assert_eq!(r.sheet().snap.row_text(2), "more");
    assert_eq!(r.sheet().snap.row_text(0), "hello red");
    r.fake.push_frame(true, 50, 12, b"fresh");
    r.msg(TerminalMsg::Poll);
    assert_eq!((r.sheet().snap.cols, r.sheet().snap.rows), (50, 12));
    assert_eq!(r.sheet().snap.row_text(0), "fresh");
}

#[test]
fn application_cursor_mode_follows_the_program() {
    let mut r = Rig::new(true);
    r.open();
    r.fake.push_frame(true, 20, 5, b"\x1b[?1h");
    r.msg(TerminalMsg::Poll);
    assert!(r.sheet().app_cursor);
}

// ------------------------------------------------------------ input gates

#[test]
fn read_only_opens_observe_and_writes_nothing_at_all() {
    let mut r = Rig::new(false);
    r.open();
    assert!(matches!(r.fake.calls()[0], Call::Open { mode: TerminalMode::Observe, .. }));
    r.set_kind(0, false, 24);
    r.fake.push_frame(true, 80, 24, b"x");
    r.msg(TerminalMsg::Poll);
    r.msg(TerminalMsg::Focus(true));
    assert!(!r.sheet().focused, "observe never takes focus");
    r.msg(TerminalMsg::Input(b"rm -rf\r".to_vec()));
    r.msg(TerminalMsg::Paste("x".into()));
    r.msg(TerminalMsg::Resized(120, 40));
    r.advance(1000);
    r.msg(TerminalMsg::Wheel { dy: 3.0, precise: false });
    r.msg(TerminalMsg::Wheel { dy: -3.0, precise: false });
    assert_eq!(r.fake.writes(), vec![], "no byte, paste, resize or wheel in observe mode");
}

#[test]
fn read_only_widget_has_no_input_path() {
    let mut r = Rig::new(false);
    r.open();
    let msgs = {
        let mut s = sim(&r.app);
        s.typewrite("abc");
        s.tap_key(Named::Enter);
        s.into_messages().collect::<Vec<_>>()
    };
    assert!(inputs(msgs).is_empty());
    // make_read_only on an open control sheet closes it and releases.
    let mut w = Rig::new(true);
    w.open();
    flockbook::access::make_read_only(&mut w.app);
    assert!(!w.app.state.terminal.is_open());
    assert!(w.fake.calls().contains(&Call::Release));
}

#[test]
fn keys_reach_the_widget_callback_only_while_focused() {
    let mut r = Rig::new(true);
    r.open();
    assert!(r.sheet().focused, "a control sheet opens focused");
    let got = {
        let mut s = sim(&r.app);
        s.typewrite("a");
        s.tap_key(Named::Enter);
        s.tap_key(Named::ArrowUp);
        inputs(s.into_messages().collect())
    };
    assert_eq!(got, vec![b"a".to_vec(), b"\r".to_vec(), b"\x1b[A".to_vec()]);
    r.msg(TerminalMsg::Focus(false));
    let none = {
        let mut s = sim(&r.app);
        s.typewrite("a");
        s.tap_key(Named::Enter);
        inputs(s.into_messages().collect())
    };
    assert!(none.is_empty(), "an unfocused terminal reports no key");
}

#[test]
fn update_drops_input_unless_focused_control_and_live() {
    let mut r = Rig::new(true);
    r.open();
    r.msg(TerminalMsg::Input(b"a".to_vec()));
    assert_eq!(r.fake.writes(), vec![Call::Bytes(b"a".to_vec())]);
    r.msg(TerminalMsg::Paste("p".into()));
    assert_eq!(r.fake.writes().last(), Some(&Call::Paste("p".into())));
    let n = r.fake.writes().len();
    r.msg(TerminalMsg::Focus(false));
    r.msg(TerminalMsg::Input(b"b".to_vec()));
    r.msg(TerminalMsg::Paste("q".into()));
    assert_eq!(r.fake.writes().len(), n, "unfocused: dropped");
    r.msg(TerminalMsg::Focus(true));
    // history view: dropped
    r.set_kind(100, true, 24);
    r.fake.history.lock().unwrap().push_str("l1\nl2\nl3\n");
    r.msg(TerminalMsg::Wheel { dy: 5.0, precise: false });
    assert!(r.sheet().history.is_some());
    r.msg(TerminalMsg::Input(b"c".to_vec()));
    assert_eq!(r.fake.writes().len(), n, "history view: dropped");
    r.msg(TerminalMsg::BackToLive);
    r.msg(TerminalMsg::Input(b"d".to_vec()));
    assert_eq!(r.fake.writes().len(), n + 1);
    // closed session: dropped
    r.fake.push(TerminalEvent::Closed { reason: "terminal attach taken over".into() });
    r.msg(TerminalMsg::Poll);
    r.msg(TerminalMsg::Input(b"e".to_vec()));
    assert_eq!(r.fake.writes().len(), n + 1, "closed: dropped");
}

#[test]
fn cmd_keys_and_ctrl_backtick_are_not_sent() {
    let mut r = Rig::new(true);
    r.open();
    let msgs = {
        let mut s = sim(&r.app);
        let ev = |k: &str, m: iced::keyboard::Modifiers| iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(k.into()),
            modified_key: iced::keyboard::Key::Character(k.into()),
            physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: iced::keyboard::Location::Standard,
            modifiers: m,
            text: Some(k.into()),
            repeat: false,
        });
        s.simulate([ev("c", iced::keyboard::Modifiers::LOGO), ev("`", iced::keyboard::Modifiers::CTRL), ev("c", iced::keyboard::Modifiers::CTRL)]);
        inputs(s.into_messages().collect())
    };
    assert_eq!(msgs, vec![vec![3u8]], "only Ctrl+C is a terminal key");
}

// ------------------------------------------------------------ resize

#[test]
fn resize_waits_150_ms_and_only_the_last_size_goes() {
    let mut r = Rig::new(true);
    r.open();
    r.msg(TerminalMsg::Resized(100, 30));
    r.advance(100);
    r.msg(TerminalMsg::Resized(110, 32));
    r.advance(100);
    assert!(r.fake.writes().is_empty(), "still waiting");
    r.advance(60);
    assert_eq!(r.fake.writes(), vec![Call::Resize(110, 32)]);
    r.advance(1000);
    assert_eq!(r.fake.writes().len(), 1, "sent once");
}

#[test]
fn the_opening_size_is_not_a_resize() {
    let mut r = Rig::new(true);
    r.open();
    let (c, rows) = match r.fake.calls()[0] {
        Call::Open { cols, rows, .. } => (cols, rows),
        _ => panic!(),
    };
    r.msg(TerminalMsg::Resized(c, rows));
    r.advance(500);
    assert!(r.fake.writes().is_empty());
}

// ------------------------------------------------------------ wheel

#[test]
fn wheel_on_a_full_screen_program_goes_to_herdr_never_as_keys() {
    let mut r = Rig::new(true);
    r.open();
    r.set_kind(0, false, 24);
    r.msg(TerminalMsg::Wheel { dy: 3.0, precise: false });
    r.msg(TerminalMsg::Wheel { dy: -1.0, precise: false });
    let w = r.fake.writes();
    assert_eq!(w[0], Call::Wheel { lines: 3, up: true });
    assert_eq!(w[1], Call::Wheel { lines: 1, up: false });
    assert!(!w.iter().any(|c| matches!(c, Call::Bytes(_) | Call::Paste(_))), "no keys");
    // A trackpad: one step per 28 points, at most 4 per event.
    r.msg(TerminalMsg::Wheel { dy: 10.0, precise: true });
    assert_eq!(r.fake.writes().len(), 2, "10 pt is below one step");
    r.msg(TerminalMsg::Wheel { dy: 500.0, precise: true });
    assert_eq!(r.fake.writes().last(), Some(&Call::Wheel { lines: 4, up: true }));
}

#[test]
fn wheel_on_history_opens_the_read_only_view_and_sends_nothing() {
    let mut r = Rig::new(true);
    r.open();
    r.set_kind(500, true, 24);
    *r.fake.history.lock().unwrap() = (1..=60).map(|i| format!("line {i}\r\n")).collect();
    r.msg(TerminalMsg::Wheel { dy: -2.0, precise: false });
    assert!(r.sheet().history.is_none(), "wheel down does not open it");
    r.msg(TerminalMsg::Wheel { dy: 2.0, precise: false });
    let h = r.sheet().history.as_ref().expect("history view");
    assert_eq!(h.lines, 60);
    assert_eq!(h.snap.row_text(59), "line 60");
    assert!(r.fake.writes().is_empty());
    // The view has no input path.
    let got = {
        let mut s = sim(&r.app);
        s.typewrite("zz");
        inputs(s.into_messages().collect())
    };
    assert!(got.is_empty());
    r.msg(TerminalMsg::Escape);
    assert!(r.sheet().history.is_none() && r.app.state.terminal.is_open(), "Esc leaves the history first");
    r.msg(TerminalMsg::Escape);
    assert!(!r.app.state.terminal.is_open(), "then Esc closes the sheet");
}

#[test]
fn observe_with_a_program_without_history_shows_a_notice() {
    let mut r = Rig::new(false);
    r.open();
    r.set_kind(0, false, 24);
    *r.fake.history.lock().unwrap() = "a\nb\nc\n".into();
    r.msg(TerminalMsg::Wheel { dy: 3.0, precise: false });
    assert!(r.sheet().history.is_none());
    assert_eq!(r.sheet().notice.as_ref().map(|n| n.0.as_str()), Some(terminal::NO_HISTORY));
    assert!(r.fake.writes().is_empty());
    r.advance(7000);
    assert!(r.sheet().notice.is_none(), "the notice goes away");
}

#[test]
fn a_fresh_shell_has_nothing_to_scroll() {
    let mut r = Rig::new(true);
    r.open();
    r.set_kind(0, true, 24);
    r.msg(TerminalMsg::Wheel { dy: 3.0, precise: false });
    assert!(r.fake.writes().is_empty() && r.sheet().history.is_none() && r.sheet().notice.is_none());
}

// ------------------------------------------------------------ release, closed

#[test]
fn close_releases_the_terminal() {
    let mut r = Rig::new(true);
    r.open();
    r.msg(TerminalMsg::Close);
    assert_eq!(r.fake.calls().iter().filter(|c| **c == Call::Release).count(), 1);
    r.msg(TerminalMsg::Close);
    assert_eq!(r.fake.calls().iter().filter(|c| **c == Call::Release).count(), 1, "released once");
}

#[test]
fn leaving_the_screen_or_the_pane_releases_it() {
    let mut r = Rig::new(true);
    r.open();
    let _ = r.app.update(Message::Navigate(Screen::Overview));
    assert!(!r.app.state.terminal.is_open());
    assert!(r.fake.calls().contains(&Call::Release));

    let mut r = Rig::new(true);
    r.open();
    let other = r.app.state.workspaces.iter().flat_map(|w| w.tabs.iter().map(move |t| (w.id.clone(), t.id.clone(), t.panes[0].terminal_id.clone()))).find(|(_, _, tid)| *tid != r.sheet().terminal_id).unwrap();
    let _ = r.app.update(Message::SelectTab { workspace: other.0, tab: other.1 });
    assert!(!r.app.state.terminal.is_open(), "another pane: released");
    assert!(r.fake.calls().contains(&Call::Release));
}

#[test]
fn a_closed_session_shows_why_and_never_takes_over_by_itself() {
    let mut r = Rig::new(true);
    r.open();
    r.fake.push(TerminalEvent::Closed { reason: "terminal attach taken over".into() });
    r.advance(100);
    r.advance(5000);
    assert_eq!(r.sheet().closed.as_deref(), Some("terminal attach taken over"));
    let opens = || r.fake.calls().iter().filter(|c| matches!(c, Call::Open { .. })).count();
    assert_eq!(opens(), 1, "no automatic takeover");
    assert!(sim(&r.app).find(terminal::explain("terminal attach taken over")).is_ok());
    assert!(sim(&r.app).find("Take over").is_ok());
    r.msg(TerminalMsg::TakeOver);
    assert!(r.fake.calls().iter().any(|c| matches!(c, Call::Open { mode: TerminalMode::Control { takeover: true }, .. })));
    assert!(r.sheet().closed.is_none());
}

#[test]
fn take_over_is_not_offered_when_the_terminal_is_gone_or_read_only() {
    let mut r = Rig::new(true);
    r.open();
    r.fake.push(TerminalEvent::Closed { reason: "terminal attach failed: terminal not found".into() });
    r.advance(10);
    assert!(sim(&r.app).find("Take over").is_err());
    let mut o = Rig::new(false);
    o.open();
    o.fake.push(TerminalEvent::Closed { reason: "terminal attach taken over".into() });
    o.advance(10);
    assert!(sim(&o.app).find("Take over").is_err());
    o.msg(TerminalMsg::TakeOver);
    assert!(!o.fake.calls().iter().any(|c| matches!(c, Call::Open { mode: TerminalMode::Control { .. }, .. })), "read-only never takes over");
}

#[test]
fn detached_is_a_normal_end_without_a_banner() {
    let mut r = Rig::new(true);
    r.open();
    r.fake.push(TerminalEvent::Closed { reason: "detached".into() });
    r.advance(10);
    assert!(sim(&r.app).find("Take over").is_err());
}

// ------------------------------------------------------------ snapshots

fn shot(mode: Mode, name: &str) {
    let app = terminal::demo_app(mode, Screen::Conversation, true);
    let img = render_app(&app, (1280, 852));
    assert!(distinct_colors(&img) > 60, "{name}: blank frame");
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    write_png(&img, &dir.join(format!("{name}.png"))).unwrap();
    if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
        write_png(&img, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence").join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn terminal_sheet_light() {
    shot(Mode::Light, "app-terminal-light");
}

#[test]
fn terminal_sheet_dark() {
    shot(Mode::Dark, "app-terminal-dark");
}

#[test]
fn terminal_history_light() {
    let mut app = terminal::demo_app(Mode::Light, Screen::Conversation, true);
    let text: String = (1..=80).map(|i| format!("\x1b[2m{i:>3}\x1b[0m  \x1b[32mok\x1b[0m  step {i} of the ranking replay \u{901a}\u{904e}\r\n")).collect();
    app.state.terminal.sheet.as_mut().unwrap().history = Some(terminal::history_view(&text, 100, 80));
    let img = render_app(&app, (1280, 852));
    assert!(distinct_colors(&img) > 20, "blank frame");
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    write_png(&img, &dir.join("app-terminal-history-light.png")).unwrap();
    if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
        write_png(&img, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence").join("app-terminal-history-light.png")).unwrap();
    }
}

#[test]
fn the_demo_sheet_text_is_on_screen() {
    let app = terminal::demo_app(Mode::Light, Screen::Conversation, true);
    let s = &app.state.terminal.sheet.as_ref().unwrap().snap;
    assert!(s.row_text(0).contains("pytest tests/ranking -q"));
    assert!((0..s.rows).any(|r| s.row_text(r).contains("\u{6392}\u{540d}")), "CJK line");
    assert!(s.cursor.is_some());
}

// ------------------------------------------------------------ review fixes

fn key_ev(k: Key, m: Modifiers, text: Option<&str>) -> iced::Event {
    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
        key: k.clone(),
        modified_key: k,
        physical_key: iced::keyboard::key::Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
        location: iced::keyboard::Location::Standard,
        modifiers: m,
        text: text.map(Into::into),
        repeat: false,
    })
}

// 1. A synchronized update that never ends must not swallow later frames.

#[test]
fn a_full_frame_after_an_unfinished_sync_update_is_not_swallowed() {
    let mut r = Rig::new(true);
    r.open();
    r.fake.push_frame(true, 20, 5, b"base");
    r.fake.push_frame(false, 20, 5, b"\x1b[?2026h\x1b[2;1Hheld back");
    r.msg(TerminalMsg::Poll);
    assert_eq!(r.sheet().snap.row_text(1), "", "the update is held back");
    r.fake.push_frame(true, 20, 5, b"fresh start");
    r.msg(TerminalMsg::Poll);
    assert_eq!(r.sheet().snap.row_text(0), "fresh start");
}

#[test]
fn an_expired_sync_update_is_flushed_by_the_poll() {
    let mut r = Rig::new(true);
    r.open();
    r.fake.push_frame(true, 20, 5, b"base");
    r.fake.push_frame(false, 20, 5, b"\x1b[?2026h\x1b[2;1Hlate");
    r.msg(TerminalMsg::Poll);
    assert_eq!(r.sheet().snap.row_text(1), "");
    // No new frame comes. Time passes (the sync timeout is 150 ms).
    r.advance(400);
    assert_eq!(r.sheet().snap.row_text(1), "late");
}

// 2. Ctrl+` opens the sheet on the conversation screen only (Swift: only the tab screen).

#[test]
fn ctrl_backtick_opens_the_sheet_only_on_the_conversation_screen() {
    let msg = flockbook::toprow::on_key(Key::Character("`".into()), Modifiers::CTRL).expect("the shortcut");
    for screen in [Screen::Overview, Screen::NeedsYou, Screen::Search] {
        let mut r = Rig::new(true);
        r.app.state.screen = screen;
        let _ = r.app.update(msg.clone());
        assert!(!r.app.state.terminal.is_open(), "{screen:?}: no sheet");
        assert!(!r.app.state.toprow.terminal_on);
        assert!(r.fake.calls().is_empty(), "{screen:?}: no session opened");
        let _ = r.app.update(Message::TopRow(TopRowMsg::ToggleTerminal));
        assert!(!r.app.state.terminal.is_open(), "{screen:?}: the button is gated too");
    }
    let mut r = Rig::new(true);
    let _ = r.app.update(msg);
    assert!(r.app.state.terminal.is_open(), "the conversation screen opens it");
}

// 4. The history grid never loses a top line under the row cap.

#[test]
fn history_with_emoji_tabs_and_osc_keeps_every_line() {
    let text = "first line\r\n\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\r\n\ta\tb\tc\tdddd\r\n\x1b]8;;http://example.test/a/very/long/link/target\x07link\x1b]8;;\x07 after\r\nlast\r\n";
    let h = terminal::history_view(text, 10, 5);
    let rows: Vec<String> = (0..h.snap.rows).map(|r| h.snap.row_text(r)).collect();
    assert_eq!(rows[0], "first line", "{rows:?}");
    assert_eq!(rows.last().map(String::as_str), Some("last"), "{rows:?}");
    assert!(rows.iter().any(|r| r == "link after"), "{rows:?}");
    assert!(!rows.iter().any(|r| r.contains("http")), "the OSC sequence is not text: {rows:?}");
    assert_eq!(rows.iter().filter(|r| r.contains('\u{1f600}')).count(), 2, "8 emoji at 10 columns take two rows: {rows:?}");
}

#[test]
fn a_long_history_of_wide_lines_keeps_its_first_and_last_line() {
    let text: String = (0..400).map(|i| format!("{i:03} \u{4f60}\u{597d}\u{1f600}\u{1f600}\u{1f600}\u{1f600}\t{i}\r\n")).collect();
    let h = terminal::history_view(&text, 12, 400);
    assert!(h.snap.row_text(0).starts_with("000 "), "{}", h.snap.row_text(0));
    assert!(h.snap.row_text(h.snap.rows - 1).contains("399"), "{}", h.snap.row_text(h.snap.rows - 1));
    assert_eq!(h.snap.cursor, None);
}

#[test]
fn one_huge_history_line_is_capped_to_the_newest_rows() {
    // One 10 MB line (minified JSON, a log blob) wraps into 50000 rows at 200 columns. Only the newest are kept.
    let mut text = "a".repeat(10 * 1024 * 1024 - 3);
    text.push_str("END\r\n");
    let h = terminal::history_view(&text, 200, 1);
    assert!(h.snap.rows <= terminal::HISTORY_MAX_ROWS, "{} rows", h.snap.rows);
    assert_eq!(h.snap.rows, terminal::HISTORY_MAX_ROWS, "the grid is full: the cap, not the text, bounds it");
    assert!(h.snap.row_text(h.snap.rows - 1).ends_with("END"), "the newest row is kept");
}

// 6. Wheel down at the bottom of the history returns to live.

#[test]
fn wheel_down_at_the_bottom_of_the_history_returns_to_live() {
    let mut r = Rig::new(true);
    r.open();
    r.set_kind(500, true, 24);
    *r.fake.history.lock().unwrap() = (1..=60).map(|i| format!("line {i}\r\n")).collect();
    r.msg(TerminalMsg::Wheel { dy: 2.0, precise: false });
    assert!(r.sheet().history.is_some());
    r.msg(TerminalMsg::HistoryScrolled { at_bottom: false });
    r.msg(TerminalMsg::HistoryWheel { dy: -1.0 });
    r.msg(TerminalMsg::HistoryWheel { dy: 1.0 });
    assert!(r.sheet().history.is_some(), "away from the bottom: wheel stays in the history");
    r.msg(TerminalMsg::HistoryScrolled { at_bottom: true });
    r.msg(TerminalMsg::HistoryWheel { dy: 1.0 });
    assert!(r.sheet().history.is_some(), "wheel up at the bottom stays");
    r.msg(TerminalMsg::HistoryWheel { dy: -1.0 });
    assert!(r.sheet().history.is_none(), "wheel down at the bottom: back to live");
    assert!(r.fake.writes().is_empty());
}

#[test]
fn the_history_view_reports_the_wheel_without_taking_it() {
    let mut r = Rig::new(true);
    r.open();
    r.set_kind(500, true, 24);
    *r.fake.history.lock().unwrap() = (1..=60).map(|i| format!("line {i}\r\n")).collect();
    r.msg(TerminalMsg::Wheel { dy: 2.0, precise: false });
    let msgs = {
        let mut s = sim(&r.app);
        s.point_at(iced::Point::new(640.0, 400.0));
        s.simulate([iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 } })]);
        s.into_messages().collect::<Vec<_>>()
    };
    assert!(msgs.iter().any(|m| matches!(m, Message::Terminal(TerminalMsg::HistoryWheel { dy }) if *dy < 0.0)), "{msgs:?}");
    assert!(msgs.iter().any(|m| matches!(m, Message::Terminal(TerminalMsg::HistoryScrolled { .. }))), "the scrollable still scrolled: {msgs:?}");
}

// 5. Selection and copy: local only.

/// Origin of the live terminal widget in a 1280x852 window: the sheet is centred under the top row.
fn live_origin() -> iced::Point {
    iced::Point::new((1280.0 - terminal::SheetSize::STANDARD.0) / 2.0, fb_ui::theme::TOPROW_H + 8.0 + terminal::HEADER_H + 1.0)
}

fn cell_point(col: usize, row: usize) -> iced::Point {
    let o = live_origin();
    iced::Point::new(o.x + fb_ui::term::PAD + col as f32 * fb_ui::term::CELL_W + 2.0, o.y + fb_ui::term::PAD + row as f32 * fb_ui::term::CELL_H + 2.0)
}

fn copy_chord() -> iced::Event {
    if cfg!(target_os = "macos") {
        key_ev(Key::Character("c".into()), Modifiers::LOGO, Some("c"))
    } else {
        key_ev(Key::Character("C".into()), Modifiers::CTRL | Modifiers::SHIFT, Some("C"))
    }
}

#[test]
fn dragging_in_the_live_view_selects_and_copy_never_writes_to_the_pane() {
    let mut r = Rig::new(true);
    r.open();
    r.fake.push_frame(true, 60, 10, b"hello world\r\nsecond row");
    r.msg(TerminalMsg::Poll);
    let msgs = {
        let mut s = sim(&r.app);
        let mv = |p| iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: p });
        s.point_at(cell_point(6, 0));
        s.simulate([mv(cell_point(6, 0)), iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))]);
        s.point_at(cell_point(5, 1));
        s.simulate([mv(cell_point(5, 1)), iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))]);
        s.simulate([copy_chord()]);
        s.into_messages().collect::<Vec<_>>()
    };
    assert!(msgs.iter().any(|m| matches!(m, Message::Terminal(TerminalMsg::Copy(t)) if t == "world\nsecond")), "{msgs:?}");
    assert!(!msgs.iter().any(|m| matches!(m, Message::Terminal(TerminalMsg::Input(_) | TerminalMsg::Paste(_)))), "the copy chord is not a program key: {msgs:?}");
    for m in msgs {
        let _ = r.app.update(m);
    }
    assert!(r.fake.writes().is_empty(), "nothing reached the pane: {:?}", r.fake.writes());
}

#[test]
fn copy_works_in_observe_mode_too() {
    let mut r = Rig::new(false);
    r.open();
    r.fake.push_frame(true, 60, 10, b"read only text");
    r.msg(TerminalMsg::Poll);
    let msgs = {
        let mut s = sim(&r.app);
        let mv = |p| iced::Event::Mouse(iced::mouse::Event::CursorMoved { position: p });
        s.point_at(cell_point(0, 0));
        s.simulate([mv(cell_point(0, 0)), iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left))]);
        s.point_at(cell_point(3, 0));
        s.simulate([mv(cell_point(3, 0)), iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))]);
        s.simulate([copy_chord()]);
        s.into_messages().collect::<Vec<_>>()
    };
    assert!(msgs.iter().any(|m| matches!(m, Message::Terminal(TerminalMsg::Copy(t)) if t == "read")), "{msgs:?}");
    for m in msgs {
        let _ = r.app.update(m);
    }
    assert!(r.fake.writes().is_empty());
}

#[test]
fn the_copy_message_writes_no_bytes_and_opens_no_session() {
    let mut r = Rig::new(true);
    r.open();
    let before = r.fake.calls().len();
    r.msg(TerminalMsg::Copy("text".into()));
    assert_eq!(r.fake.calls().len(), before);
}

// 10. While the sheet is open, nothing under it may type or send.

#[derive(Default)]
struct Sends(Mutex<Vec<(String, String)>>);
impl MessageSender for Sends {
    fn send(&self, pane: &str, text: &str) -> Result<(), String> {
        self.0.lock().unwrap().push((pane.into(), text.into()));
        Ok(())
    }
}

fn insert(app: &mut App, c: char) {
    let _ = app.update(Message::Composer(ComposerMsg::Edit(Action::Edit(Edit::Insert(c)))));
}

fn modal_rig(writable: bool) -> (Rig, Arc<Sends>) {
    let mut r = Rig::new(writable);
    let sends = Arc::new(Sends::default());
    r.app.state.composer.sender = Some(sends.clone());
    (r, sends)
}

fn settle() {
    std::thread::sleep(Duration::from_millis(120));
}

#[test]
fn control_sheet_unfocused_drops_composer_typing_and_submit() {
    let (mut r, sends) = modal_rig(true);
    // The owner typed a draft before opening the sheet.
    insert(&mut r.app, 'o');
    insert(&mut r.app, 'k');
    r.open();
    r.msg(TerminalMsg::Focus(false));
    for c in "rm -rf".chars() {
        insert(&mut r.app, c);
    }
    let _ = r.app.update(Message::Composer(ComposerMsg::Submit));
    settle();
    assert!(sends.0.lock().unwrap().is_empty(), "no MessageSender call: {:?}", sends.0.lock().unwrap());
    assert!(r.fake.writes().is_empty());
    assert_eq!(r.app.state.composer.text(), "ok", "the draft did not change");
    // The sheet closes: the composer works again.
    r.msg(TerminalMsg::Close);
    insert(&mut r.app, '!');
    assert_eq!(r.app.state.composer.text(), "ok!");
    let _ = r.app.update(Message::Composer(ComposerMsg::Submit));
    settle();
    assert_eq!(sends.0.lock().unwrap().len(), 1, "after closing, submit sends");
}

#[test]
fn observe_sheet_drops_composer_typing_and_submit() {
    let (mut r, sends) = modal_rig(false);
    // read-only (writes off) keeps a sender here on purpose: the gate must hold by itself.
    insert(&mut r.app, 'x');
    r.open();
    assert!(matches!(r.fake.calls()[0], Call::Open { mode: TerminalMode::Observe, .. }));
    insert(&mut r.app, 'y');
    let _ = r.app.update(Message::Composer(ComposerMsg::Submit));
    settle();
    assert!(sends.0.lock().unwrap().is_empty());
    assert_eq!(r.app.state.composer.text(), "x");
    assert!(r.fake.writes().is_empty());
}

#[test]
fn simulated_keys_with_the_sheet_open_send_nothing_anywhere() {
    let (mut r, sends) = modal_rig(true);
    r.open();
    r.msg(TerminalMsg::Focus(false));
    let msgs = {
        let mut s = sim(&r.app);
        s.typewrite("hello");
        s.tap_key(Named::Enter);
        s.into_messages().collect::<Vec<_>>()
    };
    for m in msgs {
        let _ = r.app.update(m);
    }
    settle();
    assert!(sends.0.lock().unwrap().is_empty());
    assert!(r.fake.writes().is_empty());
    assert_eq!(r.app.state.composer.text(), "");
}

const PROMPT: &str = "Bash command\n  ./scripts/deploy.sh --env staging\n\nDo you want to proceed?\n \u{276F} 1. Yes\n   2. Yes, and always allow access to deploy/ from this project\n   3. No\n";

fn picker() -> String {
    "\u{2190}  \u{2610} Pick  \u{2714} Submit  \u{2192}\n\nWhich parts should ship?\n\n 1. Alpha\n 2. Beta\n 3. Gamma\n 4. Type something.\n\u{2500}\u{2500}\u{2500}\u{2500}\n Chat about this\n".into()
}

struct Src {
    inner: flockbook::source::DemoSource,
    items: Option<Vec<herdr_core::ConversationItem>>,
}

impl flockbook::source::DataSource for Src {
    fn label(&self) -> String {
        "test".into()
    }
    fn workspaces(&self) -> Vec<herdr_core::Workspace> {
        self.inner.workspaces()
    }
    fn connected(&self) -> bool {
        true
    }
    fn conversation(&self, pane: &herdr_core::Pane) -> Vec<herdr_core::ConversationItem> {
        match &self.items {
            Some(v) if pane.agent.as_ref().is_some_and(|a| a.name == "lighthouse-start-docs") => v.clone(),
            _ => self.inner.conversation(pane),
        }
    }
    fn revision(&self, _pane: &herdr_core::Pane) -> Option<u64> {
        None
    }
}

fn card_rig(label: &str, items: Option<Vec<herdr_core::ConversationItem>>, screen: &str) -> (App, Arc<RecordingActions>, FakeTerminals) {
    let mut app = App::new(Box::new(Src { inner: flockbook::source::DemoSource::new(), items }), Mode::Light, Screen::Conversation);
    let acts = Arc::new(RecordingActions::new(screen));
    app.state.cards.actions = Some(acts.clone() as Arc<dyn PaneActions>);
    let fake = FakeTerminals::default();
    app.state.terminal.backend = Some(Arc::new(fake.clone()));
    let found = app.state.workspaces.iter().find_map(|w| w.tabs.iter().find(|t| t.label == label).map(|t| (w.id.clone(), t.id.clone()))).expect("demo tab");
    app.state.select_tab(&found.0, &found.1);
    app.refresh();
    (app, acts, fake)
}

#[test]
fn question_card_keys_do_nothing_while_the_sheet_is_open() {
    let q = herdr_core::QuestionRequest::new("toolu_t", vec![herdr_core::Question::new("Pick", "Which parts should ship?", vec![herdr_core::QOption::new("Alpha"), herdr_core::QOption::new("Beta"), herdr_core::QOption::new("Gamma")], false)]);
    let items = vec![herdr_core::ConversationItem { id: "q1".into(), kind: herdr_core::ItemKind::Question(q), timestamp: Some(1.0), ..Default::default() }];
    // With the sheet closed the same keys do pick and send (so the test proves the gate, not a dead card).
    let (mut app, _, _) = card_rig("getting started", Some(items.clone()), &picker());
    let _ = app.update(Message::Cards(CardsMsg::Key(CardKey::Digit(2))));
    assert_eq!(app.state.cards.flow().expect("flow").picks[0], vec![1], "control: the digit picks");

    let (mut app, acts2, fake) = card_rig("getting started", Some(items), &picker());
    let _ = app.update(Message::Terminal(TerminalMsg::Toggle));
    assert!(app.state.terminal.is_open(), "the sheet is open");
    let _ = app.update(Message::Terminal(TerminalMsg::Focus(false)));
    for k in [CardKey::Digit(2), CardKey::Down, CardKey::Enter { chord: false }, CardKey::Escape] {
        let _ = app.update(Message::Cards(CardsMsg::Key(k)));
    }
    for m in [CardsMsg::Pick(1), CardsMsg::Submit, CardsMsg::Back, CardsMsg::ActivateOther, CardsMsg::OtherInput("x".into())] {
        let _ = app.update(Message::Cards(m));
    }
    std::thread::sleep(Duration::from_millis(150));
    assert!(acts2.mutations().is_empty(), "no PaneActions mutation: {:?}", acts2.mutations());
    assert!(app.state.cards.flow().expect("flow").picks[0].is_empty(), "nothing was picked");
    assert!(fake.writes().is_empty());
}

#[test]
fn approval_card_actions_do_nothing_while_the_sheet_is_open() {
    let (mut app, acts, fake) = card_rig("deploy", None, PROMPT);
    let _ = app.update(Message::Terminal(TerminalMsg::Toggle));
    let _ = app.update(Message::Terminal(TerminalMsg::Focus(false)));
    for m in [CardsMsg::Allow, CardsMsg::Deny, CardsMsg::Always(true), CardsMsg::Key(CardKey::Enter { chord: false })] {
        let _ = app.update(Message::Cards(m));
    }
    std::thread::sleep(Duration::from_millis(150));
    assert!(acts.mutations().is_empty(), "{:?}", acts.mutations());
    assert!(!app.state.cards.always, "the box did not change");
    assert!(fake.writes().is_empty());
    // Closed again: Deny works.
    let _ = app.update(Message::Terminal(TerminalMsg::Close));
    let _ = app.update(Message::Cards(CardsMsg::Deny));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(acts.mutations().len(), 1, "after closing, the card works");
}

#[test]
fn opening_the_sheet_takes_the_keyboard_from_the_widgets_below() {
    let (mut r, _) = modal_rig(true);
    let task = r.app.update(Message::Terminal(TerminalMsg::Toggle));
    assert!(task.units() > 0, "an unfocus operation is queued");
}

#[test]
fn history_with_an_open_synchronized_update_still_shows_its_text() {
    let h = terminal::history_view("before\r\n\x1b[?2026hinside\r\nlast\r\n", 20, 3);
    let rows: Vec<String> = (0..h.snap.rows).map(|r| h.snap.row_text(r)).collect();
    assert!(rows.iter().any(|r| r == "inside") && rows.iter().any(|r| r == "last"), "{rows:?}");
}
