//! Swift: `HCSession.swift`. Where herdr-core listens. Rules from upstream docs/security.md ("State and endpoints"):
//! `HC_SOCKET` overrides the endpoint; else `$XDG_RUNTIME_DIR/herdr-core/<session>.sock` when that variable is set;
//! else `/run/user/<uid>/herdr-core/` when it exists (Linux only in practice); else the session's state directory,
//! `${XDG_STATE_HOME:-~/.local/state}/herdr-core/<session>/<session>.sock`. The session name comes from
//! `HC_SESSION`, default `default`. On Windows hc listens on the named pipe
//! `\\.\pipe\herdr-core-<USERNAME>-<session>.sock` instead. Flockbook's overrides on top: `--hc-socket <path>` and
//! `HERDR_DESK_HC_SOCKET`.

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HcSession {
    pub socket_path: String,
    /// Why no herdr-core can listen for this session (hc refuses its name): never available, and every call fails
    /// with this reason. `None` for a usable session.
    pub refused: Option<String>,
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
        HcSession { socket_path: socket_path.into(), refused: None }
    }

    /// A session hc would refuse to start, with the reason.
    fn refused(why: String) -> HcSession {
        HcSession { socket_path: String::new(), refused: Some(why) }
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
            let raw = |k: &str| env.get(k).map(String::as_str);
            return match windows_pipe(raw("USERNAME"), raw("HC_SESSION")) {
                Ok(p) => HcSession::new(&p),
                Err(why) => HcSession::refused(why),
            };
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

    /// True when a socket file is there (on Windows: a server has the named pipe open). Missing is normal (hc not
    /// installed or not running), not an error.
    pub fn is_available(&self) -> bool {
        if self.refused.is_some() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt;
            std::fs::symlink_metadata(&self.socket_path).map(|m| m.file_type().is_socket()).unwrap_or(false)
        }
        #[cfg(windows)]
        {
            crate::transport::pipe_exists(&self.socket_path)
        }
        #[cfg(not(any(unix, windows)))]
        {
            false
        }
    }
}

/// The named pipe hc on Windows listens on for this `USERNAME` and `HC_SESSION` (upstream docs/security.md names
/// the Unix sockets only; hc writes the same name to the session's `core.lock`). Err: hc refuses the session name,
/// so no core can listen for it. Follows herdr-core v0.1.22, crates/herdr-core/src:
/// - `platform/windows.rs:375-379` (`endpoints`): the prefix `herdr-core-<user>-`, where `<user>` keeps only the
///   ASCII letters and digits of `USERNAME` (empty when unset).
/// - `paths.rs:83-93` (`session`): the name is trimmed, blank means `default`; `.`, `..`, control characters and
///   any of `/\:*?"<>|` are refused.
/// - `paths.rs:34-47`, `paths.rs:78`, `platform/windows.rs:373`: the longest pipe, the holder's
///   `<prefix><session>.holder.sock` plus `.` and 8 hex digits, may not pass 256 bytes.
pub fn windows_pipe(user: Option<&str>, session: Option<&str>) -> Result<String, String> {
    const MAX_PIPE: usize = 256;
    const HOLDER_ID: usize = 8;
    let user: String = user.unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let session = session.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("default");
    if session == "." || session == ".." || session.chars().any(|c| c.is_control() || r#"/\:*?"<>|"#.contains(c)) {
        return Err(format!("HC_SESSION {session:?} can't be a folder name, so herdr-core does not start with it"));
    }
    let prefix = format!(r"\\.\pipe\herdr-core-{user}-{session}");
    let longest = format!("{prefix}.holder.sock").len() + 1 + HOLDER_ID;
    if longest > MAX_PIPE {
        return Err(format!("HC_SESSION {session:?} is too long: its pipe names would pass {MAX_PIPE} bytes"));
    }
    Ok(format!("{prefix}.sock"))
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

    /// hc 0.1.22 on Windows: `\\.\pipe\herdr-core-<user>-<session>.sock`. The XDG variables do not move a pipe.
    #[cfg(windows)]
    #[test]
    fn pipe_name_rules() {
        let user = ("USERNAME", "someone");
        assert_eq!(HcSession::resolve(&env(&[user]), &[], None).socket_path, r"\\.\pipe\herdr-core-someone-default.sock");
        assert_eq!(HcSession::resolve(&env(&[user, ("HC_SESSION", "work")]), &[], None).socket_path, r"\\.\pipe\herdr-core-someone-work.sock");
        assert_eq!(HcSession::resolve(&env(&[user, ("XDG_RUNTIME_DIR", r"C:\r"), ("XDG_STATE_HOME", r"C:\s")]), &[], None).socket_path, r"\\.\pipe\herdr-core-someone-default.sock");
        assert_eq!(HcSession::resolve(&env(&[user, ("HC_SOCKET", r"\\.\pipe\h.sock")]), &[], None).socket_path, r"\\.\pipe\h.sock");
        assert_eq!(HcSession::resolve(&env(&[user, ("HC_SOCKET", r"\\.\pipe\h.sock"), ("HERDR_DESK_HC_SOCKET", r"\\.\pipe\d.sock")]), &[], None).socket_path, r"\\.\pipe\d.sock");
        assert_eq!(HcSession::resolve(&env(&[user]), &["--hc-socket".into(), r"\\.\pipe\a.sock".into()], None).socket_path, r"\\.\pipe\a.sock");
        // Empty values count as unset.
        assert_eq!(HcSession::resolve(&env(&[user, ("HC_SOCKET", ""), ("HC_SESSION", "")]), &[], None).socket_path, r"\\.\pipe\herdr-core-someone-default.sock");
    }

    /// hc 0.1.22 keeps only the ASCII letters and digits of `USERNAME`.
    #[test]
    fn pipe_user_is_ascii_alphanumeric() {
        let pipe = |u: &str| windows_pipe(Some(u), None).unwrap();
        assert_eq!(pipe("dev-user"), r"\\.\pipe\herdr-core-devuser-default.sock");
        assert_eq!(pipe("Some One"), r"\\.\pipe\herdr-core-SomeOne-default.sock");
        assert_eq!(pipe("José.Ng_2"), r"\\.\pipe\herdr-core-JosNg2-default.sock");
        assert_eq!(pipe("用戶"), r"\\.\pipe\herdr-core--default.sock");
        assert_eq!(windows_pipe(None, None).unwrap(), r"\\.\pipe\herdr-core--default.sock");
    }

    /// hc 0.1.22 trims the session name; blank means `default`.
    #[test]
    fn pipe_session_is_trimmed() {
        let pipe = |s: &str| windows_pipe(Some("someone"), Some(s)).unwrap();
        assert_eq!(pipe("work "), r"\\.\pipe\herdr-core-someone-work.sock");
        assert_eq!(pipe("  work"), r"\\.\pipe\herdr-core-someone-work.sock");
        assert_eq!(pipe("   "), r"\\.\pipe\herdr-core-someone-default.sock");
        assert_eq!(pipe("My Test"), r"\\.\pipe\herdr-core-someone-My Test.sock");
        assert_eq!(pipe("工作"), r"\\.\pipe\herdr-core-someone-工作.sock");
    }

    /// The session names hc 0.1.22 refuses to start with: no pipe is probed for them.
    #[test]
    fn pipe_session_hc_refuses() {
        for bad in ["a/b", r"a\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b", ".", "..", " .. ", "a\tb", "a\u{7f}b"] {
            let e = windows_pipe(Some("someone"), Some(bad)).unwrap_err();
            assert!(e.contains("HC_SESSION"), "{bad:?}: {e}");
        }
        // The holder's pipe, `<pipe minus .sock>.holder.<8 hex>.sock`, must fit in 256 bytes.
        let fixed = r"\\.\pipe\herdr-core-someone-".len() + ".holder.sock".len() + 1 + 8;
        let fits = "s".repeat(256 - fixed);
        assert!(windows_pipe(Some("someone"), Some(&fits)).is_ok());
        let e = windows_pipe(Some("someone"), Some(&format!("{fits}s"))).unwrap_err();
        assert!(e.contains("too long"), "{e}");
        // Bytes, not characters.
        assert!(windows_pipe(Some("someone"), Some(&"é".repeat((256 - fixed) / 2 + 1))).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn refused_session_is_never_available() {
        let s = HcSession::resolve(&env(&[("USERNAME", "someone"), ("HC_SESSION", "a/b")]), &[], None);
        assert!(s.refused.is_some());
        assert!(!s.is_available());
    }

    #[test]
    fn missing_socket_is_not_available() {
        assert!(!HcSession::new("/nonexistent/herdr-core/x.sock").is_available());
    }
}
