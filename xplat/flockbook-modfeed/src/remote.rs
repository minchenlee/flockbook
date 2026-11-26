//! Terminals on other machines. A terminal `m/tN` is on machine `m`: the same
//! helper runs there (`machine.exec`) with the same command and `--terminal tN`, and its lines come back as they are,
//! so the chat reads every log with one parser. Its core there is its own (hc gives it `HC_SESSION`).
//!
//! The helper there is this same version, never another (`remote_helper_outdated` otherwise). Putting it there is
//! the user's choice: `remote_helper_missing` carries what the mod shows (machine, path, size, sha256), and
//! `helper install` runs only with that sha256. The file comes from the folder this Flockbook was installed with
//! (`<home>/flockbook/remote/<target>/flockbook-modfeed`, `SHA256SUMS` beside it), checked here and again there.
//!
//! No shell gets a value: a command for the helper there is `machine.exec` `argv` (hc runs a `line` in cmd.exe on
//! Windows, where POSIX quotes mean nothing). The platform is known before anything with a value runs there, and a
//! Windows machine gets nothing but `uname`. The fixed lines of install and remove run only on Unix.

use crate::protocol::{JsonLines, PROTOCOL_VERSION, SIDECAR_VERSION};
use herdr_core::hc::HcClient;
use herdr_core::HerdrError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Under the home there.
pub const HELPER_DIR: &str = ".local/share/flockbook/helper";
/// The cache (and the claims beside it) there, under the home.
const CACHE: &str = ".cache/flockbook/cache.json";
/// One `file.write` takes at most 8 MB; the helper goes in parts no larger than this.
pub const MAX_WRITE: u64 = 6 * 1024 * 1024;
/// A command there: the longest a helper command takes (a send waits up to 22 s) and a margin.
const EXEC_TIMEOUT_S: u64 = 30;
/// How long a machine's platform, and that it has no helper, are remembered (the chat polls every 2 s).
const REMEMBER_S: f64 = 60.0;

pub fn helper_version() -> &'static str {
    SIDECAR_VERSION
}

fn helper_rel() -> String {
    format!("{HELPER_DIR}/{}/flockbook-modfeed", helper_version())
}

/// The fixed `sh -c` script for a forward: "$1" is the command, the rest are the values. With `cached`, the
/// cache there (under $HOME) comes right after the command, so a value cannot stand in for it.
fn forward_script(cached: bool) -> String {
    let cache = if cached { format!(" --cache \"$HOME\"/{CACHE}") } else { String::new() };
    format!("c=$1; shift; exec \"$HOME\"/{} \"$c\"{cache} \"$@\"", helper_rel())
}

/// `uname -sm` there -> the helper built for it. None: no helper for that platform (Windows has no `uname`).
pub fn target_for(uname: &str) -> Option<&'static str> {
    let mut it = uname.split_whitespace();
    match (it.next()?, it.next()?) {
        ("Linux", "x86_64") => Some("x86_64-unknown-linux-musl"),
        ("Linux", "aarch64" | "arm64") => Some("aarch64-unknown-linux-musl"),
        ("Darwin", "arm64" | "x86_64") => Some("universal-apple-darwin"),
        _ => None,
    }
}

/// `m/tN` -> Some((m, tN)); a local address -> None. Err: not one machine and one terminal.
#[allow(clippy::result_unit_err)]
pub fn split(addr: &str) -> Result<Option<(String, String)>, ()> {
    let a = addr.strip_prefix("hc:").unwrap_or(addr);
    let parts: Vec<&str> = a.split('/').collect();
    match parts.as_slice() {
        [_] => Ok(None),
        [m, t] if !m.is_empty() && !t.is_empty() => Ok(Some((m.to_string(), t.to_string()))),
        _ => Err(()),
    }
}

/// What the mod shows before the user says yes.
#[derive(Serialize, Clone, Debug)]
pub struct Install {
    pub machine: String,
    pub target: String,
    /// Where it goes there.
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    /// "release": the folder installed with this Flockbook; "dev": FLOCKBOOK_REMOTE_HELPERS (tests, local builds).
    pub source: &'static str,
}

struct Fail {
    code: &'static str,
    message: String,
}

fn fail(code: &'static str, message: impl Into<String>) -> Fail {
    Fail { code, message: message.into() }
}

struct Ran {
    code: Option<i64>,
    stdout: Vec<u8>,
    stderr: String,
    /// argv: the program is not there (hc `spawn_failed`).
    no_program: bool,
}

/// What runs there: a program and its arguments (no shell), or a fixed line for `sh -c` (Unix only).
enum Run<'a> {
    Argv(&'a [String]),
    Line(&'a str),
}

fn exec(client: &HcClient, machine: &str, run: Run) -> Result<Ran, Fail> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let mut p = json!({"on": machine, "timeout": EXEC_TIMEOUT_S});
    match run {
        Run::Argv(a) => p["argv"] = json!(a),
        Run::Line(l) => p["line"] = json!(l),
    }
    let r = match client.call("machine.exec", p, Some(Duration::from_secs(EXEC_TIMEOUT_S + 5))) {
        Ok(r) => r,
        Err(HerdrError::Server { code, .. }) if code == "spawn_failed" => {
            return Ok(Ran { code: None, stdout: vec![], stderr: String::new(), no_program: true });
        }
        Err(e) => return Err(fail("remote_failed", format!("{machine}: {e}"))),
    };
    if r.get("timed_out").and_then(Value::as_bool) == Some(true) {
        return Err(fail("remote_failed", format!("{machine} did not answer in {EXEC_TIMEOUT_S} s")));
    }
    if r.get("truncated").and_then(Value::as_bool) == Some(true) {
        return Err(fail("remote_failed", format!("{machine} answered more than hc passes on (8 MB)")));
    }
    let stdout = b64.decode(r.get("stdout").and_then(Value::as_str).unwrap_or("")).unwrap_or_default();
    let stderr = b64.decode(r.get("stderr").and_then(Value::as_str).unwrap_or("")).unwrap_or_default();
    Ok(Ran { code: r.get("code").and_then(Value::as_i64), stdout, stderr: String::from_utf8_lossy(&stderr).trim().to_string(), no_program: false })
}

fn now_s() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// What this computer remembers about other machines, in `remote.json` beside the chat's cache. Without a cache
/// path nothing is remembered. A lost or broken file only costs one more check.
#[derive(Serialize, Deserialize, Default)]
struct Memo {
    /// machine -> its platform: the helper target, or None (unsupported: `uname` says which).
    #[serde(default)]
    platform: HashMap<String, Seen>,
    /// machine -> when the helper was found missing there.
    #[serde(default)]
    missing: HashMap<String, f64>,
    /// target -> the sha256 of the local file, while its path, size and mtime are the same.
    #[serde(default)]
    sums: HashMap<String, LocalSum>,
    #[serde(skip)]
    path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Seen {
    at: f64,
    target: Option<String>,
    uname: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
struct LocalSum {
    path: String,
    size: u64,
    mtime_ns: u128,
    sha256: String,
}

impl Memo {
    fn load(cache: Option<&Path>) -> Memo {
        let Some(path) = cache.map(|c| c.with_file_name("remote.json")) else { return Memo::default() };
        let mut m: Memo = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        m.path = Some(path);
        m
    }
    fn save(&self) {
        let Some(path) = &self.path else { return };
        let Ok(text) = serde_json::to_vec(self) else { return };
        let tmp = path.with_extension(format!("json.{}", std::process::id()));
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    fn missing_now(&self, machine: &str) -> bool {
        self.missing.get(machine).is_some_and(|at| now_s() - at < REMEMBER_S)
    }
}

fn unsupported(machine: &str, uname: &str) -> Fail {
    let what = if uname.is_empty() { "a system without uname (Windows?)".to_string() } else { uname.to_string() };
    fail(
        "remote_helper_unsupported",
        format!("Flockbook cannot show the chat of agents on {machine}: it runs {what}, and the helper is only for Linux (x86_64, aarch64) and macOS."),
    )
}

/// The helper target for `machine`, found with `uname -sm` (argv: no shell on any platform) or remembered.
/// Err: unsupported (Windows, another CPU), or the machine did not answer. Nothing with a value runs before this.
fn platform(client: &HcClient, machine: &str, memo: &mut Memo) -> Result<&'static str, Fail> {
    let seen = match memo.platform.get(machine).filter(|s| now_s() - s.at < REMEMBER_S) {
        Some(s) => s.clone(),
        None => {
            let argv = ["uname".to_string(), "-sm".to_string()];
            let u = exec(client, machine, Run::Argv(&argv))?;
            let uname = String::from_utf8_lossy(&u.stdout).trim().to_string();
            let target = (u.code == Some(0)).then(|| target_for(&uname)).flatten().map(String::from);
            let s = Seen { at: now_s(), target, uname };
            memo.platform.insert(machine.into(), s.clone());
            memo.save();
            s
        }
    };
    match seen.target.as_deref().and_then(|t| target_for_name(t)) {
        Some(t) => Ok(t),
        None => Err(unsupported(machine, &seen.uname)),
    }
}

fn target_for_name(t: &str) -> Option<&'static str> {
    ["x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl", "universal-apple-darwin"].into_iter().find(|x| *x == t)
}

/// The folder of helpers for other machines, and whether it is the one installed with this Flockbook.
fn helpers_dir() -> Option<(PathBuf, &'static str)> {
    if let Some(d) = std::env::var_os("FLOCKBOOK_REMOTE_HELPERS").filter(|d| !d.is_empty()) {
        return Some((PathBuf::from(d), "dev"));
    }
    // <home>/flockbook/bin/flockbook-modfeed -> <home>/flockbook/remote
    let exe = std::env::current_exe().ok()?;
    Some((exe.parent()?.parent()?.join("remote"), "release"))
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// The sha256 that `SHA256SUMS` (beside the target folders, or in the target's folder) gives for the helper.
/// Ok(None): not listed. Err: the file names the helper more than once, or the sum is not 64 hex digits.
fn listed_sum(dir: &Path, target: &str) -> Result<Option<String>, String> {
    let lists = [
        (dir.join("SHA256SUMS"), format!("{target}/flockbook-modfeed")),
        (dir.join(target).join("SHA256SUMS"), "flockbook-modfeed".to_string()),
    ];
    for (file, name) in lists {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let found: Vec<&str> = text
            .lines()
            .filter_map(|line| {
                let mut it = line.split_whitespace();
                let (sum, n) = (it.next()?, it.next()?);
                let n = n.trim_start_matches('*');
                (n.strip_prefix("./").unwrap_or(n) == name).then_some(sum)
            })
            .collect();
        match found.as_slice() {
            [] => continue,
            [sum] if sum.len() == 64 && sum.chars().all(|c| c.is_ascii_hexdigit()) => return Ok(Some(sum.to_ascii_lowercase())),
            [_] => return Err(format!("{} has a bad sum for {name}", file.display())),
            _ => return Err(format!("{} names {name} more than once", file.display())),
        }
    }
    Ok(None)
}

/// The local helper for `target`: the folder, its source, the file. None: this Flockbook has none for it.
fn local_file(target: &str) -> Option<(PathBuf, &'static str, PathBuf)> {
    let (dir, source) = helpers_dir()?;
    let file = dir.join(target).join("flockbook-modfeed");
    file.is_file().then_some((dir, source, file))
}

fn install_info(machine: &str, target: &str, bytes: u64, sha256: String, source: &'static str) -> Install {
    Install { machine: machine.into(), target: target.into(), path: format!("~/{}", helper_rel()), bytes, sha256, source }
}

fn checked(dir: &Path, target: &str, sum: &str) -> Result<(), Fail> {
    match listed_sum(dir, target) {
        Ok(Some(listed)) if listed == sum => Ok(()),
        Ok(_) => Err(fail("checksum", format!("The helper for {target} in this Flockbook does not match its SHA256SUMS. Reinstall Flockbook."))),
        Err(why) => Err(fail("checksum", format!("The helper for {target} in this Flockbook cannot be checked: {why}. Reinstall Flockbook."))),
    }
}

/// What the card shows for `machine`. The local sum is remembered while the file's path, size and mtime stay the
/// same (install reads and checks the whole file again). Ok(None): this Flockbook has no helper for that platform.
fn card_info(client: &HcClient, machine: &str, memo: &mut Memo) -> Result<Option<Install>, Fail> {
    let target = platform(client, machine, memo)?;
    let Some((dir, source, file)) = local_file(target) else { return Ok(None) };
    let meta = std::fs::metadata(&file).map_err(|e| fail("checksum", format!("{}: {e}", file.display())))?;
    let mtime_ns = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    let key = |sha256: String| LocalSum { path: file.to_string_lossy().into_owned(), size: meta.len(), mtime_ns, sha256 };
    let sum = match memo.sums.get(target).filter(|s| **s == key(s.sha256.clone())) {
        Some(s) => s.sha256.clone(),
        None => {
            let data = std::fs::read(&file).map_err(|e| fail("checksum", format!("{}: {e}", file.display())))?;
            let sum = sha256_hex(&data);
            memo.sums.insert(target.into(), key(sum.clone()));
            memo.save();
            sum
        }
    };
    checked(&dir, target, &sum)?;
    Ok(Some(install_info(machine, target, meta.len(), sum, source)))
}

/// What would go to `machine`, read whole and checked here now. Ok(None): no helper for that platform.
fn prepare(client: &HcClient, machine: &str, memo: &mut Memo) -> Result<Option<(Install, Vec<u8>)>, Fail> {
    let target = platform(client, machine, memo)?;
    let Some((dir, source, file)) = local_file(target) else { return Ok(None) };
    let Ok(data) = std::fs::read(&file) else { return Ok(None) };
    let sum = sha256_hex(&data);
    checked(&dir, target, &sum)?;
    Ok(Some((install_info(machine, target, data.len() as u64, sum, source), data)))
}

fn header(out: &mut JsonLines, cmd: &str) {
    out.emit_value(json!({"type": "header", "protocol": PROTOCOL_VERSION, "sidecar": SIDECAR_VERSION, "cmd": cmd}));
}

fn error_lines(cmd: &str, f: Fail, extra: Value) -> String {
    let mut out = JsonLines::new();
    header(&mut out, cmd);
    let mut e = json!({"type": "error", "code": f.code, "message": f.message});
    if let (Some(e), Some(x)) = (e.as_object_mut(), extra.as_object()) {
        e.extend(x.clone());
    }
    out.emit_value(e);
    out.emit_value(json!({"type": "end", "ok": false, "ms": 0}));
    out.text()
}

/// The error with the install card's data: `install` (null when this Flockbook has nothing to put there), or the
/// error that stops it (an unsupported platform, a bad checksum).
fn with_install(client: &HcClient, cmd: &str, machine: &str, f: Fail, mut extra: Value, memo: &mut Memo) -> String {
    match card_info(client, machine, memo) {
        Ok(i) => {
            extra["install"] = serde_json::to_value(i).unwrap_or(Value::Null);
            error_lines(cmd, f, extra)
        }
        Err(stop) => error_lines(cmd, stop, json!({})),
    }
}

fn missing(machine: &str) -> Fail {
    fail("remote_helper_missing", format!("{machine} has no Flockbook helper yet, so the chat cannot read its agents."))
}

/// `cmd` with `args` (as given to this helper) for the terminal `terminal` on `machine`: the lines of the helper
/// there, as they are. `cache`: this computer's chat cache (what is remembered about machines goes beside it).
pub fn forward(client: &HcClient, cmd: &str, args: &[String], machine: &str, terminal: &str, cache: Option<&Path>) -> String {
    let mut memo = Memo::load(cache);
    // The platform first: a Windows machine gets no command with a value.
    if let Err(f) = platform(client, machine, &mut memo) {
        return error_lines(cmd, f, json!({}));
    }
    if memo.missing_now(machine) {
        return with_install(client, cmd, machine, missing(machine), json!({}), &mut memo);
    }
    // The platform is Unix now. `sh -c` with a fixed script finds the helper under $HOME (exec's cwd is not
    // promised); every value goes in "$@" as one argument and the shell never reads it.
    let mut cached = false;
    let mut there: Vec<String> = vec![cmd.to_string()];
    let mut files: Vec<String> = vec![];
    let mut text: Option<(usize, String)> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--terminal" => {
                it.next();
                there.extend(["--terminal".to_string(), terminal.to_string()]);
            }
            // This computer's cache path means nothing there: the helper there keeps its own.
            "--cache" => {
                it.next();
                cached = true;
            }
            // This computer's herdr-app home (its config.json) is not the other machine's.
            "--home" => {
                it.next();
            }
            "--file" => files.extend(it.next().cloned()),
            "--text" => {
                there.push("--text".into());
                text = it.next().map(|t| (there.len(), t.clone()));
                there.push(String::new()); // filled below, once the files are there
            }
            _ => there.push(a.clone()),
        }
    }
    if let Some((slot, t)) = text {
        // The files are here: each is put beside the terminal there first, and the text names that copy.
        let Some(typed) = client.placed(&t, &format!("hc:{machine}/{terminal}"), &files) else {
            let mut out = JsonLines::new();
            header(&mut out, cmd);
            out.emit(&crate::cards::Done::new("failed", Some(&format!("A file could not be put on {machine}. Nothing was sent."))));
            out.emit_value(json!({"type": "end", "ok": true, "ms": 0}));
            return out.text();
        };
        there[slot] = typed;
    }
    let there = [vec!["sh".to_string(), "-c".into(), forward_script(cached), "sh".into()], there].concat();
    let ran = match exec(client, machine, Run::Argv(&there)) {
        Ok(r) => r,
        Err(f) => return error_lines(cmd, f, json!({})),
    };
    if ran.no_program || matches!(ran.code, Some(126 | 127)) {
        memo.missing.insert(machine.into(), now_s());
        memo.save();
        return with_install(client, cmd, machine, missing(machine), json!({}), &mut memo);
    }
    let text = String::from_utf8_lossy(&ran.stdout).to_string();
    let first: Option<Value> = text.lines().next().and_then(|l| serde_json::from_str(l).ok());
    let Some(h) = first.filter(|h| h.get("type").and_then(Value::as_str) == Some("header")) else {
        let why = if ran.stderr.is_empty() { format!("exit {}", ran.code.map_or("?".into(), |c| c.to_string())) } else { ran.stderr };
        return error_lines(cmd, fail("remote_failed", format!("The helper on {machine} did not answer: {why}")), json!({}));
    };
    let (p, s) = (h.get("protocol").and_then(Value::as_i64), h.get("sidecar").and_then(Value::as_str));
    if p != Some(PROTOCOL_VERSION) || s != Some(SIDECAR_VERSION) {
        let f = fail(
            "remote_helper_outdated",
            format!("The helper on {machine} is another version ({}); this Flockbook needs {SIDECAR_VERSION}.", s.unwrap_or("?")),
        );
        let remote = json!({"remote": {"protocol": h.get("protocol"), "sidecar": h.get("sidecar")}});
        return with_install(client, cmd, machine, f, remote, &mut memo);
    }
    text
}

/// `helper install --machine m --sha256 h`: puts the helper there, only when `h` is the sha256 of the file read
/// here now (the one the user said yes to). Written in parts with this run's own tag; then one command there joins
/// them, computes the sum, and only when it is `h` makes it runnable and puts it in place (else removes it).
pub fn install(client: &HcClient, out: &mut JsonLines, machine: &str, sha256: &str, cache: Option<&Path>) -> Result<(), (&'static str, String)> {
    let mut memo = Memo::load(cache);
    let (info, data) = match prepare(client, machine, &mut memo) {
        Ok(Some(x)) => x,
        Ok(None) => return Err(("remote_helper_missing", format!("This Flockbook has no helper for {machine}'s platform."))),
        Err(f) => return Err((f.code, f.message)),
    };
    // Also the gate for the line below: only a sum this helper computed (64 hex digits) goes into it.
    if info.sha256 != sha256.to_ascii_lowercase() {
        return Err(("checksum", "The helper is not the one shown when you said yes. Nothing was installed.".into()));
    }
    use base64::Engine;
    let dir = format!("{HELPER_DIR}/{}", helper_version());
    let tag = format!("{}-{}", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()));
    let part = format!("flockbook-modfeed.part.{tag}");
    let parts: Vec<&[u8]> = data.chunks(MAX_WRITE as usize).collect();
    let names: Vec<String> = (0..parts.len()).map(|i| format!("{part}.{i}")).collect();
    let clean = format!("cd \"$HOME\"/{dir} 2>/dev/null && rm -f {part} {part}.*");
    let gone = |e: (&'static str, String)| {
        let _ = exec(client, machine, Run::Line(&clean));
        Err(e)
    };
    for (bytes, name) in parts.iter().zip(&names) {
        let w = client.call(
            "file.write",
            json!({"on": machine, "path": format!("{dir}/{name}"), "data": base64::engine::general_purpose::STANDARD.encode(bytes)}),
            Some(Duration::from_secs(60)),
        );
        if let Err(e) = w {
            return gone(("remote_failed", format!("Could not write the helper on {machine}: {e}")));
        }
    }
    let want = &info.sha256;
    let line = format!(
        "cd \"$HOME\"/{dir} && cat {names} > {part} && rm -f {part}.* && \
         s=$( {{ sha256sum {part} 2>/dev/null || shasum -a 256 {part}; }} | cut -c1-64 ) && \
         if [ \"$s\" = {want} ]; then chmod 755 {part} && mv -f {part} flockbook-modfeed && echo installed; \
         else rm -f {part}; echo \"changed $s\"; exit 3; fi",
        names = names.join(" ")
    );
    match exec(client, machine, Run::Line(&line)) {
        Ok(r) if r.code == Some(0) && String::from_utf8_lossy(&r.stdout).trim() == "installed" => {}
        Ok(r) => {
            let said = String::from_utf8_lossy(&r.stdout).trim().to_string();
            if let Some(there) = said.strip_prefix("changed") {
                let there = if there.trim().is_empty() { "none" } else { there.trim() };
                return gone(("checksum", format!("The helper arrived changed on {machine} (sha256 {there}). Nothing was installed.")));
            }
            return gone(("remote_failed", format!("Could not install the helper on {machine}: {}", if r.stderr.is_empty() { said } else { r.stderr })));
        }
        Err(f) => return gone((f.code, f.message)),
    }
    memo.missing.remove(machine);
    memo.save();
    out.emit_value(json!({"type": "installed", "machine": machine, "path": info.path, "bytes": info.bytes,
                          "sha256": info.sha256, "source": info.source, "parts": parts.len()}));
    Ok(())
}

/// `helper remove --machine m`: the helpers there go (every version). The chat of that machine's agents stops.
pub fn remove(client: &HcClient, out: &mut JsonLines, machine: &str, cache: Option<&Path>) -> Result<(), (&'static str, String)> {
    let mut memo = Memo::load(cache);
    // The line is fixed, but it is for sh: only on a Unix machine.
    if let Err(f) = platform(client, machine, &mut memo) {
        return Err((f.code, f.message));
    }
    match exec(client, machine, Run::Line(&format!("rm -rf \"$HOME\"/{HELPER_DIR}"))) {
        Ok(r) if r.code == Some(0) => {
            memo.missing.insert(machine.into(), now_s());
            memo.save();
            out.emit_value(json!({"type": "removed", "machine": machine, "path": format!("~/{HELPER_DIR}")}));
            Ok(())
        }
        Ok(r) => Err(("remote_failed", format!("Could not remove the helper on {machine}: {}", r.stderr))),
        Err(f) => Err((f.code, f.message)),
    }
}
