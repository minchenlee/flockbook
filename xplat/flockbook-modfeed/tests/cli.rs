//! The built binary end to end, on a fake herdr-core: the protocol's header and end, errors, and a card.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use serde_json::Value;
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
    let names: Vec<&str> = v[1]["commands"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["clear", "compact", "context", "deploy", "review"]);
    assert_eq!(v[1]["commands"][3], serde_json::json!({"name": "deploy", "description": "Deploy the app", "source": "project"}));
    assert_eq!(v.last().unwrap()["ok"], true);
    let o = run_home(&["commands", "--terminal", "t2"]);
    assert_eq!(o[1], serde_json::json!({"type": "commands", "agent": "codex", "commands": []}), "another agent: nothing");
    assert_eq!(run_home(&["commands", "--terminal", "t3"])[1]["code"], "not-agent");
    assert_eq!(run_home(&["commands"])[1]["message"], "commands needs --terminal <addr>");
    assert!(fake.keys().is_empty(), "commands sends no keys");
}
