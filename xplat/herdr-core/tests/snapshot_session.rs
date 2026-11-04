#![cfg(unix)] // a fake herdr server on a Unix socket
//! Ports of `checksSnapshot`, `checksSession`, `checksPaneLabel` (HerdrKitChecks/ChecksCore.swift).
mod common;
use common::*;
use herdr_core::snapshot;
use herdr_core::{AgentStatus, HerdrSession};
use std::collections::HashMap;
use std::path::Path;

#[test]
fn snapshot_fixture() {
    let ws = snapshot::parse(fixture("snapshot.json").as_bytes()).unwrap();
    assert_eq!(ws.len(), 1, "one workspace");
    assert_eq!(ws[0].tabs.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(), ["Main", "Shell"], "tab labels");
    assert_eq!(ws[0].tabs[0].panes.len(), 2, "two panes");
    assert_eq!(ws[0].tabs[0].status, AgentStatus::Blocked, "rollup blocked");
    assert!(ws[0].tabs[1].panes[0].agent.is_none() && ws[0].tabs[1].status == AgentStatus::Unknown, "shell pane");
    let a = ws[0].tabs[0].panes[0].agent.as_ref().unwrap();
    assert!(a.name == "demo-dev" && a.kind == "claude" && a.session_id.as_deref() == Some("sess-1") && a.title == "Fix the bug", "agent fields");
    use AgentStatus::*;
    assert_eq!(AgentStatus::rollup([Idle, Done, Working]), Working, "rollup order");
    assert_eq!(AgentStatus::rollup([]), Unknown, "empty rollup");
    assert_eq!(AgentStatus::rollup([Unknown, Idle]), Idle);
    assert_eq!(AgentStatus::rollup([Idle, Done]), Done);
    assert_eq!(AgentStatus::rollup([Working, Blocked, Done]), Blocked);

    assert_eq!(snapshot::session_id_for_pane(&ws, &ws[0].tabs[0].panes[0].id).as_deref(), Some("sess-1"), "sessionId(forPane:) finds agent session");
    assert!(snapshot::session_id_for_pane(&ws, "nope").is_none(), "unknown pane");
    assert!(snapshot::session_id_for_pane(&ws, &ws[0].tabs[1].panes[0].id).is_none(), "shell pane has no session");
}

#[test]
fn snapshot_pane_label_and_tolerance() {
    let snap = r#"{"id":"x","result":{"snapshot":{"workspaces":[{"workspace_id":"w1","label":"ws","number":1}],"tabs":[{"tab_id":"w1:t1","workspace_id":"w1","label":"1","number":1}],"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1","terminal_id":"term_1","cwd":"/","label":"build"},{"pane_id":"w1:p2","tab_id":"w1:t1","workspace_id":"w1","terminal_id":"term_2","cwd":"/"}],"agents":[]}}}"#;
    let ws = snapshot::parse(snap.as_bytes()).unwrap();
    let panes = &ws[0].tabs[0].panes;
    assert!(panes[0].label.as_deref() == Some("build") && panes[1].label.is_none(), "snapshot: pane label");
    // An empty label counts as no label; a bare `snapshot` root (no `result`) parses; garbage errors.
    let empty = snap.replace(r#""label":"build""#, r#""label":"""#);
    assert!(snapshot::parse(empty.as_bytes()).unwrap()[0].tabs[0].panes[0].label.is_none());
    let bare = r#"{"snapshot":{"workspaces":[{"workspace_id":"w9"}],"tabs":[],"panes":[]}}"#;
    let ws = snapshot::parse(bare.as_bytes()).unwrap();
    assert_eq!(ws[0].label, "w9", "missing label falls back to the id");
    assert!(snapshot::parse(br#"{"id":"x","result":{}}"#).is_err(), "no snapshot is a protocol error");
    assert!(snapshot::parse(b"nope").is_err());
}

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}
fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

#[test]
fn session_resolution() {
    let home = Path::new("/home/u");
    let r = |a: &[&str], e: &[(&str, &str)]| HerdrSession::resolve(&args(a), &env(e), home);
    assert_eq!(r(&["app", "--session", "x"], &[]).name.as_deref(), Some("x"), "--session");
    assert_eq!(r(&["app"], &[("HERDR_DESK_SESSION", "y")]).name.as_deref(), Some("y"), "app env session");
    assert_eq!(r(&["app"], &[]).name, None, "default session");
    assert_eq!(r(&["app"], &[]).socket_path, "/home/u/.config/herdr/herdr.sock");
    assert!(HerdrSession::named("c9-input-test", home).socket_path.ends_with("sessions/c9-input-test/herdr.sock"), "named socket");
    // herdr's order: --session > HERDR_SOCKET_PATH > HERDR_SESSION > default.
    let all = [("HERDR_SOCKET_PATH", "/run/h.sock"), ("HERDR_SESSION", "env-s")];
    assert_eq!(r(&["app", "--session", "cli"], &all).name.as_deref(), Some("cli"), "flag beats env");
    assert_eq!(r(&["app"], &all).socket_path, "/run/h.sock", "socket env beats session env");
    assert_eq!(r(&["app"], &all).name, None);
    assert_eq!(r(&["app"], &[("HERDR_SESSION", "env-s")]).socket_path, "/home/u/.config/herdr/sessions/env-s/herdr.sock");
    assert_eq!(r(&["app"], &[("HERDR_SESSION", "")]).name, None, "empty env is ignored");
    assert_eq!(r(&["app", "--session"], &[]).name, None, "dangling flag is ignored");
}
