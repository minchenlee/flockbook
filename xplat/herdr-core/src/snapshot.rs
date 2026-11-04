//! Turns the `session.snapshot` payload into the public model (port of SnapshotParser.swift).
//! Tolerant: unknown fields are ignored.

use crate::error::HerdrError;
use crate::model::*;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Root {
    result: Option<ResultBody>,
    snapshot: Option<Snap>,
}
#[derive(Deserialize)]
struct ResultBody {
    snapshot: Option<Snap>,
}
#[derive(Deserialize, Default)]
struct Snap {
    #[serde(default)]
    workspaces: Vec<Ws>,
    #[serde(default)]
    tabs: Vec<TabInfo>,
    #[serde(default)]
    panes: Vec<PaneInfo>,
    #[serde(default)]
    agents: Vec<AgentInfo>,
}
#[derive(Deserialize)]
struct Ws {
    workspace_id: String,
    label: Option<String>,
}
#[derive(Deserialize)]
struct TabInfo {
    tab_id: String,
    workspace_id: String,
    label: Option<String>,
    number: Option<i64>,
    agent_status: Option<String>,
}
#[derive(Deserialize)]
struct SessionRef {
    value: Option<String>,
}
#[derive(Deserialize)]
struct PaneInfo {
    pane_id: String,
    tab_id: String,
    terminal_id: Option<String>,
    cwd: Option<String>,
    label: Option<String>,
    agent_session: Option<SessionRef>,
    terminal_title_stripped: Option<String>,
}
#[derive(Deserialize)]
struct AgentInfo {
    pane_id: String,
    agent: Option<String>,
    name: Option<String>,
    agent_status: Option<String>,
    agent_session: Option<SessionRef>,
    terminal_title_stripped: Option<String>,
}

pub fn parse(data: &[u8]) -> Result<Vec<Workspace>, HerdrError> {
    let root: Root = serde_json::from_slice(data).map_err(|e| HerdrError::Protocol(format!("bad snapshot json: {e}")))?;
    let snap = root
        .result
        .and_then(|r| r.snapshot)
        .or(root.snapshot)
        .ok_or_else(|| HerdrError::Protocol("no snapshot in response".into()))?;

    // First agent entry per pane wins (Swift: uniquingKeysWith keeps the first).
    let mut agent_by_pane: HashMap<&str, &AgentInfo> = HashMap::new();
    for a in &snap.agents {
        agent_by_pane.entry(a.pane_id.as_str()).or_insert(a);
    }

    let mut panes_by_tab: HashMap<&str, Vec<Pane>> = HashMap::new();
    for p in &snap.panes {
        let mut agent = None;
        if let Some(a) = agent_by_pane.get(p.pane_id.as_str()) {
            if let Some(kind) = &a.agent {
                let title = a
                    .terminal_title_stripped
                    .clone()
                    .or_else(|| p.terminal_title_stripped.clone())
                    .or_else(|| a.name.clone())
                    .unwrap_or_else(|| kind.clone());
                let sid = a.agent_session.as_ref().or(p.agent_session.as_ref()).and_then(|s| s.value.clone());
                agent = Some(Agent {
                    name: a.name.clone().unwrap_or_else(|| kind.clone()),
                    kind: kind.clone(),
                    status: AgentStatus::parse(a.agent_status.as_deref()),
                    session_id: sid,
                    title, ..Default::default()
                });
            }
        }
        panes_by_tab.entry(p.tab_id.as_str()).or_default().push(Pane {
            id: p.pane_id.clone(),
            terminal_id: p.terminal_id.clone().unwrap_or_default(),
            cwd: p.cwd.clone().unwrap_or_default(),
            agent,
            label: p.label.clone().filter(|l| !l.is_empty()), ..Default::default()
        });
    }

    let mut tabs_by_ws: HashMap<&str, Vec<Tab>> = HashMap::new();
    for t in &snap.tabs {
        let panes = panes_by_tab.remove(t.tab_id.as_str()).unwrap_or_default();
        let rollup = AgentStatus::rollup(panes.iter().filter_map(|p| p.agent.as_ref().map(|a| a.status)));
        let status = if panes.iter().any(|p| p.agent.is_some()) { rollup } else { AgentStatus::parse(t.agent_status.as_deref()) };
        tabs_by_ws.entry(t.workspace_id.as_str()).or_default().push(Tab {
            id: t.tab_id.clone(),
            label: t.label.clone().unwrap_or_default(),
            number: t.number.unwrap_or(0),
            panes,
            status,
        });
    }

    // Snapshot order is the real tab order; `number` stays with the tab after tab.move.
    Ok(snap
        .workspaces
        .iter()
        .map(|w| Workspace {
            id: w.workspace_id.clone(),
            label: w.label.clone().unwrap_or_else(|| w.workspace_id.clone()),
            tabs: tabs_by_ws.remove(w.workspace_id.as_str()).unwrap_or_default(),
        })
        .collect())
}

/// The agent session id of a pane (None for unknown panes and shell panes). Port of `HerdrStore.sessionId(forPane:in:)`.
pub fn session_id_for_pane(workspaces: &[Workspace], pane_id: &str) -> Option<String> {
    workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .flat_map(|t| &t.panes)
        .find(|p| p.id == pane_id)
        .and_then(|p| p.agent.as_ref())
        .and_then(|a| a.session_id.clone())
}
