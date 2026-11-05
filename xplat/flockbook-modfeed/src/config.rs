//! `<herdr-app home>/flockbook/config.json`: the Flockbook settings the person sets in the mod.
//! `{"check_updates": true, "status_row": "show", "remote_helper": "ask"}`; a missing file or key is the
//! default, and so is a file that is not JSON (the mod's Flockbook settings say they could not read it).
//! The install never creates or changes this file. The helper reads the keys that change what it does.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `update-check` may ask GitHub for the latest release (at most once a day).
    pub check_updates: bool,
    /// `remote_helper: "never"`: no helper on other machines, and nothing runs there.
    pub remote_never: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { check_updates: true, remote_never: false }
    }
}

/// `<home>/flockbook`: from `--home` (the mod always passes its appHome()), else the folder this helper is in
/// when it is `<home>/flockbook/bin/flockbook-modfeed`. None: no settings, the defaults apply.
pub fn dir(home: Option<&str>) -> Option<PathBuf> {
    if let Some(h) = home.filter(|h| !h.is_empty()) {
        return Some(Path::new(h).join("flockbook"));
    }
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?;
    let fb = bin.parent()?;
    (bin.file_name()? == "bin" && fb.file_name()? == "flockbook").then(|| fb.to_path_buf())
}

pub fn parse(text: &str) -> Config {
    let d = Config::default();
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(text) else { return d };
    Config {
        check_updates: m.get("check_updates").and_then(Value::as_bool).unwrap_or(d.check_updates),
        remote_never: m.get("remote_helper").and_then(Value::as_str) == Some("never"),
    }
}

pub fn load(dir: Option<&Path>) -> Config {
    dir.and_then(|d| std::fs::read_to_string(d.join("config.json")).ok()).map(|t| parse(&t)).unwrap_or_default()
}

/// What a remote command says when the person chose Never.
pub const REMOTE_OFF: &str = "Flockbook does not run its helper on other machines. To change it: Flockbook settings, Helper on other machines: Ask.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_for_a_missing_bad_or_partial_file() {
        assert_eq!(parse(""), Config::default());
        assert_eq!(parse("{ not json"), Config::default());
        assert_eq!(parse("[1]"), Config::default());
        assert_eq!(parse(r#"{"check_updates": "no"}"#), Config::default());
        assert_eq!(parse(r#"{"other": 1}"#), Config::default());
        assert_eq!(load(None), Config::default());
    }

    #[test]
    fn the_settings() {
        assert!(!parse(r#"{"check_updates": false}"#).check_updates);
        assert!(parse(r#"{"remote_helper": "never"}"#).remote_never);
        assert!(!parse(r#"{"remote_helper": "ask"}"#).remote_never);
        assert!(!parse(r#"{"remote_helper": "always"}"#).remote_never, "only ask and never exist; anything else is ask");
    }

    #[test]
    fn the_home_names_the_folder() {
        assert_eq!(dir(Some("/h")), Some(PathBuf::from("/h/flockbook")));
    }
}
