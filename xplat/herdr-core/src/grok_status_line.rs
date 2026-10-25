//! Conservative TOML text edits: refuse ambiguous shapes rather than rewriting another setting.
use serde_json::{json, Value};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_round_trip_keeps_bytes_and_other_keys() {
        let before = "# owner\n[ui.status_line]\ntype = 'command'\ncommand = 'printf hello'\npadding = 2\n\n[other]\nx = true\n";
        let (on, saved) = edit(before, "wrapper", None, true).unwrap();
        assert!(on.contains("padding = 2"));
        let (off, _) = edit(&on, "wrapper", Some(&saved), false).unwrap();
        assert_eq!(before, off);
    }
    #[test]
    fn refuses_quoted_dotted_inline_and_multiline_shapes() {
        for s in [
            "[\"ui\".status_line]\n",
            "ui.status_line = {}\n",
            "[ui]\nstatus_line = {}\n",
            "[ui.status_line.child]\n",
            "[ui.status_line]\ncommand = \"\"\"unsafe\"\"\"\n",
        ] {
            assert!(edit(s, "wrapper", None, true).is_err(), "{s}");
        }
    }
}

fn string_value(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.starts_with('\'') {
        let end = t[1..].find('\'')? + 1;
        if !t[end + 1..].trim().is_empty() && !t[end + 1..].trim().starts_with('#') {
            return None;
        }
        return Some(t[1..end].into());
    }
    if t.starts_with('"') {
        let mut stream = serde_json::Deserializer::from_str(t).into_iter::<String>();
        let value = stream.next()?.ok()?;
        let after = t[stream.byte_offset()..].trim();
        if after.is_empty() || after.starts_with('#') {
            return Some(value);
        }
    }
    None
}
/// Returns (edited TOML, saved original table/command). Unknown table keys are retained while enabled.
pub fn edit(
    text: &str,
    command: &str,
    original: Option<&Value>,
    on: bool,
) -> Result<(String, Value), String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut table = String::new();
    let mut start = None;
    let mut end = lines.len();
    let mut keys = std::collections::HashSet::new();
    let mut current_command = None;
    let mut original_command = None;
    let mut retained = String::new();
    let mut items = Value::Null;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim().trim_start_matches('\u{feff}');
        if t.is_empty() || t.starts_with('#') {
            if table == "ui.status_line" {
                retained.push_str(line);
            }
            continue;
        }
        if t.contains("\"\"\"") || t.contains("'''") {
            return Err("Multiline TOML strings are not changed".into());
        }
        if t.starts_with('[') {
            let close = t.find(']').ok_or("Unreadable TOML header")?;
            let name = t[1..close].trim();
            let after = t[close + 1..].trim();
            if !after.is_empty() && !after.starts_with('#') && !t.starts_with("[[") {
                return Err("Unreadable TOML header suffix".into());
            }
            let bare: String = name
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '\'' && *c != '"')
                .collect();
            if (bare == "ui" || bare.starts_with("ui."))
                && (name.contains(['\'', '"', '[']) || t.starts_with("[["))
            {
                return Err("Quoted UI headers and arrays of tables are not changed".into());
            }
            if bare.contains("status_line") && bare != "ui.status_line" {
                return Err("Nested status_line tables are not changed".into());
            }
            if table == "ui.status_line" && end == lines.len() {
                end = i;
            }
            table = bare;
            if table == "ui.status_line" {
                if start.is_some() {
                    return Err("Duplicate status_line table".into());
                }
                start = Some(i);
            }
            continue;
        }
        let Some((key, value)) = t.split_once('=') else {
            if table == "ui.status_line" {
                return Err("Unreadable status_line entry".into());
            }
            continue;
        };
        let key = key.trim();
        let bare: String = key
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '\'' && *c != '"')
            .collect();
        if (table.is_empty() && (bare == "ui" || bare.starts_with("ui.")))
            || (table == "ui" && bare.starts_with("status_line"))
        {
            return Err("Dotted or inline status_line definitions are not changed".into());
        }
        if table != "ui.status_line" {
            continue;
        }
        if !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !keys.insert(key)
        {
            return Err("Unreadable or duplicate status_line key".into());
        }
        match key {
            "command" => {
                current_command =
                    Some(string_value(value).ok_or("Unreadable status_line command")?);
                original_command = current_command.clone();
            }
            "type" => {
                string_value(value).ok_or("Unreadable status_line type")?;
            }
            "items" => {
                let v: Value = serde_json::from_str(value.trim()).map_err(|_| {
                    "Unreadable status_line items; use a single-line array of double-quoted strings"
                })?;
                items = v.clone();
                if !v.as_array().is_some_and(|a| a.iter().all(Value::is_string)) {
                    return Err("Unreadable status_line items".into());
                }
            }
            _ => {
                let scalar = value.split('#').next().unwrap_or("").trim();
                if string_value(value).is_none()
                    && scalar.parse::<f64>().is_err()
                    && !matches!(scalar, "true" | "false")
                {
                    return Err("Unreadable status_line value".into());
                }
                retained.push_str(line);
            }
        }
    }
    let range = start.map(|s| (s, end));
    let ours = current_command.as_deref() == Some(command);
    if !ours
        && current_command
            .as_ref()
            .is_some_and(|s| s.contains("--flockbook-statusline"))
    {
        return Err(
            "The Grok status-line wrapper moved; restore the original before enabling this copy"
                .into(),
        );
    }
    let saved = if on && !ours {
        json!({"table":range.map(|(s,e)| lines[s..e].join("")),"command":original_command,"items":items})
    } else {
        original
            .cloned()
            .ok_or("Original Grok status line is missing")?
    };
    if !on && !ours {
        return Err("Grok's status line was replaced; current settings were left unchanged".into());
    }
    let replacement = if on {
        format!(
            "[ui.status_line]\ntype = \"command\"\ncommand = {}\n{}",
            serde_json::to_string(command).map_err(|e| e.to_string())?,
            retained
        )
    } else {
        saved
            .get("table")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let result = if let Some((s, e)) = range {
        format!(
            "{}{}{}",
            lines[..s].join(""),
            replacement,
            lines[e..].join("")
        )
    } else {
        format!(
            "{}{}{}",
            text,
            if text.is_empty() || text.ends_with('\n') {
                ""
            } else {
                "\n"
            },
            replacement
        )
    };
    Ok((result, saved))
}

pub fn builtin(value: &Value, items: Option<&Value>) -> String {
    let default = vec!["cwd", "model", "context"];
    let items: Vec<&str> = items
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or(default);
    let clean = |s: &str, limit: usize| {
        crate::status_line::strip_controls(s)
            .replace('\n', "")
            .chars()
            .take(limit)
            .collect::<String>()
    };
    let mut parts = Vec::new();
    for item in items {
        let part = match item {
            "cwd" => value
                .pointer("/workspace/current_dir")
                .or_else(|| value.get("cwd"))
                .and_then(Value::as_str)
                .and_then(|p| std::path::Path::new(p).file_name()?.to_str())
                .map(|s| clean(s, 40)),
            "model" => value
                .pointer("/model/display_name")
                .or_else(|| value.pointer("/model/id"))
                .and_then(Value::as_str)
                .map(|s| clean(s, 30)),
            "context" => value
                .pointer("/context_window/used_percentage")
                .and_then(Value::as_f64)
                .filter(|p| p.is_finite())
                .map(|p| format!("{:.0}% ctx", p.clamp(0.0, 100.0))),
            "cost" => value
                .pointer("/cost/total_cost_usd")
                .and_then(Value::as_f64)
                .filter(|c| c.is_finite() && *c >= 0.005)
                .map(|c| format!("${c:.2}")),
            "session-name" => value
                .get("session_name")
                .and_then(Value::as_str)
                .map(|s| clean(s, 40)),
            "turn-timer" => value
                .pointer("/turn/started_at_ms")
                .and_then(Value::as_f64)
                .and_then(|ms| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .ok()?
                        .as_secs_f64();
                    let sec = now - ms / 1000.0;
                    if !sec.is_finite() || !(1.0..1e9).contains(&sec) {
                        return None;
                    }
                    let s = sec as u64;
                    Some(if s >= 3600 {
                        format!("{}h {:02}m", s / 3600, s % 3600 / 60)
                    } else if s >= 60 {
                        format!("{}m {:02}s", s / 60, s % 60)
                    } else {
                        format!("{s}s")
                    })
                }),
            _ => None,
        };
        if let Some(p) = part.filter(|p| !p.is_empty()) {
            parts.push(p);
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join(" │ ") + "\n"
    }
}
