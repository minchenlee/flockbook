//! Live session tree: snapshot, event stream, reconnect and refresh timing (port of HerdrStore.swift and Preview.swift).
//!
//! Plain threads, no async runtime. Read-only against herdr: it only sends `session.snapshot` and `events.subscribe`.
//! The UI polls `workspaces()` / `connected()` or registers `set_on_change`.
//!
//! Session logs are tailed per session id (`conversation`, `conversation_state`, `load_earlier`). Left out on purpose:
//! depth/visibility trimming and the session resolver loop. The hook for the resolver is `set_resolved`: it fills `agent.session_id` where herdr gave none.

use crate::client::HerdrClient;
use crate::error::HerdrError;
use crate::model::*;
use crate::session::HerdrSession;
use crate::session_log::{ConversationLoadState, SessionLog, Snapshot};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use crate::transport::{Connector, Transport};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

/// A one-shot flag with an interruptible sleep.
pub(crate) struct Cancel {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl Cancel {
    fn new() -> Arc<Cancel> {
        Arc::new(Cancel { flag: Mutex::new(false), cv: Condvar::new() })
    }
    fn set(&self) {
        *self.flag.lock().unwrap() = true;
        self.cv.notify_all();
    }
    fn is_set(&self) -> bool {
        *self.flag.lock().unwrap()
    }
    /// Sleeps up to `d`; true when cancelled (before or during the sleep).
    fn sleep(&self, d: Duration) -> bool {
        let g = self.flag.lock().unwrap();
        let (g, _) = self.cv.wait_timeout_while(g, d, |set| !*set).unwrap();
        *g
    }
}

type CancelSlot = Arc<Mutex<Arc<Cancel>>>;

/// Wraps a connector so that a blocking read (the event stream has no timeout) wakes up every 250 ms and ends when the
/// store is stopped. The client itself needs no change.
pub struct StoppableConnector<C: Connector> {
    inner: C,
    slot: CancelSlot,
}

struct PollingTransport {
    inner: Box<dyn Transport>,
    cancel: Arc<Cancel>,
}

impl Read for PollingTransport {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.inner.read(b) {
                Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                    if self.cancel.is_set() {
                        return Ok(0);
                    }
                }
                r => return r,
            }
        }
    }
}
impl Write for PollingTransport {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.inner.write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Transport for PollingTransport {
    fn shutdown(&mut self) {
        self.inner.shutdown();
    }
}

impl<C: Connector> Connector for StoppableConnector<C> {
    fn connect(&self, timeout: Option<Duration>) -> Result<Box<dyn Transport>, HerdrError> {
        match timeout {
            Some(_) => self.inner.connect(timeout),
            None => {
                let t = self.inner.connect(Some(Duration::from_millis(250)))?;
                let cancel = self.slot.lock().unwrap().clone();
                Ok(Box::new(PollingTransport { inner: t, cancel }))
            }
        }
    }
}

/// One tailed session log: the log, its latest snapshot and the flag that ends the reader thread.
struct LogEntry {
    log: Arc<SessionLog>,
    snap: Arc<Mutex<Option<Snapshot>>>,
    /// Bumped each time the reader stores a new snapshot; a cheap change counter for the UI.
    rev: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

/// Counts a live log reader thread; decrements when the thread ends.
struct ReaderGuard(Arc<AtomicUsize>);
impl Drop for ReaderGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl LogEntry {
    fn release(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct StoreState {
    /// session id -> tailed log. Entries for sessions no pane reports any more are released on each snapshot.
    logs: HashMap<String, LogEntry>,
    /// Search root for logs; `None` = the agent's default directory.
    projects_dir: Option<PathBuf>,
    workspaces: Vec<Workspace>,
    connected: bool,
    is_preview: bool,
    /// The last snapshot as herdr sent it, before `resolved` is applied.
    last_raw: Vec<Workspace>,
    /// pane id -> log path found by the resolver.
    resolved: HashMap<String, String>,
    resolver: Option<crate::resolver::SessionResolver>,
    leads: crate::resolver::lead::LeadResolver,
    team_notes: HashMap<String, TeamNote>,
}

#[derive(Default)]
struct Sched {
    running: bool,
    again: bool,
}

struct Shared<C: Connector> {
    session: HerdrSession,
    client: HerdrClient<StoppableConnector<C>>,
    state: Mutex<StoreState>,
    slot: CancelSlot,
    sched: Mutex<Sched>,
    /// The cancel flag of the live `run` loop, if any. A stopped (cancelled) loop does not block a new `start`.
    active: Mutex<Option<Arc<Cancel>>>,
    /// Number of log reader threads alive (for tests).
    readers: Arc<AtomicUsize>,
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

pub struct HerdrStore<C: Connector + 'static> {
    shared: Arc<Shared<C>>,
}

impl<C: Connector + 'static> Shared<C> {
    fn cancel(&self) -> Arc<Cancel> {
        self.slot.lock().unwrap().clone()
    }

    fn notify(&self) {
        let cb = self.on_change.lock().unwrap().clone();
        if let Some(cb) = cb {
            cb();
        }
    }

    /// Ends the run loop and every log tailer. Safe to call twice.
    fn stop(&self) {
        let logs: Vec<LogEntry> = self.state.lock().unwrap().logs.drain().map(|(_, e)| e).collect();
        for e in logs {
            e.release();
        }
        self.cancel().set();
        self.set_connected(false);
    }

    fn set_connected(&self, v: bool) {
        let changed = {
            let mut st = self.state.lock().unwrap();
            let c = st.connected != v;
            st.connected = v;
            c
        };
        if changed {
            self.notify();
        }
    }

    fn refresh(&self) {
        if self.cancel().is_set() {
            return;
        }
        match self.client.snapshot() {
            Ok(raw) => {
                if self.cancel().is_set() {
                    return;
                }
                let was = {
                    let mut st = self.state.lock().unwrap();
                    std::mem::replace(&mut st.connected, true)
                };
                let changed = self.apply_snapshot(raw);
                if changed || !was {
                    self.notify();
                }
            }
            Err(_) => self.set_connected(false),
        }
    }

    /// True when the visible tree changed.
    fn apply_snapshot(&self, raw: Vec<Workspace>) -> bool {
        let mut st = self.state.lock().unwrap();
        st.last_raw = raw.clone();
        let claimed = raw.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).filter_map(|p| p.agent.as_ref()?.session_id.clone()).collect();
        let needs: Vec<_> = raw.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).filter_map(|p| {
            let a = p.agent.as_ref()?;
            a.session_id.is_none().then(|| crate::resolver::Need { pane_id: p.id.clone(), kind: a.kind.clone(), cwd: p.cwd.clone() })
        }).collect();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64();
        let resolver = st.resolver.get_or_insert_with(|| crate::resolver::SessionResolver::for_host(&self.session.socket_path));
        let found = resolver.resolve(&needs, &claimed, now);
        let mut resolved = st.resolved.clone();
        resolved.extend(found.into_iter().map(|(k, p)| (k, p.to_string_lossy().into_owned())));
        let mut ws = with_resolved(raw, &resolved);
        let asks: Vec<_> = ws.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).filter_map(|p| {
            let a = p.agent.as_ref()?;
            (a.kind == "claude").then(|| a.session_id.clone().map(|id| (p.id.clone(), id))).flatten()
        }).collect();
        let other_claimed = ws.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes)
            .filter_map(|p| p.agent.as_ref().filter(|a| a.kind != "claude")?.session_id.clone()).collect();
        let root = st.projects_dir.clone().unwrap_or_else(|| crate::resolver::AgentRoots::from_process().claude_projects);
        let selection = st.leads.select(&asks, &root, &other_claimed);
        let old_notes = st.team_notes.clone();
        st.team_notes.clear();
        for p in ws.iter_mut().flat_map(|w| &mut w.tabs).flat_map(|t| &mut t.panes) {
            if let Some((path, note)) = selection.get(&p.id) {
                if let Some(a) = &mut p.agent { a.session_id = path.as_ref().map(|p| p.to_string_lossy().into_owned()); }
                if let Some(note) = note { st.team_notes.insert(p.id.clone(), note.clone()); }
            }
        }
        let live: std::collections::HashSet<String> =
            ws.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).filter_map(|p| p.agent.as_ref()?.session_id.clone()).collect();
        st.logs.retain(|id, e| {
            let keep = live.contains(id);
            if !keep {
                e.release();
            }
            keep
        });
        if ws != st.workspaces {
            st.workspaces = ws;
            return true;
        }
        old_notes != st.team_notes
    }

    /// The first event refreshes at once; events that arrive while that refresh runs (or within 50 ms after it) are
    /// coalesced into one more refresh. A burst costs at most two snapshots, a single event costs no delay.
    fn schedule_refresh(self: &Arc<Self>) {
        {
            let mut s = self.sched.lock().unwrap();
            if s.running {
                s.again = true;
                return;
            }
            s.running = true;
        }
        let me = self.clone();
        thread::spawn(move || {
            me.refresh();
            loop {
                {
                    let mut s = me.sched.lock().unwrap();
                    if !(s.again && !me.cancel().is_set()) {
                        s.running = false;
                        break;
                    }
                    s.again = false;
                }
                thread::sleep(Duration::from_millis(50));
                me.refresh();
            }
        });
    }

    /// Reconnect: first retry after 100 ms, then doubling up to 1 s. A local socket connect costs microseconds, so a short
    /// cap keeps the gap after a herdr restart under about a second.
    fn run(self: Arc<Self>, cancel: Arc<Cancel>) {
        let mut delay = Duration::from_millis(100);
        while !cancel.is_set() {
            self.refresh();
            let poll_cancel = Cancel::new();
            let (me, pc) = (self.clone(), poll_cancel.clone());
            thread::spawn(move || {
                while !pc.sleep(Duration::from_secs(2)) {
                    me.refresh();
                }
            });
            if let Ok(stream) = self.client.events() {
                for r in stream {
                    match r {
                        Ok(()) => {
                            delay = Duration::from_millis(100);
                            self.schedule_refresh();
                        }
                        Err(_) => break,
                    }
                }
            }
            poll_cancel.set();
            if cancel.is_set() {
                break;
            }
            self.set_connected(false);
            cancel.sleep(delay);
            delay = (delay * 2).min(Duration::from_secs(1));
        }
        let mut a = self.active.lock().unwrap();
        if a.as_ref().map_or(false, |c| Arc::ptr_eq(c, &cancel)) {
            *a = None;
        }
    }
}

fn with_resolved(mut ws: Vec<Workspace>, resolved: &HashMap<String, String>) -> Vec<Workspace> {
    if resolved.is_empty() {
        return ws;
    }
    for w in &mut ws {
        for t in &mut w.tabs {
            for p in &mut t.panes {
                if let (Some(a), Some(path)) = (p.agent.as_mut(), resolved.get(&p.id)) {
                    if a.session_id.is_none() {
                        a.session_id = Some(path.clone());
                    }
                }
            }
        }
    }
    ws
}

impl<C: Connector + 'static> HerdrStore<C> {
    pub fn with_connector(session: HerdrSession, connector: C) -> Self {
        let slot: CancelSlot = Arc::new(Mutex::new(Cancel::new()));
        let client = HerdrClient::with_connector(StoppableConnector { inner: connector, slot: slot.clone() });
        HerdrStore {
            shared: Arc::new(Shared {
                session,
                client,
                state: Mutex::new(StoreState::default()),
                slot,
                sched: Mutex::new(Sched::default()),
                active: Mutex::new(None),
                readers: Arc::new(AtomicUsize::new(0)),
                on_change: Mutex::new(None),
            }),
        }
    }

    pub fn session(&self) -> &HerdrSession {
        &self.shared.session
    }
    pub fn client(&self) -> &HerdrClient<StoppableConnector<C>> {
        &self.shared.client
    }
    pub fn workspaces(&self) -> Vec<Workspace> {
        self.shared.state.lock().unwrap().workspaces.clone()
    }
    pub fn connected(&self) -> bool {
        self.shared.state.lock().unwrap().connected
    }

    /// Test probe: how many log reader threads are alive, and the tailed log of a session.
    #[doc(hidden)]
    pub fn log_probe(&self, session_id: &str) -> (Arc<AtomicUsize>, Option<Arc<SessionLog>>) {
        let log = self.shared.state.lock().unwrap().logs.get(session_id).map(|e| e.log.clone());
        (self.shared.readers.clone(), log)
    }

    /// Called (from a worker thread) after the tree or the connection state changed.
    pub fn set_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.shared.on_change.lock().unwrap() = Some(Arc::new(f));
    }

    pub fn start(&self) {
        if self.shared.state.lock().unwrap().is_preview {
            return;
        }
        let cancel = Cancel::new();
        {
            let mut a = self.shared.active.lock().unwrap();
            if a.as_ref().map_or(false, |c| !c.is_set()) {
                return; // already running
            }
            // Nothing live, or the old loop was stopped and is still winding down: a new generation starts now.
            *a = Some(cancel.clone());
            *self.shared.slot.lock().unwrap() = cancel.clone();
        }
        let sh = self.shared.clone();
        thread::spawn(move || sh.run(cancel));
    }

    pub fn stop(&self) {
        self.shared.stop();
    }

    /// One snapshot now (blocking).
    pub fn refresh(&self) {
        self.shared.refresh();
    }

    /// Fills `agent.session_id` from the resolver's result (pane id -> log path) where herdr gave none, and re-applies the
    /// last snapshot.
    pub fn set_resolved(&self, resolved: HashMap<String, String>) {
        let changed = {
            let mut st = self.shared.state.lock().unwrap();
            if st.resolved == resolved {
                return;
            }
            st.resolved = resolved;
            drop(st);
            let raw = self.shared.state.lock().unwrap().last_raw.clone();
            self.shared.apply_snapshot(raw)
        };
        if changed {
            self.shared.notify();
        }
    }

    /// Where session logs are searched (injected in tests). Set it before the first `conversation_state` call.
    pub fn set_projects_dir(&self, dir: PathBuf) {
        self.shared.state.lock().unwrap().projects_dir = Some(dir);
    }

    /// Starts tailing `session_id` once. Call with the state lock held.
    /// Returns true when this call created the entry.
    fn track(st: &mut StoreState, readers: &Arc<AtomicUsize>, session_id: &str) -> bool {
        if st.logs.contains_key(session_id) {
            return false;
        }
        let readers = readers.clone();
        let kind = st
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .flat_map(|t| &t.panes)
            .filter_map(|p| p.agent.as_ref())
            .find(|a| a.session_id.as_deref() == Some(session_id))
            .map(|a| a.kind.clone())
            .unwrap_or_else(|| "claude".into());
        let log = Arc::new(SessionLog::new(session_id, &kind));
        let stream = log.snapshots(st.projects_dir.as_deref(), Duration::from_millis(500), None);
        let snap = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let rev = Arc::new(AtomicU64::new(0));
        let (s2, st2, r2) = (snap.clone(), stop.clone(), rev.clone());
        readers.fetch_add(1, Ordering::SeqCst);
        let guard = ReaderGuard(readers);
        thread::spawn(move || {
            let _guard = guard;
            while !st2.load(Ordering::SeqCst) {
                match stream.recv_timeout(Duration::from_millis(250)) {
                    Ok(x) => {
                        *s2.lock().unwrap() = Some(x);
                        r2.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
            }
            // `stream` drops here: that stops the tailer thread and its file watcher.
        });
        st.logs.insert(session_id.to_string(), LogEntry { log, snap, rev, stop });
        true
    }

    /// Latest parsed conversation for a session id. Starts tailing on first call.
    pub fn conversation(&self, session_id: Option<&str>) -> Vec<ConversationItem> {
        let Some(id) = session_id else { return vec![] };
        let mut st = self.shared.state.lock().unwrap();
        if st.is_preview {
            return vec![];
        }
        if Self::track(&mut st, &self.shared.readers, id) {
            return vec![]; // tailing just started: nothing has been consumed yet
        }
        let snap = st.logs[id].snap.lock().unwrap().clone();
        snap.map(|s| s.items).unwrap_or_default()
    }

    /// Change counter of a session's conversation: equal values mean `conversation` would return the same items.
    /// `None` when the session is not tailed yet (call `conversation` to start). Never starts tailing itself.
    pub fn conversation_revision(&self, session_id: Option<&str>) -> Option<u64> {
        let st = self.shared.state.lock().unwrap();
        if st.is_preview {
            return None;
        }
        st.logs.get(session_id?).map(|e| e.rev.load(Ordering::SeqCst))
    }

    /// Whether the conversation is still loading, ready (possibly empty), has no log file, or failed. Starts tailing on
    /// first call. A `None` session id is `Missing`.
    pub fn conversation_state(&self, session_id: Option<&str>) -> ConversationLoadState {
        let mut st = self.shared.state.lock().unwrap();
        if st.is_preview {
            return ConversationLoadState::Ready;
        }
        let Some(id) = session_id else { return ConversationLoadState::Missing };
        if Self::track(&mut st, &self.shared.readers, id) {
            // Tailing just started. The state follows the tailer only once the store has consumed a snapshot, so the first
            // answer is always `Loading` (the tailer's own `Loading` then `Missing` would otherwise race this call).
            return ConversationLoadState::Loading;
        }
        let snap = st.logs[id].snap.lock().unwrap().clone();
        snap.map(|s| s.load).unwrap_or(ConversationLoadState::Loading)
    }

    pub fn can_load_earlier(&self, session_id: Option<&str>) -> bool {
        let st = self.shared.state.lock().unwrap();
        session_id.and_then(|id| st.logs.get(id)).is_some_and(|e| e.log.can_load_earlier())
    }

    pub fn log_path(&self, session_id: Option<&str>) -> Option<PathBuf> {
        let st = self.shared.state.lock().unwrap();
        st.logs.get(session_id?)?.log.path()
    }

    pub fn pane_conversation_state(&self, pane: &Pane) -> ConversationState {
        if let Some(note) = self.shared.state.lock().unwrap().team_notes.get(&pane.id).cloned() {
            return if note.lead.is_some() { ConversationState::ShowingLead(note) } else { ConversationState::WaitingForLead };
        }
        match self.conversation_state(pane.agent.as_ref().and_then(|a| a.session_id.as_deref())) {
            ConversationLoadState::Loading => ConversationState::Loading,
            ConversationLoadState::Ready => ConversationState::Ready,
            ConversationLoadState::Missing => ConversationState::Missing,
            ConversationLoadState::Failed(e) => ConversationState::Failed(e),
        }
    }

    /// Loads an older segment of that session's log. True while older content remains. False for an unknown session.
    pub fn load_earlier(&self, session_id: Option<&str>, bytes: usize) -> bool {
        let log = {
            let st = self.shared.state.lock().unwrap();
            match (st.is_preview, session_id.and_then(|id| st.logs.get(id))) {
                (false, Some(e)) => e.log.clone(),
                _ => return false,
            }
        };
        log.load_earlier(bytes)
    }

    /// Same, addressed by pane. False when the pane is unknown or has no agent session.
    pub fn load_earlier_pane(&self, pane_id: &str, bytes: usize) -> bool {
        let id = Self::session_id_for_pane(pane_id, &self.workspaces());
        self.load_earlier(id.as_deref(), bytes)
    }

    pub fn session_id_for_pane(pane_id: &str, workspaces: &[Workspace]) -> Option<String> {
        workspaces.iter().flat_map(|w| &w.tabs).flat_map(|t| &t.panes).find(|p| p.id == pane_id)?.agent.as_ref()?.session_id.clone()
    }

    /// Fixed tree, never connects. For previews and offline development.
    pub fn set_preview(&self, workspaces: Vec<Workspace>, connected: bool) {
        {
            let mut st = self.shared.state.lock().unwrap();
            st.workspaces = workspaces;
            st.connected = connected;
            st.is_preview = true;
        }
        self.shared.notify();
    }
}

#[cfg(unix)]
impl HerdrStore<crate::transport::UnixConnector> {
    pub fn new(session: &HerdrSession) -> Self {
        Self::with_connector(session.clone(), crate::transport::UnixConnector { path: session.socket_path.clone() })
    }

    fn preview_with(ws: Vec<Workspace>) -> Self {
        let s = Self::new(&HerdrSession { name: None, socket_path: String::new() });
        s.set_preview(ws, true);
        s
    }

    /// Mock tree shaped like a real snapshot.
    pub fn preview() -> Self {
        Self::preview_with(preview::workspaces())
    }

    /// Workspaces with many tabs and long names: for sidebar layout checks.
    pub fn preview_dense() -> Self {
        Self::preview_with(preview::dense())
    }

    /// Same tree with no blocked agent: the "All clear" state.
    pub fn preview_quiet() -> Self {
        Self::preview_with(preview::quiet())
    }
}

impl<C: Connector + 'static> Drop for HerdrStore<C> {
    fn drop(&mut self) {
        // No callback into a UI that is going away; stop the run loop and every log tailer and reader.
        *self.shared.on_change.lock().unwrap() = None;
        self.shared.stop();
    }
}

/// Mock data (port of Preview.swift's `PreviewData`).
pub mod preview {
    use crate::model::*;

    pub fn agent(n: &str, st: AgentStatus) -> Agent {
        Agent { name: n.into(), kind: "claude".into(), status: st, session_id: None, title: n.into() }
    }
    pub fn pane(id: &str, a: Option<Agent>) -> Pane {
        Pane { id: format!("p{id}"), terminal_id: format!("term_{id}"), cwd: "~/Orchard".into(), agent: a, label: None }
    }
    pub fn tab(id: &str, label: &str, n: i64, st: AgentStatus, panes: Vec<Pane>) -> Tab {
        Tab { id: id.into(), label: label.into(), number: n, panes, status: st }
    }

    pub fn workspaces() -> Vec<Workspace> {
        use AgentStatus::*;
        vec![
            Workspace {
                id: "w1".into(),
                label: "Orchard".into(),
                tabs: vec![
                    tab("a", "Orchestrator", 1, Idle, vec![pane("1", Some(agent("orchard-main-orch", Idle))), pane("2", None)]),
                    tab("b", "Mobile view", 2, Blocked, vec![pane("3", Some(agent("orchard-web-dev", Blocked)))]),
                    tab("c", "Core runtime", 3, Working, vec![pane("4", Some(agent("orchard-api-dev", Working)))]),
                    tab("d", "Architecture review", 4, Done, vec![pane("5", Some(agent("orchard-arch-rev", Done)))]),
                ],
            },
            Workspace {
                id: "w2".into(),
                label: "Lighthouse".into(),
                tabs: vec![
                    tab("j1", "Docs", 1, Done, vec![pane("10", Some(agent("lighthouse-docs-dev", Done)))]),
                    tab("j2", "Data import", 2, Working, vec![pane("11", Some(agent("atlas-data-dev", Working)))]),
                ],
            },
            Workspace { id: "w3".into(), label: "Harbor".into(), tabs: vec![tab("h1", "Main", 1, Working, vec![pane("12", Some(agent("harbor-ops-dev", Working)))])] },
        ]
    }

    pub fn dense() -> Vec<Workspace> {
        use AgentStatus::*;
        let sts = [Idle, Done, Working, Blocked, Done, Idle, Done, Working, Idle, Done, Idle, Done, Idle, Done];
        let w = |id: &str, name: &str, n: usize| Workspace {
            id: id.into(),
            label: name.into(),
            tabs: (0..n)
                .map(|i| {
                    let st = sts[i % sts.len()];
                    tab(&format!("{id}{i}"), &format!("Tab {i}"), i as i64 + 1, st, vec![pane(&format!("{id}{i}"), Some(agent(&format!("a-{id}{i}-dev"), st)))])
                })
                .collect(),
        };
        vec![
            w("a", "Studio · Snapshot loop", 4),
            w("b", "Orchard · All", 12),
            w("c", "Meadow · Migration", 1),
            w("d", "Lighthouse · Refactor", 7),
            w("e", "Atlas", 3),
            w("f", "Harbor · Migration", 3),
        ]
    }

    pub fn quiet() -> Vec<Workspace> {
        let mut ws = workspaces();
        for w in &mut ws {
            for t in &mut w.tabs {
                if t.status == AgentStatus::Blocked {
                    t.status = AgentStatus::Done;
                }
            }
        }
        ws
    }
}
