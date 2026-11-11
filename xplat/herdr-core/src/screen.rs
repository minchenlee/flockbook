//! Screen reads and keys for the approval and question cards (Swift `HerdrBackend` + `PaneScreen`). Synchronous: one
//! call per request. `hc::HcClient` implements it; checks use `fake::FakeScreen`. Reads are safe any time; everything
//! that sends keys runs only on an owner's action, behind a `keygate::KeyGate`.

use crate::error::HerdrError;
use crate::model::*;

/// Every pane of a snapshot, in order.
pub fn all_panes(ws: &[Workspace]) -> Vec<&Pane> {
    ws.iter().flat_map(|w| w.tabs.iter()).flat_map(|t| t.panes.iter()).collect()
}

/// What became of a message handed to a backend (Swift `DeliveryOutcome`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Sent,
    /// No answer in time: it may still arrive; the text must not come back into the composer (a retry sends it twice).
    Unconfirmed,
    Failed,
}

pub trait PaneScreen {
    fn backend(&self) -> BackendKind;
    fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError>;
    /// The pane's visible screen as plain text. Read-only.
    fn visible_text(&self, pane_id: &str) -> Result<String, HerdrError>;
    /// Answers a permission prompt: allow once (`1`), or deny (Esc). Call only when the prompt is on screen.
    fn respond(&self, pane_id: &str, allow: bool) -> Result<(), HerdrError>;
    /// Named keys in herdr's spelling ("Enter", "Down", "Escape", "C-c").
    fn send_named_keys(&self, keys: &[String], pane_id: &str) -> Result<(), HerdrError>;
    /// Raw bytes: control keys as named keys, the rest as text without Enter.
    fn send_keys(&self, bytes: &[u8], pane_id: &str) -> Result<(), HerdrError>;
    /// Types `text` and presses Enter.
    fn send_text(&self, text: &str, pane_id: &str) -> Result<(), HerdrError>;
    /// Whether `send_keys` can type every character of `text` (hc: `keys::typeable`). Checked before any key of
    /// an answer goes, so an answer never stops half typed.
    fn can_type(&self, _text: &str) -> bool {
        true
    }
    /// Sleeps between screen polls (fakes skip it).
    fn pause(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}
