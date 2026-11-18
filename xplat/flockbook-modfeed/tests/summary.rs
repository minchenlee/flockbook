//! `summary`: two lines by the agent's own small model, only with summaries on. The model here is a shell script.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn user(id: &str, text: &str) -> String {
    json!({"type": "user", "uuid": id, "timestamp": "2026-11-03T01:40:00.000Z", "message": {"role": "user", "content": text}}).to_string()
}
fn say(id: &str, text: &str) -> String {
    json!({"type": "assistant", "uuid": id, "timestamp": "2026-11-03T01:41:00.000Z", "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}}).to_string()
}
fn tool(id: &str, call: &str, name: &str, input: Value) -> String {
    json!({"type": "assistant", "uuid": id, "timestamp": "2026-11-03T01:40:30.000Z", "message": {"role": "assistant", "content": [{"type": "tool_use", "id": call, "name": name, "input": input}]}}).to_string()
}
fn result(id: &str, call: &str) -> String {
    json!({"type": "user", "uuid": id, "timestamp": "2026-11-03T01:40:40.000Z", "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": call, "content": "ok", "is_error": false}]}}).to_string()
}

fn run(sock: &str, cli: &Path, args: &[&str]) -> Vec<Value> {
    let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(args).env_clear().env("HC_SOCKET", sock).env("HOME", "/nonexistent-home").env("FLOCKBOOK_SUMMARY_CLI", cli).output().unwrap();
    assert!(out.status.success(), "exit code 0 always");
    String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// A program in place of the agent's: it keeps its arguments and its stdin in `dir`, counts its runs, then runs `body`.
fn fake_cli(dir: &Path, body: &str) -> std::path::PathBuf {
    let p = dir.join("fake-model");
    let d = dir.display();
    std::fs::write(&p, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{d}/argv'\ncat > '{d}/stdin'\necho run >> '{d}/runs'\n{body}\n")).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

struct World {
    dir: tempfile::TempDir,
    fake: FakeHc,
    home: String,
    cache: String,
}

/// One Claude Code agent (t1) with a log, a Codex agent (t3), and this machine for `machine.exec`.
fn world(summaries: Option<&str>, turn: &[String]) -> World {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "idle"), common::terminal("t3", &["codex"], Some(77), Some("idle"), None)]);
    fake.add_machine("", dir.path(), &fake.path);
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join("flockbook")).unwrap();
    if let Some(s) = summaries {
        std::fs::write(home.join("flockbook/config.json"), json!({"summaries": s}).to_string()).unwrap();
    }
    let cache = home.join("flockbook/cache.json");
    let log = dir.path().join("s.jsonl");
    std::fs::write(&log, turn.join("\n") + "\n").unwrap();
    let w = World { home: home.to_string_lossy().into(), cache: cache.to_string_lossy().into(), dir, fake };
    // `glance` writes the cache's entries; then the test names t1's log in it, as tests/glance.rs does.
    let cli = w.dir.path().join("none");
    run(&w.fake.path, &cli, &["glance", "--cache", &w.cache, "--home", &w.home]);
    let mut c: Value = serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    c["entries"]["t1"]["path"] = json!(log);
    std::fs::write(&cache, c.to_string()).unwrap();
    w
}

fn turn() -> Vec<String> {
    vec![
        user("u0", "An older turn"),
        say("x0", "OLD-REPLY that must not be sent."),
        user("u1", "Fix the importer so it reads empty lines."),
        tool("a1", "c1", "Edit", json!({"file_path": "/w/src/importer.py", "old_string": "a", "new_string": "b"})),
        result("r1", "c1"),
        tool("a2", "c2", "Bash", json!({"command": "pytest -q"})),
        result("r2", "c2"),
        say("a3", "The parser now skips empty lines. All 12 tests pass.\n\nShould I also update the changelog?"),
    ]
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

#[test]
fn off_unless_the_person_turned_it_on() {
    for setting in [None, Some("off")] {
        let w = world(setting, &turn());
        let cli = fake_cli(w.dir.path(), "echo 'STATE: x'");
        let v = run(&w.fake.path, &cli, &["summary", "--terminal", "t1", "--cache", &w.cache, "--home", &w.home]);
        assert_eq!(v[1]["code"], "summaries-off", "{v:?}");
        assert_eq!(v.last().unwrap()["ok"], false);
        assert!(w.fake.calls("machine.exec").is_empty(), "no program runs");
        assert_eq!(read(w.dir.path(), "runs"), "");
    }
    // No --home: no settings are found, so it stays off.
    let w = world(Some("on"), &turn());
    let cli = fake_cli(w.dir.path(), "echo 'STATE: x'");
    assert_eq!(run(&w.fake.path, &cli, &["summary", "--terminal", "t1", "--cache", &w.cache])[1]["code"], "summaries-off");
}

#[test]
fn the_last_turn_goes_to_the_small_model_once() {
    let w = world(Some("on"), &turn());
    let cli = fake_cli(w.dir.path(), "echo 'a plugin wrote this line'\necho 'STATE: The parser skips empty lines and the tests pass.'\necho 'NEEDS: Say whether to update the changelog.'");
    let args = ["summary", "--terminal", "t1", "--cache", &w.cache, "--home", &w.home];
    assert!(run(&w.fake.path, &cli, &["version"])[1]["commands"].as_array().unwrap().contains(&json!("summary")));
    let v = run(&w.fake.path, &cli, &args);
    assert_eq!(v[1], json!({"type": "summary", "terminal": "t1", "id": "a3-0", "state": "The parser skips empty lines and the tests pass.",
        "needs": "Say whether to update the changelog.", "model": "claude-haiku-5-5", "by": "claude", "cached": false}), "{v:?}");
    assert_eq!(v.last().unwrap()["ok"], true);
    // What ran: the agent's program with no tools and no saved session, without a terminal.
    let argv = read(w.dir.path(), "argv");
    let lines: Vec<&str> = argv.lines().collect();
    assert_eq!(&lines[..4], ["-p", "--model", "claude-haiku-5-5", "--no-session-persistence"]);
    assert!(argv.contains("--tools\n\n--disable-slash-commands\n--strict-mcp-config\n--setting-sources\n\n--system-prompt\n"), "{argv}");
    assert!(!argv.contains("--bare"));
    let exec = w.fake.calls("machine.exec");
    assert_eq!(exec.len(), 1);
    assert_eq!((exec[0].get("on"), exec[0].get("line"), exec[0]["timeout"].as_u64()), (None, None, Some(60)), "this machine, no shell");
    // What it read: the owner's message, what the agent wrote in this turn, the facts. Not the owner's messages or
    // what the agent wrote in a turn before.
    let stdin = read(w.dir.path(), "stdin");
    assert!(stdin.starts_with("OWNER ASKED:\nFix the importer so it reads empty lines.\n\nAGENT WROTE:\nThe parser now skips empty lines."), "{stdin}");
    assert!(stdin.ends_with("FROM THE LOG: edited 1 files; ran 1 commands; tests passed (pytest)\n"), "{stdin}");
    assert!(!stdin.contains("OLD-REPLY") && !stdin.contains("An older turn"));
    // The run is counted: how much went and came, not the words.
    let usage = |since: &str| run(&w.fake.path, &cli, &["summary-usage", "--cache", &w.cache, "--since", since, "--home", &w.home])[1].clone();
    let u = usage("0");
    assert_eq!((u["type"].as_str(), u["runs"].as_i64(), u["failed"].as_i64()), (Some("summary-usage"), Some(1), Some(0)), "{u}");
    assert_eq!(u["sent"].as_i64(), Some((stdin.chars().count() + modfeedkit::summary::SYSTEM.chars().count()) as i64));
    assert!(u["got"].as_i64().unwrap() > 100);
    assert_eq!(usage("99999999999999")["runs"], 0, "none from a time that is still to come");
    let counted = std::fs::read_to_string(Path::new(&w.cache).with_file_name("summary-usage.jsonl")).unwrap();
    assert!(!counted.contains("importer") && !counted.contains("changelog"), "no word of the turn or the answer: {counted}");
    // The same turn again: the kept answer, and the model does not run.
    let again = run(&w.fake.path, &cli, &args);
    assert_eq!((again[1]["cached"].as_bool(), &again[1]["state"]), (Some(true), &v[1]["state"]));
    assert_eq!(read(w.dir.path(), "runs").lines().count(), 1);
    assert_eq!(usage("0")["runs"], 1, "a kept answer is not a run");
    // The agent wrote more: asked again.
    let mut more = turn();
    more.push(user("u2", "Yes, update it."));
    more.push(say("a4", "The changelog has the new line."));
    std::fs::write(w.dir.path().join("s.jsonl"), more.join("\n") + "\n").unwrap();
    let cli = fake_cli(w.dir.path(), "echo 'STATE: The changelog is updated.'\necho 'NEEDS: nothing'");
    let v = run(&w.fake.path, &cli, &args);
    assert_eq!((v[1]["id"].as_str(), v[1]["state"].as_str(), &v[1]["needs"], v[1]["cached"].as_bool()), (Some("a4-0"), Some("The changelog is updated."), &Value::Null, Some(false)));
    assert!(v[1].get("task").is_none() && v[1].get("waits").is_none(), "{v:?}");
    assert!(read(w.dir.path(), "stdin").starts_with("OWNER ASKED:\nYes, update it.\n"));
    // An answer kept by a helper that asked for other lines is asked for again.
    let kept_at = Path::new(&w.cache).with_file_name("summary.json");
    let mut kept: Value = serde_json::from_slice(&std::fs::read(&kept_at).unwrap()).unwrap();
    kept["t1"].as_object_mut().unwrap().remove("v");
    std::fs::write(&kept_at, kept.to_string()).unwrap();
    assert_eq!(run(&w.fake.path, &cli, &args)[1]["cached"], false);
    assert_eq!(run(&w.fake.path, &cli, &args)[1]["cached"], true);
    assert!(w.fake.keys().is_empty() && w.fake.calls("terminal.send").is_empty(), "nothing goes to the agent's terminal");
}

#[test]
fn the_person_chooses_the_program_and_its_model() {
    let w = world(Some("on"), &turn());
    let set = |v: Value| std::fs::write(Path::new(&w.home).join("flockbook/config.json"), v.to_string()).unwrap();
    let args = ["summary", "--terminal", "t1", "--cache", &w.cache, "--home", &w.home];
    let cli = fake_cli(w.dir.path(), "echo 'STATE: Done by another program.'\necho 'NEEDS: nothing'");
    // Codex writes about a Claude Code agent's turn: a sandbox that only reads, the chosen model, the instructions on stdin.
    set(json!({"summaries": "on", "summary_by": "codex", "summary_model": "gpt-x-mini"}));
    let v = run(&w.fake.path, &cli, &args);
    assert_eq!((v[1]["by"].as_str(), v[1]["model"].as_str(), v[1]["cached"].as_bool()), (Some("codex"), Some("gpt-x-mini"), Some(false)), "{v:?}");
    let argv = read(w.dir.path(), "argv");
    assert!(argv.starts_with("exec\n--sandbox\nread-only\n--ephemeral\n") && argv.ends_with("-m\ngpt-x-mini\n-\n"), "{argv}");
    let stdin = read(w.dir.path(), "stdin");
    assert!(stdin.starts_with("You get the end of one turn") && stdin.contains("\n\nOWNER ASKED:\nFix the importer"), "{stdin}");
    assert_eq!(run(&w.fake.path, &cli, &args)[1]["cached"], true);
    // Another program or another model: the kept answer is not theirs, so the turn is asked again.
    set(json!({"summaries": "on", "summary_by": "pi"}));
    let v = run(&w.fake.path, &cli, &args);
    assert_eq!((v[1]["by"].as_str(), v[1]["model"].as_str(), v[1]["cached"].as_bool()), (Some("pi"), Some("pi's default model"), Some(false)));
    let argv = read(w.dir.path(), "argv");
    assert!(argv.starts_with("-p\n--no-session\n--no-tools\n") && !argv.contains("--model"), "{argv}");
    assert!(read(w.dir.path(), "stdin").starts_with("OWNER ASKED:"));
    // A model name that is not one plain word is not used; each agent's own program runs its default.
    set(json!({"summaries": "on", "summary_by": "claude", "summary_model": "--dangerously-skip-permissions"}));
    assert_eq!(run(&w.fake.path, &cli, &args)[1]["model"], "claude-haiku-5-5");
    set(json!({"summaries": "on", "summary_model": "gpt-x-mini"}));
    let v = run(&w.fake.path, &cli, &args);
    assert_eq!((v[1]["by"].as_str(), v[1]["model"].as_str(), v[1]["cached"].as_bool()), (Some("claude"), Some("claude-haiku-5-5"), Some(true)), "own: the agent's program, its default model");
    assert_eq!(read(w.dir.path(), "runs").lines().count(), 3);
}

#[test]
fn a_failure_is_an_error_line_and_nothing_is_kept() {
    let w = world(Some("on"), &turn());
    let args = ["summary", "--terminal", "t1", "--cache", &w.cache, "--home", &w.home];
    // The program fails: its first line of stderr is the reason.
    let cli = fake_cli(w.dir.path(), "echo 'Not logged in' >&2\nexit 1");
    let v = run(&w.fake.path, &cli, &args);
    assert_eq!((v[1]["code"].as_str(), v[1]["message"].as_str()), (Some("summary-failed"), Some("The model's program failed. Not logged in")));
    // An answer in another shape.
    let cli = fake_cli(w.dir.path(), "echo 'Sure! Here is a summary of the work.'");
    assert_eq!(run(&w.fake.path, &cli, &args)[1]["message"], "The model's answer had no STATE line.");
    // No such program.
    let v = run(&w.fake.path, &w.dir.path().join("missing"), &args);
    assert_eq!(v[1]["code"], "summary-failed");
    assert!(v[1]["message"].as_str().unwrap().contains("was not found"), "{v:?}");
    // It ran out of time.
    w.fake.state.lock().unwrap().exec_timed_out = true;
    let cli = fake_cli(w.dir.path(), "echo 'STATE: x'");
    assert_eq!(run(&w.fake.path, &cli, &args)[1]["message"], "The model did not answer in 60 s.");
    w.fake.state.lock().unwrap().exec_timed_out = false;
    assert!(!Path::new(&w.cache).with_file_name("summary.json").exists(), "a failure is asked again next time");
    // Then it works.
    assert_eq!(run(&w.fake.path, &cli, &args)[1]["state"], "x");
}

#[test]
fn no_summary_for_other_agents_other_machines_or_an_empty_turn() {
    let w = world(Some("on"), &[user("u1", "Go")]);
    let cli = fake_cli(w.dir.path(), "echo 'STATE: x'");
    let at = |t: &str| run(&w.fake.path, &cli, &["summary", "--terminal", t, "--cache", &w.cache, "--home", &w.home])[1].clone();
    assert_eq!(at("t1")["code"], "summary-none", "the agent wrote nothing yet");
    assert_eq!(at("t3")["code"], "summary-none", "a Codex agent with no conversation");
    assert_eq!(at("box/t1")["code"], "summary-unsupported");
    assert_eq!(at("t9")["type"], "error");
    assert_eq!(read(w.dir.path(), "runs"), "", "the model ran for none of them");
}

#[test]
fn the_models_a_program_can_run() {
    let w = world(Some("on"), &turn());
    let cli = fake_cli(w.dir.path(), "printf 'provider  model  context\\nopencode-go  glm-5.3-flash  1M\\n'");
    let codex = w.dir.path().join("codex-home");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::write(codex.join("models_cache.json"), json!({"models": [{"slug": "gpt-x", "description": "Workhorse.", "visibility": "list"}, {"slug": "gpt-h", "visibility": "hide"}]}).to_string()).unwrap();
    let at = |by: &str| {
        let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(["summary-models", "--by", by]).env_clear().env("HC_SOCKET", &w.fake.path).env("HOME", "/nonexistent-home")
            .env("CODEX_HOME", &codex).env("FLOCKBOOK_SUMMARY_CLI", &cli).output().unwrap();
        String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).nth(1).unwrap()
    };
    assert_eq!(at("pi"), json!({"type": "models", "by": "pi", "source": "program", "models": [{"id": "opencode-go/glm-5.3-flash", "note": "1M context"}]}));
    assert_eq!(read(w.dir.path(), "argv"), "--offline\n--list-models\n");
    assert_eq!(at("codex"), json!({"type": "models", "by": "codex", "source": "file", "models": [{"id": "gpt-x", "note": "Workhorse."}]}));
    let c = at("claude");
    assert_eq!((c["source"].as_str(), c["models"][0]["id"].as_str()), (Some("built-in"), Some("claude-haiku-5-5")));
    assert_eq!(at("bash")["code"], "usage");
    assert_eq!(w.fake.calls("machine.exec").len(), 1, "only pi runs a program");
}
