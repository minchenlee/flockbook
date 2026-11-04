//! Ports of the Swift checks that exercise `SessionLog`: locate, tail, load state, loadEarlier, file watcher.
mod common;

use common::*;
use herdr_core::session_log::{self, ConversationLoadState as L, SessionLog};
use herdr_core::{ConversationItem, ItemKind};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const T: Duration = Duration::from_secs(5);

fn append(path: &Path, text: &str) {
    let mut f = fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn user_line(n: usize, pad: usize) -> String {
    format!(r#"{{"type":"user","uuid":"w{n}","message":{{"content":"msg {n} {}"}}}}"#, "x".repeat(pad)) + "\n"
}

/// Collects every list an items stream yields, on a reader thread.
struct Box_ {
    all: Arc<Mutex<Vec<Vec<ConversationItem>>>>,
    _stream: Arc<session_log::LogStream<Vec<ConversationItem>>>,
}
impl Box_ {
    fn new(log: &SessionLog, root: &Path, poll: Duration) -> Self {
        let stream = Arc::new(log.items(Some(root), poll));
        let all = Arc::new(Mutex::new(Vec::new()));
        let (s2, a2) = (stream.clone(), all.clone());
        std::thread::spawn(move || {
            while let Some(x) = s2.recv() {
                a2.lock().unwrap().push(x);
            }
        });
        Box_ { all, _stream: stream }
    }
    fn wait(&self, secs: f64, ok: impl Fn(&[Vec<ConversationItem>]) -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed().as_secs_f64() < secs {
            if ok(&self.all.lock().unwrap()) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        ok(&self.all.lock().unwrap())
    }
    fn last_count(&self) -> Option<usize> {
        self.all.lock().unwrap().last().map(Vec::len)
    }
}

#[test]
fn locate_and_tail() {
    let lines = fixture_lines("session.jsonl");
    let dir = temp_dir("lt");
    fs::create_dir_all(dir.join("proj")).unwrap();
    let path = dir.join("proj").join("abc.jsonl");
    fs::write(&path, lines[0].clone() + "\n").unwrap();
    assert!(session_log::locate_claude("abc", &dir) == Some(path.clone()), "locate");
    assert!(session_log::locate_claude("../abc", &dir).is_none(), "locate rejects ..");

    // tail(path:line:)
    let s = SessionLog::new("abc", "claude").items(Some(&dir.join("proj").join("..")), Duration::from_millis(30));
    let c0 = s.recv().map(|v| v.len());
    let c1 = s.recv().map(|v| v.len());
    append(&path, &(lines[4].clone() + "\n"));
    let c2 = s.recv_timeout(T).ok().map(|v| v.len());
    let got = [c0, c1, c2];
    assert!(got == [Some(0), Some(1), Some(2)], "tail yields growing lists {got:?}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn codex_locate() {
    let root = temp_dir("cx");
    let day = root.join("2026").join("10").join("09");
    fs::create_dir_all(&day).unwrap();
    let file = day.join("rollout-2026-10-09T00-00-00-cx-1.jsonl");
    fs::write(&file, "x\n").unwrap();
    let f = session_log::locate_codex("cx-1", &root);
    assert!(f.as_ref().and_then(|p| p.file_name()).map_or(false, |n| n.to_string_lossy().ends_with("-cx-1.jsonl")), "codex locate by id");
    assert!(session_log::locate_codex(file.to_str().unwrap(), &root) == Some(file.clone()), "codex locate by path");
    assert!(session_log::locate_codex("../cx-1", &root).is_none(), "codex locate rejects ..");
    // the generic entry point routes by agent
    assert_eq!(session_log::locate("codex", "cx-1", &root), Some(file));
    fs::remove_dir_all(&root).ok();
}

#[test]
fn pi_locate() {
    let pl = fixture_lines("pi.jsonl");
    let p = std::env::temp_dir().join(format!("pi-{}.jsonl", std::process::id()));
    fs::write(&p, pl.join("\n")).unwrap();
    let ps = p.to_str().unwrap();
    let nonexistent = Path::new("/nonexistent");
    assert!(
        session_log::locate("pi", ps, nonexistent) == Some(p.clone())
            && session_log::locate("pi", &format!("{ps}.x"), nonexistent).is_none()
            && session_log::locate("claude", "/etc/passwd", nonexistent).is_none(),
        "pi: path sessions resolve directly"
    );
    fs::remove_file(&p).ok();
}

#[test]
fn cursor_locate() {
    let root = temp_dir("cu");
    let sid = "11111111-2222-3333-4444-555555555555";
    let d = root.join("proj").join("agent-transcripts").join(sid);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join(format!("{sid}.jsonl")), "").unwrap();
    let f = session_log::locate("cursor", sid, &root);
    assert!(
        f.as_ref().and_then(|p| p.file_name()).map_or(false, |n| n.to_string_lossy() == format!("{sid}.jsonl"))
            && session_log::locate("cursor", "../x", &root).is_none(),
        "cursor: locate by id"
    );
    fs::remove_dir_all(&root).ok();
}

/// Changed in the merge with rust/herdrkit: the Swift check `ChecksGrok` (default dirs) is the oracle. Swift reads
/// only `GROK_HOME`; `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and the `PI_CODING_AGENT_*` variables are not read, and pi
/// (whose sessions are paths) falls through to Claude's folder.
#[test]
fn default_dirs_follow_swift() {
    use std::collections::HashMap;
    let home = Path::new("/home/u");
    let env = |kv: &[(&str, &str)]| -> HashMap<String, String> { kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() };
    let dir = |a: &str, e: &HashMap<String, String>| session_log::default_dir_in(a, e, home);
    let none = env(&[]);
    assert_eq!(dir("claude", &none), home.join(".claude").join("projects"));
    assert_eq!(dir("codex", &none), home.join(".codex").join("sessions"));
    assert_eq!(dir("pi", &none), home.join(".claude").join("projects"));
    assert_eq!(dir("cursor", &none), home.join(".cursor").join("projects"));
    assert_eq!(dir("grok", &none), home.join(".grok").join("sessions"));
    let e = env(&[("CLAUDE_CONFIG_DIR", "/c/claude"), ("CODEX_HOME", "/c/codex"), ("PI_CODING_AGENT_DIR", "/c/pi"), ("GROK_HOME", "/c/grok")]);
    assert_eq!(dir("claude", &e), home.join(".claude").join("projects"));
    assert_eq!(dir("codex", &e), home.join(".codex").join("sessions"));
    assert_eq!(dir("grok", &e), Path::new("/c/grok").join("sessions"));
    // an empty value counts as unset
    assert_eq!(dir("grok", &env(&[("GROK_HOME", "")])), home.join(".grok").join("sessions"));
}

#[test]
fn load_state() {
    let root = temp_dir("ls");
    fs::create_dir_all(root.join("proj")).unwrap();
    let s = SessionLog::new("ls1", "claude").snapshots(Some(&root), Duration::from_millis(30), None);
    let s0 = s.recv().unwrap();
    let s1 = s.recv().unwrap();
    assert!(s0.load == L::Loading && s0.items.is_empty(), "load: first yield is .loading");
    assert!(s1.load == L::Missing, "load: no file -> .missing ({:?})", s1.load);
    fs::write(root.join("proj").join("ls1.jsonl"), "").unwrap();
    let s2 = s.recv_timeout(T).unwrap();
    assert!(s2.load == L::Ready && s2.items.is_empty(), "load: empty file -> .ready, no items");
    append(&root.join("proj").join("ls1.jsonl"), &(r#"{"type":"user","uuid":"u1","message":{"role":"user","content":"hi"}}"#.to_string() + "\n"));
    let s3 = s.recv_timeout(T).unwrap();
    assert!(s3.load == L::Ready && s3.items.len() == 1, "load: written file -> .ready with items");
    drop(s);
    fs::remove_dir_all(&root).ok();
}

#[test]
fn load_earlier() {
    let root = temp_dir("le");
    fs::create_dir_all(root.join("p")).unwrap();
    let mut ls: Vec<String> = Vec::new();
    for i in 0..100 {
        ls.push(format!(r#"{{"type":"user","uuid":"u{i}","message":{{"content":"question {i} ......................................"}}}}"#));
        ls.push(format!(r#"{{"type":"assistant","uuid":"a{i}","message":{{"content":[{{"type":"tool_use","id":"t{i}","name":"Bash","input":{{"command":"echo {i}"}}}}]}}}}"#));
        ls.push(format!(r#"{{"type":"user","uuid":"r{i}","message":{{"content":[{{"type":"tool_result","tool_use_id":"t{i}","content":"out{i}"}}]}}}}"#));
    }
    fs::write(root.join("p").join("le1.jsonl"), ls.join("\n") + "\n").unwrap();
    let log = SessionLog::new("le1", "claude");
    let s = log.snapshots(Some(&root), Duration::from_millis(20), Some(3000));
    s.recv().unwrap();
    let first = s.recv_timeout(T).unwrap().items;
    let ids0: Vec<String> = first.iter().map(|i| i.id.clone()).collect();
    assert!(first.len() > 2 && first.len() < 150, "loadEarlier: initial tail is partial ({} items)", first.len());
    let mut more = log.load_earlier(3000);
    let mut cur = s.recv_timeout(T).unwrap().items;
    assert!(more && cur.len() > first.len(), "loadEarlier: older items arrive, more remain");
    let ids: Vec<String> = cur.iter().map(|i| i.id.clone()).collect();
    let uniq: std::collections::HashSet<&String> = ids.iter().collect();
    assert!(ids[ids.len() - ids0.len()..] == ids0[..] && uniq.len() == cur.len(), "loadEarlier: ids stable and unique");
    let mut guard_n = 0;
    while more && guard_n < 50 {
        more = log.load_earlier(3000);
        guard_n += 1;
    }
    let mut drain = 0;
    loop {
        cur = s.recv_timeout(T).unwrap().items;
        drain += 1;
        if cur.len() >= 200 || drain >= 60 {
            break;
        }
    }
    let starts_q0 = cur.first().map_or(false, |i| matches!(&i.kind, ItemKind::UserText(t) if t.starts_with("question 0 ")));
    assert!(!more && cur.len() == 200 && starts_q0, "loadEarlier: reaches file start {}", cur.len());
    let calls = chip_calls(&cur);
    assert!(calls.len() == 100 && calls.iter().all(|c| c.output.is_some()), "loadEarlier: results matched across segments");
    assert!(!log.load_earlier(2_000_000), "loadEarlier: false when nothing older");
    drop(s);
    fs::remove_dir_all(&root).ok();
}

#[test]
fn file_watcher() {
    let root = temp_dir("fw");
    fs::create_dir_all(root.join("p")).unwrap();
    let path = root.join("p").join("w1.jsonl");
    fs::write(&path, user_line(1, 0) + &user_line(2, 0)).unwrap();
    // The safety-net timer is 30 s, so only the watcher can be fast.
    let b = Box_::new(&SessionLog::new("w1", "claude"), &root, Duration::from_secs(30));
    assert!(b.wait(3.0, |a| a.last().map(Vec::len) == Some(2)), "watch: first load");
    append(&path, &user_line(3, 0));
    assert!(b.wait(2.0, |a| a.last().map(Vec::len) == Some(3)), "watch: append seen within 2 s without polling");
    fs::remove_file(&path).unwrap();
    fs::write(&path, user_line(7, 0)).unwrap();
    assert!(b.wait(3.0, |a| a.last().map(Vec::len) == Some(1)), "watch: replaced file is reopened");
    append(&path, &user_line(8, 0));
    assert!(b.wait(2.0, |a| a.last().map(Vec::len) == Some(2)), "watch: append after replace seen");
    drop(b);

    // staged load: 256 KB first, then the rest of the 2 MB tail; ids stay put
    let big = root.join("p").join("w2.jsonl");
    let text: String = (0..1500).map(|n| user_line(n, 1000)).collect();
    fs::write(&big, text).unwrap();
    let b2 = Box_::new(&SessionLog::new("w2", "claude"), &root, Duration::from_secs(30));
    assert!(b2.wait(5.0, |a| a.last().map(Vec::len) == Some(1500)), "stage: backfill reaches the full file");
    let all = b2.all.lock().unwrap().clone();
    let counts: Vec<usize> = all.iter().map(Vec::len).filter(|&n| n > 0).collect();
    let mut sorted = counts.clone();
    sorted.sort();
    assert!(counts.len() >= 2 && counts[0] < 400 && counts == sorted, "stage: short view first ({} items), then longer", counts.first().copied().unwrap_or(0));
    let first_ids: Vec<String> = all.iter().find(|x| !x.is_empty()).unwrap().iter().map(|i| i.id.clone()).collect();
    let last_ids: Vec<String> = all.last().unwrap().iter().map(|i| i.id.clone()).collect();
    assert!(last_ids[last_ids.len() - first_ids.len()..] == first_ids[..], "stage: ids stable across the backfill");
    drop(b2);

    // depth switching: down drops the front items, up loads them again; short mode never backfills
    let log2 = SessionLog::new("w2", "claude");
    let b3 = Box_::new(&log2, &root, Duration::from_secs(30));
    assert!(b3.wait(5.0, |a| a.last().map(Vec::len) == Some(1500)), "depth: full by default");
    log2.set_full(false);
    assert!(b3.wait(3.0, |a| a.last().map_or(false, |l| !l.is_empty() && l.len() < 400)), "depth: setFull(false) trims to the short tail ({})", b3.last_count().unwrap_or(0));
    let newest = b3.all.lock().unwrap().last().and_then(|l| l.last().map(|i| i.id.clone()));
    assert!(newest.as_ref() == last_ids.last(), "depth: newest item kept");
    log2.set_full(true);
    assert!(b3.wait(3.0, |a| a.last().map(Vec::len) == Some(1500)), "depth: setFull(true) restores the 2 MB tail");
    drop(b3);
    let short = SessionLog::new("w2", "claude");
    short.set_full(false);
    let b4 = Box_::new(&short, &root, Duration::from_secs(30));
    b4.wait(1.5, |_| false);
    let n = b4.last_count().unwrap_or(0);
    assert!(n > 0 && n < 400, "depth: short mode stays short ({n})");
    drop(b4);
    fs::remove_dir_all(&root).ok();
}

fn user_texts(items: &[ConversationItem]) -> Vec<String> {
    items.iter().filter_map(|i| if let ItemKind::UserText(t) = &i.kind { Some(t.clone()) } else { None }).collect()
}

fn user_json(uuid: &str, text: &str) -> String {
    format!(r#"{{"type":"user","uuid":"{uuid}","message":{{"role":"user","content":"{text}"}}}}"#) + "\n"
}

#[test]
fn truncate_to_zero_clears_items() {
    let root = temp_dir("tz");
    fs::create_dir_all(root.join("p")).unwrap();
    let path = root.join("p").join("tz1.jsonl");
    fs::write(&path, user_json("a", "one") + &user_json("b", "two")).unwrap();
    let b = Box_::new(&SessionLog::new("tz1", "claude"), &root, Duration::from_millis(30));
    assert!(b.wait(5.0, |a| a.last().map_or(false, |l| l.len() == 2)), "two items first");
    fs::OpenOptions::new().write(true).truncate(true).open(&path).unwrap(); // exactly 0 bytes
    assert!(b.wait(5.0, |a| a.last().map_or(false, |l| l.is_empty()) && a.len() > 2), "empty list sent after truncate to 0");
    append(&path, &user_json("c", "three"));
    assert!(b.wait(5.0, |a| a.last().map(|l| user_texts(l)) == Some(vec!["three".to_string()])), "new content after reset");
    fs::remove_dir_all(&root).ok();
}

#[test]
fn rewrite_in_place_with_same_size_is_noticed() {
    let root = temp_dir("rw");
    fs::create_dir_all(root.join("p")).unwrap();
    let path = root.join("p").join("rw1.jsonl");
    let first = user_json("a", "aaaa");
    let second = user_json("b", "bbbb"); // same byte length, different first bytes
    assert_eq!(first.len(), second.len());
    fs::write(&path, &first).unwrap();
    let b = Box_::new(&SessionLog::new("rw1", "claude"), &root, Duration::from_millis(30));
    assert!(b.wait(5.0, |a| a.last().map(|l| user_texts(l)) == Some(vec!["aaaa".to_string()])), "first content");
    // overwrite without truncating: same inode, same size
    let mut f = fs::OpenOptions::new().write(true).open(&path).unwrap();
    f.write_all(second.as_bytes()).unwrap();
    drop(f);
    assert!(b.wait(5.0, |a| a.last().map(|l| user_texts(l)) == Some(vec!["bbbb".to_string()])), "rewritten file is re-read");
    fs::remove_dir_all(&root).ok();
}

#[test]
fn invalid_utf8_lines_are_decoded_lossily() {
    let root = temp_dir("u8");
    fs::create_dir_all(root.join("p")).unwrap();
    let path = root.join("p").join("u81.jsonl");
    let mut bytes = user_json("a", "before").into_bytes();
    let mut bad = br#"{"type":"user","uuid":"b","message":{"role":"user","content":"bad "#.to_vec();
    bad.push(0xff);
    bad.extend_from_slice(br#" byte"}}"#);
    bad.push(b'\n');
    bytes.extend(bad);
    bytes.extend(user_json("c", "after").into_bytes());
    fs::write(&path, &bytes).unwrap();
    let b = Box_::new(&SessionLog::new("u81", "claude"), &root, Duration::from_millis(30));
    assert!(b.wait(5.0, |a| a.last().map_or(false, |l| l.len() == 3)), "all three lines are shown");
    let texts = user_texts(b.all.lock().unwrap().last().unwrap());
    assert_eq!(texts[0], "before");
    assert_eq!(texts[1], "bad \u{fffd} byte");
    assert_eq!(texts[2], "after");
    fs::remove_dir_all(&root).ok();
}
