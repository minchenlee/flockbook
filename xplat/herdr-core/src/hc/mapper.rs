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

/// Agent kind from the foreground process, then the start argv; last, agy's 7501 `app` name.
pub fn kind(t: &Term) -> Option<String> {
    let mut fg = vec![t.fg_name.clone().unwrap_or_default()];
    fg.extend(t.fg_argv.iter().cloned());
    for argv in [fg, t.argv.clone()] {
        if let Some(k) = kind_of_argv(&argv) {
            return Some(k);
        }
    }
    if t.status_app.as_deref() == Some("antigravity") {
        Some("agy".into())
    } else {
        None
    }
}

/// The agent's process for a key gate: the foreground pid when the foreground program is the agent; with no
/// foreground read, the terminal's pid only when the terminal's own program is the agent. Anything else: None, so
/// keys fail closed.
pub fn process(t: &Term, k: Option<&str>) -> Option<String> {
    let k = k?;
    let has_foreground = t.fg_pid.is_some() || t.fg_name.is_some() || !t.fg_argv.is_empty();
    let pid = if has_foreground {
        let mut fg = vec![t.fg_name.clone().unwrap_or_default()];
        fg.extend(t.fg_argv.iter().cloned());
        if kind_of_argv(&fg).as_deref() == Some(k) { t.fg_pid } else { None }
    } else if kind_of_argv(&t.argv).as_deref() == Some(k) {
        t.pid
    } else {
        None
    };
    let pid = pid.filter(|p| *p > 0)?;
    Some(format!("{pid}@{}", t.created))
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

pub fn agent(t: &Term) -> Option<Agent> {
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
        process: process(t, k.as_deref()),
    })
}

pub fn pane(t: &Term) -> Pane {
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
        agent: agent(t),
        label: t.name.clone(),
        backend: BackendKind::Hc,
        process_ids: pids,
    }
}

/// `terminal.list` records → one workspace with one tab of every live terminal, in list order.
pub fn workspaces(terminals: &[Value]) -> Vec<Workspace> {
    let panes: Vec<Pane> = terminals.iter().filter_map(Term::parse).filter(|t| t.state == "live").map(|t| pane(&t)).collect();
    vec![Workspace {
        id: "hc:all".into(),
        label: "herdr-core".into(),
        tabs: vec![Tab { id: "hc:all".into(), label: "herdr-core".into(), panes, ..Default::default() }],
    }]
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
}
