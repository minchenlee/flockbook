//! Swift: `Answered.swift`. The approvals this sidecar answered, kept next to the cache file (`answered/`): one
//! answer per prompt. Each run starts from nothing and two views run their own sidecars, so the claim is a file made
//! with an exclusive create before the key goes out.
//!
//! A claim names its prompt by the screen id and by the tool and command (a resized pane re-wraps the box and gives it
//! another screen id, but it is the same prompt). A claim holds up to `HELD_WAIT`; a held key holds it as long as the
//! agent stays blocked. It goes only on positive evidence that its prompt left (`seen`).

use crate::protocol::fnv1a;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const HELD_WAIT: Duration = Duration::from_secs(60);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct Mark {
    id: String,
    tool: String,
    command: String,
    held: bool,
}

pub struct FeedAnswered {
    dir: Option<PathBuf>,
}

impl FeedAnswered {
    /// No cache file: nothing is kept (each press stands alone).
    pub fn new(cache: Option<&Path>) -> FeedAnswered {
        FeedAnswered { dir: cache.map(|c| c.parent().unwrap_or(Path::new("")).join("answered")) }
    }

    fn folder(&self, terminal: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(fnv1a(terminal)))
    }
    fn file(&self, terminal: &str, id: &str) -> Option<PathBuf> {
        self.folder(terminal).map(|f| f.join(fnv1a(id)))
    }

    /// The command without its whitespace: the same whatever the box's width.
    pub fn flat(command: &str) -> String {
        command.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn marks(&self, terminal: &str) -> Vec<(PathBuf, Option<Mark>)> {
        let Some(folder) = self.folder(terminal) else { return vec![] };
        let Ok(rd) = std::fs::read_dir(&folder) else { return vec![] };
        rd.flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.')) // a claim being made (`claim`)
            .map(|e| {
                let p = e.path();
                let m = std::fs::read(&p).ok().and_then(|d| serde_json::from_slice::<Mark>(&d).ok());
                (p, m)
            })
            .collect()
    }

    /// Whether the claim at `path` still holds. A held key's claim is renewed while the agent is blocked.
    fn live(path: &Path, mark: &Option<Mark>, blocked: bool, now: SystemTime) -> bool {
        if mark.as_ref().is_some_and(|m| m.held) && blocked {
            if let Ok(f) = std::fs::File::options().write(true).open(path) {
                let _ = f.set_modified(now);
            }
            return true;
        }
        let made = std::fs::metadata(path).and_then(|m| m.modified()).unwrap_or(now);
        // Swift: now - made < heldWait (a file from the future counts as live).
        match now.duration_since(made) {
            Ok(d) => d < HELD_WAIT,
            Err(_) => true,
        }
    }

    fn same(m: &Option<Mark>, id: &str, tool: &str, command: &str) -> bool {
        let Some(m) = m else { return false };
        m.id == id || (m.tool == tool && m.command == Self::flat(command))
    }

    /// A file that does not parse and is younger than this may be another run's claim mid-write (an older sidecar
    /// wrote it in two steps): it is not removed.
    const UNREAD_GRACE: Duration = Duration::from_secs(1);

    /// An unreadable file younger than `UNREAD_GRACE`.
    fn fresh_unread(path: &Path, mark: &Option<Mark>, now: SystemTime) -> bool {
        if mark.is_some() {
            return false;
        }
        let Ok(made) = std::fs::metadata(path).and_then(|m| m.modified()) else { return false };
        now.duration_since(made).map_or(true, |d| d < Self::UNREAD_GRACE)
    }

    /// Made before the key. False: this prompt (its id, or its tool and command) is claimed already.
    ///
    /// Runs under a lock file in the folder (`ClaimLock`): another run that presses at the same moment gets false.
    /// The mark is written to a temp file and then hard-linked to the claim name, so the claim never exists without
    /// its content. A file there that did not parse is removed only when older than `UNREAD_GRACE`.
    pub fn claim(&self, terminal: &str, id: &str, tool: &str, command: &str, blocked: bool, now: SystemTime) -> bool {
        let (Some(folder), Some(path)) = (self.folder(terminal), self.file(terminal, id)) else { return true };
        if self.is_claimed(terminal, id, tool, command, blocked, now) {
            return false;
        }
        let _ = std::fs::create_dir_all(&folder);
        // One claim at a time in this folder: two runs never both see "not claimed" and both write.
        // A folder that takes no lock file at all (not writable): as without a cache, the same as before.
        let _lock = match ClaimLock::take(&folder) {
            Took::Held(l) => Some(l),
            Took::Busy => return false,
            Took::Unavailable => None,
        };
        if self.is_claimed(terminal, id, tool, command, blocked, now) {
            return false;
        }
        if path.exists() {
            let mark = std::fs::read(&path).ok().and_then(|d| serde_json::from_slice::<Mark>(&d).ok());
            if Self::fresh_unread(&path, &mark, now) {
                return false;
            }
            let _ = std::fs::remove_file(&path); // not live (is_claimed above): its time is over
        }
        let m = Mark { id: id.into(), tool: tool.into(), command: Self::flat(command), held: false };
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let tmp = folder.join(format!(".claim-{}-{nanos}", std::process::id()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let written = opts.open(&tmp).and_then(|mut f| {
            use std::io::Write;
            f.write_all(&serde_json::to_vec(&m).unwrap_or_default())
        });
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
            return true; // as without a cache
        }
        let linked = std::fs::hard_link(&tmp, &path);
        let _ = std::fs::remove_file(&tmp);
        match linked {
            Ok(()) => true,
            // Another run claimed it just now; any other failure: as without a cache.
            Err(e) => e.kind() != std::io::ErrorKind::AlreadyExists,
        }
    }

    /// The key may still land (hc held it): the claim lasts while the agent stays blocked.
    pub fn hold(&self, terminal: &str, id: &str) {
        let Some(path) = self.file(terminal, id) else { return };
        let Some(mut m) = std::fs::read(&path).ok().and_then(|d| serde_json::from_slice::<Mark>(&d).ok()) else { return };
        m.held = true;
        // Atomic, as Swift's `.atomic` write: a temp file, then a rename.
        let Some(folder) = path.parent() else { return };
        let tmp = folder.join(format!(".hold-{}", std::process::id()));
        if std::fs::write(&tmp, serde_json::to_vec(&m).unwrap_or_default()).is_ok() && std::fs::rename(&tmp, &path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// The key did not go out: the prompt can be answered again.
    pub fn release(&self, terminal: &str, id: &str) {
        if let Some(p) = self.file(terminal, id) {
            let _ = std::fs::remove_file(p);
        }
    }

    /// Whether this prompt (its id, or its tool and command) is claimed.
    pub fn is_claimed(&self, terminal: &str, id: &str, tool: &str, command: &str, blocked: bool, now: SystemTime) -> bool {
        self.marks(terminal).iter().any(|(p, m)| Self::same(m, id, tool, command) && Self::live(p, m, blocked, now))
    }

    /// A screen read that worked shows `bx` (None: no approval box): the claims of other prompts go. Never called
    /// when the read failed: that is no evidence.
    pub fn seen(&self, terminal: &str, bx: Option<(&str, &str)>) {
        let now = SystemTime::now();
        for (p, m) in self.marks(terminal) {
            if Self::fresh_unread(&p, &m, now) {
                continue;
            }
            if let (Some((tool, command)), Some(m)) = (bx, &m) {
                if m.tool == tool && m.command == Self::flat(command) {
                    continue;
                }
            }
            let _ = std::fs::remove_file(p);
        }
    }
}

/// A lock file (`.lock`, exclusive create) held while one run claims. A lock older than `STALE` is from a run that
/// died mid-claim: it is removed and taken once more.
struct ClaimLock(PathBuf);

enum Took {
    Held(ClaimLock),
    /// Another run holds it now.
    Busy,
    /// The lock file cannot be made at all.
    Unavailable,
}

impl ClaimLock {
    const STALE: Duration = Duration::from_secs(5);

    fn take(folder: &Path) -> Took {
        let path = folder.join(".lock");
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Took::Held(ClaimLock(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let old = std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| SystemTime::now().duration_since(t).ok())
                        .is_some_and(|d| d >= Self::STALE);
                    if !old {
                        return Took::Busy;
                    }
                    let _ = std::fs::remove_file(&path);
                }
                Err(_) => return Took::Unavailable,
            }
        }
        Took::Busy
    }
}

impl Drop for ClaimLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
