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
/// A message (or a response) that is not written by then ends the connection: the peer stopped reading. A deadline
/// for the whole write, not per call: a peer that takes a few bytes now and then keeps a per-call timeout from firing.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
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

/// The connections held at once, at most `max`.
#[derive(Clone)]
pub struct Slots {
    open: Arc<AtomicUsize>,
    max: usize,
}

/// One connection's place in `Slots`; given back when dropped, also when the connection's thread panics.
pub struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Slots {
    pub fn new(max: usize) -> Slots {
        Slots { open: Arc::new(AtomicUsize::new(0)), max }
    }
    /// A place, or None when `max` are held.
    pub fn take(&self) -> Option<Slot> {
        if self.open.fetch_add(1, Ordering::SeqCst) >= self.max {
            self.open.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Slot(self.open.clone()))
    }
    pub fn held(&self) -> usize {
        self.open.load(Ordering::SeqCst)
    }
}

/// Serves until the process ends.
pub fn serve(listener: TcpListener, settings: Settings) {
    let settings = Arc::new(settings);
    let slots = Slots::new(MAX_CONNECTIONS);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let Some(slot) = slots.take() else { continue }; // dropped: closed
        let s = settings.clone();
        std::thread::spawn(move || {
            let _slot = slot;
            connection(stream, &s);
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
    let _ = s.set_write_timeout(Some(WRITE_TIMEOUT));
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
    let mut ws = WebSocket::from_partially_read(Timed { s, until: Instant::now() + WRITE_TIMEOUT }, rest, Role::Server, Some(config));
    let mut session = Session::new(settings.hub.clone());
    let opened = Instant::now();
    let mut last_tick = Instant::now();
    loop {
        let mut out = vec![];
        // `read` first flushes what is queued: that flush gets the deadline too.
        ws.get_mut().until = Instant::now() + WRITE_TIMEOUT;
        match ws.read() {
            Ok(Message::Text(t)) => out = session.receive(t.as_str()),
            Ok(Message::Close(_)) => break,
            Ok(Message::Binary(_)) => break,
            Ok(_) => {}
            // The read timeout only (a stalled write is BrokenPipe, see `Timed`).
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
            // A message the peer did not take in WRITE_TIMEOUT ends here.
            ws.get_mut().until = Instant::now() + WRITE_TIMEOUT;
            if ws.send(Message::text(m.to_string())).is_err() {
                return;
            }
        }
        ws.get_mut().until = Instant::now() + WRITE_TIMEOUT;
        if ws.flush().is_err() {
            return;
        }
    }
    ws.get_mut().until = Instant::now() + WRITE_TIMEOUT;
    let _ = ws.close(None);
    let _ = ws.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A connection thread that panics gives its place back: after more panics than places, a new connection is
    /// still taken.
    #[test]
    fn a_panic_gives_the_place_back() {
        let slots = Slots::new(4);
        for _ in 0..10 {
            let slot = slots.take().expect("a place is free");
            let t = std::thread::spawn(move || {
                let _slot = slot;
                panic!("a connection broke");
            });
            assert!(t.join().is_err());
        }
        assert_eq!(slots.held(), 0);
        let held: Vec<Slot> = (0..4).map(|_| slots.take().unwrap()).collect();
        assert!(slots.take().is_none(), "at most max at once");
        drop(held);
        assert!(slots.take().is_some());
    }
}

/// The socket with a deadline for writes (`until`). Each write waits at most until then; past it, or when a write
/// stalls, the error is BrokenPipe, so it never reads as the socket's short read timeout (WouldBlock, TimedOut).
struct Timed {
    s: TcpStream,
    until: Instant,
}

impl Read for Timed {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.s.read(buf)
    }
}

impl Write for Timed {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let stalled = || std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the peer stopped reading");
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(stalled());
        }
        self.s.set_write_timeout(Some(left))?;
        match self.s.write(buf) {
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Err(stalled()),
            r => r,
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.s.flush()
    }
}
