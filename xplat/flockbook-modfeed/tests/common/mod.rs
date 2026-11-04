//! A fake herdr-core on a Unix socket (Swift: `FakeHC.swift`, the plain-mode calls the sidecar makes). Records every
//! call. Never touches a real hc.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Mutex};

pub type OnKeys = Arc<dyn Fn(&str, &[String], &mut State) + Send + Sync>;

#[derive(Default)]
pub struct State {
    pub terminals: Vec<Value>,
    pub screens: HashMap<String, String>,
    pub calls: Vec<(String, Value)>,
    pub on_keys: Option<OnKeys>,
    /// Seconds `terminal.send` takes before it answers (0: at once; negative: never).
    pub send_delay: f64,
    /// Seconds `terminal.keys` takes before it answers (hc's typing-quiet rule can hold keys this long).
    pub keys_delay: f64,
}

pub struct FakeHc {
    pub path: String,
    pub state: Arc<Mutex<State>>,
    _dir: tempfile::TempDir,
}

/// Swift `FakeHC.terminal`: defaults as there (argv ["claude"], pid 4242, created_ms 1).
pub fn terminal(addr: &str, argv: &[&str], pid: Option<i64>, status: Option<&str>, kind: Option<&str>) -> Value {
    let mut t = json!({"address": addr, "labels": {}, "argv": argv, "cwd": "/tmp/proj", "state": "live", "created_ms": 1,
                       "screen": "exact", "cols": 80, "rows": 24, "turn": 0, "title": "", "reported": status.is_some(), "records": {},
                       "pid": pid, "parent": null, "foreground": null, "status": null});
    if let Some(s) = status {
        let mut st = json!({"state": s});
        if let Some(k) = kind {
            st["kind"] = json!(k);
        }
        t["status"] = st;
    }
    t
}

pub fn claude(addr: &str, status: &str, kind: &str) -> Value {
    terminal(addr, &["claude"], Some(4242), Some(status), Some(kind))
}

impl FakeHc {
    pub fn start() -> FakeHc {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hc.sock").to_string_lossy().to_string();
        let listener = UnixListener::bind(&path).unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let st = state.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(s) = stream else { continue };
                let st = st.clone();
                std::thread::spawn(move || serve(s, st));
            }
        });
        FakeHc { path, state, _dir: dir }
    }

    pub fn set_screen(&self, addr: &str, text: &str) {
        self.state.lock().unwrap().screens.insert(addr.into(), text.into());
    }
    pub fn set_terminals(&self, t: Vec<Value>) {
        self.state.lock().unwrap().terminals = t;
    }
    pub fn clear_screen(&self, addr: &str) {
        self.state.lock().unwrap().screens.remove(addr);
    }
    pub fn keys_delay(&self, d: f64) {
        self.state.lock().unwrap().keys_delay = d;
    }
    pub fn on_keys(&self, f: Option<OnKeys>) {
        self.state.lock().unwrap().on_keys = f;
    }
    pub fn keys(&self) -> Vec<Vec<String>> {
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(m, _)| m == "terminal.keys")
            .map(|(_, p)| p["keys"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect())
            .collect()
    }
    pub fn calls(&self, method: &str) -> Vec<Value> {
        self.state.lock().unwrap().calls.iter().filter(|(m, _)| m == method).map(|(_, p)| p.clone()).collect()
    }
}

fn serve(mut s: std::os::unix::net::UnixStream, st: Arc<Mutex<State>>) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    while reader.read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
        let o: Value = serde_json::from_str(&line).unwrap();
        line.clear();
        let id = o["id"].clone();
        let m = o["method"].as_str().unwrap_or("").to_string();
        let p = o["params"].clone();
        let reply = |r: Value| json!({"jsonrpc": "2.0", "id": id, "result": r});
        let err = |msg: String| json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": msg, "data": {"kind": "not_found"}}});
        let a = p["terminal"].as_str().unwrap_or("").to_string();
        let out = {
            let mut g = st.lock().unwrap();
            g.calls.push((m.clone(), p.clone()));
            match m.as_str() {
                "terminal.list" => Some(reply(json!({"terminals": g.terminals, "hidden_finished": 0, "machines": []}))),
                "terminal.read" => Some(match g.screens.get(&a) {
                    Some(t) => reply(json!({"text": t, "complete": true, "how": "screen", "why": null})),
                    None => err(format!("no terminal {a}")),
                }),
                "terminal.keys" => {
                    let d = g.keys_delay;
                    if d > 0.0 {
                        drop(g);
                        std::thread::sleep(std::time::Duration::from_secs_f64(d));
                        g = st.lock().unwrap();
                    }
                    if let Some(cb) = g.on_keys.clone() {
                        let keys: Vec<String> = p["keys"].as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect();
                        cb(&a, &keys, &mut g);
                    }
                    Some(reply(json!({"turn": 0, "status": "idle", "started": true})))
                }
                "terminal.send" => {
                    let d = g.send_delay;
                    if d < 0.0 {
                        None
                    } else {
                        drop(g);
                        if d > 0.0 {
                            std::thread::sleep(std::time::Duration::from_secs_f64(d));
                        }
                        Some(reply(json!({"turn": 0, "status": "idle", "started": true})))
                    }
                }
                _ => Some(err(format!("unknown method {m}"))),
            }
        };
        match out {
            Some(v) => {
                let _ = s.write_all(format!("{v}\n").as_bytes());
            }
            None => {
                std::thread::sleep(std::time::Duration::from_secs(60));
                return;
            }
        }
    }
}
