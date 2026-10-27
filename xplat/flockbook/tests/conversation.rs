//! Conversation view tests: headless, through `iced_test::Simulator` (tiny-skia). No window opens.

use fb_ui::selectable::flow::{Anchor, FlowEvent, Probe};
use fb_ui::Mode;
use flockbook::conversation::ConversationMsg;
use flockbook::state::Screen;
use flockbook::{App, Message};
use herdr_core::{ConversationItem, ItemKind, Pane, Workspace};
use flockbook::source::{DataSource, DemoSource};
use iced::{keyboard, mouse, window, Event, Point, Settings, Size};
use std::cell::Cell;
use iced_test::Simulator;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const W: f32 = 1280.0;
const H: f32 = 852.0;

fn probe(app: &App) -> Probe {
    app.state.conversation.probe.as_ref().unwrap().borrow().clone()
}

fn sim(app: &App) -> Simulator<'_, Message> {
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
    Simulator::with_size(settings, Size::new(W, H), app.view())
}


// ------------------------------------------------------------------ fake sources

/// Demo tree, custom conversation. `count` can change between refreshes; `earlier` simulates paging.
struct Fake {
    inner: DemoSource,
    items: Rc<RefCell<Vec<ConversationItem>>>,
    earlier: Rc<Cell<usize>>,
}

fn item(id: String, kind: ItemKind) -> ConversationItem {
    ConversationItem { id, kind, timestamp: Some(1_700_000_000.0) }
}

fn chat(n: usize) -> Vec<ConversationItem> {
    (0..n)
        .map(|i| {
            if i % 2 == 0 {
                item(format!("m{i}"), ItemKind::UserText(format!("Question number {i} about the plan")))
            } else {
                item(format!("m{i}"), ItemKind::AssistantText(format!("Answer {i}: the **first** step is small.\n\nSecond paragraph for {i} with \u{4f60}\u{597d}\u{4e16}\u{754c} mixed in.")))
            }
        })
        .collect()
}

thread_local! {
    /// The selected pane shows a status line (each test runs on its own thread).
    static STATUS_TAGS: Cell<bool> = const { Cell::new(false) };
}

impl DataSource for Fake {
    fn status_tags(&self, _pane: &Pane) -> Vec<herdr_core::StatusTag> {
        if !STATUS_TAGS.with(Cell::get) {
            return Vec::new();
        }
        let tag = |kind: &str, text: &str, percent: Option<f64>| herdr_core::StatusTag { kind: kind.into(), text: text.into(), spoken: text.into(), percent, warn: false };
        vec![tag("model", "Model 1", None), tag("effort", "medium", None), tag("context", "ctx 25%", Some(25.0)), tag("cost", "$1.20 est.", None)]
    }
    fn label(&self) -> String {
        "fake".into()
    }
    fn workspaces(&self) -> Vec<Workspace> {
        self.inner.workspaces()
    }
    fn connected(&self) -> bool {
        true
    }
    fn conversation(&self, _pane: &Pane) -> Vec<ConversationItem> {
        self.items.borrow().clone()
    }
    fn can_load_earlier(&self, _pane: &Pane) -> bool {
        self.earlier.get() > 0
    }
    fn load_earlier(&self, _pane: &Pane) -> bool {
        let k = self.earlier.get();
        if k > 0 {
            let mut v: Vec<ConversationItem> = (0..k).map(|i| item(format!("old{i}"), ItemKind::AssistantText(format!("Older note {i}")))).collect();
            v.extend(self.items.borrow().iter().cloned());
            *self.items.borrow_mut() = v;
            self.earlier.set(0);
        }
        false
    }
}

type Items = Rc<RefCell<Vec<ConversationItem>>>;

fn fake_app(items: Vec<ConversationItem>, earlier: usize) -> (App, Items, Rc<Cell<usize>>) {
    let demo = DemoSource::new();
    let (ws, tab) = demo.default_selection();
    let items = Rc::new(RefCell::new(items));
    let earlier = Rc::new(Cell::new(earlier));
    let src = Fake { inner: demo, items: items.clone(), earlier: earlier.clone() };
    let mut app = App::new(Box::new(src), Mode::Light, Screen::Conversation);
    app.state.select_tab(&ws, &tab);
    app.refresh();
    // `refresh` installs the selected pane's own reader (each pane keeps its document, selection and probe), so the
    // probe and the test clock go on the reader that is shown after it, never on the one it replaced.
    app.state.conversation.probe = Some(Rc::new(RefCell::new(Probe::default())));
    app.state.conversation.sel = test_clock();
    (app, items, earlier)
}

/// A selection whose clock moves 50 ms per read (one read per press). On the wall clock a slow runner can put more
/// than the multi-click time between two simulated clicks, and a double-click becomes two single clicks.
fn test_clock() -> fb_ui::selectable::flow::SelState {
    let t = Rc::new(Cell::new(std::time::Instant::now()));
    fb_ui::selectable::flow::SelState::with_clock(fb_ui::selectable::flow::Clock::new(move || {
        t.set(t.get() + std::time::Duration::from_millis(50));
        t.get()
    }))
}

// ------------------------------------------------------------------ driving

fn redraw() -> Event {
    Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))
}
fn moved(p: Point) -> Event {
    Event::Mouse(mouse::Event::CursorMoved { position: p })
}
fn down() -> Event {
    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
}
fn up() -> Event {
    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
}
fn key(c: &str, command: bool) -> Event {
    let modifiers = if command { keyboard::Modifiers::COMMAND } else { keyboard::Modifiers::empty() };
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Character(c.into()),
        modified_key: keyboard::Key::Character(c.into()),
        physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
        location: keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    })
}
fn escape() -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(keyboard::key::Named::Escape),
        modified_key: keyboard::Key::Named(keyboard::key::Named::Escape),
        physical_key: keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    })
}

/// Builds a fresh interface, feeds `events`, applies the messages it published.
fn drive(app: &mut App, events: Vec<Event>) {
    let msgs: Vec<Message> = {
        let mut s = sim(app);
        for e in events {
            // The simulator holds one cursor per call, so each event is its own call.
            if let Event::Mouse(mouse::Event::CursorMoved { position }) = &e {
                s.point_at(*position);
            }
            s.simulate([e]);
        }
        s.into_messages().collect()
    };
    for m in msgs {
        let _ = app.update(m);
    }
}

/// Lays the view out once (publishes viewport and measures) until it settles.
fn settle(app: &mut App) {
    for _ in 0..6 {
        drive(app, vec![redraw()]);
    }
}

/// Window position of the glyph at `offset` in row `idx`.
fn at(app: &App, idx: usize, offset: usize) -> Point {
    let p = probe(app);
    let l = p.point_of(idx, offset).unwrap_or_else(|| panic!("row {idx} is not mounted: {:?}", p.mounted()));
    Point::new(p.origin.x + l.x, p.origin.y + l.y)
}

fn row_of(app: &App, needle: &str) -> usize {
    app.state.conversation.doc.rows.iter().position(|r| r.plain.contains(needle)).unwrap_or_else(|| panic!("no row with {needle}"))
}

fn copied(app: &App) -> Option<String> {
    app.state.conversation.last_copied.clone()
}

fn sel_text(app: &App) -> Option<String> {
    let cs = &app.state.conversation;
    cs.sel.text(&cs.doc)
}

fn click_at(app: &mut App, p: Point) {
    drive(app, vec![moved(p), down(), up()]);
}

// ------------------------------------------------------------------ tests

#[test]
fn double_click_selects_a_word() {
    let (mut app, _, _) = fake_app(chat(6), 0);
    settle(&mut app);
    let r = row_of(&app, "Answer 1");
    let off = app.state.conversation.doc.rows[r].plain.find("first").unwrap() + 2;
    let p = at(&app, r, off);
    click_at(&mut app, p);
    // A slow machine (the debug build on a CI runner): the second click lands after the system's multi-click time.
    // The clicks still count as one double-click: the test clock, not the wall clock, decides.
    std::thread::sleep(fb_ui::selectable::flow::MULTI_CLICK + std::time::Duration::from_millis(100));
    click_at(&mut app, p);
    assert_eq!(sel_text(&app).as_deref(), Some("first"));
}

#[test]
fn double_click_on_cjk_selects_a_cjk_word() {
    let (mut app, _, _) = fake_app(chat(6), 0);
    settle(&mut app);
    let r = row_of(&app, "\u{4f60}\u{597d}");
    let off = app.state.conversation.doc.rows[r].plain.find('\u{4f60}').unwrap();
    let p = at(&app, r, off + 3);
    click_at(&mut app, p);
    click_at(&mut app, p);
    let t = sel_text(&app).expect("a selection");
    assert!(t.contains('\u{4f60}') && !t.contains("mixed"), "{t}");
}

#[test]
fn triple_click_selects_the_paragraph() {
    let (mut app, _, _) = fake_app(chat(6), 0);
    settle(&mut app);
    let r = row_of(&app, "Answer 1");
    let off = app.state.conversation.doc.rows[r].plain.find("step").unwrap();
    let p = at(&app, r, off);
    for _ in 0..3 {
        click_at(&mut app, p);
    }
    assert_eq!(sel_text(&app).as_deref(), Some("Answer 1: the first step is small."));
}

#[test]
fn shift_click_extends_the_selection() {
    let (mut app, _, _) = fake_app(chat(6), 0);
    settle(&mut app);
    let r = row_of(&app, "Answer 1");
    let a = at(&app, r, 1);
    let a = Point::new(a.x - 3.0, a.y);
    let end = app.state.conversation.doc.rows[r].plain.len();
    let b = at(&app, r, end - 1);
    let b = Point::new(b.x + 30.0, b.y);
    drive(&mut app, vec![moved(a), down(), moved(Point::new(a.x + 40.0, a.y)), up()]);
    let first = sel_text(&app).expect("drag selected something");
    assert!(first.len() < end);
    let shift = Event::Keyboard(keyboard::Event::ModifiersChanged(keyboard::Modifiers::SHIFT));
    drive(&mut app, vec![shift, moved(b), down(), up()]);
    let t = sel_text(&app).expect("extended");
    assert!(t.len() > first.len() + 10 && t.ends_with("first step is small."), "{first:?} -> {t:?}");
}

#[test]
fn drag_across_messages_then_copy() {
    let (mut app, _, _) = fake_app(chat(6), 0);
    settle(&mut app);
    let r0 = row_of(&app, "Question number 0");
    let r1 = row_of(&app, "Answer 1");
    let a = at(&app, r0, 9);
    let b = at(&app, r1, 6);
    drive(&mut app, vec![moved(a), down(), moved(b), up()]);
    let t = sel_text(&app).expect("selection");
    assert!(t.starts_with("number 0") || t.starts_with("r 0") || t.contains("about the plan"), "{t}");
    assert!(t.contains("Answer"), "{t}");
    drive(&mut app, vec![key("c", true)]);
    assert_eq!(copied(&app), Some(t));
}

#[test]
fn select_all_copy_and_escape() {
    let (mut app, _, _) = fake_app(chat(4), 0);
    settle(&mut app);
    let r = row_of(&app, "Answer 1");
    let p = at(&app, r, 3);
    click_at(&mut app, p);
    drive(&mut app, vec![key("a", true)]);
    let all = sel_text(&app).expect("all selected");
    assert!(all.contains("Question number 0") && all.contains("Answer 3"), "{all}");
    drive(&mut app, vec![key("c", true)]);
    assert_eq!(copied(&app), Some(all));
    drive(&mut app, vec![escape()]);
    assert!(sel_text(&app).is_none());
}

#[test]
fn keys_do_nothing_without_focus() {
    let (mut app, _, _) = fake_app(chat(4), 0);
    settle(&mut app);
    drive(&mut app, vec![key("a", true)]);
    assert!(sel_text(&app).is_none());
}

#[test]
fn drag_autoscroll_reaches_unmounted_rows() {
    let (mut app, _, _) = fake_app(chat(600), 0);
    settle(&mut app);
    // Leave the bottom and go to the top.
    let _ = app.update(Message::Conversation(ConversationMsg::Flow(FlowEvent::Key(fb_ui::selectable::flow::FlowKey::Home))));
    settle(&mut app);
    let p = probe(&app);
    assert!(p.mounted().contains(&0));
    let last_mounted = *p.mounted().last().unwrap();
    assert!(last_mounted < 100, "window is small: {last_mounted}");
    let start = at(&app, row_of(&app, "Question number 0"), 9);
    let below = Point::new(start.x, p.origin.y + p.view.height + 30.0);
    let mut s_events = vec![moved(start), down(), moved(below)];
    drive(&mut app, s_events.drain(..).collect());
    assert!(app.state.conversation.edge.is_some(), "drag beyond the bottom edge sets an auto-scroll speed");
    // The pointer stays at `below`; each tick scrolls and the widget re-hits the cursor on redraw.
    let before = app.state.conversation.offset;
    for _ in 0..400 {
        let _ = app.update(Message::Conversation(ConversationMsg::AutoScroll));
        drive(&mut app, vec![moved(below), redraw()]);
    }
    assert!(app.state.conversation.offset > before + 2000.0, "scrolled {}", app.state.conversation.offset);
    drive(&mut app, vec![up()]);
    assert!(app.state.conversation.edge.is_none());
    let t = sel_text(&app).expect("selection");
    let reached = (0..600).rev().find(|i| t.contains(&format!("Question number {i} ")) || t.contains(&format!("Answer {i}:"))).unwrap();
    assert!(reached > last_mounted + 20, "copied text stops at {reached}, mounted ended at {last_mounted}");
    drive(&mut app, vec![key("c", true)]);
    assert_eq!(copied(&app).as_deref(), Some(t.as_str()));
}

#[test]
fn five_thousand_messages_render() {
    let (mut app, _, _) = fake_app(chat(5000), 0);
    settle(&mut app);
    let p = probe(&app);
    assert!(!p.placed.is_empty());
    assert!(p.placed.len() < 200, "mounted {}", p.placed.len());
    assert_eq!(*p.mounted().last().unwrap(), app.state.conversation.doc.rows.len() - 1, "starts at the bottom");
    let mut s = sim(&app);
    s.snapshot(&app.theme()).expect("snapshot");
}

#[test]
fn append_while_pinned_stays_at_the_bottom() {
    let (mut app, items, _) = fake_app(chat(40), 0);
    settle(&mut app);
    assert!(app.state.conversation.pinned);
    let n = app.state.conversation.doc.rows.len();
    items.borrow_mut().push(item("m40".into(), ItemKind::AssistantText("A fresh answer arrives.\n\nWith a second paragraph.".into())));
    app.refresh();
    settle(&mut app);
    assert!(app.state.conversation.doc.rows.len() > n);
    let p = probe(&app);
    assert!(app.state.conversation.pinned);
    let last = app.state.conversation.doc.rows.len() - 1;
    assert_eq!(*p.mounted().last().unwrap(), last);
    assert!((p.eff + p.view.height - p.total).abs() < 2.0, "eff {} view {} total {}", p.eff, p.view.height, p.total);
}

#[test]
fn append_while_scrolled_up_keeps_the_place() {
    let (mut app, items, _) = fake_app(chat(60), 0);
    settle(&mut app);
    let _ = app.update(Message::Conversation(ConversationMsg::Flow(FlowEvent::Wheel(-800.0))));
    settle(&mut app);
    assert!(!app.state.conversation.pinned);
    let before = probe(&app).eff;
    items.borrow_mut().push(item("m60".into(), ItemKind::AssistantText("Late news.".into())));
    app.refresh();
    settle(&mut app);
    assert!((probe(&app).eff - before).abs() < 1.0);
}

#[test]
fn scrolling_to_the_top_loads_earlier() {
    let (mut app, items, earlier) = fake_app(chat(30), 5);
    settle(&mut app);
    assert!(app.state.conversation.more_earlier);
    let before = items.borrow().len();
    let _ = app.update(Message::Conversation(ConversationMsg::Flow(FlowEvent::Key(fb_ui::selectable::flow::FlowKey::Home))));
    settle(&mut app);
    assert_eq!(earlier.get(), 0);
    assert_eq!(items.borrow().len(), before + 5);
    assert!(!app.state.conversation.more_earlier);
    assert!(app.state.conversation.doc.rows.iter().any(|r| r.plain.contains("Older note 0")));
}

#[test]
fn selection_survives_an_append() {
    let (mut app, items, _) = fake_app(chat(6), 0);
    settle(&mut app);
    let r = row_of(&app, "Answer 1");
    let off = app.state.conversation.doc.rows[r].plain.find("first").unwrap() + 1;
    let p = at(&app, r, off);
    click_at(&mut app, p);
    // A slow machine (the debug build on a CI runner): the second click lands after the system's multi-click time.
    // The clicks still count as one double-click: the test clock, not the wall clock, decides.
    std::thread::sleep(fb_ui::selectable::flow::MULTI_CLICK + std::time::Duration::from_millis(100));
    click_at(&mut app, p);
    assert_eq!(sel_text(&app).as_deref(), Some("first"));
    items.borrow_mut().push(item("m6".into(), ItemKind::UserText("one more".into())));
    app.refresh();
    assert_eq!(sel_text(&app).as_deref(), Some("first"));
}

#[test]
fn demo_conversation_renders_light_and_dark() {
    for mode in [Mode::Light, Mode::Dark] {
        let mut app = App::demo(mode, Screen::Conversation);
        settle(&mut app);
        let mut s = sim(&app);
        let snap = s.snapshot(&app.theme()).expect("snapshot");
        let _ = snap;
    }
}

#[test]
fn public_hygiene_of_demo_text() {
    let text: String = chat(4).iter().map(|i| format!("{:?}", i.kind)).collect();
    for bad in [concat!("/Us", "ers/"), concat!("lim", "inchen")] {
        assert!(!text.contains(bad));
    }
    let _ = Anchor { id: 0, offset: 0 };
}

/// Frame timing at 5,000 messages. Run in release: `cargo test -p flockbook --release --test conversation bench -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_5k_frames() {
    let (mut app, _, _) = fake_app(chat(5000), 0);
    settle(&mut app);
    let mut times: Vec<f64> = Vec::new();
    for i in 0..200 {
        let _ = app.update(Message::Conversation(ConversationMsg::Flow(FlowEvent::Wheel(-173.0 - (i % 7) as f32 * 31.0))));
        let t = std::time::Instant::now();
        let mut s = sim(&app);
        let _ = s.snapshot(&app.theme());
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        drop(s);
        drive(&mut app, vec![redraw()]);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rss = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    println!("BENCH frames={} p50={:.2}ms p95={:.2}ms rss={}KB mounted={}", times.len(), times[times.len() / 2], times[times.len() * 95 / 100], rss, probe(&app).placed.len());
}

/// 5,000 messages, scrolled to the middle. `FB_WRITE_EVIDENCE=1` writes evidence/app-conversation-5k-mid.png.
#[test]
fn five_thousand_messages_middle_snapshot() {
    let (mut app, _, _) = fake_app(chat(5000), 0);
    settle(&mut app);
    let total = probe(&app).total;
    let _ = app.update(Message::Conversation(ConversationMsg::Flow(FlowEvent::ScrollTo(total / 2.0))));
    settle(&mut app);
    let p = probe(&app);
    assert!(!p.placed.is_empty() && p.placed.len() < 200);
    assert!(!app.state.conversation.pinned);
    assert!(*p.mounted().first().unwrap() > 1000 && *p.mounted().last().unwrap() < app.state.conversation.doc.rows.len() - 1000);
    if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
        let img = flockbook::snapshot::render_app(&app, (1280, 852));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence/app-conversation-5k-mid.png");
        flockbook::snapshot::write_png(&img, &path).unwrap();
    }
}

/// Scrolled up with a status line: "Jump to latest" floats centred over the rows, the status line sits right over the
/// composer and takes only its row. `FB_WRITE_EVIDENCE=1` writes evidence/app-conversation-scrolled-{light,dark}.png.
#[test]
fn scrolled_up_with_a_status_line() {
    STATUS_TAGS.with(|c| c.set(true));
    for (mode, name) in [(Mode::Light, "light"), (Mode::Dark, "dark")] {
        let (mut app, _, _) = fake_app(chat(400), 0);
        app.state.mode = mode;
        settle(&mut app);
        let total = probe(&app).total;
        let _ = app.update(Message::Conversation(ConversationMsg::Flow(FlowEvent::ScrollTo(total / 2.0))));
        settle(&mut app);
        assert!(!app.state.conversation.pinned);
        let mut s = sim(&app);
        assert!(s.find("Model 1").is_ok(), "the status line shows");
        let cs = &app.state.conversation;
        let newer = cs.doc.rows.len().saturating_sub(cs.doc.window.range.1);
        let label = if newer > 0 { format!("\u{2193} Jump to latest ({newer} newer)") } else { "\u{2193} Jump to latest".to_string() };
        assert!(s.find(label.as_str()).is_ok(), "the jump button shows");
        drop(s);
        // The rows end right over the status line: it takes one row, not half the column.
        let viewport = app.state.conversation.viewport;
        assert!(viewport.height > 852.0 - 260.0, "the conversation keeps the height: {viewport:?}");
        if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
            let img = flockbook::snapshot::render_app(&app, (1280, 852));
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("evidence/app-conversation-scrolled-{name}.png"));
            flockbook::snapshot::write_png(&img, &path).unwrap();
        }
    }
}

// ------------------------------------------------------------------ links

#[test]
fn clicking_a_link_records_the_url_and_spawns_nothing() {
    let items = vec![
        item("u".into(), ItemKind::UserText("hi".into())),
        item("a".into(), ItemKind::AssistantText("Read [the docs](https://example.com/docs) and [bad](javascript:alert(1)) first.".into())),
    ];
    let (mut app, _, _) = fake_app(items, 0);
    // App::new installs a recorder, not the system opener.
    assert!(app.opener.recorded().is_empty());
    settle(&mut app);
    let r = row_of(&app, "Read the docs");
    let off = app.state.conversation.doc.rows[r].plain.find("docs").unwrap() + 1;
    let p = at(&app, r, off);
    click_at(&mut app, p);
    assert_eq!(app.opener.recorded(), vec!["https://example.com/docs".to_string()]);
    assert_eq!(app.state.conversation.last_link.as_deref(), Some("https://example.com/docs"));
    // A scheme that is not web or mail is remembered as clicked but never handed to the opener.
    let off = app.state.conversation.doc.rows[r].plain.find("bad").unwrap() + 1;
    let p = at(&app, r, off);
    click_at(&mut app, p);
    assert_eq!(app.opener.recorded().len(), 1);
}

// ------------------------------------------------------------------ idle ticks

struct Counting {
    inner: DemoSource,
    fetches: Rc<Cell<usize>>,
    rev: Rc<Cell<u64>>,
}

impl DataSource for Counting {
    fn label(&self) -> String {
        "counting".into()
    }
    fn workspaces(&self) -> Vec<Workspace> {
        self.inner.workspaces()
    }
    fn connected(&self) -> bool {
        true
    }
    fn conversation(&self, pane: &Pane) -> Vec<ConversationItem> {
        self.fetches.set(self.fetches.get() + 1);
        self.inner.conversation(pane)
    }
    fn revision(&self, _pane: &Pane) -> Option<u64> {
        Some(self.rev.get())
    }
}

#[test]
fn idle_ticks_do_not_fetch_or_rebuild_when_the_source_has_a_revision() {
    let demo = DemoSource::new();
    let (ws, tab) = demo.default_selection();
    let (fetches, rev) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(1u64)));
    let mut app = App::new(Box::new(Counting { inner: demo, fetches: fetches.clone(), rev: rev.clone() }), Mode::Light, Screen::Conversation);
    app.state.select_tab(&ws, &tab);
    app.refresh();
    let (f0, b0) = (fetches.get(), app.state.conversation.rebuilds);
    assert!(f0 >= 1 && b0 >= 1);
    for _ in 0..20 {
        let _ = app.update(Message::Tick);
    }
    assert_eq!((fetches.get(), app.state.conversation.rebuilds), (f0, b0), "idle ticks are free");
    // A new revision fetches and rebuilds once.
    rev.set(2);
    let _ = app.update(Message::Tick);
    assert_eq!((fetches.get(), app.state.conversation.rebuilds), (f0 + 1, b0 + 1));
    let _ = app.update(Message::Tick);
    assert_eq!((fetches.get(), app.state.conversation.rebuilds), (f0 + 1, b0 + 1));
}

#[test]
fn idle_ticks_do_not_rebuild_without_a_revision_and_a_new_item_does() {
    let (mut app, items, _) = fake_app(chat(8), 0);
    let b0 = app.state.conversation.rebuilds;
    for _ in 0..20 {
        let _ = app.update(Message::Tick);
    }
    assert_eq!(app.state.conversation.rebuilds, b0);
    items.borrow_mut().push(item("m8".into(), ItemKind::UserText("new".into())));
    let _ = app.update(Message::Tick);
    assert_eq!(app.state.conversation.rebuilds, b0 + 1);
    // The last item grows in place (same id): that counts as a change too.
    items.borrow_mut().last_mut().unwrap().kind = ItemKind::UserText("new and longer".into());
    let _ = app.update(Message::Tick);
    assert_eq!(app.state.conversation.rebuilds, b0 + 2);
}

// ------------------------------------------------------------------ async load earlier

struct Paged {
    inner: DemoSource,
    items: Arc<Mutex<Vec<ConversationItem>>>,
    jobs_run: Arc<AtomicUsize>,
}

impl DataSource for Paged {
    fn label(&self) -> String {
        "paged".into()
    }
    fn workspaces(&self) -> Vec<Workspace> {
        self.inner.workspaces()
    }
    fn connected(&self) -> bool {
        true
    }
    fn conversation(&self, _pane: &Pane) -> Vec<ConversationItem> {
        self.items.lock().unwrap().clone()
    }
    fn can_load_earlier(&self, _pane: &Pane) -> bool {
        true
    }
    fn load_earlier_job(&self, _pane: &Pane) -> Option<flockbook::source::EarlierJob> {
        let runs = self.jobs_run.clone();
        Some(Box::new(move || {
            runs.fetch_add(1, Ordering::SeqCst);
            false
        }))
    }
}

#[test]
fn load_earlier_runs_off_the_ui_thread_and_stale_results_are_dropped() {
    let demo = DemoSource::new();
    let (ws, tab) = demo.default_selection();
    let items = Arc::new(Mutex::new(chat(30)));
    let jobs_run = Arc::new(AtomicUsize::new(0));
    let mut app = App::new(Box::new(Paged { inner: demo, items: items.clone(), jobs_run: jobs_run.clone() }), Mode::Light, Screen::Conversation);
    app.state.select_tab(&ws, &tab);
    app.refresh();
    app.state.conversation.probe = Some(Rc::new(RefCell::new(Probe::default())));
    settle(&mut app);
    let pane = app.state.conversation.pane.clone().unwrap();
    assert!(app.state.conversation.more_earlier);

    // Scroll to the top: the update starts the load but does not run it.
    let _ = app.update(Message::Conversation(ConversationMsg::Flow(FlowEvent::Key(fb_ui::selectable::flow::FlowKey::Home))));
    assert_eq!(jobs_run.load(Ordering::SeqCst), 0, "the read is not done inside update");
    let ticket = app.state.conversation.earlier_seq;
    assert!(app.state.conversation.loading_earlier);
    assert!(app.state.conversation.doc.rows.iter().any(|r| matches!(r.kind, fb_ui::selectable::flow::RowKind::LoadEarlier { loading: true })));

    // The job finishes: older items appeared in the source.
    items.lock().unwrap().insert(0, item("old0".into(), ItemKind::AssistantText("Older note 0".into())));
    // A result of an older load, or for another pane, changes nothing.
    let _ = app.update(Message::Conversation(ConversationMsg::EarlierLoaded { pane: pane.clone(), ticket: ticket - 1, more: Some(false) }));
    let _ = app.update(Message::Conversation(ConversationMsg::EarlierLoaded { pane: "other".into(), ticket, more: Some(false) }));
    assert!(app.state.conversation.loading_earlier, "stale results are ignored");
    assert!(app.state.conversation.more_earlier);
    // The matching result lands.
    let _ = app.update(Message::Conversation(ConversationMsg::EarlierLoaded { pane, ticket, more: Some(false) }));
    assert!(!app.state.conversation.loading_earlier);
    assert!(!app.state.conversation.more_earlier);
    assert!(app.state.conversation.doc.rows.iter().any(|r| r.plain.contains("Older note 0")));
}

#[test]
fn a_load_that_finishes_after_a_pane_switch_is_dropped() {
    let demo = DemoSource::new();
    let (ws, tab) = demo.default_selection();
    let other_tab = demo.workspaces().iter().find(|w| w.id == ws).unwrap().tabs[1].id.clone();
    let items = Arc::new(Mutex::new(chat(30)));
    let mut app = App::new(Box::new(Paged { inner: demo, items, jobs_run: Arc::new(AtomicUsize::new(0)) }), Mode::Light, Screen::Conversation);
    app.state.select_tab(&ws, &tab);
    app.refresh();
    settle(&mut app);
    let pane = app.state.conversation.pane.clone().unwrap();
    let _ = app.update(Message::Conversation(ConversationMsg::Flow(FlowEvent::Key(fb_ui::selectable::flow::FlowKey::Home))));
    let ticket = app.state.conversation.earlier_seq;
    let _ = app.update(Message::SelectTab { workspace: ws, tab: other_tab });
    let _ = app.update(Message::Conversation(ConversationMsg::EarlierLoaded { pane, ticket, more: Some(false) }));
    assert!(app.state.conversation.more_earlier, "the other pane's flags are untouched");
    assert!(!app.state.conversation.loading_earlier);
}

// ------------------------------------------------------------------ empty document, syntax rows

#[test]
fn an_empty_flow_document_survives_draw_click_and_copy() {
    use fb_ui::selectable::flow::{Doc, Flow};
    let doc = Doc::default();
    std::env::set_var("ICED_TEST_BACKEND", "tiny-skia");
    let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
    let t = fb_ui::Tokens::LIGHT;
    let msgs: Vec<flockbook::conversation::ConversationMsg> = {
        let mut s = Simulator::with_size(settings, Size::new(W, H), iced::Element::from(Flow::new(&doc, t, ConversationMsg::Flow)));
        s.point_at(Point::new(300.0, 300.0));
        s.simulate([down(), moved(Point::new(500.0, 500.0)), up(), key("a", true), key("c", true), redraw()]);
        let _ = s.snapshot(&iced::Theme::Light).expect("draws");
        s.into_messages().collect()
    };
    let mut sel = fb_ui::selectable::flow::SelState::default();
    for m in msgs {
        if let ConversationMsg::Flow(FlowEvent::Press { at, point, shift }) = m {
            sel.press(&doc, at, point, shift);
        }
    }
    assert!(sel.text(&doc).is_none());
}

#[test]
fn demo_code_block_has_syntax_colored_runs() {
    use fb_ui::highlight::Tone;
    use fb_ui::selectable::flow::RowKind;
    let app = App::demo(Mode::Light, Screen::Conversation);
    let row = app.state.conversation.doc.rows.iter().find(|r| matches!(&r.kind, RowKind::Code { lang } if lang.as_deref() == Some("python"))).expect("python block in the demo");
    let tones: std::collections::HashSet<Tone> = row.paras[0].inlines.iter().map(|i| i.tone).collect();
    for t in [Tone::Keyword, Tone::Number, Tone::Function, Tone::Comment, Tone::Plain] {
        assert!(tones.contains(&t), "{t:?}");
    }
    assert!(row.plain.starts_with("def score(doc, bm25, now):"), "copy gives the code without markup");
    let last = app.state.conversation.doc.rows.iter().rposition(|r| matches!(r.kind, RowKind::Code { .. })).unwrap();
    assert!(last + 4 >= app.state.conversation.doc.rows.len(), "the block sits near the end");
}
