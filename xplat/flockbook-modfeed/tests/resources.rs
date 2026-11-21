//! The resources command: what an agent pointed the human to, collected from its log (contract: herdr-app/docs/protocol.md, `res`).

use chrono::{TimeZone, Utc};
use modfeedkit::{resources, rows, tail};
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;

fn ts(s: u32) -> String {
    format!("2026-11-03T01:{:02}:{:02}.000Z", 40 + s / 60, s % 60)
}
fn ms(s: u32) -> i64 {
    Utc.with_ymd_and_hms(2026, 11, 3, 1, 40, 0).unwrap().timestamp_millis() + s as i64 * 1000
}
fn line(v: Value) -> String {
    v.to_string()
}
fn user(id: &str, t: u32, text: &str) -> String {
    line(json!({"type":"user","uuid":id,"timestamp":ts(t),"message":{"role":"user","content":text}}))
}
fn say(id: &str, t: u32, text: &str) -> String {
    line(json!({"type":"assistant","uuid":id,"timestamp":ts(t),"message":{"role":"assistant","content":[{"type":"text","text":text}]}}))
}
fn tool(id: &str, t: u32, call: &str, name: &str, input: Value) -> String {
    line(json!({"type":"assistant","uuid":id,"timestamp":ts(t),"message":{"role":"assistant","content":[{"type":"tool_use","id":call,"name":name,"input":input}]}}))
}
fn result(id: &str, t: u32, call: &str, out: &str, error: bool) -> String {
    line(json!({"type":"user","uuid":id,"timestamp":ts(t),"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":call,"content":out,"is_error":error}]}}))
}

fn convo() -> Vec<String> {
    vec![
        user("u1", 0, "Look at https://pasted.example.com/x and ~/pasted/a.md"),
        tool("a1", 2, "c1", "Bash", json!({"command": "curl https://tool.example.com"})),
        result("r1", 3, "c1", "see https://tool-output.example.com/y", false),
        say("a2", 5, "Plan: ~/plans/plan.md and PR https://github.com/o/r/pull/12 and http://localhost:5173/"),
        tool("a3", 6, "c2", "Artifact", json!({"action": "publish", "file_path": "/tmp/p.html", "title": "Launch plan"})),
        result("r3", 7, "c2", "Published: https://claude.ai/code/artifact/0f3a-11", false),
        tool("a4", 8, "c3", "Artifact", json!({"action": "publish", "file_path": "/tmp/q.html", "title": "Broken"})),
        result("r4", 9, "c3", "Failed: https://claude.ai/code/artifact/dead-1", true),
        tool("a5", 10, "c4", "Artifact", json!({"action": "read", "url": "https://claude.ai/code/artifact/read-1"})),
        result("r5", 11, "c4", "Read https://claude.ai/code/artifact/read-1", false),
        say("a6", 20, "Again ~/plans/plan.md, files https://github.com/o/r/pull/12/files, app localhost:5173/settings, docs https://Docs.Example.com/guide/#intro, CI https://github.com/o/r/actions/runs/1870"),
        user("u2", 30, "thanks"),
        say("a7", 40, "Also https://docs.example.com/guide and the issue github.com/o/r/issues/7"),
    ]
}

fn by_kind<'a>(rs: &'a [resources::Resource], kind: &str) -> Vec<&'a resources::Resource> {
    rs.iter().filter(|r| r.kind == kind).collect()
}

#[test]
fn collects_from_replies_and_artifact_results_only() {
    let items = tail::items(&convo());
    let rs = resources::collect(&items, Some(Path::new("/home/me")));
    let targets: Vec<&str> = rs.iter().map(|r| r.target.as_str()).collect();
    assert!(!targets.iter().any(|t| t.contains("pasted") || t.contains("tool")), "user text and tool output are not collected: {targets:?}");
    assert!(!targets.iter().any(|t| t.contains("dead-1")), "a failed Artifact call is not collected");
    assert!(!targets.iter().any(|t| t.contains("read-1")), "only publish results are collected");

    let doc = by_kind(&rs, "doc");
    assert_eq!(doc.len(), 1, "one item for the same path: {doc:?}");
    assert_eq!(doc[0].target, "/home/me/plans/plan.md", "~/ is the home, expanded");
    assert_eq!(doc[0].mentions, 2);
    assert_eq!((doc[0].first_ts, doc[0].last_ts), (Some(ms(5)), Some(ms(20))));
    assert_eq!(doc[0].row, "a6", "the row of the last mention");
    assert_eq!(doc[0].title, "plan.md", "no file: the file name");
    assert_eq!(doc[0].detail, "~/plans");
    assert_eq!(doc[0].exists, Some(false));

    let pr = by_kind(&rs, "pr");
    assert_eq!(pr.len(), 1, "/files is the same PR: {pr:?}");
    assert_eq!((pr[0].target.as_str(), pr[0].title.as_str(), pr[0].detail.as_str(), pr[0].mentions), ("https://github.com/o/r/pull/12", "o/r#12", "o/r", 2));
    assert_eq!(pr[0].exists, None, "exists: local files only");

    let local = by_kind(&rs, "local");
    assert_eq!(local.len(), 1, "one per host:port: {local:?}");
    assert_eq!(local[0].target, "http://localhost:5173/settings", "the newest URL");
    assert_eq!(local[0].title, "localhost:5173");

    let art = by_kind(&rs, "artifact");
    assert_eq!(art.len(), 1, "{art:?}");
    assert_eq!((art[0].target.as_str(), art[0].title.as_str(), art[0].row.as_str()), ("https://claude.ai/code/artifact/0f3a-11", "Launch plan", "a3"));

    let link = by_kind(&rs, "link");
    assert_eq!(link.len(), 1, "host case, #fragment and the trailing / are the same link: {link:?}");
    assert_eq!(link[0].mentions, 2);
    assert_eq!(link[0].title, "docs.example.com/guide");
    assert_eq!(link[0].detail, "docs.example.com");

    assert_eq!(by_kind(&rs, "ci")[0].title, "CI run 1870");
    assert_eq!(by_kind(&rs, "issue")[0].title, "o/r#7");

    // Newest first.
    let last: Vec<Option<i64>> = rs.iter().map(|r| r.last_ts).collect();
    let mut sorted = last.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(last, sorted);
    assert_eq!(rs[0].last_ts, Some(ms(40)));

    // Each row is a feed row's id; each id is a hash of its key and unique.
    let ids: Vec<String> = rows::rows(&items, &Utc::now(), &Utc).into_iter().map(|r| r.id).collect();
    for r in &rs {
        assert!(ids.contains(&r.row), "{} is not a row id: {ids:?}", r.row);
    }
    let mut u: Vec<&str> = rs.iter().map(|r| r.id.as_str()).collect();
    u.sort();
    u.dedup();
    assert_eq!(u.len(), rs.len());
}

#[test]
fn the_line_is_a_res_line() {
    let items = tail::items(&[say("a1", 0, "see https://github.com/o/r/pull/3")]);
    let v = serde_json::to_value(&resources::collect(&items, None)[0]).unwrap();
    assert_eq!(v["type"], "res");
    for k in ["id", "kind", "target", "title", "detail", "row", "first_ts", "last_ts", "mentions"] {
        assert!(v.get(k).is_some(), "{k} in {v}");
    }
    assert!(v.get("exists").is_none(), "no exists for a URL");
}

#[test]
fn titles_from_the_files() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    std::fs::write(d.join("plan.md"), "intro\n\n# The plan\n\nbody").unwrap();
    std::fs::write(d.join("r.html"), "<html><head><title> Weekly report </title></head></html>").unwrap();
    std::fs::write(d.join("plain.md"), "no heading").unwrap();
    std::fs::create_dir(d.join("folder.md")).unwrap();
    let p = |n: &str| d.join(n).to_string_lossy().to_string();
    let text = format!("{} {} {} {} {}", p("plan.md"), p("r.html"), p("plain.md"), p("folder.md"), p("gone.pdf"));
    let rs = resources::collect(&tail::items(&[say("a1", 0, &text)]), None);
    let get = |n: &str| rs.iter().find(|r| r.target == p(n)).unwrap_or_else(|| panic!("{n} in {rs:?}"));
    assert_eq!((get("plan.md").title.as_str(), get("plan.md").exists), ("The plan", Some(true)));
    assert_eq!(get("r.html").title, "Weekly report");
    assert_eq!(get("plain.md").title, "plain.md");
    assert_eq!((get("folder.md").title.as_str(), get("folder.md").exists), ("folder.md", Some(false)), "a folder is not a file");
    assert_eq!(get("gone.pdf").exists, Some(false));
}

fn write(path: &Path, lines: &[String], append: bool) {
    let mut f = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(path).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
}

#[test]
fn since_a_cursor_only_what_is_newer() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &[say("a1", 0, "see https://a.example.com"), say("a2", 5, "and https://github.com/o/r/pull/1")], false);
    let first = resources::answer(&log, None, None).unwrap();
    assert!(first.full);
    assert_eq!(first.items.len(), 2);
    assert!(first.cursor.starts_with("r1."), "{}", first.cursor);

    let same = resources::answer(&log, Some(&first.cursor), None).unwrap();
    assert_eq!((same.full, same.items.len()), (false, 0), "nothing new");
    assert_eq!(same.cursor, first.cursor);

    write(&log, &[say("a3", 9, "PR https://github.com/o/r/pull/1 and https://new.example.com")], true);
    let more = resources::answer(&log, Some(&first.cursor), None).unwrap();
    assert!(!more.full);
    let t: Vec<&str> = more.items.iter().map(|r| r.target.as_str()).collect();
    assert_eq!(t, ["https://new.example.com", "https://github.com/o/r/pull/1"], "the new item and the one mentioned again, newest first (same time: the later mention first)");
    assert_eq!(more.items[1].mentions, 2);

    // Another log, a log that got shorter, or a bad cursor: all of it again.
    write(&log, &[say("a1", 0, "see https://a.example.com")], false);
    assert!(resources::answer(&log, Some(&more.cursor), None).unwrap().full);
    assert!(resources::answer(&log, Some("junk"), None).unwrap().full);
    let other = dir.path().join("o.jsonl");
    write(&other, &[say("a1", 0, "see https://a.example.com")], false);
    let c = resources::answer(&log, None, None).unwrap().cursor;
    let o = resources::answer(&other, Some(&c), None).unwrap();
    assert!(o.full);
    assert_eq!(o.items.len(), 1);
}

/// Shared with the Swift side (next release): Tests/Fixtures/resources/.
#[test]
fn the_shared_fixture() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Tests/Fixtures/resources");
    let lines: Vec<String> = std::fs::read_to_string(dir.join("conversation.jsonl")).unwrap().lines().map(String::from).collect();
    let want: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    let got: Vec<Value> = resources::collect(&tail::items(&lines), Some(Path::new("/home/me")))
        .iter()
        .map(|r| json!({"kind": r.kind, "target": r.target, "title": r.title, "row": r.row, "mentions": r.mentions}))
        .collect();
    assert_eq!(Value::Array(got), want);
}
