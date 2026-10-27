//! Read-only Claude lead selection. Undecided and teammate logs are never published.
use super::*;
use crate::model::TeamNote;
use std::io::{BufRead, BufReader};

#[derive(Default)]
pub struct LeadResolver {
    last: HashMap<String, PathBuf>,
    verdicts: HashMap<PathBuf, Verdict>,
    /// Log path per session id. Checked in place on each refresh; the project folders are scanned only when it is gone.
    found: HashMap<String, PathBuf>,
}
struct Verdict {
    file: u128,
    size: u64,
    modified: Option<std::time::SystemTime>,
    value: Option<bool>,
    final_verdict: bool,
}

impl LeadResolver {
    /// Returns the shown path and a note for each hidden teammate/undecided log.
    pub fn select(
        &mut self,
        asks: &[(String, String)],
        root: &Path,
        claimed: &HashSet<String>,
    ) -> HashMap<String, (Option<PathBuf>, Option<TeamNote>)> {
        let ids: HashSet<_> = asks.iter().map(|(id, _)| id.clone()).collect();
        self.last.retain(|id, _| ids.contains(id));
        let sessions: HashSet<&String> = asks.iter().map(|(_, s)| s).collect();
        self.found.retain(|s, p| sessions.contains(s) && p.is_file());
        for (_, session) in asks {
            if !self.found.contains_key(session) {
                if let Some(p) = crate::session_log::locate("claude", session, root) {
                    self.found.insert(session.clone(), p);
                }
            }
        }
        let mut leads = HashMap::new();
        for (pane, session) in asks {
            let path = self.found.get(session).cloned();
            if let Some(p) = path.filter(|p| judge(&mut self.verdicts, p) == Some(false)) {
                self.last.insert(pane.clone(), p.clone());
                leads.insert(pane.clone(), p);
            }
        }
        let mut shown = claimed.clone();
        shown.extend(leads.values().map(|p| p.to_string_lossy().into_owned()));
        let wanted: HashSet<PathBuf> = asks
            .iter()
            .filter_map(|(_, session)| self.found.get(session).cloned())
            .chain(self.last.values().cloned())
            .collect();
        self.verdicts.retain(|p, _| wanted.contains(p));
        asks.iter()
            .map(|(pane, session)| {
                if let Some(p) = leads.get(pane) {
                    return (pane.clone(), (Some(p.clone()), None));
                }
                let candidate = self.last.get(pane).cloned();
                let fallback = candidate.filter(|p| {
                    judge(&mut self.verdicts, p) == Some(false)
                        && !is_claimed(&p.to_string_lossy(), &shown)
                        && !self.last.iter().any(|(other, q)| other != pane && q == p)
                });
                if let Some(p) = &fallback {
                    shown.insert(p.to_string_lossy().into_owned());
                }
                let note = TeamNote {
                    teammate: session.clone(),
                    lead: fallback.as_ref().map(|p| p.to_string_lossy().into_owned()),
                };
                (pane.clone(), (fallback, Some(note)))
            })
            .collect()
    }
}

fn judge(cache: &mut HashMap<PathBuf, Verdict>, path: &Path) -> Option<bool> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::MetadataExt;
        ((meta.dev() as u128) << 64) | meta.ino() as u128
    };
    #[cfg(not(unix))]
    let file = meta
        .created()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let size = meta.len();
    let modified = meta.modified().ok();
    if let Some(v) = cache.get_mut(path) {
        if v.file == file
            && size >= v.size
            && ((v.final_verdict && size > v.size) || (size == v.size && modified == v.modified))
        {
            v.size = size;
            v.modified = modified;
            return v.value;
        }
    }
    let value = teammate_verdict(path, 2 << 20);
    cache.insert(
        path.to_path_buf(),
        Verdict {
            file,
            size,
            modified,
            value,
            final_verdict: value.is_some() || size >= 2 << 20,
        },
    );
    value
}

/// Bounded fresh judgment deliberately avoids keeping stale verdicts after file replacement/truncation.
/// Each line is capped at 64 KiB. Unknown or partial JSON stays hidden.
pub fn teammate_verdict(path: &Path, cap: u64) -> Option<bool> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file.take(cap));
    let mut line = Vec::new();
    loop {
        line.clear();
        // Read through a large line while retaining only a bounded prefix.
        let mut ended = false;
        while !ended {
            let chunk = reader.fill_buf().ok()?;
            if chunk.is_empty() {
                break;
            }
            let n = chunk
                .iter()
                .position(|b| *b == b'\n')
                .map(|i| {
                    ended = true;
                    i + 1
                })
                .unwrap_or(chunk.len());
            let keep = (64 * 1024usize).saturating_sub(line.len()).min(n);
            line.extend_from_slice(&chunk[..keep]);
            reader.consume(n);
        }
        if line.is_empty() {
            return None;
        }
        let v = match serde_json::from_slice::<serde_json::Value>(&line) {
            Ok(v) => v,
            Err(_) => {
                let raw = String::from_utf8_lossy(&line);
                let prefix = raw.split("\"message\":").next().unwrap_or(&raw);
                let type_re = regex::Regex::new(r#""type"\s*:\s*"([^"]+)""#).ok()?;
                let ty = type_re.captures(prefix)?;
                if !matches!(&ty[1], "user" | "assistant") {
                    if ended {
                        continue;
                    }
                    return None;
                }
                let named = |key: &str| {
                    regex::Regex::new(&format!(r#""{key}"\s*:\s*"[^"\s]"#))
                        .ok()
                        .is_some_and(|r| r.is_match(prefix))
                };
                if !prefix.contains("\"teamName\"") {
                    return Some(false);
                }
                return (named("teamName") && named("agentName")).then_some(true);
            }
        };
        if matches!(
            v.get("type").and_then(|v| v.as_str()),
            Some("user" | "assistant")
        ) {
            return Some(["teamName", "agentName"].iter().all(|k| {
                v.get(*k)
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty())
            }));
        }
        if !ended {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn teammate_and_rewritten_lead_are_judged_fresh() {
        let dir = std::env::temp_dir().join(format!("fb-lead-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synthetic.jsonl");
        std::fs::write(&path, "{\"type\":\"user\",\"message\":{}}\n").unwrap();
        assert_eq!(teammate_verdict(&path, 4096), Some(false));
        std::fs::write(
            &path,
            "{\"teamName\":\"team\",\"agentName\":\"worker\",\"type\":\"user\"}\n",
        )
        .unwrap();
        assert_eq!(teammate_verdict(&path, 4096), Some(true));
        std::fs::write(&path, "{\"type\":").unwrap();
        assert_eq!(teammate_verdict(&path, 4096), None);
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn last_lead_is_rejudged_before_fallback() {
        let root = std::env::temp_dir().join(format!("fb-lead-select-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let lead = root.join("lead.jsonl");
        let mate = root.join("mate.jsonl");
        let lead_line = "{\"type\":\"user\",\"message\":{}}\n";
        let mate_line = "{\"type\":\"user\",\"teamName\":\"team\",\"agentName\":\"worker\"}\n";
        std::fs::write(&lead, lead_line).unwrap();
        std::fs::write(&mate, mate_line).unwrap();
        let mut r = LeadResolver::default();
        let asks = |p: &Path| vec![("pane".to_string(), p.to_string_lossy().into_owned())];
        assert_eq!(
            r.select(&asks(&lead), &root, &HashSet::new())["pane"].0,
            Some(lead.clone())
        );
        assert_eq!(
            r.select(&asks(&mate), &root, &HashSet::new())["pane"].0,
            Some(lead.clone())
        );
        // Shrink, then grow: neither a cached lead nor lastLead is sufficient to publish it.
        std::fs::write(&lead, "").unwrap();
        assert!(r.select(&asks(&mate), &root, &HashSet::new())["pane"]
            .0
            .is_none());
        std::fs::write(&lead, mate_line).unwrap();
        assert!(r.select(&asks(&mate), &root, &HashSet::new())["pane"]
            .0
            .is_none());
    }
}

#[cfg(all(test, unix))]
mod locate_cache_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A refresh runs on every herdr event; scanning every project folder for each Claude pane each time cost
    /// thousands of `stat` calls a second. A found log is checked in place; the folders are scanned only when it is gone.
    #[test]
    fn a_found_log_is_checked_in_place_without_scanning_the_projects() {
        let root = std::env::temp_dir().join(format!("fb-lead-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("project")).unwrap();
        let log = root.join("project").join("abc123.jsonl");
        std::fs::write(&log, "{\"type\":\"user\",\"message\":{}}\n").unwrap();
        let asks = vec![("pane".to_string(), "abc123".to_string())];
        let mut r = LeadResolver::default();
        assert_eq!(r.select(&asks, &root, &HashSet::new())["pane"].0, Some(log.clone()));
        // The projects folder can no longer be listed; a known path still resolves.
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o100)).unwrap();
        let again = r.select(&asks, &root, &HashSet::new())["pane"].clone();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(again.0, Some(log.clone()));
        assert!(again.1.is_none(), "found as the pane's own log, not the last-lead fallback");
        // Gone: the next refresh scans again and finds nothing.
        std::fs::remove_file(&log).unwrap();
        assert!(r.select(&asks, &root, &HashSet::new())["pane"].0.is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
