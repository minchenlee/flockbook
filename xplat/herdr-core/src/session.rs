//! Which herdr session to talk to, and where its socket lives (port of Session.swift, plus herdr's env order).

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HerdrSession {
    /// `None` = the default session, or a socket given by path.
    pub name: Option<String>,
    pub socket_path: String,
}

fn config_dir(home: &Path) -> String {
    format!("{}/.config/herdr", home.display())
}

impl HerdrSession {
    /// Phase-one topology writes are confined to the isolated input-test session.
    pub fn allows_workspace_writes(&self) -> bool {
        self.name.as_deref() == Some("c9-input-test")
            && self.socket_path.ends_with("/sessions/c9-input-test/herdr.sock")
    }
    pub fn default_session(home: &Path) -> Self {
        HerdrSession { name: None, socket_path: format!("{}/herdr.sock", config_dir(home)) }
    }

    pub fn named(name: &str, home: &Path) -> Self {
        HerdrSession { name: Some(name.into()), socket_path: format!("{}/sessions/{name}/herdr.sock", config_dir(home)) }
    }

    /// Resolution order: `--session NAME`, the app's own `HERDR_DESK_SESSION`, `HERDR_SOCKET_PATH`,
    /// `HERDR_SESSION`, then the default session. Empty env values count as unset.
    pub fn resolve(args: &[String], env: &HashMap<String, String>, home: &Path) -> Self {
        if let Some(i) = args.iter().position(|a| a == "--session") {
            if let Some(n) = args.get(i + 1) {
                return Self::named(n, home);
            }
        }
        let get = |k: &str| env.get(k).filter(|v| !v.is_empty());
        if let Some(n) = get("HERDR_DESK_SESSION") {
            return Self::named(n, home);
        }
        if let Some(p) = get("HERDR_SOCKET_PATH") {
            return HerdrSession { name: None, socket_path: p.clone() };
        }
        if let Some(n) = get("HERDR_SESSION") {
            return Self::named(n, home);
        }
        Self::default_session(home)
    }

    /// Resolve from the process arguments and environment.
    pub fn from_process() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let env: HashMap<String, String> = std::env::vars().collect();
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).unwrap_or_default();
        Self::resolve(&args, &env, Path::new(&home))
    }

    /// Args for the `herdr` CLI so subprocesses hit this session.
    pub fn cli_prefix(&self) -> Vec<String> {
        self.name.as_ref().map(|n| vec!["--session".into(), n.clone()]).unwrap_or_default()
    }
}
