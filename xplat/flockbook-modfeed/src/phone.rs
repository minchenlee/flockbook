//! `phone status|on|off`: the phone page (`serve`) as a user service, set from the mod's Flockbook settings.
//!
//! On writes the service file `serve --print` makes, with a marker line and the app's herdr-core session, and
//! loads it: a launchd agent on macOS (`~/Library/LaunchAgents/dev.flockbook.serve.plist`), a systemd user unit
//! on Linux (`~/.config/systemd/user/flockbook-serve.service`). It keeps running after Herdr app quits and
//! starts again at login. Off stops it and removes that one file, only when it has the marker: a service set
//! up by hand (SERVE.md) is never changed. Tailscale is only read (`tailscale status`, `tailscale serve
//! status`); its settings are never changed.
//!
//! The settings, in `<home>/flockbook/config.json`: `phone_user` (empty: the Tailscale login found),
//! `phone_port` (8787), `phone_read_only` (false). On or off is the file itself, not a setting.
//! Tests: FLOCKBOOK_PHONE_BIN is the only folder where `tailscale`, `launchctl`, `systemctl` and `id` are
//! looked up, and FLOCKBOOK_PHONE_SERVICE picks launchd or systemd.

use crate::config::Config;
use crate::serve;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const MARKER: &str = "Made by Flockbook settings: Flockbook settings, Phone page: Off removes this file.";
const LABEL: &str = "dev.flockbook.serve";
const UNIT: &str = "flockbook-serve";
const BOOTSTRAP_TRIES: u32 = 8;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Launchd,
    Systemd,
}

pub struct Env {
    pub kind: Kind,
    /// The user's home folder (HOME).
    pub user_home: PathBuf,
    /// Where the fakes are, in tests.
    bin: Option<PathBuf>,
}

impl Env {
    pub fn from_process() -> Env {
        let kind = match std::env::var("FLOCKBOOK_PHONE_SERVICE").as_deref() {
            Ok("launchd") => Kind::Launchd,
            Ok("systemd") => Kind::Systemd,
            _ if cfg!(target_os = "macos") => Kind::Launchd,
            _ => Kind::Systemd,
        };
        let home = herdr_core::hc::session::home_dir(&herdr_core::hc::session::process_env());
        Env { kind, user_home: PathBuf::from(home), bin: std::env::var_os("FLOCKBOOK_PHONE_BIN").map(PathBuf::from) }
    }

    pub fn file(&self) -> PathBuf {
        match self.kind {
            Kind::Launchd => self.user_home.join("Library/LaunchAgents").join(format!("{LABEL}.plist")),
            Kind::Systemd => {
                let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty() && self.bin.is_none()).map(PathBuf::from);
                config.unwrap_or_else(|| self.user_home.join(".config")).join("systemd/user").join(format!("{UNIT}.service"))
            }
        }
    }

    fn program(&self, name: &str) -> PathBuf {
        tool(name, self.bin.as_deref(), |p| p.exists())
    }

    /// Runs a tool: (exit code, stdout, stderr); None when it cannot start.
    fn run(&self, name: &str, args: &[&str]) -> Option<(i32, String, String)> {
        let mut c = Command::new(self.program(name));
        c.args(args).stdin(Stdio::null());
        let o = match c.output() {
            Ok(o) => o,
            // The Mac app's command line, when `tailscale` is not on PATH.
            Err(_) if name == "tailscale" && self.bin.is_none() && cfg!(target_os = "macos") => {
                Command::new("/Applications/Tailscale.app/Contents/MacOS/Tailscale").args(args).stdin(Stdio::null()).output().ok()?
            }
            Err(_) => return None,
        };
        Some((o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned()))
    }

    /// The user's id, for `gui/<uid>`; an error, never a guess, when `id -u` fails.
    fn uid(&self) -> Result<String, (&'static str, String)> {
        match self.run("id", &["-u"]) {
            Some((0, out, _)) if out.trim().chars().all(|c| c.is_ascii_digit()) && !out.trim().is_empty() => Ok(out.trim().to_string()),
            _ => Err(("service", "cannot find the user id (id -u failed)".into())),
        }
    }
}

/// Where a tool is: in `bin` (the tests' fakes), else the service tools at their fixed system paths, not by PATH
/// (a folder early in PATH could hold another `launchctl`). `tailscale` has none: PATH, then the Mac app.
fn tool(name: &str, bin: Option<&Path>, exists: impl Fn(&Path) -> bool) -> PathBuf {
    if let Some(b) = bin {
        return b.join(name);
    }
    let fixed: &[&str] = match name {
        "launchctl" => &["/bin/launchctl"],
        "id" => &["/usr/bin/id", "/bin/id"],
        "systemctl" => &["/usr/bin/systemctl", "/bin/systemctl"],
        _ => &[],
    };
    fixed.iter().map(PathBuf::from).find(|p| exists(p)).unwrap_or_else(|| PathBuf::from(name))
}

/// The Tailscale login and machine name, or why not.
pub fn tailscale(env: &Env) -> Result<(String, String), String> {
    let Some((code, out, _)) = env.run("tailscale", &["status", "--json"]) else {
        return Err("Tailscale is not installed.".into());
    };
    let v: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    if code != 0 || v.get("BackendState").and_then(Value::as_str) != Some("Running") {
        return Err("Tailscale is not running or not signed in.".into());
    }
    let me = &v["Self"];
    let id = match &me["UserID"] {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    };
    let login = v["User"][&id]["LoginName"].as_str().unwrap_or("").to_string();
    let dns = me["DNSName"].as_str().unwrap_or("").trim_end_matches('.').to_string();
    if login.is_empty() {
        return Err("Tailscale did not say which account is signed in.".into());
    }
    Ok((login, dns))
}

/// Tailscale Serve for this port: the https address when it is set, and else the command that sets it.
pub struct ServeStep {
    pub url: Option<String>,
    pub command: String,
    /// Why the command is not the plain one: 443 "/" goes to another local service.
    pub reason: Option<String>,
}

/// HTTPS ports to use, in order, when 443 "/" is another service's.
const OTHER_HTTPS: [u16; 4] = [8443, 10443, 18443, 28443];

/// Reads `tailscale serve status --json` only. With 443 "/" already used by something that is not this page,
/// `tailscale serve --bg <port>` would replace it, so the command names the first free HTTPS port instead.
pub fn serve_step(env: &Env, port: u16) -> ServeStep {
    let plain = ServeStep { url: None, command: format!("tailscale serve --bg {port}"), reason: None };
    let Some((0, out, _)) = env.run("tailscale", &["serve", "status", "--json"]) else { return plain };
    let Ok(v) = serde_json::from_str::<Value>(&out) else { return plain };
    let ours = |proxy: &str| {
        let p = proxy.trim_end_matches('/');
        [format!("127.0.0.1:{port}"), format!("localhost:{port}")].iter().any(|t| p.ends_with(t.as_str())) || p == port.to_string()
    };
    let web = v.get("Web").and_then(Value::as_object);
    for (host, w) in web.into_iter().flatten() {
        let handlers = w.get("Handlers").and_then(Value::as_object);
        if handlers.is_some_and(|h| h.values().any(|x| x.get("Proxy").and_then(Value::as_str).is_some_and(ours))) {
            let host = host.strip_suffix(":443").unwrap_or(host);
            return ServeStep { url: Some(format!("https://{host}/")), ..plain };
        }
    }
    let on_port = |n: u16| {
        let suffix = format!(":{n}");
        let tcp = v.get("TCP").and_then(|t| t.get(n.to_string())).is_some();
        let web = web.into_iter().flatten().filter(|(h, _)| h.ends_with(&suffix)).map(|(_, w)| w).collect::<Vec<_>>();
        (tcp, web)
    };
    let (_, on_443) = on_port(443);
    if !on_443.iter().any(|w| w.get("Handlers").and_then(|h| h.get("/")).is_some()) {
        return plain;
    }
    let free = OTHER_HTTPS.into_iter().find(|&n| {
        let (tcp, web) = on_port(n);
        !tcp && web.is_empty()
    });
    let Some(n) = free else {
        return ServeStep { reason: Some("443 is used by another service, and so are 8443, 10443, 18443 and 28443. Pick a free HTTPS port for --https.".into()), command: format!("tailscale serve --bg --https=<port> {port}"), ..plain };
    };
    ServeStep {
        command: format!("tailscale serve --bg --https={n} {port}"),
        reason: Some(format!("443 is used by another service on this machine, so the page goes on {n}; the plain command would replace it.")),
        ..plain
    }
}

/// A QR code of `text` as an SVG: one path of the dark modules, with a 4-module margin.
pub fn qr_svg(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let w = code.width();
    let colors = code.to_colors();
    let mut d = String::new();
    for (i, c) in colors.iter().enumerate() {
        if *c == qrcode::Color::Dark {
            d += &format!("M{} {}h1v1h-1z", i % w + 4, i / w + 4);
        }
    }
    let n = w + 8;
    Some(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {n} {n}\" shape-rendering=\"crispEdges\"><path fill=\"#000\" d=\"{d}\"/></svg>\n"))
}

/// The service file for these settings: `serve --print`, the marker, and the app's herdr-core session.
/// Err: a value that cannot go into the file (a login with spaces or control characters, say).
pub fn service_file(kind: Kind, program: &str, user_home: &str, user: &str, port: u16, read_only: bool, hc: &[(String, String)]) -> Result<String, String> {
    let mut args = vec!["--user".to_string(), user.to_string(), "--port".into(), port.to_string()];
    if read_only {
        args.push("--read-only".into());
    }
    let o = serve::parse(&args)?;
    if let Some((k, _)) = hc.iter().find(|(_, v)| v.chars().any(char::is_control)) {
        return Err(format!("{k} has a control character"));
    }
    let xml = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    match kind {
        Kind::Launchd => {
            let s = serve::service("launchd", program, user_home, &o);
            let env: String = hc.iter().map(|(k, v)| format!("<key>{}</key><string>{}</string>", xml(k), xml(v))).collect();
            let s = if env.is_empty() { s } else { s.replacen("</dict>\n</plist>", &format!("  <key>EnvironmentVariables</key><dict>{env}</dict>\n</dict>\n</plist>"), 1) };
            Ok(s.replacen("<plist version", &format!("<!-- {MARKER} -->\n<plist version"), 1))
        }
        Kind::Systemd => {
            let s = serve::service("systemd", program, user_home, &o);
            let env: String = hc.iter().map(|(k, v)| format!("Environment=\"{}={}\"\n", k, v.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"))).collect();
            Ok(format!("# {MARKER}\n{}", s.replacen("[Service]\n", &format!("[Service]\n{env}"), 1)))
        }
    }
}

fn ours(text: &str) -> bool {
    text.contains(MARKER)
}

/// The service: off, by-hand, running, stopped or error, with a message for an error.
pub fn service_state(env: &Env) -> Value {
    let file = env.file();
    let text = std::fs::read_to_string(&file).ok();
    let (state, message): (&str, Option<String>) = match text {
        None => ("off", None),
        Some(t) if !ours(&t) => ("by-hand", Some(format!("Set up by hand in {} (see SERVE.md). Flockbook settings leave it alone.", file.display()))),
        Some(_) => match env.kind {
            Kind::Launchd => match env.uid().map(|uid| env.run("launchctl", &["print", &format!("gui/{uid}/{LABEL}")])) {
                Err((_, e)) => ("error", Some(e)),
                Ok(r) => match r {
                Some((0, out, _)) if out.contains("state = running") => ("running", None),
                Some((0, out, _)) => {
                    let code = out.lines().find_map(|l| l.trim().strip_prefix("last exit code = ")).map(str::trim).unwrap_or("0");
                    if code != "0" && !code.is_empty() {
                        ("error", Some(format!("It stopped (exit code {code}). Its log: ~/Library/Logs/flockbook-modfeed-serve.log")))
                    } else {
                        ("stopped", None)
                    }
                }
                Some(_) => ("stopped", Some("The agent is not loaded. Turn it off and on again.".into())),
                None => ("error", Some("launchctl did not run.".into())),
                },
            },
            Kind::Systemd => match env.run("systemctl", &["--user", "is-active", UNIT]) {
                Some((0, _, _)) => ("running", None),
                Some((_, out, _)) if out.trim() == "failed" => {
                    ("error", Some(format!("It failed. Its log: journalctl --user -u {UNIT}")))
                }
                Some(_) => ("stopped", None),
                None => ("error", Some("systemctl --user did not run.".into())),
            },
        },
    };
    json!({"kind": match env.kind { Kind::Launchd => "launchd", Kind::Systemd => "systemd" }, "state": state, "file": file, "message": message})
}

/// The whole picture, as the mod shows it. Writes the QR code when Serve is set.
pub fn status(env: &Env, cfg: &Config, dir: Option<&Path>) -> Value {
    let ts = tailscale(env);
    let user = if !cfg.phone_user.is_empty() { Some(cfg.phone_user.clone()) } else { ts.as_ref().ok().map(|(l, _)| l.clone()) };
    let step = ts.as_ref().ok().map(|_| serve_step(env, cfg.phone_port));
    let url = step.as_ref().and_then(|s| s.url.clone());
    let qr = match (&url, dir) {
        (Some(u), Some(d)) => qr_svg(u).and_then(|svg| {
            let p = d.join("phone-qr.svg");
            if std::fs::read_to_string(&p).ok().as_deref() != Some(svg.as_str()) {
                let _ = std::fs::create_dir_all(d);
                std::fs::write(&p, svg).ok()?;
            }
            Some(p)
        }),
        _ => None,
    };
    json!({
        "type": "phone",
        "tailscale": ts.as_ref().ok().map(|(login, dns)| json!({"login": login, "dns": dns})),
        "tailscale_error": ts.as_ref().err(),
        "user": user,
        "port": cfg.phone_port,
        "read_only": cfg.phone_read_only,
        "serve": {
            "set": url.is_some(),
            "url": url,
            "command": step.as_ref().map_or_else(|| format!("tailscale serve --bg {}", cfg.phone_port), |s| s.command.clone()),
            "reason": step.as_ref().and_then(|s| s.reason.clone()),
        },
        "qr": qr,
        "service": service_state(env),
    })
}

/// The app's herdr-core session and socket, for the service (it runs outside the app).
fn hc_env() -> Vec<(String, String)> {
    ["HC_SESSION", "HC_SOCKET"].iter().filter_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()).map(|v| (k.to_string(), v))).collect()
}

fn check(r: Option<(i32, String, String)>, what: &str) -> Result<(), (&'static str, String)> {
    match r {
        Some((0, _, _)) => Ok(()),
        Some((c, out, err)) => Err(("service", format!("{what} failed ({c}): {}", if err.trim().is_empty() { out.trim() } else { err.trim() }))),
        None => Err(("service", format!("{what} did not run"))),
    }
}

/// On: writes the file (again, when the settings changed) and (re)loads it.
pub fn on(env: &Env, cfg: &Config) -> Result<(), (&'static str, String)> {
    let file = env.file();
    if std::fs::read_to_string(&file).is_ok_and(|t| !ours(&t)) {
        return Err(("by-hand", format!("{} was set up by hand (see SERVE.md); Flockbook settings leave it alone.", file.display())));
    }
    // No Tailscale, no way to reach the page: not even with a login typed in.
    let (login, _) = tailscale(env).map_err(|e| ("no-tailscale", e))?;
    let user = if cfg.phone_user.is_empty() { login } else { cfg.phone_user.clone() };
    // Its real path, as the install compares it (restart_after_install).
    let program = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| ("service", format!("cannot find this helper: {e}")))?;
    let text = service_file(env.kind, &program, &env.user_home.to_string_lossy(), &user, cfg.phone_port, cfg.phone_read_only, &hc_env()).map_err(|e| ("bad-value", e))?;
    let uid = if env.kind == Kind::Launchd { Some(env.uid()?) } else { None };
    std::fs::create_dir_all(file.parent().unwrap()).map_err(|e| ("write", format!("cannot make {}: {e}", file.parent().unwrap().display())))?;
    let tmp = file.with_extension("new");
    if let Err(e) = std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, &file)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(("write", format!("cannot write {}: {e}", file.display())));
    }
    let f = file.to_string_lossy().into_owned();
    match env.kind {
        Kind::Launchd => {
            let uid = uid.unwrap_or_default();
            // Loaded already: out first, so the new file counts. bootout finishes on its own time; until then
            // bootstrap says 5 (Input/output error) or 37. A few tries, about 2 s in all.
            let _ = env.run("launchctl", &["bootout", &format!("gui/{uid}/{LABEL}")]);
            let mut tries = 0;
            loop {
                let r = env.run("launchctl", &["bootstrap", &format!("gui/{uid}"), &f]);
                tries += 1;
                match r {
                    Some((5 | 37, _, _)) if tries < BOOTSTRAP_TRIES => std::thread::sleep(std::time::Duration::from_millis(250)),
                    r => return check(r, "launchctl bootstrap"),
                }
            }
        }
        Kind::Systemd => {
            check(env.run("systemctl", &["--user", "daemon-reload"]), "systemctl --user daemon-reload")?;
            check(env.run("systemctl", &["--user", "enable", UNIT]), "systemctl --user enable")?;
            check(env.run("systemctl", &["--user", "restart", UNIT]), "systemctl --user restart")
        }
    }
}

/// Off: stops it and removes the file, only ours.
pub fn off(env: &Env) -> Result<(), (&'static str, String)> {
    let file = env.file();
    let Ok(text) = std::fs::read_to_string(&file) else { return Ok(()) };
    if !ours(&text) {
        return Err(("by-hand", format!("{} was set up by hand (see SERVE.md); Flockbook settings leave it alone.", file.display())));
    }
    match env.kind {
        Kind::Launchd => {
            // No uid: it cannot be stopped, so the file stays and says it is on.
            let _ = env.run("launchctl", &["bootout", &format!("gui/{}/{LABEL}", env.uid()?)]);
        }
        Kind::Systemd => {
            let _ = env.run("systemctl", &["--user", "disable", "--now", UNIT]);
        }
    }
    std::fs::remove_file(&file).map_err(|e| ("write", format!("cannot remove {}: {e}", file.display())))?;
    if env.kind == Kind::Systemd {
        let _ = env.run("systemctl", &["--user", "daemon-reload"]);
    }
    Ok(())
}

/// After the install replaced `helper`: our service runs the old program until restarted. Restarts it only when
/// the file is ours and names this helper. Some(message) when it restarted (or tried).
pub fn restart_after_install(env: &Env, helper: &Path) -> Option<String> {
    let file = env.file();
    let text = std::fs::read_to_string(&file).ok()?;
    if !ours(&text) || !text.contains(&*helper.to_string_lossy()) {
        return None;
    }
    let r = match env.kind {
        Kind::Launchd => env.uid().ok().and_then(|uid| env.run("launchctl", &["kickstart", "-k", &format!("gui/{uid}/{LABEL}")])),
        Kind::Systemd => env.run("systemctl", &["--user", "try-restart", UNIT]),
    };
    Some(match r {
        Some((0, _, _)) => "  the phone page (Flockbook settings): restarted with the new helper".into(),
        _ => "  the phone page (Flockbook settings): could not restart it; turn it off and on in Flockbook settings".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_tools_are_found_at_fixed_paths() {
        let all = |_: &Path| true;
        assert_eq!(tool("launchctl", None, all), PathBuf::from("/bin/launchctl"));
        assert_eq!(tool("id", None, all), PathBuf::from("/usr/bin/id"));
        assert_eq!(tool("systemctl", None, all), PathBuf::from("/usr/bin/systemctl"));
        assert_eq!(tool("systemctl", None, |p: &Path| p == Path::new("/bin/systemctl")), PathBuf::from("/bin/systemctl"));
        assert_eq!(tool("tailscale", None, all), PathBuf::from("tailscale"), "tailscale: PATH, then the Mac app");
        assert_eq!(tool("launchctl", Some(Path::new("/fake")), all), PathBuf::from("/fake/launchctl"));
    }

    #[test]
    fn a_session_value_with_a_line_break_is_refused() {
        let hc = vec![("HC_SESSION".to_string(), "s1\nExecStartPre=/bin/sh".to_string())];
        assert!(service_file(Kind::Systemd, "/h/bin/fm", "/h/me", "me@x.com", 8787, false, &hc).is_err());
        assert!(service_file(Kind::Launchd, "/h/bin/fm", "/h/me", "me@x.com", 8787, false, &hc).is_err());
    }

    #[test]
    fn a_qr_code_is_a_square_of_modules() {
        let s = qr_svg("https://box.tail1.ts.net/").unwrap();
        assert!(s.starts_with("<svg") && s.contains("viewBox=\"0 0 33 33\""), "version 2 (25) plus the margin: {s}");
    }

    #[test]
    fn the_file_carries_the_marker_and_the_session() {
        let hc = vec![("HC_SESSION".to_string(), "s1".to_string())];
        let l = service_file(Kind::Launchd, "/h/bin/fm", "/h/me", "me@x.com", 8787, true, &hc).unwrap();
        assert!(ours(&l) && l.contains("<key>EnvironmentVariables</key><dict><key>HC_SESSION</key><string>s1</string></dict>"), "{l}");
        assert!(l.contains("<string>--read-only</string>") && l.ends_with("</dict>\n</plist>\n"), "{l}");
        let s = service_file(Kind::Systemd, "/h/bin/fm", "/home/me", "me@x.com", 8787, false, &hc).unwrap();
        assert!(s.starts_with("# Made by Flockbook settings") && s.contains("[Service]\nEnvironment=\"HC_SESSION=s1\"\nExecStart="), "{s}");
        assert!(!ours(&serve::service("systemd", "/h/bin/fm", "/home/me", &serve::parse(&["--user".into(), "a".into()]).unwrap())));
    }
}
