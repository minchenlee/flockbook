//! Swift: `Tail.swift`. Reads the end of a session log and parses it.

use herdr_core::ConversationItem;
use herdr_core::ConversationParser;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The app shows the last 2 MB first; the mod does the same.
pub const BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub lines: Vec<String>,
    /// File offset of the first whole line read.
    pub start: u64,
    /// File size when read. Bytes after the last newline (a line still being written) are left for the next read.
    pub size: u64,
}

fn read_span(path: &Path, from: u64, to: u64) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(from))?;
    let mut data = Vec::with_capacity((to - from) as usize);
    f.take(to - from).read_to_end(&mut data)?;
    Ok(data)
}

fn size_of(path: &Path) -> std::io::Result<u64> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::End(0))
}

/// The whole lines from `from` (a line start), or else from the last `bytes` of the file, where a cut first line is
/// dropped. A last line with no newline yet is left out.
pub fn read(path: &Path, from: Option<u64>, bytes: u64) -> std::io::Result<Range> {
    let size = size_of(path)?;
    let mut start = match from {
        Some(f) => f.min(size),
        None => size.saturating_sub(bytes),
    };
    let data = read_span(path, start, size)?;
    let mut body: &[u8] = &data;
    if from.is_none() && start > 0 {
        let Some(nl) = body.iter().position(|&b| b == b'\n') else {
            return Ok(Range { lines: vec![], start: size, size });
        };
        start += nl as u64 + 1;
        body = &body[nl + 1..];
    }
    let mut parts: Vec<&[u8]> = body.split(|&b| b == b'\n').collect();
    parts.pop(); // after the last "\n": empty, or a line still being written
    Ok(Range {
        lines: parts.into_iter().filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect(),
        start,
        size,
    })
}

/// The whole lines that end before `before` (a line start): up to `bytes` back, a cut first line dropped.
/// `start` 0 means the file's start was reached.
pub fn read_before(path: &Path, before: u64, bytes: u64) -> std::io::Result<Range> {
    read_before_max(path, before, bytes, MAX_LINE)
}

/// A line longer than this before the range (a screenshot) is skipped, not read: rows do not show it whole.
pub const MAX_LINE: u64 = 16 * 1024 * 1024;

/// `read_before` with the longest line it reads whole: a longer one gives no lines and `start` at its start.
pub fn read_before_max(path: &Path, before: u64, bytes: u64, max_line: u64) -> std::io::Result<Range> {
    let size = size_of(path)?;
    let end = before.min(size);
    let mut start = end.saturating_sub(bytes);
    let data = read_span(path, start, end)?;
    let mut body: &[u8] = &data;
    if start > 0 {
        // The last byte is the newline that ends the line before `before`: a newline that starts a whole line
        // must come before it.
        let Some(nl) = body[..body.len().saturating_sub(1)].iter().position(|&b| b == b'\n') else {
            // One line longer than `bytes` (a tool result with a screenshot): the range still moves back, to its
            // start (staying at `before` would stop older rows there for good). Read whole up to `max_line`.
            let from = line_start(path, start)?;
            if end - from > max_line {
                return Ok(Range { lines: vec![], start: from, size });
            }
            let data = read_span(path, from, end)?;
            return Ok(Range { lines: whole_lines(&data), start: from, size });
        };
        start += nl as u64 + 1;
        body = &body[nl + 1..];
    }
    Ok(Range { lines: whole_lines(body), start, size })
}

fn whole_lines(body: &[u8]) -> Vec<String> {
    body.split(|&b| b == b'\n').filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).into_owned()).collect()
}

/// The start of the line that holds byte `at`: just after the newline before it, or 0.
fn line_start(path: &Path, at: u64) -> std::io::Result<u64> {
    const STEP: u64 = 64 * 1024;
    let mut end = at;
    while end > 0 {
        let from = end.saturating_sub(STEP);
        let data = read_span(path, from, end)?;
        if let Some(nl) = data.iter().rposition(|&b| b == b'\n') {
            return Ok(from + nl as u64 + 1);
        }
        end = from;
    }
    Ok(0)
}

pub fn items(lines: &[String]) -> Vec<ConversationItem> {
    let mut p = ConversationParser::new();
    for l in lines {
        p.feed(l);
    }
    p.items()
}

/// The model named by the newest assistant line, for logs with no status line.
pub fn model(lines: &[String]) -> Option<String> {
    for l in lines.iter().rev().filter(|l| l.contains("\"model\"")) {
        let Ok(o) = serde_json::from_str::<Value>(l) else { continue };
        if !o.is_object() {
            continue;
        }
        if let Some(m) = o.get("message").and_then(|m| m.get("model")).and_then(Value::as_str) {
            if !m.is_empty() && !m.starts_with('<') {
                return Some(m.into());
            }
        }
        if let Some(m) = o.get("payload").and_then(|m| m.get("model")).and_then(Value::as_str) {
            if !m.is_empty() {
                return Some(m.into());
            }
        }
    }
    None
}
