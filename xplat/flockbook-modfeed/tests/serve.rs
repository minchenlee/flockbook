//! `serve` end to end: the built binary on 127.0.0.1, a fake herdr-core, raw HTTP. Who is refused (403), that the
//! socket reads and never sends keys, and that keys go with POST only.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const ME: &str = "owner@example.com";
const HOST: &str = "mac.tail1.ts.net";

struct Server {
    child: Child,
    port: u16,
    _dir: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start(sock: &str, extra: &[&str]) -> Server {
    start_env(sock, extra, &[])
}

fn start_env(sock: &str, extra: &[&str], env: &[(&str, &str)]) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache.json").to_string_lossy().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
        .args(["serve", "--user", ME, "--port", "0", "--cache", &cache])
        .args(extra)
        .env_clear()
        .env("HC_SOCKET", sock)
        .env("HOME", "/nonexistent-home")
        .envs(env.iter().copied())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    let mut err = BufReader::new(child.stderr.take().unwrap());
    err.read_line(&mut line).unwrap();
    // The rest of the server's stderr (a panic) shows with the test's output.
    std::thread::spawn(move || {
        for l in err.lines().map_while(Result::ok) {
            eprintln!("serve: {l}");
        }
    });
    let port: u16 = line.split("127.0.0.1:").nth(1).and_then(|r| r.split(' ').next()).and_then(|p| p.parse().ok()).unwrap_or_else(|| panic!("{line}"));
    Server { child, port, _dir: dir }
}

/// One raw request; the status code and the body.
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\n");
    for (k, v) in headers {
        req += &format!("{k}: {v}\r\n");
    }
    if method == "POST" {
        req += &format!("Content-Length: {}\r\n", body.len());
    }
    req += "\r\n";
    req += body;
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let code = out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (code, out.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default())
}

fn who(user: &str) -> Vec<(&'static str, String)> {
    vec![("Host", HOST.into()), ("Tailscale-User-Login", user.into())]
}

fn api(port: u16, extra: &[(&str, &str)], body: &Value) -> (u16, String) {
    let mut h: Vec<(&str, &str)> = vec![("Host", HOST), ("Tailscale-User-Login", ME), ("Content-Type", "application/json"), ("Origin", "https://mac.tail1.ts.net")];
    for (k, v) in extra {
        h.retain(|(hk, _)| !hk.eq_ignore_ascii_case(k));
        if !v.is_empty() {
            h.push((k, v));
        }
    }
    http(port, "POST", "/api", &h, &body.to_string())
}

fn blocked_fake() -> FakeHc {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "blocked", "permission")]);
    fake.set_screen("t1", "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No");
    fake
}

#[test]
fn who_is_refused() {
    let fake = blocked_fake();
    let srv = start(&fake.path, &[]);
    let p = srv.port;
    // The page itself: Tailscale's identity, the right account, not Funnel.
    let me = who(ME);
    let me: Vec<(&str, &str)> = me.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let (c, body) = http(p, "GET", "/", &me, "");
    assert_eq!(c, 200, "{body}");
    assert!(body.contains("<html") || body.contains("<!doctype") || body.contains("<!DOCTYPE"));
    assert_eq!(http(p, "GET", "/", &[("Host", HOST)], "").0, 403, "no identity header");
    assert_eq!(http(p, "GET", "/", &[("Host", HOST), ("Tailscale-User-Login", "other@example.com")], "").0, 403, "another account");
    let (c, body) = http(p, "GET", "/", &[("Host", HOST), ("Tailscale-User-Login", ME), ("Tailscale-Funnel-Request", "?1")], "");
    assert_eq!(c, 403, "Funnel");
    assert!(body.contains("Funnel"), "{body}");
    assert_eq!(http(p, "GET", "/", &[("Host", HOST), ("Tailscale-Funnel-Request", "?1")], "").0, 403, "Funnel without identity");
    let (c, hub) = http(p, "GET", "/hub.json", &me, "");
    assert_eq!(c, 200);
    assert_eq!(serde_json::from_str::<Value>(&hub).unwrap(), json!({"auth": "tailscale", "post": ["approve", "answer", "send", "pullQueued"]}));
    assert_eq!(http(p, "GET", "/hub.json", &[("Host", HOST)], "").0, 403, "every path is gated");
    assert_eq!(http(p, "GET", "/nope.js", &me, "").0, 404);

    // POST /api: identity, then a same-origin JSON POST.
    let deny = json!({"v": 1, "id": "c1", "type": "approve", "pane": "hc:t1", "prompt": "x", "allow": false});
    assert_eq!(api(p, &[("Tailscale-User-Login", "")], &deny).0, 403, "no identity");
    assert_eq!(api(p, &[("Tailscale-User-Login", "other@example.com")], &deny).0, 403, "another account");
    assert_eq!(api(p, &[("Tailscale-Funnel-Request", "?1")], &deny).0, 403, "Funnel");
    assert_eq!(api(p, &[("Origin", "")], &deny).0, 403, "no Origin");
    assert_eq!(api(p, &[("Origin", "https://evil.example")], &deny).0, 403, "another Origin");
    assert_eq!(api(p, &[("Origin", "http://mac.tail1.ts.net")], &deny).0, 403, "not https");
    assert_eq!(api(p, &[("Content-Type", "text/plain")], &deny).0, 403, "a form-like body");
    assert_eq!(api(p, &[("Sec-Fetch-Site", "cross-site")], &deny).0, 403, "cross-site");
    assert_eq!(http(p, "GET", "/api", &[("Host", HOST), ("Tailscale-User-Login", ME), ("Origin", "https://mac.tail1.ts.net")], "").0, 403, "GET /api");
    assert!(fake.keys().is_empty(), "no refused request sent a key");

    // The socket: the same identity rules, and an Origin naming this host.
    let ws = |extra: &[(&str, &str)]| {
        let mut h: Vec<(&str, &str)> = vec![("Host", HOST), ("Tailscale-User-Login", ME), ("Origin", "https://mac.tail1.ts.net"), ("Upgrade", "websocket"),
                                             ("Connection", "Upgrade"), ("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ=="), ("Sec-WebSocket-Version", "13")];
        for (k, v) in extra {
            h.retain(|(hk, _)| !hk.eq_ignore_ascii_case(k));
            if !v.is_empty() {
                h.push((k, v));
            }
        }
        let mut s = TcpStream::connect(("127.0.0.1", p)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut req = "GET /ws HTTP/1.1\r\n".to_string();
        for (k, v) in &h {
            req += &format!("{k}: {v}\r\n");
        }
        s.write_all(format!("{req}\r\n").as_bytes()).unwrap();
        let mut buf = [0u8; 12];
        s.read_exact(&mut buf).unwrap();
        String::from_utf8_lossy(&buf).split(' ').nth(1).and_then(|c| c.parse::<u16>().ok()).unwrap_or(0)
    };
    assert_eq!(ws(&[]), 101);
    assert_eq!(ws(&[("Tailscale-User-Login", "")]), 403, "socket: no identity");
    assert_eq!(ws(&[("Tailscale-User-Login", "other@example.com")]), 403, "socket: another account");
    assert_eq!(ws(&[("Tailscale-Funnel-Request", "?1")]), 403, "socket: Funnel");
    assert_eq!(ws(&[("Origin", "")]), 403, "socket: no Origin");
    assert_eq!(ws(&[("Origin", "https://evil.example")]), 403, "socket: another Origin");
}

fn socket(port: u16) -> tungstenite::WebSocket<TcpStream> {
    let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    let req = tungstenite::http::Request::builder()
        .uri(format!("ws://127.0.0.1:{port}/ws"))
        .header("Host", HOST)
        .header("Origin", "https://mac.tail1.ts.net")
        .header("Tailscale-User-Login", ME)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
        .body(())
        .unwrap();
    tungstenite::client(req, s).unwrap().0
}

fn read_until(ws: &mut tungstenite::WebSocket<TcpStream>, done: impl Fn(&Value) -> bool) -> Vec<Value> {
    let mut seen = vec![];
    loop {
        let m = ws.read().expect("a message in time");
        if let tungstenite::Message::Text(t) = m {
            let v: Value = serde_json::from_str(t.as_str()).unwrap();
            let stop = done(&v);
            seen.push(v);
            if stop {
                return seen;
            }
        }
    }
}

fn say(ws: &mut tungstenite::WebSocket<TcpStream>, v: Value) {
    ws.send(tungstenite::Message::text(v.to_string())).unwrap();
}

#[test]
fn socket_reads_and_keys_go_with_post() {
    let fake = blocked_fake();
    let srv = start(&fake.path, &[]);
    let mut ws = socket(srv.port);
    say(&mut ws, json!({"v": 1, "id": "h", "type": "hello", "token": "", "client": "test", "clientVersion": "1"}));
    let got = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "h");
    assert_eq!(got[0]["type"], "welcome");
    assert_eq!(got[0]["level"], "input");
    let snap = got.iter().find(|m| m["type"] == "snapshot").unwrap();
    let pane = snap["workspaces"][0]["tabs"][0]["panes"][0].clone();
    let id = pane["id"].as_str().unwrap().to_string();
    assert_eq!(pane["agent"]["status"], "blocked");

    // Watching a blocked agent: its prompt comes, read from the screen. No key.
    say(&mut ws, json!({"v": 1, "id": "w", "type": "watch", "pane": id}));
    let got = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "w");
    assert_eq!(got.last().unwrap()["ok"], true, "{got:?}");
    let prompt = got.iter().find(|m| m["type"] == "prompt").unwrap();
    assert_eq!(prompt["approval"]["command"], "rm -rf build");
    let request = prompt["approval"]["id"].as_str().unwrap().to_string();
    assert!(got.iter().any(|m| m["type"] == "conversation"));

    // Keys over the socket: refused, nothing sent.
    for (i, m) in [json!({"type": "approve", "pane": id, "prompt": request, "allow": false}), json!({"type": "send", "pane": id, "text": "hi"}),
                   json!({"type": "answer", "pane": id, "question": "q", "answers": []})].into_iter().enumerate() {
        let mut m = m;
        m["v"] = json!(1);
        m["id"] = json!(format!("k{i}"));
        say(&mut ws, m);
        let r = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == format!("k{i}"));
        assert_eq!(r.last().unwrap()["error"]["code"], "badRequest");
    }
    assert!(fake.keys().is_empty() && fake.calls("terminal.send").is_empty(), "the socket never sends keys");

    // The same approval over POST: the key goes (deny is Esc).
    let (c, body) = api(srv.port, &[], &json!({"v": 1, "id": "p1", "type": "approve", "pane": id, "prompt": request, "allow": false}));
    assert_eq!(c, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap(), json!([{"type": "reply", "re": "p1", "ok": true}]));
    assert_eq!(fake.keys(), vec![vec!["esc".to_string()]]);
    // A stale prompt id: nothing more is sent.
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p2", "type": "approve", "pane": id, "prompt": "old", "allow": true}));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()[0]["ok"], false);
    assert_eq!(fake.keys().len(), 1);
    // Send over POST types the text.
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p3", "type": "send", "pane": id, "text": "hello", "agent": "claude"}));
    let r: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(r[0]["re"], "p3");
    assert_eq!(r[0]["ok"], true, "{r}");
    assert_eq!(fake.calls("terminal.send").len(), 1, "the text went once");
    // Control characters are refused before anything goes out.
    let sends = fake.calls("terminal.send").len();
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p4", "type": "send", "pane": id, "text": "x\u{1b}[2J", "agent": "claude"}));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()[0]["error"]["code"], "badRequest");
    assert_eq!(fake.calls("terminal.send").len(), sends);
}

#[test]
fn read_only_and_bind() {
    let fake = blocked_fake();
    let srv = start(&fake.path, &["--read-only"]);
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p1", "type": "approve", "pane": "hc:t1", "prompt": "x", "allow": true}));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()[0]["error"]["code"], "readOnly");
    assert!(fake.keys().is_empty());
    // No option binds another address.
    let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(["serve", "--user", ME, "--host", "0.0.0.0"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("127.0.0.1 only"));
    let out = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(["serve"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "--user is needed");
    // Only loopback answers: the listener's address is 127.0.0.1.
    let (c, _) = http(srv.port, "GET", "/", &[("Host", HOST), ("Tailscale-User-Login", ME)], "");
    assert_eq!(c, 200);
}

/// Values from the page are never read as options: a prompt id "--allow" with allow false is a stale prompt, not
/// an allow; a question id "--answers" is a stale question; a text "--terminal t9" is typed as it is.
#[test]
fn values_are_never_options() {
    let fake = blocked_fake();
    fake.set_terminals(vec![claude("t1", "blocked", "permission"), claude("t9", "idle", "")]);
    let srv = start(&fake.path, &[]);
    let reply = |m: Value| -> Value { serde_json::from_str::<Value>(&api(srv.port, &[], &m).1).unwrap()[0].clone() };
    for (prompt, allow) in [("--allow", false), ("--deny", true), ("--allow --deny", true), ("t9", true)] {
        let r = reply(json!({"v": 1, "id": "a", "type": "approve", "pane": "hc:t1", "prompt": prompt, "allow": allow}));
        assert_eq!(r["error"]["code"], "stale", "prompt {prompt:?}: {r}");
    }
    assert!(fake.keys().is_empty(), "no key for a prompt that is not on screen");
    let r = reply(json!({"v": 1, "id": "q", "type": "answer", "pane": "hc:t1", "question": "--answers", "answers": [{"picks": [0]}]}));
    assert_eq!(r["error"]["code"], "stale", "{r}");
    assert!(fake.keys().is_empty());
    let r = reply(json!({"v": 1, "id": "s", "type": "send", "pane": "hc:t1", "text": "--terminal", "agent": "claude"}));
    assert_eq!(r["ok"], true, "{r}");
    let sent = fake.calls("terminal.send");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["terminal"], "t1", "to the pane asked, never t9");
    assert_eq!(sent[0]["text"], "--terminal");
}

/// `send` names the agent the page showed: another agent in that pane now gets nothing (the mod's --expect-kind).
#[test]
fn send_checks_the_agent() {
    let fake = blocked_fake();
    let srv = start(&fake.path, &[]);
    let r: Value = serde_json::from_str(&api(srv.port, &[], &json!({"v": 1, "id": "s", "type": "send", "pane": "hc:t1", "text": "hi", "agent": "codex"})).1).unwrap();
    assert_eq!(r[0]["error"]["code"], "stale", "{r}");
    let r: Value = serde_json::from_str(&api(srv.port, &[], &json!({"v": 1, "id": "s2", "type": "send", "pane": "hc:t1", "text": "hi"})).1).unwrap();
    assert_eq!(r[0]["error"]["code"], "badRequest", "the agent is needed: {r}");
    assert!(fake.calls("terminal.send").is_empty());
}

/// A masked client text frame (payload under 126 bytes).
fn frame(text: &str) -> Vec<u8> {
    let b = text.as_bytes();
    assert!(b.len() < 126);
    let mask = [1u8, 2, 3, 4];
    let mut f = vec![0x81, 0x80 | b.len() as u8];
    f.extend_from_slice(&mask);
    f.extend(b.iter().enumerate().map(|(i, c)| c ^ mask[i % 4]));
    f
}

/// A peer that asks but never reads: once the hub cannot write for its write timeout, it lets the connection go.
/// Each hello is answered with a large snapshot (2000 terminals), so a few dozen small requests are megabytes the
/// peer never takes: the hub's writes stall for sure, whatever the machine's load.
#[test]
fn a_peer_that_does_not_read_is_let_go() {
    let fake = blocked_fake();
    fake.set_terminals((0..2000).map(|i| claude(&format!("t{i}"), "idle", "")).collect());
    let srv = start(&fake.path, &[]);
    let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
    let req = format!("GET /ws HTTP/1.1\r\nHost: {HOST}\r\nTailscale-User-Login: {ME}\r\nOrigin: https://{HOST}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let hello = frame(r#"{"v":1,"id":"h","type":"hello","token":"","client":"t","clientVersion":"1"}"#);
    for _ in 0..60 {
        s.write_all(&hello).unwrap();
    }
    // Long past the hub's write timeout, then read: what was sent, then the end. A hub still holding the
    // connection would send the rest and then wait (the read times out).
    std::thread::sleep(Duration::from_secs(15));
    // A socket the hub already reset may refuse the option (macOS: EINVAL); then it has ended.
    if s.set_read_timeout(Some(Duration::from_secs(10))).is_err() {
        return;
    }
    let mut buf = vec![0u8; 1 << 16];
    let ended = loop {
        match s.read(&mut buf) {
            Ok(0) => break true,
            Ok(_) => continue,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break false,
            Err(_) => break true, // reset
        }
    };
    assert!(ended, "the hub let the connection go");
}

fn user_line(id: &str, text: &str) -> String {
    format!(r#"{{"type":"user","uuid":"{id}","timestamp":"2026-11-03T01:40:00.000Z","message":{{"role":"user","content":"{text}"}}}}"#)
}

/// An idle agent whose log `log` is known (the cache's entry for t1, as `feed` keeps it).
fn with_log(dir: &std::path::Path, log: &std::path::Path) -> (FakeHc, String) {
    let fake = FakeHc::start();
    fake.set_terminals(vec![claude("t1", "idle", "")]);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let cache = dir.join("cache.json");
    let c = json!({"entries": {"t1": {"kind": "claude", "cwd": "/tmp/proj", "pids": [4242], "path": log, "checked": now}}, "feeds": {}});
    std::fs::write(&cache, c.to_string()).unwrap();
    (fake, cache.to_string_lossy().to_string())
}

#[test]
fn load_earlier_reads_back_in_the_log() {
    // 50 old rows, then 2.1 MB with no rows (beyond the tail `feed` reads), then 120 new rows.
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("sess-1.jsonl");
    let mut lines: Vec<String> = (0..50).map(|i| user_line(&format!("old{i}"), &format!("old {i}"))).collect();
    let pad = "x".repeat(1000);
    lines.extend((0..2200).map(|_| format!(r#"{{"type":"file-history-snapshot","pad":"{pad}"}}"#)));
    lines.extend((0..120).map(|i| user_line(&format!("new{i}"), &format!("new {i}"))));
    std::fs::write(&log, lines.join("\n") + "\n").unwrap();
    let (fake, cache) = with_log(dir.path(), &log);
    let srv = start(&fake.path, &["--cache", &cache]);
    let mut ws = socket(srv.port);
    say(&mut ws, json!({"v": 1, "id": "h", "type": "hello", "token": "", "client": "test", "clientVersion": "1"}));
    let got = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "h");
    let pane = got.iter().find(|m| m["type"] == "snapshot").unwrap()["workspaces"][0]["tabs"][0]["panes"][0]["id"].as_str().unwrap().to_string();
    let session = log.to_string_lossy().to_string();

    // Asked before watching: no pane shows it here yet.
    say(&mut ws, json!({"v": 1, "id": "e0", "type": "loadEarlier", "session": session, "before": "new20-0"}));
    let r = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "e0");
    assert_eq!(r.last().unwrap()["error"]["code"], "notFound");

    say(&mut ws, json!({"v": 1, "id": "w", "type": "watch", "pane": pane}));
    let got = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "w");
    let c = got.iter().find(|m| m["type"] == "conversation").unwrap();
    assert_eq!(c["session"], session.as_str());
    let items = c["items"].as_array().unwrap();
    assert_eq!(items.len(), 100, "the newest 100");
    assert_eq!(items[0]["id"], "new20-0");
    assert_eq!(c["hasEarlier"], true);

    // Unknown session or item: notFound, nothing read.
    for (i, (s, b)) in [("/etc/passwd", "new20-0"), (session.as_str(), "nope")].into_iter().enumerate() {
        say(&mut ws, json!({"v": 1, "id": format!("x{i}"), "type": "loadEarlier", "session": s, "before": b}));
        let r = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == format!("x{i}"));
        assert_eq!(r.last().unwrap()["error"]["code"], "notFound", "{r:?}");
    }

    // Before new20: the 20 rows here and the 50 the log has before the tail. The first row of the log is shown now.
    say(&mut ws, json!({"v": 1, "id": "e1", "type": "loadEarlier", "session": session, "before": "new20-0"}));
    let r = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "e1");
    assert_eq!(r.last().unwrap()["ok"], true, "{r:?}");
    let p = r.iter().find(|m| m["type"] == "conversation").unwrap();
    assert_eq!(p["mode"], "prepend");
    let ids: Vec<&str> = p["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), 70, "{ids:?}");
    assert_eq!((ids[0], ids[49], ids[50], ids[69]), ("old0-0", "old49-0", "new0-0", "new19-0"));
    assert_eq!(p["hasEarlier"], false);

    // A new row: only it comes (append), and the older rows stay on the page.
    std::fs::write(&log, lines.join("\n") + "\n" + &user_line("new120", "newest") + "\n").unwrap();
    let got = read_until(&mut ws, |v| v["type"] == "conversation");
    let c = got.last().unwrap();
    assert_eq!(c["mode"], "append", "{c}");
    assert_eq!(c["items"].as_array().unwrap().iter().map(|i| i["id"].clone()).collect::<Vec<_>>(), vec![json!("new120-0")]);
    assert_eq!(c["hasEarlier"], false);
}

/// One space `sp1` with tab `tb1` holding the given terminals, with pane titles (the official app's document).
fn one_space(name: &str, tab: &str, terms: &[(&str, &str)]) -> Value {
    let kids: Vec<Value> = terms.iter().enumerate().map(|(i, (t, title))| json!({"id": format!("p{i}"), "type": "block", "content": {"terminal": t}, "title": title})).collect();
    json!({"spaces": [{"id": "sp1", "name": name, "folder": "/w", "tabs": [{"id": "tb1", "name": tab, "root": {"id": "r", "type": "row", "children": kids}}]}]})
}

#[test]
fn tabs_on_other_machines() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("box-home");
    let here = FakeHc::start();
    let there = FakeHc::start();
    let lab = |mut t: Value| {
        t["labels"] = json!({"herdr.space": "sp1", "herdr.tab": "tb1"});
        t
    };
    here.set_terminals(vec![lab(common::terminal("t1", &["zsh"], Some(11), None, None)), lab(claude("box/t3", "idle", ""))]);
    there.set_terminals(vec![claude("t3", "idle", "")]);
    here.add_machine("box", &home, &there.path);
    here.set_machine_list(vec![json!({"name": "box", "state": "connected"}), json!({"name": "tmp", "state": "connected", "temporary": true}),
                              json!({"name": "off", "state": "unreachable"})]);
    here.set_doc("herdr-app", one_space("Here", "main", &[("t1", "")]));
    here.set_doc("box/herdr-app", one_space("Remote", "build", &[("t3", "agent")]));
    // The helper there (this test build).
    let installed = home.join(modfeedkit::remote::HELPER_DIR).join(modfeedkit::remote::helper_version()).join("flockbook-modfeed");
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_flockbook-modfeed"), &installed).unwrap();

    let srv = start(&here.path, &[]);
    let mut ws = socket(srv.port);
    say(&mut ws, json!({"v": 1, "id": "h", "type": "hello", "token": "", "client": "test", "clientVersion": "1"}));
    // The other machine's document is read in the background: it shows in this snapshot or a later one.
    let got = read_until(&mut ws, |v| v["type"] == "snapshot" && v["workspaces"].as_array().is_some_and(|w| w.len() == 2 && w[1]["label"] == "Remote"));
    let snap = &got.last().unwrap()["workspaces"];
    assert_eq!(snap[0]["label"], "Here");
    assert_eq!(snap[0]["tabs"][0]["label"], "main");
    assert_eq!(snap[1]["id"], "hc:space:box/sp1");
    let tab = &snap[1]["tabs"][0];
    assert_eq!((tab["id"].as_str(), tab["label"].as_str()), (Some("hc:tab:box/sp1/tb1"), Some("build")));
    let pane = &tab["panes"][0];
    assert_eq!((pane["id"].as_str(), pane["label"].as_str()), (Some("hc:box/t3"), Some("agent")));
    let ons: Vec<Value> = here.calls("doc.get").iter().filter_map(|p| p.get("on").cloned()).collect();
    assert!(!ons.is_empty() && ons.iter().all(|o| o == "box"), "only box's document is read: {ons:?}");

    // Watching it: the helper there reads it. Nothing is read here.
    say(&mut ws, json!({"v": 1, "id": "w", "type": "watch", "pane": "hc:box/t3"}));
    let got = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "w");
    assert_eq!(got.last().unwrap()["ok"], true, "{got:?}");
    assert!(got.iter().any(|m| m["type"] == "conversation" && m["pane"] == "hc:box/t3"));
    assert!(here.calls("terminal.read").is_empty());

    // Send over POST: the agent there gets the text.
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p1", "type": "send", "pane": "hc:box/t3", "text": "hello there", "agent": "claude"}));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()[0]["ok"], true, "{body}");
    assert_eq!(there.calls("terminal.send").last().unwrap()["text"], "hello there");
    assert!(here.calls("terminal.send").is_empty());
    // Another agent kind than the page saw: nothing goes there.
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p2", "type": "send", "pane": "hc:box/t3", "text": "x", "agent": "codex"}));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()[0]["ok"], false, "{body}");
    assert_eq!(there.calls("terminal.send").len(), 1);
}

#[test]
fn the_agents_queue_and_pull_back() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("sess-q.jsonl");
    let lines = [user_line("u1", "start"), r#"{"type":"queue-operation","operation":"enqueue","content":"QC reply GAMMA"}"#.to_string()];
    std::fs::write(&log, lines.join("\n") + "\n").unwrap();
    let (fake, cache) = with_log(dir.path(), &log);
    fake.set_terminals(vec![claude("t1", "working", "")]);
    let rule = "─".repeat(60);
    let queued = format!("⏺ ALPHA\n❯ QC reply GAMMA\n  ctrl+enter to send now\n· Pouncing… (10s)\n{rule}\n❯ Press up to edit queued messages\n{rule}");
    fake.set_screen("t1", &queued);
    // Up brings the text into the box; Ctrl-U empties it.
    let (r1, r2) = (rule.clone(), rule.clone());
    fake.on_keys(Some(std::sync::Arc::new(move |a: &str, k: &[String], s: &mut common::State| {
        let text = match k[0].as_str() {
            "up" => format!("⏺ ALPHA\n{r1}\n❯ QC reply GAMMA\n{r2}"),
            _ => format!("⏺ ALPHA\n{r1}\n❯ \n{r2}"),
        };
        s.screens.insert(a.to_string(), text);
    })));
    let srv = start(&fake.path, &["--cache", &cache]);
    let mut ws = socket(srv.port);
    say(&mut ws, json!({"v": 1, "id": "h", "type": "hello", "token": "", "client": "test", "clientVersion": "1"}));
    let got = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "h");
    let pane = got.iter().find(|m| m["type"] == "snapshot").unwrap()["workspaces"][0]["tabs"][0]["panes"][0]["id"].as_str().unwrap().to_string();
    assert!(!got.iter().any(|m| m["type"] == "pulled"), "nothing pulled yet");

    // Watch: the queue, as the log and the screen give it.
    say(&mut ws, json!({"v": 1, "id": "w", "type": "watch", "pane": pane}));
    let got = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "w");
    let q = got.iter().find(|m| m["type"] == "queued").unwrap_or_else(|| panic!("{got:?}"));
    assert_eq!(q, &json!({"type": "queued", "pane": pane, "agent": "claude", "items": ["QC reply GAMMA"], "pullable": true, "pullsAll": true}));
    // On the socket it is refused: keys go with POST.
    say(&mut ws, json!({"v": 1, "id": "s", "type": "pullQueued", "pane": pane}));
    let r = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "s");
    assert_eq!(r.last().unwrap()["ok"], false);
    assert!(fake.keys().is_empty());

    // POST: Up, then Ctrl-U; the text comes back with an id.
    let (c, body) = api(srv.port, &[], &json!({"v": 1, "id": "p", "type": "pullQueued", "pane": pane}));
    assert_eq!(c, 200);
    let r: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(r[1], json!({"type": "reply", "re": "p", "ok": true}), "{r}");
    assert_eq!((r[0]["type"].as_str(), r[0]["text"].as_str(), r[0]["interrupted"].as_bool()), (Some("pulled"), Some("QC reply GAMMA"), Some(false)), "{r}");
    assert!(r[0].get("note").is_none(), "{r}");
    let id = r[0]["id"].as_str().unwrap().to_string();
    assert_eq!(fake.keys(), vec![vec!["up".to_string()], vec!["ctrl+u".to_string()]]);

    // A page that did not confirm it gets it again at hello; after pulledAck, no more.
    let mut ws2 = socket(srv.port);
    say(&mut ws2, json!({"v": 1, "id": "h2", "type": "hello", "token": "", "client": "test", "clientVersion": "1"}));
    let got = read_until(&mut ws2, |v| v["type"] == "reply" && v["re"] == "h2");
    let again = got.iter().find(|m| m["type"] == "pulled").unwrap();
    assert_eq!((again["id"].as_str(), again["text"].as_str()), (Some(id.as_str()), Some("QC reply GAMMA")));
    say(&mut ws2, json!({"v": 1, "id": "a", "type": "pulledAck", "pulled": id}));
    read_until(&mut ws2, |v| v["type"] == "reply" && v["re"] == "a");
    let mut ws3 = socket(srv.port);
    say(&mut ws3, json!({"v": 1, "id": "h3", "type": "hello", "token": "", "client": "test", "clientVersion": "1"}));
    let got = read_until(&mut ws3, |v| v["type"] == "reply" && v["re"] == "h3");
    assert!(!got.iter().any(|m| m["type"] == "pulled"), "{got:?}");

    // The queue went: an empty one clears it on the page.
    fake.set_screen("t1", &format!("⏺ ALPHA\n{rule}\n❯ \n{rule}"));
    let got = read_until(&mut ws, |v| v["type"] == "queued");
    assert_eq!(got.last().unwrap()["items"], json!([]));
}

#[test]
fn older_rows_have_a_bound() {
    // 2300 old rows beyond the tail, then the tail's 120: loadEarlier keeps at most MAX_OLDER of the old ones.
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("sess-big.jsonl");
    let mut lines: Vec<String> = (0..2300).map(|i| user_line(&format!("old{i}"), &format!("old {i}"))).collect();
    let pad = "x".repeat(1000);
    lines.extend((0..2200).map(|_| format!(r#"{{"type":"file-history-snapshot","pad":"{pad}"}}"#)));
    lines.extend((0..120).map(|i| user_line(&format!("new{i}"), &format!("new {i}"))));
    std::fs::write(&log, lines.join("\n") + "\n").unwrap();
    let (fake, cache) = with_log(dir.path(), &log);
    let srv = start(&fake.path, &["--cache", &cache]);
    let mut ws = socket(srv.port);
    say(&mut ws, json!({"v": 1, "id": "h", "type": "hello", "token": "", "client": "test", "clientVersion": "1"}));
    let got = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "h");
    let pane = got.iter().find(|m| m["type"] == "snapshot").unwrap()["workspaces"][0]["tabs"][0]["panes"][0]["id"].as_str().unwrap().to_string();
    say(&mut ws, json!({"v": 1, "id": "w", "type": "watch", "pane": pane}));
    read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "w");
    let session = log.to_string_lossy().to_string();
    let (mut before, mut total, mut calls) = ("new20-0".to_string(), 0, 0);
    loop {
        calls += 1;
        assert!(calls < 60, "it ends");
        say(&mut ws, json!({"v": 1, "id": format!("e{calls}"), "type": "loadEarlier", "session": session, "before": before}));
        let r = read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == format!("e{calls}"));
        let p = r.iter().find(|m| m["type"] == "conversation").unwrap_or_else(|| panic!("{r:?}"));
        let items = p["items"].as_array().unwrap();
        total += items.len();
        if p["hasEarlier"] == false || items.is_empty() {
            assert_eq!(p["hasEarlier"], false);
            break;
        }
        before = items[0]["id"].as_str().unwrap().to_string();
    }
    assert_eq!(total, 20 + modfeedkit::serve::hub::MAX_OLDER, "the 20 tail rows before new20, then the bound");
}

#[test]
fn never_comes_from_the_herdr_app_home() {
    // Flockbook settings in $HERDR_APP_HOME say Never: nothing runs on box, keys to it get remote-off.
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app-home");
    std::fs::create_dir_all(app.join("flockbook")).unwrap();
    std::fs::write(app.join("flockbook/config.json"), r#"{"remote_helper": "never"}"#).unwrap();
    let here = FakeHc::start();
    let there = FakeHc::start();
    here.set_terminals(vec![claude("box/t3", "idle", "")]);
    there.set_terminals(vec![claude("t3", "idle", "")]);
    here.add_machine("box", &dir.path().join("box-home"), &there.path);
    here.set_machine_list(vec![json!({"name": "box", "state": "connected"})]);
    here.set_doc("box/herdr-app", one_space("Remote", "build", &[("t3", "agent")]));
    let srv = start_env(&here.path, &[], &[("HERDR_APP_HOME", app.to_str().unwrap())]);
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p1", "type": "send", "pane": "hc:box/t3", "text": "hello", "agent": "claude"}));
    let r: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(r[0]["ok"], false, "{r}");
    assert!(r[0]["error"]["message"].as_str().unwrap_or("").contains("does not run its helper"), "{r}");
    let mut ws = socket(srv.port);
    say(&mut ws, json!({"v": 1, "id": "h", "type": "hello", "token": "", "client": "test", "clientVersion": "1"}));
    read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "h");
    say(&mut ws, json!({"v": 1, "id": "w", "type": "watch", "pane": "hc:box/t3"}));
    read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "w");
    assert!(here.calls("machine.exec").is_empty(), "nothing ran on box");
    // Not even a look: its layout is not read (its tab shows under the labels).
    std::thread::sleep(std::time::Duration::from_millis(400));
    say(&mut ws, json!({"v": 1, "id": "p", "type": "ping"}));
    read_until(&mut ws, |v| v["type"] == "reply" && v["re"] == "p");
    assert!(here.calls("machine.list").is_empty(), "no machine list");
    assert!(here.calls("doc.get").iter().all(|p| p.get("on").is_none()), "no document of box: {:?}", here.calls("doc.get"));
}
