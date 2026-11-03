//! Swift: `HCSession.swift`. Where herdr-core listens. Rules from upstream docs/security.md ("State and endpoints"):
//! `HC_SOCKET` overrides the endpoint; else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock` when that variable is set;
//! else `/run/user/<uid>/herdr-core/` when it exists (Linux only in practice); else the session's state directory,
//! `${XDG_STATE_HOME:-~/.local/state}/herdr-core/<session>/<session>.sock`. The session name comes from
//! `HC_SESSION`, default `default`. Flockbook's overrides on top: `--hc-socket <path>` and `HERDR_DESK_HC_SOCKET`.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HcSession {
    pub socket_path: String,
}

/// The user's home directory: `HOME` on Unix, `USERPROFILE` on Windows. Empty when unset.
pub fn home_dir(env: &HashMap<String, String>) -> String {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env.get(key).cloned().unwrap_or_default()
}

pub fn process_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

fn non_empty<'a>(env: &'a HashMap<String, String>, k: &str) -> Option<&'a str> {
    env.get(k).map(String::as_str).filter(|s| !s.is_empty())
}

impl HcSession {
    pub fn new(socket_path: &str) -> HcSession {
        HcSession { socket_path: socket_path.into() }
    }

    /// The session from this process's environment.
    pub fn resolve_env() -> HcSession {
        Self::resolve(&process_env(), &[], run_user_dir().as_deref())
    }

    /// `run_user`: `/run/user/<uid>` when it exists on this machine (Swift checks the file system; passed in for checks).
    pub fn resolve(env: &HashMap<String, String>, arguments: &[String], run_user: Option<&str>) -> HcSession {
        if let Some(i) = arguments.iter().position(|a| a == "--hc-socket") {
            if let Some(p) = arguments.get(i + 1) {
                return HcSession::new(p);
            }
        }
        if let Some(p) = non_empty(env, "HERDR_DESK_HC_SOCKET") {
            return HcSession::new(p);
        }
        if let Some(p) = non_empty(env, "HC_SOCKET") {
            return HcSession::new(p);
        }
        let session = non_empty(env, "HC_SESSION").unwrap_or("default");
        if cfg!(windows) {
            // TODO(windows): herdr-core's endpoint on Windows is not documented upstream (docs/security.md names Unix
            // sockets only). Until it is, only an explicit HC_SOCKET / HERDR_DESK_HC_SOCKET is used; this path never
            // exists, so the sidecar answers `no-core`.
            return HcSession::new(&format!("<no herdr-core endpoint on Windows yet: set HC_SOCKET> ({session})"));
        }
        if let Some(r) = non_empty(env, "XDG_RUNTIME_DIR") {
            return HcSession::new(&format!("{r}/herdr-core/{session}.sock"));
        }
        if let Some(run) = run_user {
            return HcSession::new(&format!("{run}/herdr-core/{session}.sock"));
        }
        let state = non_empty(env, "XDG_STATE_HOME").map(String::from).unwrap_or_else(|| home_dir(env) + "/.local/state");
        HcSession::new(&format!("{state}/herdr-core/{session}/{session}.sock"))
    }

    /// True when a socket file is there. Missing is normal (hc not installed or not running), not an error.
    pub fn is_available(&self) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            std::fs::symlink_metadata(&self.socket_path).map(|m| m.file_type().is_socket()).unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            // TODO(windows): see `resolve`; no endpoint kind is known, so nothing counts as available.
            false
        }
    }
}

/// `/run/user/<uid>` when it exists.
pub fn run_user_dir() -> Option<String> {
    #[cfg(unix)]
    {
        let p = format!("/run/user/{}", unsafe { libc::getuid() });
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn env(kv: &[(&str, &str)]) -> HashMap<String, String> {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[cfg(unix)]
    #[test]
    fn socket_path_rules() {
        let home = "/home/someone";
        assert_eq!(HcSession::resolve(&env(&[("HOME", home)]), &[], None).socket_path, "/home/someone/.local/state/herdr-core/default/default.sock");
        assert_eq!(HcSession::resolve(&env(&[("HOME", home), ("HC_SESSION", "work")]), &[], None).socket_path, "/home/someone/.local/state/herdr-core/work/work.sock");
        assert_eq!(HcSession::resolve(&env(&[("HOME", home), ("XDG_STATE_HOME", "/s")]), &[], None).socket_path, "/s/herdr-core/default/default.sock");
        assert_eq!(HcSession::resolve(&env(&[("HOME", home)]), &[], Some("/run/user/501")).socket_path, "/run/user/501/herdr-core/default.sock");
        assert_eq!(HcSession::resolve(&env(&[("XDG_RUNTIME_DIR", "/r"), ("HC_SESSION", "x")]), &[], Some("/run/user/501")).socket_path, "/r/herdr-core/x.sock");
        assert_eq!(HcSession::resolve(&env(&[("XDG_RUNTIME_DIR", "/r"), ("HC_SOCKET", "/h.sock")]), &[], None).socket_path, "/h.sock");
        assert_eq!(HcSession::resolve(&env(&[("HC_SOCKET", "/h.sock"), ("HERDR_DESK_HC_SOCKET", "/d.sock")]), &[], None).socket_path, "/d.sock");
        assert_eq!(HcSession::resolve(&env(&[("HC_SOCKET", "")]), &["--hc-socket".into(), "/a.sock".into()], None).socket_path, "/a.sock");
        // Empty values count as unset.
        assert_eq!(HcSession::resolve(&env(&[("HOME", home), ("HC_SOCKET", ""), ("HC_SESSION", "")]), &[], None).socket_path, "/home/someone/.local/state/herdr-core/default/default.sock");
    }

    #[test]
    fn missing_socket_is_not_available() {
        assert!(!HcSession::new("/nonexistent/herdr-core/x.sock").is_available());
    }
}
