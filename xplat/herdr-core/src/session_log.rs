//! Finds and tails an agent's JSONL session log (Claude Code, Codex, Cursor, pi). Port of `SessionLog.swift`.
//!
//! The tailer runs on its own thread and hands out `Snapshot`s over a channel. A file watcher (the `notify` crate)
//! wakes it at once when the log directory changes; a timer is the safety net. Dropping the stream stops the thread.

use crate::conversation::ConversationParser;
use crate::model::ConversationItem;
use notify::{RecursiveMode, Watcher};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Short mode keeps about 256 KB of tail loaded, full mode up to 2 MB.
pub const SHORT_TAIL: u64 = 256 * 1024;
pub const FULL_TAIL: u64 = 2 * 1024 * 1024;
/// Maximum byte range explicit older-history paging reparses.
pub const HISTORY_LIMIT: u64 = 64 * 1024 * 1024;

/// Load state of one conversation log, for the UI's empty and loading states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationLoadState {
    /// The first read has not finished (or tailing has not started).
    Loading,
    /// The log was read. Items may still be empty (a new session that has not written anything).
    Ready,
    /// No log file for this session id (yet). The tailer keeps looking and turns `Ready` when it appears.
    Missing,
    /// The file exists but cannot be opened.
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Conversation as logged. May end with a `Pending` permission request.
    pub items: Vec<ConversationItem>,
    /// A tool call has no result yet (running or waiting for approval).
    pub has_open_tool_use: bool,
    /// Where the first read stands. `Ready` with no items means the log exists and has nothing to show yet.
    pub load: ConversationLoadState,
}

// ---------------------------------------------------------------- locating

/// Where an agent keeps its logs (the search root for an id). Honors `CLAUDE_CONFIG_DIR`, `CODEX_HOME`,
/// `PI_CODING_AGENT_DIR` and `PI_CODING_AGENT_SESSION_DIR`. Thin wrapper over [`default_dir_in`] with the real env.
pub fn default_dir(agent: &str) -> PathBuf {
    let env: HashMap<String, String> = std::env::vars().collect();
    default_dir_in(agent, &env, &crate::resolver::home_dir())
}

/// Pure form of [`default_dir`]: the environment and home are passed in. One rule, shared with the resolver
/// (`AgentRoots::from_env`).
pub fn default_dir_in(agent: &str, env: &HashMap<String, String>, home: &Path) -> PathBuf {
    crate::resolver::AgentRoots::from_env(env, home).dir_for(agent)
}

/// An id that is safe to join onto a directory: not empty, no separators, no `..`.
fn plain_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('/') && !id.contains('\\') && !id.contains("..")
}

/// An absolute `.jsonl` path with no `..` part that exists.
fn existing_jsonl(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    let ok = p.extension().map_or(false, |e| e == "jsonl")
        && !p.components().any(|c| c == Component::ParentDir)
        && p.is_file();
    ok.then(|| p.to_path_buf())
}

fn is_abs(id: &str) -> bool {
    id.starts_with('/') || Path::new(id).is_absolute()
}

/// The log file for a session: an absolute `.jsonl` path is used as is (any agent); otherwise it is looked up by id.
pub fn locate(agent: &str, session_id: &str, dir: &Path) -> Option<PathBuf> {
    if is_abs(session_id) {
        return existing_jsonl(session_id);
    }
    match agent {
        "codex" => locate_codex(session_id, dir),
        "cursor" => locate_cursor(session_id, dir),
        "grok" => locate_grok(session_id, dir),
        _ => locate_claude(session_id, dir), // Claude Code (and, as before, any other id)
    }
}

/// Grok Build: `<sessions>/<cwd>/<id>/updates.jsonl`.
pub fn locate_grok(session_id: &str, sessions_dir: &Path) -> Option<PathBuf> {
    if !plain_id(session_id) {
        return None;
    }
    for cwd in sorted_names_desc(sessions_dir) {
        let path = sessions_dir
            .join(cwd)
            .join(session_id)
            .join("updates.jsonl");
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

fn sorted_names_desc(p: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(p)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.cmp(a));
    v
}

/// Codex: `<sessions>/YYYY/MM/DD/rollout-<time>-<id>.jsonl`. Accepts a session id or the file path itself.
pub fn locate_codex(session_id: &str, sessions_dir: &Path) -> Option<PathBuf> {
    if is_abs(session_id) {
        return existing_jsonl_plain(session_id);
    }
    if !plain_id(session_id) {
        return None;
    }
    // Newest day first: a live session is almost always in the last few day folders, so this stops early.
    let suffix = format!("-{session_id}.jsonl");
    for y in sorted_names_desc(sessions_dir) {
        let yd = sessions_dir.join(&y);
        for m in sorted_names_desc(&yd) {
            let md = yd.join(&m);
            for d in sorted_names_desc(&md) {
                let dd = md.join(&d);
                if let Some(f) = sorted_names_desc(&dd)
                    .into_iter()
                    .find(|f| f.starts_with("rollout-") && f.ends_with(&suffix))
                {
                    return Some(dd.join(f));
                }
            }
        }
    }
    None
}

/// Like the Swift `locateCodex` path branch: exists and ends in `.jsonl` (no `..` check there).
fn existing_jsonl_plain(path: &str) -> Option<PathBuf> {
    let p = Path::new(path);
    (p.is_file() && path.ends_with(".jsonl")).then(|| p.to_path_buf())
}

/// Cursor: `<projects>/*/agent-transcripts/<id>/<id>.jsonl`.
pub fn locate_cursor(session_id: &str, projects_dir: &Path) -> Option<PathBuf> {
    if !plain_id(session_id) {
        return None;
    }
    let rd = std::fs::read_dir(projects_dir).ok()?;
    for d in rd.filter_map(|e| e.ok()) {
        let p = d
            .path()
            .join("agent-transcripts")
            .join(session_id)
            .join(format!("{session_id}.jsonl"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Claude Code: `<projects>/*/<id>.jsonl`.
pub fn locate_claude(session_id: &str, projects_dir: &Path) -> Option<PathBuf> {
    if !plain_id(session_id) {
        return None;
    }
    let rd = std::fs::read_dir(projects_dir).ok()?;
    for d in rd.filter_map(|e| e.ok()) {
        let p = d.path().join(format!("{session_id}.jsonl"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

// ---------------------------------------------------------------- shared state

/// Shared between the tailer thread and `load_earlier`. Holds only byte offsets, not the lines: a re-parse reads
/// `[start, consumed)` from the file again, so memory does not grow with the raw log.
#[derive(Default)]
struct Inner {
    path: Option<PathBuf>,
    start: u64,    // file offset of the first loaded line
    consumed: u64, // file offset after the last whole line the tailer parsed
    loaded: bool,
    dirty: bool, // the range changed; the tailer must re-parse it
    want_full: bool,
    generation: u64,
}

struct Shared {
    /// Tailer threads alive for this log (tests).
    tailers: std::sync::atomic::AtomicUsize,
    inner: Mutex<Inner>,
    wake: Mutex<Option<SyncSender<()>>>,
}

impl Shared {
    fn poke(&self) {
        if let Some(w) = self.wake.lock().unwrap().as_ref() {
            let _ = w.try_send(());
        }
    }
}

pub struct SessionLog {
    pub session_id: String,
    /// `Agent.kind`: "claude", "codex", "pi", "cursor"
    pub agent: String,
    shared: Arc<Shared>,
}

/// A stream of values from the tailer. Dropping it stops the tailer thread.
pub struct LogStream<T> {
    rx: Mutex<Receiver<T>>,
    stop: Arc<AtomicBool>,
    wake: SyncSender<()>,
}

impl<T> LogStream<T> {
    /// Blocks until the next value; `None` when the tailer has ended.
    pub fn recv(&self) -> Option<T> {
        self.rx.lock().unwrap().recv().ok()
    }
    pub fn recv_timeout(&self, d: Duration) -> Result<T, RecvTimeoutError> {
        self.rx.lock().unwrap().recv_timeout(d)
    }
    pub fn try_recv(&self) -> Option<T> {
        self.rx.lock().unwrap().try_recv().ok()
    }
}

impl<T> Drop for LogStream<T> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.wake.try_send(());
    }
}

impl SessionLog {
    /// `session_id` is herdr's `agent_session.value`: an id, or (kind "path", e.g. pi) the log file path itself.
    pub fn new(session_id: &str, agent: &str) -> Self {
        SessionLog {
            session_id: session_id.to_string(),
            agent: agent.to_string(),
            shared: Arc::new(Shared {
                tailers: std::sync::atomic::AtomicUsize::new(0),
                inner: Mutex::new(Inner {
                    want_full: true,
                    ..Default::default()
                }),
                wake: Mutex::new(None),
            }),
        }
    }

    /// Test probe: number of tailer threads still running for this log.
    #[doc(hidden)]
    pub fn active_tailers(&self) -> usize {
        self.shared.tailers.load(Ordering::SeqCst)
    }

    /// The actual located log, including resolver-selected lead logs.
    pub fn path(&self) -> Option<PathBuf> {
        self.shared.inner.lock().unwrap().path.clone()
    }

    pub fn can_load_earlier(&self) -> bool {
        let g = self.shared.inner.lock().unwrap();
        g.loaded && g.start > 0 && g.consumed.saturating_sub(g.start) < HISTORY_LIMIT
    }

    pub fn is_codex(&self) -> bool {
        self.agent == "codex"
    }

    /// Pure parse for tests.
    pub fn parse<S: AsRef<str>>(lines: &[S]) -> Vec<ConversationItem> {
        crate::conversation::parse_lines(lines)
    }

    // ------------------------------------------------------------ paging

    /// Loads up to `bytes` more from before the current start, aligned to whole lines, and puts the parsed items in
    /// front of the stream (the next emission, within one poll). Tool results that sit in a later segment still match
    /// their calls, because the whole loaded range is parsed again together. Returns true while older content remains.
    /// Returns false before the first load or when nothing is left.
    pub fn load_earlier(&self, bytes: usize) -> bool {
        let (path, start, consumed, generation) = {
            let g = self.shared.inner.lock().unwrap();
            (
                if g.loaded { g.path.clone() } else { None },
                g.start,
                g.consumed,
                g.generation,
            )
        };
        let budget = HISTORY_LIMIT.saturating_sub(consumed.saturating_sub(start));
        if budget == 0 {
            return false;
        }
        let Some(path) = path else { return false };
        if start == 0 {
            return false;
        }
        let Ok(mut f) = std::fs::File::open(&path) else {
            return false;
        };
        let mut span = (bytes.max(1) as u64).min(budget);
        loop {
            let from = start.saturating_sub(span);
            // one byte before `from` tells whether `from` is already a line start
            let seek_to = if from > 0 { from - 1 } else { 0 };
            let want = (start - from) + u64::from(from > 0);
            let mut data = Vec::new();
            if f.seek(SeekFrom::Start(seek_to)).is_err()
                || (&mut f).take(want).read_to_end(&mut data).is_err()
            {
                return false;
            }
            let mut new_start = from;
            if from > 0 {
                if data.is_empty() {
                    return false;
                }
                let prev = data[0];
                let rest = &data[1..];
                if prev != b'\n' {
                    match rest.iter().position(|&b| b == b'\n') {
                        None => {
                            // one line longer than the span: widen
                            if span >= budget {
                                return false;
                            }
                            span = span.saturating_mul(2).min(budget);
                            continue;
                        }
                        Some(nl) => new_start = from + nl as u64 + 1,
                    }
                }
            }
            if new_start == start && from > 0 {
                if span >= budget {
                    return false;
                }
                span = span.saturating_mul(2).min(budget); // nothing whole in this span: widen
                continue;
            }
            let more = {
                let mut g = self.shared.inner.lock().unwrap();
                if g.generation != generation
                    || g.start != start
                    || g.path.as_ref() != Some(&path)
                    || !g.loaded
                    || g.consumed < consumed
                {
                    g.start > 0 && g.consumed.saturating_sub(g.start) < HISTORY_LIMIT
                // another load or a reset won
                } else {
                    g.start = new_start;
                    g.dirty = true;
                    new_start > 0 && g.consumed.saturating_sub(new_start) < HISTORY_LIMIT
                }
            };
            self.shared.poke();
            return more;
        }
    }

    /// Full mode keeps up to 2 MB of tail loaded; short mode keeps about 256 KB. Switching up loads earlier content in
    /// the background (items grow at the front); switching down drops the front items. Call before `snapshots` to
    /// choose the mode of the first load. New logs start in full mode.
    pub fn set_full(&self, full: bool) {
        let (path, start, consumed, loaded) = {
            let mut g = self.shared.inner.lock().unwrap();
            g.want_full = full;
            (g.path.clone(), g.start, g.consumed, g.loaded)
        };
        let Some(path) = path.filter(|_| loaded) else {
            return;
        };
        if full {
            let have = consumed.saturating_sub(start);
            if start > 0 && have < FULL_TAIL {
                let shared = self.shared.clone();
                let me = SessionLog {
                    session_id: self.session_id.clone(),
                    agent: self.agent.clone(),
                    shared,
                };
                std::thread::spawn(move || {
                    me.load_earlier((FULL_TAIL - have) as usize);
                });
            }
        } else if consumed.saturating_sub(start) > SHORT_TAIL + SHORT_TAIL / 2 {
            let Ok(mut f) = std::fs::File::open(&path) else {
                return;
            };
            let mut cur = consumed - SHORT_TAIL - 1; // offset of the byte read next
            if f.seek(SeekFrom::Start(cur)).is_err() {
                return;
            }
            let mut prev: Option<u8> = None;
            let mut found: Option<u64> = None;
            let mut buf = vec![0u8; 65536];
            'outer: while cur < consumed {
                let n = match f.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                for &b in &buf[..n] {
                    if prev == Some(b'\n') {
                        found = Some(cur);
                        break 'outer;
                    }
                    prev = Some(b);
                    cur += 1;
                }
            }
            let Some(pos) = found else { return };
            if pos <= start || pos >= consumed {
                return;
            }
            {
                let mut g = self.shared.inner.lock().unwrap();
                if g.start != start {
                    return;
                }
                g.start = pos;
                g.dirty = true;
            }
            self.shared.poke();
        }
    }

    // ------------------------------------------------------------ streams

    /// Full item list, re-sent whenever the file grows. Sends `[]` first.
    pub fn items(
        &self,
        projects_dir: Option<&Path>,
        poll_interval: Duration,
    ) -> LogStream<Vec<ConversationItem>> {
        self.spawn(projects_dir, poll_interval, None, |s| s.items)
    }

    /// The first load reads only the last `initial_tail_bytes` (default 256 KB, aligned to a line) and sends it at
    /// once; then it backfills up to 2 MB in the background (items grow at the front, ids stay the same). Passing
    /// `initial_tail_bytes` explicitly turns the backfill off. Later growth is read when the file watcher fires.
    /// `poll_interval` is only the safety-net timer.
    pub fn snapshots(
        &self,
        projects_dir: Option<&Path>,
        poll_interval: Duration,
        initial_tail_bytes: Option<u64>,
    ) -> LogStream<Snapshot> {
        self.spawn(projects_dir, poll_interval, initial_tail_bytes, |s| s)
    }

    fn spawn<T: Send + 'static>(
        &self,
        projects_dir: Option<&Path>,
        poll_interval: Duration,
        initial_tail_bytes: Option<u64>,
        map: impl Fn(Snapshot) -> T + Send + 'static,
    ) -> LogStream<T> {
        let dir = projects_dir
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_dir(&self.agent));
        let (tx, rx) = mpsc::channel();
        let (wake_tx, wake_rx) = mpsc::sync_channel::<()>(1);
        *self.shared.wake.lock().unwrap() = Some(wake_tx.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let t = Tailer {
            id: self.session_id.clone(),
            agent: self.agent.clone(),
            dir,
            shared: self.shared.clone(),
            first: initial_tail_bytes.unwrap_or(SHORT_TAIL),
            total: initial_tail_bytes.unwrap_or(FULL_TAIL),
            heartbeat: poll_interval.max(Duration::from_millis(10)),
            wake_tx: wake_tx.clone(),
            wake_rx,
            stop: stop.clone(),
            me: SessionLog {
                session_id: self.session_id.clone(),
                agent: self.agent.clone(),
                shared: self.shared.clone(),
            },
        };
        self.shared.tailers.fetch_add(1, Ordering::SeqCst);
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            struct Done(Arc<Shared>);
            impl Drop for Done {
                fn drop(&mut self) {
                    self.0.tailers.fetch_sub(1, Ordering::SeqCst);
                }
            }
            let _done = Done(shared);
            t.run(tx, map)
        });
        LogStream {
            rx: Mutex::new(rx),
            stop,
            wake: wake_tx,
        }
    }
}

// ---------------------------------------------------------------- tailer

/// File identity, to notice a replaced file (rename or delete, then create). Unix only; elsewhere it is `None`.
#[cfg(unix)]
fn file_id(m: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(m.ino())
}
#[cfg(not(unix))]
fn file_id(_: &std::fs::Metadata) -> Option<u64> {
    None
}

struct Tailer {
    id: String,
    agent: String,
    dir: PathBuf,
    shared: Arc<Shared>,
    first: u64,
    total: u64,
    heartbeat: Duration,
    wake_tx: SyncSender<()>,
    wake_rx: Receiver<()>,
    stop: Arc<AtomicBool>,
    me: SessionLog,
}

struct Armed {
    _watcher: notify::RecommendedWatcher,
    id: Option<u64>,
}

impl Tailer {
    #[allow(unused_assignments)]
    fn run<T>(self, tx: Sender<T>, map: impl Fn(Snapshot) -> T) {
        let mut parser = ConversationParser::new();
        let mut offset: u64 = 0;
        let mut leftover: Vec<u8> = Vec::new();
        let mut leftover_pos: u64 = 0; // file offset of leftover's first byte
        let mut started = false;
        let mut skip_partial = false;
        let mut backfill = false;
        let mut url: Option<PathBuf> = None;
        let mut armed: Option<Armed> = None;
        let mut misses = 0u32;
        let mut head: Vec<u8> = Vec::new(); // first bytes of the file as last seen (identity where there is no inode)
        let mut last_mtime: Option<std::time::SystemTime> = None;
        let shared = self.shared.clone();

        macro_rules! send {
            ($s:expr) => {
                if tx.send(map($s)).is_err() {
                    return;
                }
            };
        }
        macro_rules! reset {
            () => {{
                offset = 0;
                leftover = Vec::new();
                leftover_pos = 0;
                parser = ConversationParser::new();
                started = false;
                skip_partial = false;
                backfill = false;
                head = Vec::new();
                last_mtime = None;
                let mut g = shared.inner.lock().unwrap();
                g.generation = g.generation.wrapping_add(1);
                g.path = None;
                g.start = 0;
                g.consumed = 0;
                g.loaded = false;
                g.dirty = false;
            }};
        }
        reset!();
        send!(Snapshot {
            items: vec![],
            has_open_tool_use: false,
            load: ConversationLoadState::Loading
        });
        let mut reported = ConversationLoadState::Loading;
        macro_rules! report {
            ($l:expr) => {{
                let l: ConversationLoadState = $l;
                if l != reported {
                    reported = l.clone();
                    send!(Snapshot {
                        items: parser.items(),
                        has_open_tool_use: parser.has_open_tool_use(),
                        load: l
                    });
                }
            }};
        }

        while !self.stop.load(Ordering::SeqCst) {
            // The file was renamed, deleted or replaced since the last pass.
            if let (Some(u), Some(a)) = (&url, &armed) {
                let now = std::fs::metadata(u).ok();
                let gone = match &now {
                    None => true,
                    Some(m) => a.id.is_some() && file_id(m) != a.id,
                };
                if gone {
                    armed = None;
                    url = None;
                    reset!();
                    reported = ConversationLoadState::Loading; // whatever follows is sent, even an empty list
                }
            }
            if url.is_none() {
                url = locate(&self.agent, &self.id, &self.dir);
            }
            if let Some(u) = &url {
                shared.inner.lock().unwrap().path = Some(u.clone());
                if armed.is_none() {
                    armed = self.arm(u);
                }
            }
            if url.is_none() {
                report!(ConversationLoadState::Missing);
                misses += 1;
            } else {
                misses = 0;
            }
            if let Some(u) = &url {
                match std::fs::File::open(u) {
                    Err(_) => report!(ConversationLoadState::Failed(format!(
                        "cannot read {}",
                        u.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ))),
                    Ok(mut f) => {
                        let size = f.seek(SeekFrom::End(0)).unwrap_or(0);
                        let mut rewritten = size < offset; // truncated
                        if !rewritten && offset > 0 {
                            // Replaced in place with at least as many bytes: the first bytes or (without inode) the mtime changed.
                            let n = size.min(64) as usize;
                            let mut cur = vec![0u8; n];
                            let read_ok = f.seek(SeekFrom::Start(0)).is_ok()
                                && f.read_exact(&mut cur).is_ok();
                            if read_ok && cur.len() >= head.len() && cur[..head.len()] != head[..] {
                                rewritten = true;
                            }
                            let mt = f.metadata().ok().and_then(|m| m.modified().ok());
                            if armed.as_ref().map_or(false, |a| a.id.is_none()) {
                                if let (Some(old), Some(new)) = (last_mtime, mt) {
                                    if new < old {
                                        rewritten = true;
                                    }
                                }
                            }
                        }
                        if rewritten {
                            reset!();
                            reported = ConversationLoadState::Loading; // an empty file must still clear the old items
                        }
                        {
                            let n = size.min(64) as usize;
                            if n > head.len() {
                                let mut cur = vec![0u8; n];
                                if f.seek(SeekFrom::Start(0)).is_ok()
                                    && f.read_exact(&mut cur).is_ok()
                                {
                                    head = cur;
                                }
                            }
                            last_mtime = f.metadata().ok().and_then(|m| m.modified().ok());
                        }
                        if !started {
                            started = true;
                            if size > self.first {
                                offset = size - self.first;
                                skip_partial = true;
                            }
                            leftover_pos = offset;
                            if !skip_partial {
                                let mut g = shared.inner.lock().unwrap();
                                g.start = 0;
                                g.loaded = true;
                            }
                        }
                        if size > offset {
                            let mut data = Vec::new();
                            let _ = f.seek(SeekFrom::Start(offset));
                            let _ = f.read_to_end(&mut data);
                            offset += data.len() as u64;
                            leftover.extend_from_slice(&data);
                            if skip_partial {
                                // we landed mid-line: drop up to the first newline
                                if let Some(nl) = leftover.iter().position(|&b| b == b'\n') {
                                    leftover.drain(..=nl);
                                    leftover_pos += nl as u64 + 1;
                                    skip_partial = false;
                                    let p = leftover_pos;
                                    let mut g = shared.inner.lock().unwrap();
                                    g.start = p;
                                    g.consumed = p;
                                    g.loaded = true;
                                    backfill = self.total > self.first && p > 0 && g.want_full;
                                } else {
                                    leftover = Vec::new();
                                    leftover_pos = offset;
                                }
                            }
                            let mut cut = 0usize;
                            while let Some(nl) = leftover[cut..].iter().position(|&b| b == b'\n') {
                                let line = String::from_utf8_lossy(&leftover[cut..cut + nl]);
                                parser.feed_at(&line, Some(leftover_pos));
                                cut += nl + 1;
                                leftover_pos += nl as u64 + 1;
                            }
                            leftover.drain(..cut);
                            shared.inner.lock().unwrap().consumed = leftover_pos;
                            reported = ConversationLoadState::Ready;
                            send!(Snapshot {
                                items: parser.items(),
                                has_open_tool_use: parser.has_open_tool_use(),
                                load: ConversationLoadState::Ready
                            });
                        }
                        report!(ConversationLoadState::Ready); // located and read, even when the file is still empty
                        if backfill {
                            // the short first view is out; fill in the rest of the tail now
                            backfill = false;
                            self.me.load_earlier((self.total - self.first) as usize);
                        }
                        let dirty = {
                            let mut g = shared.inner.lock().unwrap();
                            if g.dirty {
                                g.dirty = false;
                                Some((g.start, g.consumed))
                            } else {
                                None
                            }
                        };
                        if let Some((from, to)) = dirty {
                            parser = ConversationParser::new();
                            let mut data = Vec::new();
                            let _ = f.seek(SeekFrom::Start(from));
                            let _ = (&mut f)
                                .take(to.saturating_sub(from))
                                .read_to_end(&mut data);
                            let mut pos = from;
                            let mut pieces: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
                            pieces.pop(); // after the last newline
                            for raw in pieces {
                                parser.feed_at(&String::from_utf8_lossy(raw), Some(pos));
                                pos += raw.len() as u64 + 1;
                            }
                            send!(Snapshot {
                                items: parser.items(),
                                has_open_tool_use: parser.has_open_tool_use(),
                                load: reported.clone()
                            });
                        }
                    }
                }
            }
            // wait: watcher event, poke, or the safety-net timer
            let interval = if armed.is_some() {
                self.heartbeat
            } else if url.is_some() {
                self.heartbeat.min(Duration::from_millis(250)) // located but no watcher: poll quickly
            } else if misses >= 10 {
                self.heartbeat.max(Duration::from_secs(5)) // a missing log is looked up rarely
            } else {
                self.heartbeat.min(Duration::from_secs(1))
            };
            match self.wake_rx.recv_timeout(interval) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    /// Watches the log's directory (not the file, so a replaced file still wakes us).
    fn arm(&self, path: &Path) -> Option<Armed> {
        let parent = path.parent()?;
        let tx = self.wake_tx.clone();
        let mut w = notify::recommended_watcher(move |_: notify::Result<notify::Event>| {
            let _ = tx.try_send(());
        })
        .ok()?;
        w.watch(parent, RecursiveMode::NonRecursive).ok()?;
        let id = std::fs::metadata(path).ok().and_then(|m| file_id(&m));
        Some(Armed { _watcher: w, id })
    }
}

#[cfg(test)]
mod paging_budget_tests {
    use super::*;
    #[test]
    fn history_budget_disables_older_reads_without_opening_a_file() {
        let log = SessionLog::new("synthetic", "claude");
        {
            let mut g = log.shared.inner.lock().unwrap();
            g.loaded = true;
            g.path = Some(PathBuf::from("synthetic.jsonl"));
            g.start = 1;
            g.consumed = HISTORY_LIMIT + 1;
        }
        assert!(!log.can_load_earlier());
        assert!(!log.load_earlier(usize::MAX));
    }
    #[test]
    fn grok_ids_cannot_escape_the_sessions_root() {
        assert!(locate_grok("../escape", Path::new("synthetic")).is_none());
        assert!(locate_grok("nested/id", Path::new("synthetic")).is_none());
    }
}
