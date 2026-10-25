//! Opt-in status-line capture. Configuration edits are reversible and never execute an owner's command.
use crate::model::StatusTag;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restores_the_exact_original_setting_and_keeps_other_keys() {
        let original = json!({"theme":"dark", "statusLine":{"type":"command","command":"printf hello","padding":2}});
        let (enabled, saved) = claude_edit(&original, "wrapper", None, true).unwrap();
        let (restored, _) = claude_edit(&enabled, "wrapper", Some(&saved), false).unwrap();
        assert_eq!(restored, original);
        assert!(claude_edit(&json!([]), "wrapper", None, true).is_err());
    }
    #[test]
    fn tags_clamp_context_and_keep_unknown_fields_absent() {
        let tags = claude_tags(
            &json!({"model":{"display_name":"Model"},"context_window":{"used_percentage":120}}),
        );
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[1].percent, Some(100.0));
        assert!(tags[1].warn);
    }
}

pub fn claude_tags(v: &Value) -> Vec<StatusTag> {
    let mut tags = Vec::new();
    let mut add = |kind: &str, text: String, spoken: String, percent: Option<f64>| {
        tags.push(StatusTag {
            kind: kind.into(),
            text,
            spoken,
            percent,
            warn: percent.is_some_and(|p| p >= 80.0),
        })
    };
    if let Some(s) = v
        .pointer("/model/display_name")
        .or_else(|| v.pointer("/model/id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        add("model", s.into(), format!("Model {s}"), None);
    }
    if let Some(s) = v
        .pointer("/effort/level")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        add("effort", s.into(), format!("Effort {s}"), None);
    }
    if let Some(p) = v
        .pointer("/context_window/used_percentage")
        .and_then(Value::as_f64)
        .filter(|p| p.is_finite())
    {
        let p = p.clamp(0.0, 100.0).round();
        add(
            "context",
            format!("ctx {p:.0}%"),
            format!("Context {p:.0}% used"),
            Some(p),
        );
    }
    if let Some(c) = v
        .pointer("/cost/total_cost_usd")
        .and_then(Value::as_f64)
        .filter(|c| c.is_finite() && *c >= 0.0)
    {
        add(
            "cost",
            format!("${c:.2} est."),
            format!("Session cost about ${c:.2}, an estimate"),
            None,
        );
    }
    tags
}

pub fn plain_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn folder(agent: &str) -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::resolver::home_dir().join(".local/state"));
    base.join("flockbook/statusline").join(agent)
}
fn session_name(id: &str) -> Option<String> {
    if plain_name(id) {
        return Some(id.into());
    }
    let path = Path::new(id);
    let name = if path.file_name()?.to_str()? == "updates.jsonl" {
        path.parent()?.file_name()?.to_str()?
    } else {
        path.file_stem()?.to_str()?
    };
    plain_name(name).then(|| name.into())
}
pub fn read_tags(agent: &str, id: &str) -> Vec<StatusTag> {
    if !matches!(agent, "claude" | "grok") {
        return Vec::new();
    }
    let Some(id) = session_name(id) else {
        return Vec::new();
    };
    let root = folder(agent);
    let input = root.join(format!("{id}.json"));
    let Ok(meta) = std::fs::symlink_metadata(&input) else {
        return Vec::new();
    };
    if !meta.is_file()
        || meta.len() > 262144
        || meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_none_or(|d| d.as_secs() > 86400)
    {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(input) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    let mut tags = claude_tags(&v);
    let out_path = root.join(format!("{id}.out"));
    if let Ok(meta) = std::fs::symlink_metadata(&out_path) {
        if meta.is_file() && meta.len() <= 65536 {
            if let Ok(out) = std::fs::read(&out_path) {
                if out.len() <= 65536 {
                    let clean = strip_controls(&String::from_utf8_lossy(&out));
                    let split = regex::Regex::new(r"\s+[·|•]\s+|\s*│\s*|\s{2,}").unwrap();
                    for part in split
                        .split(&clean)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .take(40)
                    {
                        if tags.iter().any(|t| t.text == part) {
                            continue;
                        }
                        tags.push(StatusTag {
                            kind: "own".into(),
                            text: part.into(),
                            spoken: part.into(),
                            percent: None,
                            warn: false,
                        });
                    }
                }
            }
        }
    }
    tags
}
/// Drop CSI, OSC and hidden direction controls, retaining only bounded visible lines.
pub fn strip_controls(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    let mut lines = 1;
    let mut cols = 0;
    while let Some(c) = it.next() {
        if c == '\x1b' {
            match it.next() {
                Some('[') => {
                    for x in it.by_ref() {
                        if ('@'..='~').contains(&x) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(x) = it.next() {
                        if x == '\x07' || (x == '\x1b' && it.peek() == Some(&'\\')) {
                            if x == '\x1b' {
                                it.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        if c == '\n' {
            lines += 1;
            if lines > 8 {
                break;
            }
            cols = 0;
            out.push(c);
            continue;
        }
        if c.is_control()
            || matches!(c, '\u{061c}' | '\u{200b}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2069}' | '\u{feff}')
        {
            continue;
        }
        if cols < 400 {
            out.push(c);
            cols += 1;
        }
    }
    out
}

/// Returns (new settings, saved original). Refuses to overwrite an externally replaced command on Off.
pub fn claude_edit(
    settings: &Value,
    command: &str,
    original: Option<&Value>,
    on: bool,
) -> Result<(Value, Value), String> {
    let mut obj = settings
        .as_object()
        .cloned()
        .ok_or("settings.json must be an object")?;
    let current = obj.get("statusLine").cloned().unwrap_or(Value::Null);
    let ours = current.get("command").and_then(Value::as_str) == Some(command);
    if !ours
        && current
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|s| s.contains("--flockbook-statusline"))
    {
        return Err(
            "The status-line wrapper moved; restore its original setting before enabling this copy"
                .into(),
        );
    }
    if on {
        let saved = if ours {
            original.cloned().ok_or("Original status line is missing")?
        } else {
            current
        };
        obj.insert(
            "statusLine".into(),
            json!({"type":"command","command":command}),
        );
        Ok((Value::Object(obj), saved))
    } else {
        if !ours {
            return Err(
                "The status line was replaced; its current command was left unchanged".into(),
            );
        }
        let saved = original.ok_or("Original status line is missing")?;
        if saved.is_null() {
            obj.remove("statusLine");
        } else {
            obj.insert("statusLine".into(), saved.clone());
        }
        Ok((Value::Object(obj), saved.clone()))
    }
}

pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp = parent.join(format!(".statusline-{}-{stamp}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp).map_err(|e| e.to_string())?;
    f.write_all(data)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}
fn config_text(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.to_string()),
    }
}
/// Explicit Settings action only. No configuration changes occur when reading tags or launching the app.
pub fn set_enabled(agent: &str, on: bool) -> Result<(), String> {
    if !matches!(agent, "claude" | "grok") {
        return Err("Unsupported agent".into());
    }
    let root = folder(agent);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        let _ = (on, root, exe);
        return Err("Status-line configuration on Windows is not available".into());
    }
    #[cfg(not(windows))]
    {
        let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let command = format!(
            "{} --flockbook-statusline {}",
            quote(&exe.to_string_lossy()),
            agent
        );
        let config = if agent == "claude" {
            std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| crate::resolver::home_dir().join(".claude"))
                .join("settings.json")
        } else {
            std::env::var_os("GROK_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| crate::resolver::home_dir().join(".grok"))
                .join("config.toml")
        };
        let text = config_text(&config)?;
        let backup = root.join("original.json");
        let original = match std::fs::read(&backup) {
            Ok(b) => Some(serde_json::from_slice::<Value>(&b).map_err(|e| e.to_string())?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        let (new, saved) = if agent == "claude" {
            let v = if text.trim().is_empty() {
                json!({})
            } else {
                serde_json::from_str(&text).map_err(|e| e.to_string())?
            };
            let (new, saved) = claude_edit(&v, &command, original.as_ref(), on)?;
            (
                serde_json::to_string_pretty(&new).map_err(|e| e.to_string())?,
                saved,
            )
        } else {
            crate::grok_status_line::edit(&text, &command, original.as_ref(), on)?
        };
        if on {
            atomic_write(
                &backup,
                &serde_json::to_vec(&saved).map_err(|e| e.to_string())?,
            )?;
        }
        atomic_write(&config, new.as_bytes())
    }
}

/// Called only by the agent's configured command, before any iced initialization.
/// The app's tag reader never executes a command. Output is returned unchanged, up to the bounded wrapper limit.
pub fn wrapper(agent: &str) -> i32 {
    if std::env::var_os("FLOCKBOOK_STATUSLINE").is_some() || !matches!(agent, "claude" | "grok") {
        return 0;
    }
    let mut input = Vec::new();
    if std::io::stdin()
        .take(262145)
        .read_to_end(&mut input)
        .is_err()
        || input.len() > 262144
    {
        return 1;
    }
    let root = folder(agent);
    let value: Value = serde_json::from_slice(&input).unwrap_or(Value::Null);
    let id = value
        .get("session_id")
        .or_else(|| value.get("sessionId"))
        .and_then(Value::as_str)
        .filter(|s| plain_name(s));
    if let Some(id) = id {
        let _ = atomic_write(&root.join(format!("{id}.json")), &input);
    }
    let saved: Value = std::fs::read(root.join("original.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    if agent == "grok" && saved.get("command").is_none_or(Value::is_null) {
        let output = crate::grok_status_line::builtin(&value, saved.get("items"));
        let _ = std::io::stdout().write_all(output.as_bytes());
        if let Some(id) = id {
            let _ = atomic_write(&root.join(format!("{id}.out")), output.as_bytes());
        }
        return 0;
    }
    let command = saved.get("command").and_then(Value::as_str).filter(|_| {
        saved
            .get("type")
            .and_then(Value::as_str)
            .is_none_or(|t| t == "command")
    });
    let Some(command) = command.filter(|s| !s.is_empty() && !s.contains("--flockbook-statusline"))
    else {
        if let Some(id) = id {
            let _ = atomic_write(&root.join(format!("{id}.out")), b"");
        }
        return 0;
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", command]);
        c
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let Ok(mut child) = cmd
        .env("FLOCKBOOK_STATUSLINE", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
    else {
        return 1;
    };
    let stdin = child.stdin.take();
    let bytes = input.clone();
    std::thread::spawn(move || {
        if let Some(mut s) = stdin {
            let _ = s.write_all(&bytes);
        }
    });
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.take(1 << 20).read_to_end(&mut out);
        let _ = tx.send(out);
    });
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code().unwrap_or(1),
            Err(_) => break 1,
            _ => {}
        }
        if std::time::Instant::now() >= end {
            #[cfg(unix)]
            {
                let _ = std::process::Command::new("kill")
                    .args(["-KILL", "--", &format!("-{}", child.id())])
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
            break 124;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let output = rx
        .recv_timeout(std::time::Duration::from_millis(200))
        .unwrap_or_default();
    let _ = std::io::stdout().write_all(&output);
    if status == 0 || !output.is_empty() {
        if let Some(id) = id {
            let _ = atomic_write(
                &root.join(format!("{id}.out")),
                &output[..output.len().min(65536)],
            );
        }
    }
    status
}
