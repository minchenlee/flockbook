//! `glance`: what an agent's last turn did, by rules, from its log.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use modfeedkit::{glance, tail};
use serde_json::{json, Value};
use std::process::Command;

fn ts(s: u32) -> String {
    format!("2026-11-03T01:{:02}:{:02}.000Z", 40 + s / 60, s % 60)
}
fn user(id: &str, t: u32, text: &str) -> String {
    json!({"type": "user", "uuid": id, "timestamp": ts(t), "message": {"role": "user", "content": text}}).to_string()
}
fn say(id: &str, t: u32, text: &str) -> String {
    json!({"type": "assistant", "uuid": id, "timestamp": ts(t), "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}).to_string()
}
fn call(id: &str, t: u32, call: &str, name: &str, input: Value) -> String {
    json!({"type": "assistant", "uuid": id, "timestamp": ts(t), "message": {"role": "assistant", "content": [{"type": "tool_use", "id": call, "name": name, "input": input}]}}).to_string()
}
fn bash(id: &str, t: u32, c: &str, cmd: &str) -> String {
    call(id, t, c, "Bash", json!({"command": cmd}))
}
fn edit(id: &str, t: u32, c: &str, path: &str) -> String {
    call(id, t, c, "Edit", json!({"file_path": path, "old_string": "a", "new_string": "b"}))
}
fn result(id: &str, t: u32, call: &str, error: bool) -> String {
    json!({"type": "user", "uuid": id, "timestamp": ts(t), "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": call, "content": "out", "is_error": error}]}}).to_string()
}
fn glance_of(lines: &[String]) -> glance::Facts {
    glance::facts(&tail::items(lines), "claude")
}

#[test]
fn edits_then_a_test_that_passes() {
    let g = glance_of(&[
        user("u0", 0, "Old turn"),
        edit("x1", 1, "c0", "/w/old.rs"),
        result("x2", 2, "c0", false),
        say("x3", 3, "Old reply."),
        user("u1", 10, "Fix the parser"),
        edit("a1", 11, "c1", "/w/src/a.rs"),
        result("r1", 12, "c1", false),
        edit("a2", 13, "c2", "/w/src/a.rs"),
        result("r2", 14, "c2", false),
        call("a3", 15, "c3", "Write", json!({"file_path": "/w/src/b.rs", "content": "x"})),
        result("r3", 16, "c3", false),
        bash("a4", 17, "c4", "cd /w && cargo test -p parser 2>&1 | tail -5"),
        result("r4", 30, "c4", false),
        say("a5", 31, "Fixed the parser. It now reads empty lines.\n\nAll 12 tests pass."),
    ]);
    assert_eq!(g.edited, 2, "distinct files of the last turn only");
    assert_eq!(g.commands, 1);
    assert_eq!(g.failed, None);
    assert_eq!((g.tests.as_deref(), g.test.as_deref()), (Some("ran"), Some("cargo test")), "its exit code went into a pipe");
    assert_eq!(g.said.as_deref(), Some("Fixed the parser."));
    assert_eq!(g.id.as_deref(), Some("a5-0"), "the newest item's id");
    assert!(g.ended.is_some());
}

#[test]
fn an_edit_after_the_test_means_no_test_run() {
    let g = glance_of(&[
        user("u1", 0, "Go"),
        bash("a1", 1, "c1", "npm test"),
        result("r1", 2, "c1", false),
        edit("a2", 3, "c2", "/w/a.js"),
        result("r2", 4, "c2", false),
        bash("a3", 5, "c3", "git status"),
        result("r3", 6, "c3", false),
        say("a4", 7, "Done"),
    ]);
    assert_eq!((g.edited, g.commands), (1, 2));
    assert_eq!(g.tests.as_deref(), Some("none"), "the test ran before the last edit");
    assert_eq!(g.said.as_deref(), Some("Done"));
}

#[test]
fn the_last_command_failed() {
    let g = glance_of(&[
        user("u1", 0, "Build"),
        edit("a1", 1, "c1", "/w/a.js"),
        result("r1", 2, "c1", false),
        bash("a2", 3, "c2", "pytest -q"),
        result("r2", 4, "c2", false),
        bash("a3", 5, "c3", "npm run build\n# the release build"),
        result("r3", 6, "c3", true),
        say("a4", 7, "The build fails. I stopped here."),
    ]);
    assert_eq!(g.failed.as_deref(), Some("npm run build"), "its first line");
    assert_eq!(g.tests.as_deref(), Some("passed"));
    // A failed test run is both.
    let t = glance_of(&[user("u1", 0, "Go"), edit("a1", 1, "c1", "/w/a.py"), result("r1", 2, "c1", false), bash("a2", 3, "c2", "python -m pytest"), result("r2", 4, "c2", true)]);
    assert_eq!((t.tests.as_deref(), t.test.as_deref(), t.failed.as_deref()), (Some("failed"), Some("pytest"), Some("python -m pytest")));
    // A later command that passes clears it.
    let ok = glance_of(&[user("u1", 0, "Go"), bash("a2", 3, "c2", "ls nope"), result("r2", 4, "c2", true), bash("a3", 5, "c3", "ls"), result("r3", 6, "c3", false)]);
    assert_eq!(ok.failed, None);
    // A command with no result yet is not a failure, and not a test run.
    let open = glance_of(&[user("u1", 0, "Go"), edit("a1", 1, "c1", "/w/a.py"), result("r1", 2, "c1", false), bash("a2", 3, "c2", "pytest")]);
    assert_eq!((open.failed.clone(), open.tests.as_deref()), (None, Some("none")));
}

#[test]
fn no_turn_yet_and_test_names() {
    let g = glance_of(&[]);
    assert_eq!((g.id.clone(), g.edited, g.commands, g.said.clone(), g.tests.clone()), (None, 0, 0, None, None));
    for (cmd, name) in [
        ("pytest tests/", Some("pytest")),
        ("cd herdr-app && npm test", Some("npm test")),
        ("npm run test:unit", Some("npm test")),
        ("go test ./...", Some("go test")),
        ("cargo nextest run", Some("cargo nextest")),
        ("swift test --filter A", Some("swift test")),
        ("node --test test/", Some("node --test")),
        ("npx vitest run", Some("vitest")),
        ("make test", Some("make test")),
        ("bundle exec rspec", Some("rspec")),
        ("queue run --pool build -- cargo test -p core", Some("cargo test")),
        ("A=1 timeout 600 go test ./...", Some("go test")),
        ("cargo test -- --nocapture", Some("cargo test")),
        ("git checkout -- test", None),
        ("cat pytest.ini", None),
        ("echo latest", None),
        ("git log --grep test", None),
        ("ls tests", None),
    ] {
        assert_eq!(glance::test_name(cmd).as_deref(), name, "{cmd}");
    }
    assert_eq!(glance::first_sentence("- **Done.** More here").as_deref(), Some("Done."));
    assert_eq!(glance::first_sentence("改好了。接下來要測試").as_deref(), Some("改好了。"));
    assert_eq!(glance::first_sentence("Updated v1.2 of a.rs and more").as_deref(), Some("Updated v1.2 of a.rs and more"));
    assert_eq!(glance::first_sentence("```\ncode\n```\n\nAfter the code."), Some("After the code.".to_string()));
    assert_eq!(glance::first_sentence("See [the notes](file:///w/notes.md) and a[0]. Then more").as_deref(), Some("See the notes and a[0]."));
    assert_eq!(glance::first_sentence("  \n"), None);
    // Words only: no test is expected. A code file beside them: it is.
    let words = glance_of(&[user("u1", 0, "Write it up"), edit("a1", 1, "c1", "/w/notes/REPORT.md"), result("r1", 2, "c1", false), say("a2", 3, "Written.")]);
    assert_eq!((words.edited, words.tests), (1, None));
    let both = glance_of(&[user("u1", 0, "Go"), edit("a1", 1, "c1", "/w/README.md"), result("r1", 2, "c1", false), edit("a2", 3, "c2", "/w/Makefile"), result("r2", 4, "c2", false)]);
    assert_eq!((both.edited, both.tests.as_deref()), (2, Some("none")));
}

#[test]
fn codex_edits_and_exit_codes() {
    let line = |t: u32, payload: Value| json!({"timestamp": ts(t), "type": "response_item", "payload": payload}).to_string();
    let lines = vec![
        line(0, json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Fix it"}]})),
        line(1, json!({"type": "custom_tool_call", "call_id": "c1", "name": "apply_patch", "input": "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-a\n+b\n*** Add File: src/b.rs\n+x\n*** End Patch"})),
        line(2, json!({"type": "custom_tool_call_output", "call_id": "c1", "output": "Success"})),
        line(3, json!({"type": "function_call", "call_id": "c2", "name": "exec_command", "arguments": "{\"cmd\":\"cargo test\"}"})),
        line(4, json!({"type": "function_call_output", "call_id": "c2", "output": "Chunk ID: 1\nProcess exited with code 101\nOutput:\nfailed"})),
        line(5, json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "One test still fails."}]})),
    ];
    let g = glance::facts(&tail::items(&lines), "codex");
    assert_eq!((g.edited, g.commands), (2, 1), "{g:?}");
    assert_eq!((g.tests.as_deref(), g.failed.as_deref()), (Some("failed"), Some("cargo test")), "{g:?}");
    assert_eq!(g.said.as_deref(), Some("One test still fails."));
}

fn run(sock: &str, args: &[&str]) -> Vec<Value> {
    let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(args).env_clear().env("HC_SOCKET", sock).env("HOME", "/nonexistent-home").output().unwrap();
    assert!(out.status.success(), "exit code 0 always");
    String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn the_command_answers_for_every_agent_on_this_machine() {
    assert!(run("/nonexistent.sock", &["version"])[1]["commands"].as_array().unwrap().contains(&json!("glance")));
    assert_eq!(run("/nonexistent.sock", &["glance"])[1]["code"], "no-core");
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache.json");
    let cache_s = cache.to_string_lossy().to_string();
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "idle"), common::terminal("t2", &["zsh"], Some(9), None, None), common::terminal("t3", &["codex"], Some(77), Some("working"), None)]);
    // No log found yet: a line for each agent, with nothing in it.
    let v = run(&fake.path, &["glance", "--cache", &cache_s]);
    let lines: Vec<&Value> = v.iter().filter(|l| l["type"] == "glance").collect();
    assert_eq!(lines.iter().map(|l| l["terminal"].as_str().unwrap()).collect::<Vec<_>>(), ["t1", "t3"], "agents only");
    assert_eq!(lines[0], &json!({"type": "glance", "terminal": "t1", "agent": "claude", "path": null, "edited": 0, "commands": 0}));
    assert_eq!(v.last().unwrap()["ok"], true);
    // The cache names t1's log: its last turn.
    let log = dir.path().join("s.jsonl");
    let turn = [user("u1", 0, "Go"), edit("a1", 1, "c1", "/w/a.rs"), result("r1", 2, "c1", false), say("a2", 3, "Changed it. Want me to run the tests?")];
    std::fs::write(&log, turn.join("\n") + "\n").unwrap();
    let mut c: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    c["entries"]["t1"]["path"] = json!(log);
    std::fs::write(&cache, c.to_string()).unwrap();
    let v = run(&fake.path, &["glance", "--cache", &cache_s]);
    let g = v.iter().find(|l| l["type"] == "glance" && l["terminal"] == "t1").unwrap();
    assert_eq!(g["edited"], 1);
    assert_eq!(g["tests"], "none");
    assert_eq!(g["said"], "Changed it.");
    assert_eq!(g["id"], "a2-0");
    assert_eq!(g["path"], json!(log));
    // The same log again: the kept answer, word for word; a longer log: read again.
    let again = run(&fake.path, &["glance", "--cache", &cache_s]);
    assert_eq!(again.iter().find(|l| l["terminal"] == "t1"), Some(g));
    std::fs::write(&log, turn.join("\n") + "\n" + &user("u2", 9, "Yes") + "\n" + &say("a3", 10, "Running them now.") + "\n").unwrap();
    let more = run(&fake.path, &["glance", "--cache", &cache_s]);
    let g2 = more.iter().find(|l| l["type"] == "glance" && l["terminal"] == "t1").unwrap();
    assert_eq!((g2["said"].as_str(), g2["edited"].as_i64()), (Some("Running them now."), Some(0)));
    assert!(fake.keys().is_empty() && fake.calls("terminal.send").is_empty(), "glance only reads");
}

#[test]
fn a_line_holds_what_the_card_reads_only() {
    let g = glance_of(&[
        user("u1", 10, "Make the importer faster."),
        say("a1", 11, "## Plan\n\nI will measure it first. Then cut.\n\nShould I go on?"),
        edit("a2", 12, "c1", "/w/src/importer.py"),
        result("r2", 13, "c1", false),
        bash("a3", 14, "c2", "pytest -q"),
    ]);
    assert_eq!(g.said.as_deref(), Some("Plan"), "markdown marks are left out");
    let v = serde_json::to_value(&g).unwrap();
    for gone in ["task", "doing", "files", "asks", "started"] {
        assert!(v.get(gone).is_none(), "{gone}: {v}");
    }
    // Paths in the first sentence are their last name only; other slashes stay.
    assert_eq!(glance::first_sentence("Done with `loader` (`/w/atlas/src/loader.rs`) and https://example.com/a/b.").as_deref(), Some("Done with loader (loader.rs) and https://example.com/a/b."));
    assert_eq!(glance::first_sentence("The and/or case in src/a.rs, 1/2/3 and /tmp stay.").as_deref(), Some("The and/or case in src/a.rs, 1/2/3 and /tmp stay."));
}
