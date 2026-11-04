//! Read-only probe: connects to a herdr session and sends ONLY `session.snapshot`.
//! Prints counts (workspaces, tabs, panes, agent status counts). Never prints pane contents or conversation text,
//! and never sends input, keys, resize, takeover or any mutating method.
//!
//!     cargo run -p herdr-core --example snapshot -- [--session NAME]
#![cfg_attr(not(unix), allow(unused_imports))]

use herdr_core::{AgentStatus, HerdrClient, HerdrSession};
use std::collections::BTreeMap;

#[cfg(not(unix))]
fn main() {
    eprintln!("herdr serves a Unix socket: this probe runs on Unix only");
}

#[cfg(unix)]
fn main() {
    let session = HerdrSession::from_process();
    let label = session.name.clone().unwrap_or_else(|| "(default / from env)".into());
    let client = HerdrClient::new(&session);
    let started = std::time::Instant::now();
    let workspaces = match client.snapshot() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("snapshot failed: {e}");
            std::process::exit(1);
        }
    };
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    let tabs: usize = workspaces.iter().map(|w| w.tabs.len()).sum();
    let panes: usize = workspaces.iter().flat_map(|w| &w.tabs).map(|t| t.panes.len()).sum();
    let mut agents: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for a in workspaces.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).filter_map(|p| p.agent.as_ref()) {
        *agents.entry(a.status.as_str()).or_default() += 1;
        *by_kind.entry(a.kind.clone()).or_default() += 1;
    }
    let mut tab_status: BTreeMap<&str, usize> = BTreeMap::new();
    for t in workspaces.iter().flat_map(|w| &w.tabs) {
        *tab_status.entry(t.status.as_str()).or_default() += 1;
    }
    println!("session: {label}");
    println!("round trip: {ms:.1} ms (connect + snapshot + parse)");
    println!("workspaces: {}", workspaces.len());
    println!("tabs: {tabs}");
    println!("panes: {panes}");
    println!("agents: {}", agents.values().sum::<usize>());
    for s in [AgentStatus::Blocked, AgentStatus::Working, AgentStatus::Done, AgentStatus::Idle, AgentStatus::Unknown] {
        println!("  agent status {:<8} {}", s.as_str(), agents.get(s.as_str()).copied().unwrap_or(0));
    }
    println!("agent kinds: {by_kind:?}");
    println!("tab status rollup: {tab_status:?}");
}
