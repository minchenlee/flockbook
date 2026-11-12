//! Swift: `HCClient.swift`, the plain-mode part: each JSON-RPC call on its own short-lived connection. No attach,
//! no layout writes, no remote log mirrors (the sidecar does not reach them).

use super::{keys, mapper, HcSession};
use crate::error::HerdrError;
use crate::model::*;
use crate::screen::{DeliveryOutcome, PaneScreen};
use crate::transport::{Connector, LineSocket};
#[cfg(not(unix))]
use crate::transport::Transport;
use serde_json::{json, Value};
use std::cell::Cell;
use std::time::Duration;

pub struct HcClient {
    pub session: HcSession,
    connector: Box<dyn Connector>,
    seq: Cell<i64>,
    /// Seconds a plain call waits for its reply (per read, as Swift's socket timeout).
    pub call_timeout: Duration,
    /// Seconds `terminal.send` waits: the core answers only after nobody typed for ~3 s and the program went `working`.
    pub send_timeout: Duration,
}

/// The `result` object (empty when null or not an object), or the error as `Server{code: kind}`.
pub fn rpc_result(o: &Value) -> Result<Value, HerdrError> {
    if let Some(err) = o.get("error").filter(|e| e.is_object()) {
        let kind = err.get("data").and_then(|d| d.get("kind")).and_then(Value::as_str);
        let code = kind.map(String::from).unwrap_or_else(|| err.get("code").and_then(Value::as_i64).unwrap_or(0).to_string());
        return Err(HerdrError::Server { code, message: err.get("message").and_then(Value::as_str).unwrap_or("").into() });
    }
    Ok(match o.get("result") {
        Some(r) if r.is_object() => r.clone(),
        _ => json!({}),
    })
}

/// `s` between single quotes for /bin/sh (Swift `ShellQuote`).
pub fn shell_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `s` as it is when every character is plain, else `shell_quoted(s)`.
pub fn shell_quoted_if_needed(s: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+=:@%,".contains(c);
    if !s.is_empty() && s.chars().all(plain) {
        s.to_string()
    } else {
        shell_quoted(s)
    }
}

/// The Unix socket at the session's path.
#[cfg(unix)]
fn connector_for(session: &HcSession) -> Box<dyn Connector> {
    Box::new(crate::transport::UnixConnector { path: session.socket_path.clone() })
}

/// TODO(windows): herdr-core's endpoint on Windows is not documented upstream (docs/security.md names Unix sockets
/// only). Until it is, every call fails closed with a socket error.
#[cfg(not(unix))]
fn connector_for(_session: &HcSession) -> Box<dyn Connector> {
    struct NoEndpoint(String);
    impl Connector for NoEndpoint {
        fn connect(&self, _timeout: Option<Duration>) -> Result<Box<dyn Transport>, HerdrError> {
            Err(HerdrError::Socket(format!("connect {} failed: herdr-core sockets are not supported on this OS yet", self.0)))
        }
    }
    Box::new(NoEndpoint("herdr-core".into()))
}

impl HcClient {
    pub fn new(session: HcSession, call_timeout_s: u64, send_timeout_s: u64) -> HcClient {
        HcClient {
            connector: connector_for(&session),
            session,
            seq: Cell::new(0),
            call_timeout: Duration::from_secs(call_timeout_s),
            send_timeout: Duration::from_secs(send_timeout_s),
        }
    }

    /// One plain-mode call on its own connection. Skips notifications until the reply with our id.
    /// `Timeout` once the request was written: it may still take effect.
    pub fn call(&self, method: &str, params: Value, timeout: Option<Duration>) -> Result<Value, HerdrError> {
        let id = self.seq.get() + 1;
        self.seq.set(id);
        let body = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .map_err(|e| HerdrError::Protocol(e.to_string()))?;
        let mut sock = LineSocket::connect(self.connector.as_ref(), Some(timeout.unwrap_or(self.call_timeout)))?;
        sock.write_line(&body)?;
        loop {
            let line = sock.read_line()?;
            let Ok(o) = serde_json::from_slice::<Value>(&line) else { continue };
            if o.is_object() && o.get("id").and_then(Value::as_i64) == Some(id) {
                return rpc_result(&o);
            }
        }
    }

    /// The pane's first paint (attributes and cursor), through a watch-only attach that ends at `limit` at the
    /// latest. See `observe`.
    pub fn first_paint(&self, pane_id: &str, limit: Duration) -> Result<Vec<u8>, HerdrError> {
        super::observe::first_paint(self.connector.as_ref(), &Self::address(pane_id)?, limit)
    }

    fn address(pane_id: &str) -> Result<String, HerdrError> {
        mapper::address_from_pane_id(pane_id)
            .filter(|a| !a.is_empty())
            .ok_or_else(|| HerdrError::Protocol(format!("not an hc pane: {pane_id}")))
    }

    /// `terminal.send`: pastes `text` and presses Enter once nobody has typed for ~3 s.
    pub fn send(&self, text: &str, pane_id: &str) -> Result<(), HerdrError> {
        self.call("terminal.send", json!({"terminal": Self::address(pane_id)?, "text": text}), Some(self.send_timeout))?;
        Ok(())
    }

    /// `send`, as the composer needs it: a timeout is `Unconfirmed`, anything else that fails is `Failed`.
    pub fn deliver(&self, text: &str, pane_id: &str) -> DeliveryOutcome {
        match self.send(text, pane_id) {
            Ok(()) => DeliveryOutcome::Sent,
            Err(HerdrError::Timeout(_)) => DeliveryOutcome::Unconfirmed,
            Err(_) => DeliveryOutcome::Failed,
        }
    }

    /// `deliver` with pasted files named in `text`. A terminal on another machine cannot read them: each is first put
    /// beside it (`terminal.put`) and the text names that copy. A file that cannot be read or put fails before
    /// anything is sent.
    pub fn deliver_files(&self, text: &str, pane_id: &str, files: &[String]) -> DeliveryOutcome {
        match self.placed(text, pane_id, files) {
            Some(typed) => self.deliver(&typed, pane_id),
            None => DeliveryOutcome::Failed,
        }
    }

    pub fn placed(&self, text: &str, pane_id: &str, files: &[String]) -> Option<String> {
        let a = Self::address(pane_id).ok()?;
        let mut text = text.to_string();
        if !mapper::machine_prefix(&a).is_empty() {
            for f in files {
                if !text.contains(f.as_str()) {
                    continue;
                }
                let data = std::fs::read(f).ok()?;
                let name = std::path::Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let there = self.put(pane_id, &name, &data).ok()?;
                let q = shell_quoted_if_needed(&there);
                for form in [format!("'{f}'"), shell_quoted_if_needed(f)] {
                    if form != *f {
                        text = text.replace(&form, &q);
                    }
                }
                text = text.replace(f.as_str(), &q);
            }
        }
        Some(text)
    }

    /// `terminal.put`: a file beside the terminal, on its machine. The path there.
    pub fn put(&self, pane_id: &str, name: &str, data: &[u8]) -> Result<String, HerdrError> {
        use base64::Engine;
        let r = self.call(
            "terminal.put",
            json!({"terminal": Self::address(pane_id)?, "name": name, "data": base64::engine::general_purpose::STANDARD.encode(data)}),
            None,
        )?;
        r.get("path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(String::from)
            .ok_or_else(|| HerdrError::Protocol("terminal.put gave no path".into()))
    }
}

impl PaneScreen for HcClient {
    fn backend(&self) -> BackendKind {
        BackendKind::Hc
    }

    /// `terminal.list`, mapped by the mapper (live terminals only).
    fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError> {
        let list = self.call("terminal.list", json!({}), None)?;
        let terms = list.get("terminals").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(mapper::workspaces(&terms))
    }

    /// `terminal.read` with no history: the visible screen as text. Read-only.
    fn visible_text(&self, pane_id: &str) -> Result<String, HerdrError> {
        let r = self.call("terminal.read", json!({"terminal": Self::address(pane_id)?}), None)?;
        r.get("text").and_then(Value::as_str).map(String::from).ok_or_else(|| HerdrError::Protocol("terminal.read has no text".into()))
    }

    /// Claude's permission prompt: `1` allows once, `esc` denies.
    fn respond(&self, pane_id: &str, allow: bool) -> Result<(), HerdrError> {
        let key = if allow { "1".to_string() } else { keys::translate("Escape") };
        self.call("terminal.keys", json!({"terminal": Self::address(pane_id)?, "keys": [key]}), None)?;
        Ok(())
    }

    fn send_named_keys(&self, k: &[String], pane_id: &str) -> Result<(), HerdrError> {
        let names: Vec<String> = k.iter().map(|x| keys::translate(x)).collect();
        self.call("terminal.keys", json!({"terminal": Self::address(pane_id)?, "keys": names}), None)?;
        Ok(())
    }

    fn send_keys(&self, bytes: &[u8], pane_id: &str) -> Result<(), HerdrError> {
        let names = keys::names(bytes);
        if names.is_empty() {
            return Ok(());
        }
        self.call("terminal.keys", json!({"terminal": Self::address(pane_id)?, "keys": names}), None)?;
        Ok(())
    }

    fn send_text(&self, text: &str, pane_id: &str) -> Result<(), HerdrError> {
        self.send(text, pane_id)
    }

    fn can_type(&self, text: &str) -> bool {
        text.chars().all(keys::typeable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_errors() {
        let e = rpc_result(&json!({"id":1,"error":{"code":-32000,"message":"no such terminal","data":{"kind":"not_found"}}})).unwrap_err();
        assert_eq!(e, HerdrError::Server { code: "not_found".into(), message: "no such terminal".into() });
        let e = rpc_result(&json!({"id":1,"error":{"code":-32601,"message":"x"}})).unwrap_err();
        assert_eq!(e, HerdrError::Server { code: "-32601".into(), message: "x".into() });
        assert_eq!(rpc_result(&json!({"id":1,"result":null})).unwrap(), json!({}));
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quoted_if_needed("/tmp/a.png"), "/tmp/a.png");
        assert_eq!(shell_quoted_if_needed("/tmp/a b.png"), "'/tmp/a b.png'");
        assert_eq!(shell_quoted("it's"), "'it'\\''s'");
    }
}

/// A fake herdr-core on a Unix socket for checks: answers each call from `reply(method, params)`.
#[cfg(all(test, unix))]
pub mod fake_core {
    use serde_json::Value;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    pub struct FakeCore {
        pub path: String,
        pub calls: Arc<Mutex<Vec<(String, Value)>>>,
        _dir: tempfile::TempDir,
    }

    pub fn start(reply: impl Fn(&str, &Value) -> Option<Value> + Send + Sync + 'static) -> FakeCore {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hc.sock").to_string_lossy().to_string();
        let listener = UnixListener::bind(&path).unwrap();
        let calls = Arc::new(Mutex::new(vec![]));
        let c2 = calls.clone();
        std::thread::spawn(move || {
            let mut held = vec![];
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut line = String::new();
                if BufReader::new(s.try_clone().unwrap()).read_line(&mut line).is_err() {
                    continue;
                }
                let o: Value = serde_json::from_str(&line).unwrap();
                let m = o["method"].as_str().unwrap().to_string();
                c2.lock().unwrap().push((m.clone(), o["params"].clone()));
                // A notification first: the client must skip it.
                let _ = s.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"event\",\"params\":{}}\n");
                if let Some(r) = reply(&m, &o["params"]) {
                    let out = serde_json::json!({"jsonrpc":"2.0","id":o["id"],"result":r});
                    let _ = s.write_all(format!("{out}\n").as_bytes());
                } else {
                    held.push(s); // no answer, connection kept open: the client times out
                }
            }
        });
        FakeCore { path, calls, _dir: dir }
    }
}

#[cfg(all(test, unix))]
mod socket_tests {
    use super::*;

    #[test]
    fn calls_over_a_socket() {
        let core = fake_core::start(|m, p| match m {
            "terminal.list" => Some(json!({"terminals":[{"address":"t1","argv":["claude"],"pid":5,"status":{"state":"blocked","kind":"permission"}}]})),
            "terminal.read" => Some(json!({"text": format!("screen of {}", p["terminal"].as_str().unwrap())})),
            "terminal.keys" => Some(json!({})),
            "terminal.send" => None, // never answers: a timeout
            _ => None,
        });
        let c = HcClient::new(HcSession::new(&core.path), 2, 1);
        let ws = c.snapshot().unwrap();
        let p = crate::screen::all_panes(&ws)[0].clone();
        assert_eq!(p.id, "hc:t1");
        assert_eq!(p.agent.unwrap().process.as_deref(), Some("5@0"));
        assert_eq!(c.visible_text("hc:t1").unwrap(), "screen of t1");
        c.respond("hc:t1", false).unwrap();
        c.respond("hc:t1", true).unwrap();
        c.send_named_keys(&["Down".into(), "Enter".into()], "hc:t1").unwrap();
        assert_eq!(c.deliver("hi", "hc:t1"), DeliveryOutcome::Unconfirmed);
        assert!(c.visible_text("w1:p1").is_err());
        let calls = core.calls.lock().unwrap().clone();
        let keys: Vec<Value> = calls.iter().filter(|(m, _)| m == "terminal.keys").map(|(_, p)| p["keys"].clone()).collect();
        assert_eq!(keys, vec![json!(["esc"]), json!(["1"]), json!(["down", "enter"])]);
        assert_eq!(calls.last().unwrap().1, json!({"terminal":"t1","text":"hi"}));
    }

    #[test]
    fn no_core_is_a_socket_error() {
        let c = HcClient::new(HcSession::new("/nonexistent/hc.sock"), 1, 1);
        assert!(matches!(c.snapshot(), Err(HerdrError::Socket(_))));
    }
}
