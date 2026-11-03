//! Finding an agent's JSONL session log (Swift: the static parts of `SessionLog.swift`): where each agent keeps its
//! logs, the lookup by session id, and Claude Code's `continued-in` chain. Not the live tailer.
//!
//! Paths are built as strings with `/`, as Swift builds them, so they compare equal to the Swift results.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The home folder (Swift: `NSHomeDirectory()`): `HOME` on unix, `USERPROFILE` on Windows. Empty when unset.
pub fn home_dir() -> String {
    #[cfg(windows)]
    let key = "USERPROFILE";
    #[cfg(not(windows))]
    let key = "HOME";
    std::env::var(key).unwrap_or_default()
}

/// `NSString.lastPathComponent`: the last component, trailing slashes ignored; "/" for "/".
pub(crate) fn last_path_component(p: &str) -> &str {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.is_empty() { "" } else { "/" };
    }
    match t.rfind('/') {
        Some(i) => &t[i + 1..],
        None => t,
    }
}

/// `NSString.deletingLastPathComponent`.
pub(crate) fn deleting_last_path_component(p: &str) -> String {
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.starts_with('/') { "/".into() } else { String::new() };
    }
    match t.rfind('/') {
        Some(i) => {
            let head = t[..i].trim_end_matches('/');
            if head.is_empty() { "/".into() } else { head.to_string() }
        }
        None => String::new(),
    }
}

/// `NSString.standardizingPath`, as far as it is a matter of the string: `~` expanded, `//` and `/./` reduced, `..`
/// resolved in absolute paths, no trailing slash. On macOS an initial `/private` is dropped when the rest exists
/// (as Foundation does). Symlinks are not resolved (Foundation resolves them only around a `..`).
pub fn standardize_path(p: &str) -> String {
    // Windows: both separators name the same path; `/` is kept (drive paths stay `C:/...`).
    let mut s = if cfg!(windows) { p.replace('\\', "/") } else { p.to_string() };
    if s == "~" || s.starts_with("~/") {
        s = home_dir() + &s[1..];
    }
    let abs = is_abs_path(&s);
    let mut parts: Vec<&str> = vec![];
    for c in s.split('/') {
        match c {
            "" | "." => {}
            ".." if abs => {
                parts.pop();
            }
            _ => parts.push(c),
        }
    }
    let joined = parts.join("/");
    let out = if s.starts_with('/') { format!("/{joined}") } else { joined };
    #[cfg(target_os = "macos")]
    {
        if let Some(rest) = out.strip_prefix("/private/") {
            let cand = format!("/{rest}");
            if Path::new(&cand).exists() {
                return cand;
            }
        }
    }
    if out.is_empty() && !p.is_empty() {
        return p.to_string();
    }
    out
}

fn file_exists(p: &str) -> bool {
    Path::new(p).exists()
}

fn list_dir(p: &str) -> Vec<String> {
    match std::fs::read_dir(p) {
        Ok(rd) => rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).collect(),
        Err(_) => vec![],
    }
}

fn id_ok(session_id: &str) -> bool {
    !session_id.is_empty() && !session_id.contains('/') && !session_id.contains("..")
}

/// Where an agent keeps its logs (the search root for an id), with the process environment.
pub fn default_dir_env(agent: &str) -> String {
    let env: HashMap<String, String> = std::env::vars().collect();
    default_dir(agent, &env)
}

/// Where an agent keeps its logs. Grok Build: `$GROK_HOME/sessions`, else `~/.grok/sessions`; Codex
/// `~/.codex/sessions`; Cursor `~/.cursor/projects`; everything else `~/.claude/projects` (as Swift: no
/// `CODEX_HOME` / `CLAUDE_CONFIG_DIR` here). The home is always `home_dir()`, not `env`'s.
pub fn default_dir(agent: &str, env: &HashMap<String, String>) -> String {
    let home = home_dir();
    if agent == "grok" {
        let g = env.get("GROK_HOME").filter(|s| !s.is_empty()).cloned().unwrap_or_else(|| home + "/.grok");
        return g + "/sessions";
    }
    // TODO(windows): the agents' folders on Windows are not confirmed; the unix layout under %USERPROFILE% is assumed.
    home + match agent {
        "codex" => "/.codex/sessions",
        "cursor" => "/.cursor/projects",
        _ => "/.claude/projects",
    }
}

/// An absolute path: `/...`, or on Windows also `C:\\...` / `C:/...` (Swift only knows the first).
pub fn is_abs_path(s: &str) -> bool {
    s.starts_with('/') || Path::new(s).is_absolute()
}

/// The log file for a session: an absolute `.jsonl` path is used as is (any agent); otherwise it is looked up by id
/// in `dir` (`default_dir_env(agent)` normally).
pub fn locate(agent: &str, session_id: &str, dir: &str) -> Option<PathBuf> {
    // Windows: a drive path is a path too (herdr's `agent_session` paths there are not documented yet).
    if is_abs_path(session_id) {
        return if session_id.ends_with(".jsonl") && !session_id.contains("/../") && !(cfg!(windows) && session_id.contains("\\..\\")) && file_exists(session_id) {
            Some(PathBuf::from(session_id))
        } else {
            None
        };
    }
    match agent {
        "codex" => locate_codex(session_id, dir),
        "cursor" => locate_cursor(session_id, dir),
        "grok" => locate_grok(session_id, dir),
        _ => locate_claude(session_id, dir), // Claude Code (and, as before, any other id)
    }
}

/// Codex: `<sessions>/YYYY/MM/DD/rollout-<time>-<id>.jsonl`, newest day first. Accepts a session id or the file
/// path itself.
pub fn locate_codex(session_id: &str, sessions_dir: &str) -> Option<PathBuf> {
    if is_abs_path(session_id) {
        return if file_exists(session_id) && session_id.ends_with(".jsonl") { Some(PathBuf::from(session_id)) } else { None };
    }
    if !id_ok(session_id) {
        return None;
    }
    fn sub(p: &str) -> Vec<String> {
        let mut v = list_dir(p);
        v.sort_by(|a, b| b.cmp(a));
        v
    }
    let suffix = format!("-{session_id}.jsonl");
    for y in sub(sessions_dir) {
        for m in sub(&format!("{sessions_dir}/{y}")) {
            for d in sub(&format!("{sessions_dir}/{y}/{m}")) {
                let dir = format!("{sessions_dir}/{y}/{m}/{d}");
                if let Some(f) = sub(&dir).into_iter().find(|f| f.starts_with("rollout-") && f.ends_with(&suffix)) {
                    return Some(PathBuf::from(format!("{dir}/{f}")));
                }
            }
        }
    }
    None
}

/// Cursor: `<projects>/*/agent-transcripts/<id>/<id>.jsonl`.
pub fn locate_cursor(session_id: &str, projects_dir: &str) -> Option<PathBuf> {
    if !id_ok(session_id) {
        return None;
    }
    list_dir(projects_dir)
        .into_iter()
        .map(|d| format!("{projects_dir}/{d}/agent-transcripts/{session_id}/{session_id}.jsonl"))
        .find(|p| file_exists(p))
        .map(PathBuf::from)
}

/// Grok Build: `<sessions>/<url-encoded cwd, or a slug>/<id>/updates.jsonl` (not `chat_history.jsonl`).
pub fn locate_grok(session_id: &str, sessions_dir: &str) -> Option<PathBuf> {
    if !id_ok(session_id) {
        return None;
    }
    list_dir(sessions_dir)
        .into_iter()
        .map(|d| format!("{sessions_dir}/{d}/{session_id}/updates.jsonl"))
        .find(|p| file_exists(p))
        .map(PathBuf::from)
}

/// Claude Code: `<projects>/*/<id>.jsonl`, followed to the newest log of its conversation (see `continued`).
pub fn locate_claude(session_id: &str, projects_dir: &str) -> Option<PathBuf> {
    locate_one(session_id, projects_dir, None).map(|p| continued(&p, Some(projects_dir)))
}

fn locate_one(session_id: &str, projects_dir: &str, near: Option<&Path>) -> Option<PathBuf> {
    if !id_ok(session_id) {
        return None;
    }
    if let Some(near) = near {
        let p = near.parent().unwrap_or(Path::new("/")).join(format!("{session_id}.jsonl"));
        if p.exists() {
            return Some(p);
        }
    }
    let dirs = std::fs::read_dir(projects_dir).ok()?;
    for d in dirs.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()) {
        let p = format!("{projects_dir}/{d}/{session_id}.jsonl");
        if file_exists(&p) {
            return Some(PathBuf::from(p));
        }
    }
    None
}

/// Claude Code ends a log with a `continued-in` line when the conversation goes on in another session's log, while
/// herdr keeps the old id. The newest log of the chain; `log` itself when its last line is not one, or the next log
/// is not there. At most 16 steps; a loop ends. `projects_dir` None: `~/.claude/projects` (as Swift's default:
/// not `default_dir`'s environment).
pub fn continued(log: &Path, projects_dir: Option<&str>) -> PathBuf {
    let default = home_dir() + "/.claude/projects";
    let projects_dir = projects_dir.unwrap_or(&default);
    let mut at = log.to_path_buf();
    let mut seen: std::collections::HashSet<PathBuf> = [log.to_path_buf()].into_iter().collect();
    while seen.len() <= 16 {
        let Some(id) = continued_in(&at) else { break };
        let Some(next) = locate_one(&id, projects_dir, Some(&at)) else { break };
        if !seen.insert(next.clone()) {
            break;
        }
        at = next;
    }
    at
}

/// The `continuedInSessionId` of a log whose last line is a `continued-in` line, else None.
pub fn continued_in(log: &Path) -> Option<String> {
    let mut f = std::fs::File::open(log).ok()?;
    let size = f.seek(SeekFrom::End(0)).ok()?;
    if size == 0 {
        return None;
    }
    f.seek(SeekFrom::Start(size.saturating_sub(4096))).ok()?;
    let mut tail = vec![];
    f.read_to_end(&mut tail).ok()?;
    let last = tail.split(|b| *b == b'\n').rfind(|s| !s.is_empty())?;
    let o: serde_json::Value = serde_json::from_slice(last).ok()?;
    let o = o.as_object()?;
    if o.get("type")?.as_str()? != "continued-in" {
        return None;
    }
    o.get("continuedInSessionId")?.as_str().map(String::from)
}

#[cfg(test)]
mod tests {
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
        assert_eq!(continued(Path::new(&format!("{dir}/old1.jsonl")), Some(&projects)), PathBuf::from(&new1));
        assert_eq!(continued_in(Path::new("/nonexistent/x.jsonl")), None);
    }
}
