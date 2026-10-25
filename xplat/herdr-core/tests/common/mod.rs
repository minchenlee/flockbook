//! Shared helpers for the ported checks: fixture loading, `describe`, and an in-test fake herdr server.
#![allow(dead_code)]

use herdr_core::{ConversationItem, ItemKind, ToolCall};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Tests/HerdrKitTests/Fixtures")
}

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures_dir().join(name)).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

pub fn fixture_lines(name: &str) -> Vec<String> {
    fixture(name).lines().map(str::to_string).collect()
}

pub fn parse(lines: &[String]) -> Vec<ConversationItem> {
    herdr_core::parse_lines(lines)
}

pub fn describe(items: &[ConversationItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| match &i.kind {
            ItemKind::UserText(t) => format!("user:{t}"),
            ItemKind::AssistantText(t) => format!("asst:{t}"),
            ItemKind::ToolChips(c) => format!("chips:{}", c.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("|")),
            ItemKind::PermissionRequest(p) => format!("perm:{}", p.command),
            ItemKind::Question(q) => format!("q:{}", q.questions.iter().map(|q| q.header.as_str()).collect::<Vec<_>>().join("|")),
            ItemKind::Status(_) => "status".to_string(),
            ItemKind::Images(r) => format!("images:{}", r.len()),
        })
        .collect()
}

pub fn chip_calls(items: &[ConversationItem]) -> Vec<ToolCall> {
    items
        .iter()
        .flat_map(|i| match &i.kind {
            ItemKind::ToolChips(c) => c.iter().flat_map(|c| c.calls.clone()).collect(),
            _ => Vec::new(),
        })
        .collect()
}

pub fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    // Short names: a unix socket path is limited to about 100 bytes.
    let d = std::env::temp_dir().join(format!("hk{}-{}{}", std::process::id(), tag, N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A tiny herdr stand-in on a Unix socket (port of the Swift `FakeHerdr`).
/// Answers `session.snapshot` with one workspace labelled `label`, keeps `events.subscribe`
/// connections open, and can push an event, drop every connection, or stop listening.
pub struct FakeHerdr {
    pub path: PathBuf,
    state: Arc<State>,
}

#[derive(Default)]
struct State {
    label: Mutex<String>,
    snapshots: AtomicUsize,
    stop: AtomicBool,
    conns: Mutex<Vec<UnixStream>>,
    subscribers: Mutex<Vec<UnixStream>>,
    /// When set, `session.snapshot` stalls this long before answering (timeout test).
    stall_ms: AtomicUsize,
    seen: Mutex<Vec<String>>,
}

impl FakeHerdr {
    pub fn new(path: PathBuf) -> Self {
        let state = Arc::new(State::default());
        *state.label.lock().unwrap() = "v0".into();
        FakeHerdr { path, state }
    }
    pub fn set_label(&self, l: &str) {
        *self.state.label.lock().unwrap() = l.into();
    }
    pub fn snapshot_count(&self) -> usize {
        self.state.snapshots.load(Ordering::SeqCst)
    }
    pub fn set_stall_ms(&self, ms: usize) {
        self.state.stall_ms.store(ms, Ordering::SeqCst);
    }
    /// Every method name the fake received, in order.
    pub fn methods_seen(&self) -> Vec<String> {
        self.state.seen.lock().unwrap().clone()
    }

    pub fn start(&self) {
        let _ = std::fs::remove_file(&self.path);
        let listener = UnixListener::bind(&self.path).expect("fake bind");
        listener.set_nonblocking(true).unwrap();
        self.state.stop.store(false, Ordering::SeqCst);
        let st = self.state.clone();
        thread::spawn(move || loop {
            if st.stop.load(Ordering::SeqCst) {
                return;
            }
            match listener.accept() {
                Ok((c, _)) => {
                    c.set_nonblocking(false).unwrap();
                    st.conns.lock().unwrap().push(c.try_clone().unwrap());
                    let st2 = st.clone();
                    thread::spawn(move || serve(c, st2));
                }
                Err(_) => thread::sleep(Duration::from_millis(2)),
            }
        });
    }

    /// Socket loss: stop listening, close every connection, remove the socket file.
    pub fn kill(&self) {
        self.state.stop.store(true, Ordering::SeqCst);
        for c in self.state.conns.lock().unwrap().drain(..) {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
        self.state.subscribers.lock().unwrap().clear();
        thread::sleep(Duration::from_millis(10));
        let _ = std::fs::remove_file(&self.path);
    }

    /// Sends one event line to every subscriber.
    pub fn push_event(&self) {
        let line = b"{\"event\":\"workspace.renamed\",\"data\":{\"workspace_id\":\"w1\"}}\n";
        for s in self.state.subscribers.lock().unwrap().iter_mut() {
            let _ = s.write_all(line);
        }
    }
}

fn serve(mut c: UnixStream, st: Arc<State>) {
    let reader = BufReader::new(c.try_clone().unwrap());
    for line in reader.lines() {
        let Ok(line) = line else { return };
        let Ok(o) = serde_json::from_str::<Value>(&line) else { continue };
        let id = o["id"].as_str().unwrap_or("").to_string();
        let method = o["method"].as_str().unwrap_or("").to_string();
        st.seen.lock().unwrap().push(method.clone());
        let reply = match method.as_str() {
            "session.snapshot" => {
                st.snapshots.fetch_add(1, Ordering::SeqCst);
                let ms = st.stall_ms.load(Ordering::SeqCst);
                if ms > 0 {
                    thread::sleep(Duration::from_millis(ms as u64));
                }
                let l = st.label.lock().unwrap().clone();
                json!({"id": id, "result": {"type": "snapshot", "snapshot": {
                    "workspaces": [{"workspace_id": "w1", "label": l, "number": 1}],
                    "tabs": [{"tab_id": "w1:t1", "workspace_id": "w1", "label": "1", "number": 1}],
                    "panes": [{"pane_id": "w1:p1", "tab_id": "w1:t1", "workspace_id": "w1", "terminal_id": "term_1", "cwd": "/tmp"}],
                    "agents": []}}})
            }
            "events.subscribe" => {
                st.subscribers.lock().unwrap().push(c.try_clone().unwrap());
                json!({"id": id, "result": {"type": "subscription_started"}})
            }
            "test.garbage" => {
                let _ = c.write_all(b"not json\n");
                continue;
            }
            _ => json!({"id": id, "error": {"code": "unknown", "message": "fake"}}),
        };
        let mut out = reply.to_string().into_bytes();
        out.push(b'\n');
        if c.write_all(&out).is_err() {
            return;
        }
    }
}
