//! Process-tree check and bounded cache for wrapped hc agents (`hc::mapper::AgentCheck`).

use crate::hc::mapper::{AgentCheck, WrapperAsk};
use crate::resolver::{self, Proc, ProcessSource};
use std::borrow::Cow;
use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const YES_TTL: Duration = Duration::from_secs(10);
const NO_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, PartialEq, Eq)]
struct YesKey {
    fg_pid: i32,
    created: i64,
    kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NoKey {
    fg_pid: i32,
    created: i64,
    kind: String,
    status_ms: i64,
}

enum CacheEntry {
    Yes { key: YesKey, agent_pid: i32, at: Instant },
    No { key: NoKey, at: Instant },
}

/// Checks whether a wrapper terminal's foreground tree runs the expected agent (`W5`, `W6`), caching results per
/// terminal address bounded by `kill(pid, 0)` and short TTLs.
pub struct TreeCheck {
    source: Arc<dyn ProcessSource>,
    now: Arc<dyn Fn() -> Instant + Send + Sync>,
    alive: Arc<dyn Fn(i32) -> bool + Send + Sync>,
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl TreeCheck {
    pub fn new() -> Self {
        Self::with_source(Arc::from(resolver::default_source()))
    }

    pub fn with_source(source: Arc<dyn ProcessSource>) -> Self {
        Self::with_hooks(source, Arc::new(Instant::now), Arc::new(pid_alive))
    }

    pub fn with_hooks(
        source: Arc<dyn ProcessSource>,
        now: Arc<dyn Fn() -> Instant + Send + Sync>,
        alive: Arc<dyn Fn(i32) -> bool + Send + Sync>,
    ) -> Self {
        TreeCheck { source, now, alive, entries: Mutex::new(HashMap::new()) }
    }

    fn lookup<'a>(&self, ask: &WrapperAsk, load_procs: impl FnOnce() -> Cow<'a, [Proc]>) -> Option<i32> {
        let now = (self.now)();
        let cached = {
            let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            match entries.get(&ask.address) {
                Some(CacheEntry::Yes { key, agent_pid, at })
                    if key.fg_pid == ask.fg_pid
                        && key.created == ask.created
                        && key.kind == ask.kind
                        && now.saturating_duration_since(*at) <= YES_TTL
                        && (self.alive)(*agent_pid) =>
                {
                    Some(Some(*agent_pid))
                }
                Some(CacheEntry::No { key, at })
                    if key.fg_pid == ask.fg_pid
                        && key.created == ask.created
                        && key.kind == ask.kind
                        && key.status_ms == ask.status_ms
                        && now.saturating_duration_since(*at) <= NO_TTL =>
                {
                    Some(None)
                }
                _ => None,
            }
        };
        if let Some(hit) = cached {
            return hit;
        }

        let procs = load_procs();
        let found = resolver::agent_in_tree(ask.fg_pid, &ask.kind, &procs, false)
            .filter(|agent| ask.status_ms.saturating_add(1000) >= agent.start.timestamp_millis())
            .map(|agent| agent.pid)
            .filter(|&pid| pid > 0);

        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(agent_pid) = found {
            entries.insert(
                ask.address.clone(),
                CacheEntry::Yes {
                    key: YesKey { fg_pid: ask.fg_pid, created: ask.created, kind: ask.kind.clone() },
                    agent_pid,
                    at: now,
                },
            );
        } else if !procs.is_empty() {
            entries.insert(
                ask.address.clone(),
                CacheEntry::No {
                    key: NoKey { fg_pid: ask.fg_pid, created: ask.created, kind: ask.kind.clone(), status_ms: ask.status_ms },
                    at: now,
                },
            );
        } else {
            entries.remove(&ask.address);
        }
        found
    }
}

impl Default for TreeCheck {
    fn default() -> Self {
        Self::new()
    }
}

struct TreeSnapshot<'a> {
    tree: &'a TreeCheck,
    procs: OnceCell<Vec<Proc>>,
}

impl AgentCheck for TreeSnapshot<'_> {
    fn agent_under(&self, ask: &WrapperAsk) -> Option<i32> {
        self.tree.lookup(ask, || Cow::Borrowed(self.procs.get_or_init(|| self.tree.source.processes()).as_slice()))
    }
}

impl AgentCheck for TreeCheck {
    fn agent_under(&self, ask: &WrapperAsk) -> Option<i32> {
        self.lookup(ask, || Cow::Owned(self.source.processes()))
    }

    fn snapshot(&self) -> Option<Box<dyn AgentCheck + '_>> {
        Some(Box::new(TreeSnapshot { tree: self, procs: OnceCell::new() }))
    }

    fn prune(&self, live_addresses: &[&str]) {
        let live: HashSet<&str> = live_addresses.iter().copied().collect();
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).retain(|addr, _| live.contains(addr.as_str()));
    }
}

#[cfg(unix)]
fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 probes existence for a single pid without sending a signal.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_alive(_pid: i32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hc::mapper;
    use crate::screen::all_panes;
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct CountingSource {
        procs: Mutex<Vec<Proc>>,
        reads: AtomicUsize,
    }

    impl CountingSource {
        fn new(procs: Vec<Proc>) -> Self {
            CountingSource { procs: Mutex::new(procs), reads: AtomicUsize::new(0) }
        }
        fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }
        fn set(&self, procs: Vec<Proc>) {
            *self.procs.lock().unwrap() = procs;
        }
    }

    impl ProcessSource for CountingSource {
        fn processes(&self) -> Vec<Proc> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.procs.lock().unwrap().clone()
        }
        fn open_files(&self, _: &[i32]) -> Vec<String> {
            vec![]
        }
    }

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-31T10:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn proc_(pid: i32, ppid: i32, start: DateTime<Utc>, line: &str) -> Proc {
        Proc { pid, ppid, start, line: line.into() }
    }

    fn ask(addr: &str, fg_pid: i32, status_ms: i64) -> WrapperAsk {
        WrapperAsk { address: addr.into(), fg_pid, created: 1, kind: "claude".into(), status_ms }
    }

    fn fake_clock() -> (Arc<Mutex<Duration>>, Arc<dyn Fn() -> Instant + Send + Sync>) {
        let base = Instant::now();
        let offset = Arc::new(Mutex::new(Duration::ZERO));
        let off = offset.clone();
        (offset, Arc::new(move || base + *off.lock().unwrap()))
    }

    #[test]
    fn tree_check_cache_and_batching() {
        let start = t0();
        let status_ms = start.timestamp_millis() + 2000;
        let wrap = vec![proc_(500, 1, start, "secretspec run -- claude"), proc_(501, 500, start, "claude")];
        let src = Arc::new(CountingSource::new(wrap.clone()));
        let (clock, now) = fake_clock();
        let alive_flag = Arc::new(AtomicBool::new(true));
        let af = alive_flag.clone();
        let check = TreeCheck::with_hooks(src.clone(), now, Arc::new(move |_| af.load(Ordering::SeqCst)));

        // Test 16: two calls inside 10 s, agent alive -> one table read.
        assert_eq!(check.agent_under(&ask("t1", 500, status_ms)), Some(501));
        assert_eq!(src.reads(), 1);
        *clock.lock().unwrap() = Duration::from_secs(5);
        assert_eq!(check.agent_under(&ask("t1", 500, status_ms)), Some(501));
        assert_eq!(src.reads(), 1, "inside 10 s and alive -> cached");

        // Test 18: Yes entry older than 10 s -> reads again.
        *clock.lock().unwrap() = Duration::from_secs(16);
        assert_eq!(check.agent_under(&ask("t1", 500, status_ms)), Some(501));
        assert_eq!(src.reads(), 2, "older than 10 s -> reads again");

        // Test 17: alive turns false -> next call reads again and gives None when gone.
        alive_flag.store(false, Ordering::SeqCst);
        src.set(vec![proc_(500, 1, start, "secretspec run -- claude")]);
        assert_eq!(check.agent_under(&ask("t1", 500, status_ms)), None);
        assert_eq!(src.reads(), 3, "alive false -> reads again immediately");

        // Test 19: No entry: same status_ms inside 30 s -> no new read; new status_ms -> reads at once.
        *clock.lock().unwrap() += Duration::from_secs(10);
        assert_eq!(check.agent_under(&ask("t1", 500, status_ms)), None);
        assert_eq!(src.reads(), 3, "No entry reused inside 30 s for same status_ms");

        src.set(vec![proc_(500, 1, start, "secretspec run -- claude"), proc_(502, 500, start, "claude")]);
        assert_eq!(check.agent_under(&ask("t1", 500, status_ms + 5000)), Some(502));
        assert_eq!(src.reads(), 4, "new status_ms -> reads at once");

        // Test 20: three wrapper terminals in one snapshot -> one table read.
        let batch_src = Arc::new(CountingSource::new(vec![
            proc_(100, 1, start, "secretspec run -- claude"),
            proc_(101, 100, start, "claude"),
            proc_(200, 1, start, "secretspec run -- claude"),
            proc_(201, 200, start, "claude"),
            proc_(300, 1, start, "secretspec run -- claude"),
            proc_(301, 300, start, "claude"),
        ]));
        let (_c2, now2) = fake_clock();
        let batch_check = TreeCheck::with_hooks(batch_src.clone(), now2, Arc::new(|_| true));
        let term = |addr: &str, fg_pid: i64| {
            json!({
                "address": addr,
                "argv": ["zsh"],
                "foreground": {"name": "secretspec", "argv": ["secretspec", "run", "--", "claude"], "pid": fg_pid},
                "status": {"state": "working", "app": "claude-code"},
                "status_ms": status_ms,
                "created_ms": 9
            })
        };
        let ws = mapper::workspaces_with(&[term("a", 100), term("b", 200), term("c", 300)], &batch_check);
        let panes = all_panes(&ws);
        assert_eq!(batch_src.reads(), 1, "three wrapper terminals in one snapshot -> one table read");
        assert_eq!(
            panes.iter().map(|p| p.agent.as_ref().and_then(|a| a.process.as_deref())).collect::<Vec<_>>(),
            vec![Some("100/101@9"), Some("200/201@9"), Some("300/301@9")]
        );
        // Pruning removes addresses dropped from the snapshot, so reappearing reads again.
        let _ = mapper::workspaces_with(&[term("a", 100)], &batch_check);
        assert_eq!(batch_src.reads(), 1);
        let _ = mapper::workspaces_with(&[term("a", 100), term("b", 200)], &batch_check);
        assert_eq!(batch_src.reads(), 2, "pruned address b reads again when returning");
    }

    #[test]
    fn positive_cache_invalidates_on_new_status_ms() {
        let start = t0();
        let status_ms = start.timestamp_millis() + 2000;
        let src = Arc::new(CountingSource::new(vec![
            proc_(500, 1, start, "secretspec run -- claude"),
            proc_(501, 500, start, "claude"),
        ]));
        let (clock, now) = fake_clock();
        let check = TreeCheck::with_hooks(src.clone(), now, Arc::new(|_| true));

        assert_eq!(check.agent_under(&ask("t1", 500, status_ms)), Some(501));
        assert_eq!(src.reads(), 1);

        // Within YES_TTL, a second same-kind child appears at the same depth and status_ms advances.
        *clock.lock().unwrap() = Duration::from_secs(2);
        src.set(vec![
            proc_(500, 1, start, "secretspec run -- claude"),
            proc_(501, 500, start, "claude"),
            proc_(502, 500, start, "claude"),
        ]);
        assert_eq!(
            check.agent_under(&ask("t1", 500, status_ms + 500)),
            None,
            "new status_ms must re-check tree and fail closed on ambiguity"
        );
        assert_eq!(src.reads(), 2);
    }
}
