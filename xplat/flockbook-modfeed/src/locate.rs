//! Swift: `Locate.swift`. Finds the session log behind one herdr-core terminal, with a small file cache so most calls
//! skip the process scan.

use crate::feed;
use herdr_core::*;
use herdr_core::session_log;
use herdr_core::resolver::{Need, SessionResolver};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: String,
    pub cwd: String,
    pub pids: Vec<i32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub process: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub path: Option<String>,
    /// Seconds since 1970.
    pub checked: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Cache {
    #[serde(default)]
    pub entries: BTreeMap<String, Entry>,
    /// terminal → the rows the last `feed` printed.
    #[serde(default)]
    pub feeds: BTreeMap<String, feed::State>,
}

pub struct Found {
    pub pane: Pane,
    pub path: Option<String>,
    /// The cache was used (no process scan).
    pub cached: bool,
}

/// A cached path is checked again after this long.
pub const RECHECK_S: f64 = 30.0;

#[derive(Debug)]
pub enum Failure {
    NoTerminal(String),
    NotAgent(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::NoTerminal(t) => write!(f, "no terminal {t} in herdr-core"),
            Failure::NotAgent(t) => write!(f, "terminal {t} runs no agent"),
        }
    }
}

/// The pane for a terminal address ("t7"), its pane id ("hc:t7").
pub fn pane<'a>(ws: &'a [Workspace], terminal: &str) -> Option<&'a Pane> {
    herdr_core::all_panes(ws)
        .into_iter()
        .find(|p| p.terminal_id == terminal || p.id == terminal || p.id == format!("hc:{terminal}"))
}

fn pane_process(pane: &Pane) -> Option<&str> {
    pane.agent.as_ref().and_then(|a| a.process.as_deref())
}

/// The cache entry still holds: same agent kind, processes, and resolved child identity, checked lately, and its log is still there.
pub fn fresh(e: Option<&Entry>, pane: &Pane, kind: &str, now_s: f64) -> bool {
    let Some(e) = e else { return false };
    if e.kind != kind || e.pids != pane.process_ids || e.process.as_deref() != pane_process(pane) || now_s - e.checked >= RECHECK_S {
        return false;
    }
    e.path.as_ref().is_some_and(|p| Path::new(p).exists())
}

pub fn read_cache(path: Option<&Path>) -> Cache {
    path.and_then(|p| std::fs::read(p).ok()).and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
}

/// Written whole through a temp file, so a reader never sees half a file.
pub fn write_cache(c: &Cache, to: Option<&Path>) {
    write_json(c, to)
}

/// Any of the helper's kept files: written whole through a temp file, for the owner only.
pub fn write_json<T: Serialize>(c: &T, to: Option<&Path>) {
    let Some(url) = to else { return };
    let Ok(d) = serde_json::to_vec(c) else { return };
    let dir = url.parent().unwrap_or(Path::new(""));
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    let _ = b.create(dir);
    let tmp = dir.join(format!(".{}.{}", url.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), std::process::id()));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let ok = o.open(&tmp).and_then(|mut f| std::io::Write::write_all(&mut f, &d)).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if std::fs::rename(&tmp, url).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Claude goes on in a new log and herdr keeps the old id: both paths follow `continued-in`.
fn latest(p: Option<String>, kind: &str) -> Option<String> {
    let p = p?;
    if kind != "claude" {
        return Some(p);
    }
    Some(session_log::continued(Path::new(&p), None).to_string_lossy().into_owned())
}

/// The resolver gives a full path, or for some agents only a session id: look that up in the agent's folder.
fn absolute(v: Option<&String>, kind: &str) -> Option<String> {
    let v = v.filter(|v| !v.is_empty())?;
    if v.starts_with('/') || Path::new(v).is_absolute() {
        return Some(v.clone());
    }
    session_log::locate(kind, v, session_log::default_dir(kind)).map(|p| p.to_string_lossy().into_owned())
}

/// For a send: the log looked up now, never the cached one (`/clear` gives the same agent a new log and writes no
/// `continued-in`), and the entry the cache had for this terminal before (`was`). The cache is written again, so the
/// next `feed` reads the log found now.
pub fn recheck(terminal: &str, ws: &[Workspace], resolver: &mut SessionResolver, cache_path: Option<&Path>, now_s: f64) -> Result<(Found, Option<Entry>), Failure> {
    let was = pane(ws, terminal).and_then(|p| read_cache(cache_path).entries.get(&p.terminal_id).cloned());
    look(terminal, ws, resolver, cache_path, now_s, false).map(|f| (f, was))
}

/// The cached entry `was` is this pane's agent process (same kind, processes, and resolved child identity), and it showed `path`: a log the
/// agent wrote. Proof that another log now is a new conversation of the same agent, not another agent.
pub fn same_agent(was: &Entry, pane: &Pane, kind: &str, path: &str) -> bool {
    was.kind == kind && !was.pids.is_empty() && was.pids == pane.process_ids && was.process.as_deref() == pane_process(pane) && latest(was.path.clone(), kind).as_deref() == Some(path)
}

/// Finds the log for `terminal`. On a cache miss it resolves every local agent pane in one pass, so two panes never
/// get the same log, and caches them all.
pub fn find(terminal: &str, ws: &[Workspace], resolver: &mut SessionResolver, cache_path: Option<&Path>, now_s: f64) -> Result<Found, Failure> {
    look(terminal, ws, resolver, cache_path, now_s, true)
}

fn look(terminal: &str, ws: &[Workspace], resolver: &mut SessionResolver, cache_path: Option<&Path>, now_s: f64, use_cache: bool) -> Result<Found, Failure> {
    let pane = pane(ws, terminal).cloned().ok_or_else(|| Failure::NoTerminal(terminal.into()))?;
    let kind = pane.agent.as_ref().map(|a| a.kind.clone()).ok_or_else(|| Failure::NotAgent(terminal.into()))?;
    let cache = read_cache(cache_path);
    if let Some(e) = cache.entries.get(&pane.terminal_id).filter(|_| use_cache) {
        if fresh(Some(e), &pane, &kind, now_s) {
            let path = latest(e.path.clone(), &kind);
            return Ok(Found { pane, path, cached: true });
        }
    }
    let next = resolve(ws, resolver, &cache, cache_path, now_s);
    let path = next.entries.get(&pane.terminal_id).filter(|_| !pane.terminal_id.contains('/')).and_then(|e| e.path.clone());
    Ok(Found { pane, path, cached: false })
}

/// The agent panes on this machine.
fn local_agents(ws: &[Workspace]) -> Vec<&Pane> {
    herdr_core::all_panes(ws).into_iter().filter(|p| p.agent.is_some() && !p.terminal_id.contains('/')).collect()
}

/// Resolves every local agent pane in one pass, writes the cache (the feed states kept) and returns it.
fn resolve(ws: &[Workspace], resolver: &mut SessionResolver, cache: &Cache, cache_path: Option<&Path>, now_s: f64) -> Cache {
    let agents = local_agents(ws);
    let needs: Vec<Need> = agents
        .iter()
        .map(|p| Need::new(&p.id, &p.agent.as_ref().unwrap().kind, &p.cwd, p.process_ids.clone()))
        .collect();
    let found = resolver.resolve(&needs, &HashSet::new());
    let mut next = Cache { feeds: cache.feeds.clone(), ..Default::default() };
    for p in &agents {
        let k = &p.agent.as_ref().unwrap().kind;
        next.entries.insert(
            p.terminal_id.clone(),
            Entry { kind: k.clone(), cwd: p.cwd.clone(), pids: p.process_ids.clone(), process: pane_process(p).map(String::from), path: latest(absolute(found.get(&p.id), k), k), checked: now_s },
        );
    }
    write_cache(&next, cache_path);
    next
}

/// As `fresh`, but an agent with no log yet also counts: `glance` asks for every agent, and one without a log must
/// not start a process scan on each call.
fn settled(e: Option<&Entry>, pane: &Pane, kind: &str, now_s: f64) -> bool {
    let Some(e) = e else { return false };
    if e.kind != kind || e.pids != pane.process_ids || e.process.as_deref() != pane_process(pane) || now_s - e.checked >= RECHECK_S {
        return false;
    }
    e.path.as_ref().is_none_or(|p| Path::new(p).exists())
}

/// Every agent pane on this machine with its log (`glance`). The cache answers while every entry holds; else one
/// pass resolves them all.
pub fn all(ws: &[Workspace], resolver: &mut SessionResolver, cache_path: Option<&Path>, now_s: f64) -> Vec<Found> {
    let agents = local_agents(ws);
    let kind = |p: &Pane| p.agent.as_ref().map(|a| a.kind.clone()).unwrap_or_default();
    let mut cache = read_cache(cache_path);
    let cached = agents.iter().all(|p| settled(cache.entries.get(&p.terminal_id), p, &kind(p), now_s));
    if !cached {
        cache = resolve(ws, resolver, &cache, cache_path, now_s);
    }
    agents
        .into_iter()
        .map(|p| Found { pane: p.clone(), path: cache.entries.get(&p.terminal_id).and_then(|e| latest(e.path.clone(), &kind(p))), cached })
        .collect()
}
