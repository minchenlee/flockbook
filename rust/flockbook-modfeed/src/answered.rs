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

    /// Made before the key. False: this prompt (its id, or its tool and command) is claimed already.
    pub fn claim(&self, terminal: &str, id: &str, tool: &str, command: &str, blocked: bool, now: SystemTime) -> bool {
        let (Some(folder), Some(path)) = (self.folder(terminal), self.file(terminal, id)) else { return true };
        if self.is_claimed(terminal, id, tool, command, blocked, now) {
            return false;
        }
        let _ = std::fs::create_dir_all(&folder);
        let _ = std::fs::remove_file(&path); // not live (is_claimed above): its time is over
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                let m = Mark { id: id.into(), tool: tool.into(), command: Self::flat(command), held: false };
                let _ = f.write_all(&serde_json::to_vec(&m).unwrap_or_default());
                true
            }
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
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
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
        for (p, m) in self.marks(terminal) {
            if let (Some((tool, command)), Some(m)) = (bx, &m) {
                if m.tool == tool && m.command == Self::flat(command) {
                    continue;
                }
            }
            let _ = std::fs::remove_file(p);
        }
    }
}
