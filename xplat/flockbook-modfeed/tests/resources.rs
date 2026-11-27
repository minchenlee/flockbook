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
    assert_eq!(doc[0].row, "a6-0", "the row of the last mention (the parser's item id)");
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
    assert_eq!((art[0].target.as_str(), art[0].title.as_str(), art[0].row.as_str()), ("https://claude.ai/code/artifact/0f3a-11", "Launch plan", "tools-a3-0"));

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

// Unix only: a doc is a path from "/" or "~/" (detect), and a temp folder on Windows is "C:\…".
#[test]
#[cfg(unix)]
fn titles_from_the_files() {
    let dir = tempfile::tempdir().unwrap();
    // The folder's real path: a doc is named by it (/var is /private/var on macOS).
    let d = &dir.path().canonicalize().unwrap();
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
#[cfg(unix)]
fn a_doc_opens_only_when_its_real_file_is_a_kind_flockbook_opens() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().canonicalize().unwrap();
    std::fs::write(d.join("notes.txt"), "# Not a doc").unwrap();
    std::fs::write(d.join("b.md"), "# Real").unwrap();
    std::fs::create_dir(d.join("x.md")).unwrap();
    symlink(d.join("notes.txt"), d.join("preview.md")).unwrap();
    symlink(d.join("b.md"), d.join("a.md")).unwrap();
    symlink(d.join("x.md"), d.join("dir-link.md")).unwrap();
    assert!(resources::openable(&d.join("preview.md")).is_err(), "a .md link to a .txt does not open");
    assert_eq!(resources::openable(&d.join("a.md")), Ok(d.join("b.md")), "a .md link to a .md opens its real file");
    assert!(resources::openable(&d.join("x.md")).is_err(), "a folder does not open");
    assert!(resources::openable(&d.join("dir-link.md")).is_err(), "a link to a folder does not open");
    assert_eq!(resources::openable(&d.join("gone.md")), Err("File not found."));
    let p = |n: &str| d.join(n).to_string_lossy().to_string();
    let text = format!("{} {}", p("preview.md"), p("a.md"));
    let rs = resources::collect(&tail::items(&[say("a1", 0, &text)]), None);
    let get = |n: &str| rs.iter().find(|r| r.target == p(n)).unwrap_or_else(|| panic!("{n} in {rs:?}"));
    assert_eq!(get("preview.md").exists, Some(false), "the link to a .txt is not shown as there");
    assert_eq!((get("a.md").title.as_str(), get("a.md").exists), ("Real", Some(true)));
}

#[test]
fn since_a_cursor_only_what_is_newer() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &[say("a1", 0, "see https://a.example.com"), say("a2", 5, "and https://github.com/o/r/pull/1")], false);
    let first = resources::answer(&log, None, None).unwrap();
    assert!(first.full);
    assert_eq!(first.items.len(), 2);
    assert!(first.cursor.starts_with("r4."), "{}", first.cursor);

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
    // An "r1." or "r2." cursor of this very log (old Codex ids; a log cut at 64 MB): all of it again, not "nothing new".
    let now = resources::answer(&log, None, None).unwrap().cursor;
    let p: Vec<&str> = now.split('.').collect();
    for v in ["r1", "r2", "r3"] {
        let c = format!("{v}.{}.{}.{}", p[1], p[2], p[3]);
        let old = resources::answer(&log, Some(&c), None).unwrap();
        assert!(old.full && !old.items.is_empty(), "an {v} cursor gets everything again");
    }
}

#[test]
fn the_whole_log_is_read_not_only_its_last_2_mb() {
    // A long session: what was pointed to early is still listed, also after a cursor (hg-res-older-log).
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    let mut lines = vec![
        say("a1", 0, "Report: https://early.example.com/report"),
        tool("a2", 1, "c1", "Artifact", json!({"action": "publish", "title": "Early page"})),
        result("r2", 2, "c1", "Published https://claude.ai/code/artifact/early-1", false),
    ];
    let filler = "x".repeat(1000);
    for i in 0..3000 {
        lines.push(user(&format!("u{i}"), 10, &filler));
    }
    lines.push(say("a3", 20, "Again https://early.example.com/report"));
    write(&log, &lines, false);
    assert!(std::fs::metadata(&log).unwrap().len() > tail::BYTES + 512 * 1024);
    let first = resources::answer(&log, None, None).unwrap();
    let get = |rs: &[resources::Resource], t: &str| rs.iter().find(|r| r.target == t).map(|r| r.mentions);
    assert_eq!(get(&first.items, "https://early.example.com/report"), Some(2), "{:?}", first.items);
    assert!(first.items.iter().any(|r| r.target.ends_with("/early-1")), "{:?}", first.items);

    write(&log, &[say("a4", 30, "Once more https://early.example.com/report")], true);
    let more = resources::answer(&log, Some(&first.cursor), None).unwrap();
    assert!(!more.full);
    assert_eq!(get(&more.items, "https://early.example.com/report"), Some(3), "mentions counted over the whole log: {:?}", more.items);
}

#[test]
fn a_codex_reply_has_the_same_row_id_in_resources_and_in_the_feed() {
    // A Codex log over 2 MB: the feed reads from its last 2 MB, resources from the start. Codex lines have no ids,
    // so a reply must get the same one both ways ("Show in chat" finds that row).
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("rollout.jsonl");
    let line = |t: u32, payload: Value| json!({"timestamp": ts(t), "type": "response_item", "payload": payload}).to_string();
    let msg = |t: u32, role: &str, text: &str| {
        let kind = if role == "user" { "input_text" } else { "output_text" };
        line(t, json!({"type": "message", "role": role, "content": [{"type": kind, "text": text}]}))
    };
    let filler = "x".repeat(1000);
    let mut lines: Vec<String> = (0..3000).map(|_| msg(1, "user", &filler)).collect();
    lines.push(msg(2, "user", "where is the report?"));
    lines.push(msg(3, "assistant", "Here: https://report.example.com/r"));
    write(&log, &lines, false);
    assert!(std::fs::metadata(&log).unwrap().len() > tail::BYTES + 512 * 1024);
    let a = resources::answer(&log, None, None).unwrap();
    let row = &a.items.iter().find(|r| r.target == "https://report.example.com/r").expect("found").row;
    let mut out = modfeedkit::protocol::JsonLines::new();
    let now = Utc.timestamp_opt(1_793_670_000, 0).unwrap();
    modfeedkit::feed::run(Some(&log), "codex", None, None, false, &mut out, &now, &Utc).unwrap();
    let ids: Vec<String> = out.lines.iter().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["type"] == "row").map(|v| v["id"].as_str().unwrap_or("").to_string()).collect();
    assert!(ids.contains(row), "resources row {row} not among the feed's last rows {:?}", &ids[ids.len().saturating_sub(3)..]);
    // The rows read with --before have ids of their own too: none is also in the tail.
    let start = out.lines.iter().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find(|v| v["type"] == "log").unwrap()["start"].as_u64().unwrap();
    let mut older = modfeedkit::protocol::JsonLines::new();
    modfeedkit::feed::older(&log, start, false, &mut older, &now, &Utc).unwrap();
    let old: Vec<String> = older.lines.iter().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["type"] == "row").map(|v| v["id"].as_str().unwrap_or("").to_string()).collect();
    assert!(!old.is_empty());
    assert!(old.iter().all(|i| !ids.contains(i)), "an older row has a tail row's id");
}

#[test]
fn a_github_url_with_an_odd_path_is_a_link_not_a_panic() {
    let rs = resources::collect(&tail::items(&[say("a1", 0, "see https://github.com?x=/o/r/pull/1 and https://github.com/o/r/pull/2")]), None);
    assert!(rs.iter().all(|r| r.kind == "link" || r.target == "https://github.com/o/r/pull/2"), "{rs:?}");
}

#[test]
fn a_later_call_in_the_same_run_of_tools_is_not_missed() {
    // A run of tool calls is one row with its first call's time: a second publish there has the cursor's time.
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    let publish = |n: u32, id: &str| {
        vec![
            tool(&format!("a{n}"), 1, &format!("c{n}"), "Artifact", json!({"action": "publish", "title": format!("P{n}")})),
            result(&format!("r{n}"), 2, &format!("c{n}"), &format!("Published https://claude.ai/code/artifact/{id}"), false),
        ]
    };
    write(&log, &publish(1, "one"), false);
    let first = resources::answer(&log, None, None).unwrap();
    assert_eq!(first.items.len(), 1);
    write(&log, &publish(2, "two"), true);
    let more = resources::answer(&log, Some(&first.cursor), None).unwrap();
    assert!(more.items.iter().any(|r| r.target.ends_with("/two")), "{:?}", more.items);
}

#[test]
fn an_artifact_without_a_title_takes_its_pages_title() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("p.html");
    std::fs::write(&page, "<title>Weekly plan</title>").unwrap();
    let lines = [
        tool("a1", 1, "c1", "Artifact", json!({"action": "publish", "file_path": page.to_string_lossy()})),
        result("r1", 2, "c1", "Published /x/p.html at https://claude.ai/artifact/abc", false),
        tool("a2", 3, "c2", "Artifact", json!({"action": "publish", "file_path": "/nonexistent/q.html"})),
        result("r2", 4, "c2", "Published at https://claude.ai/artifact/def", false),
    ];
    let rs = resources::collect(&tail::items(&lines), None);
    let t = |id: &str| rs.iter().find(|r| r.target.ends_with(id)).map(|r| r.title.clone());
    assert_eq!((t("abc").as_deref(), t("def").as_deref()), (Some("Weekly plan"), Some("Artifact")));
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


// Commands: a one-line shell command in a fenced block of an assistant reply (`cmd`).
fn cmds(text: &str) -> Vec<(String, String)> {
    resources::commands(text).into_iter().map(|c| (c.title, c.cmd)).collect()
}
fn one_cmd(text: &str) -> Option<String> {
    let c = resources::commands(text);
    assert!(c.len() <= 1, "{text}: {c:?}");
    c.into_iter().next().map(|c| c.cmd)
}

#[test]
fn a_command_block_in_each_shell_language_is_collected() {
    for lang in ["", "sh", "bash", "zsh", "shell", "console", "Bash", "sh title=x"] {
        let t = format!("Try:\n\n```{lang}\nnpm test\n```\n");
        assert_eq!(one_cmd(&t).as_deref(), Some("npm test"), "{lang:?}");
    }
    assert_eq!(one_cmd("```\n$ cargo build --release\n```").as_deref(), Some("cargo build --release"), "the leading \"$ \" goes");
    assert_eq!(one_cmd("~~~sh\nls -la\n~~~").as_deref(), Some("ls -la"), "a tilde fence");
    assert_eq!(one_cmd("- step:\n\n  ```sh\n  make\n  ```").as_deref(), Some("make"), "in a list item");
}

#[test]
fn a_command_on_several_lines_joined_by_backslashes_is_one() {
    let t = "```bash\ncd ~/x && \\\n  ./run.sh --fast \\\n  demo/README.md\n```";
    assert_eq!(one_cmd(t).as_deref(), Some("cd ~/x && \\\n  ./run.sh --fast \\\n  demo/README.md"), "kept as written, so it pastes as it was");
}

#[test]
fn not_commands() {
    for (why, t) in [
        ("a script: two commands", "```sh\nnpm ci\nnpm test\n```"),
        ("output after the command", "```console\n$ ls\nREADME.md\n```"),
        ("text", "```text\nnpm test\n```"),
        ("a log", "```log\nerror: x\n```"),
        ("json", "```json\n{\"a\": 1}\n```"),
        ("python", "```python\nprint(1)\n```"),
        ("inline code", "Run \x60npm test\x60 now."),
        ("indented code", "Run:\n\n    npm test\n"),
        ("empty", "```sh\n\n```"),
        ("a comment only", "```sh\n# npm test\n```"),
        ("a backslash on the last line", "```sh\nnpm test \\\n```"),
    ] {
        assert!(cmds(t).is_empty(), "{why}: {:?}", cmds(t));
    }
    let long = format!("```sh\necho {}\n```", "a".repeat(400));
    assert!(cmds(&long).is_empty(), "over 400 characters");
    let fits = format!("```sh\necho {}\n```", "a".repeat(395));
    assert_eq!(cmds(&fits).len(), 1, "400 is fine");
}

#[test]
fn a_commands_title_is_the_text_before_it() {
    let t = "Done.\n\n試用（一行）：\n\n```\ncd ~/x && ./run demo/README.md\n```\n\nThen:\n\n```sh\nmake check\n```";
    assert_eq!(cmds(t), vec![("試用（一行）".to_string(), "cd ~/x && ./run demo/README.md".to_string()), ("Then".to_string(), "make check".to_string())]);
    assert_eq!(cmds("Build it with this:\n```sh\nmake\n```")[0].0, "Build it with this", "a block right after the line");
    assert_eq!(cmds("First line.\nSecond line, the one that says it:\n\n```sh\nmake\n```")[0].0, "Second line, the one that says it", "the paragraph's last line");
    let long = format!("{}:\n\n```sh\nmake\n```", "word ".repeat(40));
    let title = &cmds(&long)[0].0;
    assert!(title.chars().count() <= 80 && title.ends_with('…'), "{title}");
    assert_eq!(cmds("```sh\nnpm run build -- --mode production --watch\n```")[0].0, "npm run build -- --mode production --watch", "no text before it: the command");
    assert_eq!(cmds("# Heading\n\n- a list\n\n```sh\nmake\n```")[0].0, "make", "a list before it is not its title");
    let head = cmds(&format!("```sh\nnpm {}\n```", "x".repeat(100)))[0].0.clone();
    assert!(head.starts_with("npm xxx") && head.chars().count() <= 60 && head.ends_with('…'), "{head}");
}

#[test]
fn commands_are_collected_from_replies_once_per_command() {
    let lines = [
        user("u1", 0, "```sh\nrm -rf /tmp/pasted\n```"),
        say("a1", 1, "Run:\n\n```sh\nnpm test\n```\n\nSee https://x.com/a"),
        say("a2", 5, "Again:\n\n```bash\n  npm test  \n```"),
    ];
    let rs = resources::collect(&tail::items(&lines), None);
    let c: Vec<_> = rs.iter().filter(|r| r.kind == "cmd").collect();
    assert_eq!(c.len(), 1, "the same command once; user text is not collected: {rs:?}");
    assert_eq!((c[0].target.as_str(), c[0].title.as_str(), c[0].detail.as_str(), c[0].mentions), ("npm test", "Again", "npm test", 2));
    assert_eq!((c[0].row.as_str(), c[0].last_ts), ("a2-0", Some(ms(5))));
    assert_eq!(c[0].exists, None);
    assert_eq!(c[0].id, modfeedkit::protocol::fnv1a("cmd:npm test"), "the hash of its key, as the other kinds");
    let json = serde_json::to_value(c[0]).unwrap();
    assert_eq!(json["kind"], "cmd");
    // A command on several lines: the detail is one line.
    let rs = resources::collect(&tail::items(&[say("a1", 1, "```sh\nmake \\\n  all\n```")]), None);
    assert_eq!(rs[0].detail, "make all");
}


// ---- Only the new lines are parsed (hg-res-incremental) ----

const HOME: &str = "/home/me";

/// An assistant line of message `mid` (Claude Code writes each block of one message as its own line).
fn block(id: &str, t: u32, mid: &str, content: Value) -> String {
    line(json!({"type":"assistant","uuid":id,"timestamp":ts(t),"message":{"id":mid,"role":"assistant","content":[content]}}))
}

/// A Claude log with what a restart point must not cut: a run of tools, an Artifact call and its result apart,
/// thinking before a tool call of the same message, a message in several lines, and a long stretch of work
/// with no line from the person.
fn claude_mix() -> Vec<String> {
    let mut l = vec![
        user("u1", 0, "Start"),
        say("a1", 1, "Plan in ~/plans/p.md and https://a.example.com/x"),
        block("a2", 2, "m2", json!({"type":"thinking","thinking":"I will look first","signature":"s"})),
        block("a3", 2, "m2", json!({"type":"tool_use","id":"c1","name":"Bash","input":{"command":"ls"}})),
        result("r1", 3, "c1", "https://tool.example.com", false),
        tool("a4", 4, "c2", "Bash", json!({"command": "pwd"})),
        result("r2", 5, "c2", "/w", false),
        tool("a5", 6, "c3", "Artifact", json!({"action": "publish", "file_path": "/tmp/none.html", "title": "First page"})),
        say("a6", 7, "Waiting for the publish; see https://b.example.com"),
        result("r3", 8, "c3", "Published https://claude.ai/code/artifact/one-1", false),
        block("a7", 9, "m7", json!({"type":"text","text":"Two parts: https://c.example.com"})),
        block("a8", 9, "m7", json!({"type":"text","text":"and PR https://github.com/o/r/pull/3"})),
        user("u2", 10, "go on alone"),
    ];
    // Long autonomous work: no user line, tool calls, results and replies.
    for i in 0..30u32 {
        let t = 11 + i * 3;
        l.push(tool(&format!("w{i}"), t, &format!("k{i}"), "Bash", json!({"command": format!("step {i}")})));
        l.push(result(&format!("x{i}"), t + 1, &format!("k{i}"), "ok", false));
        if i % 3 == 0 {
            l.push(say(&format!("s{i}"), t + 2, &format!("Step {i}: https://step{}.example.com and again https://a.example.com/x", i % 4)));
        }
        if i % 7 == 0 {
            l.push(tool(&format!("p{i}"), t + 2, &format!("q{i}"), "Artifact", json!({"action": "publish", "title": format!("Page {i}")})));
            l.push(result(&format!("y{i}"), t + 2, &format!("q{i}"), &format!("Published https://claude.ai/code/artifact/p-{i}"), false));
        }
    }
    l.push(say("z1", 200, "Done: ~/plans/p.md, https://claude.ai/code/artifact/one-1"));
    l
}

fn codex_mix() -> Vec<String> {
    let line = |t: u32, payload: Value| json!({"timestamp": ts(t), "type": "response_item", "payload": payload}).to_string();
    let msg = |t: u32, role: &str, text: &str| {
        let kind = if role == "user" { "input_text" } else { "output_text" };
        line(t, json!({"type": "message", "role": role, "content": [{"type": kind, "text": text}]}))
    };
    let mut l = vec![msg(0, "user", "Fix it"), msg(1, "assistant", "See https://a.example.com/x and ~/plans/c.md")];
    for i in 0..20u32 {
        let t = 2 + i * 3;
        l.push(line(t, json!({"type": "function_call", "call_id": format!("c{i}"), "name": "exec_command", "arguments": "{\"cmd\":\"cargo test\"}"})));
        l.push(line(t + 1, json!({"type": "function_call_output", "call_id": format!("c{i}"), "output": "Process exited with code 0"})));
        if i % 2 == 0 {
            l.push(msg(t + 2, "assistant", &format!("Step {i}: https://step{}.example.com", i % 3)));
        }
    }
    l.push(msg(90, "user", "thanks"));
    l.push(msg(91, "assistant", "Again https://a.example.com/x"));
    l
}

fn values(rs: &[resources::Resource]) -> Vec<Value> {
    rs.iter().map(|r| serde_json::to_value(r).unwrap()).collect()
}

/// What reading the whole log again gives.
fn whole_log(log: &Path) -> Vec<Value> {
    values(&resources::answer(log, None, Some(Path::new(HOME))).unwrap().items)
}

/// One `resources` call with a kept state: the items, the state to keep, and where parsing started.
fn kept(log: &Path, k: Option<&resources::Kept>) -> (Vec<Value>, Option<resources::Kept>, u64) {
    let (a, next, from) = resources::answer_kept(log, None, Some(Path::new(HOME)), k, true).unwrap();
    (values(&a.items), next, from)
}

fn every_cut(lines: &[String], what: &str) {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    let mut later = 0;
    for k in 1..lines.len() {
        write(&log, &lines[..k], false);
        let (first, s, _) = kept(&log, None);
        assert_eq!(first, whole_log(&log), "{what}: the first {k} lines");
        write(&log, &lines[k..], true);
        let (got, _, from) = kept(&log, s.as_ref());
        assert_eq!(got, whole_log(&log), "{what}: cut after line {k}");
        if from > 0 {
            later += 1;
        }
    }
    assert!(later * 2 > lines.len(), "{what}: parsing started after the log's start for only {later} of {} cuts", lines.len());
}

#[test]
fn cut_at_every_line_the_same_as_the_whole_log() {
    every_cut(&claude_mix(), "claude");
    every_cut(&codex_mix(), "codex");
}

fn line_by_line(lines: &[String], what: &str) -> Vec<u64> {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &[], false);
    let (mut s, mut froms) = (None, vec![]);
    for (i, l) in lines.iter().enumerate() {
        write(&log, std::slice::from_ref(l), true);
        let (got, next, from) = kept(&log, s.as_ref());
        assert_eq!(got, whole_log(&log), "{what}: after line {i}");
        froms.push(from);
        s = next;
    }
    assert!(froms.windows(2).all(|w| w[0] <= w[1]), "{what}: the start only moves on: {froms:?}");
    froms
}

#[test]
fn a_line_at_a_time_the_start_moves_on_also_with_no_user_line() {
    let lines = claude_mix();
    let froms = line_by_line(&lines, "claude");
    // The last user line ("u2") is long before the end: the start moved past it, in the stretch with no user line.
    let u2: u64 = lines.iter().take_while(|l| !l.contains("\"u2\"")).map(|l| l.len() as u64 + 1).sum();
    assert!(*froms.last().unwrap() > u2 + 2000, "start {} not past the last user line at {u2}", froms.last().unwrap());
    let froms = line_by_line(&codex_mix(), "codex");
    assert!(*froms.last().unwrap() > 0);
}

#[test]
fn a_line_still_being_written_is_read_once_it_is_whole() {
    let lines = claude_mix();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    for k in [5usize, 9, 20, 40] {
        let half = &lines[k][..lines[k].len() / 2];
        write(&log, &lines[..k], false);
        std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(half.as_bytes()).unwrap();
        let (got, s, _) = kept(&log, None);
        assert_eq!(got, whole_log(&log), "half of line {k}");
        std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(format!("{}\n", &lines[k][lines[k].len() / 2..]).as_bytes()).unwrap();
        write(&log, &lines[k + 1..], true);
        let (got, _, _) = kept(&log, s.as_ref());
        assert_eq!(got, whole_log(&log), "line {k} finished");
    }
}

#[test]
fn a_changed_shorter_or_other_log_is_read_whole() {
    let lines = claude_mix();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &lines, false);
    let (_, s, _) = kept(&log, None);
    let s = s.expect("a state");
    assert!(s.at > 0);
    // Rewritten before the start (same length; at its start, just before the start, and in the middle): read from 0,
    // and the new text counts.
    let text = std::fs::read_to_string(&log).unwrap();
    let cut = s.at as usize;
    for (at, what) in [(text.find("https://a.example.com/x").unwrap(), "head"), (text[..cut].rfind("https://").unwrap(), "near"), (text[..cut / 2].rfind("https://").unwrap(), "middle")] {
        let mut changed = text.clone();
        changed.replace_range(at + 8..at + 9, "Z");
        std::fs::write(&log, &changed).unwrap();
        write(&log, &[say("n1", 300, "more")], true);
        let (got, _, from) = kept(&log, Some(&s));
        assert_eq!(from, 0, "rewritten ({what}): from the start");
        assert_eq!(got, whole_log(&log));
        if what == "head" {
            assert!(got.iter().any(|r| r["target"] == "https://Z.example.com/x"), "{what}: {got:?}");
        }
    }
    // Shorter.
    write(&log, &lines[..3], false);
    let (got, _, from) = kept(&log, Some(&s));
    assert_eq!((from, got), (0, whole_log(&log)));
    // Another log with the same lines.
    let other = dir.path().join("o.jsonl");
    write(&other, &lines, false);
    let (_, _, from) = resources::answer_kept(&other, None, Some(Path::new(HOME)), Some(&s), true).unwrap();
    assert_eq!(from, 0, "another path");
    // Another home, or a log that is not read line after line (pi, agy): from the start, nothing kept.
    write(&log, &lines, false);
    let (_, _, from) = resources::answer_kept(&log, None, Some(Path::new("/home/other")), Some(&s), true).unwrap();
    assert_eq!(from, 0, "another home");
    let (_, next, from) = resources::answer_kept(&log, None, Some(Path::new(HOME)), Some(&s), false).unwrap();
    assert_eq!((from, next.is_none()), (0, true), "not linear");
    // The state as kept on disk (JSON) works the same.
    let back: resources::Kept = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    write(&log, &[say("n2", 301, "https://after.example.com")], true);
    let (got, _, from) = kept(&log, Some(&back));
    assert_eq!(from, s.at);
    assert_eq!(got, whole_log(&log));
}

#[test]
fn a_log_over_64_mb_is_read_whole_and_then_only_its_new_lines() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&log).unwrap());
        writeln!(f, "{}", say("a1", 0, "Report: https://early.example.com/report")).unwrap();
        let filler = user("u", 10, &"x".repeat(64 * 1024));
        for _ in 0..1200 {
            writeln!(f, "{filler}").unwrap();
        }
        writeln!(f, "{}", say("a2", 20, "Again https://early.example.com/report")).unwrap();
    }
    let size = std::fs::metadata(&log).unwrap().len();
    assert!(size > 70 * 1024 * 1024, "{size}");
    let get = |rs: &[Value]| rs.iter().find(|r| r["target"] == "https://early.example.com/report").map(|r| r["mentions"].clone());
    let (first, s, _) = kept(&log, None);
    assert_eq!(get(&first), Some(json!(2)), "the early link, over the old 64 MB limit");
    write(&log, &[say("a3", 30, "Once more https://early.example.com/report")], true);
    let (more, _, from) = kept(&log, s.as_ref());
    assert!(from > 64 * 1024 * 1024, "only the new lines: from {from}");
    assert_eq!(get(&more), Some(json!(3)));
}

/// 100 MB of work with no user line, then 100 KB more: the whole log vs only the new lines. Release:
/// `cargo test -p flockbook-modfeed --release --test resources -- --ignored --nocapture timing`.
#[test]
#[ignore]
fn timing_100_mb_then_100_kb() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    let docs = dir.path().to_string_lossy().to_string();
    let chunk = |n: u32| -> Vec<String> {
        let mut l = vec![];
        for i in 0..20u32 {
            let k = n * 100 + i;
            l.push(tool(&format!("w{k}"), 10, &format!("k{k}"), "Bash", json!({"command": format!("step {k}")})));
            l.push(result(&format!("x{k}"), 11, &format!("k{k}"), &"o".repeat(4000), false));
            l.push(say(&format!("s{k}"), 12, &format!("Step {k}: https://step{}.example.com and {docs}/r/{}.md", k % 50, k % 30)));
        }
        l
    };
    write(&log, &[user("u0", 0, "go")], false);
    let mut n = 0;
    while std::fs::metadata(&log).unwrap().len() < 100 * 1024 * 1024 {
        write(&log, &chunk(n), true);
        n += 1;
    }
    let t = std::time::Instant::now();
    let (_, s, _) = kept(&log, None);
    let full_first = t.elapsed();
    let before = std::fs::metadata(&log).unwrap().len();
    while std::fs::metadata(&log).unwrap().len() < before + 100 * 1024 {
        write(&log, &chunk(n), true);
        n += 1;
    }
    let t = std::time::Instant::now();
    let whole = whole_log(&log);
    let full = t.elapsed();
    let t = std::time::Instant::now();
    let (got, _, from) = kept(&log, s.as_ref());
    let inc = t.elapsed();
    assert_eq!(got, whole);
    println!("timing: log {} bytes; first read with a state {:?}; whole log {:?}; only the new {} bytes {:?}; state {} bytes", before, full_first, full, std::fs::metadata(&log).unwrap().len() - from, inc, serde_json::to_vec(&s).unwrap().len());
}


/// A real log (`RES_REAL_LOG=<path>`), copied in 200 steps with the state kept between them: the same answer as the
/// whole copy at 10 of them and at the end, and how far behind the end the kept start falls (release, --ignored --nocapture).
#[test]
#[ignore]
fn real_log_in_steps() {
    let Some(src) = std::env::var_os("RES_REAL_LOG") else { return };
    let data = std::fs::read(&src).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    std::fs::write(&log, b"").unwrap();
    let (mut s, mut behind, mut at, mut checked) = (None, vec![], 0usize, 0);
    for i in 1..=200usize {
        let to = data.len() * i / 200;
        std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(&data[at..to]).unwrap();
        at = to;
        let (got, next, _) = kept(&log, s.as_ref());
        let next = next.unwrap();
        behind.push(to as u64 - next.at);
        if i % 20 == 0 {
            assert_eq!(got, whole_log(&log), "step {i}");
            checked += 1;
        }
        s = Some(next);
    }
    behind.sort();
    println!("real: {} bytes, {checked} checks equal; bytes from the kept start to the end: median {}, p90 {}, max {}; state {} bytes", data.len(), behind[100], behind[180], behind[199], serde_json::to_vec(&s).unwrap().len());
}

/// The state folder stays small: at most 64 state files (the newest kept), none older than 7 days, no old temp files.
/// Only files named as this command names them (`<hex>.json`, `.<hex>.json.<pid>`) are dropped.
#[test]
fn kept_states_have_a_cap() {
    let dir = tempfile::tempdir().unwrap();
    let now = std::time::SystemTime::now();
    let put = |name: &str, age_s: u64| {
        let p = dir.path().join(name);
        std::fs::write(&p, b"{}").unwrap();
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(now - std::time::Duration::from_secs(age_s)).unwrap();
    };
    let hex = |i: u64| format!("{:x}.json", 0xa000 + i);
    for i in 0..80u64 {
        put(&hex(i), 60 + i * 60);
    }
    put("ffff1.json", 8 * 24 * 3600);
    put(".ffff2.json.123", 2 * 3600);
    put(".ffff3.json.124", 60);
    for other in ["notes.txt", "keep.json", "glance.json", ".other.json.1", "Ab12.json"] {
        put(other, 9 * 24 * 3600);
    }
    let log = dir.path().join("s.jsonl");
    std::fs::write(&log, format!("{}\n", say("a1", 0, "https://a.example.com"))).unwrap();
    let (_, kept, _) = resources::answer_kept(&log, None, None, None, true).unwrap();
    let at = dir.path().join("beef.json");
    resources::keep(&kept.expect("a state"), &at);
    let names: std::collections::BTreeSet<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let states = names.iter().filter(|n| n.len() <= 21 && n.ends_with(".json") && n[..n.len() - 5].bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())).count();
    assert_eq!(states, 64, "{names:?}");
    assert!(names.contains("beef.json"), "the file just written stays");
    assert!(names.contains(&hex(0)) && names.contains(&hex(62)) && !names.contains(&hex(63)), "the newest are kept: {names:?}");
    assert!(!names.contains("ffff1.json") && !names.contains(".ffff2.json.123"), "old ones go");
    assert!(names.contains(".ffff3.json.124"), "a temp file being written now stays");
    for other in ["notes.txt", "keep.json", "glance.json", ".other.json.1", "Ab12.json", "s.jsonl"] {
        assert!(names.contains(other), "{other} is not a state file: it stays");
    }
}


// ---- Codex R1 (hg-res-incremental) ----

/// The bytes the mark samples (resources.rs `print`): the first 4 KB, 16 pieces of 1 KB, the last 4 KB.
fn sampled(n: u64, at: u64) -> bool {
    at < 4096 || at + 4096 >= n || (0..16).any(|i| (n / 16 * i..n / 16 * i + 1024).contains(&at))
}

/// A log of filler with one reply whose words sit where the mark does not look (so only the file's own identity tells
/// two files apart), and that reply's offset.
fn unsampled_log(word: &str) -> (Vec<String>, u64) {
    let filler = |i: usize| say(&format!("f{i}"), 1, &format!("Filler {i:04} {}", "x".repeat(300)));
    for at in 20..180usize {
        let mut lines: Vec<String> = vec![user("u0", 0, "go")];
        lines.extend((0..200).map(filler));
        lines.insert(at, say("v1", 1, &format!("{word} https://{word}.example.com")));
        let n: u64 = lines.iter().map(|l| l.len() as u64 + 1).sum();
        let off: u64 = lines[..at].iter().map(|l| l.len() as u64 + 1).sum();
        let len = lines[at].len() as u64;
        if (off..off + len).all(|b| !sampled(n, b)) {
            return (lines, off);
        }
    }
    panic!("no unsampled place");
}

/// Another file at the same path (a log written again), the same size or bigger: all of it again (`full`), with
/// or without a kept state, never merged into the old list. Also when the new file differs only where the mark does
/// not sample: the file itself is another one.
#[test]
fn another_file_at_the_path_is_all_again() {
    let home = Some(Path::new(HOME));
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    let swap = |lines: &[String]| {
        let tmp = dir.path().join("new.jsonl");
        write(&tmp, lines, false);
        std::fs::rename(&tmp, &log).unwrap();
    };
    let (old, _) = unsampled_log("old");
    write(&log, &old, false);
    let (a, k, _) = resources::answer_kept(&log, None, home, None, true).unwrap();
    let size = std::fs::metadata(&log).unwrap().len();
    // The same size, other words where the mark does not look.
    let (new, _) = unsampled_log("new");
    swap(&new);
    assert_eq!(std::fs::metadata(&log).unwrap().len(), size);
    for kept in [k.as_ref(), None] {
        let (b, _, _) = resources::answer_kept(&log, Some(&a.cursor), home, kept, true).unwrap();
        assert!(b.full, "same size, kept {}: {:?}", kept.is_some(), b.items);
        assert_eq!(values(&b.items), whole_log(&log));
    }
    // Bigger: the same, and one more line.
    let mut bigger = new.clone();
    bigger.push(say("v2", 0, "and https://more.example.com"));
    swap(&bigger);
    assert!(std::fs::metadata(&log).unwrap().len() > size);
    for kept in [k.as_ref(), None] {
        let (b, _, _) = resources::answer_kept(&log, Some(&a.cursor), home, kept, true).unwrap();
        assert!(b.full, "bigger, kept {}: {:?}", kept.is_some(), b.items);
        assert_eq!(values(&b.items), whole_log(&log));
        assert!(!b.items.iter().any(|r| r.target.contains("old.example")), "nothing of the old file");
    }
    // Other words at the start too (a smaller log, its mentions older than the cursor's newest).
    swap(&[user("u0", 0, "go"), say("a1", 0, "first https://first.example.com and words"), say("a2", 0, "second https://second.example.com"), say("a3", 0, "x".repeat(size as usize).as_str())]);
    let (b, _, _) = resources::answer_kept(&log, Some(&a.cursor), home, k.as_ref(), true).unwrap();
    assert!(b.full);
    assert_eq!(values(&b.items), whole_log(&log));
}

/// A kept state made with other rules: the log unchanged, still all of it again (the rules may find other things).
#[test]
fn a_state_of_other_rules_is_all_again_also_unchanged() {
    let home = Some(Path::new(HOME));
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &[user("u0", 0, "go"), say("a1", 1, "see https://a.example.com"), user("u1", 2, "ok")], false);
    let (a, k, _) = resources::answer_kept(&log, None, home, None, true).unwrap();
    let mut k = k.unwrap();
    k.rules += 100;
    let (b, _, _) = resources::answer_kept(&log, Some(&a.cursor), home, Some(&k), true).unwrap();
    assert!(b.full, "{:?}", b.items);
    assert_eq!(values(&b.items), whole_log(&log));
}

/// A result that comes again for a call before the kept start (an Artifact call whose failed result is followed by a
/// good one) changes an item already kept: the log is read whole, and the answer is all of it.
#[test]
fn a_later_result_for_a_call_before_the_kept_start_reads_all_again() {
    let home = Some(Path::new(HOME));
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &[
        user("u0", 0, "publish it"),
        tool("a1", 1, "c1", "Artifact", json!({"action": "publish", "file_path": "/tmp/p.html", "title": "Plan"})),
        result("r1", 2, "c1", "Failed: https://claude.ai/code/artifact/late-1", true),
        say("a2", 3, "It failed; see https://a.example.com"),
        user("u1", 4, "ok"),
        say("a3", 5, "Waiting."),
    ], false);
    let (a, k, _) = resources::answer_kept(&log, None, home, None, true).unwrap();
    assert!(k.as_ref().unwrap().at > 0, "a kept start after the call");
    write(&log, &[result("r2", 9, "c1", "Published: https://claude.ai/code/artifact/late-1", false)], true);
    let want = whole_log(&log);
    assert!(want.iter().any(|r| r["kind"] == "artifact"), "the whole log has the artifact: {want:?}");
    let late_at = k.as_ref().unwrap().size;
    assert!(k.as_ref().unwrap().at <= late_at);
    let (b, k2, from) = resources::answer_kept(&log, Some(&a.cursor), home, k.as_ref(), true).unwrap();
    assert_eq!(values(&b.items), want);
    assert!(b.full, "the artifact's time is older than the cursor's: only a full answer brings it");
    assert_eq!(from, 0, "read from the log's start");
    let k2 = k2.unwrap();
    assert!(k2.at == 0 || k2.at > late_at, "the next start is not before the late result: at {} late line {late_at}", k2.at);
    // The next call, one more line: the same as the whole log again, and the start now after the late result.
    write(&log, &[user("u2", 10, "next"), say("a4", 11, "Done https://b.example.com")], true);
    let (c, k3, _) = resources::answer_kept(&log, None, home, Some(&k2), true).unwrap();
    assert_eq!(values(&c.items), whole_log(&log));
    assert!(k3.unwrap().at > late_at, "the start moved past the late result");
}


// ---- Relative doc paths (hg-res-relative-doc) ----

/// A Claude line with the agent's working folder (`cwd`, as Claude Code writes it on each line).
fn in_cwd(l: String, cwd: &Path) -> String {
    let mut v: Value = serde_json::from_str(&l).unwrap();
    v["cwd"] = json!(cwd);
    v.to_string()
}

fn docs_of(log: &Path, cwd: Option<&Path>) -> Vec<resources::Resource> {
    resources::answer_in(log, None, Some(Path::new(HOME)), cwd, None, true).unwrap().0.items.into_iter().filter(|r| r.kind == "doc").collect()
}

fn real(p: &Path) -> std::path::PathBuf {
    p.canonicalize().unwrap()
}

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn a_relative_doc_opens_from_the_agents_folder() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    std::fs::create_dir_all(d.join("proj/docs")).unwrap();
    std::fs::write(d.join("proj/docs/a.md"), "# Plan A").unwrap();
    let log = d.join("s.jsonl");
    let cwd = d.join("proj");
    write(&log, &[
        in_cwd(user("u0", 0, "go"), &cwd),
        in_cwd(say("a1", 1, "Saved in `docs/a.md`."), &cwd),
        in_cwd(say("a2", 2, "Also ./docs/a.md and [the plan](docs/a.md)."), &cwd),
    ], false);
    let docs = docs_of(&log, None);
    assert_eq!(docs.len(), 1, "{docs:?}");
    let a = &docs[0];
    assert_eq!(a.target, d.join("proj/docs/a.md").to_string_lossy());
    assert_eq!((a.mentions, a.exists, a.title.as_str()), (3, Some(true), "Plan A"));
    // The pane's folder when the log names none.
    write(&log, &[user("u0", 0, "go"), say("a1", 1, "Saved in `docs/a.md`.")], false);
    let docs = docs_of(&log, Some(&cwd));
    assert_eq!((docs.len(), docs[0].exists), (1, Some(true)), "{docs:?}");
}

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn a_relative_doc_not_under_the_folder_is_found_by_a_folder_the_agent_went_to() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    std::fs::create_dir_all(d.join("status/reports")).unwrap();
    std::fs::create_dir_all(d.join("repos")).unwrap();
    std::fs::write(d.join("status/reports/map.md"), "# Map").unwrap();
    let log = d.join("s.jsonl");
    let cwd = d.join("repos");
    let st = d.join("status").to_string_lossy().to_string();
    write(&log, &[
        in_cwd(user("u0", 0, "go"), &cwd),
        in_cwd(tool("a1", 1, "c1", "Bash", json!({"command": format!("cd {st} && ls reports")})), &cwd),
        in_cwd(result("r1", 2, "c1", "map.md", false), &cwd),
        in_cwd(say("a2", 3, "The map is in `reports/map.md`."), &cwd),
    ], false);
    let docs = docs_of(&log, None);
    assert_eq!(docs.len(), 1, "{docs:?}");
    assert_eq!((docs[0].target.as_str(), docs[0].exists), (d.join("status/reports/map.md").to_str().unwrap(), Some(true)));
}

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn a_relative_doc_matches_a_path_the_log_wrote_and_joins_its_item() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    std::fs::create_dir_all(d.join("repos")).unwrap();
    let log = d.join("s.jsonl");
    let cwd = d.join("repos");
    let file = d.join("notes/reports/y.md");
    write(&log, &[
        in_cwd(user("u0", 0, "go"), &cwd),
        in_cwd(say("a0", 1, "I will write reports/y.md now."), &cwd),
        in_cwd(tool("a1", 2, "c1", "Write", json!({"file_path": file, "content": "# Y"})), &cwd),
        in_cwd(result("r1", 3, "c1", "ok", false), &cwd),
        in_cwd(say("a2", 4, &format!("Done: {}", file.display())), &cwd),
    ], false);
    let docs = docs_of(&log, None);
    assert_eq!(docs.len(), 1, "the relative mention and the absolute one are one item: {docs:?}");
    assert_eq!((docs[0].target.as_str(), docs[0].mentions, docs[0].exists), (file.to_str().unwrap(), 2, Some(false)));
    assert_eq!(docs[0].first_ts, Some(ms(1)));
}

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn a_relative_doc_with_two_matches_or_none_is_not_listed() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    for f in ["one/reports/z.md", "two/reports/z.md"] {
        std::fs::create_dir_all(d.join(f).parent().unwrap()).unwrap();
        std::fs::write(d.join(f), "# Z").unwrap();
    }
    std::fs::create_dir_all(d.join("repos")).unwrap();
    let log = d.join("s.jsonl");
    let cwd = d.join("repos");
    let cd = |n: &str| json!({"command": format!("cd {} && ls", d.join(n).display())});
    write(&log, &[
        in_cwd(user("u0", 0, "go"), &cwd),
        in_cwd(tool("a1", 1, "c1", "Bash", cd("one")), &cwd),
        in_cwd(result("r1", 2, "c1", "", false), &cwd),
        in_cwd(tool("a2", 3, "c2", "Bash", cd("two")), &cwd),
        in_cwd(result("r2", 4, "c2", "", false), &cwd),
        in_cwd(say("a3", 5, "See reports/z.md and docs/gone.md."), &cwd),
    ], false);
    let docs = docs_of(&log, None);
    assert!(docs.is_empty(), "two matches (not picked) and none (it does not open, maybe an example): not listed: {docs:?}");
}

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn what_is_not_a_relative_doc() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    for t in ["github.com/o/r/blob/main/a.md", "x.com/a.png", "README.md", "see a.md", "docs/a.mdx", "docs/a.md/b", "https://x.com/docs/a.md", "`coord/*.bak.md`", "`contracts/<id>.md`", "`a/{b}.md`", "```\ndocs/a.md\n```"] {
        write(&log, &[say("a1", 1, t)], false);
        assert!(docs_of(&log, Some(dir.path())).is_empty(), "{t}: {:?}", docs_of(&log, Some(dir.path())));
    }
    // The feed's links are as before: a relative path is not a link in a row.
    assert!(resources::detect("docs/a.md and `reports/b.md`").is_empty());
}

fn relative_mix(d: &Path) -> Vec<String> {
    let cwd = d.join("repos");
    let st = d.join("status").to_string_lossy().to_string();
    vec![
        in_cwd(user("u0", 0, "go"), &cwd),
        in_cwd(say("a1", 1, "First `reports/late.md` and docs/a.md."), &cwd),
        in_cwd(user("u1", 2, "and?"), &cwd),
        in_cwd(tool("a2", 3, "c1", "Bash", json!({"command": format!("cd {st}")})), &cwd),
        in_cwd(result("r2", 4, "c1", "", false), &cwd),
        in_cwd(say("a3", 5, "Then reports/map.md."), &cwd),
        in_cwd(user("u2", 6, "ok"), &cwd),
        in_cwd(tool("a4", 7, "c2", "Write", json!({"file_path": format!("{st}/w/reports/late.md"), "content": "x"})), &cwd),
        in_cwd(result("r4", 8, "c2", "ok", false), &cwd),
        in_cwd(say("a5", 9, "Wrote it; also https://x.example.com"), &cwd),
    ]
}

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn relative_docs_cut_at_every_line_the_same_as_the_whole_log() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    std::fs::create_dir_all(d.join("status/reports")).unwrap();
    std::fs::create_dir_all(d.join("repos/docs")).unwrap();
    std::fs::write(d.join("status/reports/map.md"), "# Map").unwrap();
    std::fs::write(d.join("repos/docs/a.md"), "# A").unwrap();
    let lines = relative_mix(&d);
    let all = { let l = d.join("all.jsonl"); write(&l, &lines, false); whole_log(&l) };
    let t: Vec<&str> = all.iter().filter(|r| r["kind"] == "doc").map(|r| r["target"].as_str().unwrap()).collect();
    assert!(t.contains(&d.join("status/w/reports/late.md").to_str().unwrap()), "the Write after the mention resolves it: {t:?}");
    every_cut(&lines, "relative");
}

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn a_relative_doc_that_resolves_later_is_all_again() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    std::fs::create_dir_all(d.join("repos")).unwrap();
    std::fs::create_dir_all(d.join("status")).unwrap();
    let cwd = d.join("repos");
    let log = d.join("s.jsonl");
    let home = Some(Path::new(HOME));
    let st = d.join("status").to_string_lossy().to_string();
    write(&log, &[
        in_cwd(user("u0", 0, "go"), &cwd),
        in_cwd(tool("a1", 1, "c1", "Bash", json!({"command": format!("cd {st}")})), &cwd),
        in_cwd(result("r1", 2, "c1", "", false), &cwd),
        in_cwd(say("a2", 3, "Will be reports/new.md."), &cwd),
        in_cwd(user("u1", 4, "ok"), &cwd),
    ], false);
    let (a, k, _) = resources::answer_in(&log, None, home, None, None, true).unwrap();
    assert!(!a.items.iter().any(|r| r.kind == "doc"), "not found yet: not listed: {:?}", a.items);
    // The file is made where the agent went, the log grows: the mention is now that file. Its time is older than the
    // cursor's newest: only a full answer brings it.
    std::fs::create_dir_all(d.join("status/reports")).unwrap();
    std::fs::write(d.join("status/reports/new.md"), "# New").unwrap();
    write(&log, &[in_cwd(say("a3", 5, "More https://y.example.com"), &cwd)], true);
    let (b, _, _) = resources::answer_in(&log, Some(&a.cursor), home, None, k.as_ref(), true).unwrap();
    assert_eq!(values(&b.items), whole_log(&log));
    assert_eq!(b.items.iter().find(|r| r.kind == "doc").map(|r| r.target.clone()), Some(d.join("status/reports/new.md").to_string_lossy().into()));
    assert!(b.full, "a relative path found now: all of it");
}

/// A real log (`RES_REAL_LOG`, cwd `RES_REAL_CWD`): the docs that only relative paths give, to read by eye (--ignored --nocapture).
#[test]
#[ignore]
fn real_log_relative_docs() {
    let Some(src) = std::env::var_os("RES_REAL_LOG") else { return };
    let cwd = std::env::var_os("RES_REAL_CWD").map(std::path::PathBuf::from);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let log = Path::new(&src);
    let t = std::time::Instant::now();
    let all = resources::answer_in(log, None, home.as_deref(), cwd.as_deref(), None, true).unwrap().0.items;
    println!("read in {:?}", t.elapsed());
    let mut abs: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for l in std::fs::read_to_string(log).unwrap_or_default().lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else { continue };
        if v["type"] != "assistant" { continue }
        for b in v["message"]["content"].as_array().into_iter().flatten() {
            if let Some(s) = b["text"].as_str() {
                for f in resources::detect(s) {
                    if f.kind == "doc" {
                        // Named as the list names it: the folder's real path.
                        let p = std::path::PathBuf::from(f.target.replace("~/", &format!("{}/", home.as_ref().unwrap().display())));
                        let p = match (p.parent().and_then(|d| d.canonicalize().ok()), p.file_name()) { (Some(d), Some(n)) if !p.is_symlink() => p.canonicalize().unwrap_or(d.join(n)), (Some(d), Some(n)) => d.join(n), _ => p };
                        *abs.entry(p.to_string_lossy().into_owned()).or_default() += 1;
                    }
                }
            }
        }
    }
    let new: Vec<_> = all.iter().filter(|r| r.kind == "doc" && !abs.contains_key(&r.target)).collect();
    let there = new.iter().filter(|r| r.exists == Some(true)).count();
    let joined = all.iter().filter(|r| r.kind == "doc" && abs.get(&r.target).is_some_and(|&n| r.mentions > n)).count();
    println!("docs {} (absolute only {}), new rows from relative paths {} ({} there), absolute rows that relative mentions joined {}", all.iter().filter(|r| r.kind == "doc").count(), abs.len(), new.len(), there, joined);
    for r in new.iter().take(60) {
        println!("  {:?} {} x{}", r.exists, r.target, r.mentions);
    }
}


// ---- relative-doc Codex R1 ----

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn two_files_under_the_working_folders_are_not_listed_whatever_else_the_log_says() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    for p in ["a/docs/plan.md", "b/docs/plan.md"] {
        std::fs::create_dir_all(d.join(p).parent().unwrap()).unwrap();
        std::fs::write(d.join(p), "# Plan").unwrap();
    }
    let log = d.join("s.jsonl");
    let (a, b) = (d.join("a"), d.join("b"));
    write(&log, &[
        in_cwd(user("u0", 0, "go"), &a),
        in_cwd(tool("t1", 1, "c1", "Read", json!({"file_path": a.join("docs/plan.md")})), &a),
        in_cwd(result("r1", 2, "c1", "# Plan", false), &a),
        in_cwd(user("u1", 3, "now there"), &b),
        in_cwd(say("a2", 4, "See `docs/plan.md`."), &b),
    ], false);
    assert!(docs_of(&log, None).is_empty(), "two under the folders: not listed, the Read does not pick one: {:?}", docs_of(&log, None));
}

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn only_paths_of_tool_calls_that_worked_are_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    std::fs::create_dir_all(d.join("repos")).unwrap();
    for p in ["x/reports/f.md", "y/reports/g.md", "z/reports/h.md"] {
        std::fs::create_dir_all(d.join(p).parent().unwrap()).unwrap();
        std::fs::write(d.join(p), "# F").unwrap();
    }
    let cwd = d.join("repos");
    let log = d.join("s.jsonl");
    write(&log, &[
        in_cwd(user("u0", 0, "go"), &cwd),
        in_cwd(tool("t1", 1, "c1", "Write", json!({"file_path": d.join("x/reports/f.md"), "content": "x"})), &cwd),
        in_cwd(result("r1", 2, "c1", "denied", true), &cwd),
        in_cwd(tool("t2", 3, "c2", "Bash", json!({"command": format!("cat {}", d.join("z/reports/h.md").display()), "description": d.join("z/reports/h.md")})), &cwd),
        in_cwd(result("r2", 4, "c2", "# F", false), &cwd),
        in_cwd(tool("t3", 5, "c3", "Bash", json!({"command": format!("cd {} && ls", d.join("x").display())})), &cwd),
        in_cwd(result("r3", 6, "c3", "no", true), &cwd),
        in_cwd(say("a1", 7, &format!("Earlier: {}", d.join("y/reports/g.md").display())), &cwd),
        in_cwd(user("u1", 8, "and?"), &cwd),
        in_cwd(say("a2", 9, "See reports/f.md, reports/g.md and reports/h.md."), &cwd),
    ], false);
    let docs = docs_of(&log, None);
    let t: Vec<(&str, usize)> = docs.iter().map(|r| (r.target.as_str(), r.mentions)).collect();
    assert_eq!(t, vec![(d.join("y/reports/g.md").to_str().unwrap(), 1)], "a failed Write or cd, a command's text, another reply's path: no candidates");
}

#[test]
#[cfg(unix)]
fn a_relative_path_with_dot_dot_goes_through_links_as_the_file_system_does() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    let cwd = d.join("proj");
    std::fs::create_dir_all(cwd.join("deep/inner")).unwrap();
    std::fs::create_dir_all(cwd.join("deep/docs")).unwrap();
    std::fs::create_dir_all(cwd.join("docs")).unwrap();
    std::fs::write(cwd.join("deep/docs/plan.md"), "# Deep").unwrap();
    std::fs::write(cwd.join("docs/plan.md"), "# Top").unwrap();
    std::os::unix::fs::symlink(cwd.join("deep/inner"), cwd.join("alias")).unwrap();
    let log = d.join("s.jsonl");
    write(&log, &[in_cwd(user("u0", 0, "go"), &cwd), in_cwd(say("a1", 1, "See `alias/../docs/plan.md`."), &cwd)], false);
    let docs = docs_of(&log, None);
    assert_eq!(docs.len(), 1, "{docs:?}");
    assert_eq!((docs[0].target.as_str(), docs[0].title.as_str()), (cwd.join("deep/docs/plan.md").to_str().unwrap(), "Deep"), "alias/.. is deep, not proj");
}

#[test]
#[cfg(unix)]
fn one_file_written_two_ways_is_one_item() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    std::fs::create_dir_all(d.join("real")).unwrap();
    std::fs::write(d.join("real/a.md"), "# A").unwrap();
    std::os::unix::fs::symlink(d.join("real"), d.join("link")).unwrap();
    let log = d.join("s.jsonl");
    let mut text = format!("{} and {}", d.join("link/a.md").display(), d.join("real/a.md").display());
    // A volume that does not tell case apart (macOS): the other case is the same file too.
    let upper = d.join("REAL/A.md");
    let ci = upper.is_file();
    if ci {
        text.push_str(&format!(" and {}", upper.display()));
    }
    write(&log, &[say("a1", 1, &text)], false);
    let docs = docs_of(&log, None);
    assert_eq!(docs.len(), 1, "{docs:?}");
    assert_eq!((docs[0].target.as_str(), docs[0].mentions), (d.join("real/a.md").to_str().unwrap(), if ci { 3 } else { 2 }));
}


// ---- relative-doc Codex R2 ----

// Unix only: a relative doc opens only from a working folder that starts with "/" (a Windows temp folder is "C:\…").
#[test]
#[cfg(unix)]
fn only_a_real_cd_is_a_folder_the_agent_went_to() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    for p in ["arch/reports/a.md", "echo/reports/b.md", "real/reports/c.md", "esc/reports/e.md", "semi/reports/s.md"] {
        std::fs::create_dir_all(d.join(p).parent().unwrap()).unwrap();
        std::fs::write(d.join(p), "# X").unwrap();
    }
    std::fs::create_dir_all(d.join("repos")).unwrap();
    let cwd = d.join("repos");
    let q = |n: &str| d.join(n).to_string_lossy().to_string();
    let cmds = [
        format!("printf 'x ; cd {} && y'", q("arch")),
        format!("echo \"cd {}\"", q("echo")),
        format!("echo cd {} \\; cd {}x", q("esc"), q("esc")),
        format!("ls && cd \"{}\" && ls", q("real")),
        format!("true;cd {}", q("semi")),
    ];
    let mut lines = vec![in_cwd(user("u0", 0, "go"), &cwd)];
    for (i, c) in cmds.iter().enumerate() {
        lines.push(in_cwd(tool(&format!("t{i}"), 1, &format!("c{i}"), "Bash", json!({"command": c})), &cwd));
        lines.push(in_cwd(result(&format!("r{i}"), 2, &format!("c{i}"), "ok", false), &cwd));
    }
    lines.push(in_cwd(say("a1", 3, "See reports/a.md, reports/b.md, reports/c.md, reports/e.md and reports/s.md."), &cwd));
    let log = d.join("s.jsonl");
    write(&log, &lines, false);
    let mut t: Vec<String> = docs_of(&log, None).into_iter().map(|r| r.target).collect();
    t.sort();
    assert_eq!(t, vec![q("real/reports/c.md"), q("semi/reports/s.md")], "a cd in quotes, an echoed one, an escaped ';': not folders");
}

#[test]
fn a_state_or_a_cursor_of_the_rules_before_is_all_again() {
    let home = Some(Path::new(HOME));
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &[user("u0", 0, "go"), say("a1", 1, "see https://a.example.com"), user("u1", 2, "ok")], false);
    const { assert!(resources::RULES >= 3, "rules 3: states of 101da5e took paths from failed tools") };
    let (a, k, _) = resources::answer_kept(&log, None, home, None, true).unwrap();
    let mut k = k.unwrap();
    k.rules = 2;
    let (b, _, _) = resources::answer_kept(&log, Some(&a.cursor), home, Some(&k), true).unwrap();
    assert!(b.full, "a state of rules 2 is not used");
    // An r4 cursor made with the rules before (its mark has them): all of it again, with no state too.
    let p: Vec<&str> = a.cursor.split('.').collect();
    let size: u64 = p[2].parse().unwrap();
    let old = format!("r4.{}.{}.{}.{}.{}", p[1], p[2], p[3], resources::mark_for_rules(&log, size, resources::RULES - 1).unwrap(), p[5]);
    let c = resources::answer_kept(&log, Some(&old), home, None, true).unwrap().0;
    assert!(c.full && !c.items.is_empty(), "{:?}", c.items);
    let same = resources::answer_kept(&log, Some(&a.cursor), home, None, true).unwrap().0;
    assert!(!same.full && same.items.is_empty(), "this rules' cursor: nothing new");
}


// --- Progressive loading (`--chunk`): newest part first, the list so far after each part. ---

/// Asks with `--chunk` until the load is done, with the states kept between calls as the helper keeps them: the
/// final items, the answers on the way, and the state to keep.
fn chunked(log: &Path, chunk: u64, cwd: Option<&Path>) -> (Vec<Value>, Vec<resources::Answer>, Option<resources::Kept>) {
    let (mut since, mut part, mut kept, mut seen) = (None::<String>, None::<resources::Part>, None::<resources::Kept>, vec![]);
    for _ in 0..100_000 {
        let (a, k, p) = resources::answer_parts(log, since.as_deref(), Some(Path::new(HOME)), cwd, kept.as_ref(), part.as_ref(), true, Some(chunk)).unwrap();
        (kept, part, since) = (k, p, Some(a.cursor.clone()));
        let done = a.more.is_none();
        seen.push(a);
        if done {
            assert!(part.is_none(), "no part state once done");
            return (values(&seen.last().unwrap().items), seen, kept);
        }
    }
    panic!("the load never ended");
}

fn every_chunk(lines: &[String], what: &str, cwd: Option<&Path>) {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, lines, false);
    let size = std::fs::metadata(&log).unwrap().len();
    let all = values(&resources::answer_in(&log, None, Some(Path::new(HOME)), cwd, None, true).unwrap().0.items);
    for chunk in [1u64, 64, 200, 500, 1000, 3000, 1 << 20] {
        let (got, seen, kept) = chunked(&log, chunk, cwd);
        assert_eq!(got, all, "{what}: chunk {chunk}");
        assert!(kept.is_some(), "{what}: chunk {chunk}: a state to keep once done");
        if chunk < size / 4 && lines.len() >= 8 {
            assert!(seen.len() > 1, "{what}: chunk {chunk} of {size} bytes in one call");
        }
        let mut read = 0;
        for a in &seen[..seen.len() - 1] {
            let (r, s) = a.more.unwrap();
            assert_eq!(s, size, "{what}: the size is the log's");
            assert!(r > read && r < s, "{what}: read {r} after {read}");
            read = r;
            assert!(a.full, "{what}: each part's answer is the whole list so far");
            assert!(a.cursor.starts_with("p1."), "{what}: a part's cursor");
        }
        assert!(seen.last().unwrap().full && seen.last().unwrap().cursor.starts_with("r4."), "{what}: the last answer: all of it, a cursor for deltas");
    }
}

#[test]
fn a_load_in_parts_ends_with_the_same_list_as_one_read() {
    every_chunk(&claude_mix(), "claude", None);
    every_chunk(&codex_mix(), "codex", None);
    every_chunk(&convo(), "convo", None);
    // Parts that end at every place: each length of the log.
    let lines = claude_mix();
    for k in (3..lines.len()).step_by(4) {
        every_chunk(&lines[..k], &format!("claude, the first {k} lines"), None);
    }
}

#[test]
#[cfg(unix)]
fn a_load_in_parts_resolves_relative_docs_as_one_read() {
    let dir = tempfile::tempdir().unwrap();
    let d = real(dir.path());
    std::fs::create_dir_all(d.join("status/reports")).unwrap();
    std::fs::create_dir_all(d.join("repos/docs")).unwrap();
    std::fs::write(d.join("status/reports/map.md"), "# Map").unwrap();
    std::fs::write(d.join("repos/docs/a.md"), "# A").unwrap();
    every_chunk(&relative_mix(&d), "relative", None);
    every_chunk(&relative_mix(&d), "relative, the pane's folder", Some(&d.join("repos")));
}

#[test]
fn the_first_part_has_the_newest_items_and_later_parts_only_add() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &claude_mix(), false);
    let (all, seen, _) = chunked(&log, 300, None);
    let first = values(&seen[0].items);
    assert!(!first.is_empty(), "the first part lists something");
    assert_eq!(first[0]["id"], all[0]["id"], "the newest item is in the first part");
    let ids = |v: &[Value]| v.iter().map(|r| r["id"].as_str().unwrap().to_string()).collect::<std::collections::BTreeSet<_>>();
    for w in seen.windows(2) {
        assert!(ids(&values(&w[0].items)).is_subset(&ids(&values(&w[1].items))), "a later part never drops an item");
    }
}

#[test]
fn after_a_load_in_parts_new_lines_come_as_a_delta() {
    let lines = claude_mix();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &lines[..40], false);
    let (_, seen, kept) = chunked(&log, 300, None);
    let cursor = seen.last().unwrap().cursor.clone();
    write(&log, &lines[40..], true);
    let (a, k, p) = resources::answer_parts(&log, Some(&cursor), Some(Path::new(HOME)), None, kept.as_ref(), None, true, Some(300)).unwrap();
    assert!(!a.full && a.more.is_none() && p.is_none(), "a delta, not a new load");
    assert!(k.is_some());
    // Merged by id as the mod does: the whole log's list.
    let mut by: Vec<Value> = values(&seen.last().unwrap().items);
    for r in values(&a.items) {
        by.retain(|x| x["id"] != r["id"]);
        by.push(r);
    }
    let key = |v: &Value| (std::cmp::Reverse(v["last_ts"].as_i64()), v["id"].as_str().unwrap().to_string());
    let mut want = whole_log(&log);
    by.sort_by_key(key);
    want.sort_by_key(key);
    assert_eq!(by, want);
}

#[test]
fn lines_written_during_a_load_come_after_it() {
    let lines = claude_mix();
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &lines[..60], false);
    let size = std::fs::metadata(&log).unwrap().len();
    let home = Some(Path::new(HOME));
    let (a, mut kept, mut part) = resources::answer_parts(&log, None, home, None, None, None, true, Some(300)).unwrap();
    assert!(a.more.is_some());
    write(&log, &lines[60..], true);
    let mut since = a.cursor;
    let last = loop {
        let (a, k, p) = resources::answer_parts(&log, Some(&since), home, None, kept.as_ref(), part.as_ref(), true, Some(300)).unwrap();
        (kept, part, since) = (k, p, a.cursor.clone());
        if let Some((_, s)) = a.more {
            assert_eq!(s, size, "a load reads the log as it was when it began");
        } else {
            break a;
        }
    };
    let cut = tempfile::tempdir().unwrap();
    let first = cut.path().join("s.jsonl");
    write(&first, &lines[..60], false);
    assert_eq!(values(&last.items), whole_log(&first), "the load: the log as it began");
    let (d, _, _) = resources::answer_parts(&log, Some(&since), home, None, kept.as_ref(), None, true, Some(300)).unwrap();
    assert!(!d.full && !d.items.is_empty(), "then the lines after it, as a delta");
}

#[test]
fn a_lost_part_state_starts_the_load_again_and_no_chunk_is_one_read() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &claude_mix(), false);
    let home = Some(Path::new(HOME));
    let (a, _, p) = resources::answer_parts(&log, None, home, None, None, None, true, Some(300)).unwrap();
    assert!(a.more.is_some() && p.is_some());
    // The cursor of a part, but its state gone: the load begins again (the newest part, all of it so far).
    let (b, _, p2) = resources::answer_parts(&log, Some(&a.cursor), home, None, None, None, true, Some(300)).unwrap();
    assert_eq!(values(&b.items), values(&a.items));
    assert_eq!(b.more, a.more);
    assert!(p2.is_some());
    // No --chunk (an older mod), or an agent whose log is not linear: one read, as before.
    let (c, _, p3) = resources::answer_parts(&log, None, home, None, None, None, true, None).unwrap();
    assert!(c.more.is_none() && c.full && p3.is_none());
    assert_eq!(values(&c.items), whole_log(&log));
    let (c, _, p3) = resources::answer_parts(&log, None, home, None, None, None, false, Some(300)).unwrap();
    assert!(c.more.is_none() && p3.is_none());
    // A part's cursor with no --chunk: one read, all of it.
    let (c, _, _) = resources::answer_parts(&log, Some(&a.cursor), home, None, None, p.as_ref(), true, None).unwrap();
    assert!(c.more.is_none() && c.full);
    assert_eq!(values(&c.items), whole_log(&log));
}

#[test]
fn a_state_kept_from_before_means_no_load_in_parts() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("s.jsonl");
    write(&log, &claude_mix(), false);
    let home = Some(Path::new(HOME));
    let (_, k, _) = resources::answer_kept(&log, None, home, None, true).unwrap();
    // A full read (no cursor) with a state that fits: from the kept start, at once.
    let (a, _, p) = resources::answer_parts(&log, None, home, None, k.as_ref(), None, true, Some(300)).unwrap();
    assert!(a.more.is_none() && p.is_none() && a.full);
    assert_eq!(values(&a.items), whole_log(&log));
}


/// A real log (`RES_REAL_LOG=<path>`, a copy that does not grow) read in parts (`RES_CHUNK`, default 8 MiB) with each
/// part's state written and read back as the helper does: the time to the first items and to the end, and the list
/// at the end against one read (release, --ignored --nocapture). Prints numbers only.
#[test]
#[ignore]
fn real_log_in_parts() {
    let Some(src) = std::env::var_os("RES_REAL_LOG") else { return };
    let log = Path::new(&src);
    let chunk = std::env::var("RES_CHUNK").ok().and_then(|c| c.parse().ok()).unwrap_or(8u64 << 20);
    // This machine's home, as the helper (a made-up /home/… waits on the automounter on a Mac).
    let real_home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let home = real_home.as_deref();
    let t = std::time::Instant::now();
    let one = resources::answer_in(log, None, home, None, None, true).unwrap().0;
    let one_s = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let (mut since, mut part, mut kept, mut calls, mut first, mut first_n) = (None::<String>, None::<resources::Part>, None::<resources::Kept>, 0, 0.0, 0);
    let mut biggest = 0;
    let last = loop {
        let (a, k, p) = resources::answer_parts(log, since.as_deref(), home, None, kept.as_ref(), part.as_ref(), true, Some(chunk)).unwrap();
        calls += 1;
        if calls == 1 {
            (first, first_n) = (t.elapsed().as_secs_f64(), a.items.len());
        }
        // As the helper: the states through JSON.
        let pj = p.as_ref().map(|p| serde_json::to_vec(p).unwrap());
        biggest = biggest.max(pj.as_ref().map_or(0, |j| j.len()));
        part = pj.map(|j| serde_json::from_slice(&j).unwrap());
        kept = k.map(|k| serde_json::from_slice(&serde_json::to_vec(&k).unwrap()).unwrap());
        since = Some(a.cursor.clone());
        if a.more.is_none() {
            break a;
        }
    };
    let all_s = t.elapsed().as_secs_f64();
    let same = values(&last.items) == values(&one.items);
    println!("parts: {} bytes, chunk {chunk}; one read {one_s:.2}s {} items; in parts: first answer {first:.2}s {first_n} items, all {all_s:.2}s in {calls} calls, {} items, same list: {same}; biggest part state {biggest} bytes", std::fs::metadata(log).unwrap().len(), one.items.len(), last.items.len());
    assert!(same);
}


/// Parts only for logs where each line is whole on its own (Claude Code, Codex): a Grok reply streams in pieces over
/// several lines (a cut between them would split a URL), pi and agy are trees.
#[test]
fn only_claude_and_codex_logs_load_in_parts() {
    assert!(resources::in_parts("claude") && resources::in_parts("codex"));
    for k in ["grok", "cursor", "pi", "agy", "", "other"] {
        assert!(!resources::in_parts(k), "{k}");
    }
}
