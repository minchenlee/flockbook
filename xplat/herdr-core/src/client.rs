//! JSON-RPC client over NDJSON (port of the read-only part of HerdrClient.swift).
//!
//! One request per short-lived connection, ids `hk:<n>`, 10 s read/write timeout. Only read methods
//! are exposed here (`session.snapshot`, `events.subscribe`). Mutating methods are out of scope for this gate;
//! `raw_request` is generic, so callers decide what to send.

use crate::error::HerdrError;
use crate::model::Workspace;
#[cfg(unix)]
use crate::session::HerdrSession;
use crate::snapshot;
use crate::transport::{Connector, LineSocket};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Event types for `events.subscribe`. `pane.agent_status_changed` is left out on purpose: herdr requires a `pane_id`
/// for it and rejects the whole subscription otherwise. Status changes arrive via `pane.updated`.
pub const EVENT_TYPES: &[&str] = &[
    "workspace.created", "workspace.updated", "workspace.renamed", "workspace.moved", "workspace.reordered",
    "tab.created", "tab.renamed", "tab.moved", "tab.focused",
    "pane.created", "pane.moved", "pane.updated", "pane.closed", "pane.agent_detected",
    "workspace.closed", "tab.closed", "layout.updated",
];

pub struct HerdrClient<C: Connector> {
    connector: C,
    seq: AtomicU64,
    timeout: Duration,
}

#[cfg(unix)]
pub use crate::transport::UnixConnector;

#[cfg(unix)]
impl HerdrClient<UnixConnector> {
    pub fn new(session: &HerdrSession) -> Self {
        Self::with_connector(UnixConnector { path: session.socket_path.clone() })
    }
}

impl<C: Connector> HerdrClient<C> {
    pub fn with_connector(connector: C) -> Self {
        HerdrClient { connector, seq: AtomicU64::new(0), timeout: DEFAULT_TIMEOUT }
    }

    /// A stuck server must not hold the caller forever; this bounds each read and write.
    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    /// One request on its own connection. Returns the full response line; a server `error` becomes `HerdrError::Server`.
    pub fn raw_request(&self, method: &str, params: Value) -> Result<Vec<u8>, HerdrError> {
        let n = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let body = json!({"id": format!("hk:{n}"), "method": method, "params": params});
        let mut sock = LineSocket::connect(&self.connector, Some(self.timeout))?;
        let result = (|| {
            sock.write_line(body.to_string().as_bytes())?;
            let line = sock.read_line()?;
            throw_if_error(&line)?;
            Ok(line)
        })();
        sock.close();
        result
    }

    /// Read-only: the whole session tree.
    pub fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError> {
        snapshot::parse(&self.raw_request("session.snapshot", json!({}))?)
    }

    /// Subscribes to change events. The stream yields once for the subscription ack, then once per event line.
    /// It ends with an `Err` when the connection drops, then returns `None`.
    pub fn events(&self) -> Result<EventStream, HerdrError> {
        let mut sock = LineSocket::connect(&self.connector, None)?;
        let subs: Vec<Value> = EVENT_TYPES.iter().map(|t| json!({"type": t})).collect();
        let body = json!({"id": "hk:sub", "method": "events.subscribe", "params": {"subscriptions": subs}});
        sock.write_line(body.to_string().as_bytes())?;
        let ack = sock.read_line()?;
        throw_if_error(&ack)?;
        Ok(EventStream { sock, state: StreamState::Ack })
    }
}

enum StreamState {
    Ack,
    Reading,
    Done,
}

pub struct EventStream {
    sock: LineSocket,
    state: StreamState,
}

impl EventStream {
    pub fn close(&mut self) {
        self.sock.close();
        self.state = StreamState::Done;
    }
}

impl Iterator for EventStream {
    type Item = Result<(), HerdrError>;
    fn next(&mut self) -> Option<Self::Item> {
        match self.state {
            StreamState::Done => None,
            StreamState::Ack => {
                self.state = StreamState::Reading;
                Some(Ok(()))
            }
            StreamState::Reading => loop {
                match self.sock.read_line() {
                    Ok(line) => {
                        // Any object with an `event` or a `type` counts (including `events_lost`).
                        if let Ok(Value::Object(o)) = serde_json::from_slice::<Value>(&line) {
                            if o.contains_key("event") || o.contains_key("type") {
                                return Some(Ok(()));
                            }
                        }
                    }
                    Err(e) => {
                        self.state = StreamState::Done;
                        return Some(Err(e));
                    }
                }
            },
        }
    }
}

pub(crate) fn throw_if_error(line: &[u8]) -> Result<(), HerdrError> {
    let v: Value = serde_json::from_slice(line).map_err(|_| HerdrError::Protocol("response is not an object".into()))?;
    let Value::Object(o) = v else { return Err(HerdrError::Protocol("response is not an object".into())) };
    if let Some(Value::Object(err)) = o.get("error") {
        return Err(HerdrError::Server {
            code: err.get("code").and_then(Value::as_str).unwrap_or("error").to_string(),
            message: err.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
        });
    }
    Ok(())
}
