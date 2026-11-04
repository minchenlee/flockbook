//! Rust only: the Swift sidecar skips terminals on other machines. A terminal `m/tN` is on machine `m`: the same
//! helper runs there (`machine.exec`) with the same command and `--terminal tN`, and its lines come back as they are,
//! so the chat reads every log with one parser. Its core there is its own (hc gives it `HC_SESSION`).
//!
//! The helper there is this same version, never another (`remote_helper_outdated` otherwise). Putting it there is
//! the user's choice: `remote_helper_missing` carries what the mod shows (machine, path, size, sha256), and
//! `helper install` runs only with that sha256. The file comes from the folder this Flockbook was installed with
//! (`<home>/flockbook/remote/<target>/flockbook-modfeed`, `SHA256SUMS` beside it), checked here and again there.

use crate::protocol::{JsonLines, PROTOCOL_VERSION, SIDECAR_VERSION};
use herdr_core::hc::client::shell_quoted;
use herdr_core::hc::HcClient;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Under the home there.
pub const HELPER_DIR: &str = ".local/share/flockbook/helper";
/// The cache (and the claims beside it) there, under the home.
const CACHE: &str = ".cache/flockbook/cache.json";
/// One `file.write` takes at most 8 MB; the helper goes in parts no larger than this.
pub const MAX_WRITE: u64 = 6 * 1024 * 1024;
/// A command there: the longest a helper command takes (a send waits up to 22 s) and a margin.
const EXEC_TIMEOUT_S: u64 = 30;

pub fn helper_version() -> &'static str {
    SIDECAR_VERSION
}

fn helper_rel() -> String {
    format!("{HELPER_DIR}/{}/flockbook-modfeed", helper_version())
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
}

fn exec(client: &HcClient, machine: &str, line: &str) -> Result<Ran, Fail> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let r = client
        .call(
            "machine.exec",
            json!({"on": machine, "line": line, "timeout": EXEC_TIMEOUT_S}),
            Some(Duration::from_secs(EXEC_TIMEOUT_S + 5)),
        )
        .map_err(|e| fail("remote_failed", format!("{machine}: {e}")))?;
    if r.get("timed_out").and_then(Value::as_bool) == Some(true) {
        return Err(fail("remote_failed", format!("{machine} did not answer in {EXEC_TIMEOUT_S} s")));
    }
    if r.get("truncated").and_then(Value::as_bool) == Some(true) {
        return Err(fail("remote_failed", format!("{machine} answered more than hc passes on (8 MB)")));
    }
    let stdout = b64.decode(r.get("stdout").and_then(Value::as_str).unwrap_or("")).unwrap_or_default();
    let stderr = b64.decode(r.get("stderr").and_then(Value::as_str).unwrap_or("")).unwrap_or_default();
    Ok(Ran { code: r.get("code").and_then(Value::as_i64), stdout, stderr: String::from_utf8_lossy(&stderr).trim().to_string() })
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
fn listed_sum(dir: &Path, target: &str) -> Option<String> {
    let lists = [
        (dir.join("SHA256SUMS"), format!("{target}/flockbook-modfeed")),
        (dir.join(target).join("SHA256SUMS"), "flockbook-modfeed".to_string()),
    ];
    for (file, name) in lists {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        for line in text.lines() {
            let mut it = line.split_whitespace();
            if let (Some(sum), Some(n)) = (it.next(), it.next()) {
                if n.trim_start_matches('*') == name {
                    return Some(sum.to_ascii_lowercase());
                }
            }
        }
    }
    None
}

/// What would go to `machine`, read and checked here. Ok(None): this Flockbook has no helper for that platform.
fn prepare(client: &HcClient, machine: &str) -> Result<Option<(Install, Vec<u8>)>, Fail> {
    let u = exec(client, machine, "uname -sm")?;
    let uname = String::from_utf8_lossy(&u.stdout).trim().to_string();
    let target = match (u.code, target_for(&uname)) {
        (Some(0), Some(t)) => t,
        _ => {
            let what = if uname.is_empty() { "a system without uname (Windows?)".to_string() } else { uname };
            return Err(fail(
                "remote_helper_unsupported",
                format!("Flockbook cannot show the chat of agents on {machine}: it runs {what}, and the helper is only for Linux (x86_64, aarch64) and macOS."),
            ));
        }
    };
    let Some((dir, source)) = helpers_dir() else { return Ok(None) };
    let Ok(data) = std::fs::read(dir.join(target).join("flockbook-modfeed")) else { return Ok(None) };
    let sum = sha256_hex(&data);
    if listed_sum(&dir, target).as_deref() != Some(sum.as_str()) {
        return Err(fail("checksum", format!("The helper for {target} in this Flockbook does not match its SHA256SUMS. Reinstall Flockbook.")));
    }
    let install = Install {
        machine: machine.into(),
        target: target.into(),
        path: format!("~/{}", helper_rel()),
        bytes: data.len() as u64,
        sha256: sum,
        source,
    };
    Ok(Some((install, data)))
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
fn with_install(client: &HcClient, cmd: &str, machine: &str, f: Fail, mut extra: Value) -> String {
    match prepare(client, machine) {
        Ok(i) => {
            extra["install"] = serde_json::to_value(i.map(|(i, _)| i)).unwrap_or(Value::Null);
            error_lines(cmd, f, extra)
        }
        Err(stop) => error_lines(cmd, stop, json!({})),
    }
}

/// `cmd` with `args` (as given to this helper) for the terminal `terminal` on `machine`: the lines of the helper
/// there, as they are.
pub fn forward(client: &HcClient, cmd: &str, args: &[String], machine: &str, terminal: &str) -> String {
    let mut there: Vec<String> = vec![];
    let mut files: Vec<String> = vec![];
    let mut text: Option<(usize, String)> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--terminal" => {
                it.next();
                there.push(shell_quoted("--terminal"));
                there.push(shell_quoted(terminal));
            }
            // This computer's cache path means nothing there: the helper there keeps its own.
            "--cache" => {
                it.next();
                there.push(shell_quoted("--cache"));
                there.push(format!("\"$HOME\"/{CACHE}"));
            }
            "--file" => files.extend(it.next().cloned()),
            "--text" => {
                there.push(shell_quoted("--text"));
                text = it.next().map(|t| (there.len(), t.clone()));
                there.push(String::new()); // filled below, once the files are there
            }
            _ => there.push(shell_quoted(a)),
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
        there[slot] = shell_quoted(&typed);
    }
    let line = format!("exec \"$HOME\"/{} {} {}", helper_rel(), shell_quoted(cmd), there.join(" "));
    let ran = match exec(client, machine, &line) {
        Ok(r) => r,
        Err(f) => return error_lines(cmd, f, json!({})),
    };
    if matches!(ran.code, Some(126 | 127)) {
        let f = fail("remote_helper_missing", format!("{machine} has no Flockbook helper yet, so the chat cannot read its agents."));
        return with_install(client, cmd, machine, f, json!({}));
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
        return with_install(client, cmd, machine, f, remote);
    }
    text
}

/// `helper install --machine m --sha256 h`: puts the helper there, only when `h` is the sha256 of the file read
/// here now (the one the user said yes to). Written in parts, joined and checked there, then made runnable.
pub fn install(client: &HcClient, out: &mut JsonLines, machine: &str, sha256: &str) -> Result<(), (&'static str, String)> {
    let (info, data) = match prepare(client, machine) {
        Ok(Some(x)) => x,
        Ok(None) => return Err(("remote_helper_missing", format!("This Flockbook has no helper for {machine}'s platform."))),
        Err(f) => return Err((f.code, f.message)),
    };
    if info.sha256 != sha256.to_ascii_lowercase() {
        return Err(("checksum", "The helper is not the one shown when you said yes. Nothing was installed.".into()));
    }
    use base64::Engine;
    let dir = format!("{HELPER_DIR}/{}", helper_version());
    let parts: Vec<&[u8]> = data.chunks(MAX_WRITE as usize).collect();
    let names: Vec<String> = (0..parts.len()).map(|i| format!("flockbook-modfeed.part.{i}")).collect();
    let clean = format!("cd \"$HOME\"/{dir} 2>/dev/null && rm -f flockbook-modfeed.part flockbook-modfeed.part.*");
    let gone = |e: (&'static str, String)| {
        let _ = exec(client, machine, &clean);
        Err(e)
    };
    for (part, name) in parts.iter().zip(&names) {
        let w = client.call(
            "file.write",
            json!({"on": machine, "path": format!("{dir}/{name}"), "data": base64::engine::general_purpose::STANDARD.encode(part)}),
            Some(Duration::from_secs(60)),
        );
        if let Err(e) = w {
            return gone(("remote_failed", format!("Could not write the helper on {machine}: {e}")));
        }
    }
    let join = format!(
        "cd \"$HOME\"/{dir} && cat {} > flockbook-modfeed.part && rm -f flockbook-modfeed.part.* && \
         {{ sha256sum flockbook-modfeed.part 2>/dev/null || shasum -a 256 flockbook-modfeed.part; }}",
        names.join(" ")
    );
    let there = match exec(client, machine, &join) {
        Ok(r) => String::from_utf8_lossy(&r.stdout).split_whitespace().next().unwrap_or("").to_ascii_lowercase(),
        Err(f) => return gone((f.code, f.message)),
    };
    if there != info.sha256 {
        return gone(("checksum", format!("The helper arrived changed on {machine} (sha256 {there}). Nothing was installed.")));
    }
    let live = format!("cd \"$HOME\"/{dir} && chmod 755 flockbook-modfeed.part && mv -f flockbook-modfeed.part flockbook-modfeed");
    match exec(client, machine, &live) {
        Ok(r) if r.code == Some(0) => {}
        Ok(r) => return gone(("remote_failed", format!("Could not make the helper runnable on {machine}: {}", r.stderr))),
        Err(f) => return gone((f.code, f.message)),
    }
    out.emit_value(json!({"type": "installed", "machine": machine, "path": info.path, "bytes": info.bytes,
                          "sha256": info.sha256, "source": info.source, "parts": parts.len()}));
    Ok(())
}

/// `helper remove --machine m`: the helpers there go (every version). The chat of that machine's agents stops.
pub fn remove(client: &HcClient, out: &mut JsonLines, machine: &str) -> Result<(), (&'static str, String)> {
    match exec(client, machine, &format!("rm -rf \"$HOME\"/{HELPER_DIR}")) {
        Ok(r) if r.code == Some(0) => {
            out.emit_value(json!({"type": "removed", "machine": machine, "path": format!("~/{HELPER_DIR}")}));
            Ok(())
        }
        Ok(r) => Err(("remote_failed", format!("Could not remove the helper on {machine}: {}", r.stderr))),
        Err(f) => Err((f.code, f.message)),
    }
}
