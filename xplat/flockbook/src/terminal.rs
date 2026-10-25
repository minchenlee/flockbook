//! The live terminal sheet (port of TerminalSheet.swift and the terminal part of AppModel.swift).
//!
//! "Show terminal" (top row button, Ctrl+`) opens a sheet over the conversation for the selected pane. The sheet draws
//! herdr frames through `fb_ui::term`. Everything that talks to herdr goes through [`TerminalBackend`] and
//! [`TerminalLink`]:
//!
//! - The real backend, [`LiveBackend`], runs `herdr terminal session control|observe` (herdr-core `TerminalSession`).
//!   It is installed only on the live windowed path in `main.rs` ([`install_live`]). A control session WRITES to a real
//!   terminal. Live write tests target the `c9-input-test` session only.
//! - Tests and the demo use [`FakeTerminals`]: canned frames, every call recorded, no process, no socket.
//!
//! Rules this file keeps (each has a test in `tests/terminal.rs`):
//! - Read-only (`access.rs`) opens an Observe session. The widget then has no input callback at all, and `update`
//!   drops any input message as well.
//! - Keys reach the link only while the terminal has focus, the session is Control, writes are on, and the live view
//!   is shown (not the history view).
//! - Resize goes to the link 150 ms after the last size change, in Control mode only.
//! - The wheel never becomes keys. A full-screen program in Control mode gets `TerminalLink::wheel` (herdr forwards
//!   it). History opens a read-only view. Observe, or a program without history, shows a notice.
//! - Closing the sheet, or leaving the screen or the pane, releases the session. The sheet never takes over by itself:
//!   "Take over" is a button.

use crate::state::Screen;
use crate::{style, App, Message};
use fb_ui::icons::{icon, Icon, SIZE_DEFAULT, SIZE_INLINE};
use fb_ui::term::{GridSnapshot, TermGrid, TermPalette, Terminal, CELL_H, PAD};
use fb_ui::theme::*;
use herdr_core::scrollback::{ScrollInfo, ScrollKind, Scrollback};
use herdr_core::terminal::{TerminalEvent, TerminalFrame, TerminalMode};
use iced::widget::{button, column, container, mouse_area, responsive, row, scrollable, stack, Space};
use iced::{keyboard, time, Alignment, Background, Border, Element, Length, Subscription, Task};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Height of the sheet's title row: the 28-pt close button plus the 12-pt padding above and below it
/// (Swift `TerminalSheet`: `HStack { ... }.padding(12)` around a 28x28 close button).
pub const HEADER_H: f32 = 52.0;
/// How long a pty resize waits for the next size change (Swift: 150 ms).
pub const RESIZE_DELAY: Duration = Duration::from_millis(150);
/// How long a scroll-state answer is trusted before the wheel asks herdr again.
const KIND_TTL: Duration = Duration::from_millis(1500);
const NOTICE_TTL: Duration = Duration::from_secs(6);
/// Poll rate of the frame queue while the sheet is open.
const POLL_EVERY: Duration = Duration::from_millis(33);

pub const NO_HISTORY: &str = "This program has no history to scroll. Read the conversation on the conversation screen.";

/// Size of the sheet (port of `TerminalSheetSize`): 600x360 up to the window minus a margin.
pub struct SheetSize;

impl SheetSize {
    pub const MIN: (f32, f32) = (600.0, 360.0);
    pub const STANDARD: (f32, f32) = (880.0, 560.0);

    /// Clamps to [MIN, window minus a margin]. An unknown window (zero) leaves the maximum open.
    pub fn clamp(size: (f32, f32), window: (f32, f32)) -> (f32, f32) {
        let max_w = if window.0 > 0.0 { Self::MIN.0.max(window.0 - 40.0) } else { f32::INFINITY };
        let max_h = if window.1 > 0.0 { Self::MIN.1.max(window.1 - 60.0) } else { f32::INFINITY };
        (size.0.max(Self::MIN.0).min(max_w), size.1.max(Self::MIN.1).min(max_h))
    }
}

/// Why a session ended, in words for the owner (port of `ObservedTerminal.explain`).
pub fn explain(reason: &str) -> String {
    if reason.contains("taken over") {
        "Another client took over this terminal.".into()
    } else if reason.contains("already has an attached client") {
        "Another client is controlling this terminal.".into()
    } else if reason.contains("not found") {
        "This terminal no longer exists.".into()
    } else {
        reason.to_string()
    }
}

// ---------------------------------------------------------------- backend

/// One open connection to a terminal.
pub trait TerminalLink: Send {
    /// Everything that arrived since the last call.
    fn poll(&mut self) -> Vec<TerminalEvent>;
    /// MUTATING (Control only; an Observe link returns false and sends nothing).
    fn send_bytes(&self, bytes: &[u8]) -> bool;
    /// MUTATING. Bracketed per the session.
    fn paste(&self, text: &str) -> bool;
    /// MUTATING (it resizes the pty).
    fn resize(&self, cols: usize, rows: usize);
    /// One wheel event per line, forwarded by herdr to the program. Never keys.
    fn wheel(&self, lines: usize, up: bool) -> bool;
    /// Gives the terminal back to herdr and ends the session.
    fn release(&self);
}

/// Opens terminals and answers the read-only scroll questions. Implementations may block (the live one calls herdr).
pub trait TerminalBackend: Send + Sync {
    fn open(&self, terminal_id: &str, mode: TerminalMode, cols: usize, rows: usize) -> Box<dyn TerminalLink>;
    /// Read-only.
    fn scroll_info(&self, pane_id: &str) -> Result<ScrollInfo, String>;
    /// Read-only: the last 1000 lines with colors.
    fn read_history(&self, pane_id: &str) -> Result<Scrollback, String>;
    /// True when the answers are instant and local (fakes, demo): the app asks inline instead of on a thread.
    fn instant(&self) -> bool {
        false
    }
}

/// The real thing. A Control session WRITES to a real terminal: never used in tests.
#[cfg(unix)]
pub struct LiveBackend {
    session: herdr_core::HerdrSession,
    client: herdr_core::client::HerdrClient<herdr_core::transport::UnixConnector>,
}

#[cfg(unix)]
impl LiveBackend {
    pub fn new(session: &herdr_core::HerdrSession) -> LiveBackend {
        LiveBackend { session: session.clone(), client: herdr_core::client::HerdrClient::new(session) }
    }
}

#[cfg(unix)]
struct LiveLink {
    session: herdr_core::terminal::TerminalSession,
    rx: std::sync::mpsc::Receiver<TerminalEvent>,
}

#[cfg(unix)]
impl TerminalLink for LiveLink {
    fn poll(&mut self) -> Vec<TerminalEvent> {
        self.rx.try_iter().collect()
    }
    fn send_bytes(&self, bytes: &[u8]) -> bool {
        self.session.send_bytes(bytes)
    }
    fn paste(&self, text: &str) -> bool {
        self.session.paste(text, None)
    }
    fn resize(&self, cols: usize, rows: usize) {
        self.session.resize(cols, rows)
    }
    fn wheel(&self, lines: usize, up: bool) -> bool {
        self.session.wheel(lines, up, true)
    }
    fn release(&self) {
        self.session.release()
    }
}

#[cfg(unix)]
impl TerminalBackend for LiveBackend {
    fn open(&self, terminal_id: &str, mode: TerminalMode, cols: usize, rows: usize) -> Box<dyn TerminalLink> {
        let (session, rx) = herdr_core::terminal::TerminalSession::new(terminal_id, &self.session, mode, Some(cols), Some(rows), None);
        session.start();
        Box::new(LiveLink { session, rx })
    }
    fn scroll_info(&self, pane_id: &str) -> Result<ScrollInfo, String> {
        self.client.scroll_info(pane_id).map_err(|e| e.to_string())
    }
    fn read_history(&self, pane_id: &str) -> Result<Scrollback, String> {
        self.client.read_scrollback(pane_id, Scrollback::READ_LIMIT, true, false).map_err(|e| e.to_string())
    }
}

/// What a fake link or backend was asked. Tests read it; the demo ignores it.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    Open { terminal_id: String, mode: TerminalMode, cols: usize, rows: usize },
    Bytes(Vec<u8>),
    Paste(String),
    Resize(usize, usize),
    Wheel { lines: usize, up: bool },
    Release,
    ScrollInfo(String),
    ReadHistory(String),
}

/// Canned terminals: no process, no socket. Control calls are recorded and refused in Observe mode, like the real
/// session. Events pushed with [`FakeTerminals::push`] reach every open link.
#[derive(Clone)]
pub struct FakeTerminals {
    pub calls: Arc<Mutex<Vec<Call>>>,
    queue: Arc<Mutex<VecDeque<TerminalEvent>>>,
    pub info: Arc<Mutex<ScrollInfo>>,
    pub history: Arc<Mutex<String>>,
    /// Every `open` is answered with the demo shell screen.
    demo: bool,
}

impl Default for FakeTerminals {
    fn default() -> Self {
        FakeTerminals {
            calls: Default::default(),
            queue: Default::default(),
            info: Arc::new(Mutex::new(ScrollInfo { max_offset_from_bottom: 0, offset_from_bottom: 0, viewport_rows: 24, foreground_name: Some("zsh".into()), shell_is_foreground: Some(true) })),
            history: Default::default(),
            demo: false,
        }
    }
}

impl FakeTerminals {
    pub fn push(&self, e: TerminalEvent) {
        self.queue.lock().unwrap().push_back(e);
    }

    pub fn push_frame(&self, full: bool, width: u32, height: u32, bytes: &[u8]) {
        let seq = self.queue.lock().unwrap().len() as i64;
        self.push(TerminalEvent::Frame(TerminalFrame { seq, full, width, height, bytes: bytes.to_vec() }));
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    /// Calls that write to the terminal: bytes, paste, resize, wheel.
    pub fn writes(&self) -> Vec<Call> {
        self.calls().into_iter().filter(|c| matches!(c, Call::Bytes(_) | Call::Paste(_) | Call::Resize(..) | Call::Wheel { .. })).collect()
    }

    pub fn set_info(&self, info: ScrollInfo) {
        *self.info.lock().unwrap() = info;
    }

    /// The demo: every open session shows one shell screen, sized to what the sheet asks for.
    pub fn demo() -> FakeTerminals {
        FakeTerminals { demo: true, ..FakeTerminals::default() }
    }
}

struct FakeLink {
    mode: TerminalMode,
    fake: FakeTerminals,
}

impl FakeLink {
    fn control(&self) -> bool {
        self.mode != TerminalMode::Observe
    }
    fn log(&self, c: Call) {
        self.fake.calls.lock().unwrap().push(c);
    }
}

impl TerminalLink for FakeLink {
    fn poll(&mut self) -> Vec<TerminalEvent> {
        self.fake.queue.lock().unwrap().drain(..).collect()
    }
    fn send_bytes(&self, bytes: &[u8]) -> bool {
        if !self.control() {
            return false;
        }
        self.log(Call::Bytes(bytes.to_vec()));
        true
    }
    fn paste(&self, text: &str) -> bool {
        if !self.control() {
            return false;
        }
        self.log(Call::Paste(text.to_string()));
        true
    }
    fn resize(&self, cols: usize, rows: usize) {
        if self.control() {
            self.log(Call::Resize(cols, rows));
        }
    }
    fn wheel(&self, lines: usize, up: bool) -> bool {
        if !self.control() || lines == 0 {
            return false;
        }
        self.log(Call::Wheel { lines, up });
        true
    }
    fn release(&self) {
        self.log(Call::Release);
    }
}

impl TerminalBackend for FakeTerminals {
    fn open(&self, terminal_id: &str, mode: TerminalMode, cols: usize, rows: usize) -> Box<dyn TerminalLink> {
        self.calls.lock().unwrap().push(Call::Open { terminal_id: terminal_id.to_string(), mode, cols, rows });
        if self.demo {
            self.push_frame(true, cols as u32, rows as u32, &demo_screen_bytes());
        }
        Box::new(FakeLink { mode, fake: self.clone() })
    }
    fn scroll_info(&self, pane_id: &str) -> Result<ScrollInfo, String> {
        self.calls.lock().unwrap().push(Call::ScrollInfo(pane_id.to_string()));
        Ok(self.info.lock().unwrap().clone())
    }
    fn read_history(&self, pane_id: &str) -> Result<Scrollback, String> {
        self.calls.lock().unwrap().push(Call::ReadHistory(pane_id.to_string()));
        let text = self.history.lock().unwrap().clone();
        let line_count = text.lines().count();
        Ok(Scrollback { text, truncated: false, revision: 1, line_count })
    }
    fn instant(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------- state

pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

/// Trailing-edge debounce of the pty size. Pure: the caller passes the time.
#[derive(Debug, Clone, Default)]
pub struct ResizeDebounce {
    pending: Option<((usize, usize), Instant)>,
    sent: Option<(usize, usize)>,
}

impl ResizeDebounce {
    /// `sent` is the size the session was opened with, so the widget's first report is not a change.
    pub fn new(sent: (usize, usize)) -> ResizeDebounce {
        ResizeDebounce { pending: None, sent: Some(sent) }
    }

    /// A size change. Each one restarts the wait.
    pub fn request(&mut self, size: (usize, usize), now: Instant) {
        if self.pending.is_none() && self.sent == Some(size) {
            return;
        }
        self.pending = Some((size, now + RESIZE_DELAY));
    }

    /// The size to send now, once the wait is over and it differs from the last sent.
    pub fn due(&mut self, now: Instant) -> Option<(usize, usize)> {
        let (size, at) = self.pending?;
        if now < at {
            return None;
        }
        self.pending = None;
        if self.sent == Some(size) {
            return None;
        }
        self.sent = Some(size);
        Some(size)
    }
}

/// Wheel bookkeeping (port of `WheelState`).
#[derive(Default)]
struct Wheel {
    accum: f32,
    kind: Option<(ScrollKind, Instant)>,
    fetching: bool,
}

/// The read-only history view: the last lines, with colors, in a grid of their own. It has no link to the session.
pub struct History {
    pub snap: GridSnapshot,
    pub lines: usize,
    /// The end of the history is in view: a wheel step toward the live end returns to live.
    pub at_bottom: bool,
}

pub struct Sheet {
    pub terminal_id: String,
    pub pane_id: String,
    pub title: String,
    pub mode: TerminalMode,
    /// The screen the sheet opened on: leaving it closes the sheet.
    pub screen: Screen,
    link: Option<Box<dyn TerminalLink>>,
    grid: TermGrid,
    pub snap: GridSnapshot,
    pub app_cursor: bool,
    pub focused: bool,
    /// Why the session ended, if it did.
    pub closed: Option<String>,
    pub history: Option<History>,
    pub notice: Option<(String, Instant)>,
    resize: ResizeDebounce,
    wheel: Wheel,
    /// Answers of an older request are dropped.
    epoch: u64,
}

impl Sheet {
    pub fn is_control(&self) -> bool {
        self.mode != TerminalMode::Observe
    }
}

pub struct TerminalState {
    pub backend: Option<Arc<dyn TerminalBackend>>,
    /// Swift `writesEnabled`. Off: sessions open in Observe mode.
    pub writable: bool,
    pub sheet: Option<Sheet>,
    /// Size the owner chose, before the clamp. Not saved (no settings store yet).
    pub size: (f32, f32),
    pub clock: Clock,
    epoch: u64,
}

impl Default for TerminalState {
    fn default() -> Self {
        TerminalState { backend: None, writable: true, sheet: None, size: SheetSize::STANDARD, clock: Arc::new(Instant::now), epoch: 0 }
    }
}

impl TerminalState {
    fn now(&self) -> Instant {
        (self.clock)()
    }

    pub fn is_open(&self) -> bool {
        self.sheet.is_some()
    }
}

// ---------------------------------------------------------------- messages

#[derive(Debug, Clone)]
pub enum TerminalMsg {
    Toggle,
    Close,
    /// Esc outside the program: leaves the history view first, then closes the sheet.
    Escape,
    /// Drains the frame queue and runs the resize debounce. Fired by the subscription.
    Poll,
    /// Bytes from the keyboard or the input method.
    Input(Vec<u8>),
    Paste(String),
    /// The widget's grid size changed (cols, rows).
    Resized(usize, usize),
    /// Wheel over the live view: `dy` > 0 is toward older content.
    Wheel { dy: f32, precise: bool },
    Focus(bool),
    BackToLive,
    /// Text selected in the sheet, for the clipboard. Local only: nothing goes to the pane.
    Copy(String),
    /// The history list scrolled: is its end in view?
    HistoryScrolled { at_bottom: bool },
    /// Wheel over the history view (`dy` < 0 scrolls toward the live end). At the bottom it returns to live.
    HistoryWheel { dy: f32 },
    TakeOver,
    SetSize(f32, f32),
    /// The scroll state asked for by a wheel step.
    ScrollInfoReady { epoch: u64, info: Result<ScrollInfo, String>, dy: f32, precise: bool },
    HistoryReady { epoch: u64, result: Result<(ScrollInfo, Scrollback), String> },
}

// ---------------------------------------------------------------- update

/// Opens the sheet for the selected pane. Does nothing without a backend or a pane, or when it is open.
pub fn open(app: &mut App) {
    open_with(app, false);
}

fn open_with(app: &mut App, takeover: bool) {
    // Swift shows the sheet on the tab (conversation) screen only: the other screens have no pane in front.
    if app.state.screen != Screen::Conversation {
        return;
    }
    let Some(backend) = app.state.terminal.backend.clone() else { return };
    let Some(pane) = app.state.selected_pane().cloned() else { return };
    if app.state.terminal.sheet.is_some() && !takeover {
        return;
    }
    let title = app.state.selected_tab().map(|t| t.label.clone()).unwrap_or_default();
    // The window size is unknown here: the widget reports the real grid size on its first draw.
    let size = SheetSize::clamp(app.state.terminal.size, (0.0, 0.0));
    let (cols, rows) = fb_ui::term::widget::grid_size(size.0, size.1 - HEADER_H);
    let writable = app.state.terminal.writable;
    let mode = if writable { TerminalMode::Control { takeover } } else { TerminalMode::Observe };
    let link = backend.open(&pane.terminal_id, mode, cols, rows);
    app.state.terminal.epoch += 1;
    let epoch = app.state.terminal.epoch;
    let grid = TermGrid::new(cols, rows);
    let snap = grid.snapshot();
    app.state.terminal.sheet = Some(Sheet {
        terminal_id: pane.terminal_id.clone(),
        pane_id: pane.id.clone(),
        title,
        mode,
        screen: app.state.screen,
        link: Some(link),
        grid,
        snap,
        app_cursor: false,
        focused: writable,
        closed: None,
        history: None,
        notice: None,
        resize: ResizeDebounce::new((cols, rows)),
        wheel: Wheel::default(),
        epoch,
    });
    app.state.toprow.terminal_on = true;
}

/// Closes the sheet and gives the terminal back (`release`). Safe to call when nothing is open.
pub fn close(app: &mut App) {
    if let Some(mut sheet) = app.state.terminal.sheet.take() {
        if let Some(link) = sheet.link.take() {
            link.release();
        }
    }
    app.state.toprow.terminal_on = false;
}

pub fn toggle(app: &mut App) {
    if app.state.terminal.sheet.is_some() {
        close(app);
    } else {
        open(app);
    }
}

/// Input for the widgets below the sheet (composer editing and submit, card keys and buttons). While the sheet is
/// open `App::update` drops these: a click on the sheet header or the scrim leaves the terminal unfocused, and typing
/// must not reach a composer that still has the keyboard. Results of earlier sends (`Sent`, `Done`) still pass.
pub fn blocks_underlying(m: &Message) -> bool {
    use crate::cards::CardsMsg;
    use crate::composer::ComposerMsg;
    matches!(
        m,
        Message::Composer(ComposerMsg::Edit(_) | ComposerMsg::Submit)
            | Message::Cards(CardsMsg::Key(_) | CardsMsg::Allow | CardsMsg::Deny | CardsMsg::Always(_) | CardsMsg::Pick(_) | CardsMsg::ActivateOther | CardsMsg::OtherInput(_) | CardsMsg::Submit | CardsMsg::Back)
    )
}

/// Runs after every `App::update`: the sheet follows the screen and the pane. Leaving either releases the terminal.
pub fn after_update(app: &mut App) {
    let Some(sheet) = &app.state.terminal.sheet else { return };
    let same_pane = app.state.selected_pane().is_some_and(|p| p.terminal_id == sheet.terminal_id);
    if app.state.screen != sheet.screen || !same_pane {
        close(app);
    }
}

fn blocking<T: Send + 'static>(job: impl FnOnce() -> T + Send + 'static, map: impl Fn(T) -> Message + Send + 'static) -> Task<Message> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job());
    });
    Task::perform(async move { rx.await.ok() }, move |v| match v {
        Some(v) => map(v),
        None => Message::Noop,
    })
}

fn set_notice(app: &mut App, text: &str) {
    let until = app.state.terminal.now() + NOTICE_TTL;
    if let Some(s) = app.state.terminal.sheet.as_mut() {
        if s.notice.is_none() {
            s.notice = Some((text.to_string(), until));
        }
    }
}

pub fn update(app: &mut App, msg: TerminalMsg) -> Task<Message> {
    match msg {
        TerminalMsg::Toggle => {
            toggle(app);
            Task::none()
        }
        TerminalMsg::Close => {
            close(app);
            Task::none()
        }
        TerminalMsg::Escape => {
            match app.state.terminal.sheet.as_mut() {
                Some(s) if s.history.is_some() => s.history = None,
                Some(_) => close(app),
                None => {}
            }
            Task::none()
        }
        TerminalMsg::Poll => {
            poll(app);
            Task::none()
        }
        TerminalMsg::Input(bytes) => {
            if let Some(link) = live_control_link(app) {
                link.send_bytes(&bytes);
            }
            Task::none()
        }
        TerminalMsg::Paste(text) => {
            if let Some(link) = live_control_link(app) {
                link.paste(&text);
            }
            Task::none()
        }
        TerminalMsg::Resized(cols, rows) => {
            let now = app.state.terminal.now();
            if let Some(s) = app.state.terminal.sheet.as_mut() {
                // Observe never resizes the pty.
                if s.is_control() && s.closed.is_none() {
                    s.resize.request((cols, rows), now);
                }
            }
            Task::none()
        }
        TerminalMsg::Wheel { dy, precise } => wheel(app, dy, precise),
        TerminalMsg::Focus(on) => {
            if let Some(s) = app.state.terminal.sheet.as_mut() {
                s.focused = on && s.is_control();
            }
            Task::none()
        }
        TerminalMsg::BackToLive => {
            if let Some(s) = app.state.terminal.sheet.as_mut() {
                s.history = None;
            }
            Task::none()
        }
        // The selected text goes to the clipboard. It never goes to the link.
        TerminalMsg::Copy(text) => iced::clipboard::write(text),
        TerminalMsg::HistoryScrolled { at_bottom } => {
            if let Some(h) = app.state.terminal.sheet.as_mut().and_then(|s| s.history.as_mut()) {
                h.at_bottom = at_bottom;
            }
            Task::none()
        }
        TerminalMsg::HistoryWheel { dy } => {
            // A wheel step toward the live end, with the end already in view: back to live (Swift onBottomScroll).
            if let Some(s) = app.state.terminal.sheet.as_mut() {
                if dy < 0.0 && s.history.as_ref().is_some_and(|h| h.at_bottom) {
                    s.history = None;
                }
            }
            Task::none()
        }
        TerminalMsg::TakeOver => {
            // An explicit click of the owner: a new control session that takes the terminal. Control only.
            let ok = app.state.terminal.writable && app.state.terminal.sheet.as_ref().is_some_and(|s| s.is_control() && s.closed.is_some());
            if ok {
                close(app);
                open_with(app, true);
            }
            Task::none()
        }
        TerminalMsg::SetSize(w, h) => {
            app.state.terminal.size = SheetSize::clamp((w, h), (0.0, 0.0));
            Task::none()
        }
        TerminalMsg::ScrollInfoReady { epoch, info, dy, precise } => {
            let now = app.state.terminal.now();
            let Some(s) = app.state.terminal.sheet.as_mut().filter(|s| s.epoch == epoch) else { return Task::none() };
            s.wheel.fetching = false;
            let Ok(info) = info else { return Task::none() };
            let kind = info.kind();
            s.wheel.kind = Some((kind, now));
            act(app, kind, dy, precise)
        }
        TerminalMsg::HistoryReady { epoch, result } => {
            let Some(s) = app.state.terminal.sheet.as_mut().filter(|s| s.epoch == epoch) else { return Task::none() };
            s.wheel.fetching = false;
            let Ok((info, sb)) = result else { return Task::none() };
            if info.kind() == ScrollKind::None {
                return Task::none();
            }
            if info.kind() == ScrollKind::FullScreenProgram && sb.line_count as i64 <= info.viewport_rows {
                set_notice(app, NO_HISTORY);
                return Task::none();
            }
            let cols = s.snap.cols;
            s.history = Some(history_view(&sb.text, cols, sb.line_count));
            Task::none()
        }
    }
}

/// The link, only when keys may go to it: Control session, writes on, live view shown, focused, still connected.
fn live_control_link(app: &App) -> Option<&dyn TerminalLink> {
    let st = &app.state.terminal;
    let s = st.sheet.as_ref()?;
    if !st.writable || !s.is_control() || !s.focused || s.history.is_some() || s.closed.is_some() {
        return None;
    }
    s.link.as_deref()
}

fn poll(app: &mut App) {
    let now = app.state.terminal.now();
    let Some(s) = app.state.terminal.sheet.as_mut() else { return };
    let mut dirty = false;
    if let Some(link) = s.link.as_mut() {
        for e in link.poll() {
            match e {
                TerminalEvent::Frame(f) => {
                    s.grid.feed(f.full, f.width, f.height, &f.bytes);
                    dirty = true;
                }
                TerminalEvent::Closed { reason } => s.closed = Some(reason),
                TerminalEvent::Reconnecting { .. } | TerminalEvent::Notice(_) => {}
            }
        }
    }
    // vte holds every byte of a synchronized update (BSU) until its end. A program that never sends the end must not
    // freeze the view: after the timeout the update is applied anyway.
    if s.grid.flush_sync(now) {
        dirty = true;
    }
    if dirty {
        s.snap = s.grid.snapshot();
        s.app_cursor = s.grid.app_cursor();
    }
    if s.closed.is_some() {
        // The stream ended: nothing to release, nothing to resize.
        s.link = None;
    }
    if s.is_control() {
        if let (Some(size), Some(link)) = (s.resize.due(now), s.link.as_ref()) {
            link.resize(size.0, size.1);
        }
    }
    if s.notice.as_ref().is_some_and(|(_, until)| now >= *until) {
        s.notice = None;
    }
}

/// Wheel over the live view (port of `ObservedTerminal.wheel`). Never sends keys.
fn wheel(app: &mut App, dy: f32, precise: bool) -> Task<Message> {
    let now = app.state.terminal.now();
    let backend = app.state.terminal.backend.clone();
    let Some(s) = app.state.terminal.sheet.as_mut() else { return Task::none() };
    if s.history.is_some() {
        return Task::none();
    }
    if let Some((kind, at)) = s.wheel.kind {
        if now.saturating_duration_since(at) < KIND_TTL {
            return act(app, kind, dy, precise);
        }
    }
    if s.wheel.fetching {
        return Task::none();
    }
    let Some(backend) = backend else { return Task::none() };
    s.wheel.fetching = true;
    let (pane, epoch) = (s.pane_id.clone(), s.epoch);
    if backend.instant() {
        let info = backend.scroll_info(&pane);
        return update(app, TerminalMsg::ScrollInfoReady { epoch, info, dy, precise });
    }
    blocking(move || backend.scroll_info(&pane), move |info| Message::Terminal(TerminalMsg::ScrollInfoReady { epoch, info, dy, precise }))
}

fn act(app: &mut App, kind: ScrollKind, dy: f32, precise: bool) -> Task<Message> {
    let writable = app.state.terminal.writable;
    match kind {
        ScrollKind::None => Task::none(),
        ScrollKind::History => {
            if dy > 0.0 {
                request_history(app)
            } else {
                Task::none()
            }
        }
        ScrollKind::FullScreenProgram => {
            let Some(s) = app.state.terminal.sheet.as_mut() else { return Task::none() };
            if writable && s.is_control() && s.closed.is_none() {
                // Trackpad: one step per ~28 pt of movement (at most 4 per event). A wheel notch is one step.
                s.wheel.accum += dy.abs();
                let unit = if precise { 28.0 } else { 1.0 };
                let n = ((s.wheel.accum / unit) as usize).min(4);
                if n == 0 {
                    return Task::none();
                }
                s.wheel.accum -= n as f32 * unit;
                if let Some(link) = s.link.as_ref() {
                    link.wheel(n, dy > 0.0);
                }
                Task::none()
            } else if dy > 0.0 {
                request_history(app)
            } else {
                Task::none()
            }
        }
    }
}

fn request_history(app: &mut App) -> Task<Message> {
    let Some(backend) = app.state.terminal.backend.clone() else { return Task::none() };
    let Some(s) = app.state.terminal.sheet.as_mut() else { return Task::none() };
    if s.wheel.fetching || s.history.is_some() {
        return Task::none();
    }
    s.wheel.fetching = true;
    let (pane, epoch) = (s.pane_id.clone(), s.epoch);
    let job = move || -> Result<(ScrollInfo, Scrollback), String> {
        let info = backend.scroll_info(&pane)?;
        let sb = backend.read_history(&pane)?;
        Ok((info, sb))
    };
    let is_instant = app.state.terminal.backend.as_ref().is_some_and(|b| b.instant());
    if is_instant {
        let result = job();
        return update(app, TerminalMsg::HistoryReady { epoch, result });
    }
    blocking(job, move |result| Message::Terminal(TerminalMsg::HistoryReady { epoch, result }))
}

/// True when the history list shows its end (a list shorter than the view always does).
pub fn at_bottom(offset_y: f32, content_h: f32, view_h: f32) -> bool {
    content_h <= view_h + 0.5 || offset_y >= content_h - view_h - 1.0
}

/// The most rows the history grid keeps, screen included. herdr returns at most 1000 lines, but one long line (minified
/// JSON, a log blob) wraps into many rows. The newest rows stay; the oldest scroll out.
pub const HISTORY_MAX_ROWS: usize = 20_000;
const HISTORY_SCREEN_ROWS: usize = 24;

/// Builds the history grid. The text is fed into a small screen with scrollback, so every row is kept whatever its
/// width (wide characters, tabs and escape sequences make a row count guess unsafe), up to [`HISTORY_MAX_ROWS`].
/// Blank rows at the end are trimmed.
pub fn history_view(text: &str, cols: usize, lines: usize) -> History {
    let norm = text.replace("\r\n", "\n");
    let body = norm.trim_end_matches('\n');
    let cols = cols.clamp(1, fb_ui::term::grid::MAX_COLS);
    let mut grid = TermGrid::with_history(cols, HISTORY_SCREEN_ROWS, HISTORY_MAX_ROWS - HISTORY_SCREEN_ROWS);
    grid.feed(true, cols as u32, HISTORY_SCREEN_ROWS as u32, body.replace('\n', "\r\n").as_bytes());
    // No more bytes come: a synchronized update the text left open would hide everything, so end it now.
    grid.flush_sync(std::time::Instant::now() + std::time::Duration::from_secs(3600));
    History { snap: grid.snapshot_all(), lines, at_bottom: true }
}

// ---------------------------------------------------------------- wiring

/// Real terminals for the live windowed path. A control session WRITES to a real terminal.
#[cfg(unix)]
pub fn install_live(app: &mut App, session: &herdr_core::HerdrSession) {
    app.state.terminal.backend = Some(Arc::new(LiveBackend::new(session)));
}

/// Fake terminals for demo and snapshots: no process, no socket.
pub fn install_demo(app: &mut App) -> FakeTerminals {
    let fake = FakeTerminals::demo();
    app.state.terminal.backend = Some(Arc::new(fake.clone()));
    fake
}

/// Frames for the demo screen: a shell session that shows colors, attributes, CJK and a prompt.
pub fn demo_screen_bytes() -> Vec<u8> {
    let mut s = String::new();
    s.push_str("\x1b[2J\x1b[H");
    s.push_str("\x1b[1;32mquarry\x1b[0m \x1b[34m~/ranking\x1b[0m \x1b[2m(main)\x1b[0m $ pytest tests/ranking -q\r\n");
    s.push_str("\x1b[32m..............\x1b[0m                                              [100%]\r\n");
    s.push_str("\x1b[1m14 passed\x1b[0m in 0.82s\r\n\r\n");
    s.push_str("\x1b[1;32mquarry\x1b[0m \x1b[34m~/ranking\x1b[0m \x1b[2m(main)\x1b[0m $ ls -la\r\n");
    s.push_str("drwxr-xr-x  6 dev  staff   192 Oct 10 21:10 \x1b[1;34m.\x1b[0m\r\n");
    s.push_str("-rw-r--r--  1 dev  staff  2814 Oct 10 21:08 score.py\r\n");
    s.push_str("-rwxr-xr-x  1 dev  staff   311 Oct  9 17:42 \x1b[1;32mreplay.sh\x1b[0m\r\n");
    s.push_str("lrwxr-xr-x  1 dev  staff    12 Oct  9 17:40 \x1b[36mlatest\x1b[0m -> logs/latest\r\n\r\n");
    s.push_str("\x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \x1b[9mstrike\x1b[0m \x1b[7m inverse \x1b[0m \x1b[2mdim\x1b[0m\r\n");
    s.push_str("\x1b[31m red \x1b[32m green \x1b[33m yellow \x1b[34m blue \x1b[35m magenta \x1b[36m cyan \x1b[0m \x1b[91m bright red \x1b[94m bright blue \x1b[0m\r\n");
    s.push_str("\x1b[48;5;196m  \x1b[48;5;208m  \x1b[48;5;226m  \x1b[48;5;46m  \x1b[48;5;51m  \x1b[48;5;21m  \x1b[48;5;129m  \x1b[0m  \x1b[48;2;255;120;60m  \x1b[48;2;60;160;255m  \x1b[0m\r\n");
    s.push_str("\u{6392}\u{540d}\u{6e2c}\u{8a66}: \u{5f9e} 3 \u{500b}\u{6aa2}\u{67e5}\u{9ede}\u{901a}\u{904e}  \x1b[1;33m\u{8b66}\u{544a}\x1b[0m\u{ff1a}\u{5feb}\u{53d6}\u{904e}\u{671f}\r\n\r\n");
    s.push_str("\x1b[1;32mquarry\x1b[0m \x1b[34m~/ranking\x1b[0m \x1b[2m(main)\x1b[0m $ ");
    s.into_bytes()
}

/// The demo app with the sheet open on the demo shell screen (snapshots). `writable` false is the read-only (Observe) look.
pub fn demo_app(mode: fb_ui::Mode, screen: Screen, writable: bool) -> App {
    let mut app = App::demo(mode, screen);
    app.state.terminal.writable = writable;
    open(&mut app);
    poll(&mut app);
    app
}

pub fn subscription(app: &App) -> Subscription<Message> {
    if app.state.terminal.sheet.is_none() {
        return Subscription::none();
    }
    let poll = time::every(POLL_EVERY).map(|_| Message::Terminal(TerminalMsg::Poll));
    let keys = keyboard::listen().filter_map(|e| match e {
        // Only keys the terminal did not take get here: Esc outside the program, or while the history shows.
        keyboard::Event::KeyPressed { key: keyboard::Key::Named(keyboard::key::Named::Escape), .. } => Some(Message::Terminal(TerminalMsg::Escape)),
        _ => None,
    });
    Subscription::batch([poll, keys])
}

// ---------------------------------------------------------------- view

/// The sheet layer, drawn above the whole window. Empty while closed.
pub fn layer(app: &App) -> Element<'_, Message> {
    if app.state.terminal.sheet.is_none() {
        return Space::new().into();
    }
    responsive(move |window| sheet_in(app, (window.width, window.height))).into()
}

fn sheet_in(app: &App, window: (f32, f32)) -> Element<'_, Message> {
    let t = app.tokens();
    let st = &app.state.terminal;
    let Some(sheet) = st.sheet.as_ref() else { return Space::new().into() };
    let size = SheetSize::clamp(st.size, window);
    let palette = app.state.settings.terminal_palette(&t);

    let title = {
        // Swift `TerminalSheet`: the icon is the regular (16 pt) symbol, the title `headline`; both in the primary label colour.
        let mut r = row![icon(Icon::SquareTerminal, SIZE_DEFAULT, t.label), HEADLINE.text(format!("Terminal \u{00b7} {}", sheet.title)).color(t.label)].spacing(8).align_y(Alignment::Center);
        if !st.writable {
            r = r.push(CAPTION.text("read-only").color(t.label_2));
        }
        r
    };
    // Swift: `xmark` (subheadline semibold) in a 28x28 circle that always carries the island glass fill, not only on hover.
    let close = button(container(icon(Icon::X, SIZE_INLINE, t.label)).center(Length::Fixed(28.0)))
        .padding(0)
        .on_press(Message::Terminal(TerminalMsg::Close))
        .style(move |_, status| button::Style {
            background: Some(Background::Color(if matches!(status, button::Status::Hovered) { t.fill_2 } else { t.fill_1 })),
            border: Border { color: t.sep, width: 1.0, radius: 14.0.into() },
            ..Default::default()
        });
    // Swift `TerminalSheet`: the title row is padded 12 on every side; the fixed height keeps the 28-pt close button centred.
    let header = container(row![title, Space::new().width(Length::Fill), close].align_y(Alignment::Center)).padding([12, 12]).height(Length::Fixed(HEADER_H)).align_y(Alignment::Center);
    let divider = container(Space::new()).width(Length::Fill).height(Length::Fixed(1.0)).style(style::fill(t.sep, 0.0));

    let body: Element<Message> = match &sheet.history {
        Some(h) => history_body(app, h, palette),
        None => live_body(app, sheet, palette),
    };
    let mut layers = vec![container(body).style(style::fill(palette.bg, 0.0)).into()];
    if let Some(reason) = sheet.closed.as_ref().filter(|r| r.as_str() != "detached") {
        layers.push(banner(app, sheet, reason));
    }
    if let Some((text, _)) = &sheet.notice {
        layers.push(container(container(BODY.text(text.clone()).color(t.label)).padding([6, 12]).style(style::float_e2(t, 14.0))).padding(12).width(Length::Fill).height(Length::Fill).align_x(Alignment::Center).align_y(Alignment::End).into());
    }
    let sheet_card = container(column![header, divider, stack(layers).width(Length::Fill).height(Length::Fill)])
        .width(Length::Fixed(size.0))
        .height(Length::Fixed(size.1))
        .style(style::float_e2(t, R_CARD));
    // A modal: the scrim takes the clicks, so nothing below the sheet is reachable while it shows.
    let scrim = mouse_area(container(Space::new()).width(Length::Fill).height(Length::Fill).style(style::fill(iced::Color { a: 0.18, ..iced::Color::BLACK }, 0.0))).on_press(Message::Noop);
    let placed = container(sheet_card).width(Length::Fill).height(Length::Fill).align_x(Alignment::Center).padding(iced::Padding { top: TOPROW_H + 8.0, ..iced::Padding::ZERO });
    stack![scrim, placed].into()
}

fn live_body<'a>(app: &'a App, sheet: &'a Sheet, palette: TermPalette) -> Element<'a, Message> {
    let mut w = Terminal::new(&sheet.snap, palette)
        .focused(sheet.focused)
        .app_cursor(sheet.app_cursor)
        .on_resize(|(c, r)| Message::Terminal(TerminalMsg::Resized(c, r)))
        .on_wheel(|(dy, precise)| Message::Terminal(TerminalMsg::Wheel { dy, precise }))
        .on_focus(|on| Message::Terminal(TerminalMsg::Focus(on)))
        .on_copy(|t| Message::Terminal(TerminalMsg::Copy(t)));
    // Observe or read-only: the widget gets no input callback at all.
    if app.state.terminal.writable && sheet.is_control() && sheet.closed.is_none() {
        w = w.on_input(|b| Message::Terminal(TerminalMsg::Input(b))).on_paste(|s| Message::Terminal(TerminalMsg::Paste(s)));
    }
    container(w).width(Length::Fill).height(Length::Fill).into()
}

fn history_body<'a>(app: &'a App, h: &'a History, palette: TermPalette) -> Element<'a, Message> {
    let t = app.tokens();
    // Swift `ObservedTerminal` floats this bar over the history view (`overlay(alignment: .top)`):
    // `HStack(spacing: 10)`, `.padding(.horizontal, 12).padding(.vertical, 6)` on a material fill.
    let bar = container(
        row![
            icon(Icon::Layers, SIZE_INLINE, t.label_2),
            BODY.text(format!("History \u{00b7} last {} lines", h.lines)).color(t.label),
            Space::new().width(Length::Fill),
            button(CALLOUT.text("Back to live").color(t.accent)).padding([2, 8]).on_press(Message::Terminal(TerminalMsg::BackToLive)).style(move |_, st| button::Style {
                background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.accent_soft)),
                border: Border { radius: R_ROW.into(), ..Default::default() },
                ..Default::default()
            }),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([6, 12])
    .width(Length::Fill)
    .style(style::fill(t.fill_1, 0.0));
    // No callbacks: the history view reads and sends nothing.
    let tall = h.snap.rows as f32 * CELL_H + 2.0 * PAD;
    let view = Terminal::new(&h.snap, palette).height(Length::Fixed(tall)).on_copy(|t| Message::Terminal(TerminalMsg::Copy(t))).on_wheel_passive(|(dy, _)| Message::Terminal(TerminalMsg::HistoryWheel { dy }));
    let list = scrollable(view).on_scroll(|v| Message::Terminal(TerminalMsg::HistoryScrolled { at_bottom: at_bottom(v.absolute_offset().y, v.content_bounds().height, v.bounds().height) })).direction(scrollable::Direction::Vertical(scrollable::Scrollbar::new().anchor(scrollable::Anchor::End))).width(Length::Fill).height(Length::Fill);
    // The bar floats over the terminal (it does not push the text down): the history text scrolls under it.
    stack![
        list,
        container(bar).width(Length::Fill).height(Length::Fill).align_y(Alignment::Start)
    ]
    .into()
}

fn banner<'a>(app: &'a App, sheet: &'a Sheet, reason: &str) -> Element<'a, Message> {
    let t = app.tokens();
    let mut r = row![icon(Icon::CircleHelp, SIZE_DEFAULT, t.blocked), BODY.text(explain(reason)).color(t.label)].spacing(10).align_y(Alignment::Center);
    // Take over is a click of the owner, never automatic. Not offered in read-only, or when the terminal is gone.
    if app.state.terminal.writable && sheet.is_control() && !reason.contains("not found") {
        r = r.push(button(CALLOUT.text("Take over").color(t.accent)).padding([2, 8]).on_press(Message::Terminal(TerminalMsg::TakeOver)).style(move |_, st| button::Style {
            background: matches!(st, button::Status::Hovered).then_some(Background::Color(t.accent_soft)),
            border: Border { radius: R_ROW.into(), ..Default::default() },
            ..Default::default()
        }));
    }
    container(container(r).padding([8, 12]).style(style::float_e2(t, R_CARD))).padding(10).width(Length::Fill).height(Length::Fill).align_x(Alignment::Center).align_y(Alignment::Start).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn sheet_size_clamps_between_min_and_the_window() {
        assert_eq!(SheetSize::clamp((100.0, 100.0), (1280.0, 852.0)), (600.0, 360.0));
        assert_eq!(SheetSize::clamp((5000.0, 5000.0), (1280.0, 852.0)), (1240.0, 792.0));
        assert_eq!(SheetSize::clamp((880.0, 560.0), (1280.0, 852.0)), (880.0, 560.0));
        assert_eq!(SheetSize::clamp((5000.0, 5000.0), (0.0, 0.0)), (5000.0, 5000.0), "an unknown window leaves the maximum open");
        assert_eq!(SheetSize::clamp((5000.0, 5000.0), (500.0, 300.0)), (600.0, 360.0), "a small window never goes below the minimum");
    }

    #[test]
    fn explain_matches_swift() {
        assert_eq!(explain("terminal attach taken over"), "Another client took over this terminal.");
        assert_eq!(explain("terminal attach failed: terminal x already has an attached client"), "Another client is controlling this terminal.");
        assert_eq!(explain("terminal attach failed: terminal not found"), "This terminal no longer exists.");
        assert_eq!(explain("something else"), "something else");
    }

    #[test]
    fn the_debounce_waits_150_ms_after_the_last_change() {
        let t = t0();
        let mut d = ResizeDebounce::new((80, 24));
        d.request((90, 30), t);
        assert_eq!(d.due(t + Duration::from_millis(149)), None);
        d.request((100, 32), t + Duration::from_millis(100));
        assert_eq!(d.due(t + Duration::from_millis(249)), None, "a newer change restarts the wait");
        assert_eq!(d.due(t + Duration::from_millis(250)), Some((100, 32)));
        assert_eq!(d.due(t + Duration::from_secs(5)), None, "sent once");
    }

    #[test]
    fn the_debounce_skips_the_size_already_sent() {
        let t = t0();
        let mut d = ResizeDebounce::new((80, 24));
        d.request((80, 24), t);
        assert_eq!(d.due(t + Duration::from_secs(1)), None);
        d.request((81, 24), t);
        d.request((80, 24), t + Duration::from_millis(10));
        assert_eq!(d.due(t + Duration::from_secs(1)), None, "back to the sent size: nothing to send");
    }

    #[test]
    fn at_bottom_is_true_for_a_short_list_and_at_the_end() {
        assert!(at_bottom(0.0, 100.0, 300.0), "the list is shorter than the view");
        assert!(at_bottom(700.0, 1000.0, 300.0));
        assert!(at_bottom(699.5, 1000.0, 300.0), "within a point of the end");
        assert!(!at_bottom(400.0, 1000.0, 300.0));
    }

    #[test]
    fn the_history_grid_holds_every_line() {
        let h = history_view("one\r\n\x1b[31mtwo\x1b[0m\r\nthree\r\n", 20, 3);
        assert_eq!(h.snap.rows, 3);
        assert_eq!(h.snap.row_text(0), "one");
        assert_eq!(h.snap.row_text(2), "three");
        assert_eq!(h.snap.cursor, None);
    }
}
