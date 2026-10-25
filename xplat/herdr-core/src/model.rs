//! Public model types (port of Models.swift). Plain data, `Clone + PartialEq`.

/// Identity to pin before an asynchronous input operation. Status/title changes are not identity changes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentIdentity {
    pub pane_id: String,
    pub name: String,
    pub kind: String,
    pub session_id: Option<String>,
}

impl AgentIdentity {
    pub fn for_pane(pane: &Pane) -> Option<Self> {
        let a = pane.agent.as_ref()?;
        Some(Self { pane_id: pane.id.clone(), name: a.name.clone(), kind: a.kind.clone(), session_id: a.session_id.clone() })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamNote {
    pub teammate: String,
    pub lead: Option<String>,
}

/// Pane-level state includes the reason a log is deliberately hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationState {
    Loading,
    Ready,
    Missing,
    Failed(String),
    WaitingForLead,
    ShowingLead(TeamNote),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusTag {
    pub kind: String,
    pub text: String,
    pub spoken: String,
    pub percent: Option<f64>,
    pub warn: bool,
}

/// Agent state. Rollup order: blocked > working > done > idle > unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    Unknown,
}

impl AgentStatus {
    /// Parses herdr's wire name. Anything else is `Unknown`.
    pub fn parse(s: Option<&str>) -> AgentStatus {
        match s {
            Some("idle") => AgentStatus::Idle,
            Some("working") => AgentStatus::Working,
            Some("blocked") => AgentStatus::Blocked,
            Some("done") => AgentStatus::Done,
            _ => AgentStatus::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Working => "working",
            AgentStatus::Blocked => "blocked",
            AgentStatus::Done => "done",
            AgentStatus::Unknown => "unknown",
        }
    }

    /// Higher wins in a rollup.
    pub fn rank(self) -> u8 {
        match self {
            AgentStatus::Blocked => 4,
            AgentStatus::Working => 3,
            AgentStatus::Done => 2,
            AgentStatus::Idle => 1,
            AgentStatus::Unknown => 0,
        }
    }

    /// The strongest status of the set; `Unknown` for an empty set.
    pub fn rollup<I: IntoIterator<Item = AgentStatus>>(statuses: I) -> AgentStatus {
        statuses.into_iter().max_by_key(|s| s.rank()).unwrap_or(AgentStatus::Unknown)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub name: String,
    pub kind: String,
    pub status: AgentStatus,
    /// Session id (Claude) or log path (Codex).
    pub session_id: Option<String>,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    pub id: String,
    pub terminal_id: String,
    pub cwd: String,
    pub agent: Option<Agent>,
    /// Name set with `pane.rename`; `None` when the pane has none.
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: String,
    pub label: String,
    pub number: i64,
    pub panes: Vec<Pane>,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    pub id: String,
    pub label: String,
    pub tabs: Vec<Tab>,
}

/// One tool call behind a chip. `input` and `output` are cut at 4 KB (a trailing "…" marks the cut).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub tool_name: String,
    /// Bash = command, Read/Write/Edit = path, Task/Agent = description, else the first string argument.
    pub summary: String,
    pub input: String,
    /// `None` until the result arrives.
    pub output: Option<String>,
    pub is_error: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolChip {
    /// Stable across `items()` calls (derived from the run key and the label).
    pub id: String,
    /// Symbol name, kept from the Swift model (an icon key, not an SF Symbol requirement).
    pub symbol: String,
    pub text: String,
    pub calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermState {
    /// The log ends with a tool call that has no result yet. It may wait for approval or just run.
    Pending,
    Resolved,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PermissionRequest {
    pub title: String,
    pub command: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub state: PermState,
    /// The call's own one-line description (Claude's Bash `description`), shown under the command in the prompt.
    /// Empty when the call has none.
    pub detail: String,
    /// How many tool calls are still waiting for a result, this one included. More than one means parallel calls:
    /// the prompt on screen may belong to another of them.
    pub open_calls: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The TUI shows a picker; answer it with keys.
    Picker,
    /// The tool returned at once; the answer is a normal user message.
    Message,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QOption {
    /// Label as shown, with a trailing "(Recommended)" style suffix removed.
    pub label: String,
    pub detail: String,
    pub recommended: bool,
}

impl QOption {
    pub fn new(label: &str) -> Self {
        QOption { label: label.into(), detail: String::new(), recommended: false }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    /// Codex question id (answers are keyed by it); Claude Code: the question text.
    pub id: String,
    pub header: String,
    pub text: String,
    pub options: Vec<QOption>,
    pub multi_select: bool,
}

impl Question {
    pub fn new(header: &str, text: &str, options: Vec<QOption>, multi_select: bool) -> Self {
        Question { id: text.into(), header: header.into(), text: text.into(), options, multi_select }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum QuestionState {
    Pending,
    /// One answer per question, in order (multiSelect: labels joined with ", "; "" = skipped).
    /// `None` when the result could not be matched to the questions.
    Answered(Option<Vec<String>>),
    /// The user rejected the tool call.
    Declined,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuestionRequest {
    pub tool_use_id: String,
    /// `Agent.kind` of the asking agent: "claude" or "codex".
    pub agent: String,
    pub delivery: Delivery,
    pub questions: Vec<Question>,
    pub state: QuestionState,
    /// Screen reads only: every question's header from the picker tab bar.
    pub headers: Vec<String>,
}

impl QuestionRequest {
    pub fn new(tool_use_id: &str, questions: Vec<Question>) -> Self {
        QuestionRequest {
            tool_use_id: tool_use_id.into(),
            agent: "claude".into(),
            delivery: Delivery::Picker,
            questions,
            state: QuestionState::Pending,
            headers: Vec::new(),
        }
    }

    /// Questions known only by their header.
    pub fn unread(&self) -> usize {
        self.headers.len().saturating_sub(self.questions.len())
    }
}

/// The user's answer to one question: 0-based option indexes and/or free text ("Other").
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuestionAnswer {
    pub picks: Vec<usize>,
    pub other: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    UserText(String),
    AssistantText(String),
    ToolChips(Vec<ToolChip>),
    PermissionRequest(PermissionRequest),
    Question(QuestionRequest),
    Status(String),
    Images(Vec<crate::log_image::ImageRef>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationItem {
    pub id: String,
    pub kind: ItemKind,
    /// Seconds since the Unix epoch of the log line (for chips: the first call). `None` when the line has no time.
    pub timestamp: Option<f64>,
}
