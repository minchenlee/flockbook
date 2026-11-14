//! Swift: `PermissionGate.swift`, `ApprovalNotice.swift` and `ApprovalKeys.swift` (the rules the cards use; not the
//! notification text), ported 1:1 from `rust/herdrkit`'s `approval`. An approval card shows only for a permission
//! prompt on screen, and its keys go only while that same prompt is still on screen, read again right before the key.
//!
//! Claude Code's keys (verified against 2.1.294, manual permission mode, Bash tool): the prompt lists `1. Yes`,
//! `2. Yes, and always allow...`, `3. Yes, and switch to auto mode`, `4. No`. `1` (typed, no Enter) approves once;
//! `Escape` denies. Never Enter blindly: the folder-trust dialog highlights "No, exit" by default. The number of
//! options changes with the prompt type, so deny uses Escape, never a number.

use crate::client::HerdrClient;
use crate::error::HerdrError;
use crate::keygate::{fresh_agent, passes, KeyGate};
use crate::model::*;
use crate::question as question_adapter;
use crate::screen::PaneScreen;
use crate::transport::Connector;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;

pub const ALLOW_ONCE: &str = "1";
pub const DENY: &str = "Escape";

/// The TUI `herdr` client's approval input. `respond` sends input to a pane: call it only while the prompt is on screen.
impl<C: Connector> HerdrClient<C> {
    /// Named keys such as "Enter", "Down", "Escape", "C-c".
    pub fn send_named_keys(&self, keys: &[&str], pane_id: &str) -> Result<(), HerdrError> {
        self.raw_request("pane.send_keys", json!({"pane_id": pane_id, "keys": keys})).map(|_| ())
    }

    /// Answer a permission prompt in `pane_id`: allow once (`1`) or deny (Escape).
    pub fn respond(&self, pane_id: &str, allow: bool) -> Result<(), HerdrError> {
        if allow {
            self.raw_request("pane.send_text", json!({"pane_id": pane_id, "text": ALLOW_ONCE})).map(|_| ())
        } else {
            self.send_named_keys(&[DENY], pane_id)
        }
    }

    /// Read-only: does the pane's visible screen show a permission prompt right now (`screen_shows_prompt`)?
    pub fn check_prompt(&self, pane_id: &str) -> bool {
        let Ok(data) = self.raw_request("pane.read", json!({"pane_id": pane_id, "source": "visible", "format": "text", "lines": 40})) else {
            return false;
        };
        let Ok(o) = serde_json::from_slice::<Value>(&data) else { return false };
        o.pointer("/result/read/text").and_then(Value::as_str).map(screen_shows_prompt).unwrap_or(false)
    }
}

/// Swift `PermissionRequest(title:command:toolName:toolUseId:state:)`: no detail, no open-call count.
pub(crate) fn new_permission(title: &str, command: &str, tool_name: &str, tool_use_id: &str, state: PermState) -> PermissionRequest {
    PermissionRequest {
        title: title.into(),
        command: command.into(),
        tool_name: tool_name.into(),
        tool_use_id: tool_use_id.into(),
        state,
        ..Default::default()
    }
}

/// Swift `trimmingCharacters(in: .whitespaces)`: spaces and tabs (Unicode Zs), not newlines.
pub fn trim_ws(s: &str) -> &str {
    s.trim_matches(|c: char| c == '\t' || is_zs(c))
}

fn is_zs(c: char) -> bool {
    matches!(c, ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

fn lines(s: &str) -> Vec<String> {
    s.split('\n').map(String::from).collect()
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

// MARK: - PermissionGate

pub const PROMPT_MARKER: &str = "Do you want";

/// The whole prompt shape: a line "Do you want …?" followed within a few lines by the option "1. Yes", and no
/// question-picker footer ("Type something", "Chat about this"). Grok's and agy's prompts are their own boxes.
pub fn screen_shows_prompt(screen: &str) -> bool {
    if grok_box(screen).is_some() || agy_box(screen).is_some() {
        return true;
    }
    if screen.contains("Chat about this") || screen.contains("Type something") {
        return false;
    }
    let ls: Vec<String> = screen.split('\n').map(|l| l.trim_matches(|c| "│|┃ \t".contains(c)).to_string()).collect();
    static YES: OnceLock<Regex> = OnceLock::new();
    let yes = re(&YES, r"^(?:❯\s*)?1\.\s+Yes\b.*$");
    for (i, l) in ls.iter().enumerate() {
        if !l.starts_with(PROMPT_MARKER) {
            continue;
        }
        // A long file name wraps the question: it ends with "?" within three lines.
        let Some(end) = (i..(i + 3).min(ls.len())).find(|&j| ls[j].ends_with('?')) else { continue };
        let next: Vec<&String> = ls[end + 1..].iter().filter(|x| !x.is_empty()).take(3).collect();
        if next.iter().any(|x| yes.is_match(x)) {
            return true;
        }
    }
    false
}

/// Whether to read the screen for a permission prompt. hc: only `blocked`, and never when the kind says `question`.
pub fn check_approval(agent: Option<&Agent>, pending_tool: bool) -> bool {
    let Some(agent) = agent else { return pending_tool };
    if agent.backend == BackendKind::Hc {
        return agent.status == AgentStatus::Blocked && agent.status_kind.as_deref() != Some("question");
    }
    agent.status == AgentStatus::Blocked || pending_tool
}

/// Whether to read the screen for Claude's question picker: a blocked Claude agent; on hc not when 7501 says `permission`.
pub fn read_question(agent: Option<&Agent>) -> bool {
    let Some(agent) = agent else { return false };
    if agent.status != AgentStatus::Blocked || agent.kind != "claude" {
        return false;
    }
    agent.backend == BackendKind::Tui || agent.status_kind.as_deref() != Some("permission")
}

// MARK: - ApprovalNotice

/// The blocked pane at one moment: what the app knows plus the visible screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneState {
    pub agent_kind: String,
    pub blocked: bool,
    pub pending_tool_use_id: Option<String>,
    pub screen: String,
    pub pending_tool: Option<String>,
    pub pending_command: Option<String>,
}

/// The ids of the prompt the pane shows now. Empty: no buttons. Always the prompt box on screen; the log's pending
/// call too, but only when it is that box (same tool, same command): a log that lags the screen still names the
/// previous call, and its id must never answer the next prompt.
pub fn prompt_ids(s: &PaneState, writes: bool) -> Vec<String> {
    if s.agent_kind == "grok" {
        let Some(b) = grok_box(&s.screen).filter(|_| writes && s.blocked) else { return vec![] };
        let on_screen = grok_id(&b);
        match &s.pending_tool_use_id {
            Some(logged)
                if !logged.is_empty()
                    && !b.command.is_empty()
                    && same_call(s.pending_tool.as_deref(), s.pending_command.as_deref(), "Bash", &b.command) =>
            {
                vec![logged.clone(), on_screen]
            }
            _ => vec![on_screen],
        }
    } else if s.agent_kind == "agy" {
        let Some(b) = agy_box(&s.screen).filter(|_| writes && s.blocked) else { return vec![] };
        let on_screen = agy_id(&b);
        match &s.pending_tool_use_id {
            Some(logged) if !logged.is_empty() && same_call(s.pending_tool.as_deref(), s.pending_command.as_deref(), &b.tool, &b.command) => {
                vec![logged.clone(), on_screen]
            }
            _ => vec![on_screen],
        }
    } else {
        if !(writes && s.blocked && s.agent_kind == "claude") || question_adapter::claude_screen(&s.screen).is_some() {
            return vec![];
        }
        let (Some(b), Some(on_screen)) = (screen_box(&s.screen), screen_prompt(&s.screen)) else { return vec![] };
        match &s.pending_tool_use_id {
            Some(logged) if !logged.is_empty() && same_call(s.pending_tool.as_deref(), s.pending_command.as_deref(), &b.tool, &b.command) => {
                vec![logged.clone(), on_screen]
            }
            _ => vec![on_screen],
        }
    }
}

/// Whitespace-insensitive: the box wraps and indents a long command.
fn words(s: &str) -> String {
    s.split(char::is_whitespace).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The log's call is the prompt box: same tool, same non-empty command. Each line boundary of the box may stand for
/// one space or none; inside a line, spaces must match (as runs).
fn same_call(tool: Option<&str>, command: Option<&str>, box_tool: &str, box_command: &str) -> bool {
    let (Some(tool), Some(command)) = (tool, command) else { return false };
    if words(command).is_empty() || tool != box_tool {
        return false;
    }
    let all = words(command);
    let mut rest: &str = &all;
    let box_lines: Vec<String> = box_command.split('\n').map(words).filter(|l| !l.is_empty()).collect();
    for (i, line) in box_lines.iter().enumerate() {
        if i > 0 && rest.starts_with(' ') {
            rest = &rest[1..];
        }
        if !rest.starts_with(line.as_str()) {
            return false;
        }
        rest = &rest[line.len()..];
    }
    rest.is_empty()
}

/// The prompt the buttons answer: the pending call's id, else the prompt on screen. None: no buttons.
pub fn prompt(s: &PaneState, writes: bool) -> Option<String> {
    prompt_ids(s, writes).into_iter().next()
}

/// Claude's permission prompt box on screen. `tool` = the first word of its header ("Bash command" -> "Bash").
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ScreenBox {
    pub header: String,
    pub tool: String,
    pub command: String,
    pub lines: Vec<String>,
    /// The top of the box is off screen: no header, tool or command.
    pub clipped: bool,
}

/// "⚠ Claude Code will automatically deny this request in 1:23, …": its time changes every second.
pub fn is_countdown(line: &str) -> bool {
    static CD: OnceLock<Regex> = OnceLock::new();
    line.starts_with('⚠') && re(&CD, r"\b\d{1,2}:\d{2}\b").is_match(line)
}

pub fn screen_box(screen: &str) -> Option<ScreenBox> {
    let raw = lines(screen);
    let ask = raw.iter().rposition(|l| l.contains(PROMPT_MARKER))?;
    if question_adapter::claude_screen(screen).is_some() {
        return None;
    }
    let rule = raw[..ask].iter().rposition(|l| trim_ws(l).starts_with('─'));
    let top = rule.map(|r| r + 1).unwrap_or(0);
    // Options: down to the first blank line or the key help line.
    let mut end = ask;
    while end + 1 < raw.len() && !trim_ws(&raw[end + 1]).is_empty() && !trim_ws(&raw[end + 1]).starts_with("Esc to cancel") {
        end += 1;
    }
    let bx: Vec<String> = raw[top..=end]
        .iter()
        .map(|l| {
            let t = trim_ws(l);
            match t.strip_prefix('❯') {
                Some(r) => trim_ws(r).to_string(),
                None => t.to_string(),
            }
        })
        .filter(|t| !t.is_empty() && !t.starts_with("Tip:") && !is_countdown(t))
        .collect();
    let header = bx.first()?.clone();
    if Some(&header) == bx.last() {
        return None;
    }
    let is_rule = |s: &String| s.starts_with('╌');
    let ask_at = bx.iter().rposition(|l| l.contains(PROMPT_MARKER)).unwrap_or(bx.len());
    // Swift slices box[1..<askAt]; an ask line that is the header itself leaves no body.
    let body: Vec<String> = if ask_at >= 1 { bx[1..ask_at].to_vec() } else { vec![] };
    let command: Vec<String> = match body.iter().position(is_rule) {
        Some(r1) => match body[r1 + 1..].iter().position(is_rule) {
            Some(off) => body[r1 + 1..r1 + 1 + off].to_vec(),
            None => body.iter().filter(|l| !is_rule(l)).cloned().collect(),
        },
        None => body.iter().filter(|l| !is_rule(l)).cloned().collect(),
    };
    // A wrapped command has a "│" gutter on each line.
    let ungutter = |s: &String| match s.strip_prefix('│') {
        Some(r) => trim_ws(r).to_string(),
        None => s.clone(),
    };
    // The box is taller than the pane and its top scrolled off into the command: neither header nor command is read.
    if rule.is_none() && (header.starts_with('│') || body.iter().filter(|l| is_rule(l)).count() == 1) {
        return Some(ScreenBox { header: String::new(), tool: String::new(), command: String::new(), lines: bx, clipped: true });
    }
    let tool = header.split(' ').find(|w| !w.is_empty()).unwrap_or("").to_string();
    Some(ScreenBox { header, tool, command: command.iter().map(ungutter).collect::<Vec<_>>().join("\n"), lines: bx, clipped: false })
}

/// Id of the permission prompt on screen: a hash of the tool name and the whole box. None: no prompt.
pub fn screen_prompt(screen: &str) -> Option<String> {
    let b = screen_box(screen)?;
    Some(hash_id(&format!("tool:{}\n{}", b.tool, b.lines.join("\n"))))
}

/// FNV-1a, "screen-<hex>".
pub fn hash_id(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("screen-{h:x}")
}

pub const CLIPPED_TITLE: &str = "Command too long to show here. Read it in the terminal before you answer.";

/// The approval card's request read from the screen. Its `tool_use_id` is the screen prompt's id.
pub fn screen_request(screen: &str, agent: &str) -> Option<PermissionRequest> {
    if agent == "grok" {
        let b = grok_box(screen)?;
        let tool = if b.command.is_empty() { "" } else { "Bash" };
        return Some(new_permission(&b.title, &b.command, tool, &grok_id(&b), PermState::Pending));
    }
    if agent == "agy" {
        let b = agy_box(screen)?;
        return Some(new_permission(&b.header, &b.command, &b.tool, &agy_id(&b), PermState::Pending));
    }
    let b = screen_box(screen)?;
    let id = screen_prompt(screen)?;
    let title = if b.clipped { CLIPPED_TITLE.to_string() } else { b.header.clone() };
    Some(new_permission(&title, &b.command, &b.tool, &id, PermState::Pending))
}

/// agy 1.4.12's command prompt. Only this layout is read; any other (or a box whose top is off screen) is not.
pub fn agy_box(screen: &str) -> Option<ScreenBox> {
    let raw = lines(screen);
    let ask = raw.iter().rposition(|l| trim_ws(l) == "Run this command?")?;
    let req = raw[..ask].iter().rposition(|l| trim_ws(l) == "Requesting permission for:")?;
    let rule = raw[..req].iter().rposition(|l| trim_ws(l).starts_with('─'))?;
    if !raw[rule + 1..req].iter().all(|l| trim_ws(l).is_empty()) {
        return None;
    }
    let command: Vec<String> = raw[req + 1..ask].iter().map(|l| trim_ws(l).to_string()).filter(|l| !l.is_empty()).collect();
    if command.is_empty() {
        return None;
    }
    static ROW: OnceLock<Regex> = OnceLock::new();
    let row = re(&ROW, r"^(>\s*)?(\d+)\.\s+(.*)$");
    let (mut rows, mut cursor, mut at) = (Vec::<String>::new(), Vec::<usize>::new(), ask + 1);
    while at < raw.len() {
        let Some(m) = row.captures(trim_ws(&raw[at])) else { break };
        if m[2].parse::<usize>().ok() != Some(rows.len() + 1) {
            return None;
        }
        if m.get(1).is_some() {
            cursor.push(rows.len());
        }
        rows.push(format!("{}. {}", &m[2], &m[3]));
        at += 1;
    }
    let key = raw[at..].iter().find(|l| !trim_ws(l).is_empty())?;
    if rows.first().map(String::as_str) != Some("1. Yes, run command")
        || !rows.last().is_some_and(|l| l.ends_with(". No, cancel"))
        || cursor.len() != 1
        || !trim_ws(key).starts_with("↑/↓ Navigate")
    {
        return None;
    }
    let mut ls = vec!["Requesting permission for:".to_string()];
    ls.extend(command.iter().cloned());
    ls.push("Run this command?".into());
    ls.extend(rows);
    Some(ScreenBox { header: "Run this command?".into(), tool: "Bash".into(), command: command.join("\n"), lines: ls, clipped: false })
}

pub fn agy_id(b: &ScreenBox) -> String {
    hash_id(&format!("tool:agy\n{}", b.lines.join("\n")))
}

/// Grok Build 1.0.67's permission prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrokBox {
    pub title: String,
    pub command: String,
    pub rows: Vec<String>,
    pub focus: usize,
    pub lines: Vec<String>,
}

impl GrokBox {
    pub fn yes(&self) -> Option<usize> {
        self.rows.iter().position(|r| r == "Yes, proceed")
    }
    /// The row as Grok draws it while empty; typed feedback changes it, and that text is the user's.
    pub fn no(&self) -> Option<usize> {
        self.rows.iter().position(|r| r == "No, reject (type to add feedback)")
    }
}

pub fn grok_box(screen: &str) -> Option<GrokBox> {
    let raw = lines(screen);
    let block = question_adapter::grok_block(&raw)?;
    let key = raw[block.end..].iter().find(|l| !trim_ws(l).is_empty()).map(|l| trim_ws(l).to_string())?;
    static KEY: OnceLock<Regex> = OnceLock::new();
    if !re(&KEY, r"^1/\d+:select\s").is_match(&key) || !key.contains("Ctrl+c:cancel") {
        return None;
    }
    static ROW: OnceLock<Regex> = OnceLock::new();
    let row = re(&ROW, r"^(\d+) \(([○●])\) (.*)$");
    let (mut title, mut command, mut rows, mut focus, mut ls): (Option<String>, Vec<String>, Vec<String>, Vec<usize>, Vec<String>) =
        (None, vec![], vec![], vec![], vec![]);
    for t in question_adapter::grok_lines(&raw[block.clone()]) {
        if let Some(m) = row.captures(&t) {
            if m[1].parse::<usize>().ok() != Some(rows.len() + 1) {
                return None;
            }
            if &m[2] == "●" {
                focus.push(rows.len());
            }
            rows.push(m[3].to_string());
            ls.push(format!("{} {}", &m[1], &m[3]));
        } else {
            if !rows.is_empty() {
                return None; // text below the rows: not a layout this reads
            }
            if title.is_none() && t.starts_with('›') {
                title = Some(trim_ws(&t['›'.len_utf8()..]).to_string());
            } else if let Some(r) = t.strip_prefix('│') {
                command.push(trim_ws(r).to_string());
            }
            ls.push(t);
        }
    }
    let title = title?;
    if focus.len() != 1 || !rows.iter().any(|r| r == "Yes, proceed") || !rows.iter().any(|r| r.starts_with("No, reject")) {
        return None;
    }
    Some(GrokBox { title, command: command.join("\n"), rows, focus: focus[0], lines: ls })
}

pub fn grok_id(b: &GrokBox) -> String {
    hash_id(&format!("tool:grok\n{}", b.lines.join("\n")))
}

/// The next key for Allow or Deny on Grok's prompt, or None: send nothing. Allow only from "Yes, proceed"; Deny from
/// "No, reject", or Down first when that row is right below "Yes, proceed". The always-approve and never-allow rows
/// change the user's settings: never chosen.
pub fn grok_key(screen: &str, allow: bool) -> Option<&'static str> {
    let b = grok_box(screen)?;
    let yes = b.yes()?;
    if allow {
        return if b.focus == yes { Some("Enter") } else { None };
    }
    let no = b.no()?;
    if b.focus == no {
        return Some("Enter");
    }
    if b.focus == yes && no == yes + 1 {
        Some("Down")
    } else {
        None
    }
}

/// The request the card shows and binds its buttons to: the log's call when it is the box on screen, else the box.
pub fn card_request(logged: Option<&PermissionRequest>, on_screen: Option<&PermissionRequest>) -> Option<PermissionRequest> {
    let Some(on_screen) = on_screen else { return logged.cloned() };
    match logged {
        Some(l) if same_call(Some(&l.tool_name), Some(&l.command), &on_screen.tool_name, &on_screen.command) => Some(l.clone()),
        _ => Some(on_screen.clone()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Send { allow: bool },
    Open,
}

/// `now` is read after the action (None: the pane is gone). Anything but the same prompt still on screen: no keys.
pub fn outcome(allow: bool, bound_prompt: Option<&str>, now: Option<&PaneState>, writes: bool) -> Outcome {
    let (Some(bound), Some(now)) = (bound_prompt, now) else { return Outcome::Open };
    if prompt_ids(now, writes).iter().any(|p| p == bound) {
        Outcome::Send { allow }
    } else {
        Outcome::Open
    }
}

/// Read-only: the pane's state with its visible screen and the log's pending call. None when the screen cannot be read.
pub fn approval_state<S: PaneScreen + ?Sized>(
    screen: &S,
    pane_id: &str,
    agent_kind: &str,
    blocked: bool,
    pending: Option<&PermissionRequest>,
) -> Option<PaneState> {
    let text = screen.visible_text(pane_id).ok()?;
    Some(PaneState {
        agent_kind: agent_kind.into(),
        blocked,
        pending_tool_use_id: pending.map(|p| p.tool_use_id.clone()),
        screen: text,
        pending_tool: pending.map(|p| p.tool_name.clone()),
        pending_command: pending.map(|p| p.command.clone()),
    })
}

/// The approval card's input, sent only if `prompt` is still open: the pane is read again right before the keys.
/// `Ok(false)`: nothing sent.
#[allow(clippy::too_many_arguments)]
pub fn respond_if_open<S: PaneScreen + ?Sized>(
    screen: &S,
    pane_id: &str,
    prompt: &str,
    allow: bool,
    agent_kind: &str,
    blocked: bool,
    pending: Option<&PermissionRequest>,
    gate: Option<&KeyGate>,
) -> Result<bool, HerdrError> {
    // `gate`: the same agent first, its status as it is now, then the screen read is the last call before the key.
    let mut blocked = blocked;
    if let Some(seen) = gate.and_then(|g| g.agent.as_ref()) {
        let Some(now) = fresh_agent(screen, seen, pane_id) else { return Ok(false) };
        blocked = now.status == AgentStatus::Blocked;
    }
    let Some(now) = approval_state(screen, pane_id, agent_kind, blocked, pending) else { return Ok(false) };
    if outcome(allow, Some(prompt), Some(&now), true) != (Outcome::Send { allow }) {
        return Ok(false);
    }
    if let Some(allowed) = gate.and_then(|g| g.allowed.as_ref()) {
        if !allowed() {
            return Ok(false);
        }
    }
    if agent_kind == "grok" {
        let Some(on_screen) = prompt_ids(&now, true).last().cloned() else { return Ok(false) };
        return respond_grok(screen, pane_id, &on_screen, allow, gate);
    }
    screen.respond(pane_id, allow)?;
    Ok(true)
}

/// Grok's prompt: each key only while that prompt is on screen with the focus where the key needs it, read right
/// before the key. Deny: Down, wait for the focus on "No, reject", Enter. `Ok(false)`: no answer was given.
pub fn respond_grok<S: PaneScreen + ?Sized>(
    screen: &S,
    pane_id: &str,
    screen_prompt: &str,
    allow: bool,
    gate: Option<&KeyGate>,
) -> Result<bool, HerdrError> {
    let mut moved = false;
    // hc may hold a key for its typing-quiet rule (about 3 s): wait longer there for the focus to move.
    let tries = if screen.backend() == BackendKind::Hc { 60 } else { 20 };
    for _ in 0..tries {
        let text = screen.visible_text(pane_id)?;
        let Some(b) = grok_box(&text) else { return Ok(false) };
        if grok_id(&b) != screen_prompt {
            return Ok(false);
        }
        let key = grok_key(&text, allow);
        if key == Some("Down") && moved {
            screen.pause(150); // the move not drawn yet
            continue;
        }
        let Some(key) = key else { return Ok(false) };
        // The answer's gate before each key: the same agent and the input level, read now. After the Down: the stop
        // is told (`KeyGateStopSent`), not "nothing was sent".
        if !passes(screen, gate, pane_id, None, None) {
            return if moved { Err(HerdrError::KeyGateStopSent) } else { Ok(false) };
        }
        screen.send_named_keys(&[key.to_string()], pane_id)?;
        if key == "Enter" {
            return Ok(true);
        }
        moved = true;
        screen.pause(150);
    }
    // hc: the Down went out but the focus never showed on "No, reject": not "nothing sent".
    if moved && screen.backend() == BackendKind::Hc {
        return Err(HerdrError::Timeout("grok: the focus did not reach No, reject".into()));
    }
    Ok(false)
}

#[cfg(test)]
mod tests;
