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
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache.json").to_string_lossy().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
        .args(["serve", "--user", ME, "--port", "0", "--cache", &cache])
        .args(extra)
        .env_clear()
        .env("HC_SOCKET", sock)
        .env("HOME", "/nonexistent-home")
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
    assert_eq!(serde_json::from_str::<Value>(&hub).unwrap(), json!({"auth": "tailscale", "post": ["approve", "answer", "send"]}));
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
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p3", "type": "send", "pane": id, "text": "hello"}));
    let r: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(r[0]["re"], "p3");
    assert_eq!(r[0]["ok"], true, "{r}");
    assert_eq!(fake.calls("terminal.send").len(), 1, "the text went once");
    // Control characters are refused before anything goes out.
    let sends = fake.calls("terminal.send").len();
    let (_, body) = api(srv.port, &[], &json!({"v": 1, "id": "p4", "type": "send", "pane": id, "text": "x\u{1b}[2J"}));
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
