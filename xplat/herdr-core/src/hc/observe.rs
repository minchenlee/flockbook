//! A terminal's first paint, read through a watch-only attach (`terminal.attach` with `observe: true`) on a framed
//! connection. Only the paint has the screen's attributes (dim) and the cursor; `terminal.read` has text only.
//!
//! Read-only: the connection sends the framed `hello`, one attach request and a CLOSE for its channel, nothing else
//! (no keys, no resize, no takeover; the core also holds an observer to that). It ends at `limit` at the latest.
//!
//! Frames (hc framed mode): `[channel u32 LE][kind u8][len u32 LE][payload]`; channel 0 carries JSON-RPC.

use crate::error::HerdrError;
use crate::transport::{Connector, Transport};
use serde_json::{json, Value};
use std::io::{self, Write};
use std::time::{Duration, Instant};

pub const DATA: u8 = 0;
pub const CLOSE: u8 = 2;

/// Each read waits this long at most, so the whole look keeps to its limit.
const STEP: Duration = Duration::from_millis(50);
/// The paint is done once nothing more came for this long.
const QUIET: Duration = Duration::from_millis(60);
/// A frame larger than this is not one hc sends: a broken stream.
const MAX_FRAME: usize = 16 << 20;

/// The first paint of `terminal`, or an error: the connection failed, the core refused, or no paint came in `limit`.
pub fn first_paint(connector: &dyn Connector, terminal: &str, limit: Duration) -> Result<Vec<u8>, HerdrError> {
    let end = Instant::now() + limit;
    let mut sock = connector.connect(Some(STEP))?;
    let mut buf: Vec<u8> = vec![];
    let mut channel: Option<u32> = None;
    let r = look(sock.as_mut(), terminal, end, &mut buf, &mut channel);
    // Detach at once, whatever happened.
    if let Some(ch) = channel {
        let _ = sock.write_all(&frame(ch, CLOSE, &[]));
        let _ = sock.flush();
    }
    sock.shutdown();
    r
}

fn look(sock: &mut dyn Transport, terminal: &str, end: Instant, buf: &mut Vec<u8>, channel: &mut Option<u32>) -> Result<Vec<u8>, HerdrError> {
    let hello = json!({"jsonrpc": "2.0", "id": 1, "method": "hello", "params": {"generation": 1, "framed": true}});
    sock.write_all(format!("{hello}\n").as_bytes()).map_err(|e| HerdrError::Socket(format!("write: {e}")))?;
    let line = loop {
        if let Some(nl) = buf.iter().position(|b| *b == b'\n') {
            let line = buf[..nl].to_vec();
            buf.drain(..=nl);
            break line;
        }
        // One step with nothing is not the end: wait up to the limit, as for the paint.
        match fill(sock, buf, end) {
            Ok(()) => {}
            Err(HerdrError::Timeout(_)) if Instant::now() < end => {}
            Err(e) => return Err(e),
        }
    };
    let o: Value = serde_json::from_slice(&line).map_err(|e| HerdrError::Protocol(format!("hello: {e}")))?;
    if o["result"]["framed"] != true {
        return Err(HerdrError::Protocol("hello: the core did not switch to framed mode".into()));
    }
    let attach = json!({"jsonrpc": "2.0", "id": 2, "method": "terminal.attach",
                        "params": {"terminal": terminal, "cols": 80, "rows": 24, "history": 0, "observe": true}});
    sock.write_all(&frame(0, DATA, attach.to_string().as_bytes())).map_err(|e| HerdrError::Socket(format!("write: {e}")))?;

    // Frames for a channel before its attach reply is read are kept, then counted.
    let mut early: Vec<(u32, Vec<u8>)> = vec![];
    let mut paint: Vec<u8> = vec![];
    let mut last = Instant::now();
    loop {
        while let Some((ch, kind, p)) = take_frame(buf)? {
            match (ch, kind, *channel) {
                (0, DATA, None) => {
                    let o: Value = serde_json::from_slice(&p).map_err(|e| HerdrError::Protocol(format!("attach: {e}")))?;
                    if o["id"] != 2 {
                        continue;
                    }
                    if !o["error"].is_null() {
                        return Err(HerdrError::Protocol(format!("terminal.attach: {}", o["error"]["message"])));
                    }
                    let ch = o["result"]["channel"].as_u64().ok_or_else(|| HerdrError::Protocol("terminal.attach: no channel".into()))? as u32;
                    *channel = Some(ch);
                    for (_, p) in early.drain(..).filter(|(c, _)| *c == ch) {
                        paint.extend_from_slice(&p);
                        last = Instant::now();
                    }
                }
                (c, DATA, None) if c != 0 => early.push((c, p)),
                (c, DATA, Some(ch)) if c == ch => {
                    paint.extend_from_slice(&p);
                    last = Instant::now();
                }
                (c, CLOSE, Some(ch)) if c == ch => {
                    *channel = None;
                    return if paint.is_empty() { Err(HerdrError::Closed) } else { Ok(paint) };
                }
                _ => {}
            }
        }
        // Bytes left in `buf` are a frame still coming (a redraw, maybe of typed text): not quiet, and not done.
        let now = Instant::now();
        if !paint.is_empty() && buf.is_empty() && now.duration_since(last) >= QUIET {
            return Ok(paint);
        }
        if now >= end {
            return if paint.is_empty() || !buf.is_empty() { Err(HerdrError::Timeout("first paint".into())) } else { Ok(paint) };
        }
        match fill(sock, buf, end) {
            Ok(()) | Err(HerdrError::Timeout(_)) => {}
            Err(e) => return Err(e),
        }
    }
}

/// One read into `buf`; `Timeout` when nothing came in a step or the limit passed.
fn fill(sock: &mut dyn Transport, buf: &mut Vec<u8>, end: Instant) -> Result<(), HerdrError> {
    if Instant::now() >= end {
        return Err(HerdrError::Timeout("first paint".into()));
    }
    let mut chunk = [0u8; 65536];
    match sock.read(&mut chunk) {
        Ok(0) => Err(HerdrError::Closed),
        Ok(n) => {
            buf.extend_from_slice(&chunk[..n]);
            Ok(())
        }
        Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted) => {
            Err(HerdrError::Timeout("read".into()))
        }
        Err(e) => Err(HerdrError::Socket(format!("read: {e}"))),
    }
}

fn take_frame(buf: &mut Vec<u8>) -> Result<Option<(u32, u8, Vec<u8>)>, HerdrError> {
    if buf.len() < 9 {
        return Ok(None);
    }
    let n = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]) as usize;
    if n > MAX_FRAME {
        return Err(HerdrError::Protocol(format!("a frame of {n} bytes")));
    }
    if buf.len() < 9 + n {
        return Ok(None);
    }
    let ch = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let kind = buf[4];
    let p = buf[9..9 + n].to_vec();
    buf.drain(..9 + n);
    Ok(Some((ch, kind, p)))
}

fn frame(ch: u32, kind: u8, p: &[u8]) -> Vec<u8> {
    let mut f = ch.to_le_bytes().to_vec();
    f.push(kind);
    f.extend_from_slice(&(p.len() as u32).to_le_bytes());
    f.extend_from_slice(p);
    f
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::transport::UnixConnector;
    use std::io::{BufRead, BufReader, Read};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    /// What the fake core answers to the attach.
    #[derive(Clone)]
    enum Answer {
        Paint(Vec<u8>),
        /// A paint, then a redraw whose frame comes in two parts, the second this late.
        Redraw(Vec<u8>, Vec<u8>, Duration),
        Refuse,
        Silent,
    }

    /// A framed fake core for one connection. Records the hello and every frame it got, as (channel, kind, payload).
    /// The hello, then each frame as (channel, kind, payload).
    type Got = Arc<Mutex<(Value, Vec<(u32, u8, Vec<u8>)>)>>;

    fn fake(answer: Answer) -> (tempfile::TempDir, String, Got) {
        fake_slow(answer, Duration::ZERO)
    }

    /// `hello_delay`: the core answers the hello this late (a busy machine).
    fn fake_slow(answer: Answer, hello_delay: Duration) -> (tempfile::TempDir, String, Got) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hc.sock").to_string_lossy().to_string();
        let l = UnixListener::bind(&path).unwrap();
        let got = Arc::new(Mutex::new((Value::Null, vec![])));
        let g = got.clone();
        std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut w = s.try_clone().unwrap();
            let mut r = BufReader::new(s);
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let hello: Value = serde_json::from_str(&line).unwrap();
            g.lock().unwrap().0 = hello.clone();
            std::thread::sleep(hello_delay);
            w.write_all(format!("{}\n", json!({"jsonrpc": "2.0", "id": hello["id"], "result": {"framed": true}})).as_bytes()).unwrap();
            loop {
                let mut h = [0u8; 9];
                if r.read_exact(&mut h).is_err() {
                    return;
                }
                let ch = u32::from_le_bytes(h[0..4].try_into().unwrap());
                let n = u32::from_le_bytes(h[5..9].try_into().unwrap()) as usize;
                let mut p = vec![0u8; n];
                r.read_exact(&mut p).unwrap();
                g.lock().unwrap().1.push((ch, h[4], p.clone()));
                if ch == 0 && h[4] == DATA {
                    let o: Value = serde_json::from_slice(&p).unwrap();
                    let reply = match &answer {
                        Answer::Refuse => json!({"jsonrpc": "2.0", "id": o["id"], "error": {"code": -32000, "message": "no terminal", "data": {"kind": "not_found"}}}),
                        _ => json!({"jsonrpc": "2.0", "id": o["id"], "result": {"channel": 7, "terminal": {"cols": 120, "rows": 40}}}),
                    };
                    w.write_all(&frame(0, DATA, reply.to_string().as_bytes())).unwrap();
                    if let Answer::Redraw(a, b, late) = &answer {
                        w.write_all(&frame(7, DATA, a)).unwrap();
                        let f = frame(7, DATA, b);
                        let (x, y) = f.split_at(f.len() / 2);
                        w.write_all(x).unwrap();
                        std::thread::sleep(*late);
                        let _ = w.write_all(y);
                    }
                    if let Answer::Paint(b) = &answer {
                        // In two parts, as a big paint can come.
                        let (a, z) = b.split_at(b.len() / 2);
                        w.write_all(&frame(7, DATA, a)).unwrap();
                        w.write_all(&frame(7, DATA, z)).unwrap();
                    }
                }
            }
        });
        (dir, path, got)
    }

    #[test]
    fn reads_the_paint_and_only_watches() {
        let paint = b"\x1b[1;1H\xe2\x9d\xaf\xc2\xa0\x1b[2m/clear\x1b[0m\x1b[1;3H".to_vec();
        let (_d, path, got) = fake(Answer::Paint(paint.clone()));
        let p = first_paint(&UnixConnector { path }, "t1", Duration::from_millis(500)).expect("the paint");
        assert_eq!(p, paint, "both parts, in order");
        std::thread::sleep(Duration::from_millis(100));
        let g = got.lock().unwrap();
        assert_eq!(g.0["method"], "hello");
        assert_eq!(g.0["params"]["framed"], true);
        let frames = &g.1;
        assert_eq!(frames.len(), 2, "the attach and its CLOSE, nothing else ({frames:?})");
        let a: Value = serde_json::from_slice(&frames[0].2).unwrap();
        assert_eq!((frames[0].0, frames[0].1, a["method"].as_str()), (0, DATA, Some("terminal.attach")));
        assert_eq!(a["params"]["terminal"], "t1");
        assert_eq!(a["params"]["observe"], true, "watch only");
        assert_eq!((frames[1].0, frames[1].1), (7, CLOSE), "detached right after");
    }

    #[test]
    fn a_slow_hello_still_gets_the_paint() {
        let paint = b"\x1b[1;1Hpaint".to_vec();
        let (_d, path, _) = fake_slow(Answer::Paint(paint.clone()), Duration::from_millis(120));
        let p = first_paint(&UnixConnector { path }, "t1", Duration::from_millis(500));
        assert_eq!(p.ok(), Some(paint), "the hello may take more than one read step; the limit is 500 ms");
    }

    #[test]
    fn a_frame_not_all_here_is_never_quiet() {
        let (dim, typed) = (b"dim suggestion".to_vec(), b" then typed".to_vec());
        // The rest comes after the limit: no paint (the send is refused), never the old one.
        let (_d, path, _) = fake(Answer::Redraw(dim.clone(), typed.clone(), Duration::from_millis(900)));
        let p = first_paint(&UnixConnector { path }, "t1", Duration::from_millis(500));
        assert!(p.is_err(), "a frame still coming at the limit: an error ({p:?})");
        // The rest comes in time: the paint with the redraw.
        let (_d, path, _) = fake(Answer::Redraw(dim.clone(), typed.clone(), Duration::from_millis(250)));
        let p = first_paint(&UnixConnector { path }, "t1", Duration::from_millis(500));
        assert_eq!(p.ok(), Some([dim, typed].concat()), "waits for the whole frame");
    }

    #[test]
    fn a_refused_attach_is_an_error() {
        let (_d, path, _) = fake(Answer::Refuse);
        assert!(first_paint(&UnixConnector { path }, "t1", Duration::from_millis(500)).is_err());
    }

    #[test]
    fn no_paint_in_time_is_an_error_at_the_limit() {
        let (_d, path, got) = fake(Answer::Silent);
        let t0 = Instant::now();
        assert!(first_paint(&UnixConnector { path }, "t1", Duration::from_millis(300)).is_err());
        assert!(t0.elapsed() < Duration::from_millis(800), "ends at the limit ({:?})", t0.elapsed());
        std::thread::sleep(Duration::from_millis(100));
        let g = got.lock().unwrap();
        assert!(g.1.iter().any(|f| f.0 == 7 && f.1 == CLOSE), "detached all the same ({:?})", g.1);
    }

    #[test]
    fn no_core_is_an_error() {
        assert!(first_paint(&UnixConnector { path: "/nonexistent/hc.sock".into() }, "t1", Duration::from_millis(300)).is_err());
    }
}
