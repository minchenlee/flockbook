//! Port of `checksSessionFallback` (14 checks) plus tests for the cross-platform process sources.
//! Every assertion message equals the Swift `check(...)` text it ports.
//!
//! Changed in the merge with rust/herdrkit (one resolver, the Swift port): process start times are `Date`
//! (`DateTime<Utc>`) and log paths are strings built with `/`, as in Swift; `log_path` takes the agent's home
//! (Swift's prefixes under `~`) instead of the env-derived `AgentRoots`, which are gone (Swift reads no
//! `CODEX_HOME` / `CLAUDE_CONFIG_DIR`); `resolve` keeps its own clock. The fuller Swift checks live in
//! `src/resolver/tests.rs`.
mod common;

use chrono::{DateTime, Local, TimeZone, Utc};
use common::temp_dir;
use herdr_core::resolver::{
    agent_process, codex_candidates, codex_meta, is_claimed, log_path, parse_linux_stat, parse_lsof, parse_ps, subtree, Need,
    ProcessSource, Proc, SessionResolver,
};
#[cfg(unix)]
use herdr_core::resolver::LinuxProcSource;
use herdr_core::util::Date;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const SOCK: &str = "/home/u/.config/herdr/herdr.sock";
const OTHER: &str = "/home/u/.config/herdr/sessions/t/herdr.sock";

fn ps_text() -> String {
    format!(
        "\
  100     1 Sat Oct 10 13:50:00 2026     -zsh HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={SOCK} TERM=x
  101   100 Sat Oct 10 13:55:02 2026     codex HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={SOCK} TERM=x
  102   101 Sat Oct 10 13:56:00 2026     /bin/zsh -c ls HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={SOCK}
  103   100 Sat Oct 10 13:51:00 2026     vim HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={SOCK}
  200     1 Sat Oct 10 13:40:00 2026     -zsh HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={OTHER}
  201   200 Sat Oct 10 13:59:00 2026     codex HERDR_PANE_ID=w1:p1 HERDR_SOCKET_PATH={OTHER}
  300     1 Sat Oct 10 13:40:00 2026     -zsh HERDR_PANE_ID=w1:p10 HERDR_SOCKET_PATH={SOCK}
  301   300 Sat Oct 10 14:00:00 2026     codex HERDR_PANE_ID=w1:p10 HERDR_SOCKET_PATH={SOCK}
  garbage line
"
    )
}

fn pid_of(p: Option<Proc>) -> Option<i32> {
    p.map(|p| p.pid)
}

fn at(secs: i64) -> Date {
    DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[test]
fn session_fallback_pure_parts() {
    let procs = parse_ps(&ps_text());
    assert!(procs.len() == 8, "resolver: ps lines parsed");
    assert!(
        pid_of(agent_process("w1:p1", SOCK, &procs)) == Some(101),
        "resolver: newest shell child of the pane, this session only"
    );
    assert!(pid_of(agent_process("w1:p1", OTHER, &procs)) == Some(201), "resolver: other session's pane is separate");
    assert!(pid_of(agent_process("w1:p10", SOCK, &procs)) == Some(301), "resolver: w1:p1 does not match w1:p10");
    assert!(agent_process("w9:p9", SOCK, &procs).is_none(), "resolver: unknown pane");
    assert!(subtree(101, &procs).into_iter().collect::<HashSet<_>>() == HashSet::from([101, 102]), "resolver: process subtree");

    let home = "/home/u";
    let ro = format!("{home}/.codex/sessions/2026/10/10/rollout-2026-10-10T13-55-04-abc.jsonl");
    let ro2 = format!("{home}/.codex/sessions/2026/10/10/rollout-2026-10-10T14-00-00-def.jsonl");
    assert!(
        log_path(&[format!("{home}/.codex/logs_2.sqlite"), ro.clone(), ro.clone()], "codex", None, home) == Some(ro.clone()),
        "resolver: open rollout"
    );
    assert!(log_path(&[ro.clone(), ro2], "codex", None, home).is_none(), "resolver: two open rollouts: no guess");
    assert!(
        log_path(std::slice::from_ref(&ro), "claude", None, home).is_none()
            && log_path(&[format!("{home}/.claude/projects/x/id.jsonl")], "claude", None, home).is_some(),
        "resolver: log kind per agent"
    );
    let set = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<HashSet<String>>();
    assert!(
        is_claimed(&ro, &set(&["abc"])) && is_claimed(&ro, &set(&[ro.as_str()])) && !is_claimed(&ro, &set(&["bc"])) && !is_claimed(&ro, &set(&[])),
        "resolver: claimed by id or path"
    );
}

/// Builds a codex sessions tree under a temp dir, like the Swift check.
struct Tree {
    root: PathBuf,
    day: PathBuf,
    start: Date,
}

fn local_fmt(t: Date, f: &str) -> String {
    t.with_timezone(&Local).format(f).to_string()
}

impl Tree {
    fn new() -> Tree {
        let root = temp_dir("res");
        let start = at(1_791_000_000);
        let day = root.join(local_fmt(start, "%Y/%m/%d"));
        fs::create_dir_all(&day).unwrap();
        Tree { root, day, start }
    }
    fn root(&self) -> String {
        s(&self.root)
    }
    fn roll(&self, dt: i64, id: &str, cwd: &str) -> String {
        let t = self.start + chrono::Duration::seconds(dt);
        let p = self.day.join(format!("rollout-{}-{id}.jsonl", local_fmt(t, "%Y-%m-%dT%H-%M-%S")));
        let ts = t.format("%Y-%m-%dT%H:%M:%S.000Z");
        let meta = format!(r#"{{"timestamp":"x","type":"session_meta","payload":{{"id":"{id}","timestamp":"{ts}","cwd":"{cwd}"}}}}"#) + "\n";
        fs::write(&p, meta).unwrap();
        s(&p)
    }
}

#[test]
fn session_fallback_codex_tree() {
    let t = Tree::new();
    t.roll(-600, "old", "/w/a");
    let hit = t.roll(2, "hit", "/w/a");
    t.roll(5, "elsewhere", "/w/b");
    let now = t.start + chrono::Duration::seconds(60);
    assert!(codex_meta(&hit).map(|m| m.0).as_deref() == Some("/w/a"), "resolver: codex session_meta read");
    assert!(codex_candidates(&t.root(), "/w/a", t.start, now) == vec![hit.clone()], "resolver: codex cwd + start: one match");
    assert!(codex_candidates(&t.root(), "/w/a/", t.start, now) == vec![hit.clone()], "resolver: cwd compared standardized");
    t.roll(30, "second", "/w/a");
    assert!(codex_candidates(&t.root(), "/w/a", t.start, now).len() == 2, "resolver: two matches are both returned (caller refuses)");
    let _ = fs::remove_dir_all(&t.root);
}

#[test]
fn codex_meta_edge_cases() {
    let d = temp_dir("meta");
    let bad = d.join("bad.jsonl");
    fs::write(&bad, "{\"type\":\"other\"}\n").unwrap();
    assert!(codex_meta(&s(&bad)).is_none(), "resolver: non session_meta line is rejected");
    assert!(codex_meta(&s(&d.join("missing.jsonl"))).is_none(), "resolver: missing file is none");
    // no trailing newline, outer timestamp used when the payload has none
    let nonl = d.join("nonl.jsonl");
    fs::write(&nonl, r#"{"timestamp":"2026-10-10T00:00:00Z","type":"session_meta","payload":{"cwd":"/w/z"}}"#).unwrap();
    assert!(codex_meta(&s(&nonl)) == Some(("/w/z".into(), at(1_791_590_400))), "resolver: outer timestamp fallback");
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn parse_ps_start_time_is_local() {
    let procs = parse_ps("  7  1 Sat Oct 10 13:55:02 2026   codex X=1\n  8  1 Sun Oct  4 01:02:03 2026   vim\n");
    let want = Local.with_ymd_and_hms(2026, 10, 10, 13, 55, 2).earliest().unwrap().with_timezone(&Utc);
    assert_eq!(procs[0].start, want);
    assert_eq!(procs.len(), 2, "single-digit day is padded with a space");
}

#[test]
fn parse_lsof_takes_path_lines() {
    let out = "p101\nfcwd\nn/w/a\nf3\nn/home/u/.codex/sessions/x.jsonl\nf4\nnTCP 1.2.3.4:5\n";
    assert_eq!(parse_lsof(out), vec!["/w/a".to_string(), "/home/u/.codex/sessions/x.jsonl".to_string()]);
}

#[test]
fn linux_stat_parses_names_with_parens() {
    let stat = "42 (my (odd) cmd) S 7 42 42 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 5000 1000 100 18446744073709551615";
    assert_eq!(parse_linux_stat(stat), Some((7, 5000)));
    assert_eq!(parse_linux_stat("garbage"), None);
}

/// A synthetic /proc tree: 100 = shell, 101 = agent child holding a rollout open, 999 = unrelated.
#[cfg(unix)]
fn fake_proc(root: &Path, rollout: &Path) {
    let mk = |pid: i32, ppid: i32, ticks: u64, cmd: &[&str], env: &[&str]| {
        let d = root.join(pid.to_string());
        fs::create_dir_all(d.join("fd")).unwrap();
        fs::write(d.join("stat"), format!("{pid} (x y) S {ppid} 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {ticks} 1 1")).unwrap();
        fs::write(d.join("cmdline"), cmd.join("\0") + "\0").unwrap();
        fs::write(d.join("environ"), env.join("\0") + "\0").unwrap();
        d
    };
    fs::write(root.join("stat"), "cpu 1 2 3\nbtime 1000000\n").unwrap();
    let env = ["HERDR_PANE_ID=w1:p1", &format!("HERDR_SOCKET_PATH={SOCK}")];
    mk(100, 1, 1000, &["-zsh"], &env);
    let agent = mk(101, 100, 2000, &["codex"], &env);
    mk(999, 1, 10, &["sleep", "1"], &["PATH=/bin"]);
    std::os::unix::fs::symlink(rollout, agent.join("fd").join("3")).unwrap();
    fs::write(root.join("not-a-pid"), "").unwrap();
}

#[cfg(unix)]
#[test]
fn linux_source_reads_synthetic_proc() {
    let d = temp_dir("proc");
    let rollout = d.join("r.jsonl");
    fs::write(&rollout, "").unwrap();
    fake_proc(&d, &rollout);
    let src = LinuxProcSource::with_root(d.clone());
    let procs = src.processes();
    assert_eq!(procs.len(), 3);
    let agent = agent_process("w1:p1", SOCK, &procs).expect("agent");
    assert_eq!(agent.pid, 101);
    assert_eq!(agent.ppid, 100);
    assert_eq!(agent.start, at(1_000_000 + 20), "start = btime + ticks / 100");
    assert!(agent.line.starts_with("codex "), "cmdline NULs become spaces");
    let open = src.open_files(&subtree(101, &procs));
    assert_eq!(open, vec![rollout.to_string_lossy().into_owned()]);
    let _ = fs::remove_dir_all(&d);
}

/// Replaces `roots_follow_env_overrides`: the resolver's roots are the log locator's (one rule, Swift's).
#[test]
fn resolver_roots_are_the_locators() {
    let r = SessionResolver::new(SOCK);
    let d = |a: &str| herdr_core::session_log::default_dir(a).to_string_lossy().into_owned();
    assert_eq!(r.codex_dir, d("codex"));
    assert_eq!(r.projects_dir, d("claude"));
}

/// A canned source for the end-to-end resolver flow.
struct Fake {
    procs: Vec<Proc>,
    open: Vec<String>,
}
impl ProcessSource for Fake {
    fn processes(&self) -> Vec<Proc> {
        self.procs.clone()
    }
    fn open_files(&self, _pids: &[i32]) -> Vec<String> {
        self.open.clone()
    }
}

#[test]
fn resolver_prefers_open_file_then_codex_cwd_match() {
    let t = Tree::new();
    let hit = t.roll(2, "hit", "/w/a");
    let mut procs = parse_ps(&ps_text());
    for p in &mut procs {
        p.start = t.start; // agent 101 starts at the tree's start
    }
    let need = Need::new("w1:p1", "codex", "/w/a", vec![]);
    let make = |procs: &[Proc], open: Vec<String>| {
        let mut r = SessionResolver::new(SOCK).with_source(Box::new(Fake { procs: procs.to_vec(), open }));
        r.codex_dir = t.root();
        r
    };
    // no open files: falls back to cwd + start
    let out = make(&procs, vec![]).resolve(&[need.clone()], &HashSet::new());
    assert_eq!(out.get("w1:p1"), Some(&hit));
    // claimed by another pane: never handed out again
    let claimed: HashSet<String> = [hit.clone()].into();
    assert!(make(&procs, vec![]).resolve(&[need.clone()], &claimed).is_empty());
    // two candidates: no guess
    t.roll(3, "other", "/w/a");
    assert!(make(&procs, vec![]).resolve(std::slice::from_ref(&need), &HashSet::new()).is_empty());
    // an open rollout wins over the cwd match
    let open = format!("{}/.codex/sessions/2026/10/10/rollout-2026-10-10T13-55-04-abc.jsonl", herdr_core::session_log::home_dir());
    let out = make(&procs, vec![open.clone()]).resolve(&[need], &HashSet::new());
    assert_eq!(out.get("w1:p1"), Some(&open));
    let _ = fs::remove_dir_all(&t.root);
}

#[test]
fn standardize_matches_foundation_rules() {
    use herdr_core::session_log::standardize_path_in;
    let home = "/home/u";
    let st = |p: &str| standardize_path_in(p, home);
    // Changed: Foundation drops `/private` only on macOS and only when the rest exists (the Swift port does the
    // same), and does not lowercase on Windows; those cases are no longer asserted here.
    assert_eq!(st("/private/other/x"), "/private/other/x");
    assert_eq!(st("/private/tmpx"), "/private/tmpx");
    assert_eq!(st("~/w/./a/../b"), "/home/u/w/b");
    assert_eq!(st("~"), "/home/u");
    assert_eq!(st("/w/a/"), st("/w/a"));
    assert_eq!(st("/a/./b//c/../d"), "/a/b/d");
}
