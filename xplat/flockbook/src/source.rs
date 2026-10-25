//! Data sources. The UI only reads through `DataSource`.
//!
//! `LiveSource` wraps herdr-core's `HerdrStore` and only reads (snapshot + events). `DemoSource` serves fixed,
//! neutral demo data and never opens a socket.

use herdr_core::{ConversationItem, Pane, Workspace};

pub trait DataSource {
    fn conversation_state(&self, _pane: &Pane) -> herdr_core::ConversationState { herdr_core::ConversationState::Ready }
    fn status_tags(&self, _pane: &Pane) -> Vec<herdr_core::StatusTag> { Vec::new() }
    fn log_path(&self, _pane: &Pane) -> Option<std::path::PathBuf> { None }

    /// Short label for the title or a status line, e.g. "demo" or a session name.
    fn label(&self) -> String;
    fn workspaces(&self) -> Vec<Workspace>;
    fn connected(&self) -> bool;
    /// Conversation of one pane (the agent's session log). Empty when there is none.
    fn conversation(&self, pane: &Pane) -> Vec<ConversationItem>;
    /// True while the pane's log may have older content ("Show earlier"). Sources without paging say false.
    fn can_load_earlier(&self, _pane: &Pane) -> bool {
        false
    }
    /// Loads an older part of the pane's log. Returns true while older content remains.
    fn load_earlier(&self, _pane: &Pane) -> bool {
        false
    }
    /// A job that does the same as `load_earlier` but can run on a worker thread (the read is blocking file I/O).
    /// `None` means the source has no such job; the app then calls `load_earlier` on the UI thread.
    fn load_earlier_job(&self, _pane: &Pane) -> Option<EarlierJob> {
        None
    }
    /// A cheap change counter of the pane's conversation: while it is equal, the conversation did not change, and
    /// the app does not fetch or rebuild it. `None` = unknown (the app falls back to comparing the fetched tail).
    fn revision(&self, _pane: &Pane) -> Option<u64> {
        None
    }
}

/// See [`DataSource::load_earlier_job`]. Returns true while older content remains.
pub type EarlierJob = Box<dyn FnOnce() -> bool + Send + 'static>;

pub use crate::demo::DemoSource;

#[cfg(unix)]
pub use live::LiveSource;

#[cfg(unix)]
mod live {
    use super::DataSource;
    use herdr_core::transport::UnixConnector;
    use herdr_core::store::HerdrStore;
    use herdr_core::{ConversationItem, HerdrSession, Pane, Workspace};
    use std::collections::HashMap;
    use std::path::Path;

    /// The live herdr session. Read-only: `HerdrStore` sends `session.snapshot` and `events.subscribe` only.
    pub struct LiveSource {
        store: std::sync::Arc<HerdrStore<UnixConnector>>,
        label: String,
    }

    impl LiveSource {
        /// Session order: `--session NAME`, `HERDR_DESK_SESSION`, the herdr env, then the default session.
        pub fn from_args(args: &[String]) -> LiveSource {
            let env: HashMap<String, String> = std::env::vars().collect();
            let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
            let session = HerdrSession::resolve(args, &env, Path::new(&home));
            let label = session.name.clone().unwrap_or_else(|| "default".to_string());
            let store = std::sync::Arc::new(HerdrStore::new(&session));
            store.start();
            LiveSource { store, label }
        }
    }

    impl DataSource for LiveSource {
        fn conversation_state(&self, pane: &Pane) -> herdr_core::ConversationState { self.store.pane_conversation_state(pane) }
        fn log_path(&self, pane: &Pane) -> Option<std::path::PathBuf> {
            self.store.log_path(pane.agent.as_ref().and_then(|a| a.session_id.as_deref()))
        }
        fn status_tags(&self, pane: &Pane) -> Vec<herdr_core::StatusTag> {
            let Some(a) = &pane.agent else { return Vec::new() };
            let Some(id) = &a.session_id else { return Vec::new() };
            herdr_core::status_line::read_tags(&a.kind, id)
        }

        fn load_earlier_job(&self, pane: &Pane) -> Option<super::EarlierJob> {
            let id = pane.agent.as_ref().and_then(|a| a.session_id.clone())?;
            let store = self.store.clone();
            Some(Box::new(move || store.load_earlier(Some(&id), 256 * 1024)))
        }
        fn label(&self) -> String {
            self.label.clone()
        }
        fn workspaces(&self) -> Vec<Workspace> {
            self.store.workspaces()
        }
        fn connected(&self) -> bool {
            self.store.connected()
        }
        fn revision(&self, pane: &Pane) -> Option<u64> {
            self.store.conversation_revision(pane.agent.as_ref().and_then(|a| a.session_id.as_deref()))
        }
        fn conversation(&self, pane: &Pane) -> Vec<ConversationItem> {
            self.store.conversation(pane.agent.as_ref().and_then(|a| a.session_id.as_deref()))
        }
        fn can_load_earlier(&self, pane: &Pane) -> bool {
            self.store.can_load_earlier(pane.agent.as_ref().and_then(|a| a.session_id.as_deref()))
        }
        fn load_earlier(&self, pane: &Pane) -> bool {
            // Reads an older part of the session log file: no herdr call.
            self.store.load_earlier(pane.agent.as_ref().and_then(|a| a.session_id.as_deref()), 256 * 1024)
        }
    }
}
