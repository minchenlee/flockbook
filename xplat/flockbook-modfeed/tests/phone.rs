//! `phone status|on|off`: the phone page as a user service, set from the mod's Flockbook settings.
//! Temp HOME, and fake `tailscale`, `launchctl`, `systemctl` and `id` in FLOCKBOOK_PHONE_BIN: nothing here
//! reaches the real service manager or Tailscale. Each fake logs its arguments to `log`.
#![cfg(unix)]

use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct T {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf, // the app home (--home)
    user: PathBuf, // HOME
    bin: PathBuf,  // the fakes
}

const LAUNCHCTL: &str = r#"#!/bin/sh
echo "launchctl $*" >> "$FAKE/log"
case "$1" in
  print) [ -f "$FAKE/loaded" ] || exit 113; if [ -f "$FAKE/exit" ]; then echo "state = not running"; echo "last exit code = $(cat "$FAKE/exit")"; else echo "state = running"; fi ;;
  bootstrap)
    # Right after a bootout launchd is still busy: "Bootstrap failed: 5: Input/output error", `busy` times.
    n=$(cat "$FAKE/busy" 2>/dev/null || echo 0)
    if [ "$n" -gt 0 ]; then echo $((n - 1)) > "$FAKE/busy"; echo "Bootstrap failed: 5: Input/output error" >&2; exit 5; fi
    touch "$FAKE/loaded" ;;
  bootout) [ -f "$FAKE/loaded" ] || exit 3; rm -f "$FAKE/loaded" ;;
esac
exit 0
"#;
const SYSTEMCTL: &str = r#"#!/bin/sh
echo "systemctl $*" >> "$FAKE/log"
case "$2" in
  is-active) if [ -f "$FAKE/active" ]; then cat "$FAKE/active"; grep -qx active "$FAKE/active"; exit $?; fi; echo inactive; exit 3 ;;
  enable|restart|start) echo active > "$FAKE/active" ;;
  disable|stop) rm -f "$FAKE/active" ;;
esac
exit 0
"#;
const TAILSCALE: &str = r#"#!/bin/sh
echo "tailscale $*" >> "$FAKE/log"
case "$1" in
  status) [ -f "$FAKE/ts-status.json" ] || { echo "failed to connect to local tailscaled" >&2; exit 1; }; cat "$FAKE/ts-status.json" ;;
  serve) if [ -f "$FAKE/ts-serve.json" ]; then cat "$FAKE/ts-serve.json"; else echo "{}"; fi ;;
  *) exit 2 ;;
esac
"#;
const ID: &str = "#!/bin/sh\n[ -f \"$FAKE/no-id\" ] && exit 1\necho 501\n";

fn t() -> T {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let (home, user, bin) = (root.join("app"), root.join("user"), root.join("fake"));
    fs::create_dir_all(home.join("flockbook")).unwrap();
    fs::create_dir_all(&user).unwrap();
    fs::create_dir_all(&bin).unwrap();
    for (name, body) in [("launchctl", LAUNCHCTL), ("systemctl", SYSTEMCTL), ("tailscale", TAILSCALE), ("id", ID)] {
        let p = bin.join(name);
        fs::write(&p, body).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    T { _tmp: tmp, root, home, user, bin }
}

fn signed_in(t: &T) {
    fs::write(
        t.bin.join("ts-status.json"),
        json!({"BackendState": "Running", "Self": {"UserID": 42, "DNSName": "box.tail1.ts.net."},
               "User": {"42": {"ID": 42, "LoginName": "me@example.com"}, "7": {"ID": 7, "LoginName": "other@example.com"}}})
        .to_string(),
    )
    .unwrap();
}

impl T {
    fn run(&self, kind: &str, args: &[&str]) -> Vec<Value> {
        self.run_as(Path::new(env!("CARGO_BIN_EXE_flockbook-modfeed")), kind, args)
    }
    /// Runs the helper by `exe` (a symlink to it, say).
    fn run_as(&self, exe: &Path, kind: &str, args: &[&str]) -> Vec<Value> {
        let mut c = Command::new(exe);
        c.arg("phone").args(args).args(["--home", self.home.to_str().unwrap()]).env_clear();
        c.env("PATH", "/usr/bin:/bin").env("HOME", &self.user).env("FAKE", &self.bin);
        c.env("FLOCKBOOK_PHONE_BIN", &self.bin).env("FLOCKBOOK_PHONE_SERVICE", kind).env("HC_SESSION", "fb-test-phone");
        let o = c.output().unwrap();
        let text = String::from_utf8(o.stdout).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.first().unwrap()["type"], "header", "{text}");
        assert_eq!(lines.last().unwrap()["type"], "end", "{text}");
        lines
    }
    fn phone(&self, kind: &str, args: &[&str]) -> Value {
        let lines = self.run(kind, args);
        lines.iter().find(|l| l["type"] == "phone").cloned().unwrap_or_else(|| panic!("a phone line: {lines:?}"))
    }
    fn error(&self, kind: &str, args: &[&str]) -> Option<Value> {
        self.run(kind, args).into_iter().find(|l| l["type"] == "error")
    }
    fn log(&self) -> String {
        fs::read_to_string(self.bin.join("log")).unwrap_or_default()
    }
    fn clear_log(&self) {
        let _ = fs::remove_file(self.bin.join("log"));
    }
    fn config(&self, v: Value) {
        fs::write(self.home.join("flockbook/config.json"), v.to_string()).unwrap();
    }
    fn plist(&self) -> PathBuf {
        self.user.join("Library/LaunchAgents/dev.flockbook.serve.plist")
    }
    fn unit(&self) -> PathBuf {
        self.user.join(".config/systemd/user/flockbook-serve.service")
    }
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn no_tailscale_the_page_cannot_be_turned_on() {
    let t = t();
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["tailscale"], Value::Null);
    assert!(p["tailscale_error"].as_str().unwrap().contains("Tailscale"), "{p}");
    assert_eq!(p["service"]["state"], "off");
    let e = t.error("launchd", &["on"]).expect("on is refused");
    assert_eq!(e["code"], "no-tailscale");
    assert!(!t.plist().exists());
    assert!(!t.log().contains("launchctl bootstrap"));
}

#[test]
fn status_names_the_login_the_address_and_the_command_to_set_serve() {
    let t = t();
    signed_in(&t);
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["tailscale"], json!({"login": "me@example.com", "dns": "box.tail1.ts.net"}));
    assert_eq!(p["user"], "me@example.com", "no phone_user: the login found");
    assert_eq!(p["port"], 8787);
    assert_eq!(p["read_only"], false);
    assert_eq!(p["serve"]["set"], false);
    assert_eq!(p["serve"]["url"], Value::Null);
    assert_eq!(p["serve"]["command"], "tailscale serve --bg 8787");
    assert_eq!(p["qr"], Value::Null);
    assert_eq!(p["service"]["state"], "off");
    assert!(!t.log().contains("launchctl bootstrap") && !t.log().contains("tailscale serve --"), "status changes nothing: {}", t.log());
}

#[test]
fn on_with_launchd_writes_the_agent_marked_as_ours_and_loads_it() {
    let t = t();
    signed_in(&t);
    let p = t.phone("launchd", &["on"]);
    let f = read(&t.plist());
    assert!(f.contains("Made by Flockbook settings"), "{f}");
    assert!(f.contains("<string>dev.flockbook.serve</string>"));
    assert!(f.contains("<string>me@example.com</string>") && f.contains("<string>8787</string>"), "{f}");
    assert!(f.contains(env!("CARGO_BIN_EXE_flockbook-modfeed")), "the helper that wrote it runs it: {f}");
    assert!(f.contains("<key>HC_SESSION</key><string>fb-test-phone</string>"), "the app's core session: {f}");
    assert!(!f.contains("--read-only"));
    assert!(t.log().contains(&format!("launchctl bootstrap gui/501 {}", t.plist().display())), "{}", t.log());
    assert_eq!(p["service"]["state"], "running", "{p}");
    assert_eq!(p["service"]["file"], t.plist().to_str().unwrap());
    assert_eq!(t.phone("launchd", &["status"])["service"]["state"], "running");
    // launchd reads it: the file is a valid plist (macOS's own check; it reads the file only).
    if Path::new("/usr/bin/plutil").exists() {
        let lint = std::process::Command::new("/usr/bin/plutil").arg("-lint").arg(t.plist()).output().unwrap();
        assert!(lint.status.success(), "{}{}", String::from_utf8_lossy(&lint.stdout), String::from_utf8_lossy(&lint.stderr));
    }
}

#[test]
fn the_settings_go_into_the_file_and_a_change_reloads_it() {
    let t = t();
    signed_in(&t);
    t.config(json!({"phone_user": "boss@example.com", "phone_read_only": true, "phone_port": 9000}));
    let p = t.phone("launchd", &["on"]);
    let f = read(&t.plist());
    assert!(f.contains("<string>boss@example.com</string>") && f.contains("<string>9000</string>") && f.contains("<string>--read-only</string>"), "{f}");
    assert_eq!((p["user"].clone(), p["port"].clone(), p["read_only"].clone()), (json!("boss@example.com"), json!(9000), json!(true)));
    assert_eq!(p["serve"]["command"], "tailscale serve --bg 9000");
    // A change while on: the file is written again and the agent loaded again.
    t.config(json!({"phone_read_only": false}));
    t.clear_log();
    t.phone("launchd", &["on"]);
    let f = read(&t.plist());
    assert!(!f.contains("--read-only") && f.contains("<string>me@example.com</string>"), "{f}");
    let log = t.log();
    let (out, boot) = (log.find("launchctl bootout gui/501/dev.flockbook.serve"), log.find("launchctl bootstrap"));
    assert!(out.is_some() && boot.is_some() && out < boot, "out, then in: {log}");
}

#[test]
fn off_removes_exactly_that_file() {
    let t = t();
    signed_in(&t);
    t.phone("launchd", &["on"]);
    let other = t.user.join("Library/LaunchAgents/com.example.other.plist");
    fs::write(&other, "x").unwrap();
    let p = t.phone("launchd", &["off"]);
    assert!(!t.plist().exists());
    assert!(other.exists(), "nothing else in LaunchAgents");
    assert!(t.log().contains("launchctl bootout gui/501/dev.flockbook.serve"));
    assert_eq!(p["service"]["state"], "off");
    // Off again: nothing to do, no error.
    assert!(t.error("launchd", &["off"]).is_none());
}

#[test]
fn a_service_set_up_by_hand_is_left_alone() {
    let t = t();
    signed_in(&t);
    fs::create_dir_all(t.plist().parent().unwrap()).unwrap();
    // As SERVE.md has you make it: --print, no marker.
    let by_hand = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).args(["serve", "--print", "launchd", "--user", "me@example.com"]).env("HOME", &t.user).output().unwrap();
    fs::write(t.plist(), &by_hand.stdout).unwrap();
    let before = read(&t.plist());
    assert_eq!(t.phone("launchd", &["status"])["service"]["state"], "by-hand");
    assert_eq!(t.error("launchd", &["off"]).unwrap()["code"], "by-hand");
    assert_eq!(t.error("launchd", &["on"]).unwrap()["code"], "by-hand");
    assert_eq!(read(&t.plist()), before, "not changed");
    assert!(!t.log().contains("launchctl boot"), "{}", t.log());
}

#[test]
fn systemd_user_unit_on_and_off() {
    let t = t();
    signed_in(&t);
    let p = t.phone("systemd", &["on"]);
    let f = read(&t.unit());
    assert!(f.starts_with("# Made by Flockbook settings"), "{f}");
    assert!(f.contains("Environment=\"HC_SESSION=fb-test-phone\""), "{f}");
    assert!(f.contains("\"--user\" \"me@example.com\" \"--port\" \"8787\""), "{f}");
    let log = t.log();
    for want in ["systemctl --user daemon-reload", "systemctl --user enable flockbook-serve", "systemctl --user restart flockbook-serve"] {
        assert!(log.contains(want), "{want}: {log}");
    }
    assert_eq!(p["service"]["state"], "running");
    // systemd reads it: its own check of the file (offline; it starts nothing).
    if Path::new("/usr/bin/systemd-analyze").exists() {
        let v = std::process::Command::new("/usr/bin/systemd-analyze").args(["verify", "--man=no"]).arg(t.unit()).output().unwrap();
        let said = String::from_utf8_lossy(&v.stderr).into_owned() + &String::from_utf8_lossy(&v.stdout);
        assert!(v.status.success() && !said.contains("flockbook-serve.service:"), "{said}");
    }
    let p = t.phone("systemd", &["off"]);
    assert!(!t.unit().exists());
    assert!(t.log().contains("systemctl --user disable --now flockbook-serve"));
    assert_eq!(p["service"]["state"], "off");
}

#[test]
fn a_failed_service_is_an_error_with_where_to_look() {
    let t = t();
    signed_in(&t);
    t.phone("systemd", &["on"]);
    fs::write(t.bin.join("active"), "failed\n").unwrap();
    let s = &t.phone("systemd", &["status"])["service"];
    assert_eq!(s["state"], "error", "{s}");
    assert!(s["message"].as_str().unwrap().contains("journalctl --user -u flockbook-serve"), "{s}");
    // launchd: loaded, not running, a last exit code.
    let t = crate::t();
    signed_in(&t);
    t.phone("launchd", &["on"]);
    fs::write(t.bin.join("exit"), "1").unwrap();
    let s = &t.phone("launchd", &["status"])["service"];
    assert_eq!(s["state"], "error", "{s}");
    assert!(s["message"].as_str().unwrap().contains("flockbook-modfeed-serve.log"), "{s}");
}

fn serve_json(t: &T, v: Value) {
    fs::write(t.bin.join("ts-serve.json"), v.to_string()).unwrap();
}

#[test]
fn serve_443_used_by_another_service_gives_an_8443_command_and_why() {
    let t = t();
    signed_in(&t);
    // 443 "/" is another local service: the plain command would replace it.
    serve_json(&t, json!({"TCP": {"443": {"HTTPS": true}}, "Web": {"box.tail1.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:3000"}}}}}));
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["serve"]["set"], false, "{p}");
    assert_eq!(p["serve"]["command"], "tailscale serve --bg --https=8443 8787");
    assert!(p["serve"]["reason"].as_str().unwrap().contains("443 is used by another service"), "{p}");
    // 8443 is taken as well: the next one.
    serve_json(&t, json!({"TCP": {"443": {"HTTPS": true}, "8443": {"HTTPS": true}}, "Web": {
        "box.tail1.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:3000"}}},
        "box.tail1.ts.net:8443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:4000"}}}}}));
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["serve"]["command"], "tailscale serve --bg --https=10443 8787");
    // Another path on 443 is not in the way: the plain command keeps it.
    serve_json(&t, json!({"TCP": {"443": {"HTTPS": true}}, "Web": {"box.tail1.ts.net:443": {"Handlers": {"/grafana": {"Proxy": "http://127.0.0.1:3000"}}}}}));
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["serve"]["command"], "tailscale serve --bg 8787");
    assert_eq!(p["serve"]["reason"], Value::Null);
    assert!(!t.log().contains("tailscale serve --"), "status changes nothing: {}", t.log());
}

#[test]
fn serve_443_free_gives_the_plain_command_and_no_reason() {
    let t = t();
    signed_in(&t);
    serve_json(&t, json!({}));
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["serve"]["command"], "tailscale serve --bg 8787");
    assert_eq!(p["serve"]["reason"], Value::Null);
}

#[test]
fn serve_set_by_us_on_8443_gives_the_address_with_the_port() {
    let t = t();
    signed_in(&t);
    serve_json(&t, json!({"TCP": {"443": {"HTTPS": true}, "8443": {"HTTPS": true}}, "Web": {
        "box.tail1.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:3000"}}},
        "box.tail1.ts.net:8443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8787"}}}}}));
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["serve"]["set"], true, "{p}");
    assert_eq!(p["serve"]["url"], "https://box.tail1.ts.net:8443/");
    assert_eq!(p["serve"]["reason"], Value::Null);
}

#[test]
fn a_new_address_gets_a_new_qr_file_and_the_old_ones_go() {
    let t = t();
    signed_in(&t);
    let dir = t.home.join("flockbook");
    fs::create_dir_all(&dir).unwrap();
    // An old fixed-name file, an old hashed one, and files that are not ours.
    for f in ["phone-qr.svg", "phone-qr-0badc0de.svg", "phone-qr-notes.txt", "config.json", "phone-qr-12345678.png"] {
        fs::write(dir.join(f), "x").unwrap();
    }
    serve_json(&t, json!({"Web": {"box.tail1.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8787"}}}}}));
    let first = PathBuf::from(t.phone("launchd", &["status"])["qr"].as_str().unwrap());
    // The same address again: the same file.
    let again = PathBuf::from(t.phone("launchd", &["status"])["qr"].as_str().unwrap());
    assert_eq!(first, again);
    // Now on 8443: another name, so a cached picture of the old one is not shown.
    serve_json(&t, json!({"Web": {"box.tail1.ts.net:8443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8787"}}}}}));
    let second = PathBuf::from(t.phone("launchd", &["status"])["qr"].as_str().unwrap());
    assert_ne!(first, second);
    let mut left: Vec<String> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    let mut want = vec!["config.json".to_string(), "phone-qr-12345678.png".into(), "phone-qr-notes.txt".into(), second.file_name().unwrap().to_string_lossy().into_owned()];
    want.sort();
    assert_eq!(left, want, "only the helper's older phone-qr*.svg files are removed");
}

#[test]
fn once_serve_is_set_the_address_and_a_qr_code() {
    let t = t();
    signed_in(&t);
    fs::write(
        t.bin.join("ts-serve.json"),
        json!({"TCP": {"443": {"HTTPS": true}}, "Web": {"box.tail1.ts.net:443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:8787"}}}}}).to_string(),
    )
    .unwrap();
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["serve"]["set"], true, "{p}");
    assert_eq!(p["serve"]["url"], "https://box.tail1.ts.net/");
    let qr = PathBuf::from(p["qr"].as_str().expect("a qr path"));
    let name = qr.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(qr.parent(), Some(t.home.join("flockbook").as_path()));
    let hex = name.strip_prefix("phone-qr-").and_then(|n| n.strip_suffix(".svg")).unwrap_or_else(|| panic!("{name}"));
    assert!(hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "8 hex of the content: {name}");
    let svg = read(&qr);
    assert!(svg.starts_with("<svg") && svg.contains("<path"), "{svg}");
    // Serve for another port is not this page.
    t.config(json!({"phone_port": 9000}));
    let p = t.phone("launchd", &["status"]);
    assert_eq!(p["serve"]["set"], false);
    assert_eq!(p["qr"], Value::Null);
    let _ = &t.root;
}

#[test]
fn bootstrap_busy_right_after_bootout_is_tried_again() {
    let t = t();
    signed_in(&t);
    fs::write(t.bin.join("busy"), "2").unwrap();
    let p = t.phone("launchd", &["on"]);
    assert_eq!(p["service"]["state"], "running", "{p} {}", t.log());
    assert_eq!(t.log().matches("launchctl bootstrap").count(), 3, "{}", t.log());
}

#[test]
fn bootstrap_that_stays_busy_gives_up_with_its_error() {
    let t = t();
    signed_in(&t);
    fs::write(t.bin.join("busy"), "1000").unwrap();
    let e = t.error("launchd", &["on"]).expect("an error");
    assert_eq!(e["code"], "service");
    assert!(e["message"].as_str().unwrap().contains("Input/output error"), "{e}");
    let tries = t.log().matches("launchctl bootstrap").count();
    assert!((2..=10).contains(&tries), "a few tries, not forever: {tries}");
}

#[test]
fn no_uid_is_an_error_not_gui_0() {
    let t = t();
    signed_in(&t);
    fs::write(t.bin.join("no-id"), "").unwrap();
    let e = t.error("launchd", &["on"]).expect("an error");
    assert_eq!(e["code"], "service");
    assert!(e["message"].as_str().unwrap().contains("id -u"), "{e}");
    assert!(!t.log().contains("gui/0"), "{}", t.log());
}

#[test]
fn a_login_with_a_line_break_or_a_space_is_refused_and_nothing_is_written() {
    for bad in ["me@example.com\nExecStartPre=/bin/sh -c id", "me @example.com", "me@example.com\u{7}"] {
        let t = t();
        signed_in(&t);
        t.config(json!({"phone_user": bad}));
        let e = t.error("systemd", &["on"]).unwrap_or_else(|| panic!("an error for {bad:?}"));
        assert_eq!(e["code"], "bad-value", "{e}");
        assert!(!t.unit().exists(), "{bad:?}");
        assert!(!t.log().contains("systemctl"), "{}", t.log());
    }
}

#[test]
fn a_dollar_in_a_login_stays_a_dollar_in_the_unit() {
    let t = t();
    signed_in(&t);
    t.config(json!({"phone_user": "me$HOME@example.com"}));
    t.phone("systemd", &["on"]);
    assert!(read(&t.unit()).contains("\"me$$HOME@example.com\""), "{}", read(&t.unit()));
}

#[test]
fn a_failed_write_leaves_no_new_file_behind() {
    let t = t();
    signed_in(&t);
    fs::create_dir_all(t.plist()).unwrap(); // the rename onto it fails
    let e = t.error("launchd", &["on"]).expect("an error");
    assert_eq!(e["code"], "write", "{e}");
    let left: Vec<_> = fs::read_dir(t.plist().parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from("dev.flockbook.serve.plist")], "no .new left");
}

#[test]
fn the_file_names_the_helper_by_its_real_path() {
    let t = t();
    signed_in(&t);
    let link = t.root.join("fm-link");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_flockbook-modfeed"), &link).unwrap();
    t.run_as(&link, "launchd", &["on"]);
    let real = Path::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).canonicalize().unwrap();
    let f = read(&t.plist());
    assert!(f.contains(&format!("<string>{}</string>", real.display())) && !f.contains("fm-link"), "{f}");
}
