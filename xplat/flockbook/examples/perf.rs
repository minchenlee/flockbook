//! Headless performance bench: no window, no socket, no herdr.
//!
//! `cargo run --release -p flockbook --example perf` (or without `--release` to see the dev profile).
//! `perf <scenario>` runs one of: startup, idle, scroll, earlier, updates, overview, terminal.
//!
//! Frames are warm, as in the running app: each frame builds the view, lays it out against the previous frame's
//! `user_interface::Cache`, feeds a redraw (plus any input) and draws, and the messages it emits go back through
//! `App::update`. The renderer is the CPU one (tiny-skia) without the final rasterization, so the numbers are the
//! UI thread's own work per frame: update, view, layout, text shaping and drawing.

use fb_ui::Mode;
use flockbook::source::{DataSource, DemoSource};
use flockbook::state::Screen;
use flockbook::{App, Message};
use herdr_core::{AgentStatus, ConversationItem, ItemKind, Pane, Tab, ToolCall, ToolChip, Workspace};
use iced::{mouse, window, Event, Point, Settings, Size};
use iced_test::core::renderer::Headless;
use iced_test::core::{clipboard, renderer, theme::Base};
use iced_test::runtime::user_interface::{self, UserInterface};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

const W: f32 = 1280.0;
const H: f32 = 852.0;

// ------------------------------------------------------------------------------------------------ frame driver

struct Frames {
    renderer: iced_test::renderer::Renderer,
    cache: Option<user_interface::Cache>,
    cursor: mouse::Cursor,
    /// Also rasterize each frame (`screenshot` at 2x), as presenting it would.
    raster: bool,
}

/// Time of one frame, split into its parts (ms).
#[derive(Default, Clone, Copy)]
struct Cost {
    update: f64,
    view: f64,
    layout: f64,
    draw: f64,
    raster: f64,
    /// CPU time of the whole process over the frame, all threads (the GPU driver's too). Not part of `total`.
    cpu: f64,
}

impl Cost {
    fn total(&self) -> f64 {
        self.update + self.view + self.layout + self.draw + self.raster
    }
}

impl Frames {
    fn new() -> Frames {
        // `--wgpu`: the GPU renderer on an offscreen device (no window). Default: tiny-skia, the CPU renderer.
        let backend = if std::env::args().any(|a| a == "--wgpu") { "wgpu" } else { "tiny-skia" };
        std::env::set_var("ICED_TEST_BACKEND", backend);
        // The simulator loads the bundled fonts into the shared font system; the renderer below then finds them.
        let settings = Settings { fonts: fb_ui::fonts::bytes(), default_font: fb_ui::fonts::default_font(), ..Settings::default() };
        drop(iced_test::Simulator::<'_, ()>::with_size(settings, Size::new(10.0, 10.0), iced::widget::Space::new()));
        let renderer = iced_test::futures::futures::executor::block_on(iced_test::renderer::Renderer::new(fb_ui::fonts::default_font(), iced::Pixels(16.0), Some(backend))).expect("headless renderer");
        let raster = std::env::args().any(|a| a == "--raster");
        Frames { renderer, cache: None, cursor: mouse::Cursor::Available(Point::new(800.0, 420.0)), raster }
    }

    /// One frame with `events` (a redraw is always added). Messages the widgets emit are applied after it, as the
    /// runtime does; their update time counts toward this frame.
    fn frame(&mut self, app: &mut App, events: &[Event]) -> Cost {
        let cpu0 = cpu_ms();
        let mut cost = Cost::default();
        let theme = app.theme();
        let mut msgs: Vec<Message> = Vec::new();
        {
            let t = Instant::now();
            let el = app.view();
            cost.view = ms(t);
            let t = Instant::now();
            let mut ui = UserInterface::build(el, Size::new(W, H), self.cache.take().unwrap_or_default(), &mut self.renderer);
            cost.layout = ms(t);
            let t = Instant::now();
            let mut evs: Vec<Event> = events.to_vec();
            for e in &evs {
                if let Event::Mouse(mouse::Event::CursorMoved { position }) = e {
                    self.cursor = mouse::Cursor::Available(*position);
                }
            }
            evs.push(Event::Window(window::Event::RedrawRequested(Instant::now())));
            let _ = ui.update(&evs, self.cursor, &mut self.renderer, &mut clipboard::Null, &mut msgs);
            ui.draw(&mut self.renderer, &theme, &renderer::Style { text_color: theme.base().text_color }, self.cursor);
            cost.draw = ms(t);
            self.cache = Some(ui.into_cache());
        }
        if self.raster {
            let t = Instant::now();
            let _ = self.renderer.screenshot(Size::new((W * 2.0) as u32, (H * 2.0) as u32), 2.0, theme.base().background_color);
            cost.raster = ms(t);
        }
        cost.cpu = cpu_ms() - cpu0;
        let t = Instant::now();
        for m in msgs {
            let _ = app.update(m);
        }
        cost.update = ms(t);
        cost
    }

    /// `update(msg)` then one frame: what one timer tick or one source change costs.
    fn tick(&mut self, app: &mut App, msg: Message) -> Cost {
        let t = Instant::now();
        let _ = app.update(msg);
        let up = ms(t);
        let mut c = self.frame(app, &[]);
        c.update += up;
        c.cpu += up;
        c
    }

    /// An empty window: what the renderer costs with nothing on screen (with `--raster`, mostly the readback that a
    /// presented frame does not have). Subtract it from a frame to estimate the cost of drawing the UI.
    fn empty(&mut self) -> Cost {
        let cpu0 = cpu_ms();
        let t = Instant::now();
        let mut ui: UserInterface<'_, Message, iced::Theme, _> = UserInterface::build(iced::widget::Space::new(), Size::new(W, H), user_interface::Cache::default(), &mut self.renderer);
        let theme = iced::Theme::Light;
        let _ = ui.update(&[Event::Window(window::Event::RedrawRequested(Instant::now()))], self.cursor, &mut self.renderer, &mut clipboard::Null, &mut Vec::new());
        ui.draw(&mut self.renderer, &theme, &renderer::Style { text_color: theme.base().text_color }, self.cursor);
        let mut cost = Cost { draw: ms(t), ..Cost::default() };
        if self.raster {
            let t = Instant::now();
            let _ = self.renderer.screenshot(Size::new((W * 2.0) as u32, (H * 2.0) as u32), 2.0, theme.base().background_color);
            cost.raster = ms(t);
        }
        cost.cpu = cpu_ms() - cpu0;
        cost
    }

    /// `n` frames that only redraw, as the runtime does when a widget asked for a redraw and no message came: the
    /// same `UserInterface` takes the redraw event and draws again; no `update`, no `view`, no layout. Returns the
    /// costs and the redraw request of the last frame (a turning arc asks for the next one).
    fn redraws(&mut self, app: &App, n: usize) -> (Vec<Cost>, iced::window::RedrawRequest) {
        let theme = app.theme();
        let mut ui = UserInterface::build(app.view(), Size::new(W, H), self.cache.take().unwrap_or_default(), &mut self.renderer);
        let mut costs = Vec::new();
        let mut last = iced::window::RedrawRequest::Wait;
        for _ in 0..n {
            let cpu0 = cpu_ms();
            let t = Instant::now();
            let (state, _) = ui.update(&[Event::Window(window::Event::RedrawRequested(Instant::now()))], self.cursor, &mut self.renderer, &mut clipboard::Null, &mut Vec::new());
            if let user_interface::State::Updated { redraw_request, .. } = state {
                last = redraw_request;
            }
            ui.draw(&mut self.renderer, &theme, &renderer::Style { text_color: theme.base().text_color }, self.cursor);
            let mut cost = Cost { draw: ms(t), ..Cost::default() };
            if self.raster {
                let t = Instant::now();
                let _ = self.renderer.screenshot(Size::new((W * 2.0) as u32, (H * 2.0) as u32), 2.0, theme.base().background_color);
                cost.raster = ms(t);
            }
            cost.cpu = cpu_ms() - cpu0;
            costs.push(cost);
        }
        self.cache = Some(ui.into_cache());
        (costs, last)
    }

    fn settle(&mut self, app: &mut App) {
        for _ in 0..6 {
            self.frame(app, &[]);
        }
    }
}

/// User + system CPU time of this process so far (ms).
fn cpu_ms() -> f64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    let tv = |t: libc::timeval| t.tv_sec as f64 * 1000.0 + t.tv_usec as f64 / 1000.0;
    tv(u.ru_utime) + tv(u.ru_stime)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

struct Stats {
    n: usize,
    p50: f64,
    p95: f64,
    max: f64,
    mean: Cost,
}

fn stats(costs: &[Cost]) -> Stats {
    let mut v: Vec<f64> = costs.iter().map(Cost::total).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = costs.len().max(1) as f64;
    let mean = costs.iter().fold(Cost::default(), |a, c| Cost { update: a.update + c.update / n, view: a.view + c.view / n, layout: a.layout + c.layout / n, draw: a.draw + c.draw / n, raster: a.raster + c.raster / n, cpu: a.cpu + c.cpu / n });
    Stats { n: v.len(), p50: v[v.len() / 2], p95: v[(v.len() * 95 / 100).min(v.len() - 1)], max: *v.last().unwrap(), mean }
}

fn report(name: &str, costs: &[Cost]) {
    let s = stats(costs);
    println!(
        "{name:<36} n={:<4} p50={:>7.2}ms p95={:>7.2}ms max={:>7.2}ms | mean update {:.2} view {:.2} layout {:.2} draw {:.2} raster {:.2} | cpu {:.2}",
        s.n, s.p50, s.p95, s.max, s.mean.update, s.mean.view, s.mean.layout, s.mean.draw, s.mean.raster, s.mean.cpu
    );
}

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output();
    out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok()).unwrap_or(0.0) / 1024.0
}

// ------------------------------------------------------------------------------------------------ synthetic data

fn item(id: String, kind: ItemKind) -> ConversationItem {
    ConversationItem { id, kind, timestamp: Some(1_700_000_000.0) }
}

fn chip(id: String, text: &str, tool: &str, summary: &str) -> ToolChip {
    ToolChip { id: id.clone(), symbol: "terminal".into(), text: text.into(), calls: vec![ToolCall { id: format!("{id}-c"), tool_name: tool.into(), summary: summary.into(), input: summary.into(), output: Some("ok".into()), is_error: false }] }
}

/// A long working session: questions, tool chips, and markdown answers with headings, lists, code and tables.
fn session(n: usize, prefix: &str) -> Vec<ConversationItem> {
    (0..n)
        .map(|i| {
            let id = format!("{prefix}{i}");
            match i % 4 {
                0 => item(id, ItemKind::UserText(format!("Step {i}: can you check why the ranking test fails on the second run?"))),
                1 => item(id.clone(), ItemKind::ToolChips(vec![chip(format!("{id}a"), "Read 3 files", "Read", "src/ranking/score.py"), chip(format!("{id}b"), "Ran a command", "Bash", "pytest tests/ranking -q"), chip(format!("{id}c"), "Edited a file", "Edit", "src/ranking/score.py")])),
                2 => item(
                    id,
                    ItemKind::AssistantText(format!(
                        "## Finding {i}\n\nThe score uses `datetime.now()` so the **second** run sees a later time and the order changes.\n\n- Freeze the clock in the fixture\n- Pass `now` into `score()`\n- Keep the old signature for callers\n\n```python\ndef score(item, now):\n    age = (now - item.created).total_seconds()\n    return item.votes / (age + 2) ** 1.5\n```\n\n| Run | Before | After |\n|---|---|---|\n| 1 | pass | pass |\n| 2 | fail | pass |\n"
                    )),
                ),
                _ => item(id, ItemKind::AssistantText(format!("Done with step {i}. The suite passes twice in a row now, and the change is small: one argument and a fixture. 你好，這一段也有中文。"))),
            }
        })
        .collect()
}

/// The demo tree with one shared, mutable conversation for every pane, and an optional larger tree.
struct Synth {
    workspaces: Vec<Workspace>,
    items: Rc<RefCell<Vec<ConversationItem>>>,
    earlier: Rc<RefCell<Vec<ConversationItem>>>,
    rev: Rc<RefCell<u64>>,
}

impl DataSource for Synth {
    fn label(&self) -> String {
        "bench".into()
    }
    fn workspaces(&self) -> Vec<Workspace> {
        self.workspaces.clone()
    }
    fn connected(&self) -> bool {
        true
    }
    fn conversation(&self, _pane: &Pane) -> Vec<ConversationItem> {
        self.items.borrow().clone()
    }
    fn revision(&self, _pane: &Pane) -> Option<u64> {
        Some(*self.rev.borrow())
    }
    fn can_load_earlier(&self, _pane: &Pane) -> bool {
        !self.earlier.borrow().is_empty()
    }
    fn load_earlier(&self, _pane: &Pane) -> bool {
        let older: Vec<ConversationItem> = std::mem::take(&mut *self.earlier.borrow_mut());
        let mut v = older;
        v.extend(self.items.borrow().iter().cloned());
        *self.items.borrow_mut() = v;
        *self.rev.borrow_mut() += 1;
        false
    }
}

struct Rig {
    app: App,
    items: Rc<RefCell<Vec<ConversationItem>>>,
    rev: Rc<RefCell<u64>>,
}

fn big_tree(workspaces: usize, tabs: usize) -> Vec<Workspace> {
    let statuses = [AgentStatus::Working, AgentStatus::Idle, AgentStatus::Done, AgentStatus::Blocked];
    (0..workspaces)
        .map(|w| Workspace {
            id: format!("w{w}"),
            label: format!("Workspace {w}"),
            tabs: (0..tabs)
                .map(|t| {
                    let n = w * tabs + t;
                    Tab {
                        id: format!("t{n}"),
                        label: format!("task {n}"),
                        number: t as i64 + 1,
                        status: statuses[n % 4],
                        panes: vec![Pane { id: format!("p{n}"), terminal_id: format!("term_{n}"), cwd: "~/work".into(), agent: Some(herdr_core::model::Agent { name: format!("agent-{n}"), kind: "claude".into(), status: statuses[n % 4], session_id: Some(format!("s{n}")), title: format!("task {n}") }), label: None }],
                    }
                })
                .collect(),
        })
        .collect()
}

fn rig(items: Vec<ConversationItem>, earlier: Vec<ConversationItem>, tree: Option<Vec<Workspace>>, screen: Screen) -> Rig {
    let demo = DemoSource::new();
    let workspaces = tree.unwrap_or_else(|| demo.workspaces());
    let (ws, tab) = (workspaces[0].id.clone(), workspaces[0].tabs[0].id.clone());
    let items = Rc::new(RefCell::new(items));
    let rev = Rc::new(RefCell::new(1));
    let src = Synth { workspaces, items: items.clone(), earlier: Rc::new(RefCell::new(earlier)), rev: rev.clone() };
    let mut app = App::new(Box::new(src), Mode::Light, screen);
    app.state.select_tab(&ws, &tab);
    app.state.screen = screen;
    app.refresh();
    Rig { app, items, rev }
}

/// Frame count for a scenario; `--quick` runs a tenth (raster in the dev profile is slow).
fn k(n: usize) -> usize {
    if std::env::args().any(|a| a == "--quick") { (n / 10).max(6) } else { n }
}

// ------------------------------------------------------------------------------------------------ scenarios

fn startup() {
    let t = Instant::now();
    let mut f = Frames::new();
    let fonts = ms(t);
    let t = Instant::now();
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let boot = ms(t);
    let first = f.frame(&mut app, &[]);
    let t = Instant::now();
    f.settle(&mut app);
    let settle = ms(t);
    println!("startup: fonts+renderer {fonts:.1}ms, App::demo {boot:.1}ms, first frame {:.1}ms, 6 more frames {settle:.1}ms", first.total());
    let mut r = rig(session(2000, "m"), Vec::new(), None, Screen::Conversation);
    let first = f.frame(&mut r.app, &[]);
    println!("startup: first frame of a 2000-item conversation {:.1}ms (layout {:.1}, draw {:.1})", first.total(), first.layout, first.draw);
}

/// Every wakeup while nothing happens, on the demo (6 working tabs, so the arcs turn). The arcs ask for a redraw every
/// 60 ms by themselves: the runtime then redraws the same tree, with no message, `view` or layout.
fn idle() {
    let mut f = Frames::new();
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    app.state.toprow.animate = true;
    f.settle(&mut app);
    let (spin, ask) = f.redraws(&app, k(120));
    println!("idle: a working arc asks for {ask:?} after a redraw");
    let tick: Vec<Cost> = (0..k(60)).map(|_| f.tick(&mut app, Message::Tick)).collect();
    let poll: Vec<Cost> = (0..k(30)).map(|_| f.tick(&mut app, Message::Composer(flockbook::composer::ComposerMsg::PollQueue))).collect();
    let empty: Vec<Cost> = (0..k(60)).map(|_| f.empty()).collect();
    report("idle: empty window (baseline)", &empty);
    report("idle: arc redraw (every 60 ms)", &spin);
    report("idle: Tick (every 800 ms)", &tick);
    report("idle: composer poll (every 1600 ms)", &poll);
    // Per second: each wakeup's rate times its mean cost (process CPU time, so GPU driver threads count too).
    let per_s = 1000.0 / 60.0 * stats(&spin).mean.cpu + 1000.0 / 800.0 * stats(&tick).mean.cpu + 1000.0 / 1600.0 * stats(&poll).mean.cpu;
    let base = stats(&empty).mean.cpu;
    let over = |c: &[Cost]| (stats(c).mean.cpu - base).max(0.0);
    let ui_s = 1000.0 / 60.0 * over(&spin) + 1000.0 / 800.0 * over(&tick) + 1000.0 / 1600.0 * over(&poll);
    println!("idle: ~{ui_s:.1} ms of CPU per second over the empty-window baseline ({:.1}% of one core)", ui_s / 10.0);
    println!("idle: ~{:.1} ms of CPU per second ({:.1}% of one core), {:.1} wakeups/s", per_s, per_s / 10.0, 1000.0 / 60.0 + 1000.0 / 800.0 + 1000.0 / 1600.0);
}

fn wheel(dy: f32) -> Vec<Event> {
    vec![
        Event::Mouse(mouse::Event::CursorMoved { position: Point::new(800.0, 420.0) }),
        Event::Mouse(mouse::Event::WheelScrolled { delta: mouse::ScrollDelta::Pixels { x: 0.0, y: dy } }),
    ]
}

fn scroll() {
    let mut f = Frames::new();
    let rss0 = rss_mb();
    let mut r = rig(session(2000, "m"), Vec::new(), None, Screen::Conversation);
    f.settle(&mut r.app);
    let rss1 = rss_mb();
    // Fling up through ~60 screens, then back down: new rows shape on the way up, cached ones on the way down.
    let start = r.app.state.conversation.offset;
    let up: Vec<Cost> = (0..k(300)).map(|i| f.frame(&mut r.app, &wheel(120.0 + (i % 5) as f32 * 30.0))).collect();
    println!("scroll: offset {start:.0} -> {:.0} after the fling up", r.app.state.conversation.offset);
    let down: Vec<Cost> = (0..k(300)).map(|i| f.frame(&mut r.app, &wheel(-120.0 - (i % 5) as f32 * 30.0))).collect();
    let still: Vec<Cost> = (0..k(60)).map(|_| f.frame(&mut r.app, &[])).collect();
    report("scroll 2000 items: up (new rows)", &up);
    report("scroll 2000 items: down (seen rows)", &down);
    report("scroll 2000 items: still frame", &still);
    println!("memory: {rss0:.0} MB before, {rss1:.0} MB with 2000 items laid out, {:.0} MB after scrolling", rss_mb());
}

fn earlier() {
    let mut f = Frames::new();
    let mut r = rig(session(400, "m"), session(1600, "old"), None, Screen::Conversation);
    f.settle(&mut r.app);
    let t = Instant::now();
    let _ = r.app.update(Message::Conversation(flockbook::conversation::ConversationMsg::Flow(fb_ui::selectable::flow::FlowEvent::LoadEarlier)));
    let up = ms(t);
    let mut frames = vec![];
    for _ in 0..6 {
        frames.push(f.frame(&mut r.app, &[]));
    }
    println!("load earlier (+1600 items onto 400): update {up:.1}ms, next frame {:.1}ms, 6 frames {:.1}ms, rows now {}", frames[0].total(), frames.iter().map(Cost::total).sum::<f64>(), r.app.state.conversation.doc.rows.len());
}

/// The source changes several times a second: one new item each time (an agent streaming), pinned to the bottom.
fn updates() {
    let mut f = Frames::new();
    let mut r = rig(session(2000, "m"), Vec::new(), None, Screen::Conversation);
    f.settle(&mut r.app);
    let mut costs = vec![];
    let mut refresh = vec![];
    let rounds = std::env::var("FB_PERF_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(k(100));
    for i in 0..rounds {
        r.items.borrow_mut().push(item(format!("n{i}"), ItemKind::AssistantText(format!("Streaming line {i}: still **working** on `step {i}`."))));
        *r.rev.borrow_mut() += 1;
        let t = Instant::now();
        r.app.refresh();
        refresh.push(ms(t));
        costs.push(f.frame(&mut r.app, &[]));
    }
    refresh.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("updates: refresh() p50 {:.2}ms p95 {:.2}ms", refresh[refresh.len() / 2], refresh[(refresh.len() * 95 / 100).min(refresh.len() - 1)]);
    report("updates: frame after a new item", &costs);
    let same: Vec<Cost> = (0..k(60)).map(|_| f.tick(&mut r.app, Message::Tick)).collect();
    report("updates: Tick with no change", &same);
}

fn overview() {
    let mut f = Frames::new();
    let mut r = rig(session(40, "m"), Vec::new(), Some(big_tree(20, 10)), Screen::Overview);
    f.settle(&mut r.app);
    r.app.state.toprow.animate = true;
    let (still, _) = f.redraws(&r.app, k(60));
    report("overview 200 tabs: arc redraw", &still);
    let scroll: Vec<Cost> = (0..k(120)).map(|i| f.frame(&mut r.app, &wheel(if i < k(60) { -120.0 } else { 120.0 }))).collect();
    report("overview 200 tabs: scroll frame", &scroll);
    r.app.state.screen = Screen::Conversation;
    f.settle(&mut r.app);
    let (side, _) = f.redraws(&r.app, k(60));
    report("200 tabs in sidebar: arc redraw", &side);
}

fn terminal() {
    use flockbook::terminal::{FakeTerminals, TerminalMsg};
    let mut f = Frames::new();
    let mut app = App::demo(Mode::Light, Screen::Conversation);
    let fake = FakeTerminals::default();
    app.state.terminal.backend = Some(std::sync::Arc::new(fake.clone()));
    let _ = app.update(Message::Terminal(TerminalMsg::Toggle));
    fake.push_frame(true, 160, 48, b"$ ");
    f.settle(&mut app);
    // `yes`-like output: 40 lines of 150 colored columns per poll, 30 polls a second.
    let mut chunk = Vec::new();
    for i in 0..40 {
        chunk.extend_from_slice(format!("\x1b[3{}mline {i:04} ", i % 8).as_bytes());
        chunk.extend_from_slice(&[b'x'; 140]);
        chunk.extend_from_slice(b"\x1b[0m\r\n");
    }
    let costs: Vec<Cost> = (0..k(150))
        .map(|_| {
            fake.push_frame(false, 160, 48, &chunk);
            f.tick(&mut app, Message::Terminal(TerminalMsg::Poll))
        })
        .collect();
    report("terminal: poll with 6 KB of output", &costs);
    let quiet: Vec<Cost> = (0..k(60)).map(|_| f.tick(&mut app, Message::Terminal(TerminalMsg::Poll))).collect();
    report("terminal: poll with no output", &quiet);
}

fn main() {
    let which: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--")).collect();
    let all = ["startup", "idle", "scroll", "earlier", "updates", "overview", "terminal"];
    let run: Vec<&str> = if which.is_empty() { all.to_vec() } else { which.iter().map(String::as_str).collect() };
    let flags: Vec<String> = std::env::args().filter(|a| a.starts_with("--")).collect();
    println!("profile: {} {flags:?}", if cfg!(debug_assertions) { "dev (debug)" } else { "release" });
    for s in run {
        match s {
            "startup" => startup(),
            "idle" => idle(),
            "scroll" => scroll(),
            "earlier" => earlier(),
            "updates" => updates(),
            "overview" => overview(),
            "terminal" => terminal(),
            other => eprintln!("unknown scenario {other}; one of {all:?}"),
        }
    }
}
