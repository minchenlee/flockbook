//! The built binary end to end, on a fake herdr-core: the protocol's header and end, errors, and a card.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use serde_json::Value;
use std::io::Write;
use std::process::Command;

fn run(sock: &str, args: &[&str]) -> Vec<Value> {
    let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
        .args(args)
        .env_clear()
        .env("HC_SOCKET", sock)
        .env("HOME", "/nonexistent-home")
        .output()
        .unwrap();
    assert!(out.status.success(), "exit code 0 always");
    String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// A real path that is not UTF-8 is not sent (the mod would open another path than the one checked).
/// APFS does not allow such names, so Linux only.
#[test]
#[cfg(target_os = "linux")]
fn open_check_refuses_a_real_path_that_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let t = tempfile::tempdir().unwrap();
    let d = t.path().canonicalize().unwrap();
    let bad = d.join(std::ffi::OsStr::from_bytes(b"r\xff.md"));
    std::fs::write(&bad, "# R").unwrap();
    std::os::unix::fs::symlink(&bad, d.join("a.md")).unwrap();
    let v = run("/nonexistent.sock", &["open-check", "--path", d.join("a.md").to_str().unwrap()]);
    assert_eq!((v[1]["type"].as_str(), v[1]["ok"].as_bool()), (Some("openable"), Some(false)), "{v:?}");
    assert!(v[1].get("path").is_none());
}

#[test]
fn protocol_lines() {
    let v = run("/nonexistent.sock", &["version"]);
    assert_eq!(v[0], serde_json::json!({"type":"header","protocol":1,"sidecar":"0.1.0","cmd":"version"}));
    assert_eq!(v[1]["commands"], serde_json::json!(["version", "feed", "status", "card", "approve", "answer", "send", "paste-image", "helper", "update-check", "queue", "phone", "commands", "glance", "summary", "summary-models", "summary-usage", "resources", "open-check"]));
    assert_eq!(v.last().unwrap()["type"], "end");
    assert_eq!(v.last().unwrap()["ok"], true);

    let u = run("/nonexistent.sock", &["nope"]);
    assert_eq!(u[1]["code"], "usage");
    assert_eq!(u.last().unwrap()["ok"], false);
    assert_eq!(run("/nonexistent.sock", &["feed"])[1]["message"], "feed needs --terminal <addr>");
    assert_eq!(run("/nonexistent.sock", &["feed", "--terminal", "t1"])[1]["code"], "no-core");
    assert_eq!(run("/nonexistent.sock", &["resources"])[1]["message"], "resources needs --terminal <addr>");
    assert_eq!(run("/nonexistent.sock", &["resources", "--terminal", "t1"])[1]["code"], "no-core");
    assert_eq!(run("/nonexistent.sock", &["open-check"])[1]["message"], "open-check needs --path <file>");

    // open-check: local, no core needed; the real file, or why not.
    let t = tempfile::tempdir().unwrap();
    let d = t.path().canonicalize().unwrap();
    std::fs::write(d.join("b.md"), "# B").unwrap();
    std::fs::write(d.join("notes.txt"), "x").unwrap();
    std::os::unix::fs::symlink(d.join("b.md"), d.join("a.md")).unwrap();
    std::os::unix::fs::symlink(d.join("notes.txt"), d.join("preview.md")).unwrap();
    let ok = run("/nonexistent.sock", &["open-check", "--path", d.join("a.md").to_str().unwrap()]);
    assert_eq!(ok[1], serde_json::json!({"type": "openable", "ok": true, "path": d.join("b.md").to_str().unwrap()}));
    assert_eq!(ok.last().unwrap()["ok"], true);
    let no = run("/nonexistent.sock", &["open-check", "--path", d.join("preview.md").to_str().unwrap()]);
    assert_eq!((no[1]["type"].as_str(), no[1]["ok"].as_bool()), (Some("openable"), Some(false)));
    assert!(no[1]["why"].as_str().is_some_and(|w| !w.is_empty()));
    assert_eq!(run("/nonexistent.sock", &["open-check", "--path", "/nonexistent/x.md"])[1]["why"], "File not found.");

    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "blocked", "permission"), common::terminal("t2", &["zsh"], Some(9), None, None)]);
    fake.set_screen("t1", "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No");
    assert_eq!(run(&fake.path, &["card", "--terminal", "t9"])[1]["code"], "no-terminal");
    assert_eq!(run(&fake.path, &["card", "--terminal", "t2"])[1]["code"], "not-agent");
    let c = run(&fake.path, &["card", "--terminal", "t1"]);
    assert_eq!(c[1]["type"], "card");
    assert_eq!(c[1]["kind"], "approval");
    assert_eq!(c[1]["tool"], "Bash");
    assert_eq!(c[1]["command"], "rm -rf build");
    assert!(fake.keys().is_empty(), "card sends no keys");
    assert_eq!(run(&fake.path, &["approve", "--terminal", "t1", "--request", "x"])[1]["code"], "usage");
    let a = run(&fake.path, &["approve", "--terminal", "hc:t1", "--request", c[1]["request"].as_str().unwrap(), "--deny"]);
    assert_eq!(a[1], serde_json::json!({"type":"done","result":"sent"}));
    assert_eq!(fake.keys(), vec![vec!["esc".to_string()]]);
    // No log found: path null, nothing to splice.
    let f = run(&fake.path, &["feed", "--terminal", "t1"]);
    assert_eq!(f[1], serde_json::json!({"type":"log","agent":"claude","path":null}));
    assert_eq!(f[2], serde_json::json!({"type":"splice","at":0,"count":0}));
}

/// `commands`: a Claude Code terminal's "/" commands, read from files in its folder and the user's home. No key goes
/// to the agent. Codex, grok and omp terminals get their own lists. Not an agent: an error.
#[test]
fn commands_lists_a_claude_terminals_slash_commands() {
    let t = tempfile::tempdir().unwrap();
    let (home, proj) = (t.path().join("home"), t.path().join("proj"));
    std::fs::create_dir_all(proj.join(".claude/commands")).unwrap();
    std::fs::write(proj.join(".claude/commands/deploy.md"), "---\ndescription: Deploy the app\n---\n").unwrap();
    std::fs::create_dir_all(home.join(".claude/commands")).unwrap();
    std::fs::write(home.join(".claude/commands/init.md"), "Make my own CLAUDE.md.").unwrap();
    let fake = FakeHc::start();
    let mut c = claude("t1", "idle", "idle");
    c["cwd"] = serde_json::json!(proj);
    let mut x = common::terminal("t2", &["codex"], Some(77), Some("idle"), None);
    x["cwd"] = serde_json::json!(proj);
    std::fs::create_dir_all(proj.join(".grok/commands")).unwrap();
    std::fs::write(proj.join(".grok/commands/ship.md"), "Ship it").unwrap();
    let mut g = common::terminal("t4", &["grok"], Some(78), Some("idle"), None);
    g["cwd"] = serde_json::json!(proj);
    let mut m = common::terminal("t5", &["omp"], Some(79), Some("idle"), None);
    m["cwd"] = serde_json::json!(proj);
    fake.set_terminals(vec![c, x, common::terminal("t3", &["zsh"], Some(9), None, None), g, m]);
    let run_home = |args: &[&str]| -> Vec<Value> {
        let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(args).env_clear().env("HC_SOCKET", &fake.path).env("HOME", &home).output().unwrap();
        String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    };
    let v = run_home(&["commands", "--terminal", "t1"]);
    assert_eq!(v[1]["type"], "commands");
    assert_eq!(v[1]["agent"], "claude");
    let cmds = v[1]["commands"].as_array().unwrap();
    let own: Vec<&Value> = cmds.iter().filter(|c| c["source"] != "builtin").collect();
    assert_eq!(own, vec![&serde_json::json!({"name": "deploy", "description": "Deploy the app", "source": "project"})], "the user's init is hidden by the built-in /init");
    let model = cmds.iter().find(|c| c["name"] == "model").unwrap();
    assert_eq!(model["args"][1], serde_json::json!({"value": "claude-opus-5-5", "label": "Opus 5.5"}), "full ids, with labels");
    assert!(model["args"].as_array().unwrap().contains(&serde_json::json!({"value": "opus", "label": "The latest Opus"})));
    let effort = cmds.iter().find(|c| c["name"] == "effort").unwrap();
    assert_eq!(effort["args"][0], serde_json::json!({"value": "low"}), "no label where the value says it all");
    assert_eq!(model["screen"], true);
    assert_eq!(v.last().unwrap()["ok"], true);
    let o = run_home(&["commands", "--terminal", "t2"]);
    assert_eq!(o[1]["agent"], "codex");
    let codex: Vec<&str> = o[1]["commands"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(codex.contains(&"compact") && !codex.contains(&"deploy"), "Codex: its built-ins, not Claude's files: {codex:?}");
    assert!(!codex.contains(&"model"), "Codex's /model takes no value: it can never be sent from the chat");
    assert_eq!(o[1]["screens"], serde_json::json!(["agents", "config", "copy", "memories", "model", "permissions", "presets", "resume", "skills", "subagents", "usage", "warnings"]));
    assert!(v[1]["screens"].as_array().unwrap().contains(&serde_json::json!("status")));
    let g = run_home(&["commands", "--terminal", "t4"]);
    assert_eq!(g[1]["agent"], "grok");
    let grok: Vec<&str> = g[1]["commands"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(grok.contains(&"compact") && grok.contains(&"ship") && !grok.contains(&"deploy"), "grok: its built-ins and its own commands: {grok:?}");
    let m = run_home(&["commands", "--terminal", "t5"]);
    assert_eq!(m[1]["agent"], "omp");
    assert!(m[1]["commands"].as_array().unwrap().iter().all(|c| c["source"] == "builtin"), "omp: its built-ins only");
    assert!(m[1]["screens"].as_array().unwrap().contains(&serde_json::json!("hotkeys")));
    assert_eq!(run_home(&["commands", "--terminal", "t3"])[1]["code"], "not-agent");
    assert_eq!(run_home(&["commands"])[1]["message"], "commands needs --terminal <addr>");
    assert!(fake.keys().is_empty(), "commands sends no keys");
}

/// `send` of a screen command sent bare ("/model", "/status"): refused with `opens_screen`, nothing goes to the
/// agent. With a value, or for an agent whose command prints in the chat, it goes out.
#[test]
fn send_refuses_a_bare_screen_command() {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "idle"), common::terminal("t2", &["codex"], Some(77), Some("idle"), None)]);
    let send = |t: &str, text: &str| -> Value {
        let v = run(&fake.path, &["send", "--terminal", t, "--text", text]);
        v.into_iter().find(|l| l["type"] == "done").unwrap()
    };
    let d = send("t1", " /model ");
    assert_eq!(d["result"], "opens_screen");
    assert_eq!(d["message"], "/model opens a menu in the terminal. Add a value, like /model claude-fable-5-2.");
    assert_eq!(send("t1", "/status")["message"], "/status opens a screen in the terminal. Use it there.");
    assert_eq!(send("t2", "/model")["result"], "opens_screen");
    assert!(fake.calls("terminal.send").is_empty(), "nothing was typed");
    assert_ne!(send("t1", "/model sonnet")["result"], "opens_screen");
    assert_ne!(send("t2", "/status")["result"], "opens_screen", "Codex's /status prints in the chat");
}

/// `send` reads the agent's input box first: text in it (a half-typed "/") and nothing is typed (`box_not_empty`).
/// An empty box, its placeholder, or no screen to read: it goes as before.
#[test]
fn send_refuses_when_the_agents_box_has_text() {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "idle")]);
    let rule = "─".repeat(40);
    let screen = |row: &str| format!("● Done.\n\n{rule}\n{row}\n{rule}\n  ~/proj\n");
    let send = || -> Value { run(&fake.path, &["send", "--terminal", "t1", "--text", "/compact"]).into_iter().find(|l| l["type"] == "done").unwrap() };
    fake.set_screen("t1", &screen("❯\u{a0}/"));
    let d = send();
    assert_eq!(d["result"], "box_not_empty");
    assert_eq!(d["message"], "The agent's box has text in it. Clear it in the terminal first. Nothing was sent.");
    assert!(fake.calls("terminal.send").is_empty(), "nothing was typed");
    fake.set_screen("t1", &screen("❯\u{a0}Try \"fix typecheck errors\""));
    assert_ne!(send()["result"], "box_not_empty", "only its placeholder: empty");
    let typed = fake.calls("terminal.send").len();
    assert!(typed >= 1);
    fake.clear_screen("t1");
    assert_ne!(send()["result"], "box_not_empty", "no screen: it goes as before");
    assert!(fake.calls("terminal.send").len() > typed);
}


/// `resources --cache`: the state kept beside the cache (one file per terminal) makes the next call parse only the new
/// lines, with the same answer as the whole log.
#[test]
fn resources_keeps_a_state_beside_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache.json");
    let cache_s = cache.to_string_lossy().to_string();
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "idle")]);
    run(&fake.path, &["glance", "--cache", &cache_s]);
    let log = dir.path().join("s.jsonl");
    let say = |id: &str, text: &str| serde_json::json!({"type":"assistant","uuid":id,"timestamp":"2026-11-03T01:40:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":text}]}}).to_string();
    let lines: Vec<String> = (0..40).map(|i| say(&format!("a{i}"), &format!("Step {i}: https://s{}.example.com", i % 5))).collect();
    std::fs::write(&log, lines.join("\n") + "\n").unwrap();
    let mut c: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    c["entries"]["t1"]["path"] = serde_json::json!(log);
    std::fs::write(&cache, c.to_string()).unwrap();
    let res = |v: &[Value]| -> Vec<Value> { v.iter().filter(|l| l["type"] == "res").cloned().collect() };
    let first = run(&fake.path, &["resources", "--terminal", "t1", "--cache", &cache_s]);
    assert_eq!(first.last().unwrap()["ok"], true, "{first:?}");
    assert_eq!(res(&first).len(), 5);
    let kept_dir = dir.path().join("resources");
    let files: Vec<_> = std::fs::read_dir(&kept_dir).expect("resources/ beside the cache").flatten().map(|e| e.path()).collect();
    assert_eq!(files.len(), 1, "one file for the terminal");
    let state: Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
    let at = state["at"].as_u64().unwrap();
    assert!(at > 0, "{state}");
    // More lines: parsed from the kept start, the same answer as the whole log.
    std::fs::OpenOptions::new().append(true).open(&log).unwrap().write_all(format!("{}\n", say("b1", "New https://new.example.com and https://s1.example.com")).as_bytes()).unwrap();
    let more = run(&fake.path, &["resources", "--terminal", "t1", "--cache", &cache_s]);
    let whole = modfeedkit::resources::answer(&log, None, Some(std::path::Path::new("/nonexistent-home"))).unwrap();
    let want: Vec<Value> = whole.items.iter().map(|r| serde_json::to_value(r).unwrap()).collect();
    assert_eq!(res(&more), want);
    assert_eq!(more.last().unwrap()["cursor"], serde_json::json!(whole.cursor));
    let state: Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).unwrap();
    assert!(state["at"].as_u64().unwrap() > at, "the start moved on: {state}");
    // A broken state file is no state: the whole log is read, the answer is the same, and a good file is written again.
    std::fs::write(&files[0], b"{\"rules\":1,\"at\":").unwrap();
    let broken = run(&fake.path, &["resources", "--terminal", "t1", "--cache", &cache_s]);
    assert_eq!(res(&broken), want);
    let again: Value = serde_json::from_slice(&std::fs::read(&files[0]).unwrap()).expect("written whole again");
    assert!(again["at"].as_u64().unwrap() > 0, "{again}");
}
