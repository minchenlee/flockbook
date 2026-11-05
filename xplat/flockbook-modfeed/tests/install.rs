//! `flockbook-modfeed install`, run against temp homes only (the port of herdr-app/test/install.test.mjs).
//! HOME is a temp folder too, so the default (~/.config/herdr-app) can never be the real one. A fake
//! herdr-app on PATH records its calls.
#![cfg(unix)]

use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn mod_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../herdr-app")
}

struct Sandbox {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    app: PathBuf,
    helper: PathBuf,
}

fn write_exe(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn sandbox() -> Sandbox {
    let tmp = tempfile::Builder::new().prefix("fb-install-").tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let (home, app, bin) = (root.join("user"), root.join("app"), root.join("bin"));
    for d in [&home, &app, &bin] {
        fs::create_dir(d).unwrap();
    }
    let calls = root.join("calls");
    let fail = root.join("fail");
    write_exe(
        &bin.join("herdr-app"),
        &format!(
            "#!/bin/sh\necho \"$HERDR_APP_HOME $*\" >> \"{}\"\n[ -f \"{}\" ] && {{ echo \"bad mod\"; exit 1; }}\necho ok\n",
            calls.display(),
            fail.display()
        ),
    );
    let helper = root.join("flockbook-modfeed");
    write_exe(&helper, "#!/bin/sh\necho helper\n");
    Sandbox { _tmp: tmp, root, home, app, helper }
}

impl Sandbox {
    fn path_env(&self) -> String {
        format!("{}:/usr/bin:/bin", self.root.join("bin").display())
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
            .arg("install")
            .args(args)
            .env_clear()
            .env("PATH", self.path_env())
            .env("HOME", &self.home)
            .output()
            .unwrap()
    }
    fn calls(&self) -> Vec<String> {
        match fs::read_to_string(self.root.join("calls")) {
            Ok(t) => t.trim().lines().map(String::from).collect(),
            Err(_) => vec![],
        }
    }
    fn app_s(&self) -> &str {
        self.app.to_str().unwrap()
    }
}

fn read(p: impl AsRef<Path>) -> String {
    fs::read_to_string(p).unwrap()
}
fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
fn both(o: &Output) -> String {
    out(o) + &err(o)
}
fn keys_of(app: &Path) -> Value {
    serde_json::from_str(&read(app.join("keys.json"))).unwrap()
}
fn ours() -> Value {
    serde_json::from_str(&read(mod_dir().join("keys.json"))).unwrap()
}
fn sha(text: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(text).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn installs_into_an_empty_home_puts_the_helper_in_place_and_runs_check() {
    let s = sandbox();
    let r = s.run(&["--home", s.app_s(), "--helper", s.helper.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert!(s.app.join("layouts/flockbook.json").exists());
    assert!(s.app.join("mods/flockbook/mod.json").exists());
    assert!(s.app.join("themes/flockbook-light.json").exists());
    assert_eq!(read(s.app.join("flockbook/bin/flockbook-modfeed")), read(&s.helper));
    let mode = fs::metadata(s.app.join("flockbook/bin/flockbook-modfeed")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
    assert_eq!(keys_of(&s.app), ours());
    assert_eq!(s.calls(), vec![format!("{0} check {0}/mods/flockbook", s.app_s())]);
}

#[test]
fn installs_exactly_the_files_in_the_repo() {
    let s = sandbox();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    let marker: Value = serde_json::from_str(&read(s.app.join("flockbook/install.json"))).unwrap();
    let files = marker["files"].as_object().unwrap();
    let mut want = vec!["layouts/flockbook.json".to_string()];
    for e in fs::read_dir(mod_dir().join("themes")).unwrap() {
        let n = e.unwrap().file_name().into_string().unwrap();
        if n.ends_with(".json") {
            want.push(format!("themes/{n}"));
        }
    }
    let git = Command::new("git").args(["ls-files", "mods/flockbook"]).current_dir(mod_dir()).output().unwrap();
    want.extend(String::from_utf8(git.stdout).unwrap().lines().map(String::from));
    want.sort();
    let have: Vec<String> = files.keys().cloned().collect();
    assert_eq!(have, want);
    for rel in &want {
        assert_eq!(fs::read(s.app.join(rel)).unwrap(), fs::read(mod_dir().join(rel)).unwrap(), "{rel}");
        assert_eq!(files[rel], json!(sha(&fs::read(mod_dir().join(rel)).unwrap())), "{rel}");
    }
}

#[test]
fn dry_run_writes_nothing_and_does_not_run_herdr_app() {
    let s = sandbox();
    let r = s.run(&["--home", s.app_s(), "--helper", s.helper.to_str().unwrap(), "--dry-run"]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert_eq!(fs::read_dir(&s.app).unwrap().count(), 0);
    assert!(s.calls().is_empty());
    assert!(out(&r).contains("layouts/flockbook.json"));
    assert!(out(&r).to_lowercase().contains("dry run"));
}

#[test]
fn stops_without_writing_when_the_person_has_their_own_layout() {
    let s = sandbox();
    fs::create_dir(s.app.join("layouts")).unwrap();
    fs::write(s.app.join("layouts/flockbook.json"), "{\"mine\": true}\n").unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(err(&r).contains("layouts/flockbook.json"));
    assert_eq!(read(s.app.join("layouts/flockbook.json")), "{\"mine\": true}\n");
    assert!(!s.app.join("mods").exists());
    assert!(!s.app.join("keys.json").exists());
}

#[test]
fn stops_when_the_person_has_their_own_mod_folder() {
    let s = sandbox();
    fs::create_dir_all(s.app.join("mods/flockbook")).unwrap();
    fs::write(s.app.join("mods/flockbook/main.js"), "mine\n").unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(err(&r).contains("mods/flockbook"));
    assert_eq!(read(s.app.join("mods/flockbook/main.js")), "mine\n");
    assert!(!s.app.join("layouts").exists());
}

#[test]
fn merges_keys_keeps_the_persons_sections_and_keys() {
    let s = sandbox();
    fs::write(
        s.app.join("keys.json"),
        r#"{"global":{"cmd-k":"app.palette"},"layout:flockbook":{"cmd-b":"app.something-else"}}"#,
    )
    .unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    let k = keys_of(&s.app);
    assert_eq!(k["global"], json!({"cmd-k": "app.palette"}));
    assert_eq!(k["layout:flockbook"]["cmd-b"], "app.something-else");
    assert_eq!(k["layout:flockbook"]["cmd-t"], "tab.new");
    assert!(out(&r).contains("cmd-b"));
    // The person's order stays: their sections first, then ours.
    let text = read(s.app.join("keys.json"));
    assert!(text.find("\"global\"").unwrap() < text.find("\"layout:flockbook\"").unwrap());
}

#[test]
fn a_keys_file_that_is_not_json_stops_everything() {
    let s = sandbox();
    fs::write(s.app.join("keys.json"), "{ not json").unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(err(&r).contains("is not valid JSON"));
    assert!(!s.app.join("layouts").exists());
}

#[test]
fn a_symlinked_keys_file_is_merged_where_it_points_and_the_link_stays() {
    let s = sandbox();
    let dots = s.root.join("dotfiles");
    fs::create_dir(&dots).unwrap();
    fs::write(dots.join("keys.json"), r#"{"global":{"cmd-k":"app.palette"}}"#).unwrap();
    std::os::unix::fs::symlink(dots.join("keys.json"), s.app.join("keys.json")).unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert!(fs::symlink_metadata(s.app.join("keys.json")).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(s.app.join("keys.json")).unwrap(), dots.join("keys.json"));
    let k: Value = serde_json::from_str(&read(dots.join("keys.json"))).unwrap();
    assert_eq!(k["global"], json!({"cmd-k": "app.palette"}));
    assert_eq!(k["layout:flockbook"]["cmd-t"], "tab.new");
    // The temp file was in the target's folder and is gone; nothing was left beside the link.
    let left: Vec<_> = fs::read_dir(&dots).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(left, vec![std::ffi::OsString::from("keys.json")]);
    assert!(!s.app.join("keys.json.flockbook-new").exists());

    // A symlinked layout that already matches ours is written through too, and stays a link.
    let lay = s.app.join("layouts/flockbook.json");
    fs::copy(&lay, dots.join("flockbook.json")).unwrap();
    fs::remove_file(&lay).unwrap();
    std::os::unix::fs::symlink(dots.join("flockbook.json"), &lay).unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert!(fs::symlink_metadata(&lay).unwrap().file_type().is_symlink());

    // A link to a keys.json that is not there yet: it is created where the link points.
    let s = sandbox();
    let dots = s.root.join("dotfiles");
    fs::create_dir(&dots).unwrap();
    std::os::unix::fs::symlink(dots.join("keys.json"), s.app.join("keys.json")).unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert!(fs::symlink_metadata(s.app.join("keys.json")).unwrap().file_type().is_symlink());
    assert_eq!(keys_of(&s.app)["layout:flockbook"]["cmd-t"], "tab.new");
}

#[test]
fn without_home_or_dash_dash_home_it_stops_and_writes_nothing() {
    let s = sandbox();
    let r = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
        .args(["install", "--helper", s.helper.to_str().unwrap()])
        .env_clear()
        .env("PATH", s.path_env())
        .current_dir(&s.root)
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(1), "{}", both(&r));
    assert!(err(&r).contains("--home"), "{}", err(&r));
    assert!(!s.root.join(".config").exists());
    assert!(s.calls().is_empty());
    let e = Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
        .args(["install", "--dry-run"])
        .env_clear()
        .env("PATH", s.path_env())
        .env("HOME", "")
        .current_dir(&s.root)
        .output()
        .unwrap();
    assert_eq!(e.status.code(), Some(1), "{}", both(&e));
}

#[test]
fn installing_again_updates_its_own_files_keeps_edits() {
    let s = sandbox();
    assert_eq!(s.run(&["--home", s.app_s()]).status.code(), Some(0));
    let stale = "stale copy from an older release\n";
    fs::write(s.app.join("mods/flockbook/main.js"), stale).unwrap();
    let mp = s.app.join("flockbook/install.json");
    let mut marker: Value = serde_json::from_str(&read(&mp)).unwrap();
    marker["files"]["mods/flockbook/main.js"] = json!(sha(stale.as_bytes()));
    fs::write(&mp, marker.to_string()).unwrap();
    let again = s.run(&["--home", s.app_s()]);
    assert_eq!(again.status.code(), Some(0), "{}", both(&again));
    assert_eq!(read(s.app.join("mods/flockbook/main.js")), read(mod_dir().join("mods/flockbook/main.js")));

    fs::write(s.app.join("layouts/flockbook.json"), "{\"edited\": true}\n").unwrap();
    let third = s.run(&["--home", s.app_s()]);
    assert_eq!(third.status.code(), Some(1));
    assert!(err(&third).contains("layouts/flockbook.json"));
    assert_eq!(read(s.app.join("layouts/flockbook.json")), "{\"edited\": true}\n");
}

#[test]
fn installing_again_removes_its_own_dropped_files_nothing_else() {
    let s = sandbox();
    assert_eq!(s.run(&["--home", s.app_s()]).status.code(), Some(0));
    fs::write(s.app.join("mods/flockbook/old.js"), "gone\n").unwrap();
    fs::write(s.app.join("notes.txt"), "mine\n").unwrap();
    let mp = s.app.join("flockbook/install.json");
    let mut marker: Value = serde_json::from_str(&read(&mp)).unwrap();
    marker["files"]["mods/flockbook/old.js"] = json!(sha(b"gone\n"));
    marker["files"]["notes.txt"] = json!(sha(b"mine\n"));
    fs::write(&mp, marker.to_string()).unwrap();
    assert_eq!(s.run(&["--home", s.app_s()]).status.code(), Some(0));
    assert!(!s.app.join("mods/flockbook/old.js").exists());
    assert_eq!(read(s.app.join("notes.txt")), "mine\n");
}

#[test]
fn files_herdr_app_writes_into_the_mod_folder_do_not_block() {
    let s = sandbox();
    assert_eq!(s.run(&["--home", s.app_s()]).status.code(), Some(0));
    fs::write(s.app.join("mods/flockbook/jsconfig.json"), "{}\n").unwrap();
    fs::write(s.app.join("mods/flockbook/gpui-kit.d.ts"), "// types\n").unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert_eq!(read(s.app.join("mods/flockbook/jsconfig.json")), "{}\n");
}

#[test]
fn a_failed_herdr_app_check_fails_the_install() {
    let s = sandbox();
    fs::write(s.root.join("fail"), "").unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(both(&r).contains("herdr-app check"));
}

#[test]
fn without_herdr_app_on_path_it_installs_and_says_check_did_not_run() {
    let s = sandbox();
    fs::remove_file(s.root.join("bin/herdr-app")).unwrap();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert!(out(&r).contains("herdr-app is not on PATH"));
    assert!(s.app.join("mods/flockbook/mod.json").exists());
}

#[test]
fn without_helper_and_none_in_place_it_says_where_to_get_one() {
    let s = sandbox();
    let r = s.run(&["--home", s.app_s()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert!(out(&r).contains("github.com/minchenlee/flockbook/releases"));
}

#[test]
fn a_missing_helper_file_stops_before_anything() {
    let s = sandbox();
    let r = s.run(&["--home", s.app_s(), "--helper", "/nonexistent/flockbook-modfeed"]);
    assert_eq!(r.status.code(), Some(1));
    assert!(err(&r).contains("no helper at /nonexistent/flockbook-modfeed"));
    assert_eq!(fs::read_dir(&s.app).unwrap().count(), 0);
}

#[test]
fn bad_arguments_exit_2() {
    let s = sandbox();
    assert_eq!(s.run(&["--nope"]).status.code(), Some(2));
    assert_eq!(s.run(&["--home"]).status.code(), Some(2));
    let h = s.run(&["--help"]);
    assert_eq!(h.status.code(), Some(0));
    assert!(out(&h).contains("--dry-run"));
}

#[test]
fn the_home_defaults_to_herdr_app_home_then_dot_config() {
    let s = sandbox();
    let exe = env!("CARGO_BIN_EXE_flockbook-modfeed");
    let r = Command::new(exe).args(["install", "--dry-run"]).env_clear().env("PATH", s.path_env()).env("HOME", &s.home).output().unwrap();
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert!(out(&r).contains(s.home.join(".config/herdr-app").to_str().unwrap()), "{}", out(&r));
    let e = Command::new(exe)
        .args(["install", "--dry-run"])
        .env_clear()
        .env("PATH", s.path_env())
        .env("HOME", &s.home)
        .env("HERDR_APP_HOME", &s.app)
        .output()
        .unwrap();
    assert!(out(&e).contains(s.app_s()), "{}", out(&e));
}

/// A remote/ folder as the release zip has it: <target>/flockbook-modfeed and SHA256SUMS.
fn remote_dir(s: &Sandbox, corrupt: Option<&str>) -> PathBuf {
    let dir = s.root.join("remote");
    let mut sums = String::new();
    for t in ["universal-apple-darwin", "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        fs::create_dir_all(dir.join(t)).unwrap();
        let body = format!("#!/bin/sh\necho {t}\n");
        write_exe(&dir.join(t).join("flockbook-modfeed"), &body);
        let sum = if corrupt == Some(t) { sha(b"something else") } else { sha(body.as_bytes()) };
        sums.push_str(&format!("{sum}  {t}/flockbook-modfeed\n"));
    }
    fs::write(dir.join("SHA256SUMS"), sums).unwrap();
    dir
}

#[test]
fn remote_helpers_are_checked_and_copied_with_their_sums() {
    let s = sandbox();
    let dir = remote_dir(&s, None);
    let dry = s.run(&["--home", s.app_s(), "--remote", dir.to_str().unwrap(), "--dry-run"]);
    assert_eq!(dry.status.code(), Some(0), "{}", both(&dry));
    assert!(out(&dry).contains("flockbook/remote/aarch64-unknown-linux-musl/flockbook-modfeed"));
    assert_eq!(fs::read_dir(&s.app).unwrap().count(), 0);

    let r = s.run(&["--home", s.app_s(), "--remote", dir.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    for t in ["universal-apple-darwin", "x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"] {
        let p = s.app.join("flockbook/remote").join(t).join("flockbook-modfeed");
        assert_eq!(read(&p), format!("#!/bin/sh\necho {t}\n"));
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o755);
    }
    assert_eq!(read(s.app.join("flockbook/remote/SHA256SUMS")), read(dir.join("SHA256SUMS")));
}

#[test]
fn a_remote_helper_with_the_wrong_sum_stops_before_anything() {
    let s = sandbox();
    let dir = remote_dir(&s, Some("x86_64-unknown-linux-musl"));
    let r = s.run(&["--home", s.app_s(), "--remote", dir.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(err(&r).contains("x86_64-unknown-linux-musl"), "{}", err(&r));
    assert_eq!(fs::read_dir(&s.app).unwrap().count(), 0);
}

#[test]
fn a_remote_sums_line_that_leaves_the_folder_is_refused() {
    let s = sandbox();
    let dir = remote_dir(&s, None);
    fs::write(dir.join("SHA256SUMS"), format!("{}  ../evil/flockbook-modfeed\n", sha(b"x"))).unwrap();
    let r = s.run(&["--home", s.app_s(), "--remote", dir.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));
    assert_eq!(fs::read_dir(&s.app).unwrap().count(), 0);
}

#[test]
fn a_remote_sums_file_naming_a_target_twice_is_refused() {
    let s = sandbox();
    let dir = remote_dir(&s, None);
    let sums = read(dir.join("SHA256SUMS"));
    let first = sums.lines().next().unwrap().to_string();
    fs::write(dir.join("SHA256SUMS"), format!("{sums}{first}\n")).unwrap();
    let r = s.run(&["--home", s.app_s(), "--remote", dir.to_str().unwrap()]);
    assert_eq!(r.status.code(), Some(1));
    assert!(err(&r).contains("twice"), "{}", err(&r));
    assert_eq!(fs::read_dir(&s.app).unwrap().count(), 0);
}

// The phone page runs the installed helper as a user service (Flockbook settings, `phone on`): an install that
// replaces the helper restarts it, only when the service file is the settings' own and names this helper.
#[test]
fn an_install_restarts_the_phone_page_that_runs_this_helper() {
    let s = sandbox();
    let bin = s.root.join("bin");
    write_exe(&bin.join("systemctl"), &format!("#!/bin/sh\necho \"systemctl $*\" >> \"{}\"\n", s.root.join("calls").display()));
    write_exe(&bin.join("id"), "#!/bin/sh\necho 501\n");
    let unit = s.home.join(".config/systemd/user/flockbook-serve.service");
    fs::create_dir_all(unit.parent().unwrap()).unwrap();
    let helper = s.app.join("flockbook/bin/flockbook-modfeed");
    let install = |s: &Sandbox| {
        Command::new(env!("CARGO_BIN_EXE_flockbook-modfeed"))
            .args(["install", "--home", s.app_s(), "--helper", s.helper.to_str().unwrap()])
            .env_clear()
            .env("PATH", s.path_env())
            .env("HOME", &s.home)
            .env("FLOCKBOOK_PHONE_BIN", &bin)
            .env("FLOCKBOOK_PHONE_SERVICE", "systemd")
            .output()
            .unwrap()
    };
    // Set up by hand (no marker): left alone.
    fs::write(&unit, format!("[Service]\nExecStart=\"{}\" \"serve\"\n", helper.display())).unwrap();
    let r = install(&s);
    assert_eq!(r.status.code(), Some(0), "{}", both(&r));
    assert!(!s.calls().iter().any(|c| c.starts_with("systemctl")), "{:?}", s.calls());
    // The settings' own, for another home's helper: left alone.
    fs::write(&unit, format!("# {}\n[Service]\nExecStart=\"/elsewhere/flockbook-modfeed\" \"serve\"\n", modfeedkit::phone::MARKER)).unwrap();
    install(&s);
    assert!(!s.calls().iter().any(|c| c.starts_with("systemctl")), "{:?}", s.calls());
    // The settings' own, running this helper: restarted.
    fs::write(&unit, format!("# {}\n[Service]\nExecStart=\"{}\" \"serve\"\n", modfeedkit::phone::MARKER, helper.display())).unwrap();
    let r = install(&s);
    assert!(s.calls().iter().any(|c| c == "systemctl --user try-restart flockbook-serve"), "{:?}", s.calls());
    assert!(out(&r).contains("the phone page (Flockbook settings): restarted"), "{}", out(&r));
}
