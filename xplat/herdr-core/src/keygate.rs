//! Swift: `KeyGate.swift`. Asked right before every key of an input that the owner did not type at the terminal: the
//! pane must still show the agent the checks saw, and `allowed` must still say yes. A refusal stops the input there;
//! nothing more is sent. herdr has no compare-and-send, so the moment between the last read and the key stays;
//! `passes` keeps it to one screen read.

use crate::model::*;
use crate::error::HerdrError;
use crate::screen::{all_panes, PaneScreen};
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
        let a = all_panes(&ws).into_iter().find(|p| p.id == pane_id)?.agent.clone()?;
        if a.backend == BackendKind::Hc && a.kind == seen.kind && a.process.as_ref() == Some(process) {
            return Some(a);
        }
        return None;
    }
    let sid = seen.session_id.as_ref().filter(|s| !s.is_empty())?;
    let ws = screen.snapshot().ok()?;
    let a = all_panes(&ws).into_iter().find(|p| p.id == pane_id)?.agent.clone()?;
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

/// Parity with Swift's `HerdrKitChecks` for the key gate (ChecksHubReview4/5, ChecksHCGate, ChecksHCGrok): `passes`
/// on a scripted `FakeScreen`. Each `assert!` names the Swift check it ports; "(rust extra)" ones have no Swift check.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{hc_pane, FakeScreen};
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn yes() -> Allowed {
        Arc::new(|| true)
    }

    fn no() -> Allowed {
        Arc::new(|| false)
    }

    /// `allowed` says yes `n` times, then no.
    fn budget(n: i64) -> Allowed {
        let left = Arc::new(AtomicI64::new(n));
        Arc::new(move || left.fetch_sub(1, Ordering::SeqCst) > 0)
    }

    fn tui_agent(kind: &str, session: Option<&str>) -> Agent {
        let mut a = Agent::new(kind, kind, AgentStatus::Blocked);
        a.backend = BackendKind::Tui;
        a.session_id = session.map(String::from);
        a
    }

    fn tui_screen(agent: Option<Agent>, screen: &str) -> FakeScreen {
        let pane = Pane {
            id: "w1:p1".into(),
            terminal_id: "term_1".into(),
            cwd: "/w".into(),
            agent,
            label: None,
            backend: BackendKind::Tui,
            process_ids: vec![],
        };
        FakeScreen::with_panes(vec![pane], vec![screen])
    }

    fn gate(a: &Agent, allowed: Allowed) -> KeyGate {
        KeyGate::new(Some(a.clone()), Some(allowed))
    }

    /// Records the order of snapshot reads, screen reads and keys.
    struct Rec {
        fake: FakeScreen,
        log: RefCell<Vec<&'static str>>,
    }

    impl Rec {
        fn new(fake: FakeScreen) -> Rec {
            Rec { fake, log: RefCell::new(vec![]) }
        }
    }

    impl PaneScreen for Rec {
        fn backend(&self) -> BackendKind {
            self.fake.backend()
        }
        fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError> {
            self.log.borrow_mut().push("snapshot");
            self.fake.snapshot()
        }
        fn visible_text(&self, pane_id: &str) -> Result<String, HerdrError> {
            self.log.borrow_mut().push("read");
            self.fake.visible_text(pane_id)
        }
        fn respond(&self, pane_id: &str, allow: bool) -> Result<(), HerdrError> {
            self.log.borrow_mut().push("respond");
            self.fake.respond(pane_id, allow)
        }
        fn send_named_keys(&self, keys: &[String], pane_id: &str) -> Result<(), HerdrError> {
            self.log.borrow_mut().push("keys");
            self.fake.send_named_keys(keys, pane_id)
        }
        fn send_keys(&self, bytes: &[u8], pane_id: &str) -> Result<(), HerdrError> {
            self.log.borrow_mut().push("bytes");
            self.fake.send_keys(bytes, pane_id)
        }
        fn send_text(&self, text: &str, pane_id: &str) -> Result<(), HerdrError> {
            self.log.borrow_mut().push("text");
            self.fake.send_text(text, pane_id)
        }
        fn pause(&self, _ms: u64) {}
    }

    // MARK: ChecksHubReview4.swift (passes)

    #[test]
    fn review4_passes() {
        let seen = tui_agent("claude", Some("s1"));
        let ready: ScreenOk = &|s| s == "ready";
        let other: ScreenOk = &|s| s == "other";
        let f = Rec::new(tui_screen(Some(tui_agent("claude", Some("s1"))), "ready"));
        assert!(passes(&f, Some(&gate(&seen, yes())), "w1:p1", Some(ready), None), "r4 gate: same agent, screen ok, allowed -> passes");
        assert_eq!(*f.log.borrow(), ["snapshot", "read"], "r4 gate: the screen is read last");
        assert!(!passes(&f.fake, Some(&gate(&seen, yes())), "w1:p1", Some(other), None), "r4 gate: screen changed -> no");
        assert!(!passes(&f.fake, Some(&gate(&seen, no())), "w1:p1", None, None), "r4 gate: level gone -> no");
        let f = tui_screen(Some(tui_agent("claude", Some("s2"))), "ready");
        assert!(!passes(&f, Some(&gate(&seen, yes())), "w1:p1", None, None), "r4 gate: another session in the pane -> no");
        let f = tui_screen(Some(tui_agent("codex", Some("s1"))), "ready");
        assert!(!passes(&f, Some(&gate(&seen, yes())), "w1:p1", None, None), "r4 gate: another agent kind -> no");
        let f = tui_screen(None, "ready");
        assert!(!passes(&f, Some(&gate(&seen, yes())), "w1:p1", None, None), "r4 gate: no agent -> no");
        let f = tui_screen(Some(tui_agent("claude", Some("s1"))), "ready");
        *f.fail_reads.borrow_mut() = 1;
        assert!(!passes(&f, Some(&gate(&seen, yes())), "w1:p1", Some(ready), None), "(rust extra) gate: the screen read fails -> no");
    }

    // MARK: ChecksHubReview5.swift (4: identity)

    #[test]
    fn review5_no_session() {
        let f = tui_screen(Some(tui_agent("claude", None)), "> ");
        assert!(
            !passes(&f, Some(&gate(&tui_agent("claude", None), yes())), "w1:p1", None, None),
            "r5 identity: no session id on either side -> no"
        );
        let f = tui_screen(Some(tui_agent("claude", Some(""))), "> ");
        assert!(
            !passes(&f, Some(&gate(&tui_agent("claude", Some("")), yes())), "w1:p1", None, None),
            "(rust extra) r5 identity: an empty session id on either side -> no"
        );
    }

    // MARK: ChecksHCGate.swift (hc: bound to the program in the terminal; the store adds the log's session id)

    fn hc_seen(addr: &str, kind: &str, process: Option<&str>) -> Agent {
        let mut a = hc_pane(addr, kind, AgentStatus::Blocked).agent.unwrap();
        a.process = process.map(String::from);
        a.session_id = Some("sid-1".into());
        a
    }

    fn hc_screen(addr: &str, kind: &str, process: Option<&str>) -> FakeScreen {
        let mut p = hc_pane(addr, kind, AgentStatus::Blocked);
        p.agent.as_mut().unwrap().process = process.map(String::from);
        FakeScreen::with_panes(vec![p], vec!["> "])
    }

    #[test]
    fn hc_gate() {
        let a = hc_seen("t1", "claude", Some("4242@1"));
        let f = hc_screen("t1", "claude", Some("4242@1"));
        assert!(passes(&f, Some(&gate(&a, yes())), "hc:t1", None, None), "hc gate: the same program in the terminal -> keys may go");
        let r = hc_seen("box/t3", "claude", Some("4242@1"));
        let f = hc_screen("box/t3", "claude", Some("4242@1"));
        assert!(passes(&f, Some(&gate(&r, yes())), "hc:box/t3", None, None), "hc gate: a terminal on another machine binds the same way");
        // Another program now: the agent quit and a new one started in the same terminal (new pid), or under the same shell.
        let f = hc_screen("t1", "claude", Some("5151@1"));
        assert!(!passes(&f, Some(&gate(&a, yes())), "hc:t1", None, None), "hc gate: a new pid -> no");
        let under_shell = hc_seen("t1", "claude", Some("6161@1"));
        let f = hc_screen("t1", "claude", Some("7171@1"));
        assert!(!passes(&f, Some(&gate(&under_shell, yes())), "hc:t1", None, None), "hc gate: a new foreground pid under the same shell -> no");
        let f = hc_screen("t1", "codex", Some("4242@1"));
        assert!(!passes(&f, Some(&gate(&a, yes())), "hc:t1", None, None), "hc gate: another agent kind -> no");
        // No program to bind to (a shell runs the agent, or no pid): no, even with the session id from the log.
        let shell = hc_seen("t1", "claude", None);
        let f = hc_screen("t1", "claude", None);
        assert!(!passes(&f, Some(&gate(&shell, yes())), "hc:t1", None, None), "hc gate: no foreground under a shell -> no");
        let f = hc_screen("t1", "claude", None);
        assert!(!passes(&f, Some(&gate(&shell, yes())), "hc:t1", None, None), "hc gate: no pid -> no");
        let f = FakeScreen::with_panes(vec![], vec!["> "]);
        assert!(!passes(&f, Some(&gate(&a, yes())), "hc:t1", None, None), "hc gate: the terminal is gone -> no");
        // A non-hc agent in the pane now (same kind, same process string): not the program the gate saw.
        let mut p = hc_pane("t1", "claude", AgentStatus::Blocked);
        let mut tui = tui_agent("claude", Some("sid-1"));
        tui.process = Some("4242@1".into());
        p.agent = Some(tui);
        let f = FakeScreen::with_panes(vec![p], vec!["> "]);
        assert!(!passes(&f, Some(&gate(&a, yes())), "hc:t1", None, None), "(rust extra) hc gate: a non-hc agent in the pane -> no");
    }

    // MARK: ChecksHCGrok.swift (passes on hc)

    #[test]
    fn hc_grok_passes() {
        let f = FakeScreen::with_panes(vec![hc_pane("t2", "grok", AgentStatus::Working)], vec!["> "]);
        assert!(
            passes(&f, Some(&KeyGate::new(None, Some(yes()))), "hc:t2", None, None)
                && !passes(&f, Some(&KeyGate::new(None, Some(no()))), "hc:t2", None, None),
            "hc hub: passes asks the device's gate"
        );
        // A gate bound to an agent without a session id (and, on hc, no process) fails closed.
        let seen = Agent::new("g", "grok", AgentStatus::Working);
        assert!(
            !passes(&f, Some(&KeyGate::new(Some(seen), Some(yes()))), "hc:t2", None, None),
            "hc hub: a gate agent with no session fails closed"
        );
    }

    // MARK: KeyGate.swift's own rules (no Swift check of their own)

    #[test]
    fn rules() {
        let f = tui_screen(None, "x");
        assert!(passes(&f, None, "w1:p1", Some(&|_| false), None) && *f.reads.borrow() == 0, "(rust extra) no gate: true, nothing read");
        let seen = tui_agent("claude", Some("s1"));
        let f = tui_screen(Some(tui_agent("claude", Some("s1"))), "x");
        assert!(
            !passes(&f, Some(&gate(&seen, yes())), "w1:p1", None, Some(&|a| a.status == AgentStatus::Idle)),
            "(rust extra) agent_ok is asked of the agent as it is now -> no"
        );
        assert!(
            passes(&f, Some(&gate(&seen, yes())), "w1:p1", None, Some(&|a| a.status == AgentStatus::Blocked)),
            "(rust extra) agent_ok holds -> passes"
        );
        // One gated call per key: the level gone after the first key stops the rest.
        let f = Rec::new(tui_screen(Some(tui_agent("claude", Some("s1"))), "x"));
        let keys = vec!["Down".to_string(), "Enter".to_string()];
        let r = gated_keys(&f, &keys, Some(&gate(&seen, budget(1))), "w1:p1", Some(&|_| true));
        assert!(r == Err(HerdrError::KeyGateStop) && f.fake.sent() == ["Down"], "(rust extra) gated keys: stopped after the first key");
        assert_eq!(f.log.borrow()[..3], ["snapshot", "read", "keys"], "(rust extra) gated keys: the screen read right before the key");
        let f = tui_screen(None, "x");
        assert!(
            gated_keys(&f, &keys, None, "w1:p1", None).is_ok() && f.sent() == ["Down", "Enter"],
            "(rust extra) gated keys: no gate, one call"
        );
        // has_identity and on_mac.
        let mut hc = Agent::new("c", "claude", AgentStatus::Blocked);
        hc.session_id = Some("s".into());
        assert!(!KeyGate::has_identity(&hc), "(rust extra) hc: a session id is not its identity");
        hc.process = Some("1@1".into());
        assert!(KeyGate::has_identity(&hc), "(rust extra) hc: the process is");
        assert!(
            KeyGate::has_identity(&seen) && !KeyGate::has_identity(&tui_agent("claude", Some(""))),
            "(rust extra) TUI: a non-empty session id"
        );
        assert!(KeyGate::on_mac(Some(tui_agent("claude", None)), None).agent.is_none(), "(rust extra) on_mac: no identity, no agent check");
        assert_eq!(KeyGate::on_mac(Some(seen.clone()), None).agent, Some(seen), "(rust extra) on_mac: bound to an agent with an identity");
    }

    #[test]
    fn wrapper_gate() {
        // Test 14 (Brief 7): gate bound to wrapper identity; snapshot then shows new fg_pid -> passes is false.
        let seen = hc_seen("t1", "claude", Some("500/501@1"));
        assert!(KeyGate::has_identity(&seen));
        let same = hc_screen("t1", "claude", Some("500/501@1"));
        assert!(passes(&same, Some(&gate(&seen, yes())), "hc:t1", None, None));
        let new_fg = hc_screen("t1", "claude", Some("600/601@1"));
        assert!(!passes(&new_fg, Some(&gate(&seen, yes())), "hc:t1", None, None), "new fg pid -> no");
        let restarted = hc_screen("t1", "claude", Some("500/509@1"));
        assert!(!passes(&restarted, Some(&gate(&seen, yes())), "hc:t1", None, None), "restarted agent under same wrapper -> no");

        // Test 15: agent gone after first key (wrapper stays -> process becomes None) -> gated_keys stops after first key.
        struct DisappearingAgent {
            inner: FakeScreen,
        }
        impl PaneScreen for DisappearingAgent {
            fn backend(&self) -> BackendKind {
                self.inner.backend()
            }
            fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError> {
                self.inner.snapshot()
            }
            fn visible_text(&self, pane_id: &str) -> Result<String, HerdrError> {
                self.inner.visible_text(pane_id)
            }
            fn respond(&self, pane_id: &str, allow: bool) -> Result<(), HerdrError> {
                self.inner.respond(pane_id, allow)
            }
            fn send_named_keys(&self, keys: &[String], pane_id: &str) -> Result<(), HerdrError> {
                let r = self.inner.send_named_keys(keys, pane_id);
                // After the first key, the agent exits and the wrapper remains with no agent identity.
                self.inner.workspaces.borrow_mut()[0].tabs[0].panes[0].agent.as_mut().unwrap().process = None;
                r
            }
            fn send_keys(&self, bytes: &[u8], pane_id: &str) -> Result<(), HerdrError> {
                self.inner.send_keys(bytes, pane_id)
            }
            fn send_text(&self, text: &str, pane_id: &str) -> Result<(), HerdrError> {
                self.inner.send_text(text, pane_id)
            }
            fn pause(&self, ms: u64) {
                self.inner.pause(ms)
            }
        }
        let scr = DisappearingAgent { inner: hc_screen("t1", "claude", Some("500/501@1")) };
        let keys = vec!["Down".to_string(), "Enter".to_string()];
        let res = gated_keys(&scr, &keys, Some(&gate(&seen, yes())), "hc:t1", None);
        assert_eq!(res, Err(HerdrError::KeyGateStop));
        assert_eq!(scr.inner.sent(), ["Down"], "agent gone after first key -> stops before Enter");
    }
}
