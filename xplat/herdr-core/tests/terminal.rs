#![cfg(unix)] // a fake herdr server on a Unix socket
//! Port of ChecksTerminal.swift (checksTerminalSession, checksScroll, checksStoreTiming).
//! Everything runs against fakes: a shell script that stands in for `herdr terminal session ...`, canned JSON, and the
//! in-test fake herdr socket. Live writes to a real herdr are for the `c9-input-test` session only and never happen here.
#![cfg(unix)]
mod common;
use common::*;
use base64::Engine;
use herdr_core::scrollback::{ScrollKind, Scrollback, ScrollInfo};
use herdr_core::store::HerdrStore;
use herdr_core::terminal::{MouseEncoding, TerminalEvent, TerminalFrame, TerminalMode, TerminalSession};
use herdr_core::session_log::ConversationLoadState;
use herdr_core::HerdrSession;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn until(sec: f64, mut ok: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed().as_secs_f64() < sec {
        if ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    ok()
}

type Events = Arc<Mutex<Vec<TerminalEvent>>>;

fn collect(rx: std::sync::mpsc::Receiver<TerminalEvent>) -> Events {
    let ev: Events = Arc::new(Mutex::new(Vec::new()));
    let e2 = ev.clone();
    std::thread::spawn(move || {
        for e in rx {
            e2.lock().unwrap().push(e);
        }
    });
    ev
}

fn frames(ev: &Events) -> Vec<TerminalFrame> {
    ev.lock().unwrap().iter().filter_map(|e| if let TerminalEvent::Frame(f) = e { Some(f.clone()) } else { None }).collect()
}

fn has_closed(ev: &Events, pred: impl Fn(&str) -> bool) -> bool {
    ev.lock().unwrap().iter().any(|e| matches!(e, TerminalEvent::Closed { reason } if pred(reason)))
}

const SCRIPT: &str = r#"#!/bin/sh
echo "ARGS $@" >> "@LOG@"
if [ "@MODE@" = takeover ]; then echo '{"type":"terminal.closed","reason":"terminal attach taken over"}'; exit 0; fi
printf '{"type":"terminal.frame","seq":1,"full":true,"encoding":"ansi","width":80,"height":24,"bytes":"%s"}\n' "$(printf '\033[?2004hhello' | base64)"
while IFS= read -r line; do
  echo "$line" >> "@LOG@"
  case "$line" in
    *terminal.resize*) printf '{"type":"terminal.frame","seq":2,"full":true,"width":100,"height":30,"bytes":"%s"}\n' "$(printf '\033[?2004l' | base64)" ;;
    *crash*) exit 1 ;;
    *release*) printf '{"type":"terminal.closed","reason":"detached"}\n'; exit 0 ;;
  esac
done
"#;

fn write_script(dir: &Path, name: &str, log: &Path, mode: &str) -> String {
    let p = dir.join(name);
    std::fs::write(&p, SCRIPT.replace("@LOG@", &log.to_string_lossy()).replace("@MODE@", mode)).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p.to_string_lossy().into_owned()
}

fn log_text(log: &Path) -> String {
    std::fs::read_to_string(log).unwrap_or_default()
}

fn spawns(log: &Path) -> usize {
    log_text(log).matches("ARGS ").count()
}

fn session() -> HerdrSession {
    HerdrSession { name: None, socket_path: String::new() }
}

#[test]
fn terminal_session_against_fake_control() {
    let dir = temp_dir("t");
    let log = dir.join("log.txt");
    let script = write_script(&dir, "fake.sh", &log, "normal");

    let (ts, rx) = TerminalSession::new("term_x", &session(), TerminalMode::Control { takeover: false }, None, None, Some(vec![script.clone()]));
    let ev = collect(rx);
    ts.start();
    assert!(until(3.0, || frames(&ev).first().map(|f| f.full && f.width == 80) == Some(true)), "term: first frame is full, 80 wide");
    assert!(ts.size() == Some((80, 24)), "term: size from the frame");
    assert!(ts.bracketed_paste(), "term: bracketed paste on by default / after ?2004h");
    assert!(log_text(&log).contains("ARGS terminal session control term_x"), "term: control args");

    let bytes: Vec<u8> = vec![0x1b, 0x5b, 0x41, 0x03, 0xff, 0xe4, 0xbd, 0xa0];
    ts.send_bytes(&bytes);
    assert!(until(2.0, || log_text(&log).contains(&b64(&bytes))), "term: arbitrary bytes sent as base64 terminal.input");
    ts.send_text("你好");
    assert!(until(2.0, || log_text(&log).contains(&b64("你好".as_bytes()))), "term: UTF-8 text sent");
    ts.paste("a\nb", None);
    assert!(until(2.0, || log_text(&log).contains(&b64("\u{1b}[200~a\nb\u{1b}[201~".as_bytes()))), "term: paste wrapped in bracketed paste");
    ts.paste("x\u{1b}[201~\nrm -rf y\n", None);
    assert!(until(2.0, || log_text(&log).contains(&b64("\u{1b}[200~x[201~\nrm -rf y\n\u{1b}[201~".as_bytes()))), "term: an end marker inside the paste loses its ESC");

    ts.resize(100, 30);
    assert!(until(2.0, || log_text(&log).contains(r#""cols":100"#) && log_text(&log).contains(r#""rows":30"#)), "term: resize sent");
    assert!(until(2.0, || ts.size() == Some((100, 30))), "term: size follows the resize frame");
    assert!(!ts.bracketed_paste(), "term: ?2004l in a frame turns bracketed paste off");
    ts.resize(0, 30);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!log_text(&log).contains(r#""cols":0"#), "term: resize with 0 is ignored");

    ts.right_click(10, 5, MouseEncoding::Sgr);
    assert!(until(2.0, || log_text(&log).contains(&b64("\u{1b}[<2;10;5M\u{1b}[<2;10;5m".as_bytes()))), "term: right click as SGR press+release at col,row");
    ts.right_click(3, 4, MouseEncoding::Legacy);
    assert!(until(2.0, || log_text(&log).contains(&b64(&[0x1b, 0x5b, 0x4d, 34, 35, 36, 0x1b, 0x5b, 0x4d, 35, 35, 36]))), "term: right click legacy (X10) encoding");
    assert!(!ts.right_click(0, 4, MouseEncoding::Sgr), "term: right click with col 0 is ignored");
    ts.wheel(3, true, false);
    assert!(
        until(2.0, || {
            let t = log_text(&log);
            t.contains(r#""type":"terminal.scroll""#) && t.contains(r#""lines":3"#) && t.contains(r#""direction":"up""#)
        }),
        "term: wheel sends terminal.scroll"
    );
    ts.wheel(2, false, true);
    assert!(
        until(2.0, || {
            let t = log_text(&log);
            t.matches(r#""direction":"down""#).count() == 2 && t.contains(r#""lines":1"#)
        }),
        "term: wheel eventPerLine sends one scroll per line"
    );
    assert!(!ts.wheel(0, true, false), "term: wheel(0) sends nothing");

    let spawns0 = spawns(&log);
    ts.send_raw(r#"{"type":"crash"}"#);
    assert!(
        until(5.0, || ev.lock().unwrap().iter().any(|e| matches!(e, TerminalEvent::Reconnecting { .. })) && frames(&ev).len() >= 3),
        "term: reconnects after the process dies, new full frame"
    );
    let t2 = log_text(&log);
    assert!(t2.matches("ARGS ").count() == spawns0 + 1 && t2.contains("--cols 100 --rows 30"), "term: reconnect keeps the last size");

    ts.release();
    assert!(until(3.0, || has_closed(&ev, |r| r == "detached")), "term: release -> closed(detached)");
    std::thread::sleep(Duration::from_millis(800));
    assert!(spawns(&log) == spawns0 + 1, "term: no reconnect after release");

    // taken over: reported once, never restarted
    let log2 = dir.join("log2.txt");
    let script2 = write_script(&dir, "fake2.sh", &log2, "takeover");
    let (ts2, rx2) = TerminalSession::new("term_y", &session(), TerminalMode::Control { takeover: false }, None, None, Some(vec![script2]));
    let ev2 = collect(rx2);
    ts2.start();
    assert!(until(3.0, || has_closed(&ev2, |r| r.contains("taken over"))), "term: taken over is reported");
    std::thread::sleep(Duration::from_millis(800));
    assert!(spawns(&log2) == 1, "term: no reconnect after taken over");

    let (plain, _rx) = TerminalSession::new("term_z", &session(), TerminalMode::Observe, None, None, Some(vec![script]));
    assert!(
        !plain.right_click(1, 1, MouseEncoding::Sgr) && !plain.wheel(1, true, false) && !plain.paste("a\nb", None),
        "term: observe mode cannot send"
    );
}

#[test]
fn control_args_carry_takeover_and_size() {
    let dir = temp_dir("t");
    let log = dir.join("log.txt");
    let script = write_script(&dir, "fake.sh", &log, "normal");
    let (ts, rx) = TerminalSession::new("term_q", &session(), TerminalMode::Control { takeover: true }, Some(90), Some(20), Some(vec![script]));
    let ev = collect(rx);
    ts.start();
    assert!(until(3.0, || !frames(&ev).is_empty()));
    assert!(log_text(&log).contains("ARGS terminal session control term_q --takeover --cols 90 --rows 20"), "{}", log_text(&log));
    ts.stop();
}

#[test]
fn missing_binary_reports_and_finishes() {
    let (ts, rx) = TerminalSession::new("term_m", &session(), TerminalMode::Observe, None, None, Some(vec!["/nonexistent/herdr-fake".into()]));
    ts.start();
    let mut notice = false;
    let t = Instant::now();
    while let Ok(e) = rx.recv_timeout(Duration::from_secs(3)) {
        if matches!(&e, TerminalEvent::Notice(n) if n.contains("cannot start herdr")) {
            notice = true;
        }
        if t.elapsed() > Duration::from_secs(3) {
            break;
        }
    }
    assert!(notice, "start failure is reported as a notice, then the stream ends");
}

// Scroll info and scrollback parsing (fixtures from herdr 0.9.1 responses)
#[test]
fn scroll_info_and_scrollback() {
    let pane_shell = br#"{"id":"x","result":{"type":"pane_info","pane":{"pane_id":"w4:p1","scroll":{"max_offset_from_bottom":363,"offset_from_bottom":12,"viewport_rows":40}}}}"#;
    let pane_flat = br#"{"id":"x","result":{"type":"pane_info","pane":{"pane_id":"w4:p1","scroll":{"max_offset_from_bottom":0,"offset_from_bottom":0,"viewport_rows":40}}}}"#;
    let proc_shell = br#"{"id":"x","result":{"type":"pane_process_info","process_info":{"foreground_process_group_id":5303,"foreground_processes":[{"name":"zsh","pid":5303}],"shell_pid":5303}}}"#;
    let proc_less = br#"{"id":"x","result":{"type":"pane_process_info","process_info":{"foreground_process_group_id":20415,"foreground_processes":[{"name":"less","pid":20415}],"shell_pid":5303}}}"#;
    let a = ScrollInfo::parse(pane_shell, Some(proc_shell)).unwrap();
    assert!(a.max_offset_from_bottom == 363 && a.offset_from_bottom == 12 && a.viewport_rows == 40, "scrollInfo: offsets");
    assert!(a.kind() == ScrollKind::History && a.shell_is_foreground == Some(true) && a.foreground_name.as_deref() == Some("zsh"), "scrollInfo: shell with history -> .history");
    let b = ScrollInfo::parse(pane_flat, Some(proc_less)).unwrap();
    assert!(b.kind() == ScrollKind::FullScreenProgram && b.foreground_name.as_deref() == Some("less"), "scrollInfo: no history + other program -> .fullScreenProgram");
    let c = ScrollInfo::parse(pane_flat, Some(proc_shell)).unwrap();
    assert!(c.kind() == ScrollKind::None, "scrollInfo: fresh shell -> .none");
    let d = ScrollInfo::parse(pane_flat, None).unwrap();
    assert!(d.kind() == ScrollKind::None && d.shell_is_foreground.is_none(), "scrollInfo: unknown process -> .none");
    let read_ok = br#"{"id":"x","result":{"type":"pane_read","read":{"pane_id":"w4:p1","source":"recent","format":"ansi","text":"a\r\nb\r\n","revision":3,"truncated":true}}}"#;
    let sb = Scrollback::parse(read_ok).unwrap();
    assert!(sb.text == "a\r\nb\r\n" && sb.truncated && sb.line_count == 2 && sb.revision == 3, "scrollback: parse");
    assert!(ScrollInfo::parse(br#"{"result":{}}"#, None).is_err() && Scrollback::parse(br#"{"result":{}}"#).is_err());
}

// Store timing against the fake herdr socket: one event reaches the store fast, a restart is picked up within ~1 s.
#[test]
fn store_timing() {
    let dir = temp_dir("s");
    let fake = FakeHerdr::new(dir.join("s.sock"));
    fake.start();
    let sess = HerdrSession { name: Some("fake".into()), socket_path: fake.path.to_string_lossy().into_owned() };
    let st = HerdrStore::new(&sess);
    st.start();
    let wait_ms = |sec: f64, ok: &dyn Fn() -> bool| -> Option<f64> {
        let t = Instant::now();
        while t.elapsed().as_secs_f64() < sec {
            if ok() {
                return Some(t.elapsed().as_secs_f64() * 1000.0);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    };
    let label = || st.workspaces().first().map(|w| w.label.clone());
    wait_ms(3.0, &|| label().as_deref() == Some("v0"));
    std::thread::sleep(Duration::from_millis(300));
    fake.set_label("e1");
    fake.push_event();
    let lat = wait_ms(2.0, &|| label().as_deref() == Some("e1"));
    assert!(lat.unwrap_or(9999.0) < 100.0, "store: event -> workspaces < 100 ms ({} ms)", lat.unwrap_or(-1.0) as i64);
    fake.kill();
    std::thread::sleep(Duration::from_millis(2500));
    fake.set_label("r1");
    fake.start();
    let back = wait_ms(3.0, &|| st.connected() && label().as_deref() == Some("r1"));
    assert!(back.unwrap_or(9999.0) < 1500.0, "store: back within 1.5 s after a 2.5 s socket loss ({} ms)", back.unwrap_or(-1.0) as i64);
    st.stop();
    fake.kill();
}

#[test]
fn store_preview_and_pane_lookup() {
    let st = HerdrStore::preview();
    assert!(st.connected() && st.workspaces().len() == 3);
    assert!(HerdrStore::preview_dense().workspaces().len() == 6);
    let quiet = HerdrStore::preview_quiet();
    assert!(quiet.workspaces().iter().flat_map(|w| &w.tabs).all(|t| t.status != herdr_core::AgentStatus::Blocked));
    let ws = st.workspaces();
    assert_eq!(HerdrStore::<herdr_core::UnixConnector>::session_id_for_pane("none", &ws), None);
}

// Store log tracking: ports of checksSnapshot / checksLoadState store assertions.
#[test]
fn store_log_state() {
    let root = temp_dir("storelog");
    let sess = HerdrSession { name: Some("x".into()), socket_path: root.join("nonexistent.sock").to_string_lossy().into_owned() };
    let st = HerdrStore::new(&sess);
    st.set_projects_dir(root.clone());
    assert!(!st.load_earlier_pane("nope", 2_000_000), "loadEarlier(paneId:) false for unknown pane");
    assert!(st.conversation_state(None) == herdr_core::session_log::ConversationLoadState::Missing, "store: nil session id is .missing");
    assert!(
        st.conversation_state(Some("no-such-session-abc")) == herdr_core::session_log::ConversationLoadState::Loading,
        "store: before the first read it is .loading"
    );
    st.stop();
}

// stop() between spawn and storing the child must still kill the child.
#[test]
fn stop_during_spawn_kills_child() {
    let dir = temp_dir("sk");
    let pidfile = dir.join("pid.txt");
    let script = dir.join("sleeper.sh");
    std::fs::write(&script, format!("#!/bin/sh\necho $$ > \"{}\"\nsleep 30\n", pidfile.display())).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (ts, rx) = TerminalSession::new("term_k", &session(), TerminalMode::Observe, None, None, Some(vec![script.to_string_lossy().into_owned()]));
    let ts = Arc::new(ts);
    let _ev = collect(rx);
    let weak = Arc::downgrade(&ts);
    let pf = pidfile.clone();
    ts.set_after_spawn_hook(move || {
        // wait until the script has written its pid, then stop while the child is not stored yet
        let t = Instant::now();
        while !pf.exists() && t.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(50));
        if let Some(s) = weak.upgrade() {
            s.stop();
        }
    });
    ts.start();
    assert!(until(5.0, || std::fs::read_to_string(&pidfile).map_or(false, |s| !s.trim().is_empty())), "child wrote its pid");
    let pid = std::fs::read_to_string(&pidfile).unwrap().trim().to_string();
    let alive = |pid: &str| std::process::Command::new("kill").args(["-0", pid]).stderr(std::process::Stdio::null()).status().map_or(false, |s| s.success());
    assert!(until(3.0, || !alive(&pid)), "child {pid} is killed after stop() raced the spawn");
}

#[test]
fn store_start_after_stop_restarts() {
    let dir = temp_dir("rs");
    let fake = FakeHerdr::new(dir.join("s.sock"));
    fake.start();
    let sess = HerdrSession { name: Some("fake".into()), socket_path: fake.path.to_string_lossy().into_owned() };
    let st = HerdrStore::new(&sess);
    st.start();
    assert!(until(3.0, || st.connected()), "connected after start");
    st.stop();
    st.start(); // right after stop: the old loop is still winding down
    assert!(until(3.0, || st.connected() && !st.workspaces().is_empty()), "restart connects again");
    // and a second start() while running changes nothing
    st.start();
    std::thread::sleep(Duration::from_millis(400)); // the old loop has noticed its cancel by now
    assert!(st.connected(), "still connected after the old loop ended");
    st.stop();
    fake.kill();
}

#[test]
fn dropping_a_store_ends_log_threads() {
    let root = temp_dir("dr");
    std::fs::create_dir_all(root.join("p")).unwrap();
    std::fs::write(root.join("p").join("drop1.jsonl"), "").unwrap();
    let sess = HerdrSession { name: Some("x".into()), socket_path: root.join("none.sock").to_string_lossy().into_owned() };
    let st = HerdrStore::new(&sess);
    st.set_projects_dir(root.clone());
    st.conversation_state(Some("drop1"));
    let (readers, log) = st.log_probe("drop1");
    let log = log.expect("tailing started");
    assert!(until(2.0, || log.active_tailers() == 1 && readers.load(std::sync::atomic::Ordering::SeqCst) == 1), "tailer and reader are running");
    drop(st);
    assert!(
        until(3.0, || log.active_tailers() == 0 && readers.load(std::sync::atomic::Ordering::SeqCst) == 0),
        "tailer ({}) and reader ({}) threads end after the store is dropped",
        log.active_tailers(),
        readers.load(std::sync::atomic::Ordering::SeqCst)
    );
}

// The revision is a cheap change counter: None until tailing starts, then it moves only when a new snapshot is consumed.
#[test]
fn store_conversation_revision_moves_only_on_new_snapshots() {
    let root = temp_dir("storerev");
    let proj = root.join("p");
    std::fs::create_dir_all(&proj).unwrap();
    let log = proj.join("rev-session.jsonl");
    std::fs::write(&log, "").unwrap();
    let sess = HerdrSession { name: Some("x".into()), socket_path: root.join("nonexistent.sock").to_string_lossy().into_owned() };
    let st = HerdrStore::new(&sess);
    st.set_projects_dir(root.clone());
    assert!(st.conversation_revision(Some("rev-session")).is_none(), "revision: none before tailing");
    assert!(st.conversation_revision(None).is_none(), "revision: none for no session");
    let _ = st.conversation(Some("rev-session"));
    assert!(until(3.0, || st.conversation_revision(Some("rev-session")).is_some_and(|r| r > 0)), "revision: first snapshot counted");
    // Loading, then Ready: both are new snapshots. Under load Ready can come late, so wait for it.
    assert!(until(5.0, || st.conversation_state(Some("rev-session")) == ConversationLoadState::Ready), "revision: log loaded");
    std::thread::sleep(Duration::from_millis(300));
    let r1 = st.conversation_revision(Some("rev-session")).unwrap();
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(st.conversation_revision(Some("rev-session")), Some(r1), "revision: stable while the log is idle");
    let line = r#"{"type":"user","uuid":"u1","timestamp":"2026-10-10T10:00:00Z","message":{"role":"user","content":"hello"}}"#;
    std::fs::write(&log, format!("{line}\n")).unwrap();
    assert!(until(3.0, || st.conversation_revision(Some("rev-session")).is_some_and(|r| r > r1)), "revision: moves on append");
    st.stop();
}
