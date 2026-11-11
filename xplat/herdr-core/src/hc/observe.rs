//! A terminal's first paint, read through a watch-only attach (`terminal.attach` with `observe: true`) on a framed
//! connection. Only the paint has the screen's attributes (dim) and the cursor; `terminal.read` has text only.
//!
//! Read-only: the connection sends the framed `hello`, one attach request and a CLOSE for its channel, nothing else
//! (no keys, no resize, no takeover; the core also holds an observer to that). It ends at `limit` at the latest.
//!
//! Frames (hc framed mode): `[channel u32 LE][kind u8][len u32 LE][payload]`; channel 0 carries JSON-RPC.

use crate::error::HerdrError;
use crate::transport::Connector;
use std::time::Duration;

pub const DATA: u8 = 0;
pub const CLOSE: u8 = 2;

/// The first paint of `terminal`, or an error: the connection failed, the core refused, or no paint came in `limit`.
pub fn first_paint(_connector: &dyn Connector, _terminal: &str, _limit: Duration) -> Result<Vec<u8>, HerdrError> {
    Err(HerdrError::Protocol("first_paint: not written yet".into()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::transport::UnixConnector;
    use serde_json::{json, Value};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// What the fake core answers to the attach.
    #[derive(Clone)]
    enum Answer {
        Paint(Vec<u8>),
        Refuse,
        Silent,
    }

    fn frame(ch: u32, kind: u8, p: &[u8]) -> Vec<u8> {
        let mut f = ch.to_le_bytes().to_vec();
        f.push(kind);
        f.extend_from_slice(&(p.len() as u32).to_le_bytes());
        f.extend_from_slice(p);
        f
    }

    /// A framed fake core for one connection. Records the hello and every frame it got, as (channel, kind, payload).
    fn fake(answer: Answer) -> (tempfile::TempDir, String, Arc<Mutex<(Value, Vec<(u32, u8, Vec<u8>)>)>>) {
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
