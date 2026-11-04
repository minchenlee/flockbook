//! Swift: `Feed.swift`. `feed`: the rows of one agent's conversation, whole the first time and as a splice after.
//!
//! The sidecar keeps no process between calls. The mod passes back the `cursor` of the last answer; the sidecar keeps
//! the rows it printed for that cursor (their hashes) in the cache file. When the log grew, it parses the same range
//! again, compares, and prints only the rows from the first one that changed: `splice {at}` and the rows after it.

use crate::protocol::{fnv1a, JsonLines};
use crate::{rows, tail};
use herdr_core::util::Date;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

/// The range parsed again on each growth stops growing here; then the sidecar starts a new 2 MB tail (splice 0).
pub const MAX_SPAN: u64 = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub path: String,
    pub start: u64,
    pub size: u64,
    pub hashes: Vec<String>,
}

impl State {
    pub fn cursor(&self) -> String {
        format!("1.{}.{}.{}.{}", fnv1a(&self.path), self.start, self.size, fnv1a(&self.hashes.join(",")))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Cursor {
    path_hash: String,
    start: u64,
    size: u64,
    rows_hash: String,
}

fn parse(c: Option<&str>) -> Option<Cursor> {
    let p: Vec<&str> = c?.split('.').collect();
    if p.len() != 5 || p[0] != "1" {
        return None;
    }
    Some(Cursor { path_hash: p[1].into(), start: p[2].parse().ok()?, size: p[3].parse().ok()?, rows_hash: p[4].into() })
}

/// Prints the `log` line and the rows (or `unchanged`) for `path`, and returns the state to keep.
/// `previous`: the state kept for this terminal. It counts only when `cursor` names it.
pub fn run<Tz: chrono::TimeZone>(
    path: Option<&Path>,
    agent: &str,
    cursor: Option<&str>,
    previous: Option<&State>,
    out: &mut JsonLines,
    now: &Date,
    tz: &Tz,
) -> std::io::Result<Option<State>> {
    let Some(url) = path else {
        out.emit_value(json!({"type": "log", "agent": agent, "path": null}));
        out.emit_value(json!({"type": "splice", "at": 0, "count": 0}));
        return Ok(None);
    };
    let path_s = url.to_string_lossy().to_string();
    let size = std::fs::metadata(url).map(|m| m.len()).unwrap_or(0);
    let c = parse(cursor);
    let prev = previous.filter(|p| {
        c.as_ref().is_some_and(|c| {
            c.path_hash == fnv1a(&p.path) && c.start == p.start && c.size == p.size && c.rows_hash == fnv1a(&p.hashes.join(","))
        })
    });
    if let Some(prev) = prev {
        if prev.path == path_s && prev.size == size {
            out.emit_value(json!({"type": "log", "agent": agent, "path": path_s, "start": prev.start, "size": size, "cursor": prev.cursor()}));
            out.emit_value(json!({"type": "unchanged"}));
            return Ok(Some(prev.clone()));
        }
    }
    // Keep the old range start while the log only grew and the range is not too big.
    let keep = prev.and_then(|p| (p.path == path_s && size >= p.size && size - p.start <= MAX_SPAN).then_some(p.start));
    let range = tail::read(url, keep, tail::BYTES)?;
    let rs = rows::rows(&tail::items(&range.lines), now, tz);
    let mut enc = JsonLines::new();
    for r in &rs {
        enc.emit(r);
    }
    let hashes: Vec<String> = enc.lines.iter().map(|l| fnv1a(l)).collect();
    let mut at = 0;
    if let (Some(prev), Some(_)) = (prev, keep) {
        while at < prev.hashes.len().min(hashes.len()) && prev.hashes[at] == hashes[at] {
            at += 1;
        }
    }
    let state = State { path: path_s.clone(), start: range.start, size: range.size, hashes };
    let mut log = json!({"type": "log", "agent": agent, "path": path_s, "start": range.start, "size": range.size, "cursor": state.cursor()});
    if let Some(m) = tail::model(&range.lines) {
        log["model"] = json!(m);
    }
    out.emit_value(log);
    out.emit_value(json!({"type": "splice", "at": at, "count": rs.len()}));
    for l in &enc.lines[at..] {
        out.raw(l.clone());
    }
    Ok(Some(state))
}

/// `feed --before <start>`: the rows of the 2 MB before `start`. Keeps no state.
pub fn older<Tz: chrono::TimeZone>(path: &Path, before: u64, out: &mut JsonLines, now: &Date, tz: &Tz) -> std::io::Result<()> {
    let range = tail::read_before(path, before, tail::BYTES)?;
    let rs = rows::rows(&tail::items(&range.lines), now, tz);
    out.emit_value(json!({"type": "older", "path": path.to_string_lossy(), "start": range.start, "count": rs.len()}));
    for r in &rs {
        out.emit(r);
    }
    Ok(())
}
