//! Swift: `KeyGate.swift`. Asked right before every key of an input that the owner did not type at the terminal: the
//! pane must still show the agent the checks saw, and `allowed` must still say yes. A refusal stops the input there;
//! nothing more is sent. herdr has no compare-and-send, so the moment between the last read and the key stays;
//! `passes` keeps it to one screen read.

use crate::models::*;
use std::sync::Arc;

pub type Allowed = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Clone)]
pub struct KeyGate {
    pub agent: Option<Agent>,
    pub allowed: Option<Allowed>,
}

impl std::fmt::Debug for KeyGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyGate").field("agent", &self.agent).field("allowed", &self.allowed.is_some()).finish()
    }
}

impl KeyGate {
    pub fn new(agent: Option<Agent>, allowed: Option<Allowed>) -> KeyGate {
        KeyGate { agent, allowed }
    }

    /// What a gate can bind an agent to: hc, the program in the terminal; herdr, the session id.
    pub fn has_identity(a: &Agent) -> bool {
        if a.backend == BackendKind::Hc {
            a.process.as_deref().is_some_and(|p| !p.is_empty())
        } else {
            a.session_id.as_deref().is_some_and(|s| !s.is_empty())
        }
    }

    /// The Mac's rule (the owner is at the screen). The sidecar never uses it: it stays fail closed (`KeyGate::new`).
    pub fn on_mac(agent: Option<Agent>, allowed: Option<Allowed>) -> KeyGate {
        KeyGate { agent: agent.filter(Self::has_identity), allowed }
    }
}

/// The pane's agent now (fresh snapshot), when it is `seen`. hc: the same kind and the same program in the
/// terminal (`Agent.process`); none reported: None. TUI: same kind and session id; none: None. Fails closed.
pub fn fresh_agent<S: PaneScreen + ?Sized>(screen: &S, seen: &Agent, pane_id: &str) -> Option<Agent> {
    if seen.backend == BackendKind::Hc {
        let process = seen.process.as_ref()?;
        let ws = screen.snapshot().ok()?;
        let a = Workspace::all_panes(&ws).into_iter().find(|p| p.id == pane_id)?.agent.clone()?;
        if a.backend == BackendKind::Hc && a.kind == seen.kind && a.process.as_ref() == Some(process) {
            return Some(a);
        }
        return None;
    }
    let sid = seen.session_id.as_ref().filter(|s| !s.is_empty())?;
    let ws = screen.snapshot().ok()?;
    let a = Workspace::all_panes(&ws).into_iter().find(|p| p.id == pane_id)?.agent.clone()?;
    if a.kind == seen.kind && a.session_id.as_ref() == Some(sid) {
        Some(a)
    } else {
        None
    }
}

pub type ScreenOk<'a> = &'a dyn Fn(&str) -> bool;
pub type AgentOk<'a> = &'a dyn Fn(&Agent) -> bool;

/// True when the next key may go: the pane still shows the gate's agent (same kind and identity; fresh snapshot) and
/// `agent_ok` holds for it as it is now, then `ok` holds on a fresh screen read, then `allowed()`. The screen is read
/// last, right before the answer. No gate: true, nothing read.
pub fn passes<S: PaneScreen + ?Sized>(
    screen: &S,
    gate: Option<&KeyGate>,
    pane_id: &str,
    ok: Option<ScreenOk>,
    agent_ok: Option<AgentOk>,
) -> bool {
    let Some(gate) = gate else { return true };
    if let Some(seen) = &gate.agent {
        let Some(now) = fresh_agent(screen, seen, pane_id) else { return false };
        if let Some(f) = agent_ok {
            if !f(&now) {
                return false;
            }
        }
    }
    if let Some(ok) = ok {
        match screen.visible_text(pane_id) {
            Ok(s) if ok(&s) => {}
            _ => return false,
        }
    }
    if let Some(allowed) = &gate.allowed {
        if !allowed() {
            return false;
        }
    }
    true
}

/// `Err(KeyGateStop)` when `passes` says no.
pub fn gated<S: PaneScreen + ?Sized>(screen: &S, gate: Option<&KeyGate>, pane_id: &str, ok: Option<ScreenOk>) -> Result<(), HerdrError> {
    if passes(screen, gate, pane_id, ok, None) {
        Ok(())
    } else {
        Err(HerdrError::KeyGateStop)
    }
}

/// Named keys, one gated call per key, so no committing key (Enter, Tab) rides in a batch past a check.
/// No gate: one call.
pub fn gated_keys<S: PaneScreen + ?Sized>(
    screen: &S,
    keys: &[String],
    gate: Option<&KeyGate>,
    pane_id: &str,
    ok: Option<ScreenOk>,
) -> Result<(), HerdrError> {
    if gate.is_none() {
        return screen.send_named_keys(keys, pane_id);
    }
    for k in keys {
        gated(screen, gate, pane_id, ok)?;
        screen.send_named_keys(std::slice::from_ref(k), pane_id)?;
    }
    Ok(())
}
