//! Swift: `HCMapper.swift`, the part the sidecar needs: hc terminal records → panes with their agent.
//!
//! The Swift mapper also groups terminals into workspaces and tabs from the official app's `herdr-app` document
//! (`doc.get`, and each other machine's). The sidecar only looks a terminal up by address, so here every live
//! terminal goes into one workspace and one tab; the pane and agent fields are mapped exactly as in Swift.

use crate::model::*;
use serde_json::Value;

pub const PANE_PREFIX: &str = "hc:";

pub fn pane_id(address: &str) -> String {
    format!("{PANE_PREFIX}{address}")
}

pub fn address_from_pane_id(id: &str) -> Option<String> {
    id.strip_prefix(PANE_PREFIX).map(String::from)
}

/// "host2/" for a terminal on another machine ("host2/t7"), "" here.
pub fn machine_prefix(address: &str) -> String {
    match address.rfind('/') {
        Some(i) => address[..=i].to_string(),
        None => String::new(),
    }
}

/// One terminal record, only the fields the app uses. Unknown or missing fields fall back to empty values.
#[derive(Clone, Debug, Default)]
pub struct Term {
    pub address: String,
    pub argv: Vec<String>,
    pub cwd: String,
    pub name: Option<String>,
    pub title: String,
    pub pid: Option<i64>,
    pub state: String,
    pub status: Option<String>,
    pub status_kind: Option<String>,
    pub status_msg: String,
    pub status_app: Option<String>,
    pub fg_name: Option<String>,
    pub fg_argv: Vec<String>,
    pub fg_cwd: Option<String>,
    pub fg_pid: Option<i64>,
    pub status_ms: Option<i64>,
    pub created: i64,
}

fn s(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(String::from)
}
fn strings(v: Option<&Value>) -> Vec<String> {
    // Swift `as? [String]` fails for the whole array when one element is not a string.
    match v.and_then(Value::as_array) {
        Some(a) if a.iter().all(Value::is_string) => a.iter().map(|x| x.as_str().unwrap().to_string()).collect(),
        _ => vec![],
    }
}
fn int(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).or_else(|| v.as_bool().map(|b| b as i64))
}

impl Term {
    pub fn parse(o: &Value) -> Option<Term> {
        let a = o.get("address")?.as_str().filter(|a| !a.is_empty())?;
        let st = o.get("status").filter(|v| v.is_object());
        let fg = o.get("foreground").filter(|v| v.is_object());
        Some(Term {
            address: a.into(),
            argv: strings(o.get("argv")),
            cwd: s(o.get("cwd")).unwrap_or_default(),
            name: s(o.get("name")).filter(|n| !n.is_empty()),
            title: s(o.get("title")).unwrap_or_default(),
            pid: int(o.get("pid")),
            state: s(o.get("state")).unwrap_or_else(|| "live".into()),
            status: st.and_then(|x| s(x.get("state"))),
            status_kind: st.and_then(|x| s(x.get("kind"))),
            status_msg: st.and_then(|x| s(x.get("msg"))).unwrap_or_default(),
            status_app: st.and_then(|x| s(x.get("app"))),
            fg_name: fg.and_then(|x| s(x.get("name"))),
            fg_argv: strings(fg.and_then(|x| x.get("argv"))),
            fg_cwd: fg.and_then(|x| s(x.get("cwd"))).filter(|c| !c.is_empty()),
            fg_pid: fg.and_then(|x| int(x.get("pid"))),
            status_ms: int(o.get("status_ms")),
            created: int(o.get("created_ms")).unwrap_or(0),
        })
    }
}

fn last_path_component(p: &str) -> String {
    // NSString.lastPathComponent: trailing slashes dropped; "/" stays "/".
    let t = p.trim_end_matches('/');
    if t.is_empty() {
        return if p.is_empty() { String::new() } else { "/".into() };
    }
    t.rsplit('/').next().unwrap_or(t).to_string()
}

/// Agent programs Flockbook knows, by executable name.
fn known_kind(w: &str) -> Option<&'static str> {
    Some(match w {
        "claude" => "claude",
        "codex" => "codex",
        "pi" => "pi",
        "omp" => "omp",
        "cursor-agent" | "cursor" => "cursor",
        "gemini" => "gemini",
        "opencode" => "opencode",
        "amp" => "amp",
        "grok" => "grok",
        "agy" | "antigravity" => "agy",
        _ => return None,
    })
}
const WRAPPERS: [&str; 8] = ["node", "bun", "deno", "python", "python3", "env", "npx", "bunx"];

/// The agent kind one command line names, if any. Interpreters (`node …/codex`) are looked through.
pub fn kind_of_argv(argv: &[String]) -> Option<String> {
    let mut words: Vec<String> = argv.iter().filter(|a| !a.is_empty()).map(|a| last_path_component(a)).collect();
    if let Some(first) = words.first() {
        let second = words.get(1).cloned().unwrap_or_default();
        if WRAPPERS.contains(&first.as_str()) || WRAPPERS.contains(&second.as_str()) {
            words.retain(|w| !w.starts_with('-') && !WRAPPERS.contains(&w.as_str()));
        }
    }
    words.iter().take(2).find_map(|w| known_kind(w)).map(String::from)
}

/// The agent kind a command line runs (not only names): its program ([`crate::resolver::program`]) is the agent.
/// The key gate's direct path uses it; [`kind_of_argv`] (display) also looks through other words.
fn exec_kind(argv: &[String]) -> Option<&'static str> {
    let words: Vec<&str> = argv.iter().map(String::as_str).filter(|a| !a.is_empty()).collect();
    crate::resolver::program(&words).and_then(known_kind)
}

/// Agent kind reported by OSC 7501 `status.app`, for agents with live evidence.
pub fn kind_from_app(app: Option<&str>) -> Option<&'static str> {
    match app {
        Some("claude-code") => Some("claude"),
        Some("codex") => Some("codex"),
        Some("antigravity") => Some("agy"),
        Some("omp") => Some("omp"),
        _ => None,
    }
}

/// Agent kind from the foreground process, then the start argv; last, the 7501 `app` name.
pub fn kind(t: &Term) -> Option<String> {
    let mut fg = vec![t.fg_name.clone().unwrap_or_default()];
    fg.extend(t.fg_argv.iter().cloned());
    for argv in [fg, t.argv.clone()] {
        if let Some(k) = kind_of_argv(&argv) {
            return Some(k);
        }
    }
    kind_from_app(t.status_app.as_deref()).map(String::from)
}

/// Question passed to [`AgentCheck`] when a local terminal's foreground program is not a shell and does not directly
/// name the agent kind reported by `status.app` (the wrapper case).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrapperAsk {
    pub address: String,
    pub fg_pid: i32,
    pub created: i64,
    pub kind: String,
    pub status_ms: i64,
}

/// Resolves the running agent pid under a wrapper foreground process. Implementations must fail closed (`None`)
/// when uncertain.
pub trait AgentCheck {
    fn agent_under(&self, ask: &WrapperAsk) -> Option<i32>;

    /// Returns a check scoped to one snapshot so implementations can share a single process-table read across all
    /// wrapper terminals in that snapshot.
    fn snapshot(&self) -> Option<Box<dyn AgentCheck + '_>> {
        None
    }

    /// Drops any per-address state for terminals absent from the latest snapshot.
    fn prune(&self, _live_addresses: &[&str]) {}
}

/// Default fail-closed check: never resolves wrapper terminals.
pub struct NoAgentCheck;

impl AgentCheck for NoAgentCheck {
    fn agent_under(&self, _ask: &WrapperAsk) -> Option<i32> {
        None
    }
}

const SHELLS: [&str; 14] = [
    "sh", "bash", "zsh", "fish", "dash", "ksh", "csh", "tcsh", "nu", "xonsh", "elvish", "pwsh", "powershell", "cmd",
];

fn is_shell_name(raw: &str) -> bool {
    let base = last_path_component(raw);
    let base = base.strip_prefix('-').unwrap_or(&base);
    SHELLS.contains(&base)
}

fn foreground_is_shell(t: &Term) -> bool {
    let fg0 = t.fg_argv.first().map(String::as_str).unwrap_or("");
    let name = t.fg_name.as_deref().unwrap_or("");
    if name.is_empty() && fg0.is_empty() {
        return true;
    }
    (!name.is_empty() && is_shell_name(name)) || (!fg0.is_empty() && is_shell_name(fg0))
}

/// The agent's process for a key gate: the foreground pid when the foreground program is the agent; with no
/// foreground read, the terminal's pid only when the terminal's own program is the agent; else, when a wrapper
/// passes W1-W4 and `check` verifies W5-W6, `<fg_pid>/<agent_pid>@<created>`. Anything else: None, so keys fail closed.
pub fn process_with(t: &Term, k: Option<&str>, check: &dyn AgentCheck) -> Option<String> {
    let k = k?;
    let has_foreground = t.fg_pid.is_some() || t.fg_name.is_some() || !t.fg_argv.is_empty();
    if has_foreground {
        // The foreground's own argv (its name only without one: a native agent's name can be its version).
        let fg = if t.fg_argv.is_empty() { vec![t.fg_name.clone().unwrap_or_default()] } else { t.fg_argv.clone() };
        if exec_kind(&fg) == Some(k) {
            let pid = t.fg_pid.filter(|p| *p > 0)?;
            return Some(format!("{pid}@{}", t.created));
        }
    } else if exec_kind(&t.argv) == Some(k) {
        let pid = t.pid.filter(|p| *p > 0)?;
        return Some(format!("{pid}@{}", t.created));
    } else {
        return None;
    }

    // Wrapper path (W1-W4): local terminal, status.app matches kind, fg_pid > 0 and status_ms present, non-shell foreground.
    if kind_from_app(t.status_app.as_deref()) != Some(k) || t.address.contains('/') || foreground_is_shell(t) {
        return None;
    }
    let fg_pid = t.fg_pid.filter(|&p| p > 0).and_then(|p| i32::try_from(p).ok())?;
    let status_ms = t.status_ms?;
    let ask = WrapperAsk {
        address: t.address.clone(),
        fg_pid,
        created: t.created,
        kind: k.into(),
        status_ms,
    };
    let agent_pid = check.agent_under(&ask).filter(|&p| p > 0)?;
    Some(format!("{fg_pid}/{agent_pid}@{}", t.created))
}

/// [`process_with`] with [`NoAgentCheck`] (direct paths only; wrapper paths fail closed).
pub fn process(t: &Term, k: Option<&str>) -> Option<String> {
    process_with(t, k, &NoAgentCheck)
}

/// hc reports only what the program said (OSC 7501). No report yet = unknown, never idle. `error` shows as done.
pub fn status(word: Option<&str>) -> AgentStatus {
    match word {
        Some("idle") => AgentStatus::Idle,
        Some("working") => AgentStatus::Working,
        Some("blocked") => AgentStatus::Blocked,
        Some("done") | Some("error") => AgentStatus::Done,
        _ => AgentStatus::Unknown,
    }
}

pub fn agent_with(t: &Term, check: &dyn AgentCheck) -> Option<Agent> {
    let k = kind(t);
    if k.is_none() && t.status.is_none() {
        return None; // a shell that never reported: plain pane
    }
    let kind_s = k.clone().unwrap_or_else(|| {
        t.fg_name.clone().or_else(|| t.argv.first().map(|a| last_path_component(a))).unwrap_or_else(|| "agent".into())
    });
    let name = t.name.clone().unwrap_or_else(|| kind_s.clone());
    let mut title = if t.title.is_empty() { name.clone() } else { t.title.clone() };
    let mut status_kind = if t.status.as_deref() == Some("blocked") { t.status_kind.clone() } else { None };
    if t.status.as_deref() == Some("error") {
        title = if t.status_msg.is_empty() { "error".into() } else { format!("error: {}", t.status_msg) };
        status_kind = Some("error".into());
    }
    Some(Agent {
        name,
        kind: kind_s,
        status: status(t.status.as_deref()),
        session_id: None,
        title,
        backend: BackendKind::Hc,
        status_kind,
        process: process_with(t, k.as_deref(), check),
    })
}

pub fn agent(t: &Term) -> Option<Agent> {
    agent_with(t, &NoAgentCheck)
}

pub fn pane_with(t: &Term, check: &dyn AgentCheck) -> Pane {
    let mut pids: Vec<i32> = vec![];
    for p in [t.fg_pid, t.pid].into_iter().flatten() {
        if p > 0 && !pids.contains(&(p as i32)) {
            pids.push(p as i32);
        }
    }
    // Remote terminals (`host2/t7`) run on another machine: their pids mean nothing here.
    if t.address.contains('/') {
        pids.clear();
    }
    Pane {
        id: pane_id(&t.address),
        terminal_id: t.address.clone(),
        cwd: t.fg_cwd.clone().unwrap_or_else(|| t.cwd.clone()),
        agent: agent_with(t, check),
        label: t.name.clone(),
        backend: BackendKind::Hc,
        process_ids: pids,
    }
}

pub fn pane(t: &Term) -> Pane {
    pane_with(t, &NoAgentCheck)
}

/// `terminal.list` records → one workspace with one tab of every live terminal, in list order.
pub fn workspaces_with(terminals: &[Value], check: &dyn AgentCheck) -> Vec<Workspace> {
    let live: Vec<Term> = terminals.iter().filter_map(Term::parse).filter(|t| t.state == "live").collect();
    let snap = check.snapshot();
    let active = snap.as_deref().unwrap_or(check);
    let panes: Vec<Pane> = live.iter().map(|t| pane_with(t, active)).collect();
    drop(snap);
    let live_addrs: Vec<&str> = live.iter().map(|t| t.address.as_str()).collect();
    check.prune(&live_addrs);
    vec![Workspace {
        id: "hc:all".into(),
        label: "herdr-core".into(),
        tabs: vec![Tab { id: "hc:all".into(), label: "herdr-core".into(), panes, ..Default::default() }],
    }]
}

/// `terminal.list` records → one workspace with one tab of every live terminal, in list order (`NoAgentCheck`).
pub fn workspaces(terminals: &[Value]) -> Vec<Workspace> {
    workspaces_with(terminals, &NoAgentCheck)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kinds_and_processes() {
        let t = Term::parse(&json!({"address":"t1","argv":["/usr/bin/node","/opt/bin/codex","--yolo"],"pid":10,"created_ms":5})).unwrap();
        assert_eq!(kind(&t).as_deref(), Some("codex"));
        assert_eq!(process(&t, Some("codex")).as_deref(), Some("10@5"));
        // A shell in front of the agent: no identity.
        let t = Term::parse(&json!({"address":"t2","argv":["claude"],"pid":11,"foreground":{"name":"zsh","pid":12}})).unwrap();
        assert_eq!(kind(&t).as_deref(), Some("claude"));
        assert_eq!(process(&t, Some("claude")), None);
        let t = Term::parse(&json!({"address":"t3","argv":["zsh"],"pid":11,"foreground":{"name":"claude","argv":["claude"],"pid":13}})).unwrap();
        assert_eq!(process(&t, Some("claude")).as_deref(), Some("13@0"));
        // agy by its 7501 app name only.
        let t = Term::parse(&json!({"address":"t4","argv":["sh"],"status":{"state":"blocked","kind":"permission","app":"antigravity"}})).unwrap();
        let a = agent(&t).unwrap();
        assert_eq!(a.kind, "agy");
        assert_eq!(a.status, AgentStatus::Blocked);
        assert_eq!(a.status_kind.as_deref(), Some("permission"));
        assert_eq!(a.process, None);
    }

    #[test]
    fn omp_kind() {
        let t = Term::parse(&json!({"address":"t5","argv":["/usr/local/bin/omp"],"pid":20,"created_ms":7})).unwrap();
        assert_eq!(kind(&t).as_deref(), Some("omp"));
        assert_eq!(process(&t, Some("omp")).as_deref(), Some("20@7"));
        let t = Term::parse(&json!({"address":"t6","argv":["bun","/opt/omp/bin/omp"],"pid":21})).unwrap();
        assert_eq!(kind(&t).as_deref(), Some("omp"), "through an interpreter");
        // A launcher shows a shell in front: omp by its 7501 app name.
        let t = Term::parse(&json!({"address":"t7","argv":["launch"],"pid":22,"foreground":{"name":"fish","pid":23},"status":{"state":"idle","app":"omp"}})).unwrap();
        assert_eq!(kind(&t).as_deref(), Some("omp"));
        assert_eq!(process(&t, Some("omp")), None, "keys still need the omp process itself");
    }

    #[test]
    fn panes_and_errors() {
        let terms = vec![
            json!({"address":"t1","argv":["zsh"],"cwd":"/w"}),
            json!({"address":"t2","argv":["claude"],"pid":7,"cwd":"/w","foreground":{"name":"claude","pid":8,"cwd":"/w/sub"},"status":{"state":"error","msg":"boom"}}),
            json!({"address":"t3","argv":["claude"],"state":"exited"}),
            json!({"address":"box/t4","argv":["claude"],"pid":9}),
        ];
        let ws = workspaces(&terms);
        let panes = crate::screen::all_panes(&ws);
        assert_eq!(panes.len(), 3);
        assert!(panes[0].agent.is_none());
        let p = panes[1];
        assert_eq!(p.id, "hc:t2");
        assert_eq!(p.cwd, "/w/sub");
        assert_eq!(p.process_ids, vec![8, 7]);
        let a = p.agent.as_ref().unwrap();
        assert_eq!(a.status, AgentStatus::Done);
        assert_eq!(a.title, "error: boom");
        assert_eq!(a.status_kind.as_deref(), Some("error"));
        assert!(panes[2].process_ids.is_empty());
        assert_eq!(machine_prefix("box/t4"), "box/");
        assert_eq!(machine_prefix("t4"), "");
    }

    #[test]
    fn wrapper_kind_from_status_app() {
        // Test 1: claude-code -> claude and codex -> codex with secretspec in front.
        let t = Term::parse(&json!({
            "address": "t1",
            "argv": ["zsh"],
            "foreground": {"name": "secretspec", "argv": ["secretspec", "run", "--profile", "claude", "--", "claude"], "pid": 500},
            "status": {"state": "working", "app": "claude-code"}
        })).unwrap();
        assert_eq!(kind(&t).as_deref(), Some("claude"));

        let t = Term::parse(&json!({
            "address": "t2",
            "argv": ["zsh"],
            "foreground": {"name": "secretspec", "argv": ["secretspec", "run", "--", "codex"], "pid": 501},
            "status": {"state": "idle", "app": "codex"}
        })).unwrap();
        assert_eq!(kind(&t).as_deref(), Some("codex"));

        for app in ["pi", "claude", "zsh"] {
            let t = Term::parse(&json!({
                "address": "t3",
                "argv": ["zsh"],
                "foreground": {"name": "secretspec", "argv": ["secretspec", "run", "--", "x"], "pid": 502},
                "status": {"state": "idle", "app": app}
            })).unwrap();
            assert_eq!(kind(&t), None, "app {app} must not map to a kind");
        }

        // Test 2: foreground wins over status.app.
        let t = Term::parse(&json!({
            "address": "t4",
            "argv": ["/bin/zsh", "-l"],
            "foreground": {"name": "pi", "argv": ["pi"], "pid": 503},
            "status": {"state": "done", "app": "claude-code"}
        })).unwrap();
        assert_eq!(kind(&t).as_deref(), Some("pi"));
    }

    struct PanicCheck;
    impl AgentCheck for PanicCheck {
        fn agent_under(&self, ask: &WrapperAsk) -> Option<i32> {
            panic!("check must not be called for {ask:?}");
        }
    }

    struct FixedProcs(Vec<crate::resolver::Proc>);
    impl crate::resolver::ProcessSource for FixedProcs {
        fn processes(&self) -> Vec<crate::resolver::Proc> {
            self.0.clone()
        }
        fn open_files(&self, _: &[i32]) -> Vec<String> {
            vec![]
        }
    }

    fn tree_with(procs: Vec<crate::resolver::Proc>) -> crate::hc::wrapper::TreeCheck {
        use std::sync::Arc;
        use std::time::Instant;
        crate::hc::wrapper::TreeCheck::with_hooks(Arc::new(FixedProcs(procs)), Arc::new(Instant::now), Arc::new(|_| true))
    }

    fn proc_(pid: i32, ppid: i32, ms: i64, line: &str) -> crate::resolver::Proc {
        crate::resolver::Proc {
            pid,
            ppid,
            start: chrono::DateTime::from_timestamp_millis(ms).unwrap(),
            line: line.into(),
        }
    }

    #[test]
    fn wrapper_identities_and_fail_closed() {
        let base_ms: i64 = 1_794_500_000_000;
        let wrap_json = json!({
            "address": "t1",
            "argv": ["zsh"],
            "pid": 400,
            "created_ms": 7,
            "foreground": {"name": "secretspec", "argv": ["secretspec", "run", "--profile", "claude", "--", "claude"], "pid": 500},
            "status": {"state": "working", "app": "claude-code"},
            "status_ms": base_ms + 2000
        });
        let wrap = Term::parse(&wrap_json).unwrap();

        // Test 3 (Brief 1): wrapper + one agent child + status newer than start -> <fg_pid>/<agent_pid>@<created>.
        let chk = tree_with(vec![proc_(500, 400, base_ms, "secretspec run -- claude"), proc_(501, 500, base_ms + 1000, "claude")]);
        assert_eq!(process_with(&wrap, Some("claude"), &chk).as_deref(), Some("500/501@7"));

        // Test 4 (Brief 2): wrapper in front, no agent process -> None.
        let chk_none = tree_with(vec![proc_(500, 400, base_ms, "secretspec run -- claude")]);
        assert_eq!(process_with(&wrap, Some("claude"), &chk_none), None);

        // Test 5 (Brief 3): shell in front, stale status.app -> None, and check is never called.
        for (name, argv) in [
            ("zsh", json!(["zsh"])),
            ("-zsh", json!(["-zsh"])),
            ("bash", json!(["/bin/bash", "-l"])),
            ("wrapper.sh", json!(["sh", "wrapper.sh"])),
        ] {
            let t = Term::parse(&json!({
                "address": "t-sh",
                "argv": ["zsh"],
                "foreground": {"name": name, "argv": argv, "pid": 500},
                "status": {"state": "done", "app": "claude-code"},
                "status_ms": base_ms + 2000,
                "created_ms": 7
            })).unwrap();
            assert_eq!(process_with(&t, Some("claude"), &PanicCheck), None, "shell {name} must fail closed without checking");
        }

        // Test 6 (Brief 4): vim and less in front, stale status.app, no agent below -> None.
        for viewer in ["vim", "less"] {
            let t = Term::parse(&json!({
                "address": "t-view",
                "argv": ["zsh"],
                "foreground": {"name": viewer, "argv": [viewer, "notes.md"], "pid": 500},
                "status": {"state": "done", "app": "claude-code"},
                "status_ms": base_ms + 2000,
                "created_ms": 7
            })).unwrap();
            assert_eq!(process_with(&t, Some("claude"), &chk_none), None, "{viewer} without agent below -> None");
        }

        // Test 7 (Brief 5): two agent children at the same depth -> None; agent + deeper helper -> nearer wins.
        let chk_tie = tree_with(vec![
            proc_(500, 400, base_ms, "secretspec run -- claude"),
            proc_(501, 500, base_ms + 1000, "claude"),
            proc_(502, 500, base_ms + 1500, "claude"),
        ]);
        assert_eq!(process_with(&wrap, Some("claude"), &chk_tie), None, "tie at same depth -> None");

        let chk_helper = tree_with(vec![
            proc_(500, 400, base_ms, "secretspec run -- claude"),
            proc_(501, 500, base_ms + 1000, "claude"),
            proc_(502, 501, base_ms + 2000, "claude -p helper"),
        ]);
        assert_eq!(process_with(&wrap, Some("claude"), &chk_helper).as_deref(), Some("500/501@7"), "nearer agent wins over helper");

        // Test 8: agent restarted under same wrapper -> different identity.
        let chk_restart = tree_with(vec![proc_(500, 400, base_ms, "secretspec run -- claude"), proc_(509, 500, base_ms + 1500, "claude")]);
        let id1 = process_with(&wrap, Some("claude"), &chk);
        let id2 = process_with(&wrap, Some("claude"), &chk_restart);
        assert_eq!(id2.as_deref(), Some("500/509@7"));
        assert_ne!(id1, id2);

        // Test 9 (Brief 6): direct claude (foreground and no-foreground) never calls check.
        let direct_fg = Term::parse(&json!({
            "address": "t-direct",
            "argv": ["zsh"],
            "foreground": {"name": "2.1.322", "argv": ["claude"], "pid": 500},
            "status": {"state": "working", "app": "claude-code"},
            "status_ms": base_ms,
            "created_ms": 7
        })).unwrap();
        assert_eq!(process_with(&direct_fg, Some("claude"), &PanicCheck).as_deref(), Some("500@7"));

        let direct_start = Term::parse(&json!({
            "address": "t-direct2",
            "argv": ["claude"],
            "pid": 400,
            "status": {"state": "working", "app": "claude-code"},
            "status_ms": base_ms,
            "created_ms": 7
        })).unwrap();
        assert_eq!(process_with(&direct_start, Some("claude"), &PanicCheck).as_deref(), Some("400@7"));

        // Test 10 (W6): agent started > 1s after status_ms -> None; inside 1s slack -> Some; missing status_ms -> None (no check).
        let chk_late = tree_with(vec![
            proc_(500, 400, base_ms, "secretspec run -- claude"),
            proc_(501, 500, base_ms + 4000, "claude"),
        ]);
        assert_eq!(process_with(&wrap, Some("claude"), &chk_late), None, "agent started >1s after status_ms -> None");

        let chk_slack = tree_with(vec![
            proc_(500, 400, base_ms, "secretspec run -- claude"),
            proc_(501, 500, base_ms + 3000, "claude"), // status_ms is base_ms + 2000; +1000 == 3000
        ]);
        assert_eq!(process_with(&wrap, Some("claude"), &chk_slack).as_deref(), Some("500/501@7"), "1s lstart slack allowed");

        let no_status_ms = Term::parse(&json!({
            "address": "t1",
            "argv": ["zsh"],
            "foreground": {"name": "secretspec", "argv": ["secretspec", "run", "--", "claude"], "pid": 500},
            "status": {"state": "working", "app": "claude-code"},
            "created_ms": 7
        })).unwrap();
        assert_eq!(process_with(&no_status_ms, Some("claude"), &PanicCheck), None, "missing status_ms -> None without check");

        // Test 11 (W2): remote address -> None, check never called.
        let remote = Term::parse(&json!({
            "address": "box/t1",
            "argv": ["zsh"],
            "foreground": {"name": "secretspec", "argv": ["secretspec", "run", "--", "claude"], "pid": 500},
            "status": {"state": "working", "app": "claude-code"},
            "status_ms": base_ms + 2000,
            "created_ms": 7
        })).unwrap();
        assert_eq!(process_with(&remote, Some("claude"), &PanicCheck), None, "remote wrapper -> None without check");

        // Test 12: workspaces(terms) (no check) with wrapper fixture -> None.
        let ws = workspaces(&[wrap_json]);
        assert_eq!(crate::screen::all_panes(&ws)[0].agent.as_ref().unwrap().process, None);

        // Test 13: empty process list -> None.
        let chk_empty = tree_with(vec![]);
        assert_eq!(process_with(&wrap, Some("claude"), &chk_empty), None);
    }

    /// Codex P1: the key gate's direct path takes the foreground as the agent only when its program is the agent, or
    /// an interpreter whose script is the agent. `script claude` only names it.
    #[test]
    fn direct_identity_needs_the_real_executable() {
        let term = |fg: Value, argv: Value, app: Option<&str>| {
            let mut j = json!({"address": "t1", "argv": argv, "pid": 400, "created_ms": 7, "status_ms": 1_794_500_002_000i64});
            if !fg.is_null() {
                j["foreground"] = fg;
            }
            if let Some(a) = app {
                j["status"] = json!({"state": "working", "app": a});
            }
            Term::parse(&j).unwrap()
        };
        let fg = |argv: Value| json!({"argv": argv, "pid": 600});
        let zsh = json!(["zsh"]);
        assert_eq!(process(&term(fg(json!(["claude", "--resume"])), zsh.clone(), None), Some("claude")).as_deref(), Some("600@7"));
        assert_eq!(process(&term(fg(json!(["node", "/opt/x/bin/codex"])), zsh.clone(), None), Some("codex")).as_deref(), Some("600@7"));
        for argv in [json!(["script", "claude"]), json!(["vim", "claude"]), json!(["node", "--inspect", "/opt/x/bin/claude"])] {
            assert_eq!(process(&term(fg(argv.clone()), zsh.clone(), None), Some("claude")), None, "{argv} names claude, does not run it");
        }
        // No foreground read: the terminal's own program.
        assert_eq!(process(&term(Value::Null, json!(["claude"]), None), Some("claude")).as_deref(), Some("400@7"));
        assert_eq!(process(&term(Value::Null, json!(["script", "claude"]), None), Some("claude")), None, "script claude as the terminal's program");
        // `script` in front of a real claude: the tree check proves it, as for any wrapper.
        let base_ms: i64 = 1_794_500_000_000;
        let chk = tree_with(vec![proc_(600, 400, base_ms, "script claude"), proc_(601, 600, base_ms + 1000, "claude")]);
        let wrapped = term(fg(json!(["script", "claude"])), zsh, Some("claude-code"));
        assert_eq!(process_with(&wrapped, Some("claude"), &chk).as_deref(), Some("600/601@7"));
    }
}
