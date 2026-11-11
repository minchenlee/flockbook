//! Swift: `QueuedInput.swift` (QueuedInput, QueuePull). Messages the owner typed while the agent works that the agent
//! has not taken yet: Claude Code's, Codex's and Grok Build's own queues. Flockbook keeps no queue of its own.
//!
//! Claude Code: its session log records the queue (`queue-operation` lines, `ClaudeQueueLog`), the screen confirms it
//! is still open ("ctrl+enter to send now" under the queued rows) and fills in text the log left out. Codex and Grok
//! Build log nothing until a message goes in: only their screens show it.
//!
//! Pulling back (`pull`): only Claude Code's, only text the log gives exactly, and only with Up, then Ctrl-U one at a
//! time (`ALLOWED_KEYS`). Never Escape: it interrupts the running turn.

use herdr_core::conversation::ClaudeQueueLog;
use herdr_core::{Agent, AgentStatus, BackendKind};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    /// "claude", "codex" or "grok".
    pub agent: String,
    /// Oldest first. None: the agent queued a message whose text neither the log nor the screen shows.
    pub items: Vec<Option<String>>,
    /// The queue can be pulled back now: the agent's input box is empty and the queue is on screen.
    pub pullable: bool,
}

impl Queued {
    /// Claude Code pulls back every queued message as one text (one per line); Codex only the newest.
    pub fn pulls_all(&self) -> bool {
        self.agent == "claude"
    }
}

/// The `queued` line (the page's message without `pane`); none open: an empty one.
pub fn line(q: Option<&Queued>) -> Value {
    match q {
        Some(q) => json!({"type": "queued", "agent": q.agent, "items": q.items, "pullable": q.pullable, "pullsAll": q.pulls_all()}),
        None => json!({"type": "queued", "agent": "", "items": [], "pullable": false, "pullsAll": false}),
    }
}

/// The agents whose queue Flockbook reads.
pub fn reads(kind: &str) -> bool {
    matches!(kind, "claude" | "codex" | "grok")
}

/// The queue of a pane, or None when none is open. `log`: the queue rebuilt from a Claude log.
pub fn read(agent: Option<&Agent>, screen: Option<&str>, log: &ClaudeQueueLog) -> Option<Queued> {
    let agent = agent?;
    if agent.status != AgentStatus::Working {
        return None;
    }
    let screen = screen?;
    match agent.kind.as_str() {
        "claude" => claude(screen, log),
        "codex" => codex_screen(screen).map(|(items, empty)| Queued { agent: "codex".into(), items: items.into_iter().map(Some).collect(), pullable: empty }),
        "grok" => grok_screen(screen).map(|items| Queued { agent: "grok".into(), items: items.into_iter().map(Some).collect(), pullable: false }),
        _ => None,
    }
}

/// Claude Code's queue from the `queue-operation` lines of its log (JSON lines, oldest first).
pub fn log_of<'a>(lines: impl IntoIterator<Item = &'a str>) -> ClaudeQueueLog {
    let mut q = ClaudeQueueLog::default();
    for l in lines {
        if !l.contains("queue-operation") {
            continue;
        }
        if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(l) {
            q.feed(&o);
        }
    }
    q
}

// MARK: text helpers (Swift's CharacterSet and String rules)

/// Swift's `.whitespaces`: tab and the space separators (Zs).
fn ws(c: char) -> bool {
    matches!(c, '\t' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}
/// Swift's `.whitespacesAndNewlines`.
fn ws_nl(c: char) -> bool {
    ws(c) || matches!(c, '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{85}' | '\u{2028}' | '\u{2029}')
}
fn trim(s: &str) -> &str {
    s.trim_matches(ws)
}
fn drop_chars(s: &str, n: usize) -> String {
    s.chars().skip(n).collect()
}
fn lines_of(s: &str) -> Vec<String> {
    s.split('\n').map(String::from).collect()
}

/// Without trailing spaces (the screen pads rows); leading ones are the text's own indentation.
pub fn trailing(s: &str) -> String {
    s.replace('\u{a0}', " ").trim_end_matches(char::is_whitespace).to_string()
}

/// The text without spaces and line breaks: the screen wraps rows (CJK anywhere, words at spaces).
pub fn compact(s: &str) -> String {
    s.chars().filter(|c| !ws_nl(*c)).collect()
}

// MARK: Claude Code

pub const CLAUDE_HINT: &str = "ctrl+enter to send now";
pub const CLAUDE_PLACEHOLDER: &str = "Press up to edit queued messages";

#[derive(Debug, Clone, PartialEq)]
struct Row {
    text: String,
    gapped: bool,
}

/// The queue as Claude Code shows it: the "❯" rows right above the "ctrl+enter to send now" line, oldest first, and
/// whether its input box holds only the placeholder.
pub fn claude_screen(screen: &str) -> Option<(Vec<String>, bool)> {
    claude_rows(screen).map(|(rows, empty)| (rows.into_iter().map(|r| r.text).collect(), empty))
}

/// Claude Code 2.1 draws an empty row between two messages, and a blank line of a message as an empty row too; the
/// rows above the queue (a tool's rows, the sent prompt) have the same shape. So a message read across an empty row
/// is `gapped`: `claude` keeps it only when the log has its text with the same blank lines.
fn claude_rows(screen: &str) -> Option<(Vec<Row>, bool)> {
    let lines = lines_of(screen);
    let hint = lines.iter().rposition(|l| trim(l) == CLAUDE_HINT)?;
    let mut groups: Vec<Row> = vec![];
    let mut cont: Vec<String> = vec![];
    let mut i = hint as i64 - 1;
    while i >= 0 {
        let l = &lines[i as usize];
        let blank = trim(l).is_empty();
        if let Some(rest) = l.strip_prefix("❯ ") {
            let mut rows = vec![trailing(rest)];
            rows.extend(cont.iter().rev().cloned());
            let gapped = rows.iter().any(String::is_empty);
            if !groups.is_empty() && rows.len() > 1 && rows.last().is_some_and(String::is_empty) {
                rows.pop(); // the empty row above the next message
            }
            groups.insert(0, Row { text: rows.join("\n"), gapped });
            cont.clear();
        } else if blank && (!groups.is_empty() || !cont.is_empty()) {
            cont.push(String::new()); // the newest message sits right on the hint
        } else if l.starts_with("  ") && !blank {
            cont.push(trailing(&drop_chars(l, 2)));
        } else {
            break;
        }
        i -= 1;
    }
    if groups.is_empty() {
        return None;
    }
    let input = claude_box(&lines);
    Some((groups, input == [CLAUDE_PLACEHOLDER]))
}

/// The log and the screen, lined up by their text from the newest back (never by count alone). Log entries older
/// than the rows the screen shows stay. A log that does not line up: the screen's rows newer than the newest gapped
/// one.
fn claude(screen: &str, log: &ClaudeQueueLog) -> Option<Queued> {
    let (rows, input_empty) = claude_rows(screen)?;
    let entries = &log.items;
    let mut m = 0;
    while m < entries.len() && m < rows.len() {
        let e = &entries[entries.len() - 1 - m];
        let r = &rows[rows.len() - 1 - m];
        // A log entry without text matches any row but cannot vouch for one read across an empty row.
        let ok = if r.gapped { e.as_deref().is_some_and(|e| same_lines(e, &r.text)) } else { same(e.as_deref(), &r.text) };
        if !ok {
            break;
        }
        m += 1;
    }
    if m == 0 {
        let from = rows.iter().rposition(|r| r.gapped).map(|i| i + 1).unwrap_or(0);
        let newer: Vec<Option<String>> = rows[from..].iter().map(|r| Some(r.text.clone())).collect();
        return (!newer.is_empty()).then(|| Queued { agent: "claude".into(), items: newer, pullable: input_empty });
    }
    let mut items: Vec<Option<String>> = entries[..entries.len() - m].to_vec();
    for k in (0..m).rev() {
        items.push(entries[entries.len() - 1 - k].clone().or_else(|| Some(rows[rows.len() - 1 - k].text.clone())));
    }
    Some(Queued { agent: "claude".into(), items, pullable: input_empty })
}

/// A log entry and a screen row are the same message: the same characters apart from spaces and line breaks; a row
/// cut with "…" matches the start. A log entry without text matches any row.
fn same(entry: Option<&str>, row: &str) -> bool {
    let Some(entry) = entry else { return true };
    let (e, r) = (compact(entry), compact(row));
    e == r || r.strip_suffix('…').is_some_and(|cut| e.starts_with(cut))
}

/// `same`, and the same blank lines in the same places.
fn same_lines(entry: &str, row: &str) -> bool {
    fn runs(s: &str) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        let mut in_run = false;
        for l in s.split('\n') {
            let c = compact(l);
            if c.is_empty() {
                out.push(String::new());
                in_run = false;
            } else if in_run {
                out.last_mut().unwrap().push_str(&c);
            } else {
                out.push(c);
                in_run = true;
            }
        }
        out
    }
    let (e, r) = (runs(entry), runs(row));
    if e == r {
        return true;
    }
    let Some(cut) = r.last().and_then(|c| c.strip_suffix('…')) else { return false };
    if e.len() < r.len() || e[..r.len() - 1] != r[..r.len() - 1] {
        return false;
    }
    e[r.len() - 1].starts_with(cut)
}

// MARK: Codex

pub const CODEX_HEADER: &str = "• Queued follow-up inputs";
pub const CODEX_HINT: &str = "⇧← edit last queued message";
pub const CODEX_PLACEHOLDER: &str = "Ask Codex to do anything";

/// The queue as Codex shows it ("↳" rows under "• Queued follow-up inputs"), and whether its input box holds only the
/// placeholder.
pub fn codex_screen(screen: &str) -> Option<(Vec<String>, bool)> {
    let lines = lines_of(screen);
    let head = lines.iter().rposition(|l| trim(l) == CODEX_HEADER)?;
    let mut items: Vec<String> = vec![];
    let mut margin = 4; // where a row's text starts: after "  ↳ "
    for l in &lines[head + 1..] {
        let t = trim(l);
        if t == CODEX_HINT {
            break;
        }
        if let Some(rest) = t.strip_prefix("↳ ") {
            margin = l.chars().count() - t.chars().count() + 2;
            items.push(trailing(rest));
        } else if !t.is_empty() && l.starts_with(&" ".repeat(margin)) && !items.is_empty() {
            let last = items.last_mut().unwrap();
            last.push('\n');
            last.push_str(&trailing(&drop_chars(l, margin)));
        } else {
            break;
        }
    }
    if items.is_empty() {
        return None;
    }
    let empty = codex_box(&lines) == [CODEX_PLACEHOLDER];
    Some((items, empty))
}

// MARK: Grok Build

pub const GROK_FOOTER: &str = "Ctrl+;:queue";

/// The queue as Grok Build 1.0.67 shows it: rows "    #N text" above its input box, numbered from 1, while its
/// footer offers "Ctrl+;:queue". The last such run counts: numbered lines in the answer above it are not the queue.
pub fn grok_screen(screen: &str) -> Option<Vec<String>> {
    let lines = lines_of(screen);
    let footer = lines.iter().rev().find(|l| !trim(l).is_empty())?;
    if !footer.contains(GROK_FOOTER) {
        return None;
    }
    let bx = lines.iter().rposition(|l| trim(l).starts_with('╭'))?;
    fn row(l: &str) -> Option<(usize, String)> {
        let rest = l.strip_prefix("    #")?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let text = rest[digits.len()..].strip_prefix(' ')?;
        Some((digits.parse().ok()?, trailing(text)))
    }
    let last = lines[..bx].iter().rposition(|l| row(l).is_some())?;
    let mut first = last;
    while first > 0 && row(&lines[first - 1]).is_some() {
        first -= 1;
    }
    let rows: Vec<(usize, String)> = lines[first..=last].iter().filter_map(|l| row(l)).collect();
    if !rows.iter().enumerate().all(|(i, r)| r.0 == i + 1) {
        return None;
    }
    Some(rows.into_iter().map(|r| r.1).collect())
}

// MARK: Pulling back

/// The only keys this feature ever sends (herdr's names; hc takes them as "up", "shift+left", "ctrl+u").
pub const ALLOWED_KEYS: [&str; 3] = ["Up", "shift+left", "ctrl+u"];

pub fn pull_key(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some("Up"),
        "codex" => Some("shift+left"),
        _ => None,
    }
}

/// Refuses a key list with anything outside `ALLOWED_KEYS` (so never Escape, Enter or raw bytes).
pub fn safe(keys: &[&str]) -> bool {
    !keys.is_empty() && keys.iter().all(|k| ALLOWED_KEYS.contains(k))
}

/// "Interrupted" notices on screen. A rise between two reads means the turn stopped. "esc to interrupt" does not
/// count.
pub fn interrupts(screen: &str) -> usize {
    screen.to_lowercase().matches("interrupted").count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The text left the agent's queue: put it in the composer.
    Pulled(String),
    /// The text left the queue but the turn stopped.
    Interrupted(String),
    /// Nothing was sent.
    Refused(String),
    /// Keys went out but the screen did not change as expected; no more keys were sent.
    Failed(String),
    /// The text came back and goes to the composer, but the pull stopped before the box was empty (the reason).
    Partial(String, String),
}

impl Outcome {
    pub fn text(&self) -> Option<&str> {
        match self {
            Outcome::Pulled(t) | Outcome::Interrupted(t) | Outcome::Partial(t, _) => Some(t),
            Outcome::Refused(_) | Outcome::Failed(_) => None,
        }
    }
}

/// The pane as the pull needs it. `keys` returns false when the keys did not go out.
pub trait PullIo {
    fn agent(&mut self) -> Option<Agent>;
    fn screen(&mut self) -> Option<String>;
    /// Sends `keys` only if `expect` (the same kind and identity) is still the pane's agent right before them.
    fn keys(&mut self, keys: &[&str], expect: &Agent) -> bool;
    fn settle(&mut self);
}

/// What tells this agent from another of its kind: hc, the program in the terminal; herdr, the session id.
fn identity(a: &Agent) -> Option<&str> {
    if a.backend == BackendKind::Hc { a.process.as_deref() } else { a.session_id.as_deref() }.filter(|s| !s.is_empty())
}
/// The agent the pull checked: the same kind and identity, both known.
pub fn same_agent(a: &Agent, b: &Agent) -> bool {
    identity(a).is_some() && a.kind == b.kind && a.backend == b.backend && identity(a) == identity(b)
}

enum Sent {
    Out,
    Off,
    OtherAgent,
}

fn send(keys: &[&str], io: &mut impl PullIo, expect: &Agent) -> Sent {
    if !safe(keys) {
        return Sent::Off;
    }
    match io.agent() {
        Some(a) if same_agent(&a, expect) => {}
        _ => return Sent::OtherAgent,
    }
    if io.keys(keys, expect) {
        Sent::Out
    } else {
        Sent::Off
    }
}

/// Up, then Ctrl-U one at a time. Before the first Ctrl-U the text Up brought back goes to `keep` (false: no Ctrl-U
/// at all). Before each Ctrl-U the pull reads the pane again: the same agent, and an input box that still holds what
/// came back less the rows Ctrl-U took; anything else stops it. Only text the box holds exactly as the log has it goes
/// on.
pub fn run(expect: &Agent, log: &ClaudeQueueLog, io: &mut impl PullIo, keep: &mut dyn FnMut(&str) -> bool) -> Outcome {
    use Outcome::*;
    if expect.kind == "grok" {
        return Refused("Grok Build shows its queue only on screen, one row each, where Flockbook cannot read the text exactly. Change it in Grok's terminal (Ctrl+;). Nothing was sent.".into());
    }
    let Some(key) = pull_key(&expect.kind) else { return Refused("This agent has no queue Flockbook can pull back.".into()) };
    if expect.kind != "claude" {
        return Refused("Codex shows its queue only on screen, where Flockbook cannot read the text exactly. Edit it in Codex's terminal (Shift+Left takes back the newest). Nothing was sent.".into());
    }
    if identity(expect).is_none() {
        return Refused("Flockbook cannot tell which agent session runs in this pane, so it does not pull back. Nothing was sent.".into());
    }
    let Some(now) = io.agent().filter(|a| same_agent(a, expect)) else { return Refused("Another agent is in this pane now. Nothing was sent.".into()) };
    if now.status != AgentStatus::Working {
        return Refused("The agent is not working, so nothing is queued. Nothing was sent.".into());
    }
    let Some(before) = io.screen() else { return Refused("No queued message on screen. Nothing was sent.".into()) };
    let Some(q) = read(Some(&now), Some(&before), log) else { return Refused("No queued message on screen. Nothing was sent.".into()) };
    if !q.pullable {
        return Refused("The agent's input box is not empty. Nothing was sent.".into());
    }
    let pulled: Vec<Option<String>> = if q.pulls_all() { q.items.clone() } else { q.items.last().cloned().into_iter().collect() };
    if pulled.iter().any(Option::is_none) {
        return Refused("The queued text cannot be read yet. Nothing was sent.".into());
    }
    let expected = pulled.into_iter().flatten().collect::<Vec<_>>().join("\n");
    // Not exactly in the log: the screen's rows are no proof of the text. The terminal keeps it.
    let from_log = log.items.iter().all(Option::is_some) && log.items.iter().flatten().cloned().collect::<Vec<_>>().join("\n") == expected;
    if !from_log {
        return Refused("Flockbook cannot read this queued text exactly yet. Edit it in the agent's terminal, or try again in a moment. Nothing was sent.".into());
    }
    if !shows(&expected, &before) {
        return Refused("This queued text is too long or has spaces or blank lines that the agent's input box does not show, so Flockbook cannot check it exactly. Edit it in the agent's terminal. Nothing was sent.".into());
    }

    match send(&[key], io, expect) {
        Sent::Out => {}
        Sent::Off => return Refused("Input is off. Nothing was sent.".into()),
        Sent::OtherAgent => return Refused("Another agent is in this pane now. Nothing was sent.".into()),
    }
    io.settle();
    let check = "Check the agent's terminal before you send it.";
    let edit = "It may be in the agent's input box now: edit it in the terminal.";
    let Some(shown) = io.screen() else {
        return Failed(format!("Flockbook could not read the agent's input box after the pull, so it did not take the text or clear it. {edit}"));
    };
    let back = claude_box(&lines_of(&shown));
    if back.is_empty() || back == [CLAUDE_PLACEHOLDER] {
        return Failed("The queued text did not come back to the input box. It is still queued; nothing was cleared.".into());
    }
    let raw = raw_box(&shown);
    if raw.join("\n") != expected {
        return Failed(format!("The agent's input box did not hold exactly the queued text, so Flockbook did not take it or clear it. {edit}"));
    }
    let taken = expected;
    if !keep(&taken) {
        return Partial(taken, format!("Flockbook could not save the text, so it did not clear the agent's input box. {check}"));
    }

    let changed = format!("The agent's input box changed while Flockbook cleared it, so it stopped. {check}");
    let mut cur = raw.clone();
    let (mut sent, mut done) = (false, false);
    let mut after = shown;
    let most = 4 * raw.len() + 4;
    for _ in 0..=most {
        if !io.agent().is_some_and(|a| same_agent(&a, expect)) {
            return Partial(taken, format!("Another agent is in this pane now, so Flockbook stopped clearing its input box. {check}"));
        }
        let Some(s) = io.screen() else {
            return Partial(taken, format!("Flockbook could not read the agent's input box, so it stopped clearing it. {check}"));
        };
        let rows = raw_box(&s);
        after = s;
        if sent {
            let next = cleared(&cur);
            // The placeholder is an empty box only when no text row is left that reads like it.
            let empty = rows.is_empty() || rows == [""] || (cur != [CLAUDE_PLACEHOLDER] && rows == [CLAUDE_PLACEHOLDER]);
            if next.is_empty() && empty {
                done = true;
                break;
            }
            if !next.is_empty() && rows == next {
                cur = next; // and the next Ctrl-U
            } else if rows == cur {
                io.settle();
                continue;
            } else {
                return Partial(taken, changed);
            }
        } else if rows != cur {
            return Partial(taken, changed);
        }
        match send(&["ctrl+u"], io, expect) {
            Sent::Out => sent = true,
            Sent::Off => return Partial(taken, format!("Input went off while Flockbook cleared the agent's input box. {check}")),
            Sent::OtherAgent => return Partial(taken, format!("Another agent is in this pane now, so Flockbook stopped clearing its input box. {check}")),
        }
        io.settle();
    }
    if !done {
        return Partial(taken, format!("The text is still in the agent's input box. {check}"));
    }
    let status = io.agent().map(|a| a.status);
    if interrupts(&after) > interrupts(&before) || !matches!(status, Some(AgentStatus::Working | AgentStatus::Idle | AgentStatus::Done)) {
        return Interrupted(taken);
    }
    Pulled(taken)
}

/// The box after one Ctrl-U: the last row emptied, or an empty last row gone; [] when the box is empty.
fn cleared(rows: &[String]) -> Vec<String> {
    if rows.len() <= 1 {
        return vec![];
    }
    let mut next = rows[..rows.len() - 1].to_vec();
    if rows.last().is_some_and(|r| !r.is_empty()) {
        next.push(String::new());
    }
    if next == [""] {
        vec![]
    } else {
        next
    }
}

/// The most rows a screen read gives.
const VISIBLE_ROWS: i64 = 60;

/// The input box can show `text` character for character on `screen`: only characters of a known width, every line
/// on one row, no line that ends in a space or looks like the box's rule, no blank line at the end, and all the lines
/// on screen with the box's rules and what is under it.
pub fn shows(text: &str, screen: &str) -> bool {
    let Some(width) = claude_width(screen) else { return false };
    if text.is_empty() {
        return false;
    }
    let shown = lines_of(screen);
    let Some(rule) = shown.iter().rposition(|l| is_rule(l)) else { return false };
    let n = shown.len() as i64;
    let room = n.min(VISIBLE_ROWS) - (n - rule as i64) - 2;
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() as i64 > room || lines.last().is_some_and(|l| l.is_empty()) {
        return false;
    }
    lines.iter().all(|l| {
        let Some(cols) = known_columns(l) else { return false };
        let ruled = !l.is_empty() && l.chars().all(|c| ('\u{2500}'..='\u{257f}').contains(&c));
        (cols as i64) < width && !l.ends_with(' ') && !ruled
    })
}

/// The columns of `s` when every character's width is sure: printable ASCII (1), CJK and fullwidth (2), no combining
/// mark. None for anything else (emoji, combining marks, joiners, tabs, no-break spaces).
pub fn known_columns(s: &str) -> Option<usize> {
    let mut n = 0;
    for c in s.chars() {
        let v = c as u32;
        // The combining marks inside the ranges below (U+302A…302F, U+3099…309A): the screen puts them on the
        // column before.
        if (0x302A..=0x302F).contains(&v) || (0x3099..=0x309A).contains(&v) {
            return None;
        }
        n += match v {
            0x20..=0x7E => 1,
            0x3000..=0x303E | 0x3041..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFF01..=0xFF60 | 0xFFE0..=0xFFE6 => 2,
            _ => return None,
        };
    }
    Some(n)
}

/// The columns a row of Claude Code's input may fill: its rule's width less 4 (seen live: Claude Code 2.1).
fn claude_width(screen: &str) -> Option<i64> {
    screen.split('\n').rfind(|l| is_rule(l)).map(|l| trim(l).chars().count() as i64 - 4)
}

fn is_rule(line: &str) -> bool {
    let t = trim(line);
    t.chars().count() >= 10 && t.chars().all(|c| c == '─')
}

/// Claude Code's input box rows as the screen has them: only the box's two-column margin is taken off (the prompt
/// mark and its space on the first row, two columns on the others), nothing trimmed or replaced.
pub fn raw_box(screen: &str) -> Vec<String> {
    let lines = lines_of(screen);
    let rules: Vec<usize> = (0..lines.len()).filter(|&i| is_rule(&lines[i])).collect();
    if rules.len() < 2 {
        return vec![];
    }
    let (a, b) = (rules[rules.len() - 2], rules[rules.len() - 1]);
    lines[a + 1..b]
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                return if l.starts_with('❯') { drop_chars(l, 2) } else { l.clone() };
            }
            let head: Vec<char> = l.chars().take(2).collect();
            if head.len() == 2 && head.iter().all(|c| *c == ' ' || *c == '\u{a0}') {
                drop_chars(l, 2)
            } else {
                l.clone()
            }
        })
        .collect()
}

/// Claude Code: the rows between the last two rule lines; the first starts with "❯ ".
fn claude_box(lines: &[String]) -> Vec<String> {
    let rules: Vec<usize> = (0..lines.len()).filter(|&i| is_rule(&lines[i])).collect();
    if rules.len() < 2 {
        return vec![];
    }
    let (a, b) = (rules[rules.len() - 2], rules[rules.len() - 1]);
    box_rows(&lines[a + 1..b], '❯')
}

/// Codex: the last row that starts with "› ", and the rows under it down to the footer. A newer footer has a
/// status line with no "% left" (model · folder · thread title, after a blank line): that line is the footer's.
fn codex_box(lines: &[String]) -> Vec<String> {
    let Some(i) = lines.iter().rposition(|l| l.starts_with("› ")) else { return vec![] };
    let Some(end) = lines[i + 1..].iter().position(|l| l.contains("% left") || l.contains("? for shortcuts")) else { return vec![] };
    let mut rows = &lines[i..i + 1 + end];
    if let [.., blank, status] = rows {
        if rows.len() > 2 && trim(blank).is_empty() && status.starts_with("  ") && status.contains(" · ") {
            rows = &rows[..rows.len() - 2];
        }
    }
    box_rows(rows, '›')
}

fn box_rows(lines: &[String], mark: char) -> Vec<String> {
    let mut out: Vec<String> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let l = l.replace('\u{a0}', " ");
            if i == 0 && l.starts_with(mark) {
                return trailing(&drop_chars(&l, 2));
            }
            trailing(&if l.starts_with("  ") { drop_chars(&l, 2) } else { l })
        })
        .collect();
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}

// MARK: is the input box empty

/// Whether the agent's input box holds text, from its screen: `Some(true)` text, `Some(false)` empty (or only its
/// placeholder), `None` the box is not found or the agent is not one read here (then a send goes as before: no
/// block on doubt). Spike 11-10 (Claude Code 2.1, Codex 0.167, pi 1.3, agy 1.5, Grok Build 1.0): `send` types after
/// whatever is in the box, so a half-typed "/" turned "/status" into "//status".
///
/// Claude Code with prompt suggestions on (`claude_suggests`, its default) shows a suggested prompt dim in the empty
/// box, and the screen's text has no dim: then only a box that starts with "/" (a command being typed; a suggestion
/// is not one) counts as text, and any other text is a doubt.
pub fn box_has_text(kind: &str, screen: &str, claude_suggests: bool) -> Option<bool> {
    let lines = lines_of(screen);
    let rows = match kind {
        // "❯ " and its rows between the last two rules; its placeholder: "Try \"…\"", or the queue's.
        "claude" => {
            let rows = ruled(&lines, Some('❯'))?;
            let hint = rows.len() == 1 && ((rows[0].starts_with("Try \"") && rows[0].ends_with('"')) || rows[0] == CLAUDE_PLACEHOLDER);
            let rows = if hint { vec![] } else { rows };
            if claude_suggests && rows.iter().any(|r| !trim(r).is_empty()) && !rows[0].trim_start().starts_with('/') {
                return None;
            }
            rows
        }
        "codex" => {
            if !lines.iter().any(|l| l.starts_with("› ") || trim(l) == "›") {
                return None;
            }
            let rows = codex_box(&lines);
            if rows == [CODEX_PLACEHOLDER] { vec![] } else { rows }
        }
        // No placeholder: the rows between the last two rules.
        "pi" => ruled(&lines, None)?,
        // "> " between the last two rules; a mode's note stands there when it is empty ("Auto mode: … (shift+tab to
        // cycle)").
        "agy" => {
            let rows = ruled(&lines, Some('>'))?;
            if rows.len() == 1 && rows[0].ends_with("(shift+tab to cycle)") { vec![] } else { rows }
        }
        // "│ ❯ …" rows between the last "╭" and "╰".
        "grok" => {
            let top = lines.iter().rposition(|l| trim(l).starts_with('╭'))?;
            let bottom = top + 1 + lines[top + 1..].iter().position(|l| trim(l).starts_with('╰'))?;
            let inner: Vec<String> = lines[top + 1..bottom]
                .iter()
                .map(|l| trim(l).trim_start_matches('│').trim_end_matches('│').to_string())
                .collect();
            let first = inner.first()?.trim_start().to_string();
            if !first.starts_with('❯') {
                return None;
            }
            let mut rows = inner;
            rows[0] = first.trim_start_matches('❯').to_string();
            rows
        }
        _ => return None,
    };
    Some(rows.iter().any(|r| !trim(r).is_empty()))
}

/// Claude Code's suggested command in its box (not yet: always None).
pub fn suggested_slash(_screen: &str) -> Option<String> {
    None
}

/// Claude Code's box holds one row only (not yet: always false).
pub fn box_one_line(_screen: &str) -> bool {
    false
}

/// The rows between the last two rule lines, the first starting with `mark` (taken off with its space); another
/// first row (a dialog, a menu): `None`.
fn ruled(lines: &[String], mark: Option<char>) -> Option<Vec<String>> {
    let rules: Vec<usize> = (0..lines.len()).filter(|&i| is_rule(&lines[i])).collect();
    let [.., a, b] = rules[..] else { return None };
    let inside = &lines[a + 1..b];
    match mark {
        Some(m) => {
            let first = inside.first()?.replace('\u{a0}', " ");
            if !first.starts_with(m) {
                return None;
            }
            Some(box_rows(inside, m))
        }
        None => Some(inside.iter().map(|l| trailing(&l.replace('\u{a0}', " "))).collect()),
    }
}

// MARK: on a live pane

/// Claude Code's queue from the last `tail::BYTES` of its log (the part the app shows first).
pub fn log_at(path: Option<&std::path::Path>) -> ClaudeQueueLog {
    match path.map(|p| crate::tail::read(p, None, crate::tail::BYTES)) {
        Some(Ok(r)) => log_of(r.lines.iter().map(String::as_str)),
        _ => ClaudeQueueLog::default(),
    }
}

/// `PullIo` on a herdr-core pane: a fresh snapshot for the agent, the visible screen, named keys.
pub struct Live<'a> {
    pub client: &'a herdr_core::hc::HcClient,
    pub pane_id: String,
}

impl PullIo for Live<'_> {
    fn agent(&mut self) -> Option<Agent> {
        use herdr_core::PaneScreen;
        let ws = self.client.snapshot().ok()?;
        herdr_core::all_panes(&ws).into_iter().find(|p| p.id == self.pane_id)?.agent.clone()
    }
    fn screen(&mut self) -> Option<String> {
        use herdr_core::PaneScreen;
        self.client.visible_text(&self.pane_id).ok()
    }
    fn keys(&mut self, keys: &[&str], expect: &Agent) -> bool {
        use herdr_core::PaneScreen;
        if !safe(keys) {
            return false;
        }
        // The last check before the keys: the pane still runs the agent the pull checked.
        if !self.agent().is_some_and(|a| same_agent(&a, expect)) {
            return false;
        }
        let keys: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        self.client.send_named_keys(&keys, &self.pane_id).is_ok()
    }
    fn settle(&mut self) {
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}

#[cfg(test)]
mod box_tests {
    fn box_has_text(kind: &str, screen: &str) -> Option<bool> {
        super::box_has_text(kind, screen, false)
    }
    fn box_has_text_c(screen: &str) -> Option<bool> {
        super::box_has_text("claude", screen, false)
    }

    const RULE: &str = "────────────────────────────────────────";

    fn claude(row: &str) -> String {
        format!("● Done.\n\n{RULE}\n{row}\n{RULE}\n  ~/proj  ⎇main\n  Opus 5.5 ·medium\n")
    }

    #[test]
    fn claude_box() {
        assert_eq!(box_has_text_c(&claude("❯\u{a0}Try \"fix typecheck errors\"")), Some(false), "its placeholder");
        assert_eq!(box_has_text_c(&claude("❯\u{a0}")), Some(false));
        assert_eq!(box_has_text_c(&claude("❯ Press up to edit queued messages")), Some(false));
        assert_eq!(box_has_text_c(&claude("❯\u{a0}/")), Some(true), "a half-typed command");
        assert_eq!(box_has_text_c(&claude("❯\u{a0}hi there")), Some(true));
        assert_eq!(box_has_text_c(&claude("❯ \n  second line")), Some(true), "text on a later row");
        assert_eq!(box_has_text_c(&claude(" Do you want to proceed?\n ❯ 1. Yes")), None, "a dialog: not the box");
        assert_eq!(box_has_text_c("no box here"), None);
    }

    #[test]
    fn claude_with_prompt_suggestions() {
        let c = |row: &str| super::box_has_text("claude", &claude(row), true);
        assert_eq!(c("❯\u{a0}run the tests"), None, "maybe its dim suggestion: a doubt, sent as before");
        assert_eq!(c("❯\u{a0}/"), Some(true), "a command being typed is never a suggestion");
        assert_eq!(c("❯\u{a0}/sta"), Some(true));
        assert_eq!(c("❯\u{a0}"), Some(false));
        assert_eq!(c("❯\u{a0}Try \"fix typecheck errors\""), Some(false));
    }

    #[test]
    fn codex_box() {
        let codex = |row: &str| format!("• Ran ls\n\n{row}\n\n  GPT-6.1-Sol high · Context 100% left · 0 in\n  ← for agents · ? for shortcuts\n");
        assert_eq!(box_has_text("codex", &codex("› Ask Codex to do anything")), Some(false));
        assert_eq!(box_has_text("codex", &codex("› hi there")), Some(true));
        assert_eq!(box_has_text("codex", &codex("› /")), Some(true));
        assert_eq!(box_has_text("codex", "nothing"), None);
        // A newer Codex footer: a blank line, then model · folder · thread title (no "% left"), then the shortcuts.
        let titled = |row: &str| format!("• Ran ls\n\n{row}\n\n  GPT-6.1-Sol high · ~/src/app · Fix the login form · Main [default]\n  ← for agents · ? for shortcuts\n");
        assert_eq!(box_has_text("codex", &titled("› Ask Codex to do anything")), Some(false));
        assert_eq!(box_has_text("codex", &titled("› hi there")), Some(true));
        assert_eq!(box_has_text("codex", &titled("› first line\n  second line")), Some(true));
    }

    #[test]
    fn pi_box() {
        let pi = |row: &str| format!("{RULE}\n{RULE}\n{row}\n{RULE}\n~/proj  ⎇main\n");
        assert_eq!(box_has_text("pi", &pi("")), Some(false));
        assert_eq!(box_has_text("pi", &pi("hi there")), Some(true));
        assert_eq!(box_has_text("pi", &pi("\nsecond")), Some(true));
        assert_eq!(box_has_text("pi", "no rules"), None);
    }

    #[test]
    fn agy_box() {
        let agy = |row: &str| format!("  Gemini 3.8 Flash (Medium)\n\n{RULE}\n{row}\n{RULE}\n? for shortcuts\n");
        assert_eq!(box_has_text("agy", &agy(">")), Some(false));
        assert_eq!(box_has_text("agy", &agy("> Auto mode: edits & safe commands auto-approved (shift+tab to cycle)")), Some(false), "a mode's note");
        assert_eq!(box_has_text("agy", &agy("> hi there")), Some(true));
        assert_eq!(box_has_text("agy", &agy("Switch Model")), None, "a menu");
    }

    #[test]
    fn grok_box() {
        let grok = |row: &str| format!("  Update: available\n\n  ╭──────────────\n  │ {row}   \n  ╰──────────────\n  Enter:send\n");
        assert_eq!(box_has_text("grok", &grok("❯ ")), Some(false));
        assert_eq!(box_has_text("grok", &grok("❯ hi there")), Some(true));
        assert_eq!(box_has_text("grok", "  ╭──\n  │ Pick one\n  ╰──\n"), None);
    }

    #[test]
    fn others_are_not_read() {
        assert_eq!(box_has_text("cursor-agent", "❯ hi"), None);
    }

    /// Claude Code's suggestion can be a command ("/clear"), dim in the empty box. A typed "/…" shows the command list
    /// right above the box; a lone "/word" with no list above it is taken for the suggestion.
    #[test]
    fn suggested_command() {
        let s = super::suggested_slash;
        let list = "  ❯ /clear        Start a new session with empty context\n    /claude-api   Build apps\n";
        assert_eq!(s(&claude("❯\u{a0}/clear")), Some("/clear".into()), "the dim suggestion");
        assert_eq!(s(&claude("❯ /compact")), Some("/compact".into()));
        assert_eq!(s(&claude("❯\u{a0}/plugin:do-thing")), Some("/plugin:do-thing".into()), "a plugin command");
        assert_eq!(s(&format!("{list}{RULE}\n❯\u{a0}/cl\n{RULE}\n  ~/proj\n")), None, "typed: its list is above the box");
        let plugin_list = "    /claude-md-management:revise-claude-md        (claude-md-management) Update\n";
        assert_eq!(s(&format!("{plugin_list}{RULE}\n❯\u{a0}/cla\n{RULE}\n  ~/proj\n")), None, "a plugin's row in the list");
        assert_eq!(s(&claude("❯\u{a0}/model opus")), None, "a command with a value is typed");
        assert_eq!(s(&claude("❯\u{a0}/clear\n  more text")), None, "two rows");
        assert_eq!(s(&claude("❯\u{a0}/")), None, "a bare slash is typed");
        assert_eq!(s(&claude("❯\u{a0}Try \"fix typecheck errors\"")), None);
        assert_eq!(s(&claude("❯\u{a0}")), None);
        assert_eq!(s(&claude("❯\u{a0}run the tests")), None, "not a command");
        assert_eq!(s(&claude(" Do you want to proceed?\n ❯ 1. Yes")), None, "a dialog: not the box");
        assert_eq!(s("no box here"), None);
    }

    /// After ctrl+e ctrl+u cleared the box's line, whatever still stands on it is not typed (its placeholder, a
    /// suggestion); a second row would be typed text that is still there.
    #[test]
    fn box_of_one_row() {
        let one = super::box_one_line;
        assert!(one(&claude("❯\u{a0}")));
        assert!(one(&claude("❯\u{a0}Try \"x\"")));
        assert!(one(&claude("❯\u{a0}/clear")), "the suggestion again");
        assert!(!one(&claude("❯\u{a0}\n  second line")), "a typed second row is left");
        assert!(!one(&claude(" Do you want to proceed?\n ❯ 1. Yes")), "a dialog");
        assert!(!one("no box"));
    }
}
