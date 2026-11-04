//! A scripted `PaneScreen` for checks (no socket). Screens are shown in order: each read takes the current one; a
//! key (or text) moves to the next screen when `advance_on_key` is on. Every key and text is recorded.

use crate::error::HerdrError;
use crate::model::*;
use crate::screen::PaneScreen;
use std::cell::RefCell;

pub struct FakeScreen {
    pub workspaces: RefCell<Vec<Workspace>>,
    pub screens: RefCell<Vec<String>>,
    pub at: RefCell<usize>,
    pub advance_on_key: bool,
    /// Every key or text sent, in order: named keys as is, `respond` as "respond:allow" / "respond:deny",
    /// raw bytes as "bytes:<text>", text as "text:<text>".
    pub sent: RefCell<Vec<String>>,
    pub reads: RefCell<usize>,
    /// Make the next N reads fail.
    pub fail_reads: RefCell<usize>,
    /// What `respond`/keys return (e.g. a timeout).
    pub key_error: RefCell<Option<HerdrError>>,
    pub text_error: RefCell<Option<HerdrError>>,
}

impl FakeScreen {
    pub fn new(workspaces: Vec<Workspace>, screens: Vec<&str>) -> FakeScreen {
        FakeScreen {
            workspaces: RefCell::new(workspaces),
            screens: RefCell::new(screens.into_iter().map(String::from).collect()),
            at: RefCell::new(0),
            advance_on_key: true,
            sent: RefCell::new(vec![]),
            reads: RefCell::new(0),
            fail_reads: RefCell::new(0),
            key_error: RefCell::new(None),
            text_error: RefCell::new(None),
        }
    }

    /// One workspace, one tab, these panes.
    pub fn with_panes(panes: Vec<Pane>, screens: Vec<&str>) -> FakeScreen {
        FakeScreen::new(
            vec![Workspace { id: "w".into(), label: "w".into(), tabs: vec![Tab { id: "t".into(), label: "t".into(), panes, ..Default::default() }] }],
            screens,
        )
    }

    fn advance(&self) {
        if self.advance_on_key {
            let n = self.screens.borrow().len();
            let mut at = self.at.borrow_mut();
            if *at + 1 < n {
                *at += 1;
            }
        }
    }

    fn key_result(&self) -> Result<(), HerdrError> {
        match self.key_error.borrow().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    pub fn sent(&self) -> Vec<String> {
        self.sent.borrow().clone()
    }
}

impl PaneScreen for FakeScreen {
    fn backend(&self) -> BackendKind {
        BackendKind::Hc
    }
    fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError> {
        Ok(self.workspaces.borrow().clone())
    }
    fn visible_text(&self, _pane_id: &str) -> Result<String, HerdrError> {
        *self.reads.borrow_mut() += 1;
        {
            let mut f = self.fail_reads.borrow_mut();
            if *f > 0 {
                *f -= 1;
                return Err(HerdrError::Protocol("read failed".into()));
            }
        }
        let s = self.screens.borrow();
        Ok(s.get(*self.at.borrow()).cloned().unwrap_or_default())
    }
    fn respond(&self, _pane_id: &str, allow: bool) -> Result<(), HerdrError> {
        self.sent.borrow_mut().push(if allow { "respond:allow".into() } else { "respond:deny".into() });
        self.advance();
        self.key_result()
    }
    fn send_named_keys(&self, keys: &[String], _pane_id: &str) -> Result<(), HerdrError> {
        self.sent.borrow_mut().extend(keys.iter().cloned());
        self.advance();
        self.key_result()
    }
    fn send_keys(&self, bytes: &[u8], _pane_id: &str) -> Result<(), HerdrError> {
        self.sent.borrow_mut().push(format!("bytes:{}", String::from_utf8_lossy(bytes)));
        self.advance();
        self.key_result()
    }
    fn send_text(&self, text: &str, _pane_id: &str) -> Result<(), HerdrError> {
        self.sent.borrow_mut().push(format!("text:{text}"));
        self.advance();
        match self.text_error.borrow().clone() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
    fn pause(&self, _ms: u64) {}
}

/// An hc pane `hc:<addr>` running `kind` with a process identity.
pub fn hc_pane(addr: &str, kind: &str, status: AgentStatus) -> Pane {
    let mut a = Agent::new(kind, kind, status);
    a.process = Some("4242@1".into());
    Pane {
        id: format!("hc:{addr}"),
        terminal_id: addr.into(),
        cwd: "/tmp/project".into(),
        agent: Some(a),
        label: None,
        backend: BackendKind::Hc,
        process_ids: vec![4242],
    }
}
