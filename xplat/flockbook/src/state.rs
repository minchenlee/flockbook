//! Shared app state. Every module owns one state struct in its own file and registers ONE field here.

use crate::source::DataSource;
use fb_ui::{Mode, StatusKind};
use herdr_core::{AgentStatus, ConversationItem, Pane, Tab, Workspace};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Conversation,
    Overview,
    /// Stub until step 3.
    NeedsYou,
    /// Stub until step 3.
    Search,
}

impl Screen {
    pub fn parse(s: &str) -> Option<Screen> {
        match s {
            "conversation" => Some(Screen::Conversation),
            "overview" => Some(Screen::Overview),
            "needs-you" => Some(Screen::NeedsYou),
            "search" => Some(Screen::Search),
            _ => None,
        }
    }
}

pub fn status_kind(s: AgentStatus) -> StatusKind {
    match s {
        AgentStatus::Working => StatusKind::Working,
        AgentStatus::Blocked => StatusKind::Blocked,
        AgentStatus::Done => StatusKind::Done,
        AgentStatus::Idle | AgentStatus::Unknown => StatusKind::Idle,
    }
}

/// Status counts of one workspace.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub working: usize,
    pub blocked: usize,
    pub done: usize,
    pub idle: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Rev(u64),
    Tail { len: usize, first: Option<String>, last: Option<ConversationItem> },
}

pub struct State {
    pub screen: Screen,
    pub mode: Mode,
    /// Mirror of the source's tree. Refreshed by `App::refresh`.
    pub workspaces: Vec<Workspace>,
    pub selected_workspace: Option<String>,
    pub selected_tab: Option<String>,
    pub selected_pane: Option<String>,
    /// Conversation items per pane id. Filled for the selected pane on `refresh`. Shared (`Arc`), so a search worker can
    /// hold a snapshot without copying it.
    pub conversations: HashMap<String, Arc<Vec<ConversationItem>>>,
    /// What `conversations` held when it was last fetched, per pane: the source's revision, or the tail of the items.
    seen: HashMap<String, Seen>,
    pub connected: bool,
    pub session_label: String,
    pub workspace_actions: crate::workspace_actions::WorkspaceActionsState,
    /// Only inactive reading states live here; the active state remains in `conversation`.
    pub pane_readers: HashMap<String, crate::conversation::ConversationState>,
    pub zoomed: HashMap<String, String>,
    pub pending_workspace: Option<String>,
    pub pending_tab: Option<String>,
    reader_identities: HashMap<String, (Option<herdr_core::model::AgentIdentity>, Option<std::path::PathBuf>)>,

    // One field per module. A builder adds its own state struct in its own file and registers it here.
    pub sidebar: crate::sidebar::SidebarState,
    pub toprow: crate::toprow::TopRowState,
    pub overview: crate::overview::OverviewState,
    pub needs_you: crate::needs_you::NeedsYouState,
    pub conversation: crate::conversation::ConversationState,
    pub composer: crate::composer::ComposerState,
    pub cards: crate::cards::CardsState,
    pub search: crate::search::SearchState,
    pub terminal: crate::terminal::TerminalState,
    pub glass: crate::glass::GlassState,
    pub chrome: crate::chrome::ChromeState,
    pub settings: crate::settings::SettingsState,
    pub shortcuts: crate::shortcuts::ShortcutsState,
}

impl State {
    pub fn new(mode: Mode, screen: Screen) -> State {
        State {
            screen,
            mode,
            workspaces: Vec::new(),
            selected_workspace: None,
            selected_tab: None,
            selected_pane: None,
            conversations: HashMap::new(),
            seen: HashMap::new(),
            connected: false,
            session_label: String::new(),
            workspace_actions: Default::default(),
            pane_readers: HashMap::new(),
            zoomed: HashMap::new(),
            pending_workspace: None,
            pending_tab: None,
            reader_identities: HashMap::new(),
            sidebar: Default::default(),
            toprow: Default::default(),
            overview: Default::default(),
            needs_you: Default::default(),
            conversation: Default::default(),
            composer: Default::default(),
            cards: Default::default(),
            search: Default::default(),
            terminal: Default::default(),
            glass: Default::default(),
            chrome: Default::default(),
            settings: Default::default(),
            shortcuts: Default::default(),
        }
    }

    pub fn workspace(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn selected_workspace(&self) -> Option<&Workspace> {
        self.selected_workspace.as_deref().and_then(|id| self.workspace(id))
    }

    pub fn selected_tab(&self) -> Option<&Tab> {
        let ws = self.selected_workspace()?;
        let id = self.selected_tab.as_deref()?;
        ws.tabs.iter().find(|t| t.id == id)
    }

    /// The pane the conversation shows: the selected pane, else the tab's first agent pane, else its first pane.
    pub fn selected_pane(&self) -> Option<&Pane> {
        let tab = self.selected_tab()?;
        if let Some(id) = self.selected_pane.as_deref() {
            if let Some(p) = tab.panes.iter().find(|p| p.id == id) {
                return Some(p);
            }
        }
        tab.panes.iter().find(|p| p.agent.is_some()).or_else(|| tab.panes.first())
    }

    pub fn conversation_items(&self) -> &[ConversationItem] {
        self.selected_pane().and_then(|p| self.conversations.get(&p.id)).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn select_workspace(&mut self, id: &str) {
        let Some(ws) = self.workspace(id) else { return };
        // Keep the tab when it already belongs to this workspace; else the first tab, preferring a blocked one.
        let tab = ws.tabs.iter().find(|t| t.status == AgentStatus::Blocked).or_else(|| ws.tabs.first()).map(|t| t.id.clone());
        self.selected_workspace = Some(id.to_string());
        self.selected_tab = tab;
        self.selected_pane = None;
    }

    pub fn select_tab(&mut self, ws: &str, tab: &str) {
        if self.workspace(ws).is_some_and(|w| w.tabs.iter().any(|t| t.id == tab)) {
            self.selected_workspace = Some(ws.to_string());
            self.selected_tab = Some(tab.to_string());
            self.selected_pane = None;
        }
    }

    pub fn counts(ws: &Workspace) -> Counts {
        let mut c = Counts::default();
        for t in &ws.tabs {
            match t.status {
                AgentStatus::Working => c.working += 1,
                AgentStatus::Blocked => c.blocked += 1,
                AgentStatus::Done => c.done += 1,
                _ => c.idle += 1,
            }
        }
        c
    }

    /// Tabs that need the user, as (workspace, tab).
    pub fn blocked_tabs(&self) -> Vec<(&Workspace, &Tab)> {
        self.workspaces.iter().flat_map(|w| w.tabs.iter().filter(|t| t.status == AgentStatus::Blocked).map(move |t| (w, t))).collect()
    }

    pub fn tab_count(&self) -> usize {
        self.workspaces.iter().map(|w| w.tabs.len()).sum()
    }

    /// Forgets what was fetched for `pane`, so the next `reload` fetches and reports a change.
    pub fn invalidate(&mut self, pane: &str) {
        self.seen.remove(pane);
    }

    /// Pulls the tree and the selected pane's conversation from the source. Keeps the selection when it is still valid.
    /// Returns true when the conversation of the selected pane is new or changed. An idle tick costs one
    /// `source.revision` call; a source without revisions costs one fetch plus a compare of the first id and the last
    /// item, with no hashing and no row rebuild.
    pub fn reload(&mut self, source: &dyn DataSource) -> bool {
        self.workspaces = source.workspaces();
        self.connected = source.connected();
        if let Some(id) = self.pending_workspace.clone() {
            if self.workspace(&id).is_some() { self.select_workspace(&id); self.pending_workspace = None; }
        }
        if let Some(id) = self.pending_tab.clone() {
            if let Some(ws) = self.workspaces.iter().find(|w| w.tabs.iter().any(|t| t.id == id)).map(|w| w.id.clone()) {
                self.select_tab(&ws, &id); self.pending_tab = None;
            }
        }
        let live: std::collections::HashSet<String> = self.workspaces.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).map(|p| p.id.clone()).collect();
        self.conversations.retain(|id, _| live.contains(id));
        self.seen.retain(|id, _| live.contains(id));
        self.reader_identities.retain(|id, _| live.contains(id));
        self.pane_readers.retain(|id, _| live.contains(id));
        self.zoomed.retain(|tab, pane| live.contains(pane) && self.workspaces.iter().flat_map(|w| &w.tabs).any(|t| &t.id == tab && t.panes.iter().any(|p| &p.id == pane)));
        if self.selected_pane.as_ref().is_some_and(|p| !live.contains(p)) { self.selected_pane = None; }
        let valid = self.selected_tab().is_some();
        if !valid {
            let first = self.selected_workspace.clone().filter(|id| self.workspace(id).is_some()).or_else(|| self.workspaces.first().map(|w| w.id.clone()));
            if let Some(id) = first {
                self.select_workspace(&id);
            } else {
                self.selected_workspace = None; self.selected_tab = None; self.selected_pane = None;
            }
        }
        let Some(p) = self.selected_pane().cloned() else { return false };
        self.reload_pane(source, &p)
    }

    /// Refreshes one visible split pane, sharing the same revision bookkeeping as the active reader.
    pub fn reload_pane(&mut self, source: &dyn DataSource, p: &Pane) -> bool {
        let identity = (herdr_core::model::AgentIdentity::for_pane(p), source.log_path(p));
        if self.reader_identities.get(&p.id).is_some_and(|old| old != &identity) {
            self.seen.remove(&p.id);
            if let Some(old) = self.pane_readers.remove(&p.id) {
                let mut fresh = crate::conversation::ConversationState::default();
                fresh.earlier_seq = old.earlier_seq.wrapping_add(1);
                self.pane_readers.insert(p.id.clone(), fresh);
            }
            if self.conversation.pane.as_deref() == Some(&p.id) {
                let seq = self.conversation.earlier_seq;
                self.conversation = Default::default();
                self.conversation.earlier_seq = seq.wrapping_add(1);
            }
        }
        self.reader_identities.insert(p.id.clone(), identity);
        let rev = source.revision(p);
        if let (Some(r), Some(Seen::Rev(old))) = (rev, self.seen.get(&p.id)) {
            if *old == r && self.conversations.contains_key(&p.id) {
                return false;
            }
        }
        let items = source.conversation(p);
        let now = match rev {
            Some(r) => Seen::Rev(r),
            None => Seen::Tail { len: items.len(), first: items.first().map(|i| i.id.clone()), last: items.last().cloned() },
        };
        let same = self.seen.get(&p.id) == Some(&now) && self.conversations.contains_key(&p.id);
        self.seen.insert(p.id.clone(), now);
        if same {
            return false;
        }
        self.conversations.insert(p.id.clone(), Arc::new(items));
        true
    }
}

/// Workspace monograms share the chrome's Swift-compatible rules.
pub fn monogram(label: &str) -> String {
    crate::chrome::monogram(label)
}
