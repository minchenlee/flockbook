//! The built binary end to end, on a fake herdr-core: the protocol's header and end, errors, and a card.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use serde_json::Value;
use std::process::Command;

fn run(sock: &str, args: &[&str]) -> Vec<Value> {
    run_in(sock, args, "/nonexistent-home")
}

/// `run` with HOME at `home` (its ~/.claude/settings.json).
fn run_in(sock: &str, args: &[&str], home: &str) -> Vec<Value> {
    let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
        .args(args)
        .env_clear()
        .env("HC_SOCKET", sock)
        .env("HOME", home)
        .output()
        .unwrap();
    assert!(out.status.success(), "exit code 0 always");
    String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn protocol_lines() {
    let v = run("/nonexistent.sock", &["version"]);
    assert_eq!(v[0], serde_json::json!({"type":"header","protocol":1,"sidecar":"0.1.0","cmd":"version"}));
    assert_eq!(v[1]["commands"], serde_json::json!(["version", "feed", "status", "card", "approve", "answer", "send", "paste-image", "helper", "update-check", "queue", "phone", "commands"]));
    assert_eq!(v.last().unwrap()["type"], "end");
    assert_eq!(v.last().unwrap()["ok"], true);

    let u = run("/nonexistent.sock", &["nope"]);
    assert_eq!(u[1]["code"], "usage");
    assert_eq!(u.last().unwrap()["ok"], false);
    assert_eq!(run("/nonexistent.sock", &["feed"])[1]["message"], "feed needs --terminal <addr>");
    assert_eq!(run("/nonexistent.sock", &["feed", "--terminal", "t1"])[1]["code"], "no-core");

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
/// to the agent. Another agent: an empty list (the mod shows nothing).
#[test]
fn commands_lists_a_claude_terminals_slash_commands() {
    let t = tempfile::tempdir().unwrap();
    let (home, proj) = (t.path().join("home"), t.path().join("proj"));
    std::fs::create_dir_all(proj.join(".claude/commands")).unwrap();
    std::fs::write(proj.join(".claude/commands/deploy.md"), "---\ndescription: Deploy the app\n---\n").unwrap();
    std::fs::create_dir_all(home.join(".claude/commands")).unwrap();
    std::fs::write(home.join(".claude/commands/review.md"), "Review the diff.").unwrap();
    let fake = FakeHc::start();
    let mut c = claude("t1", "idle", "idle");
    c["cwd"] = serde_json::json!(proj);
    let mut x = common::terminal("t2", &["codex"], Some(77), Some("idle"), None);
    x["cwd"] = serde_json::json!(proj);
    fake.set_terminals(vec![c, x, common::terminal("t3", &["zsh"], Some(9), None, None)]);
    let run_home = |args: &[&str]| -> Vec<Value> {
        let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(args).env_clear().env("HC_SOCKET", &fake.path).env("HOME", &home).output().unwrap();
        String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    };
    let v = run_home(&["commands", "--terminal", "t1"]);
    assert_eq!(v[1]["type"], "commands");
    assert_eq!(v[1]["agent"], "claude");
    let cmds = v[1]["commands"].as_array().unwrap();
    let own: Vec<&Value> = cmds.iter().filter(|c| c["source"] != "builtin").collect();
    assert_eq!(own, vec![&serde_json::json!({"name": "deploy", "description": "Deploy the app", "source": "project"})], "the user's review is hidden by the built-in /review");
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
    assert_eq!(o[1]["screens"], serde_json::json!(["config", "model", "permissions", "resume", "skills"]));
    assert!(v[1]["screens"].as_array().unwrap().contains(&serde_json::json!("status")));
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

/// Claude Code's dim prompt suggestion can be a command ("/clear"), which reads like a typed one. A lone "/word" with
/// no command list above the box (a typed "/" shows the list): `send` clears the line (ctrl+e ctrl+u), reads the box
/// again and sends when it is down to one row. A command list above, a box still holding text after the keys, or
/// prompt suggestions turned off: `box_not_empty`, as before.
#[test]
fn send_types_over_a_suggested_command() {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "idle")]);
    let rule = "─".repeat(40);
    let screen = |above: &str, row: &str| format!("● Done.\n{above}\n{rule}\n{row}\n{rule}\n  ~/proj\n");
    let send_in = |home: &str| -> Value {
        run_in(&fake.path, &["send", "--terminal", "t1", "--text", "run the tests"], home).into_iter().find(|l| l["type"] == "done").unwrap()
    };
    let send = || send_in("/nonexistent-home");
    // ctrl+e ctrl+u leaves the box with its suggestion again (still dim, one row).
    let after = screen("", "❯\u{a0}/clear");
    fake.on_keys(Some(std::sync::Arc::new(move |a, k, st| {
        if a == "t1" && k == ["ctrl+e", "ctrl+u"] {
            st.screens.insert("t1".into(), after.clone());
        }
    })));

    fake.set_screen("t1", &screen("", "❯\u{a0}/clear"));
    assert_eq!(send()["result"], "sent");
    assert_eq!(fake.keys(), vec![vec!["ctrl+e".to_string(), "ctrl+u".to_string()]], "the line was cleared first");
    assert_eq!(fake.calls("terminal.send").len(), 1, "then the text went");

    // Typed: the command list stands above the box. Nothing is touched.
    fake.set_screen("t1", &screen("  ❯ /clear        Start a new session with empty context", "❯\u{a0}/cl"));
    assert_eq!(send()["result"], "box_not_empty");
    assert_eq!(fake.keys().len(), 1, "no keys");
    assert_eq!(fake.calls("terminal.send").len(), 1, "nothing typed");

    // After the keys the box still has a second row (typed text): nothing is sent.
    let left = screen("", "❯\u{a0}\n  second line");
    fake.on_keys(Some(std::sync::Arc::new(move |a, _k, st| {
        if a == "t1" {
            st.screens.insert("t1".into(), left.clone());
        }
    })));
    fake.set_screen("t1", &screen("", "❯\u{a0}/clear"));
    assert_eq!(send()["result"], "box_not_empty");
    assert_eq!(fake.keys().len(), 2, "the keys went");
    assert_eq!(fake.calls("terminal.send").len(), 1, "the text did not");

    // Prompt suggestions off: a "/word" in the box is typed. Nothing is touched.
    let home = std::env::temp_dir().join(format!("flockbook-cli-nosuggest-{}", std::process::id()));
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::write(home.join(".claude/settings.json"), r#"{"promptSuggestionEnabled": false}"#).unwrap();
    fake.set_screen("t1", &screen("", "❯\u{a0}/clear"));
    assert_eq!(send_in(home.to_str().unwrap())["result"], "box_not_empty");
    assert_eq!(fake.keys().len(), 2, "no keys");
    let _ = std::fs::remove_dir_all(&home);
}
