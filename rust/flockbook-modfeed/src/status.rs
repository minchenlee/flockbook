//! Swift: `Status.swift`. `status`: the tags over the composer (model, effort, ctx, cost, and the owner's own status
//! line parts). Claude Code's facts come from Flockbook's status line wrapper, when the owner turned it on; otherwise
//! the model named in the log is the only tag.

use crate::protocol::JsonLines;
use crate::tail;
use herdrkit::statusline::StatusLineFolder;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Tag {
    /// "model", "effort", "context", "cost", "own".
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<i64>,
    pub warn: bool,
}

/// The folder Flockbook's wrapper writes to: the `--dir` of a `flockbook-statusline` status line command in Claude
/// Code's settings. None when the wrapper is not on.
pub fn folder(settings: &Path) -> Option<PathBuf> {
    let d = std::fs::read(settings).ok()?;
    let o: Value = serde_json::from_slice(&d).ok()?;
    let cmd = o.get("statusLine")?.get("command")?.as_str()?;
    if !cmd.contains("flockbook-statusline") {
        return None;
    }
    let words = shell_words(cmd);
    let i = words.iter().position(|w| w == "--dir")?;
    words.get(i + 1).map(PathBuf::from)
}

/// Splits a command line into words: single quotes, double quotes and backslashes as a POSIX shell reads them.
pub fn shell_words(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut has, mut quote, mut esc): (Vec<String>, String, bool, Option<char>, bool) = (vec![], String::new(), false, None, false);
    for ch in s.chars() {
        if esc {
            cur.push(ch);
            esc = false;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' {
                esc = true;
            } else {
                cur.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                has = true;
            }
            '\\' => {
                esc = true;
                has = true;
            }
            ' ' | '\t' | '\n' => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                has = false;
            }
            _ => cur.push(ch),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn default_settings(env: &HashMap<String, String>) -> PathBuf {
    let dir = env
        .get("CLAUDE_CONFIG_DIR")
        .filter(|d| !d.is_empty())
        .cloned()
        .unwrap_or_else(|| herdrkit::hc::session::home_dir(env) + "/.claude");
    PathBuf::from(dir + "/settings.json")
}

#[derive(Serialize)]
struct Line<'a> {
    #[serde(rename = "type")]
    t: &'a str,
    agent: &'a str,
    source: &'a str,
    tags: Vec<Tag>,
}

/// Prints one `status` line. `log`: the session log (its file name is Claude Code's session id).
pub fn run(agent: &str, log: Option<&Path>, settings: &Path, out: &mut JsonLines, now: SystemTime) {
    let (mut tags, mut source): (Vec<Tag>, &str) = (vec![], "none");
    if agent == "claude" {
        if let (Some(log), Some(dir)) = (log, folder(settings)) {
            let sid = log.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            if let Some(rec) = StatusLineFolder::new(&dir).record(&sid, now) {
                if !rec.is_empty() {
                    tags = rec.tags().into_iter().map(|t| Tag { kind: t.kind.raw().into(), text: t.text, percent: t.percent, warn: t.warn }).collect();
                    source = "statusline";
                }
            }
        }
    }
    if tags.is_empty() {
        if let Some(log) = log {
            if let Ok(r) = tail::read(log, None, 256 * 1024) {
                if let Some(m) = tail::model(&r.lines) {
                    tags = vec![Tag { kind: "model".into(), text: m, percent: None, warn: false }];
                    source = "log";
                }
            }
        }
    }
    out.emit(&Line { t: "status", agent, source, tags });
}
