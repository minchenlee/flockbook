//! `update-check` and the settings in `<home>/flockbook/config.json` that the helper follows. The "release" is a
//! file:// JSON (FLOCKBOOK_UPDATE_URL), and FLOCKBOOK_UPDATE_VERSION stands in for the release version, so
//! nothing here reaches the network. Temp homes only.
#![cfg(unix)]

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

struct T {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    release: PathBuf,
}

fn t() -> T {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let home = root.join("app");
    fs::create_dir_all(home.join("flockbook")).unwrap();
    T { _tmp: tmp, release: root.join("latest.json"), home }
}

impl T {
    fn publish(&self, tag: &str) {
        fs::write(&self.release, json!({"tag_name": tag, "name": "x"}).to_string()).unwrap();
    }
    fn config(&self, text: &str) {
        fs::write(self.home.join("flockbook/config.json"), text).unwrap();
    }
    fn cache(&self) -> Option<Value> {
        fs::read(self.home.join("flockbook/update-check.json")).ok().map(|b| serde_json::from_slice(&b).unwrap())
    }
    fn set_cache(&self, v: Value) {
        fs::write(self.home.join("flockbook/update-check.json"), v.to_string()).unwrap();
    }
    fn run(&self, version: Option<&str>, extra: &[&str]) -> Value {
        let mut c = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"));
        c.arg("update-check").args(["--home", self.home.to_str().unwrap()]).args(extra).env_clear().env("PATH", "/usr/bin:/bin");
        c.env("FLOCKBOOK_UPDATE_URL", format!("file://{}", self.release.display()));
        if let Some(v) = version {
            c.env("FLOCKBOOK_UPDATE_VERSION", v);
        }
        let o = c.output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let text = String::from_utf8(o.stdout).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.first().unwrap()["type"], "header");
        assert_eq!(lines.last().unwrap()["type"], "end");
        lines.into_iter().find(|l| l["type"] == "update").expect("an update line")
    }
}

const ONE_LINE: &str = "curl -fsSL https://github.com/minchenlee/flockbook/releases/latest/download/install.sh | sh";

#[test]
fn a_newer_release_is_reported_with_the_one_line_install() {
    let t = t();
    t.publish("v0.9.0");
    let u = t.run(Some("0.8.0"), &[]);
    assert_eq!(u["state"], "newer");
    assert_eq!(u["current"], "0.8.0");
    assert_eq!(u["latest"], "0.9.0");
    assert_eq!(u["command"], ONE_LINE);
    assert_eq!(t.cache().unwrap()["latest"], "0.9.0");
}

#[test]
fn the_same_or_an_older_release_is_current() {
    let t = t();
    t.publish("v0.8.0");
    assert_eq!(t.run(Some("0.8.0"), &[])["state"], "current");
    let t = self::t();
    t.publish("v0.7.0");
    assert_eq!(t.run(Some("0.8.0"), &[])["state"], "current");
}

#[test]
fn it_asks_at_most_once_a_day() {
    let t = t();
    t.publish("v0.9.0");
    assert_eq!(t.run(Some("0.8.0"), &[])["latest"], "0.9.0");
    // A newer release within the day is not seen: the cached answer stands, no second request.
    t.publish("v0.10.0");
    assert_eq!(t.run(Some("0.8.0"), &[])["latest"], "0.9.0");
    // A day later it asks again.
    let mut c = t.cache().unwrap();
    c["checked"] = json!(c["checked"].as_u64().unwrap() - 25 * 3600);
    t.set_cache(c);
    assert_eq!(t.run(Some("0.8.0"), &[])["latest"], "0.10.0");
}

#[test]
fn off_in_config_json_means_no_request_and_no_line() {
    let t = t();
    t.publish("v0.9.0");
    t.config(r#"{"check_updates": false, "other": 1}"#);
    let u = t.run(Some("0.8.0"), &[]);
    assert_eq!(u["state"], "off");
    assert_eq!(u["latest"], Value::Null);
    assert!(t.cache().is_none(), "nothing asked, nothing kept");
    // The helper never writes config.json.
    assert_eq!(fs::read_to_string(t.home.join("flockbook/config.json")).unwrap(), r#"{"check_updates": false, "other": 1}"#);
}

#[test]
fn a_bad_or_partial_config_json_is_the_default_on() {
    let t = t();
    t.publish("v0.9.0");
    t.config("{ not json");
    assert_eq!(t.run(Some("0.8.0"), &[])["state"], "newer");
    let t = self::t();
    t.publish("v0.9.0");
    t.config(r#"{"status_row": "hide"}"#);
    assert_eq!(t.run(Some("0.8.0"), &[])["state"], "newer");
}

#[test]
fn offline_or_a_bad_answer_shows_nothing_and_waits_a_day() {
    let t = t();
    // No release file: curl fails, as offline.
    let u = t.run(Some("0.8.0"), &[]);
    assert_eq!(u["state"], "error");
    assert_eq!(u["latest"], Value::Null);
    // The try counts: within the day it does not ask again, even once the release is there.
    t.publish("v0.9.0");
    assert_eq!(t.run(Some("0.8.0"), &[])["state"], "error");
    // An answer that is not a release keeps the last good one.
    let t = self::t();
    t.publish("v0.9.0");
    t.run(Some("0.8.0"), &[]);
    fs::write(&t.release, "<html>rate limited</html>").unwrap();
    let mut c = t.cache().unwrap();
    c["checked"] = json!(0);
    t.set_cache(c);
    assert_eq!(t.run(Some("0.8.0"), &[])["latest"], "0.9.0");
}

#[test]
fn a_build_without_a_release_version_never_asks() {
    if option_env!("FLOCKBOOK_VERSION").is_some() {
        return; // a release build has one
    }
    let t = t();
    t.publish("v0.9.0");
    let u = t.run(None, &[]);
    assert_eq!(u["state"], "unknown");
    assert!(t.cache().is_none());
}

#[test]
fn dismiss_hides_that_version_until_a_newer_one() {
    let t = t();
    t.publish("v0.9.0");
    assert_eq!(t.run(Some("0.8.0"), &[])["state"], "newer");
    assert_eq!(t.run(Some("0.8.0"), &["--dismiss", "0.9.0"])["state"], "dismissed");
    assert_eq!(t.run(Some("0.8.0"), &[])["state"], "dismissed");
    t.publish("v0.10.0");
    let mut c = t.cache().unwrap();
    c["checked"] = json!(0);
    t.set_cache(c);
    let u = t.run(Some("0.8.0"), &[]);
    assert_eq!((u["state"].as_str(), u["latest"].as_str()), (Some("newer"), Some("0.10.0")));
}

/// Never in config.json: a command for a terminal on another machine stops before anything (no herdr-core
/// needed to see it), and so does `helper install` / `remove`.
#[test]
fn remote_helper_never_runs_nothing_on_other_machines() {
    let t = t();
    t.config(r#"{"remote_helper": "never"}"#);
    let run = |args: &[&str]| -> String {
        let o = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
            .args(args)
            .args(["--home", t.home.to_str().unwrap()])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HC_SOCKET", t.home.join("no-core.sock"))
            .output()
            .unwrap();
        String::from_utf8(o.stdout).unwrap()
    };
    for args in [
        &["feed", "--terminal", "m1/t1"][..],
        &["send", "--terminal", "m1/t1", "--text", "hi"][..],
        // A text that says --home is not the home: the last --home (the mod's) counts.
        &["send", "--terminal", "m1/t1", "--text", "--home"][..],
        &["card", "--terminal", "hc:m1/t2"][..],
        &["helper", "install", "--machine", "m1", "--sha256", "00"][..],
        &["helper", "remove", "--machine", "m1"][..],
    ] {
        let out = run(args);
        assert!(out.contains(r#""code":"remote-off""#), "{args:?}: {out}");
    }
    // A terminal on this machine is not affected (here: no herdr-core, so that error instead).
    let out = run(&["feed", "--terminal", "t1"]);
    assert!(!out.contains("remote-off"), "{out}");
    // Ask (or no file) does not stop it.
    fs::remove_file(t.home.join("flockbook/config.json")).unwrap();
    assert!(!run(&["feed", "--terminal", "m1/t1"]).contains("remote-off"));
}

#[test]
fn version_lists_update_check() {
    let o = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).arg("version").output().unwrap();
    assert!(String::from_utf8_lossy(&o.stdout).contains("\"update-check\""));
}

// `version` names the release baked in at build time, which the release's package job checks against the tag.
// The test stand-in does not change it: only the build does.
#[test]
fn version_names_the_baked_release() {
    let o = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed")).arg("version").env("FLOCKBOOK_UPDATE_VERSION", "0.8.0").output().unwrap();
    let line = String::from_utf8_lossy(&o.stdout).lines().find(|l| l.contains("\"type\":\"version\"")).unwrap().to_string();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    let want = option_env!("FLOCKBOOK_VERSION").map(|s| s.trim_start_matches('v').to_string());
    assert_eq!(v["release"], serde_json::json!(want), "{line}");
}

