//! The commands; returns the whole answer as text.
//!
//!   flockbook-modfeed version
//!   flockbook-modfeed feed   --terminal <addr> [--cursor <c>] [--cache <file>]
//!   flockbook-modfeed feed   --terminal <addr> --before <start> --path <log>   (older rows; no cache change)
//!   flockbook-modfeed status --terminal <addr> [--cache <file>]
//!   flockbook-modfeed card   --terminal <addr> [--all] [--answers <json>] [--cache <file>]   (--all: load every question; sends keys;
//!                            --answers: the answers so far, for the `blocked` line)
//!   flockbook-modfeed approve --terminal <addr> --request <id> --allow|--deny [--cache <file>]
//!   flockbook-modfeed answer --terminal <addr> --request <id> --answers <json> [--cache <file>]
//!   flockbook-modfeed queue  --terminal <addr> [--cache <file>]   (the agent's own queue: one `queued` line; read only)
//!   flockbook-modfeed paste-image --dir <folder>   (copied files, else the clipboard's image as a PNG there; path null when none)
//!   flockbook-modfeed send   --terminal <addr> --text <s> [--file <path>]... [--expect-kind <k>] [--expect-path <log>]
//!   flockbook-modfeed helper install --machine <m> --sha256 <hex>   (the helper on another machine; after the user's yes)
//!   flockbook-modfeed helper remove --machine <m>
//!   flockbook-modfeed update-check --home <herdr-app home> [--dismiss <version>]   (a newer release? once a day)
//!
//! Every command takes `--home <herdr-app home>` (the mod passes it): its flockbook/config.json has the
//! settings the helper follows. Without it, the folder this helper is in when that is <home>/flockbook/bin.
//!
//! A terminal `m/tN` on another machine: the command runs there, in the helper there (`remote`). Rust only.

use crate::protocol::{JsonLines, PROTOCOL_VERSION, SIDECAR_VERSION};
use crate::{cards, config, feed, locate, paste, phone, queue, remote, status, update};
use herdr_core::hc::{HcClient, HcSession};
use herdr_core::PaneScreen;
use herdr_core::resolver::SessionResolver;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

struct Run {
    t0: Instant,
    out: JsonLines,
}

impl Run {
    fn finish(mut self, ok: bool) -> String {
        let ms = self.t0.elapsed().as_millis() as i64;
        self.out.emit_value(json!({"type": "end", "ok": ok, "ms": ms}));
        self.out.text()
    }
    fn fail(mut self, code: &str, message: &str) -> String {
        self.out.emit_value(json!({"type": "error", "code": code, "message": message}));
        self.finish(false)
    }
}

pub fn run(argv: &[String]) -> String {
    let cmd = argv.first().cloned().unwrap_or_default();
    let args: Vec<String> = argv.iter().skip(1).cloned().collect();
    let opt = |name: &str| -> Option<String> {
        let i = args.iter().position(|a| a == name)?;
        args.get(i + 1).cloned()
    };
    let has = |name: &str| args.iter().any(|a| a == name);
    // The mod puts --home last; the last one counts, so a text that says "--home" is not taken for it.
    let home = || args.iter().rposition(|a| a == "--home").and_then(|i| args.get(i + 1).cloned());
    let mut r = Run { t0: Instant::now(), out: JsonLines::new() };
    r.out.emit_value(json!({"type": "header", "protocol": PROTOCOL_VERSION, "sidecar": SIDECAR_VERSION, "cmd": cmd}));
    match cmd.as_str() {
        "version" => {
            r.out.emit_value(json!({"type": "version", "commands": ["version", "feed", "status", "card", "approve", "answer", "send", "paste-image", "helper", "update-check", "queue", "phone", "commands"], "release": update::release()}));
            r.finish(true)
        }
        "paste-image" => {
            let Some(dir) = opt("--dir") else { return r.fail("usage", "paste-image needs --dir <folder>") };
            let png = match paste::clipboard() {
                paste::Clip::Files(paths) => {
                    // Finder or a file manager: attach these. `image` stays for a mod that reads only that line.
                    r.out.emit_value(json!({"type": "files", "paths": paths}));
                    r.out.emit_value(json!({"type": "image", "path": null}));
                    return r.finish(true);
                }
                paste::Clip::Image(png) => png,
                paste::Clip::Nothing => {
                    r.out.emit_value(json!({"type": "image", "path": null}));
                    return r.finish(true);
                }
            };
            match paste::save(&png, Path::new(&dir), SystemTime::now()) {
                Ok(p) => {
                    r.out.emit_value(json!({"type": "image", "path": p}));
                    r.finish(true)
                }
                Err(e) => r.fail("write", &format!("cannot save the image: {e}")),
            }
        }
        "update-check" => {
            let dir = config::dir(home().as_deref());
            let now = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            r.out.emit_value(update::check(dir.as_deref(), now, opt("--dismiss").as_deref()));
            r.finish(true)
        }
        "phone" => {
            let dir = config::dir(home().as_deref());
            let cfg = config::load(dir.as_deref());
            let env = phone::Env::from_process();
            let done = match args.first().map(String::as_str) {
                Some("status") => Ok(()),
                Some("on") => phone::on(&env, &cfg),
                Some("off") => phone::off(&env),
                _ => return r.fail("usage", "phone needs status, on or off"),
            };
            r.out.emit_value(phone::status(&env, &cfg, dir.as_deref()));
            match done {
                Ok(()) => r.finish(true),
                Err((code, message)) => r.fail(code, &message),
            }
        }
        "helper" => {
            let (Some(what), Some(machine)) = (args.first().cloned(), opt("--machine")) else {
                return r.fail("usage", "helper needs install or remove, and --machine <m>");
            };
            if config::load(config::dir(home().as_deref()).as_deref()).remote_never {
                return r.fail("remote-off", config::REMOTE_OFF);
            }
            let session = HcSession::resolve_env();
            if !session.is_available() {
                return r.fail("no-core", "herdr-core is not running");
            }
            let client = HcClient::new(session, 8, 22);
            let cache = opt("--cache").map(PathBuf::from);
            let done = match (what.as_str(), opt("--sha256")) {
                ("install", Some(sha)) => remote::install(&client, &mut r.out, &machine, &sha, cache.as_deref()),
                ("remove", _) => remote::remove(&client, &mut r.out, &machine, cache.as_deref()),
                _ => return r.fail("usage", "helper install needs --sha256 <hex> (the one the user said yes to)"),
            };
            match done {
                Ok(()) => r.finish(true),
                Err((code, message)) => r.fail(code, &message),
            }
        }
        "feed" | "status" | "card" | "approve" | "answer" | "send" | "queue" | "commands" => {
            let Some(terminal) = opt("--terminal") else { return r.fail("usage", &format!("{cmd} needs --terminal <addr>")) };
            let cache_path = opt("--cache").map(PathBuf::from);
            let cache_url = cache_path.as_deref();
            // Another machine, and the person chose Never: nothing runs there, not even a look.
            if matches!(remote::split(&terminal), Ok(Some(_))) && config::load(config::dir(home().as_deref()).as_deref()).remote_never {
                return r.fail("remote-off", config::REMOTE_OFF);
            }
            let session = HcSession::resolve_env();
            if !session.is_available() {
                return r.fail("no-core", "herdr-core is not running");
            }
            match remote::split(&terminal) {
                Err(()) => return r.fail("bad_terminal", &format!("not a terminal address: {terminal}")),
                // On another machine: the helper there answers, and its lines go back as they are.
                // exec gives the sidecar 30 s; a send that waits longer reports `unconfirmed`.
                Ok(Some((machine, there))) => {
                    let client = HcClient::new(session, 8, 22);
                    return remote::forward(&client, &cmd, &args, &machine, &there, cache_url);
                }
                Ok(None) => {}
            }
            // A send looks the log up again: the chat's log may be a conversation the agent left (`/clear`).
            let (found, was, client) = match find(&terminal, cache_url, cmd == "send") {
                Ok(x) => x,
                Err((code, message)) => return r.fail(code, &message),
            };
            let kind = found.pane.agent.as_ref().map(|a| a.kind.clone()).unwrap_or_else(|| "agent".into());
            let url = found.path.as_ref().map(PathBuf::from);
            let log = url.as_deref();
            match cmd.as_str() {
                "commands" => {
                    // Per agent kind (Claude Code, Codex, pi, agy); other agents get an empty list. Files only, in the
                    // terminal's folder and the user's home (HOME); nothing goes to the agent. `screens`: the
                    // built-ins left out because sent bare they open a screen and they take no value.
                    let home = std::env::var_os("HOME").map(PathBuf::from);
                    let list = home.as_deref().map(|h| crate::commands::list(&kind, h, Path::new(&found.pane.cwd))).unwrap_or_default();
                    let screens = crate::commands::screens(&kind, home.as_deref());
                    r.out.emit_value(json!({"type": "commands", "agent": kind, "commands": list, "screens": screens}));
                    return r.finish(true);
                }
                "status" => {
                    let env = herdr_core::hc::session::process_env();
                    status::run(&kind, log, &status::default_settings(&env), &mut r.out, SystemTime::now());
                    return r.finish(true);
                }
                "queue" => {
                        let screen = found.pane.agent.as_ref().filter(|a| a.status == herdr_core::AgentStatus::Working && queue::reads(&a.kind)).and_then(|_| client.visible_text(&found.pane.id).ok());
                        let q = screen.and_then(|s| {
                            let qlog = if kind == "claude" { queue::log_at(log) } else { Default::default() };
                            queue::read(found.pane.agent.as_ref(), Some(&s), &qlog)
                        });
                        r.out.emit_value(queue::line(q.as_ref()));
                        return r.finish(true);
                    }
                "card" => {
                    // `--answers` (optional): the picks and own answers so far, for `blocked`. Unreadable: the labels only.
                    let answers = opt("--answers").and_then(|a| cards::parse_answers(&a)).unwrap_or_default();
                    let c = cards::card_with(&found.pane, &client, log, has("--all"), cache_url, &answers);
                    r.out.emit(&c);
                    return r.finish(true);
                }
                "approve" => {
                    let (Some(request), true) = (opt("--request"), has("--allow") != has("--deny")) else {
                        return r.fail("usage", "approve needs --request <id> and one of --allow, --deny");
                    };
                    let d = cards::approve(&found.pane, &client, log, &request, has("--allow"), cache_url);
                    r.out.emit(&d);
                    return r.finish(true);
                }
                "answer" => {
                    let (Some(request), Some(answers)) = (opt("--request"), opt("--answers").and_then(|a| cards::parse_answers(&a))) else {
                        return r.fail("usage", "answer needs --request <id> and --answers <json>");
                    };
                    let d = cards::answer(&found.pane, &client, log, &request, &answers);
                    r.out.emit(&d);
                    return r.finish(true);
                }
                "send" => {
                    let Some(text) = opt("--text").filter(|t| !t.is_empty()) else { return r.fail("usage", "send needs --text <s>") };
                    let files: Vec<String> =
                        args.iter().enumerate().filter(|(i, a)| *a == "--file" && *i + 1 < args.len()).map(|(i, _)| args[i + 1].clone()).collect();
                    let d = cards::send(&found.pane, &client, &text, &files, log, opt("--expect-kind").as_deref(), opt("--expect-path").as_deref(), was.as_ref());
                    r.out.emit(&d);
                    return r.finish(true);
                }
                _ => {}
            }
            let tz = chrono::Local;
            let now = chrono::Utc::now();
            if let Some(b) = opt("--before") {
                let (Ok(before), Some(want)) = (b.parse::<u64>(), opt("--path")) else {
                    return r.fail("usage", "feed --before needs a number and --path <log>");
                };
                let Some(url) = log.filter(|u| u.to_string_lossy() == want) else { return r.fail("moved", "the agent now writes another log") };
                return match feed::older(url, before, has("--pictures"), &mut r.out, &now, &tz) {
                    Ok(()) => r.finish(true),
                    Err(e) => r.fail("read", &format!("cannot read the log: {e}")),
                };
            }
            let mut cache = locate::read_cache(cache_url);
            let tid = found.pane.terminal_id.clone();
            match feed::run(log, &kind, opt("--cursor").as_deref(), cache.feeds.get(&tid), has("--pictures"), &mut r.out, &now, &tz) {
                Ok(state) => {
                    if cache_url.is_some() && cache.feeds.get(&tid) != state.as_ref() {
                        match state {
                            Some(s) => cache.feeds.insert(tid, s),
                            None => cache.feeds.remove(&tid),
                        };
                        locate::write_cache(&cache, cache_url);
                    }
                    r.finish(true)
                }
                Err(e) => r.fail("read", &format!("cannot read the log: {e}")),
            }
        }
        // `queue` is serve's own, not offered to the mod.
        _ => r.fail("usage", "commands: version, feed, status, card, approve, answer, send"),
    }
}

/// herdr-core's client and the agent in `terminal` (with its session log), or the error line's code and message.
/// `recheck`: as `locate::recheck` (a send), with the entry the cache had; else `locate::find`.
fn find(terminal: &str, cache: Option<&Path>, recheck: bool) -> Result<(locate::Found, Option<locate::Entry>, HcClient), (&'static str, String)> {
    let session = HcSession::resolve_env();
    if !session.is_available() {
        return Err(("no-core", "herdr-core is not running".into()));
    }
    // exec gives the sidecar 30 s; a send that waits longer reports `unconfirmed`.
    let client = HcClient::new(session.clone(), 8, 22);
    let ws = client.snapshot().map_err(|e| ("core", format!("herdr-core: {e}")))?;
    let now_s = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let mut resolver = SessionResolver::new(&session.socket_path);
    let found = if recheck { locate::recheck(terminal, &ws, &mut resolver, cache, now_s) } else { locate::find(terminal, &ws, &mut resolver, cache, now_s).map(|f| (f, None)) };
    match found {
        Ok((f, was)) => Ok((f, was, client)),
        Err(e @ locate::Failure::NoTerminal(_)) => Err(("no-terminal", e.to_string())),
        Err(e) => Err(("not-agent", e.to_string())),
    }
}

/// A key command with its values typed, never parsed from arguments: `serve` passes the page's values here, so a
/// prompt id, an answer or a text can never be read as an option (`--allow`, `--terminal`).
pub enum Keys {
    Approve { request: String, allow: bool },
    Answer { request: String, answers: Vec<herdr_core::QuestionAnswer> },
    /// `expect_kind`, `expect_path`: as `send --expect-kind`, `--expect-path`.
    Send { text: String, expect_kind: Option<String>, expect_path: Option<String> },
}

/// Runs `keys` on the agent in `terminal`: the `done` line, or the `error` line (as `run` prints them). `home`: as
/// `--home` (the herdr-app home whose settings apply).
pub fn keys(terminal: &str, cache: Option<&Path>, home: Option<&str>, keys: Keys) -> serde_json::Value {
    let error = |code: &str, message: &str| json!({"type": "error", "code": code, "message": message});
    match remote::split(terminal) {
        Err(()) => return error("bad_terminal", &format!("not a terminal address: {terminal}")),
        Ok(Some((machine, there))) => return remote_keys(&machine, &there, cache, home, keys).unwrap_or_else(|(c, m)| error(c, &m)),
        Ok(None) => {}
    }
    let (found, was, client) = match find(terminal, cache, matches!(keys, Keys::Send { .. })) {
        Ok(x) => x,
        Err((code, message)) => return json!({"type": "error", "code": code, "message": message}),
    };
    let log = found.path.as_ref().map(PathBuf::from);
    let log = log.as_deref();
    let d = match keys {
        Keys::Approve { request, allow } => cards::approve(&found.pane, &client, log, &request, allow, cache),
        Keys::Answer { request, answers } => cards::answer(&found.pane, &client, log, &request, &answers),
        Keys::Send { text, expect_kind, expect_path } => cards::send(&found.pane, &client, &text, &[], log, expect_kind.as_deref(), expect_path.as_deref(), was.as_ref()),
    };
    serde_json::to_value(&d).unwrap_or_else(|_| json!({"type": "done", "result": "failed"}))
}

/// `keys` on another machine: the same command the mod's sidecar forwards (`send --terminal m/tN`), so the helper
/// there checks the prompt, the agent and the conversation. Its argv is built here from typed values. Only the text
/// may start with `-` (it is the last value, after `--text`, and `forward` passes it as one argument); any other value
/// that does would be read as an option there, so it is refused and nothing runs.
fn remote_keys(machine: &str, there: &str, cache: Option<&Path>, home: Option<&str>, keys: Keys) -> Result<serde_json::Value, (&'static str, String)> {
    if config::load(config::dir(home).as_deref()).remote_never {
        return Err(("remote-off", config::REMOTE_OFF.into()));
    }
    let session = HcSession::resolve_env();
    if !session.is_available() {
        return Err(("no-core", "herdr-core is not running".into()));
    }
    let terminal = format!("{machine}/{there}");
    let (cmd, mut args): (&str, Vec<String>) = match keys {
        Keys::Approve { request, allow } => ("approve", vec!["--request".into(), request, if allow { "--allow" } else { "--deny" }.into()]),
        Keys::Answer { request, answers } => {
            // As `cards::parse_answers` reads it there: one {picks, other} per question.
            let a = serde_json::Value::Array(answers.iter().map(|a| json!({"picks": a.picks, "other": a.other})).collect()).to_string();
            ("answer", vec!["--request".into(), request, "--answers".into(), a])
        }
        Keys::Send { text, expect_kind, expect_path } => {
            let mut a = vec![];
            for (o, v) in [("--expect-kind", expect_kind), ("--expect-path", expect_path)] {
                if let Some(v) = v {
                    a.extend([o.to_string(), v]);
                }
            }
            a.extend(["--text".into(), text]);
            ("send", a)
        }
    };
    // Option, value pairs (approve ends with its flag); the text is the last value of send.
    let text_at = (cmd == "send").then(|| args.len() - 1);
    for (i, v) in args.iter().enumerate().skip(1).step_by(2) {
        if Some(i) != text_at && (v.starts_with('-') || v.is_empty()) {
            return Err(("usage", format!("{cmd}: a value that starts with '-' cannot go to {machine}")));
        }
    }
    args.splice(0..0, ["--terminal".to_string(), terminal]);
    let client = HcClient::new(session, 8, 22);
    let text = remote::forward(&client, cmd, &args, machine, there, cache);
    let lines: Vec<serde_json::Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if let Some(e) = lines.iter().find(|l| l["type"] == "error") {
        return Ok(e.clone());
    }
    Ok(lines.into_iter().find(|l| l["type"] == "done").unwrap_or_else(|| json!({"type": "error", "code": "remote_failed", "message": format!("The helper on {machine} gave no answer.")})))
}

/// Pulls the queue of the agent in `terminal` (this machine only) back (`queue::run`): `keep` gets the text before the
/// first Ctrl-U. Err: the error line's code and message. A terminal on another machine is refused: if the link to it
/// dropped mid-pull, the text could leave the agent's box and never come back.
pub fn pull(terminal: &str, cache: Option<&Path>, keep: &mut dyn FnMut(&str) -> bool) -> Result<queue::Outcome, (&'static str, String)> {
    match remote::split(terminal) {
        Err(()) => return Err(("bad_terminal", format!("not a terminal address: {terminal}"))),
        Ok(Some((machine, _))) => {
            return Ok(queue::Outcome::Refused(format!("Flockbook pulls back queued messages on this machine only. Edit it in the agent's terminal on {machine}. Nothing was sent.")))
        }
        Ok(None) => {}
    }
    let (found, _, client) = find(terminal, cache, false)?;
    let Some(agent) = found.pane.agent.clone() else { return Err(("not-agent", format!("terminal {terminal} runs no agent"))) };
    let log = queue::log_at(found.path.as_deref().map(Path::new));
    let mut io = queue::Live { client: &client, pane_id: found.pane.id.clone() };
    Ok(queue::run(&agent, &log, &mut io, keep))
}
