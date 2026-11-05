//! `update-check`: is a newer Flockbook out? It asks GitHub for the latest release at most once a day (curl),
//! keeps the answer in `<home>/flockbook/update-check.json`, and compares it with this helper's version.
//! No network when the person turned it off (config.json `check_updates: false`), when this helper has no
//! release version (a build from source), or when there is no home folder to keep the answer in.

use crate::config::{self, Config};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

pub const ONE_LINE: &str = "curl -fsSL https://github.com/minchenlee/flockbook/releases/latest/download/install.sh | sh";
const LATEST: &str = "https://api.github.com/repos/minchenlee/flockbook/releases/latest";
const DAY: u64 = 24 * 60 * 60;

/// x.y.z from `v0.8.0` or `0.8.0`; anything else (a pre-release, a dev build) is None.
pub fn version(s: &str) -> Option<(u64, u64, u64)> {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let v = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
    it.next().is_none().then_some(v)
}

fn show(v: (u64, u64, u64)) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// This helper's release version: set when the release builds it (FLOCKBOOK_VERSION), else None.
/// FLOCKBOOK_UPDATE_VERSION stands in for it in tests.
pub fn own_version() -> Option<(u64, u64, u64)> {
    std::env::var("FLOCKBOOK_UPDATE_VERSION").ok().as_deref().or(option_env!("FLOCKBOOK_VERSION")).and_then(version)
}

/// The latest release's version from GitHub (FLOCKBOOK_UPDATE_URL points elsewhere, for tests).
fn fetch() -> Option<(u64, u64, u64)> {
    let url = std::env::var("FLOCKBOOK_UPDATE_URL").unwrap_or_else(|_| LATEST.to_string());
    let out = Command::new("curl")
        .args(["-fsSL", "--max-time", "5", "-H", "Accept: application/vnd.github+json", "-A", "flockbook-modfeed", &url])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    version(v.get("tag_name")?.as_str()?)
}

fn read(p: &Path) -> Value {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn write(p: &Path, v: &Value) {
    let tmp = p.with_extension("json.new");
    if std::fs::write(&tmp, serde_json::to_string_pretty(v).unwrap() + "\n").is_ok() {
        let _ = std::fs::rename(&tmp, p);
    }
}

/// The `update` line. `dir` is `<home>/flockbook`; `dismiss` hides the line for that version.
/// state: off | unknown (no release version) | no-home | error (no answer yet) | current | newer | dismissed.
pub fn check(dir: Option<&Path>, now: u64, dismiss: Option<&str>) -> Value {
    let cfg: Config = config::load(dir);
    let mine = own_version();
    let line = |state: &str, latest: Option<(u64, u64, u64)>| {
        json!({"type": "update", "state": state, "current": mine.map(show), "latest": latest.map(show), "command": ONE_LINE})
    };
    let Some(dir) = dir else { return line("no-home", None) };
    let cache = dir.join("update-check.json");
    let mut c = read(&cache);
    if let Some(d) = dismiss {
        let Some(d) = version(d) else { return line("error", None) };
        c["dismissed"] = json!(show(d));
        let _ = std::fs::create_dir_all(dir);
        write(&cache, &c);
    }
    if !cfg.check_updates {
        return line("off", None);
    }
    let Some(mine) = mine else { return line("unknown", None) };
    let cached = c.get("latest").and_then(Value::as_str).and_then(version);
    let checked = c.get("checked").and_then(Value::as_u64);
    // At most once a day, whether the last try worked or not; a clock that went back counts as a day.
    let fresh = checked.is_some_and(|t| t <= now && now - t < DAY);
    let latest = if fresh || dismiss.is_some() {
        cached
    } else {
        let got = fetch();
        c["checked"] = json!(now);
        if let Some(v) = got {
            c["latest"] = json!(show(v));
        }
        let _ = std::fs::create_dir_all(dir);
        write(&cache, &c);
        got.or(cached)
    };
    let Some(latest) = latest else { return line("error", None) };
    if latest <= mine {
        return line("current", Some(latest));
    }
    if c.get("dismissed").and_then(Value::as_str).and_then(version) == Some(latest) {
        return line("dismissed", Some(latest));
    }
    line("newer", Some(latest))
}

#[cfg(test)]
mod tests {
    use super::version;

    #[test]
    fn versions() {
        assert_eq!(version("v0.8.0"), Some((0, 8, 0)));
        assert_eq!(version("0.10.2"), Some((0, 10, 2)));
        assert_eq!(version("v1.0.0-rc1"), None);
        assert_eq!(version("0.8"), None);
        assert_eq!(version("0.8.0.1"), None);
        assert!(version("0.10.0") > version("0.9.9"));
    }
}
