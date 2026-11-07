// Remote protocol v1 (docs/design/remote-protocol.md): the shapes the PWA reads and sends. Types only.

export type Status = "idle" | "working" | "blocked" | "done" | "unknown";
export type Level = "read" | "input";

/** `team`: an agent team took the pane's log; the Mac shows the lead's last known log (lastLead) or none yet (waitingForLead). */
export interface Agent { name: string; kind: string; status: Status; session?: string; title: string; team?: "waitingForLead" | "lastLead" }
export interface Pane { id: string; cwd: string; label?: string; agent?: Agent }
export interface Tab { id: string; label: string; number: number; status: Status; panes: Pane[] }
export interface Workspace { id: string; label: string; tabs: Tab[] }

export interface Approval { id: string; tool: string; title: string; command: string; state: "pending" | "resolved" }
export interface Option { label: string; detail?: string; recommended?: boolean }
export interface Question { id: string; header: string; text: string; multiSelect: boolean; options: Option[] }
export interface QuestionRequest {
  id: string; agent: string; delivery: "picker" | "message"; state: "pending" | "answered" | "declined";
  headers: string[]; questions: Question[]; answers?: string[];
  /** Only the Mac answers it (read from the screen, not from the agent's log). */
  onMac?: boolean;
}
export interface ToolCall { id: string; tool: string; summary: string; done: boolean; error: boolean }
export interface ToolChip { symbol: string; text: string; calls: ToolCall[] }

/** A picture of an item: its size in the log. The client asks for it with `picture` when it comes into view. */
export interface Picture { width: number; height: number }

export type Item =
  | { id: string; time?: string; kind: "user" | "assistant" | "status"; text: string; images?: Picture[];
      /** A user item that is a note from another agent (`hc send`): its sender as hc wrote it ("t2 docs-worker", or ""). */
      from?: string }
  | { id: string; time?: string; kind: "tools"; tools: ToolChip[]; images?: Picture[] }
  | { id: string; time?: string; kind: "approval"; approval: Approval }
  | { id: string; time?: string; kind: "question"; question: QuestionRequest };

export interface HubError { code: string; message?: string }
export interface Device { id: string; name: string; level: Level }

export type HubMessage =
  | { type: "paired"; token: string; device: Device }
  | { type: "welcome"; hub: string; hubVersion: string; level: Level; session?: string; pushKey?: string; pushOn?: boolean }
  | { type: "reply"; re: string; ok: boolean; error?: HubError }
  | { type: "snapshot"; workspaces: Workspace[] }
  | { type: "status"; pane: string; status: Status }
  | { type: "conversation"; pane: string; session: string; mode: "replace" | "append" | "prepend"; items: Item[]; hasEarlier: boolean }
  | { type: "screen"; pane: string; text: string }
  | { type: "prompt"; pane: string; approval?: Approval; question?: QuestionRequest }
  | { type: "queued"; pane: string; agent: string; items: (string | null)[]; pullable: boolean; pullsAll: boolean }
  /** `id`: the hub keeps the text until `pulledAck` with it, and sends it again after each hello. `note`: the pull
   *  stopped before the agent's input box was empty (why). */
  | { type: "pulled"; id?: string; pane: string; text: string; interrupted: boolean; note?: string }
  /** One part of a picture for the `picture` message `re`; all parts come before its reply. */
  | { type: "picturePart"; re: string; pane: string; item: string; index: number; full: boolean; part: number; parts: number;
      mime: string; width: number; height: number; data: string }
  /** Claude Code's status line of a watched pane: Claude Code's facts (its cost is an estimate) and the owner's own line
   *  cut into parts, one tag each (appendix D). No parts and no facts: none to show now. */
  | ({ type: "statusLine" } & StatusLine)
  | { type: "pong" };

/** A piece of a status line in one style; colours are `#rrggbb`. */
export interface StyledRun { text: string; fg?: string; bg?: string; bold?: boolean; dim?: boolean; italic?: boolean; underline?: boolean }
/** `time`: when Claude Code last ran the line, seconds since 1970. */
export interface StatusLine { pane: string; segments: StyledRun[][]; model?: string; effort?: string; costUSD?: number; contextPercent?: number; contextSize?: number; time?: number }

/** The agent's own queue: messages typed while it works, not taken yet. A null item: its text is not readable yet. */
export interface Queue { agent: string; items: (string | null)[]; pullable: boolean; pullsAll: boolean }

export interface Answer { picks: number[]; other?: string }
