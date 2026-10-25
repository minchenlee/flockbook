//! JSON-RPC client over NDJSON (port of HerdrClient.swift).
//!
//! One request per short-lived connection, ids `hk:<n>`, 10 s read/write timeout. Only read methods
//! include `session.snapshot` and `events.subscribe`. Typed desktop topology writes require a client bound
//! to the isolated input-test session; `raw_request` remains generic for existing integrations.

use crate::error::HerdrError;
use crate::model::Workspace;
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
    workspace_writes: bool,
}

#[cfg(unix)]
pub use crate::transport::UnixConnector;

#[cfg(unix)]
impl HerdrClient<UnixConnector> {
    pub fn new(session: &HerdrSession) -> Self {
        let mut client = Self::with_connector(UnixConnector { path: session.socket_path.clone() });
        client.workspace_writes = session.allows_workspace_writes();
        client
    }
}

impl<C: Connector> HerdrClient<C> {
    pub fn with_connector(connector: C) -> Self {
        HerdrClient { connector, seq: AtomicU64::new(0), timeout: DEFAULT_TIMEOUT, workspace_writes: false }
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

/// Desktop topology operations. No implicit focused target; every target is explicit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceAction {
    CreateWorkspace { label: String, cwd: String },
    RenameWorkspace { id: String, label: String },
    CloseWorkspace { id: String },
    CreateTab { workspace: String, cwd: String },
    RenameTab { id: String, label: String },
    CloseTab { id: String },
    MoveTab { id: String, workspace: Option<String>, index: usize },
    Split { pane: String, down: bool },
    RenamePane { id: String, label: String },
    ClosePane { id: String },
    RightClick { pane: String, routed: bool },
}

impl<C: Connector> HerdrClient<C> {
    /// `session` is checked before connecting; production/default sessions cannot use this API.
    pub fn workspace_action(&self, session: &HerdrSession, action: &WorkspaceAction) -> Result<Value, HerdrError> {
        if !self.workspace_writes || !session.allows_workspace_writes() {
            return Err(HerdrError::Protocol("workspace writes require the isolated input-test session".into()));
        }
        let (method, params) = match action {
            WorkspaceAction::CreateWorkspace { label, cwd } => {
                let mut p = json!({"focus": false});
                if !label.trim().is_empty() { p["label"] = json!(label.trim()); }
                if !cwd.is_empty() { p["cwd"] = json!(cwd); }
                ("workspace.create", p)
            }
            WorkspaceAction::RenameWorkspace { id, label } => ("workspace.rename", json!({"workspace_id": id, "label": label.trim()})),
            WorkspaceAction::CloseWorkspace { id } => ("workspace.close", json!({"workspace_id": id})),
            WorkspaceAction::CreateTab { workspace, cwd } => {
                let mut p = json!({"workspace_id": workspace, "focus": false});
                if !cwd.is_empty() { p["cwd"] = json!(cwd); }
                ("tab.create", p)
            }
            WorkspaceAction::RenameTab { id, label } => ("tab.rename", json!({"tab_id": id, "label": label.trim()})),
            WorkspaceAction::CloseTab { id } => ("tab.close", json!({"tab_id": id})),
            WorkspaceAction::MoveTab { id, workspace, index } => {
                let mut p = json!({"tab_id": id});
                if let Some(ws) = workspace {
                    return self.move_tab_to_workspace(id, ws, *index);
                } else {
                    let ws = id.split_once(':').ok_or_else(|| HerdrError::Protocol("invalid tab id".into()))?.0;
                    let response: Value = serde_json::from_slice(&self.raw_request("tab.list", json!({"workspace_id": ws}))?)
                        .map_err(|e| HerdrError::Protocol(e.to_string()))?;
                    let tabs = response["result"]["tabs"].as_array().ok_or_else(|| HerdrError::Protocol("tab.list: missing tabs".into()))?;
                    let from = tabs.iter().position(|t| t["tab_id"].as_str() == Some(id.as_str()))
                        .ok_or_else(|| HerdrError::Protocol("tab.list: tab missing".into()))?;
                    let to = (*index).min(tabs.len().saturating_sub(1));
                    if to == from { return Ok(json!({})); }
                    p["insert_index"] = json!(tab_insert_slot(from, to));
                }
                ("tab.move", p)
            }
            WorkspaceAction::Split { pane, down } => ("pane.split", json!({"target_pane_id": pane, "direction": if *down { "down" } else { "right" }, "focus": false})),
            WorkspaceAction::RenamePane { id, label } => ("pane.rename", json!({"pane_id": id, "label": if label.trim().is_empty() { Value::Null } else { json!(label.trim()) }})),
            WorkspaceAction::ClosePane { id } => ("pane.close", json!({"pane_id": id})),
            WorkspaceAction::RightClick { pane, routed } => ("pane.input.set", json!({"pane_id": pane, "right_click": if *routed { "pane" } else { "herdr" }})),
        };
        serde_json::from_slice(&self.raw_request(method, params)?).map_err(|e| HerdrError::Protocol(e.to_string()))
    }

    /// The provider's `tab.move` only reorders. Move the existing processes with `pane.move` instead.
    /// This is a sequence, not a transaction: return successful id remaps even if a later step fails.
    fn move_tab_to_workspace(&self, id: &str, workspace: &str, index: usize) -> Result<Value, HerdrError> {
        let tree = self.snapshot()?;
        if !tree.iter().any(|w| w.id == workspace) { return Err(HerdrError::Protocol("destination workspace missing".into())); }
        let tab = tree.iter().flat_map(|w| &w.tabs).find(|t| t.id == id).ok_or_else(|| HerdrError::Protocol("source tab missing".into()))?;
        let mut target: Option<String> = None;
        let mut remaps = Vec::new();
        let mut warning: Option<String> = None;
        for pane in &tab.panes {
            let destination = if let Some(t) = &target { json!({"type": "tab", "tab_id": t, "split": "right"}) }
                else { json!({"type": "new_tab", "workspace_id": workspace, "label": tab.label}) };
            let moved = self.raw_request("pane.move", json!({"pane_id": pane.id, "destination": destination, "focus": false}))
                .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|e| HerdrError::Protocol(e.to_string())));
            let v = match moved {
                Ok(v) => v,
                Err(e) if remaps.is_empty() => return Err(e),
                Err(e) => { warning = Some(format!("Move stopped after {} pane(s): {e}. Inspect both workspaces before retrying.", remaps.len())); break; }
            };
            let moved = &v["result"]["move_result"];
            let new_pane = moved["pane"]["pane_id"].as_str();
            let new_tab = moved["pane"]["tab_id"].as_str().or_else(|| moved["created_tab"]["tab_id"].as_str());
            let (Some(p), Some(t)) = (new_pane, new_tab) else {
                warning = Some("Move response omitted new identifiers. Inspect both workspaces before retrying.".into()); break;
            };
            target = Some(t.into());
            remaps.push(json!({"from": pane.id, "to": p}));
        }
        if let Some(t) = &target {
            // A newly created tab is last; the final-index API handles pre-removal slots.
            if warning.is_none() {
                if let Err(e) = self.workspace_action_reorder(t, index) { warning = Some(format!("Panes moved; tab ordering failed: {e}")); }
            }
        }
        Ok(json!({"result": {"workspace": {"workspace_id": workspace}, "tab": {"tab_id": target}}, "moved_panes": remaps, "warning": warning}))
    }

    fn workspace_action_reorder(&self, id: &str, index: usize) -> Result<(), HerdrError> {
        let ws = id.split_once(':').ok_or_else(|| HerdrError::Protocol("invalid tab id".into()))?.0;
        let v: Value = serde_json::from_slice(&self.raw_request("tab.list", json!({"workspace_id": ws}))?).map_err(|e| HerdrError::Protocol(e.to_string()))?;
        let tabs = v["result"]["tabs"].as_array().ok_or_else(|| HerdrError::Protocol("tab.list: missing tabs".into()))?;
        let from = tabs.iter().position(|t| t["tab_id"].as_str() == Some(id)).ok_or_else(|| HerdrError::Protocol("moved tab missing".into()))?;
        let to = index.min(tabs.len().saturating_sub(1));
        if from != to { self.raw_request("tab.move", json!({"tab_id": id, "insert_index": tab_insert_slot(from, to)}))?; }
        Ok(())
    }
}

/// The server uses insertion slots in the list before removing the moved tab.
pub fn tab_insert_slot(from: usize, to: usize) -> usize { if to > from { to + 1 } else { to } }

#[cfg(test)]
mod workspace_tests {
    use super::*;
    #[test]
    fn reorder_uses_pre_removal_slots() {
        assert_eq!(tab_insert_slot(0, 2), 3);
        assert_eq!(tab_insert_slot(2, 0), 0);
        assert_eq!(tab_insert_slot(1, 1), 1);
    }
    #[test]
    fn topology_writes_require_named_test_session() {
        let home = std::path::Path::new("/synthetic-home");
        assert!(!HerdrSession::default_session(home).allows_workspace_writes());
        assert!(!HerdrSession::named("other", home).allows_workspace_writes());
        assert!(HerdrSession::named("c9-input-test", home).allows_workspace_writes());
        assert!(!HerdrSession { name: Some("c9-input-test".into()), socket_path: "/synthetic-home/herdr.sock".into() }.allows_workspace_writes());
    }
}

#[cfg(test)]
mod workspace_protocol_tests {
    use super::*;
    use crate::transport::Transport;
    use std::collections::VecDeque;
    use std::io::{self, Cursor, Read, Write};
    use std::sync::{Arc, Mutex};
    #[derive(Clone)]
    struct Script {
        responses: Arc<Mutex<VecDeque<Vec<u8>>>>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    struct Stream { response: Cursor<Vec<u8>>, writes: Arc<Mutex<Vec<Vec<u8>>>> }
    impl Read for Stream { fn read(&mut self, b: &mut [u8]) -> io::Result<usize> { self.response.read(b) } }
    impl Write for Stream {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> { self.writes.lock().unwrap().push(b.to_vec()); Ok(b.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    impl Transport for Stream { fn shutdown(&mut self) {} }
    impl Connector for Script {
        fn connect(&self, _: Option<Duration>) -> Result<Box<dyn Transport>, HerdrError> {
            let response = self.responses.lock().unwrap().pop_front().ok_or_else(|| HerdrError::Protocol("unexpected connection".into()))?;
            Ok(Box::new(Stream { response: Cursor::new(response), writes: self.writes.clone() }))
        }
    }
    fn script(responses: &[Value]) -> Script {
        Script { responses: Arc::new(Mutex::new(responses.iter().map(|v| format!("{v}\n").into_bytes()).collect())), writes: Default::default() }
    }
    #[test]
    fn forbidden_session_is_rejected_before_any_socket_request() {
        let script = script(&[]);
        let client = HerdrClient::with_connector(script.clone());
        let session = HerdrSession::default_session(std::path::Path::new("/synthetic-home"));
        assert!(client.workspace_action(&session, &WorkspaceAction::ClosePane { id: "w1:p1".into() }).is_err());
        assert!(script.writes.lock().unwrap().is_empty());
    }
    #[test]
    fn reorder_reads_current_order_then_sends_server_insertion_slot() {
        let script = script(&[json!({"result":{"tabs":[{"tab_id":"w1:t1"},{"tab_id":"w1:t2"},{"tab_id":"w1:t3"}]}}), json!({"result":{}})]);
        let mut client = HerdrClient::with_connector(script.clone());
        client.workspace_writes = true;
        let session = HerdrSession::named("c9-input-test", std::path::Path::new("/synthetic-home"));
        client.workspace_action(&session, &WorkspaceAction::MoveTab { id: "w1:t1".into(), workspace: None, index: 2 }).unwrap();
        let writes = script.writes.lock().unwrap();
        let list: Value = serde_json::from_slice(&writes[0]).unwrap();
        let moved: Value = serde_json::from_slice(&writes[1]).unwrap();
        assert_eq!(list["method"], "tab.list");
        assert_eq!(moved["method"], "tab.move");
        assert_eq!(moved["params"], json!({"tab_id":"w1:t1","insert_index":3}));
    }
    #[test]
    fn splitting_has_explicit_target_and_preserves_other_clients_focus() {
        let script = script(&[json!({"result":{}})]);
        let mut client = HerdrClient::with_connector(script.clone());
        client.workspace_writes = true;
        let session = HerdrSession::named("c9-input-test", std::path::Path::new("/synthetic-home"));
        client.workspace_action(&session, &WorkspaceAction::Split { pane: "w1:p2".into(), down: true }).unwrap();
        let request: Value = serde_json::from_slice(&script.writes.lock().unwrap()[0]).unwrap();
        assert_eq!(request["params"], json!({"target_pane_id":"w1:p2","direction":"down","focus":false}));
    }
    #[test]
    fn workspace_move_preserves_successful_id_remaps_when_a_later_pane_fails() {
        let script = script(&[
            json!({"result":{"snapshot":{
                "workspaces":[{"workspace_id":"w1","label":"Source"},{"workspace_id":"w2","label":"Target"}],
                "tabs":[{"workspace_id":"w1","tab_id":"w1:t1","label":"Synthetic"}],
                "panes":[{"pane_id":"w1:p1","tab_id":"w1:t1"},{"pane_id":"w1:p2","tab_id":"w1:t1"}]
            }}}),
            json!({"result":{"move_result":{"pane":{"pane_id":"w2:p8","tab_id":"w2:t5"},"created_tab":{"tab_id":"w2:t5"}}}}),
            json!({"error":{"code":"pane_not_found","message":"synthetic missing pane"}}),
        ]);
        let mut client = HerdrClient::with_connector(script.clone());
        client.workspace_writes = true;
        let session = HerdrSession::named("c9-input-test", std::path::Path::new("/synthetic-home"));
        let result = client.workspace_action(&session, &WorkspaceAction::MoveTab { id:"w1:t1".into(),workspace:Some("w2".into()),index:0 }).unwrap();
        assert_eq!(result["moved_panes"], json!([{"from":"w1:p1","to":"w2:p8"}]));
        assert!(result["warning"].as_str().unwrap().contains("Move stopped"));
        let writes=script.writes.lock().unwrap();
        let first:Value=serde_json::from_slice(&writes[1]).unwrap();
        let second:Value=serde_json::from_slice(&writes[2]).unwrap();
        assert_eq!(first["method"],"pane.move");
        assert_eq!(first["params"]["destination"]["type"],"new_tab");
        assert_eq!(second["params"]["destination"]["tab_id"],"w2:t5");
        assert_eq!(writes.len(),3,"no automatic retry or close of processes");
    }
}
