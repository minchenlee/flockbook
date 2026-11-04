//! Ports of the Swift checks on locating logs (moved from rust/herdrkit; synthetic fixtures only).

use super::*;
use std::fs;

/// A path as text, with `/` on Windows too (both separators name the same path there).
fn s(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[test]
fn path_helpers() {
    assert_eq!(standardize_path("/w/a/"), "/w/a");
    assert_eq!(standardize_path("/a/./b//c/../d"), "/a/b/d");
    assert_eq!(last_path_component("/a/b.jsonl"), "b.jsonl");
    assert_eq!(last_path_component("/a/b/"), "b");
    assert_eq!(deleting_last_path_component("/a/b/c"), "/a/b");
    assert_eq!(deleting_last_path_component("/a"), "/");
}

// ChecksGrok: GROK_HOME or ~/.grok.
#[test]
fn default_dirs() {
    let g: HashMap<String, String> = [("GROK_HOME".to_string(), "/g".to_string())].into_iter().collect();
    let h = home_dir();
    let default_dir = |a: &str, e: &HashMap<String, String>| default_dir_in(a, e, Path::new(&h)).to_string_lossy().into_owned();
    assert_eq!(default_dir("grok", &g), "/g/sessions");
    assert_eq!(default_dir("grok", &HashMap::new()), home_dir() + "/.grok/sessions");
    let empty: HashMap<String, String> = [("GROK_HOME".to_string(), String::new())].into_iter().collect();
    assert_eq!(default_dir("grok", &empty), home_dir() + "/.grok/sessions");
    assert_eq!(default_dir("codex", &g), home_dir() + "/.codex/sessions");
    assert_eq!(default_dir("cursor", &g), home_dir() + "/.cursor/projects");
    assert_eq!(default_dir("claude", &g), home_dir() + "/.claude/projects");
    assert_eq!(default_dir("pi", &g), home_dir() + "/.claude/projects");
}

// ChecksCore checksLocateTail (locate part).
#[test]
fn locate_claude_by_id() {
    let t = tempfile::tempdir().unwrap();
    let dir = s(t.path());
    fs::create_dir_all(format!("{dir}/proj")).unwrap();
    let path = format!("{dir}/proj/abc.jsonl");
    fs::write(&path, "{}\n").unwrap();
    assert_eq!(locate_claude("abc", &dir).map(|p| s(&p)), Some(path.clone()), "locate");
    assert_eq!(locate_claude("../abc", &dir), None, "locate rejects ..");
    assert_eq!(locate("claude", "abc", &dir).map(|p| s(&p)), Some(path));
}

// ChecksCore: codex locate by id, by path, rejects "..".
#[test]
fn locate_codex_rollouts() {
    let t = tempfile::tempdir().unwrap();
    let root = s(t.path());
    let cx = format!("{root}/2026/10/09");
    fs::create_dir_all(&cx).unwrap();
    let f = format!("{cx}/rollout-2026-10-09T00-00-00-cx-1.jsonl");
    fs::write(&f, "x\n").unwrap();
    assert!(locate_codex("cx-1", &root).unwrap().file_name().unwrap().to_string_lossy().ends_with("-cx-1.jsonl"), "codex locate by id");
    assert_eq!(locate_codex(&f, "/nonexistent").map(|p| s(&p)), Some(f.clone()), "codex locate by path");
    assert_eq!(locate_codex("../cx-1", &root), None, "codex locate rejects ..");
    // Newest day first.
    let newer = format!("{root}/2026/10/10");
    fs::create_dir_all(&newer).unwrap();
    let g = format!("{newer}/rollout-2026-10-10T00-00-00-cx-1.jsonl");
    fs::write(&g, "x\n").unwrap();
    assert_eq!(locate("codex", "cx-1", &root).map(|p| s(&p)), Some(g));
}

// ChecksAgents checksPi: path sessions resolve directly.
#[test]
fn locate_path_sessions() {
    let t = tempfile::tempdir().unwrap();
    let p = format!("{}/pi-x.jsonl", s(t.path()));
    fs::write(&p, "{}").unwrap();
    assert_eq!(locate("pi", &p, "/nonexistent").map(|x| s(&x)), Some(p.clone()));
    assert_eq!(locate("pi", &(p.clone() + ".x"), "/nonexistent"), None);
    assert_eq!(locate("claude", "/etc/passwd", "/nonexistent"), None);
    assert_eq!(locate("pi", &format!("{}/../x/pi-x.jsonl", s(t.path())), "/nonexistent"), None);
}

// ChecksAgents checksCursor: locate by id.
#[test]
fn locate_cursor_by_id() {
    let t = tempfile::tempdir().unwrap();
    let root = s(t.path());
    let sid = "11111111-2222-3333-4444-555555555555";
    fs::create_dir_all(format!("{root}/proj/agent-transcripts/{sid}")).unwrap();
    fs::write(format!("{root}/proj/agent-transcripts/{sid}/{sid}.jsonl"), "").unwrap();
    assert_eq!(locate("cursor", sid, &root).unwrap().file_name().unwrap().to_string_lossy(), format!("{sid}.jsonl"));
    assert_eq!(locate("cursor", "../x", &root), None);
}

// ChecksGrok: locate updates.jsonl by id.
#[test]
fn locate_grok_by_id() {
    let t = tempfile::tempdir().unwrap();
    let root = s(t.path());
    let sid = "00000000-0000-7000-8000-000000000001";
    fs::create_dir_all(format!("{root}/slug-0a1b2c/{sid}")).unwrap();
    fs::create_dir_all(format!("{root}/%2Fw%2Fa/other")).unwrap();
    fs::write(format!("{root}/slug-0a1b2c/{sid}/updates.jsonl"), "{}\n").unwrap();
    fs::write(format!("{root}/slug-0a1b2c/{sid}/chat_history.jsonl"), "").unwrap();
    assert_eq!(locate("grok", sid, &root).map(|p| s(&p)), Some(format!("{root}/slug-0a1b2c/{sid}/updates.jsonl")));
    assert_eq!(locate("grok", &format!("../{sid}"), &root), None);
    assert_eq!(locate("grok", "other", &root), None);
}

// ChecksContinuedIn: locating follows the chain (synthetic logs only).
#[test]
fn continued_in_chain() {
    let t = tempfile::tempdir().unwrap();
    let projects = format!("{}/projects", s(t.path()));
    let dir = format!("{projects}/-w-proj");
    fs::create_dir_all(&dir).unwrap();
    let mate = r#"{"type":"user","uuid":"m1","sessionId":"s","teamName":"team-1","agentName":"helper","message":{"role":"user","content":"teammate start"}}"#;
    let lead = r#"{"type":"user","uuid":"l1","sessionId":"s","message":{"role":"user","content":"lead start"}}"#;
    let next = |from: &str, to: &str| {
        format!(r#"{{"type":"continued-in","timestamp":"2026-10-30T06:54:55.989Z","sessionId":"{from}","continuedInSessionId":"{to}"}}"#)
    };
    let log = |id: &str, ls: &[&str]| {
        let p = format!("{dir}/{id}.jsonl");
        fs::write(&p, ls.join("\n") + "\n").unwrap();
        p
    };
    log("old1", &[mate, &next("old1", "mid1")]);
    log("mid1", &[lead, &next("mid1", "new1")]);
    let new1 = log("new1", &[lead]);
    assert_eq!(locate_claude("old1", &projects).map(|p| s(&p)), Some(new1.clone()), "locate follows the chain to the newest log");
    assert_eq!(locate("claude", "old1", &projects).map(|p| s(&p)), Some(new1.clone()), "locate by agent follows it too");
    let gone = log("gone1", &[lead, &next("gone1", "missing1")]);
    assert_eq!(locate_claude("gone1", &projects).map(|p| s(&p)), Some(gone), "a missing next log keeps the old one");
    log("loopa", &[lead, &next("loopa", "loopb")]);
    log("loopb", &[lead, &next("loopb", "loopa")]);
    assert!(locate_claude("loopa", &projects).is_some(), "a loop ends");
    let later = log("later1", &[lead, &next("later1", "new1"), lead]);
    assert_eq!(locate_claude("later1", &projects).map(|p| s(&p)), Some(later.clone()), "only a last line counts");
    assert_eq!(continued_in(Path::new(&later)), None);
    assert_eq!(continued_in(Path::new(&format!("{dir}/old1.jsonl"))), Some("mid1".into()));
    assert_eq!(continued(Path::new(&format!("{dir}/old1.jsonl")), Some(Path::new(&projects))), PathBuf::from(&new1));
    assert_eq!(continued_in(Path::new("/nonexistent/x.jsonl")), None);
}
