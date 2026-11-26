//! A log whose end is filled with large lines that are not chat (an `api-request-blob`, a user line with pictures):
//! the first read steps back until a message is in, so the chat, a glance, a card, a summary and the queue still see
//! the last turn. Synthetic logs only.

use chrono::{TimeZone, Utc};
use modfeedkit::protocol::JsonLines;
use modfeedkit::{cards, feed, glance, queue, summary, tail};
use std::io::Write;
use std::path::Path;

fn user(id: &str, text: &str) -> String {
    format!(r#"{{"type":"user","uuid":"{id}","timestamp":"2026-11-03T01:40:00.000Z","message":{{"role":"user","content":"{text}"}}}}"#)
}
fn say(id: &str, text: &str) -> String {
    format!(r#"{{"type":"assistant","uuid":"{id}","timestamp":"2026-11-03T01:40:05.000Z","message":{{"role":"assistant","model":"claude-test-1","content":[{{"type":"text","text":"{text}"}}]}}}}"#)
}
/// A line the parser reads nothing from, `n` bytes of payload.
fn blob(n: usize) -> String {
    format!(r#"{{"type":"api-request-blob","sessionId":"s","blob":"{}"}}"#, "q".repeat(n))
}
/// A user line with a picture, `n` bytes of base64.
fn picture(id: &str, n: usize) -> String {
    format!(
        r#"{{"type":"user","uuid":"{id}","timestamp":"2026-11-03T01:41:00.000Z","message":{{"role":"user","content":[{{"type":"text","text":"See this"}},{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{}"}}}}]}}}}"#,
        "A".repeat(n)
    )
}
const META: [&str; 3] = [
    r#"{"type":"last-prompt","sessionId":"s","lastPrompt":"x"}"#,
    r#"{"type":"ai-title","sessionId":"s","title":"x"}"#,
    r#"{"type":"permission-mode","sessionId":"s","permissionMode":"default"}"#,
];
fn chat() -> Vec<String> {
    let mut v = vec![];
    for i in 0..5 {
        v.push(user(&format!("u{i}"), &format!("question {i}")));
        v.push(say(&format!("a{i}"), &format!("answer {i}")));
    }
    v
}
fn now() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_793_670_000, 0).unwrap()
}
fn write(path: &Path, lines: &[String], tail: &str) {
    std::fs::write(path, lines.join("\n") + "\n" + tail).unwrap();
}
fn rows(out: &JsonLines) -> Vec<String> {
    out.lines.iter().filter(|l| l.contains(r#""type":"row""#)).cloned().collect()
}
fn byte_before(path: &Path, at: u64) -> u8 {
    std::fs::read(path).unwrap()[at as usize - 1]
}

/// Chat, then blobs over the last 2 MB, then small meta lines (the shape that showed "No messages yet").
fn blob_log(path: &Path) -> Vec<String> {
    // Older lines before the chat, so the range found starts inside the file.
    let mut l: Vec<String> = (0..3).map(|_| blob(1_000_000)).collect();
    l.extend(chat());
    l.push(blob(1200));
    l.push(blob(tail::BYTES as usize + 300_000));
    l.push(r#"{"type":"api-request","sessionId":"s","request":"r"}"#.into());
    l.extend(META.iter().map(|s| s.to_string()));
    write(path, &l, "");
    l
}

#[test]
fn first_feed_steps_back_past_big_lines() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("s.jsonl");
    blob_log(&f);
    let mut out = JsonLines::new();
    let s = feed::run(Some(&f), "claude", None, None, false, &mut out, &now(), &Utc).unwrap().unwrap();
    let rs = rows(&out);
    assert!(rs.iter().any(|r| r.contains("answer 4")), "bigline feed: the last reply shows: {} rows", rs.len());
    assert!(s.start > 0 && byte_before(&f, s.start) == b'\n', "bigline feed: start is a line start, not the file's: {}", s.start);
    assert_eq!(model_of(&out).as_deref(), Some("claude-test-1"), "bigline feed: model from the reply line");

    // Same cursor: unchanged. Growth: a splice after the rows kept, at the same start.
    let c = s.cursor();
    let mut o2 = JsonLines::new();
    let s2 = feed::run(Some(&f), "claude", Some(&c), Some(&s), false, &mut o2, &now(), &Utc).unwrap().unwrap();
    assert!(o2.lines.contains(&r#"{"type":"unchanged"}"#.to_string()) && s2 == s, "bigline feed: same size, unchanged");
    let mut h = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
    h.write_all((user("u9", "one more") + "\n").as_bytes()).unwrap();
    drop(h);
    let mut o3 = JsonLines::new();
    let s3 = feed::run(Some(&f), "claude", Some(&c), Some(&s), false, &mut o3, &now(), &Utc).unwrap().unwrap();
    let splice = o3.lines.iter().find(|l| l.contains(r#""type":"splice""#)).unwrap();
    assert!(!splice.contains(r#""at":0,"#), "bigline feed: growth splices after the kept rows: {splice}");
    assert_eq!(s3.start, s.start, "bigline feed: keeps the range start while the log grows");
    assert!(rows(&o3).iter().any(|r| r.contains("one more")), "bigline feed: the new message comes");
}

fn model_of(out: &JsonLines) -> Option<String> {
    let log: serde_json::Value = serde_json::from_str(&out.lines[0]).unwrap();
    log["model"].as_str().map(String::from)
}

#[test]
fn first_feed_keeps_a_big_user_line_with_pictures() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("s.jsonl");
    let mut l = chat();
    l.push(picture("p1", tail::BYTES as usize + 100_000));
    l.extend(META.iter().map(|s| s.to_string()));
    write(&f, &l, "");
    let mut out = JsonLines::new();
    feed::run(Some(&f), "claude", None, None, false, &mut out, &now(), &Utc).unwrap();
    assert!(rows(&out).iter().any(|r| r.contains("See this")), "bigline feed: the user line with a picture shows");
}

#[test]
fn reader_limits_and_unfinished_lines() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("s.jsonl");
    // Blobs longer than the limit: nothing found, the read stops within it.
    let mut l = chat();
    for _ in 0..6 {
        l.push(blob(900));
    }
    write(&f, &l, "");
    let size = std::fs::metadata(&f).unwrap().len();
    let (r, items) = tail::read_last_in(&f, 1000, 3000, tail::items_of).unwrap();
    assert!(items.is_empty() && size - r.start <= 3000, "bigline reader: stops at the limit: {} back", size - r.start);
    assert!(byte_before(&f, r.start) == b'\n', "bigline reader: start is a line start");
    // A wide enough limit finds the chat.
    let (r2, items2) = tail::read_last_in(&f, 1000, 64 * 1024, tail::items_of).unwrap();
    assert!(tail::has_message(&items2) && r2.lines.last() == l.last(), "bigline reader: steps back to the chat");
    // A line still being written (no newline yet) after the blobs stays out, also when stepping back.
    write(&f, &l, &blob(5000));
    let (r3, items3) = tail::read_last_in(&f, 1000, 64 * 1024, tail::items_of).unwrap();
    assert!(tail::has_message(&items3) && r3.lines.last() == l.last(), "bigline reader: the unfinished line is left out");
    assert_eq!(r3.size, std::fs::metadata(&f).unwrap().len(), "bigline reader: size of the first read");
    // A normal log: one read, as before.
    write(&f, &chat(), "");
    let (r4, _) = tail::read_last_in(&f, 1000, 64 * 1024, tail::items_of).unwrap();
    assert_eq!(r4, tail::read(&f, None, 1000).unwrap(), "bigline reader: a tail with a message is the plain read");
}

fn tool(i: usize) -> [String; 2] {
    [
        format!(r#"{{"type":"assistant","uuid":"t{i}","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"c{i}","name":"Bash","input":{{"command":"echo {i}"}}}}]}}}}"#),
        format!(r#"{{"type":"user","uuid":"r{i}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"c{i}","content":"{}"}}]}}}}"#, "o".repeat(2000)),
    ]
}

#[test]
fn a_long_turn_of_tool_calls_reads_as_before() {
    // More than 2 MB of tool calls with no text after the chat: the tail has rows, so it is the plain read, and growth
    // keeps the range start (no new range).
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("s.jsonl");
    let mut l = chat();
    for i in 0..1200 {
        l.extend(tool(i));
    }
    write(&f, &l, "");
    let (r, items) = tail::read_last(&f, tail::items_of).unwrap();
    assert_eq!(r, tail::read(&f, None, tail::BYTES).unwrap(), "bigline tools: the plain 2 MB read");
    assert!(!items.is_empty() && !tail::has_message(&items), "bigline tools: tool rows, no message");
    let mut out = JsonLines::new();
    let mut s = feed::run(Some(&f), "claude", None, None, false, &mut out, &now(), &Utc).unwrap().unwrap();
    let start = s.start;
    for i in 2000..2005 {
        let mut h = std::fs::OpenOptions::new().append(true).open(&f).unwrap();
        h.write_all((tool(i).join("\n") + "\n").as_bytes()).unwrap();
        drop(h);
        let mut o = JsonLines::new();
        s = feed::run(Some(&f), "claude", Some(&s.cursor()), Some(&s), false, &mut o, &now(), &Utc).unwrap().unwrap();
        // The calls fold into one row of chips, so the splice is at 0; the range start is what must stay.
        assert_eq!(s.start, start, "bigline tools: growth keeps the range start");
    }
}

#[test]
fn older_skips_chunks_with_no_rows() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("s.jsonl");
    let mut l = chat();
    for _ in 0..3 {
        l.push(blob(tail::BYTES as usize / 2));
    }
    l.push(user("u8", "latest"));
    write(&f, &l, "");
    let before = std::fs::metadata(&f).unwrap().len() - l.last().unwrap().len() as u64 - 1;
    let mut out = JsonLines::new();
    feed::older(&f, before, false, &mut out, &now(), &Utc).unwrap();
    assert!(rows(&out).iter().any(|r| r.contains("answer 4")), "bigline older: rows from before the blobs: {}", out.lines[0]);
    assert!(out.lines[0].contains(r#""start":0"#), "bigline older: reaches the file's start: {}", out.lines[0]);
}

#[test]
fn callers_see_the_last_turn() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("s.jsonl");
    let mut l = chat();
    l.insert(9, r#"{"type":"queue-operation","operation":"enqueue","content":"QA"}"#.into());
    l.push(blob(tail::BYTES as usize + 300_000));
    l.extend(META.iter().map(|s| s.to_string()));
    write(&f, &l, "");
    let facts = glance::facts_at(&f, "claude");
    assert!(facts != glance::Facts::default(), "bigline glance: facts of the last turn: {facts:?}");
    assert_eq!(summary::input_at(&f, "claude").unwrap().map(|x| x.1), summary::input(&tail::items(&chat()), "claude").map(|x| x.1), "bigline summary: the last turn");
    assert!(cards::log_items(&f).iter().any(|it| matches!(&it.kind, herdr_core::ItemKind::AssistantText(t) if t == "answer 4")), "bigline cards: the log's items");
    let q = queue::log_at(Some(&f));
    assert_eq!(q, queue::log_of(l.iter().map(String::as_str)), "bigline queue: the queue line is read");
    assert_ne!(q, Default::default(), "bigline queue: not empty");
}
