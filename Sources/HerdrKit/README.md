# HerdrKit — public API

Module `HerdrKit`. No UI. Swift 6.2, macOS 26. Types match `PLAN.md`; `Sources/HerdrDesk/KitStubs.swift` already mirrors them, so delete the stubs and `import HerdrKit`.

## Session
```swift
public struct HerdrSession: Sendable, Hashable {
    public let name: String?          // nil = default session
    public let socketPath: String
    public static func `default`() -> HerdrSession   // honors nothing from env; ~/.config/herdr/herdr.sock
    public static func named(_ name: String) -> HerdrSession  // ~/.config/herdr/sessions/<name>/herdr.sock
    public static func fromLaunch(arguments: [String], environment: [String: String]) -> HerdrSession
        // `--session <name>` or env HERDR_DESK_SESSION; otherwise default
}
```

## Model
```swift
public enum AgentStatus: String, Sendable { case idle, working, blocked, done, unknown }
public struct Agent: Sendable, Hashable { name, kind, status, sessionId: String?, title }
public struct Pane: Identifiable, Sendable, Hashable { id, terminalId, cwd, agent: Agent? }
public struct Tab: Identifiable, Sendable, Hashable { id, label, number, panes: [Pane], status: AgentStatus }   // status = rollup
public struct Workspace: Identifiable, Sendable, Hashable { id, label, tabs: [Tab] }
```
Rollup order: blocked > working > done > idle > unknown.

## Live store
```swift
@MainActor @Observable public final class HerdrStore {
    public init(session: HerdrSession)
    public private(set) var workspaces: [Workspace]
    public private(set) var connected: Bool
    public func start()                 // snapshot + events.subscribe, auto-reconnect with backoff
    public func stop()
    public func refresh() async         // force a snapshot
    public func conversation(for sessionId: String?) -> [ConversationItem]   // latest parsed items (empty if none)
    public func logURL(for sessionId: String?) -> URL?   // the log file those items come from; nil before it is found
    public static var preview: HerdrStore   // mock data
    public let client: HerdrClient
}
```

## Client (actor)
```swift
public actor HerdrClient {
    public init(session: HerdrSession)
    public func snapshot() async throws -> [Workspace]
    public func send(text: String, to paneId: String) async throws        // text + Enter
    public func sendKeys(_ bytes: [UInt8], to paneId: String) async throws // raw bytes
    public func sendNamedKeys(_ keys: [String], to paneId: String) async throws // e.g. ["Enter"], ["Down"]
    public func observe(terminalId: String) -> AsyncThrowingStream<TerminalFrame, Error>  // read-only
    public func createTab(workspaceId: String, label: String?, cwd: String?) async throws
    public func closeTab(_ tabId: String) async throws
    public func renameTab(_ tabId: String, to label: String) async throws
    public func split(paneId: String, direction: SplitDirection) async throws
}
public struct TerminalFrame: Sendable { seq: Int; full: Bool; width: Int; height: Int; bytes: Data }   // raw ANSI, feed SwiftTerm
```

## Conversation
```swift
public struct ToolChip: Identifiable, Sendable, Hashable { symbol: String; text: String; calls: [ToolCall] }   // SF Symbol name + short label
public struct ToolCall: Identifiable, Sendable, Hashable {
    id: String            // tool_use id (Codex: call_id)
    toolName: String
    summary: String       // Bash=command, Read/Write/Edit=path, Task/Agent=description, else first string arg
    input: String         // full arguments, pretty JSON; cut at 4 KB, "…" marks the cut
    output: String?       // tool_result text, cut at 4 KB; nil until the result arrives
    isError: Bool
    images: [LogImage.Ref]   // pictures in the result, in order; [] when none
}
// The result is a later log line. When it arrives, the next `items` emission carries it in the same chip
// (the chip's item id stays stable), so the UI updates the open detail in place. Codex: mapped by call_id.
public struct PermissionRequest: Sendable, Hashable { title, command, toolName, toolUseId; state: State /* .pending | .resolved */ }
public struct ConversationItem: Identifiable, Sendable, Hashable {
    public enum Kind { case userText(String), assistantText(String), toolChips([ToolChip]), permissionRequest(PermissionRequest), status(String) }
    public var id: String; public var kind: Kind
    public var timestamp: Date?   // time of the log line (`timestamp` field, Claude and Codex). Tool chips: first call of the run. Pending permission: its call. Nil when the line has none.
    public var images: [LogImage.Ref]   // userText: the pictures the owner attached, in order; [] when none
}
public enum LogImage {
    public enum Format { case png, jpeg, webp, gif }
    public struct Ref: Sendable, Hashable { format: Format; width: Int; height: Int; offset: UInt64?; index: Int }
    public static func read(_ ref: Ref, from url: URL) -> Data?   // the picture's bytes from the log file, or nil
    public static func data(line: String, index: Int) -> Data?   // the nth picture of one log line
    public static func header(_ d: Data) -> (format: Format, width: Int, height: Int)?
}
public final class SessionLog: Sendable {
    public init(sessionId: String, agent: String = "claude", remote: RemoteHost? = nil)   // agent = Agent.kind; sessionId = herdr agent_session.value
    // An absolute .jsonl path (herdr kind "path", e.g. pi) is used as is, for any agent.
    // Ids: Claude ~/.claude/projects/*/<id>.jsonl; Codex ~/.codex/sessions/**/rollout-*-<id>.jsonl.
    public static func locate(agent: String, sessionId: String, dir: String) -> URL?
    public func items() -> AsyncStream<[ConversationItem]>   // full list, re-yielded on every change
    public func loadEarlier(bytes: Int = 2_000_000) async -> EarlierLoadResult   // .more, .end, or .failed
    public func setFull(_ full: Bool)   // full = 2 MB tail, short = ~256 KB; HerdrStore drives it from visibility
    public var url: URL?   // the file the tailer reads; nil until found
    public static func parse(lines: [String]) -> [ConversationItem]   // pure, for tests
}
```
If no log is found, `items()` yields `[]` once and keeps polling for the file.

A remote log (`remote` set; `HerdrStore` passes `session.remote`) is looked up on the host by the same rules (in the host
user's home) and read through `RemoteLogMirror`: a sparse local file in `~/Library/Caches/HerdrDesk/remote-logs`
(folder 700, file 600) that holds every fetched byte at the host's offset. The tail comes first, growth is fetched by
polling the host's size about once a second, and `loadEarlier` fetches its range from the host before reading. A host
file that shrinks resets the mirror. `url` is the mirror, so `LogImage.read`, LogPictures and the phone work unchanged.
The mirror file is deleted when the log is no longer watched; files older than a day go when the next one is made.
`SessionResolver` is off for a remote session (`ps`/`lsof` here cannot see the host): panes show herdr's
`agent_session` as named, and teammate logs are not held back.

`loadEarlier` reads up to `bytes` before the current start, cut to whole lines. The older items appear at the front of the next `items()` emission (within one poll, 0.5 s). The whole loaded range is parsed again together, so a tool result in a later segment still matches its call. Item ids do not change (Codex ids come from the line's byte offset), so the UI can keep scroll position. It returns `.end` before the first load and when the file start is reached, `.more` while older content remains, and `.failed` when the range could not be read (including an unreachable host). A failed read preserves the loaded range and can be retried. `HerdrStore.loadEarlier(paneId:) async -> EarlierLoadResult` (or `loadEarlier(for: sessionId)`) wraps it for the store's own log; `.end` for an unknown pane or a pane without an agent session. Call it from the "Show earlier" button after the loaded items are all shown.

### Pictures in the log
Logs keep pictures as base64 inside the JSON line. Items never hold the bytes. They hold a `LogImage.Ref`: the byte
offset of the line, the picture's index in that line (log order), the format and size read from the picture's
header (PNG, JPEG, WebP, GIF), and `digest`: `LogImage.fingerprint(base64:)`, the first 8 bytes of the SHA-256 of the
picture's base64 text. A picture whose header cannot be read gets no ref, but still counts for `index`.
- Claude Code: `image` blocks in a user message go on its `userText` item (a message with only pictures makes a
  `userText("")`). `image` blocks inside a `tool_result` go on the matching `ToolCall`.
- Codex: `input_image` data URLs in a message (its `userText`) or in a tool's output (its `ToolCall`).
- pi: `image` blocks in a `toolResult` go on its `ToolCall`. Rebuilt branches keep each line's offset.
- A picture given as a web URL is never fetched. Cursor: not covered.

`LogImage.read(ref, from: logURL)` reads the line at `offset` and decodes picture `index`. It returns nil unless
`offset` starts a line and the picture there still has the ref's fingerprint, format and size, so a rewritten log
never shows another picture. Reading is file I/O: call it off the main thread.

### User text (Claude Code, Codex)
A user message that starts with a tag is injected context (`<system-reminder>`, `<command-name>`, `<local-command-*>`,
`<task-notification>`, Codex `<environment_context>`, …) and is skipped, except the tags in
`ConversationParser.visibleTags` (an allowlist). Today that is `pasted_content`: Claude Code stores pasted text as
`<pasted_content id="…">…</pasted_content id="…">`. `userText` then holds the text without the wrapper tags; typed
text and pasted blocks keep their order, separated by a blank line. A block ends at the closing tag with its own id,
so pasted text that quotes these tags is kept as is. Pasted blocks are not marked in the model.

### Agents and log formats

| Agent | Log | Notes |
|---|---|---|
| Claude Code | `~/.claude/projects/<slug>/<id>.jsonl` | thinking hidden, except words logged as thinking right before a tool call of the same message (shown as text, as in the terminal); sidechains skipped; a log whose last line is `continued-in` is followed to the newest log of the chain |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*-<id>.jsonl` | `response_item` lines only |
| pi | `~/.pi/agent/sessions/<cwd-slug>/<time>_<uuid>.jsonl` (herdr gives the path) | see below |
| Cursor | `~/.cursor/projects/<slug>/agent-transcripts/<id>/<id>.jsonl` | see below |

pi entries form a tree (`id`/`parentId`). The conversation is the branch that ends at the newest entry: when an entry hangs off an earlier one (an edited or retried turn), the items are rebuilt from that branch, so the abandoned turn disappears. Item ids are `<entry id>-<block>`.
- `user` / `assistant` text: user and assistant text. `thinking`: hidden.
- `toolCall` + `toolResult` (by `toolCallId`): tool chips with output and `isError`. pi tools use Claude's labels: bash→Bash, read/edit/write→Read/Edit/Write, grep→Grep, find/ls→Glob, subagent/subagent_start→Agent; others "Used <name>". A call with no result yet is the pending item, as for the others.
- `bashExecution` (the user ran `!cmd`): a "Ran a command" chip with its output.
- `stopReason` `error` → `.status("Error: <first line>")`; `aborted` → `.status("Interrupted")`.
- `compaction` → `.status("Context compacted")`. `custom_message` with `display: true` → `.status(<first line>)`.
- Not shown: `session`, `model_change`, `thinking_level_change`, `session_info`, `custom` (e.g. worked-for, permission-gate decisions), `context_edit`, the system prompt, images in user messages.

Cursor transcripts have no ids, no times and no tool results. Item ids come from the line's byte offset (`cu@<offset>-<block>`), like Codex.
- User text: the body of `<user_query>`; its `<timestamp>` (minute precision, with the UTC offset) is the item time. User lines without `<user_query>` that start with a tag or `[` are injected context and are skipped. Assistant items have no time.
- `tool_use`: chips with input but no output, never a pending item. Shell/AwaitShell→Bash, StrReplace→Edit, Task→Agent, CallMcpTool/CallDynamicTool→connector; Read/Write/Grep/Glob/WebFetch/WebSearch/TodoWrite keep their names.
- `turn_ended` `error` → `.status("Error")`, `aborted` → `.status("Interrupted")`.
- Not covered: tool output (only in `~/.cursor/chats/<hash>/<id>/store.db`: content-addressed blobs, order kept in binary blobs, readable with `?mode=ro` but not `immutable=1` while the WAL is live), AskQuestion as a question card (no answer in the file), subagent transcripts (`<id>/subagents/`), images.

### Session fallback (no `agent_session`)

herdr can report an agent pane with `agent_session: null` (seen with Codex on herdr 0.9.1: herdr learns the id from
Codex's SessionStart hook, and that report can be missing). `HerdrStore` then asks `SessionResolver` (read-only, off the
main actor, the snapshot never waits) and fills `agent.sessionId` with the log's absolute path:

1. The pane's process: `ps -xE` env with `HERDR_PANE_ID=<pane>` and `HERDR_SOCKET_PATH=<this session's socket>`. The
   agent is the newest child of the pane's shell. Panes of other herdr sessions never match.
2. A log the agent's process tree holds open (`lsof`). Exactly one, or none. (Codex does not keep its rollout open.)
3. Codex: the rollout whose `session_meta` cwd is the pane's cwd and whose start is at or after the agent's start.
   Exactly one match that no other pane uses, or none: it never guesses between two.

Results are cached per pane and kept while the agent pid stays the same (re-checked every 30 s); an unresolved pane is
tried again after 5 s. A Codex `/new` in that pane gives a second match, so the pane keeps no session until the pid
changes. `HerdrKitChecks --agents-live <kinds>` prints `session=id|path|none` per agent.

## Permission prompts
- The log parser emits a `permissionRequest` (state `.pending`) for the newest tool call with no result. It replaces that call's chip. Once the result arrives, it becomes a chip again.
- Pending does **not** mean a prompt: a running tool looks the same. Call `await client.checkPrompt(paneId:)` (reads the screen, looks for "Do you want"). True → Approval card + Allow/Deny. False → show a "Running <tool>…" chip.
- Run `checkPrompt` when an item is pending or `agent.status == .blocked`. Send keys only when it returns true.

## Approval keystrokes
`ApprovalKeys.allowOnce` = `"1"` (typed, no Enter), `.deny` = `Escape`. **Verified** in `c9-input-test` against Claude Code 2.1.294. `HerdrClient.respond(to:allow:)` sends them.

Notification buttons (`ApprovalNotice`): a blocked Claude with a permission prompt on screen gets Allow / Deny (input on, no question picker). The buttons are bound to the log's pending tool call, else to `screenPrompt` (a hash of the tool name and the whole prompt box), so a Claude without a transcript works too. `outcome(...)` re-reads the pane (`approvalState`) and sends (`respond`) only when that same id is still current, else opens the tab. `screenRequest` gives the approval card the same prompt when the log has none.

## Remote protocol
`HubProtocol.swift`: the wire types of remote protocol v1 (spec `docs/design/remote-protocol.md`, fixtures `Tests/HerdrKitTests/Fixtures/hub/`). Codable only, no listener. Clients send intent (`send`, `approve`, `answer`), never keys. `Hub.openApproval` gives the open prompt with the `ApprovalNotice` id.

## Testing
- `HerdrKitChecks` (executable): `swift run HerdrKitChecks` is the real done check; `swift test` does not run under the Command Line Tools.
  Layout: `main.swift` only dispatches; modes live in `ModesBench.swift` / `ModesLive.swift`, checks in `Checks*.swift` (one `@MainActor` function per area), shared fixtures in `Support.swift`. Keep new checks in functions: as one 1500-line top-level body, `main.swift` took over 4 GB to compile.
- `HerdrStore.preview`, `.previewQuiet` (no blocked agent), `.previewDense` (many tabs, long names): fixed preview trees for screenshots. They never connect to a server.
- **Fuzz store, tests only:** `HerdrStore.previewMutate(seed:)` returns a preview store on a session that does not exist (`hk-fuzz-none`), so it can never reach a herdr server. `fuzzStep()` applies one random change to the workspace tree (add/remove workspace, tab, pane; change agent status; rename; empty the tree; drop `connected`). The same seed gives the same sequence. Used by `HerdrDesk --fuzz <seconds> [--seed N] --read-only`. Do not use it in app code.


## TerminalSession (interactive terminal)

For a pane without an agent (a shell) the app needs input, resize and a live screen. `TerminalSession` wraps one
`herdr terminal session control <terminal_id>` process (or `observe` for read-only). Input, resize and frames share its pipes,
so a key press is one small pipe write. Verified in `c9-input-test` (`HerdrKitChecks --term-live <terminal_id> <pane_id>`):
key echo latency median 22-45 ms (frame back from herdr).

```swift
public final class TerminalSession: Sendable {
    public enum Mode { case observe, control(takeover: Bool) }
    public init(terminalId: String, session: HerdrSession, mode: Mode = .control(takeover: false), cols: Int? = nil, rows: Int? = nil)
    public let events: AsyncStream<TerminalEvent>    // .frame(TerminalFrame) | .closed(reason:) | .reconnecting(attempt:) | .notice(String)
    public func start()
    public var size: (cols: Int, rows: Int)?         // pty size as of the last frame
    public func send(bytes: [UInt8]) -> Bool         // any bytes: ESC sequences, Ctrl keys (0x03), UTF-8, SGR mouse. base64 terminal.input
    public func send(text: String) -> Bool
    public func paste(_ text: String, bracketed: Bool? = nil) -> Bool   // ESC[200~ ... ESC[201~, or CR for newlines
    public func resize(cols: Int, rows: Int)         // real pty resize (terminal.resize); ignores < 1; kept for reconnects
    public func wheel(lines: Int, up: Bool, eventPerLine: Bool = false) -> Bool   // control only; herdr routes by the program's mouse mode
    public func scroll(lines: Int, up: Bool)   // same as wheel(lines:up:)
    public func release()                            // give input back (pty returns to the other client's size); ends the stream
    public func stop()                               // end without telling herdr
}
// HerdrClient.terminalSession(terminalId:mode:cols:rows:) makes one.
```

Facts (tested in `c9-input-test`, herdr 0.9.1):
- **Frames**: the first frame is `full: true` (whole screen), later ones are increments (`full: false`). After a resize the
  controller gets a `full` frame with the new `width`/`height`. An observer keeps its own size (default 120x40).
  Frames are herdr's rendered ANSI (cursor positioning, SGR, `?2026` sync, `?25` cursor). They do not carry the program's
  DEC modes (no `?2004`, no application cursor keys, no mouse modes), so the app cannot read those from the stream.
- **Reconnect**: when the process ends without a `terminal.closed`, the session emits `.reconnecting(attempt:)`, waits
  (0.25 s, doubling to 2 s) and starts a new process with the last `--cols/--rows`. The next frame is `full` again.
  `closed` is final: `detached` (we released), `terminal attach taken over`, `… already has an attached client …`, `… not found`.
  No auto takeover; the app should show the reason.
- **Ownership**: one controller per terminal. A second control attach without `takeover` is refused (`closed`). The TUI does
  not hold the controller slot. Call `release()` when the pane leaves the screen so the TUI sees its own size again.
- **Resize**: `pane.resize` in the socket API moves a split divider; it does not set the pty size. Only `terminal.resize`
  (controller) does, so `TerminalSession.resize` is the API. There is no `HerdrClient.resize` without a controller.
- **Bytes**: `HerdrClient.sendKeys` goes through `pane.send_text` (a UTF-8 string, one socket per call). Use
  `TerminalSession.send(bytes:)` for keyboard input. SGR mouse (`ESC[<0;5;3M`) works as plain bytes; `terminal.mouse` does not exist in 0.9.1.
- **Bracketed paste**: `paste` wraps by default. herdr drops the markers when the program has not yet enabled mode 2004
  (for example right after a command ends, before the shell redraws its prompt), and then every newline runs a line.
  Pass `bracketed: false` for programs that do not use it. A frame with `ESC[?2004l` turns the default off.
- **stderr** lines (for example an ignored input) arrive as `.notice`.

## Scrolling and history

The frames of `TerminalSession` are screen snapshots, so a terminal view fed by them has no scrollback. herdr keeps the
history; there are three ways to reach it. Tested in `c9-input-test` (`HerdrKitChecks --scroll-live <terminal_id> <pane_id>`).

```swift
extension HerdrClient {
    func scrollInfo(paneId:) async throws -> ScrollInfo            // pane.get + pane.process_info (read-only)
    func readScrollback(paneId:, lines: Int = 5000, ansi: Bool = true, unwrapped: Bool = false) async throws -> Scrollback  // read-only
    func readDeepHistory(paneId:, lines: Int = 10_000, ansi: Bool = true) async throws -> Scrollback   // pages with pane.scroll; shared state for a moment
    func setScrollOffset(paneId:, fromBottom: Int) async throws    // shared state: the TUI sees it too
}
struct ScrollInfo { maxOffsetFromBottom, offsetFromBottom, viewportRows, foregroundName, shellIsForeground; var kind: .history | .fullScreenProgram | .none }
struct Scrollback { text (ANSI, CRLF line ends), truncated, revision, lineCount }
// TerminalSession.scroll(lines:up:)  (control mode only): terminal.scroll, same shared offset as setScrollOffset.
```

**Recommended path (b): local history, read-only.** On wheel-up in a pane with `kind == .history`, call `readScrollback` and show
the text in a local view (it can go into a SwiftTerm instance: SGR colors are kept, lines end with CRLF); wheel back to the
bottom returns to the live frames. It sends no input, does not move the pane's scroll position, and works for observe and
control sessions alike. Check "reading changes nothing in the pane" in `--scroll-live`.
- herdr returns at most the last **1000 lines** per read (`Scrollback.readLimit`); `truncated` is true when the pane holds more.
  The pane held 2967 lines of history in the test, so one read is a window, not everything.
- For older lines, `readDeepHistory` reads screen by screen through `pane.scroll` + `visible` and puts the offset back. Tested: 3000
  numbered lines come back as exactly 1...3000, in order. While it runs, other clients (the herdr TUI) see the pane scroll,
  and it retries up to 3 times if new output arrives meanwhile. Use it on "load more", not on every wheel tick.

**(a) Shared scroll position.** `setScrollOffset` / `TerminalSession.scroll` move herdr's own viewport; the frames then show the
scrolled screen (tested: an observer's frames follow `setScrollOffset`). The position is shared with every client of that
pane and stays until set back to 0, so it would move the user's herdr TUI too. Use only if the herdr-rendered frame itself must scroll.

**(c) Raw output stream:** not available. herdr's stream is rendered screen diffs; accumulating them locally would not give a
faithful scrollback. Not built.

**Which pane scrolls how (`ScrollInfo.kind`):**
- `.history`: herdr has scrollback lines (`max_offset_from_bottom > 0`). The wheel scrolls history; send nothing to the program.
- `.fullScreenProgram`: no scrollback and the foreground process is not the shell. That is `less`, `vim`, `htop`, and also
  Claude Code (both real Claude panes showed `max_offset 0` with `claude` in the foreground). The program handles the wheel
  itself; the app may forward wheel events as mouse or arrow bytes through `TerminalSession.send(bytes:)`. This is a heuristic:
  a quiet long-running command also looks like this.
- `.none`: fresh shell with nothing above the screen. Do nothing.
herdr's frames do not carry `?1049` (alternate screen), so the app cannot read that mode from the stream; `kind` is the signal.


## Wheel for agent panes (Claude Code and other programs)

`readScrollback` cannot help a pane without scrollback (Claude Code: `max_offset_from_bottom` 0). herdr itself turns a wheel step
into the right thing for the program, and exposes it to a **controller** as `terminal.scroll`:
`TerminalSession.wheel(lines:up:eventPerLine:)` (control mode; observe returns false and sends nothing). Tested in
`c9-input-test` with `cat -v` as the probe (`HerdrKitChecks --wheel-live <terminal_id> <pane_id>`):

| Program state | What the program receives from `wheel` |
|---|---|
| mouse tracking, SGR (`?1000h ?1006h`, also `?1003`) | `ESC[<64;1;1M` (up) / `ESC[<65;1;1M` (down) |
| mouse tracking, legacy (`?1000h` only) | `ESC[M` + byte 96/97 + `!!` (X10 encoding) |
| alternate screen + alternate scroll (`?1049h ?1007h`) | `ESC[A` / `ESC[B` |
| alternate screen, no mouse, no `?1007` | nothing |
| main screen | nothing; herdr scrolls its own history by `lines` |

- A forwarded step is **one event per call**, whatever `lines` is (tested with 1 and 5). Use `eventPerLine: true` to send `lines` events.
  On the main screen `lines` is honored as a scroll distance (10 lines = offset 10).
- The position is always column 1, row 1. There is no `at:` parameter; herdr's `terminal.scroll` has none.
- The app cannot read the program's mode (frames carry no `?1000`/`?1007`/`?1049`, `pane.get` has no mode field), so it cannot
  know which row of the table applies. It does not need to: call `wheel` and herdr decides. The app must not convert the wheel into
  arrow keys itself. Combine with `ScrollInfo.kind`: `.history` -> `readScrollback` (private, read-only) or `wheel`;
  `.fullScreenProgram` -> `wheel`.
- Not verified: whether Claude Code enables mouse tracking or alternate scroll. The Claude pane in `c9-input-test` had nothing to
  scroll, so the screen did not change. If `wheel` has no visible effect on a Claude pane, the program ignores the wheel (case "nothing").
- `wheel` needs the controller slot. An observer-only view cannot forward wheel events (use `readScrollback` there). On the main
  screen it moves the pane position that the herdr TUI shares.

## Pane menu (Rename / Split / Close / right-click routing)

```swift
extension HerdrClient {
    func renamePane(paneId:, to label: String?) async throws      // pane.rename; nil or "" clears. Shows as Pane.label
    func closePane(paneId:) async throws                           // pane.close; throws for a missing pane
    func setRightClickRouting(paneId:, to: .herdr | .pane) async throws   // pane.input.set; WRITE-ONLY
    func split(paneId:, direction:) async throws                   // existing: Split right (.right) / Split down (.down)
}
Pane.label: String?        // from the snapshot
TerminalSession.rightClick(col:row:encoding: .sgr | .legacy) -> Bool    // control mode only
```

- **Send right-clicks to pane** is `pane.input.set` with `right_click: "pane"` (send to the program) or `"herdr"` (herdr's own menu). There is no
  way to read it back: `pane.get`, the snapshot, the layout and events carry no such field. The app must keep the state per pane itself
  (and re-apply it after a restart if it wants the TUI to match). It is a setting of herdr's own mouse handling (the TUI); it does not
  change what `TerminalSession` sends, so the GUI needs its own flag besides.
- **Forwarding a right click from the GUI:** herdr 0.9.1 has no `terminal.mouse` (the control channel rejects it: `unknown variant`).
  `rightClick(col:row:)` sends the xterm bytes as input instead, with coordinates (1-based cells). Tested with `cat -v` as the
  probe: SGR `ESC[<2;12;7M ESC[<2;12;7m` and legacy `ESC[M" , '` arrive intact. The app cannot read the program's mouse mode, so a program
  that has not enabled mouse tracking will show these bytes as text; send it only when the user chose "Send right-clicks to pane".
  Wheel is different: `wheel` is routed by herdr by mode, a click is not.
- **Zoom** is not touched (`pane.zoom` exists in herdr; the GUI does its own).
- Tested in `c9-input-test` with `HerdrKitChecks --pane-live <terminal_id> <pane_id>` (splits the pane once and closes the new one).

## Workspaces (create / rename / close / focus)

```swift
let id = try await client.createWorkspace(label: "api", cwd: "/path", focus: false)  // -> "w5"
try await client.renameWorkspace(id: id, to: "backend")
try await client.focusWorkspace(id: id)
try await client.closeWorkspace(id: id)
```

- `createWorkspace` always makes **one tab (`w5:t1`, label "1") with one shell pane (`w5:p1`)**. No agent starts; the pane runs the user's shell in `cwd`.
- `label` nil: herdr picks the name. `cwd` nil: herdr's default directory. Empty strings count as nil. `focus` only moves herdr's own selection.
- The new id also appears in the next snapshot, so the store picks it up through events.
- Unknown id: `HerdrError.server(code: "workspace_not_found")`.
- `closeWorkspace` closes every tab and pane in it and **terminates their processes, agents included** (no confirmation, no undo). The GUI should confirm first when a pane has a working or blocked agent.
- The CLI's `workspace create --env KEY=VALUE` exists but is not exposed.
- Verified with `HerdrKitChecks --ws-live` in `c9-input-test` only.

## Moving tabs (drag to reorder)

```swift
try await client.moveTab("w5:t1", toIndex: 2)   // final 0-based position among the tabs of its workspace
```

- Socket method `tab.move` {`tab_id`, `insert_index`} exists; the herdr CLI has no command for it. The event `tab.moved` is already in the store's subscription, so the tree refreshes by itself.
- Raw `insert_index` is a slot **before** the tab is removed (moving tab 0 to 2 gives [1, 0, 2]). `moveTab` takes the final index instead and converts it. It reads `tab.list` first, clamps the index, and does nothing when the tab is already there.
- **Tab order is now the snapshot's array order.** `Tab.number` stays with the tab after a move, so the parser no longer sorts by `number`. Do not sort tabs by `number` in the UI. The auto label of an unnamed tab follows its position ("1" becomes "2" after a move).
- Only within one workspace. Unknown tab: `HerdrError.server(code: "tab_not_found")`.
- Verified with `HerdrKitChecks --tabmove-live` (c9-input-test only, throwaway workspace).

## Conversation load state

```swift
switch store.conversationState(for: pane.agent?.sessionId) {
case .loading: ProgressView()                    // first read still running
case .ready:   items.isEmpty ? "No messages yet" : list
case .missing: "No conversation log"             // no file for this session id (yet)
case .failed(let why): Text(why)                 // file exists but cannot be opened
}
```

- `HerdrStore.conversationState(for:) -> ConversationLoadState` starts tailing on first call, like `conversation(for:)`. Both are observable: the view updates when the state changes.
- `.loading` lasts one directory scan plus the first 256 KB read (tens of ms). No timeout guessing is needed.
- `.missing` is not final. A new Claude session writes its file only after the first message; the tailer looks every second for 10 s, then every 5 s, and turns `.ready` when the file appears. Nil session id is `.missing`.
- `.ready` with no items: the file exists and has nothing to show.
- `SessionLog.Snapshot.load` carries the same value for direct `SessionLog` users.

## Questions (AskUserQuestion, Codex request_user_input)

`ConversationItem.Kind.question(QuestionRequest)`. A question tool call never becomes a `permissionRequest` or a tool chip.
Per-agent parsing, screen markers and keys: `QuestionAdapter`. Verified keys and captures: `docs/research/ask-user-question-keys.md`.

```swift
QuestionRequest { toolUseId, agent /* "claude" | "codex" */, delivery /* .picker | .message */, questions: [Question], state }
Question { id /* Codex id; Claude: the text */, header, text, options: [Option], multiSelect }
Option   { label, detail, recommended }
State    .pending | .answered([String]?) | .declined
QuestionAnswer { picks: [Int] /* 0-based */, other: String? }
```

- `label` has a trailing "(Recommended)" / "（建議）" / "（推薦）" removed; `recommended` is true then. `detail` is the option's description.
- `.declined`: the result is an error (Esc), or Codex said the tool is unavailable.
- `.answered(answers)`: one string per question, in question order. multiSelect answers are the chosen labels joined with ", " (Claude keeps the raw label, with its suffix), then the Other text. "" = not answered. `nil` = could not be matched (Codex async: answered by a free-text message); show "Answered".
- Item id `q-<toolUseId>` is stable; only `state` changes when the result arrives.

Where the pending question comes from:
- **Claude Code**: NOT the log. Claude Code writes the AskUserQuestion call to its JSONL only after the answer, so the log has only answered/declined questions. While the picker is open:
  - `try await client.pendingQuestion(paneId:) -> QuestionRequest?` — **read-only, sends nothing**; safe to call automatically. `questions` = the one question on screen; `headers` = every question's header from the picker tab bar; `unread` = questions not read yet (0 for a single question). Nil = no picker.
  - `try await client.loadAllQuestions(paneId:gate:expect:) -> QuestionRequest?` — **sends keys** (Left to question 1, Right through the questions, Left back to where the picker was; navigation only, nothing chosen; `.stuck` if the position is not restored). Call it only on a user action (Next on question 1, a click on the card, or the first step of Send), never on a timer, refresh or render.
  - Both give the same `toolUseId` (`screen-<hash>` of the tab bar). After the answer the logged item (`q-<real id>`) appears.
- **Codex** `request_user_input` (Plan mode): the log item with `.pending` (the call is logged while the picker is open).
- **Codex** `request_user_input_async`: the log item with `delivery == .message`; it stays `.pending` until the next user message.

Answering: `try await client.answerQuestion(paneId:request:answers:)`, one `QuestionAnswer` per question.
- `.picker`: checks the picker with the first question is on screen (only the picker part counts, not the prompt text above it; Claude: if the user moved the picker to another question of this request, it goes back with Left; anything else is `.notOnScreen`, nothing sent), types the keys (`pane.send_text`, no Enter), waits for each next question (`.stuck` if it does not come), then checks the picker is gone (`.stillOnScreen`). Throws `.unsupported` before sending for: `unread > 0`, wrong answer count, option out of range, several picks on single choice, empty answer, line breaks in the text, Codex multiSelect.
- `.message`: sends the answer as a user message with Enter (one question: the answer; several: "question answer" lines). Not verified end-to-end.
- Verified end-to-end in c9-input-test: Claude (single, multiSelect + Other, 3 questions, 2 questions with multiSelect + Other) and Codex picker (single, 2 questions with notes).
