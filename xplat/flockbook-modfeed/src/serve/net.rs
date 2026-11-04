//! The listener: 127.0.0.1 only (there is no option for another address), one thread per connection. Every request
//! passes `gate::check` first. `GET` files of the page, `GET /ws` the socket (reads), `POST /api` the key requests.

use super::gate::{self, Kind};
use super::hub::{self, Config, Session};
use rust_embed::RustEmbed;
use serde_json::json;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::protocol::{Role, WebSocketConfig};
use tungstenite::{Message, WebSocket};

/// The page, built from web/ by scripts/web-build.mjs (the Mac app bundles the same files).
#[derive(RustEmbed)]
#[folder = "web/"]
struct Web;

/// The largest request head and body taken.
pub const MAX_HEAD: usize = 16 * 1024;
pub const MAX_BODY: usize = 1 << 20;
/// Connections held at once; more are closed at once.
pub const MAX_CONNECTIONS: usize = 32;
/// A socket that has not said hello by then is closed; a head that does not end by then too.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the socket's pushes go out.
pub const TICK: Duration = Duration::from_secs(1);

/// Who may use it and how it answers.
#[derive(Debug, Clone)]
pub struct Settings {
    pub user: String,
    pub origin: Option<String>,
    pub hub: Config,
}

/// Binds 127.0.0.1:`port` (0: any free port).
pub fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
}

/// Serves until the process ends.
pub fn serve(listener: TcpListener, settings: Settings) {
    let settings = Arc::new(settings);
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            continue; // dropped: closed
        }
        let (s, o) = (settings.clone(), open.clone());
        std::thread::spawn(move || {
            connection(stream, &s);
            o.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// The head up to the blank line, and the bytes read after it. None: not HTTP/1.x, too large, or too slow.
fn read_head(s: &mut TcpStream) -> Option<(Head, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let start = Instant::now();
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEAD || start.elapsed() > HELLO_TIMEOUT {
            return None;
        }
        match s.read(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(_) => return None,
        }
    };
    if end > MAX_HEAD {
        return None;
    }
    let text = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = text.split("\r\n");
    let first: Vec<&str> = lines.next()?.split(' ').collect();
    let [method, target, version] = first.as_slice() else { return None };
    if !version.starts_with("HTTP/1.") {
        return None;
    }
    let mut headers = vec![];
    for l in lines {
        let (k, v) = l.split_once(':')?;
        headers.push((k.trim().to_string(), v.trim().to_string()));
    }
    let path = target.split('?').next().unwrap_or("").to_string();
    Some((Head { method: method.to_string(), path, headers }, buf[end + 4..].to_vec()))
}

const PAGE_HEADERS: [(&str, &str); 4] = [
    ("Content-Security-Policy", "default-src 'self'; connect-src 'self'; img-src 'self' data: blob:; style-src 'self'; script-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'"),
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    ("Cache-Control", "no-cache"),
];

fn respond(s: &mut TcpStream, status: &str, kind: &str, body: &[u8], head_only: bool) {
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
    for (k, v) in PAGE_HEADERS {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    if !head_only {
        bytes.extend_from_slice(body);
    }
    let _ = s.write_all(&bytes);
    let _ = s.flush();
}

fn text(s: &mut TcpStream, status: &str, body: &str) {
    respond(s, status, "text/plain; charset=utf-8", body.as_bytes(), false);
}

fn mime(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "png" => "image/png",
        "webmanifest" => "application/manifest+json",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn connection(mut s: TcpStream, settings: &Settings) {
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let Some((head, rest)) = read_head(&mut s) else {
        text(&mut s, "400 Bad Request", "Bad request");
        return;
    };
    let socket = head.path == "/ws" && head.header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket"));
    let kind = if socket {
        Kind::Socket
    } else if head.path == "/api" {
        Kind::Api
    } else {
        Kind::Page
    };
    if kind == Kind::Page && head.method != "GET" && head.method != "HEAD" {
        text(&mut s, "405 Method Not Allowed", "GET only");
        return;
    }
    if let Err(d) = gate::check(&head.method, &head.headers, kind, &settings.user, settings.origin.as_deref()) {
        text(&mut s, "403 Forbidden", d.message());
        return;
    }
    match kind {
        Kind::Page => page(&mut s, &head),
        Kind::Api => api(&mut s, &head, rest, settings),
        Kind::Socket => socket_loop(s, &head, rest, settings),
    }
}

fn page(s: &mut TcpStream, head: &Head) {
    let head_only = head.method == "HEAD";
    if head.path == "/hub.json" {
        // Tells the page it needs no pairing here and that keys go with POST.
        let body = json!({"auth": "tailscale", "post": hub::KEYS}).to_string();
        respond(s, "200 OK", "application/json", body.as_bytes(), head_only);
        return;
    }
    let name = if head.path == "/" { "index.html" } else { head.path.trim_start_matches('/') };
    match Web::get(name) {
        Some(f) if !name.contains("..") => respond(s, "200 OK", mime(name), &f.data, head_only),
        _ => text(s, "404 Not Found", "Not found"),
    }
}

fn api(s: &mut TcpStream, head: &Head, mut body: Vec<u8>, settings: &Settings) {
    let Some(len) = head.header("content-length").and_then(|l| l.parse::<usize>().ok()) else {
        text(s, "411 Length Required", "Content-Length needed");
        return;
    };
    if len > MAX_BODY {
        text(s, "413 Content Too Large", "Too large");
        return;
    }
    let start = Instant::now();
    let mut chunk = [0u8; 16 * 1024];
    while body.len() < len {
        if start.elapsed() > HELLO_TIMEOUT {
            return;
        }
        match s.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(_) => return,
        }
    }
    body.truncate(len);
    let Ok(text_body) = String::from_utf8(body) else {
        text(s, "400 Bad Request", "Not UTF-8");
        return;
    };
    let out = Session::new(settings.hub.clone()).post(&text_body);
    respond(s, "200 OK", "application/json", serde_json::Value::Array(out).to_string().as_bytes(), false);
}

fn socket_loop(mut s: TcpStream, head: &Head, rest: Vec<u8>, settings: &Settings) {
    let (Some(key), Some("13")) = (head.header("sec-websocket-key"), head.header("sec-websocket-version")) else {
        text(&mut s, "400 Bad Request", "Not a WebSocket request");
        return;
    };
    if !head.header("connection").is_some_and(|c| c.to_ascii_lowercase().contains("upgrade")) {
        text(&mut s, "400 Bad Request", "Not a WebSocket request");
        return;
    }
    let accept = tungstenite::handshake::derive_accept_key(key.as_bytes());
    let reply = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n");
    if s.write_all(reply.as_bytes()).is_err() {
        return;
    }
    let _ = s.set_read_timeout(Some(Duration::from_millis(200)));
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_BODY);
    config.max_frame_size = Some(MAX_BODY);
    let mut ws = WebSocket::from_partially_read(s, rest, Role::Server, Some(config));
    let mut session = Session::new(settings.hub.clone());
    let opened = Instant::now();
    let mut last_tick = Instant::now();
    loop {
        let mut out = vec![];
        match ws.read() {
            Ok(Message::Text(t)) => out = session.receive(t.as_str()),
            Ok(Message::Close(_)) => break,
            Ok(Message::Binary(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        if !session.welcomed && opened.elapsed() > HELLO_TIMEOUT {
            break;
        }
        if session.welcomed && last_tick.elapsed() >= TICK {
            last_tick = Instant::now();
            out.extend(session.tick());
        }
        for m in out {
            if ws.send(Message::text(m.to_string())).is_err() {
                return;
            }
        }
        let _ = ws.flush();
    }
    let _ = ws.close(None);
    let _ = ws.flush();
}
