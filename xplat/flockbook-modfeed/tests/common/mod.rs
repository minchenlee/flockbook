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
    /// Other machines (`machine.exec`, `file.write` with `on`): name -> its home and the socket of its core. A
    /// command there runs here with `sh -c`, HOME set to that home and HC_SOCKET to that core.
    pub machines: HashMap<String, (std::path::PathBuf, String)>,
    /// What `uname -sm` gives there: Some((code, stdout)); None runs the real one.
    pub uname: Option<(i32, String)>,
    /// `machine.exec` answers that it ran out of time.
    pub exec_timed_out: bool,
    /// A Windows machine: no `uname` (argv `uname` gives `spawn_failed`).
    pub windows: bool,
    /// `file.write` changes the bytes on the way (the sum there must catch it).
    pub corrupt_writes: bool,
    /// `machine.exec` starts in `/`, not in the home there (hc does not promise the cwd).
    pub exec_in_root: bool,
    /// `doc.get` values by name ("herdr-app"), or by "<machine>/<name>" with `on`.
    pub docs: HashMap<String, Value>,
    /// What `machine.list` gives (`machines`).
    pub machine_list: Vec<Value>,
    /// The first paint a watch-only `terminal.attach` gives (framed mode), by address; none: the attach is answered
    /// and then nothing comes.
    pub paints: HashMap<String, Vec<u8>>,
    /// `terminal.attach` answers with an error.
    pub attach_refused: bool,
    /// The terminals `terminal.list` gives once an attach has been answered (another agent came in meanwhile).
    pub after_attach: Option<Vec<Value>>,
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
    pub fn set_doc(&self, key: &str, v: Value) {
        self.state.lock().unwrap().docs.insert(key.into(), v);
    }
    pub fn set_machine_list(&self, m: Vec<Value>) {
        self.state.lock().unwrap().machine_list = m;
    }
    pub fn add_machine(&self, name: &str, home: &std::path::Path, sock: &str) {
        self.state.lock().unwrap().machines.insert(name.into(), (home.to_path_buf(), sock.into()));
    }
    pub fn set_paint(&self, addr: &str, paint: &[u8]) {
        self.state.lock().unwrap().paints.insert(addr.into(), paint.into());
    }
    pub fn refuse_attach(&self, yes: bool) {
        self.state.lock().unwrap().attach_refused = yes;
    }
    pub fn calls(&self, method: &str) -> Vec<Value> {
        self.state.lock().unwrap().calls.iter().filter(|(m, _)| m == method).map(|(_, p)| p.clone()).collect()
    }
}

pub fn b64(d: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(d)
}

/// What `machine.exec` was given: `argv` (no shell) or `line` (`sh -c`).
pub enum Run {
    Argv(Vec<String>),
    Line(String),
}

/// Runs it on this computer as that machine would: in its home, with its core. Err: an hc error kind.
fn exec_there(run: &Run, home: &std::path::Path, sock: &str, uname: Option<(i32, String)>, timed_out: bool, windows: bool, in_root: bool) -> Result<Value, String> {
    if timed_out {
        return Ok(json!({"code": null, "signal": 15, "stdout": "", "stderr": "", "truncated": false, "timed_out": true, "ms": 30000}));
    }
    let is_uname = match run {
        Run::Argv(a) => a == &["uname".to_string(), "-sm".to_string()],
        Run::Line(l) => l.trim() == "uname -sm",
    };
    if windows {
        // cmd.exe would run a line, with POSIX quotes that mean nothing to it. Tests check no line got here.
        return Err(if matches!(run, Run::Line(_)) { "line_on_windows" } else { "spawn_failed" }.into());
    }
    if let (Some((code, out)), true) = (uname, is_uname) {
        return Ok(json!({"code": code, "stdout": b64(out.as_bytes()), "stderr": "", "truncated": false, "timed_out": false, "ms": 1}));
    }
    let mut c = match run {
        Run::Argv(a) => {
            let mut c = std::process::Command::new(&a[0]);
            c.args(&a[1..]);
            c
        }
        Run::Line(l) => {
            let mut c = std::process::Command::new("sh");
            c.arg("-c").arg(l);
            c
        }
    };
    c.current_dir(if in_root { std::path::Path::new("/") } else { home }).env_clear().env("HOME", home).env("HC_SOCKET", sock).env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    match c.output() {
        Ok(o) => Ok(json!({"code": o.status.code(), "stdout": b64(&o.stdout), "stderr": b64(&o.stderr), "truncated": false, "timed_out": false, "ms": 1})),
        Err(_) => Err("spawn_failed".into()),
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
        if m == "hello" && p["framed"] == true {
            st.lock().unwrap().calls.push((m.clone(), p.clone()));
            let _ = s.write_all(format!("{}\n", json!({"jsonrpc": "2.0", "id": id, "result": {"framed": true}})).as_bytes());
            return serve_framed(s, reader, st);
        }
        let out = {
            let mut g = st.lock().unwrap();
            g.calls.push((m.clone(), p.clone()));
            match m.as_str() {
                "terminal.list" => Some(reply(json!({"terminals": g.terminals, "hidden_finished": 0, "machines": []}))),
                "doc.get" => {
                    let name = p["name"].as_str().unwrap_or("");
                    let key = p["on"].as_str().map_or(name.to_string(), |m| format!("{m}/{name}"));
                    Some(reply(json!({"name": name, "value": g.docs.get(&key).cloned().unwrap_or(Value::Null), "version": 1})))
                }
                "machine.list" => Some(reply(json!({"machines": g.machine_list}))),
                "terminal.put" => {
                    let name = p["name"].as_str().unwrap_or("");
                    Some(reply(json!({"path": format!("/remote/tmp/{name}"), "bytes": 1})))
                }
                "machine.exec" | "file.write" => {
                    let on = p["on"].as_str().unwrap_or("").to_string();
                    let Some((home, sock)) = g.machines.get(&on).cloned() else {
                        let e = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": format!("unknown machine \"{on}\""), "data": {"kind": "machine_unknown"}}});
                        let _ = s.write_all(format!("{e}\n").as_bytes());
                        continue;
                    };
                    if m == "file.write" {
                        use base64::Engine;
                        let path = home.join(p["path"].as_str().unwrap_or(""));
                        let mut data = base64::engine::general_purpose::STANDARD.decode(p["data"].as_str().unwrap_or("")).unwrap();
                        if g.corrupt_writes {
                            if let Some(b) = data.first_mut() {
                                *b ^= 0xff;
                            }
                        }
                        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                        std::fs::write(&path, &data).unwrap();
                        Some(reply(json!({"path": path, "bytes": data.len()})))
                    } else {
                        let run = match p["argv"].as_array() {
                            Some(a) => Run::Argv(a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect()),
                            None => Run::Line(p["line"].as_str().unwrap_or("").to_string()),
                        };
                        let (uname, timed_out, windows, in_root) = (g.uname.clone(), g.exec_timed_out, g.windows, g.exec_in_root);
                        drop(g);
                        Some(match exec_there(&run, &home, &sock, uname, timed_out, windows, in_root) {
                            Ok(v) => reply(v),
                            Err(kind) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": kind, "data": {"kind": kind}}}),
                        })
                    }
                }
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

fn frame(ch: u32, kind: u8, p: &[u8]) -> Vec<u8> {
    let mut f = ch.to_le_bytes().to_vec();
    f.push(kind);
    f.extend_from_slice(&(p.len() as u32).to_le_bytes());
    f.extend_from_slice(p);
    f
}

/// Framed mode: only `terminal.attach` (the channel's first paint). Every frame but a channel-0 call is recorded as
/// ("frame", {channel, kind}), so a check can see that a watcher sent no keys and no resize.
fn serve_framed(mut s: std::os::unix::net::UnixStream, mut r: BufReader<std::os::unix::net::UnixStream>, st: Arc<Mutex<State>>) {
    use std::io::Read;
    loop {
        let mut h = [0u8; 9];
        if r.read_exact(&mut h).is_err() {
            return;
        }
        let ch = u32::from_le_bytes(h[0..4].try_into().unwrap());
        let n = u32::from_le_bytes(h[5..9].try_into().unwrap()) as usize;
        let mut p = vec![0u8; n];
        if r.read_exact(&mut p).is_err() {
            return;
        }
        let mut g = st.lock().unwrap();
        if ch != 0 || h[4] != 0 {
            g.calls.push(("frame".into(), json!({"channel": ch, "kind": h[4]})));
            continue;
        }
        let o: Value = serde_json::from_slice(&p).unwrap();
        g.calls.push((o["method"].as_str().unwrap_or("").into(), o["params"].clone()));
        if o["method"] != "terminal.attach" || g.attach_refused {
            let e = json!({"jsonrpc": "2.0", "id": o["id"], "error": {"code": -32000, "message": "refused", "data": {"kind": "not_found"}}});
            let _ = s.write_all(&frame(0, 0, e.to_string().as_bytes()));
            continue;
        }
        let a = o["params"]["terminal"].as_str().unwrap_or("").to_string();
        let ok = json!({"jsonrpc": "2.0", "id": o["id"], "result": {"channel": 3, "terminal": {"cols": 80, "rows": 24}}});
        let _ = s.write_all(&frame(0, 0, ok.to_string().as_bytes()));
        if let Some(paint) = g.paints.get(&a) {
            let _ = s.write_all(&frame(3, 0, paint));
        }
        if let Some(t) = g.after_attach.take() {
            g.terminals = t;
        }
    }
}
