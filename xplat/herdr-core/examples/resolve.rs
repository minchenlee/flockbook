//! Read-only smoke: takes the session snapshot and the real process table, and prints ONLY counts
//! (panes, panes with an agent process, panes with a resolved log path). No paths, no content.
//! Sends only `session.snapshot`; never input, keys, resize or takeover.
//!
//!     cargo run -p herdr-core --example resolve -- [--session NAME]
#![cfg_attr(not(unix), allow(unused_imports))]

use herdr_core::resolver::{agent_process, default_source, Need, SessionResolver};
use herdr_core::{HerdrClient, HerdrSession};
use std::collections::HashSet;

#[cfg(not(unix))]
fn main() {
    eprintln!("herdr serves a Unix socket: this probe runs on Unix only");
}

#[cfg(unix)]
fn main() {
    let session = HerdrSession::from_process();
    let workspaces = match HerdrClient::new(&session).snapshot() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("snapshot failed: {e}");
            std::process::exit(1);
        }
    };
    let panes: Vec<_> = workspaces.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).collect();
    let source = default_source();
    let procs = source.processes();
    let with_agent = panes.iter().filter(|p| agent_process(&p.id, &session.socket_path, &procs).is_some()).count();

    let needs: Vec<Need> = panes
        .iter()
        .filter_map(|p| p.agent.as_ref().map(|a| Need::new(&p.id, &a.kind, &p.cwd, p.process_ids.clone())))
        .collect();
    let resolved = SessionResolver::new(&session.socket_path).resolve(&needs, &HashSet::new());

    println!("processes seen: {}", procs.len());
    println!("panes: {}", panes.len());
    println!("panes with an agent process: {with_agent}");
    println!("panes with an agent (snapshot): {}", needs.len());
    println!("panes with a resolved log path: {}", resolved.len());
    for kind in ["claude", "codex", "cursor", "pi", "grok", "agy"] {
        let asked = needs.iter().filter(|n| n.kind == kind).count();
        let got = needs.iter().filter(|n| n.kind == kind && resolved.contains_key(&n.pane_id)).count();
        println!("  {kind}: {asked} asked, {got} resolved");
    }
}
