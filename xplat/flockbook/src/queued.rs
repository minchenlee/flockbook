//! Queue log feed and durable pull recovery. Only synthetic text belongs in tests.
use herdr_core::queued_input::ClaudeQueueLog;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeptText {
    pub id: String,
    pub pane: String,
    pub text: String,
}
#[derive(Debug)]
pub struct Journal {
    pub path: PathBuf,
}
static JOURNAL_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn read(&self) -> Result<Vec<KeptText>, String> {
        let _guard = JOURNAL_LOCK.lock().map_err(|e| e.to_string())?;
        self.read_locked()
    }
    fn read_locked(&self) -> Result<Vec<KeptText>, String> {
        match std::fs::File::open(&self.path) {
            Ok(file) => {
                let mut bytes = vec![];
                file.take((16 << 20) + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                if bytes.len() > 16 << 20 {
                    return Err("Recovery journal exceeds the safe limit".into());
                }
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.to_string()),
        }
    }
    fn write_locked(&self, items: &[KeptText]) -> Result<(), String> {
        let parent = self.path.parent().ok_or("No recovery directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("pending");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let bytes = serde_json::to_vec(items).map_err(|e| e.to_string())?;
        if bytes.len() > 16 << 20 {
            return Err("Recovery journal is full; agent input was retained".into());
        }
        let mut file = opts.open(&tmp).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        std::fs::rename(tmp, &self.path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    pub fn keep(&self, pane: &str, text: &str) -> Result<(), String> {
        let _guard = JOURNAL_LOCK.lock().map_err(|e| e.to_string())?;
        let mut items = self.read_locked()?;
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
            .to_string();
        items.push(KeptText {
            id,
            pane: pane.into(),
            text: text.into(),
        });
        self.write_locked(&items)
    }
    /// Remove only recovery entries included in the completed send, never a concurrent pull's newer text.
    pub fn sent(&self, ids: &[String]) -> Result<(), String> {
        let _guard = JOURNAL_LOCK.lock().map_err(|e| e.to_string())?;
        let mut items = self.read_locked()?;
        items.retain(|i| !ids.contains(&i.id));
        self.write_locked(&items)
    }
}
/// Incremental queue feed, one selected log cached at a time. Each call reads at most 16 MiB;
/// an initial long log catches up across polls without retaining the complete file in memory.
#[derive(Default)]
struct QueueFeed {
    path: PathBuf,
    offset: u64,
    pending: Vec<u8>,
    prefix: Vec<u8>,
    log: ClaudeQueueLog,
    modified: Option<std::time::SystemTime>,
    file_id: (u64, u64),
}
fn file_id(metadata: &std::fs::Metadata) -> (u64, u64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return (metadata.dev(), metadata.ino());
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        (0, 0)
    }
}
pub fn log_snapshot(path: Option<&Path>) -> Result<ClaudeQueueLog, String> {
    use std::io::{Seek, SeekFrom};
    let Some(path) = path else {
        return Ok(ClaudeQueueLog::default());
    };
    static FEED: std::sync::OnceLock<std::sync::Mutex<QueueFeed>> = std::sync::OnceLock::new();
    let mut feed = FEED
        .get_or_init(Default::default)
        .lock()
        .map_err(|e| e.to_string())?;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    let mut prefix = vec![];
    Read::by_ref(&mut file)
        .take(128)
        .read_to_end(&mut prefix)
        .map_err(|e| e.to_string())?;
    let modified = meta.modified().ok();
    let prefix_matches = prefix.starts_with(&feed.prefix);
    let reset = feed.path != path
        || feed.file_id != file_id(&meta)
        || meta.len() < feed.offset
        || !prefix_matches
        || (meta.len() == feed.offset && modified != feed.modified);
    if reset {
        *feed = QueueFeed {
            path: path.into(),
            prefix: prefix.clone(),
            file_id: file_id(&meta),
            ..Default::default()
        };
    }
    feed.prefix = prefix;
    feed.modified = modified;
    file.seek(SeekFrom::Start(feed.offset))
        .map_err(|e| e.to_string())?;
    let mut chunk = vec![];
    file.take(16 << 20)
        .read_to_end(&mut chunk)
        .map_err(|e| e.to_string())?;
    feed.offset += chunk.len() as u64;
    feed.pending.extend(chunk);
    if feed.pending.len() > 64 << 20 {
        *feed = QueueFeed::default();
        return Err("Queue log line exceeds the safe read limit".into());
    }
    if let Some(end) = feed.pending.iter().rposition(|b| *b == b'\n') {
        let rest = feed.pending.split_off(end + 1);
        let complete = std::mem::replace(&mut feed.pending, rest);
        for line in complete.split(|b| *b == b'\n') {
            if let Ok(value) = serde_json::from_slice(line) {
                feed.log.feed(&value);
            }
        }
    }
    if feed.offset < meta.len() {
        return Err("Queue log is catching up; exact pull is not available yet".into());
    }
    Ok(feed.log.clone())
}
