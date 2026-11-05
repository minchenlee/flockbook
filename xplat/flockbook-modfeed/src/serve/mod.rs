//! `flockbook-modfeed serve`: the phone page (web/) and its protocol, behind Tailscale Serve. Off unless started;
//! it installs nothing (`--print launchd|systemd` prints a service file to install by hand). Guide: SERVE.md.
//!
//!   flockbook-modfeed serve --user <tailscale login> [--port 8787] [--cache <file>] [--origin https://<name>.ts.net]
//!                           [--read-only] [--home <herdr-app home>]
//!   flockbook-modfeed serve --print launchd|systemd --user <login> [the same options]

pub mod gate;
pub mod hub;
pub mod layout;
pub mod net;
pub mod push;
pub mod strip;
pub mod upload;

pub const DEFAULT_PORT: u16 = 8787;

/// The parsed options.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub user: String,
    pub port: u16,
    pub cache: Option<String>,
    pub origin: Option<String>,
    pub read_only: bool,
    pub print: Option<String>,
    /// `--home`: the herdr-app home whose Flockbook settings (`flockbook/config.json`) serve follows. None: `app_home`.
    pub home: Option<String>,
}

pub const USAGE: &str = "usage: flockbook-modfeed serve --user <tailscale login> [--port 8787] [--cache <file>] [--origin https://<name>.ts.net] [--read-only] [--home <herdr-app home>] [--print launchd|systemd]";

/// The options, or why not. There is deliberately no option for the address: it is always 127.0.0.1.
pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options { user: String::new(), port: DEFAULT_PORT, cache: None, origin: None, read_only: false, print: None, home: None };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let mut val = || -> Result<String, String> {
            i += 1;
            args.get(i).cloned().ok_or(format!("{a} needs a value"))
        };
        match a {
            "--user" => o.user = val()?.trim().to_string(),
            "--port" => o.port = val()?.parse().map_err(|_| "--port needs a number from 0 to 65535".to_string())?,
            "--cache" => o.cache = Some(val()?),
            "--origin" => {
                let v = val()?;
                let v = v.trim_end_matches('/');
                match v.strip_prefix("https://") {
                    Some(host) if !host.is_empty() && !host.contains('/') => o.origin = Some(v.to_string()),
                    _ => return Err("--origin is https://<host>[:port], nothing after it".into()),
                }
            }
            "--read-only" => o.read_only = true,
            "--home" => o.home = Some(val()?).filter(|h| !h.is_empty()),
            "--print" => {
                let v = val()?;
                if v != "launchd" && v != "systemd" {
                    return Err("--print takes launchd or systemd".into());
                }
                o.print = Some(v);
            }
            "--host" | "--bind" | "--address" | "--listen" => {
                return Err("serve listens on 127.0.0.1 only; reach it from other devices with Tailscale Serve (see SERVE.md)".into())
            }
            _ => return Err(format!("unknown option {a}")),
        }
        i += 1;
    }
    if o.user.is_empty() {
        return Err("--user <tailscale login> is needed: the one account allowed (the login `tailscale whois` shows)".into());
    }
    Ok(o)
}

/// The cache when none is given: `<home>/.flockbook/serve/cache.json` (its `answered/` holds the claims).
fn default_cache() -> Option<String> {
    let home = herdr_core::hc::session::home_dir(&herdr_core::hc::session::process_env());
    (!home.is_empty()).then(|| format!("{home}/.flockbook/serve/cache.json"))
}

/// The herdr-app home whose settings serve follows: `--home`, else `$HERDR_APP_HOME`, else the home this helper is
/// installed in (`<home>/flockbook/bin`), else `~/.config/herdr-app` (as the install picks it). None: no home known.
pub fn app_home(flag: Option<&str>, env: Option<&str>, installed: Option<&std::path::Path>, user_home: Option<&str>) -> Option<String> {
    let given = |v: Option<&str>| v.filter(|h| !h.is_empty()).map(String::from);
    given(flag)
        .or_else(|| given(env))
        .or_else(|| installed.and_then(|fb| fb.parent()).map(|h| h.to_string_lossy().into_owned()))
        .or_else(|| given(user_home).map(|h| format!("{h}/.config/herdr-app")))
}

fn args_of(o: &Options) -> Vec<String> {
    let mut a = vec!["serve".to_string(), "--user".into(), o.user.clone(), "--port".into(), o.port.to_string()];
    if let Some(c) = &o.cache {
        a.extend(["--cache".into(), c.clone()]);
    }
    if let Some(c) = &o.origin {
        a.extend(["--origin".into(), c.clone()]);
    }
    if o.read_only {
        a.push("--read-only".into());
    }
    if let Some(h) = &o.home {
        a.extend(["--home".into(), h.clone()]);
    }
    a
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A systemd argument: quoted, with `\`, `"` and `%` escaped.
fn unit_arg(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"))
}

/// A service file for `program` with these options. Printed only; the owner installs it (SERVE.md).
/// `home`: the user's home folder (launchd writes the log to `~/Library/Logs`; systemd keeps it in the journal).
pub fn service(kind: &str, program: &str, home: &str, o: &Options) -> String {
    let args = args_of(o);
    if kind == "launchd" {
        let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n");
        s += "  <key>Label</key><string>dev.flockbook.serve</string>\n  <key>ProgramArguments</key>\n  <array>\n";
        for a in std::iter::once(program.to_string()).chain(args) {
            s += &format!("    <string>{}</string>\n", xml(&a));
        }
        s += "  </array>\n  <key>RunAtLoad</key><true/>\n  <key>KeepAlive</key><true/>\n";
        s += &format!("  <key>StandardErrorPath</key><string>{}</string>\n</dict>\n</plist>\n", xml(&format!("{home}/Library/Logs/flockbook-modfeed-serve.log")));
        return s;
    }
    let line: Vec<String> = std::iter::once(program.to_string()).chain(args).map(|a| unit_arg(&a)).collect();
    format!(
        "[Unit]\nDescription=Flockbook phone page (behind Tailscale Serve)\nAfter=network.target\n\n[Service]\nExecStart={}\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n",
        line.join(" ")
    )
}

/// Runs `serve`; the exit code.
pub fn main(args: &[String]) -> i32 {
    let mut o = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("serve: {e}\n{USAGE}");
            return 2;
        }
    };
    if let Some(kind) = o.print.clone() {
        let program = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "flockbook-modfeed".into());
        let home = herdr_core::hc::session::home_dir(&herdr_core::hc::session::process_env());
        if kind == "launchd" && home.is_empty() {
            eprintln!("serve: HOME is not set (the log goes to ~/Library/Logs)");
            return 1;
        }
        print!("{}", service(&kind, &program, &home, &o));
        return 0;
    }
    let user_home = herdr_core::hc::session::home_dir(&herdr_core::hc::session::process_env());
    o.home = app_home(o.home.as_deref(), std::env::var("HERDR_APP_HOME").ok().as_deref(), crate::config::dir(None).as_deref(), Some(&user_home));
    if o.cache.is_none() {
        o.cache = default_cache();
    }
    if let Some(dir) = o.cache.as_deref().and_then(|c| std::path::Path::new(c).parent()) {
        let _ = std::fs::create_dir_all(dir);
    }
    let listener = match net::bind(o.port) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("serve: cannot listen on 127.0.0.1:{}: {e}", o.port);
            return 1;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(o.port);
    eprintln!("serve: listening on http://127.0.0.1:{port} for {} (Tailscale Serve only; Funnel is refused)", o.user);
    // Notifications from serve's own loop, page open or not (`push`). Off, with the reason, when the files are not right.
    if let Some(h) = o.home.clone() {
        let (read, dir) = (h.clone(), push::folder(std::path::Path::new(&h)));
        let post = move |e: &str, hs: &[String], b: &[u8]| push::curl_post(&dir, e, hs, b);
        match push::start(std::path::Path::new(&h), std::time::Duration::from_secs(3), Box::new(post), move || hub::workspaces(Some(&read))) {
            Ok(()) => eprintln!("serve: notifications ready"),
            Err(e) => eprintln!("serve: notifications off: {e}"),
        }
    }
    let settings = net::Settings {
        user: o.user.clone(),
        origin: o.origin.clone(),
        hub: hub::Config { cache: o.cache.clone(), read_only: o.read_only, hub: "Flockbook".into(), home: o.home.clone() },
    };
    net::serve(listener, settings);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn options() {
        let o = parse(&a(&["--user", "me@example.com"])).unwrap();
        assert_eq!((o.port, o.read_only, o.origin.as_deref()), (DEFAULT_PORT, false, None));
        assert!(parse(&a(&[])).is_err(), "--user is needed");
        assert!(parse(&a(&["--user", " "])).is_err());
        for bad in ["--host", "--bind", "--address", "--listen"] {
            let e = parse(&a(&["--user", "me@example.com", bad, "0.0.0.0"])).unwrap_err();
            assert!(e.contains("127.0.0.1 only"), "{e}");
        }
        assert!(parse(&a(&["--user", "me@example.com", "--origin", "http://x.ts.net"])).is_err());
        assert!(parse(&a(&["--user", "me@example.com", "--origin", "https://x.ts.net/a"])).is_err());
        assert_eq!(parse(&a(&["--user", "me@example.com", "--origin", "https://x.ts.net/"])).unwrap().origin.as_deref(), Some("https://x.ts.net"));
        assert!(parse(&a(&["--user", "me@example.com", "--print", "cron"])).is_err());
    }

    #[test]
    fn service_files() {
        let o = parse(&a(&["--user", "me@example.com", "--port", "9000", "--read-only"])).unwrap();
        let l = service("launchd", "/opt/fb/flockbook-modfeed", "/h/me", &o);
        assert!(l.contains("<key>StandardErrorPath</key><string>/h/me/Library/Logs/flockbook-modfeed-serve.log</string>"));
        assert!(!l.contains("/tmp"), "no log in /tmp");
        assert!(l.contains("<string>/opt/fb/flockbook-modfeed</string>\n    <string>serve</string>"));
        assert!(l.contains("<string>me@example.com</string>") && l.contains("<string>9000</string>") && l.contains("<string>--read-only</string>"));
        let s = service("systemd", "/opt/fb/flockbook-modfeed", "/home/me", &o);
        assert!(!s.contains("StandardError") && !s.contains("/tmp"), "systemd: the journal");
        assert!(s.contains("ExecStart=\"/opt/fb/flockbook-modfeed\" \"serve\" \"--user\" \"me@example.com\" \"--port\" \"9000\" \"--read-only\""));
        assert!(!s.contains("0.0.0.0") && !l.contains("0.0.0.0"));
    }

    #[test]
    fn the_herdr_app_home() {
        let fb = std::path::Path::new("/opt/h/flockbook");
        assert_eq!(app_home(Some("/given"), Some("/env"), Some(fb), Some("/home/you")).as_deref(), Some("/given"));
        assert_eq!(app_home(None, Some("/env"), Some(fb), Some("/home/you")).as_deref(), Some("/env"));
        assert_eq!(app_home(Some(""), Some(""), Some(fb), Some("/home/you")).as_deref(), Some("/opt/h"), "the home the helper is installed in");
        assert_eq!(app_home(None, None, None, Some("/home/you")).as_deref(), Some("/home/you/.config/herdr-app"));
        assert_eq!(app_home(None, None, None, Some("")), None);
        assert_eq!(parse(&a(&["--user", "me@x", "--home", "/h"])).unwrap().home.as_deref(), Some("/h"));
        assert!(args_of(&parse(&a(&["--user", "me@x", "--home", "/h"])).unwrap()).ends_with(&a(&["--home", "/h"])));
    }
}
