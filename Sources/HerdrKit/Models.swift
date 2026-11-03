import Foundation

public enum AgentStatus: String, Sendable, Hashable, Codable {
    case idle, working, blocked, done, unknown

    /// Higher wins in a rollup: blocked > working > done > idle > unknown.
    var rank: Int {
        switch self {
        case .blocked: 4
        case .working: 3
        case .done: 2
        case .idle: 1
        case .unknown: 0
        }
    }

    public static func rollup<S: Sequence>(_ statuses: S) -> AgentStatus where S.Element == AgentStatus {
        statuses.max { $0.rank < $1.rank } ?? .unknown
    }
}

public struct Agent: Sendable, Hashable {
    public var name: String
    public var kind: String
    public var status: AgentStatus
    public var sessionId: String?
    public var title: String
    /// `sessionId` was found by `SessionResolver` (a log path), not given by herdr: a fresh snapshot has none, so a
    /// remote key gate cannot bind to this agent.
    public var sessionResolved = false
    /// Herdr names an agent-team teammate's log (or one not judged a lead's) for this pane: what the pane shows instead.
    public var team: TeamNote?
    /// Which runtime runs this agent. Routes input and decides which actions exist.
    public var backend: BackendKind
    /// hc only: what a `blocked` agent waits for, as the program said (`permission`, `question`, `auth`), or `error`
    /// for a program that reported 7501 `error` (status shows `done`; `title` reads "error: <msg>"). Nil otherwise.
    public var statusKind: String?
    /// hc only: the agent's process in the terminal (`HCMapper.process`): its pid plus the terminal's creation time
    /// (`created_ms`, 0 when hc gives none). The creation time is the terminal's, not the program's, so it does not
    /// guard against a pid reused inside one terminal; hc gives no process start time.
    /// A paired device's key gate binds to it: hc gives no session id, the store finds it in the log. Nil otherwise.
    public var process: String?
    public init(name: String, kind: String, status: AgentStatus, sessionId: String?, title: String,
                backend: BackendKind = .tui, statusKind: String? = nil) {
        self.name = name; self.kind = kind; self.status = status; self.sessionId = sessionId; self.title = title
        self.backend = backend; self.statusKind = statusKind
    }
}

/// A Claude agent team took the pane's log (`SessionResolver.leads`). The words are the app's and the phone's.
public enum TeamNote: String, Sendable, Hashable, Codable {
    /// No lead log is known: the pane shows none, and waits for herdr to name it.
    case waitingForLead
    /// The pane shows the lead's last known log, which may be an older session.
    case lastLead

    public static let waitingTitle = "Waiting for the lead's log"
    public static let waitingDetail = "Herdr points this pane at an agent-team teammate. The lead's conversation shows here once Herdr reports it. The terminal shows what it is doing."
    public static let lastLeadLine = "Agent team: showing the lead's last known session."
}

public struct Pane: Identifiable, Sendable, Hashable {
    public var id: String
    public var terminalId: String
    public var cwd: String
    public var agent: Agent?
    /// Name set with `renamePane`. Nil when the pane has none.
    public var label: String?
    public var backend: BackendKind
    /// hc only: the foreground process, then the terminal's own process (for finding the agent's session log).
    public var processIds: [Int32]
    public init(id: String, terminalId: String, cwd: String, agent: Agent?, label: String? = nil, backend: BackendKind = .tui,
                processIds: [Int32] = []) {
        self.id = id; self.terminalId = terminalId; self.cwd = cwd; self.agent = agent; self.label = label; self.backend = backend
        self.processIds = processIds
    }
}

public struct Tab: Identifiable, Sendable, Hashable {
    public var id: String
    public var label: String
    public var number: Int
    public var panes: [Pane]
    public var status: AgentStatus
    public var backend: BackendKind
    public init(id: String, label: String, number: Int, panes: [Pane], status: AgentStatus, backend: BackendKind = .tui) {
        self.id = id; self.label = label; self.number = number; self.panes = panes; self.status = status; self.backend = backend
    }
}

public struct Workspace: Identifiable, Sendable, Hashable {
    public var id: String
    public var label: String
    public var tabs: [Tab]
    /// `.hc`: a read-only group built from the official app's labels; no layout actions.
    public var backend: BackendKind
    public init(id: String, label: String, tabs: [Tab], backend: BackendKind = .tui) {
        self.id = id; self.label = label; self.tabs = tabs; self.backend = backend
    }
}

public enum SplitDirection: String, Sendable { case right, down }

public struct TerminalFrame: Sendable {
    public var seq: Int
    public var full: Bool
    public var width: Int
    public var height: Int
    public var bytes: Data
    /// hc only: grants the core flow-control credit for these bytes. Nil on the TUI channel.
    var ack: FrameAck? = nil

    /// Call once the view has processed `bytes` (fed them to the terminal). On hc this sends CREDIT, so the core only
    /// sends more as fast as the view keeps up; an unconsumed backlog stops at the core's window (256 KiB). Once per frame.
    public func consumed() { ack?.fire() }
}

/// Runs its action at most once.
final class FrameAck: @unchecked Sendable {
    private let lock = NSLock()
    private var action: (() -> Void)?
    init(_ action: @escaping @Sendable () -> Void) { self.action = action }
    func fire() { let a = lock.withLock { () -> (() -> Void)? in defer { action = nil }; return action }; a?() }
}

/// One tool call behind a chip. `input` and `output` are cut at 4 KB (a trailing "…" marks the cut).
public struct ToolCall: Identifiable, Sendable, Hashable {
    public var id: String          // tool_use id (Codex: call_id)
    public var toolName: String
    public var summary: String     // Bash=command, Read/Write/Edit=path, Task/Agent=description, else first string arg
    public var input: String       // full arguments as pretty JSON
    public var output: String?     // nil until the result arrives
    public var isError: Bool
    public var images: [LogImage.Ref] = []   // pictures in the result, in order
    public init(id: String, toolName: String, summary: String, input: String, output: String? = nil, isError: Bool = false) {
        self.id = id; self.toolName = toolName; self.summary = summary; self.input = input; self.output = output; self.isError = isError
    }
}

public struct ToolChip: Identifiable, Sendable, Hashable {
    public var id = UUID()
    public var symbol: String
    public var text: String
    /// The calls this chip stands for, in order. Results fill in later; resend of `items` carries the update.
    public var calls: [ToolCall]
    public init(symbol: String, text: String, calls: [ToolCall] = [], id: UUID = UUID()) { self.id = id; self.symbol = symbol; self.text = text; self.calls = calls }
}

public struct PermissionRequest: Sendable, Hashable {
    /// `pending`: the log ends with a tool call that has no result yet. It may be waiting for approval
    /// or just running; the app decides with `HerdrClient.checkPrompt`. `resolved`: a result arrived.
    public enum State: String, Sendable, Hashable { case pending, resolved }
    public var title: String
    public var command: String
    public var toolName: String
    public var toolUseId: String
    public var state: State
    public init(title: String, command: String, toolName: String = "", toolUseId: String = "", state: State = .pending) {
        self.title = title; self.command = command; self.toolName = toolName; self.toolUseId = toolUseId; self.state = state
    }
}

/// An agent asks the user to choose: Claude Code's AskUserQuestion, Codex's request_user_input (Plan mode picker)
/// or request_user_input_async (answered by the next user message). `agent` and `delivery` pick the
/// per-agent behaviour in `QuestionAdapter`; the card itself is the same for all.
public struct QuestionRequest: Sendable, Hashable {
    /// How the agent takes the answer.
    public enum Delivery: Sendable, Hashable {
        /// The TUI shows a picker; `HerdrClient.answerQuestion` drives it with keys.
        case picker
        /// The tool returned at once; the answer is a normal user message (`HerdrClient.send(text:to:)` + Enter).
        case message
    }
    public struct Option: Sendable, Hashable {
        /// Label as shown, with a trailing "(Recommended)" / "（建議）" removed.
        public var label: String
        public var detail: String
        public var recommended: Bool
        public init(label: String, detail: String = "", recommended: Bool = false) { self.label = label; self.detail = detail; self.recommended = recommended }
    }
    public struct Question: Sendable, Hashable {
        /// Codex question id (answers are keyed by it); Claude Code: the question text.
        public var id: String
        public var header: String
        public var text: String
        public var options: [Option]
        public var multiSelect: Bool
        public init(id: String? = nil, header: String, text: String, options: [Option], multiSelect: Bool = false) {
            self.id = id ?? text; self.header = header; self.text = text; self.options = options; self.multiSelect = multiSelect
        }
    }
    public enum State: Sendable, Hashable {
        /// No answer yet: the picker is open (or, `.message` delivery, the agent waits for a reply).
        case pending
        /// One answer per question, in question order (multiSelect: labels joined with ", "; "" = skipped).
        /// Nil when the result could not be matched to the questions.
        case answered([String]?)
        /// The user rejected the tool call (Esc in the TUI).
        case declined
    }
    public var toolUseId: String
    /// `Agent.kind` of the asking agent: "claude", "codex" or "grok".
    public var agent: String
    public var delivery: Delivery
    public var questions: [Question]
    public var state: State
    /// Screen reads only (Claude picker): every question's header from the picker tab bar, in order. Empty for log items.
    public var headers: [String]
    /// Questions known only by their header (a read-only screen read sees one question at a time).
    /// `HerdrClient.loadAllQuestions` reads them; `answerQuestion` refuses a request with `unread > 0`.
    public var unread: Int { max(0, headers.count - questions.count) }
    public init(toolUseId: String, agent: String = "claude", delivery: Delivery = .picker, questions: [Question], state: State = .pending, headers: [String] = []) {
        self.toolUseId = toolUseId; self.agent = agent; self.delivery = delivery; self.questions = questions; self.state = state; self.headers = headers
    }
}

/// The user's answer to one question: option indexes (0-based; one for single choice) and/or free text ("Other").
public struct QuestionAnswer: Sendable, Hashable {
    public var picks: [Int]
    public var other: String?
    public init(picks: [Int] = [], other: String? = nil) { self.picks = picks; self.other = other }
}

public struct ConversationItem: Identifiable, Sendable, Hashable {
    public enum Kind: Sendable, Hashable {
        case userText(String)
        case assistantText(String)
        case toolChips([ToolChip])
        case permissionRequest(PermissionRequest)
        /// AskUserQuestion. Never shown as a permissionRequest.
        case question(QuestionRequest)
        case status(String)
    }
    public var id: String
    public var kind: Kind
    /// When the log line was written. For tool chips: the first call of the run. Nil when the line has no time.
    public var timestamp: Date?
    /// userText: the pictures the owner attached, in order.
    public var images: [LogImage.Ref] = []
    public init(id: String = UUID().uuidString, kind: Kind, timestamp: Date? = nil) { self.id = id; self.kind = kind; self.timestamp = timestamp }
}
