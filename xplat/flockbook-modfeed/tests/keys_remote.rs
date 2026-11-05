//! `cli::keys` (what `serve` runs for approve, answer and send) on a terminal `m/tN` on another machine: the helper
//! there does it, as `send --terminal m/tN` does. One test: it sets this process's HC_SOCKET (`cli::keys` runs here,
//! not in a child), so nothing else may run beside it.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use modfeedkit::cli::{self, Keys};
use modfeedkit::remote;
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;

const BOX: &str = "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No";

fn carried() -> PathBuf {
    std::env::var_os("FLOCKBOOK_TEST_HELPER").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_flockbook-modfeed")))
}

#[test]
fn keys_to_another_machine_go_to_the_helper_there() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("box-home");
    let here = FakeHc::start();
    let there = FakeHc::start();
    there.set_terminals(vec![claude("t3", "blocked", "permission")]);
    there.set_screen("t3", BOX);
    here.add_machine("box", &home, &there.path);
    let installed = home.join(remote::HELPER_DIR).join(remote::helper_version()).join("flockbook-modfeed");
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::copy(carried(), &installed).unwrap();
    std::env::set_var("HC_SOCKET", &here.path);
    std::env::remove_var("HC_SESSION");
    let cache = dir.path().join("cache.json");

    // The card there gives the request id; approve with it goes to t3 there, nothing here.
    let card = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(["card", "--terminal", "box/t3"]).env("HC_SOCKET", &here.path).output().unwrap();
    let card: serde_json::Value = String::from_utf8(card.stdout).unwrap().lines().nth(1).map(|l| serde_json::from_str(l).unwrap()).unwrap();
    let req = card["request"].as_str().unwrap().to_string();
    let d = cli::keys("box/t3", Some(&cache), None, Keys::Approve { request: req.clone(), allow: true });
    assert_eq!(d, json!({"type": "done", "result": "sent"}), "approve there");
    assert_eq!(there.keys(), vec![vec!["1".to_string()]]);
    assert!(here.keys().is_empty(), "no key here");

    // Send: the text reaches the agent there unchanged, even text that looks like an option.
    there.set_terminals(vec![claude("t3", "idle", "")]);
    let text = "--allow --terminal t9 --home /x --file /etc/passwd --text";
    let d = cli::keys("box/t3", Some(&cache), None, Keys::Send { text: text.into(), expect_kind: Some("claude".into()), expect_path: None });
    assert_eq!(d["result"], "sent", "{d}");
    assert_eq!(there.calls("terminal.send").last().unwrap()["text"], text);
    assert_eq!(there.calls("terminal.send").last().unwrap()["terminal"], "t3");
    assert!(here.calls("terminal.send").is_empty());

    // Another agent kind there: nothing is sent.
    let d = cli::keys("box/t3", Some(&cache), None, Keys::Send { text: "hi".into(), expect_kind: Some("codex".into()), expect_path: None });
    assert_ne!(d["result"], "sent", "{d}");

    // Answer: the answers reach the helper there as it reads them (there is no question, so it says so).
    let a = vec![herdr_core::QuestionAnswer { picks: vec![0], other: Some("--deny".into()) }];
    let d = cli::keys("box/t3", Some(&cache), None, Keys::Answer { request: "screen-1".into(), answers: a });
    assert_eq!(d["type"], "done", "a done line from there, not a usage error: {d}");
    assert_ne!(d["result"], "sent");

    // A value that would be read as an option there is refused here; nothing runs there.
    let runs = here.calls("machine.exec").len();
    let d = cli::keys("box/t3", Some(&cache), None, Keys::Approve { request: "--deny".into(), allow: true });
    assert_eq!(d["type"], "error", "{d}");
    let d = cli::keys("box/t3", Some(&cache), None, Keys::Send { text: "hi".into(), expect_kind: Some("claude".into()), expect_path: Some("--home".into()) });
    assert_eq!(d["type"], "error", "{d}");
    assert_eq!(here.calls("machine.exec").len(), runs, "nothing ran there");

    // A bad address: an error, not a lookup here.
    assert_eq!(cli::keys("a/b/c", Some(&cache), None, Keys::Approve { request: "r".into(), allow: true })["code"], "bad_terminal");
}
