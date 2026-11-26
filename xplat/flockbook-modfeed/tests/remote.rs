//! A terminal `m/tN` on another machine: the helper
//! runs the same command there with `machine.exec` and gives its lines back as they are. Installing that helper:
//! only after the user said yes (the sha256 shown on the card), checked here and there. Two fake cores: this one
//! (`here`) and the machine's (`there`); `machine.exec` runs the command on this computer in the machine's home.
#![cfg(unix)]

mod common;
use common::{claude, FakeHc};
use modfeedkit::remote;
use serde_json::{json, Value};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_flockbook-modfeed");

/// The helper this Flockbook carries for other machines: FLOCKBOOK_TEST_HELPER (CI: the real musl release build),
/// else this test build.
fn carried() -> PathBuf {
    std::env::var_os("FLOCKBOOK_TEST_HELPER").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(BIN))
}
const BOX: &str = "Bash command\n  rm -rf build\nDo you want to proceed?\n❯ 1. Yes\n  2. No";

/// The helper there, from that machine's home (`machine.exec` runs in the home).
fn helper_rel() -> String {
    format!("{}/{}/flockbook-modfeed", remote::HELPER_DIR, remote::helper_version())
}

struct World {
    here: FakeHc,
    there: FakeHc,
    _dir: tempfile::TempDir,
    home: PathBuf,
    /// FLOCKBOOK_REMOTE_HELPERS: the helpers this Flockbook carries for other machines.
    helpers: PathBuf,
}

fn sha256(path: &Path) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(std::fs::read(path).unwrap());
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn uname() -> String {
    String::from_utf8(Command::new("uname").arg("-sm").output().unwrap().stdout).unwrap().trim().to_string()
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("box-home");
    std::fs::create_dir_all(&home).unwrap();
    let here = FakeHc::start();
    let there = FakeHc::start();
    there.set_terminals(vec![claude("t3", "blocked", "permission")]);
    there.set_screen("t3", BOX);
    here.add_machine("box", &home, &there.path);
    // This Flockbook carries the helper for this computer's platform (the test binary itself).
    let helpers = dir.path().join("remote");
    let target = remote::target_for(&uname()).expect("a platform the helper ships for");
    std::fs::create_dir_all(helpers.join(target)).unwrap();
    std::fs::copy(carried(), helpers.join(target).join("flockbook-modfeed")).unwrap();
    let sum = sha256(&helpers.join(target).join("flockbook-modfeed"));
    std::fs::write(helpers.join("SHA256SUMS"), format!("{sum}  {target}/flockbook-modfeed\n")).unwrap();
    World { here, there, _dir: dir, home, helpers }
}

impl World {
    fn run(&self, args: &[&str]) -> Vec<Value> {
        let out = Command::new(BIN)
            .args(args)
            .env_clear()
            .env("HC_SOCKET", &self.here.path)
            .env("HOME", "/nonexistent-home")
            .env("FLOCKBOOK_REMOTE_HELPERS", &self.helpers)
            .output()
            .unwrap();
        assert!(out.status.success(), "exit code 0 always");
        String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }
    fn installed(&self) -> PathBuf {
        self.home.join(remote::HELPER_DIR).join(remote::helper_version()).join("flockbook-modfeed")
    }
    fn install_by_hand(&self) {
        std::fs::create_dir_all(self.installed().parent().unwrap()).unwrap();
        std::fs::copy(carried(), self.installed()).unwrap();
    }
    fn error(&self, args: &[&str]) -> Value {
        let v = self.run(args);
        assert_eq!(v.last().unwrap()["ok"], false, "{v:?}");
        v.iter().find(|l| l["type"] == "error").cloned().unwrap_or_else(|| panic!("an error line: {v:?}"))
    }
}

#[test]
fn a_terminal_on_another_machine_runs_the_helper_there() {
    let w = world();
    w.install_by_hand();
    let c = w.run(&["card", "--terminal", "box/t3", "--cache", "/nonexistent-home/cache.json"]);
    assert_eq!(c[0]["type"], "header");
    assert_eq!(c[1]["type"], "card", "{c:?}");
    assert_eq!(c[1]["command"], "rm -rf build");
    assert!(w.there.calls("terminal.read").iter().any(|p| p["terminal"] == "t3"), "the helper there read t3 on its core");
    assert!(w.here.calls("terminal.read").is_empty(), "nothing read here");
    let exec = w.here.calls("machine.exec");
    assert_eq!(exec.len(), 2, "the platform first, then the command: {exec:?}");
    assert_eq!(exec[0]["argv"], json!(["uname", "-sm"]));
    assert_eq!(exec[1]["on"], "box");
    assert!(exec.iter().all(|e| e.get("line").is_none()), "no shell line carries a value: {exec:?}");
    let script = format!("c=$1; shift; exec \"$HOME\"/{} \"$c\" --cache \"$HOME\"/.cache/flockbook/cache.json \"$@\"", helper_rel());
    assert_eq!(
        exec[1]["argv"],
        json!(["sh", "-c", script, "sh", "card", "--terminal", "t3"]),
        "a fixed script finds the helper under $HOME; the address there is t3; this computer's cache path is not used there"
    );

    // Approve there: the key goes to t3 on that core, and the claim is kept in that machine's home.
    let req = c[1]["request"].as_str().unwrap().to_string();
    let a = w.run(&["approve", "--terminal", "box/t3", "--request", &req, "--allow", "--cache", "/nonexistent-home/cache.json"]);
    assert_eq!(a[1], json!({"type": "done", "result": "sent"}), "{a:?}");
    assert_eq!(w.there.keys(), vec![vec!["1".to_string()]]);
    assert!(w.here.keys().is_empty());
    assert!(w.home.join(".cache/flockbook/answered").is_dir(), "claims live on that machine");
    let again = w.run(&["approve", "--terminal", "box/t3", "--request", &req, "--allow", "--cache", "/nonexistent-home/cache.json"]);
    assert_eq!(again[1]["result"], "answered", "one answer per prompt there too");
}

#[test]
fn no_helper_there_gives_what_the_card_shows() {
    let w = world();
    let e = w.error(&["card", "--terminal", "box/t3"]);
    assert_eq!(e["code"], "remote_helper_missing", "{e}");
    let i = &e["install"];
    let target = remote::target_for(&uname()).unwrap();
    assert_eq!(i["machine"], "box");
    assert_eq!(i["target"], target);
    assert_eq!(i["path"], format!("~/{}/{}/flockbook-modfeed", remote::HELPER_DIR, remote::helper_version()));
    assert_eq!(i["bytes"], std::fs::metadata(carried()).unwrap().len());
    assert_eq!(i["sha256"], sha256(&carried()));
    assert_eq!(i["source"], "dev", "FLOCKBOOK_REMOTE_HELPERS is a development source");
    assert!(w.there.calls("terminal.read").is_empty());
    assert!(w.here.calls("file.write").is_empty(), "nothing is put there without the user's yes");
}

#[test]
fn install_after_yes_checks_here_and_there() {
    let w = world();
    let sum = sha256(&carried());
    let wrong = w.error(&["helper", "install", "--machine", "box", "--sha256", &"0".repeat(64)]);
    assert_eq!(wrong["code"], "checksum", "not the file the user said yes to");
    assert!(w.here.calls("file.write").is_empty() && !w.installed().exists());

    let ok = w.run(&["helper", "install", "--machine", "box", "--sha256", &sum]);
    let line = ok.iter().find(|l| l["type"] == "installed").unwrap_or_else(|| panic!("{ok:?}"));
    assert_eq!(line["machine"], "box");
    assert_eq!(line["sha256"], sum);
    let (bytes, parts) = (line["bytes"].as_u64().unwrap(), line["parts"].as_u64().unwrap());
    assert_eq!(parts, bytes.div_ceil(remote::MAX_WRITE), "parts of at most {} bytes (hc's file.write takes 8 MB)", remote::MAX_WRITE);
    println!("helper put there: {bytes} bytes in {parts} part(s)");
    assert_eq!(sha256(&w.installed()), sum);
    assert_eq!(std::fs::metadata(w.installed()).unwrap().permissions().mode() & 0o777, 0o755);
    let names: Vec<_> = std::fs::read_dir(w.installed().parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names.len(), 1, "no .part left ({names:?})");
    assert_eq!(w.run(&["card", "--terminal", "box/t3"])[1]["type"], "card", "now it runs there");

    let gone = w.run(&["helper", "remove", "--machine", "box"]);
    assert!(gone.iter().any(|l| l["type"] == "removed"), "{gone:?}");
    assert!(!w.home.join(remote::HELPER_DIR).exists());
    assert_eq!(w.error(&["card", "--terminal", "box/t3"])["code"], "remote_helper_missing");
}

#[test]
fn a_file_changed_after_the_checksum_is_not_installed() {
    let w = world();
    let sum = sha256(&carried());
    let target = remote::target_for(&uname()).unwrap();
    std::fs::write(w.helpers.join(target).join("flockbook-modfeed"), b"not the helper").unwrap();
    let e = w.error(&["helper", "install", "--machine", "box", "--sha256", &sum]);
    assert_eq!(e["code"], "checksum", "{e}");
    assert!(w.here.calls("file.write").is_empty());
}

#[test]
fn another_version_there_is_an_error_not_a_guess() {
    let w = world();
    std::fs::create_dir_all(w.installed().parent().unwrap()).unwrap();
    for (protocol, sidecar) in [(2, remote::helper_version().to_string()), (1, "0.0.9".to_string())] {
        let fake = format!(
            "#!/bin/sh\necho '{{\"cmd\":\"card\",\"protocol\":{protocol},\"sidecar\":\"{sidecar}\",\"type\":\"header\"}}'\necho '{{\"ms\":0,\"ok\":true,\"type\":\"end\"}}'\n"
        );
        std::fs::write(w.installed(), fake).unwrap();
        std::fs::set_permissions(w.installed(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let e = w.error(&["card", "--terminal", "box/t3"]);
        assert_eq!(e["code"], "remote_helper_outdated", "{protocol} {sidecar}: {e}");
        assert_eq!(e["remote"]["protocol"], protocol);
        assert_eq!(e["remote"]["sidecar"], sidecar);
        assert_eq!(e["install"]["machine"], "box", "the card can offer the right version");
    }
}

#[test]
fn a_platform_without_a_helper_is_unsupported() {
    let w = world();
    for (code, out) in [(127, ""), (0, "Linux riscv64"), (0, "MINGW64_NT-10.0 x86_64")] {
        w.here.state.lock().unwrap().uname = Some((code, out.into()));
        let e = w.error(&["card", "--terminal", "box/t3"]);
        assert_eq!(e["code"], "remote_helper_unsupported", "{code} {out}: {e}");
        assert!(e["message"].as_str().unwrap().contains("box"));
    }
}

#[test]
fn values_reach_the_helper_there_unchanged() {
    let w = world();
    w.install_by_hand();
    w.there.set_terminals(vec![claude("t3", "idle", "")]);
    let text = "Tom & Jerry | a; b $(id) `id` %USERPROFILE% %PATH% 'single' \"double\" back\\slash\nnext line *?[x] ~ #";
    let d = w.run(&["send", "--terminal", "box/t3", "--text", text]);
    assert_eq!(d[1]["result"], "sent", "{d:?}");
    let exec = w.here.calls("machine.exec");
    let fwd = exec.last().unwrap();
    assert!(fwd.get("line").is_none(), "no shell: {fwd}");
    let argv: Vec<&str> = fwd["argv"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    let at = argv.iter().position(|a| *a == "--text").unwrap();
    assert_eq!(argv[at + 1], text, "the value is one argument, as typed");
    assert_eq!(w.there.calls("terminal.send")[0]["text"], text, "the agent there gets exactly the text");
}

#[test]
fn the_helper_there_is_found_from_any_cwd() {
    let w = world();
    w.install_by_hand();
    w.here.state.lock().unwrap().exec_in_root = true;
    w.there.set_terminals(vec![claude("t3", "idle", "")]);
    let text = "a $HOME ~ \"$@\" $1 'x'";
    let d = w.run(&["send", "--terminal", "box/t3", "--text", text]);
    assert_eq!(d[1]["result"], "sent", "exec started in / and the helper still ran: {d:?}");
    assert_eq!(w.there.calls("terminal.send")[0]["text"], text, "the agent there gets exactly the text");
    let c = w.run(&["card", "--terminal", "box/t3", "--cache", "/here/cache.json"]);
    assert!(c.iter().all(|l| l["type"] != "error"), "{c:?}");
}

#[test]
fn a_windows_machine_runs_nothing_with_a_value() {
    let w = world();
    w.here.state.lock().unwrap().windows = true;
    let sum = sha256(&carried());
    for args in [
        vec!["card", "--terminal", "box/t3"],
        vec!["send", "--terminal", "box/t3", "--text", "build it & del /q %USERPROFILE%\\notes.txt & rem"],
        vec!["helper", "install", "--machine", "box", "--sha256", sum.as_str()],
        vec!["helper", "remove", "--machine", "box"],
    ] {
        let e = w.error(&args);
        assert_eq!(e["code"], "remote_helper_unsupported", "{args:?}: {e}");
        assert!(e["message"].as_str().unwrap().contains("box"));
    }
    let exec = w.here.calls("machine.exec");
    assert!(exec.iter().all(|e| e["argv"] == json!(["uname", "-sm"])), "only the platform check ran there: {exec:?}");
    assert!(w.here.calls("file.write").is_empty() && w.here.calls("terminal.put").is_empty());
}

#[test]
fn the_sum_there_and_making_it_runnable_are_one_command() {
    let w = world();
    let sum = sha256(&carried());
    let ok = w.run(&["helper", "install", "--machine", "box", "--sha256", &sum]);
    assert!(ok.iter().any(|l| l["type"] == "installed"), "{ok:?}");
    let writes = w.here.calls("file.write");
    let name = writes[0]["path"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
    let tag = name.strip_prefix("flockbook-modfeed.part.").and_then(|r| r.strip_suffix(".0")).unwrap_or("");
    assert!(!tag.is_empty() && !tag.contains('.'), "parts carry this run's own tag: {name}");
    let lines: Vec<String> = w.here.calls("machine.exec").iter().filter_map(|e| e["line"].as_str().map(String::from)).collect();
    assert_eq!(lines.len(), 1, "one command joins, checks and switches: {lines:?}");
    assert!(lines[0].contains(&sum) && lines[0].contains("chmod 755") && lines[0].contains(tag), "{}", lines[0]);
}

#[test]
fn a_file_changed_on_the_way_is_not_installed() {
    let w = world();
    w.here.state.lock().unwrap().corrupt_writes = true;
    let e = w.error(&["helper", "install", "--machine", "box", "--sha256", &sha256(&carried())]);
    assert_eq!(e["code"], "checksum", "{e}");
    assert!(!w.installed().exists());
    let left: Vec<_> = std::fs::read_dir(w.installed().parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert!(left.is_empty(), "no part is left: {left:?}");
}

#[test]
fn sha256sums_naming_the_helper_twice_is_refused_and_dot_slash_is_fine() {
    let w = world();
    let target = remote::target_for(&uname()).unwrap();
    let sum = sha256(&carried());
    std::fs::write(w.helpers.join("SHA256SUMS"), format!("{sum}  ./{target}/flockbook-modfeed\n")).unwrap();
    assert_eq!(w.error(&["card", "--terminal", "box/t3"])["install"]["sha256"], sum, "./ names the same file");
    for second in [sum.clone(), "0".repeat(64)] {
        std::fs::write(w.helpers.join("SHA256SUMS"), format!("{sum}  {target}/flockbook-modfeed\n{second}  ./{target}/flockbook-modfeed\n")).unwrap();
        let e = w.error(&["card", "--terminal", "box/t3"]);
        assert_eq!(e["code"], "checksum", "two lines for one file: {e}");
        assert!(e.get("install").is_none());
    }
}

#[test]
fn the_platform_and_a_missing_helper_are_remembered_for_a_minute() {
    let w = world();
    let cache = w._dir.path().join("cache/cache.json");
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    let c = cache.to_string_lossy().to_string();
    for _ in 0..3 {
        assert_eq!(w.error(&["feed", "--terminal", "box/t3", "--cache", &c])["code"], "remote_helper_missing");
    }
    let exec = w.here.calls("machine.exec");
    assert_eq!(exec.iter().filter(|e| e["argv"] == json!(["uname", "-sm"])).count(), 1, "uname once: {exec:?}");
    assert_eq!(exec.len(), 2, "the helper is looked for once; the next polls answer from the cache: {exec:?}");
    let ok = w.run(&["helper", "install", "--machine", "box", "--sha256", &sha256(&carried()), "--cache", &c]);
    assert!(ok.iter().any(|l| l["type"] == "installed"), "{ok:?}");
    assert_eq!(w.run(&["card", "--terminal", "box/t3", "--cache", &c])[1]["type"], "card", "an install forgets the missing helper");
}

#[test]
fn this_flockbook_without_the_helper_for_that_platform() {
    let w = world();
    std::fs::remove_dir_all(&w.helpers).unwrap();
    let e = w.error(&["card", "--terminal", "box/t3"]);
    assert_eq!(e["code"], "remote_helper_missing");
    assert_eq!(e["install"], Value::Null, "nothing to install: {e}");
}

#[test]
fn time_out_unknown_machine_and_bad_address() {
    let w = world();
    w.install_by_hand();
    w.here.state.lock().unwrap().exec_timed_out = true;
    assert_eq!(w.error(&["card", "--terminal", "box/t3"])["code"], "remote_failed");
    w.here.state.lock().unwrap().exec_timed_out = false;
    assert_eq!(w.error(&["card", "--terminal", "nobox/t3"])["code"], "remote_failed");
    assert_eq!(w.error(&["card", "--terminal", "a/b/t3"])["code"], "bad_terminal");
    assert_eq!(w.error(&["card", "--terminal", "/t3"])["code"], "bad_terminal");
}

#[test]
fn send_with_a_file_puts_it_there_first() {
    let w = world();
    w.install_by_hand();
    w.there.set_terminals(vec![claude("t3", "idle", "")]);
    let pic = w._dir.path().join("shot one.png");
    std::fs::write(&pic, b"png").unwrap();
    let p = pic.to_string_lossy().to_string();
    let d = w.run(&["send", "--terminal", "box/t3", "--text", &format!("look at '{p}'"), "--file", &p]);
    assert_eq!(d[1]["result"], "sent", "{d:?}");
    let put = w.here.calls("terminal.put");
    assert_eq!(put.len(), 1, "the file is put beside box/t3 from here");
    assert_eq!(put[0]["terminal"], "box/t3");
    let sent = w.there.calls("terminal.send");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["text"], "look at '/remote/tmp/shot one.png'", "the text names the copy there");
}
