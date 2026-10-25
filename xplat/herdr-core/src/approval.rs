//! Keystrokes for Claude Code's permission prompt, and prompt detection on the pane (port of ApprovalKeys.swift and
//! the client part of PermissionGate.swift; the pure `screen_shows_prompt` stays in lib.rs).
//!
//! Verified against Claude Code 2.1.294 in a throwaway test session, manual permission mode, Bash tool: the prompt lists
//! `1. Yes`, `2. Yes, and always allow...`, `3. Yes, and switch to auto mode`, `4. No`. `1` (typed, no Enter) approves once.
//! `Escape` denies. Do NOT use Enter blindly: the folder-trust dialog highlights "No, exit" by default. The number of options
//! changes with the prompt type, so deny uses Escape, never a number.
//!
//! `respond` sends input to a pane: live use is for the `c9-input-test` session only.

use crate::client::HerdrClient;
use crate::error::HerdrError;
use crate::screen_shows_prompt;
use crate::transport::Connector;
use serde_json::{json, Value};

pub const ALLOW_ONCE: &str = "1";
pub const DENY: &str = "Escape";

impl<C: Connector> HerdrClient<C> {
    /// Named keys such as "Enter", "Down", "Escape", "C-c".
    pub fn send_named_keys(&self, keys: &[&str], pane_id: &str) -> Result<(), HerdrError> {
        self.raw_request("pane.send_keys", json!({"pane_id": pane_id, "keys": keys})).map(|_| ())
    }

    /// Answer a permission prompt in `pane_id`. Call only when the agent is `blocked` on a prompt.
    pub fn respond(&self, pane_id: &str, allow: bool) -> Result<(), HerdrError> {
        if allow {
            self.raw_request("pane.send_text", json!({"pane_id": pane_id, "text": ALLOW_ONCE})).map(|_| ())
        } else {
            self.send_named_keys(&[DENY], pane_id)
        }
    }

    /// Read-only: does the pane's visible screen show a permission prompt right now?
    pub fn check_prompt(&self, pane_id: &str) -> bool {
        let Ok(data) = self.raw_request("pane.read", json!({"pane_id": pane_id, "source": "visible", "format": "text", "lines": 40})) else {
            return false;
        };
        let Ok(o) = serde_json::from_slice::<Value>(&data) else { return false };
        o.pointer("/result/read/text").and_then(Value::as_str).map(screen_shows_prompt).unwrap_or(false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokBox {
    pub title: String,
    pub command: String,
    pub rows: Vec<String>,
    pub focus: usize,
    pub lines: Vec<String>,
}
impl GrokBox {
    pub fn next_key(&self, allow: bool) -> Option<&'static str> {
        let yes = self.rows.iter().position(|r| r == "Yes, proceed")?;
        if allow { return (self.focus == yes).then_some("Enter"); }
        let no = self.rows.iter().position(|r| r == "No, reject (type to add feedback)")?;
        if self.focus == no { Some("Enter") } else if self.focus == yes && no == yes + 1 { Some("Down") } else { None }
    }
}
/// Only Grok's last framed permission dialog, with its own footer and exactly one focused row.
pub fn grok_box(screen: &str) -> Option<GrokBox> {
    let raw: Vec<_> = screen.lines().collect();
    let end = raw.iter().rposition(|l| l.trim_start().starts_with('┃'))?;
    let mut start = end;
    while start > 0 && raw[start - 1].trim_start().starts_with('┃') { start -= 1; }
    let footer = raw[end + 1..].iter().find(|l| !l.trim().is_empty())?.trim();
    if !footer.starts_with("1/") || !footer.contains(":select") || !footer.contains("Ctrl+c:cancel") { return None; }
    let mut title = None; let mut command = Vec::new(); let mut rows = Vec::new(); let mut focus = Vec::new(); let mut lines = Vec::new();
    for raw in &raw[start..=end] {
        let t = raw.trim_start().strip_prefix('┃')?.trim_end_matches(|c: char| c == '█' || c.is_whitespace()).trim();
        if t.is_empty() { continue; }
        if let Some((n, rest)) = t.split_once(" (") {
            if let Ok(n) = n.parse::<usize>() {
                if n != rows.len() + 1 { return None; }
                let (mark, row) = rest.split_once(") ")?;
                if mark == "●" { focus.push(rows.len()); } else if mark != "○" { return None; }
                rows.push(row.to_string()); lines.push(format!("{n} {row}")); continue;
            }
        }
        if !rows.is_empty() { return None; }
        if title.is_none() && t.starts_with('›') { title = Some(t.trim_start_matches('›').trim().to_string()); }
        else if let Some(c) = t.strip_prefix('│') { command.push(c.trim().to_string()); }
        lines.push(t.to_string());
    }
    if focus.len() != 1 || !rows.iter().any(|r| r == "Yes, proceed") || !rows.iter().any(|r| r.starts_with("No, reject")) { return None; }
    Some(GrokBox { title: title?, command: command.join("\n"), rows, focus: focus[0], lines })
}

#[cfg(test)]
mod grok_tests {
    use super::*;
    #[test]
    fn deny_moves_only_to_the_plain_reject_row() {
        let s = "┃ › Run command\n┃ │ pwd\n┃ 1 (●) Yes, proceed\n┃ 2 (○) No, reject (type to add feedback)\n1/2:select Ctrl+c:cancel";
        let b = grok_box(s).unwrap();
        assert_eq!(b.next_key(true), Some("Enter"));
        assert_eq!(b.next_key(false), Some("Down"));
        assert!(grok_box(&s.replace("1 (●)", "1 (○)")).is_none());
    }
}
