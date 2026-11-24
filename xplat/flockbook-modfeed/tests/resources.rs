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
    assert!(std::fs::metadata(&log).unwrap().len() < resources::BYTES);
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
