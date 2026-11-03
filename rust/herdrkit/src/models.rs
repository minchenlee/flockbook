//! The kit's model (Swift: `Models.swift`), only what the sidecar reaches.

use chrono::{DateTime, Utc};

/// Seconds-precision is not enough for rows; times are kept as UTC instants with sub-second precision.
pub type Date = DateTime<Utc>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackendKind {
    /// The `herdr` 0.9.x TUI server. Not ported (the sidecar talks to herdr-core only).
    Tui,
    /// herdr-core (`hc`).
    Hc,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Agent {
    pub name: String,
    pub kind: String,
    pub status: AgentStatus,
    pub session_id: Option<String>,
    pub title: String,
    pub backend: BackendKind,
    /// hc only: what a `blocked` agent waits for (`permission`, `question`, `auth`), or `error`.
    pub status_kind: Option<String>,
    /// hc only: the agent's process in the terminal, `<pid>@<created_ms>` (`HCMapper.process`).
    pub process: Option<String>,
}

impl Agent {
    pub fn new(name: &str, kind: &str, status: AgentStatus) -> Agent {
        Agent {
            name: name.into(),
            kind: kind.into(),
            status,
            session_id: None,
            title: name.into(),
            backend: BackendKind::Hc,
            status_kind: None,
            process: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pane {
    pub id: String,
    pub terminal_id: String,
    pub cwd: String,
    pub agent: Option<Agent>,
    pub label: Option<String>,
    pub backend: BackendKind,
    /// hc only: the foreground process, then the terminal's own process.
    pub process_ids: Vec<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tab {
    pub id: String,
    pub label: String,
    pub panes: Vec<Pane>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Workspace {
    pub id: String,
    pub label: String,
    pub tabs: Vec<Tab>,
}

impl Workspace {
    pub fn all_panes(ws: &[Workspace]) -> Vec<&Pane> {
        ws.iter().flat_map(|w| w.tabs.iter()).flat_map(|t| t.panes.iter()).collect()
    }
}

/// A picture in a log line (`LogImage.Ref`). The sidecar only counts them, but the fields are kept for parity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageRef {
    pub format: ImageFormat,
    pub width: i64,
    pub height: i64,
    pub offset: Option<u64>,
    pub index: usize,
    pub digest: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
    Gif,
}

/// One tool call behind a chip. `input` and `output` are cut at 4 KB (a trailing "…" marks the cut).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToolCall {
    pub id: String,
    pub tool_name: String,
    pub summary: String,
    pub input: String,
    pub output: Option<String>,
    pub is_error: bool,
    pub images: Vec<ImageRef>,
}

impl ToolCall {
    pub fn new(id: &str, tool_name: &str, summary: &str, input: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            tool_name: tool_name.into(),
            summary: summary.into(),
            input: input.into(),
            output: None,
            is_error: false,
            images: vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ToolChip {
    /// Swift keeps a stable UUID per chip; here its string form.
    pub id: String,
    pub symbol: String,
    pub text: String,
    pub calls: Vec<ToolCall>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PermissionState {
    Pending,
    Resolved,
}

impl PermissionState {
    pub fn raw(self) -> &'static str {
        match self {
            PermissionState::Pending => "pending",
            PermissionState::Resolved => "resolved",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PermissionRequest {
    pub title: String,
    pub command: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub state: PermissionState,
}

impl PermissionRequest {
    pub fn new(title: &str, command: &str, tool_name: &str, tool_use_id: &str, state: PermissionState) -> Self {
        PermissionRequest {
            title: title.into(),
            command: command.into(),
            tool_name: tool_name.into(),
            tool_use_id: tool_use_id.into(),
            state,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Delivery {
    /// The TUI shows a picker; keys drive it.
    Picker,
    /// The answer is a normal user message.
    Message,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QuestionOption {
    pub label: String,
    pub detail: String,
    pub recommended: bool,
}

impl QuestionOption {
    pub fn new(label: &str, detail: &str, recommended: bool) -> Self {
        QuestionOption { label: label.into(), detail: detail.into(), recommended }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Question {
    /// Codex question id; Claude Code: the question text.
    pub id: String,
    pub header: String,
    pub text: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
}

impl Question {
    /// `id` nil: the text (Swift's default).
    pub fn new(id: Option<&str>, header: &str, text: &str, options: Vec<QuestionOption>, multi_select: bool) -> Self {
        Question {
            id: id.unwrap_or(text).into(),
            header: header.into(),
            text: text.into(),
            options,
            multi_select,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum QuestionState {
    Pending,
    /// One answer per question; None when the result could not be matched.
    Answered(Option<Vec<String>>),
    Declined,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QuestionRequest {
    pub tool_use_id: String,
    pub agent: String,
    pub delivery: Delivery,
    pub questions: Vec<Question>,
    pub state: QuestionState,
    /// Screen reads only: every question's header from the picker tab bar.
    pub headers: Vec<String>,
}

impl QuestionRequest {
    pub fn new(tool_use_id: &str, agent: &str, delivery: Delivery, questions: Vec<Question>) -> Self {
        QuestionRequest {
            tool_use_id: tool_use_id.into(),
            agent: agent.into(),
            delivery,
            questions,
            state: QuestionState::Pending,
            headers: vec![],
        }
    }
    /// Questions known only by their header.
    pub fn unread(&self) -> usize {
        self.headers.len().saturating_sub(self.questions.len())
    }
}

/// One answer: option indexes (0-based) and/or free text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct QuestionAnswer {
    pub picks: Vec<i64>,
    pub other: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    UserText(String),
    AssistantText(String),
    ToolChips(Vec<ToolChip>),
    PermissionRequest(PermissionRequest),
    Question(QuestionRequest),
    Status(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ConversationItem {
    pub id: String,
    pub kind: ItemKind,
    pub timestamp: Option<Date>,
    /// userText: the pictures the owner attached, in order.
    pub images: Vec<ImageRef>,
}

impl ConversationItem {
    pub fn new(id: &str, kind: ItemKind, timestamp: Option<Date>) -> Self {
        ConversationItem { id: id.into(), kind, timestamp, images: vec![] }
    }
}

/// The kit's errors (Swift: `HerdrError`, plus the `KeyGateStop` and `QuestionAdapter.Failure` it throws alongside).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HerdrError {
    Socket(String),
    Server { code: String, message: String },
    Protocol(String),
    Closed,
    /// A read or write ran past the timeout. The request may still have reached the server.
    Timeout(String),
    /// A `KeyGate` said no: the input stopped, the keys after it were not sent.
    KeyGateStop,
    /// `QuestionAdapter.Failure`.
    Question(QuestionFailure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionFailure {
    /// The pane does not show this question's picker: nothing was sent.
    NotOnScreen,
    /// This answer cannot be given with keys. Nothing was sent.
    Unsupported(String),
    /// Keys were sent, but the expected screen did not come (stopped there).
    Stuck(String),
    /// All keys were sent, but the picker is still on screen.
    StillOnScreen,
}

impl std::fmt::Display for HerdrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HerdrError::Socket(m) => write!(f, "socket: {m}"),
            HerdrError::Server { code, message } => write!(f, "server error {code}: {message}"),
            HerdrError::Protocol(m) => write!(f, "protocol: {m}"),
            HerdrError::Closed => write!(f, "connection closed"),
            HerdrError::Timeout(m) => write!(f, "timeout: {m}"),
            HerdrError::KeyGateStop => write!(f, "stopped: the pane, its agent or this device's input level changed"),
            HerdrError::Question(q) => match q {
                QuestionFailure::NotOnScreen => write!(f, "the question is not on screen"),
                QuestionFailure::Unsupported(m) => write!(f, "cannot answer: {m}"),
                QuestionFailure::Stuck(m) => write!(f, "stopped waiting for: {m}"),
                QuestionFailure::StillOnScreen => write!(f, "the question is still on screen after answering"),
            },
        }
    }
}

impl std::error::Error for HerdrError {}

/// What became of a message handed to a backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Sent,
    /// No answer in time: it may still arrive; the text must not come back into the composer.
    Unconfirmed,
    Failed,
}

/// Screen reads and keys for the cards (Swift: `HerdrBackend` + `PaneScreen`). Synchronous: the sidecar runs one
/// command and exits. `HcClient` implements it; checks use a fake.
pub trait PaneScreen {
    fn backend(&self) -> BackendKind;
    fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError>;
    /// The pane's visible screen as plain text. Read-only.
    fn visible_text(&self, pane_id: &str) -> Result<String, HerdrError>;
    /// Answers a permission prompt: allow once (`1`), or deny (Esc). Call only when the prompt is on screen.
    fn respond(&self, pane_id: &str, allow: bool) -> Result<(), HerdrError>;
    /// Named keys in herdr's spelling ("Enter", "Down", "Escape", "C-c").
    fn send_named_keys(&self, keys: &[String], pane_id: &str) -> Result<(), HerdrError>;
    /// Raw bytes: control keys as named keys, the rest as text without Enter (`HCKeys.names(for:)`).
    fn send_keys(&self, bytes: &[u8], pane_id: &str) -> Result<(), HerdrError>;
    /// Types `text` and presses Enter.
    fn send_text(&self, text: &str, pane_id: &str) -> Result<(), HerdrError>;
    /// Sleeps between screen polls (fakes skip it).
    fn pause(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}
