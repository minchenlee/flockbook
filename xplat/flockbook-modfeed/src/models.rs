//! The values `/model <value>` takes, per agent kind, read from the agent's own files where it keeps them
//! (spike 11-10: Claude Code 2.1, Codex 0.167, pi 1.3, agy 1.5; each switched with the value and no screen):
//! - Claude Code: the full ids of its current models and their aliases (`CLAUDE_MODELS`; its list is inside its
//!   program), plus the ids in the user's settings (`modelSettings`, `availableModels`) and the extra options it
//!   saved (`~/.claude.json` `additionalModelOptionsCache`). Its `/model <id>` also saves the id as the default
//!   for new sessions, as its own picker does.
//! - Codex: its model cache (`~/.codex/models_cache.json`, the ones its picker lists, in its order). `/model <slug>`
//!   is for this session only.
//! - pi: `provider/id` of the models of the providers it has a key for (`auth.json`, or an `apiKey` in
//!   `models.json`), from `models-store.json` and `models.json`; only `enabledModels` when its settings name some
//!   (then `/model` picks from those only). An exact `provider/id` switches; anything else opens its picker.
//! - agy: its list comes from its server, not a file: `AGY_MODELS`, as agy 1.5.3 named them (an unknown name prints
//!   the valid ones). `/model <slug>` also saves it as the default.

use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// A value offered after "/name ", with a label for people ("Opus 5.5"); no label: the value says it all.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Arg {
    pub value: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
}

impl Arg {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Arg {
        Arg { value: value.into(), label: label.into() }
    }
}

/// Claude Code 2.1.322's models (its picker: Default is Fable 5.2), full ids first, then the aliases.
pub const CLAUDE_MODELS: [(&str, &str); 10] = [
    ("claude-fable-5-2", "Fable 5.2"),
    ("claude-opus-5-5", "Opus 5.5"),
    ("claude-sonnet-5-5", "Sonnet 5.5"),
    ("claude-haiku-5-5", "Haiku 5.5"),
    ("claude-fable-5-1", "Fable 5.1"),
    ("default", "The recommended model"),
    ("fable", "The latest Fable"),
    ("opus", "The latest Opus"),
    ("sonnet", "The latest Sonnet"),
    ("haiku", "The latest Haiku"),
];

/// agy 1.5.3's models ("Invalid model" names them; the picker shows the labels, effort apart).
pub const AGY_MODELS: [(&str, &str); 13] = [
    ("gemini-4-argon-low", "Gemini 4 Argon (Low)"),
    ("gemini-4-argon-medium", "Gemini 4 Argon (Medium)"),
    ("gemini-4-argon-high", "Gemini 4 Argon (High)"),
    ("gemini-3.8-flash-low", "Gemini 3.8 Flash (Low)"),
    ("gemini-3.8-flash-medium", "Gemini 3.8 Flash (Medium)"),
    ("gemini-3.8-flash-high", "Gemini 3.8 Flash (High)"),
    ("claude-opus-5-5-low", "Claude Opus 5.5 (Low)"),
    ("claude-opus-5-5-medium", "Claude Opus 5.5 (Medium)"),
    ("claude-opus-5-5-high", "Claude Opus 5.5 (High)"),
    ("claude-sonnet-5-5-low", "Claude Sonnet 5.5 (Low)"),
    ("claude-sonnet-5-5-medium", "Claude Sonnet 5.5 (Medium)"),
    ("claude-sonnet-5-5-high", "Claude Sonnet 5.5 (High)"),
    ("gpt-oss-120b-medium", "GPT-OSS 120B (Medium)"),
];

/// At most this many values; a file larger than this is not read.
const MAX: usize = 400;
const READ_MAX: u64 = 16 * 1024 * 1024;

/// The values of `/model` for an agent of `kind`, with the user's home `home`. None: the agent's `/model` takes no
/// value the helper knows, so it stays a screen with no value.
pub fn values(kind: &str, home: &Path) -> Vec<Arg> {
    let mut l = Values::default();
    match kind {
        "claude" => claude(home, &mut l),
        "codex" => codex(home, &mut l),
        "pi" => pi(home, &mut l),
        "agy" => AGY_MODELS.iter().for_each(|(v, n)| l.add(v, n)),
        _ => {}
    }
    l.out
}

#[derive(Default)]
struct Values {
    out: Vec<Arg>,
    seen: HashSet<String>,
}

impl Values {
    fn add(&mut self, value: &str, label: &str) {
        let value = value.trim();
        if self.out.len() < MAX && !value.is_empty() && !value.chars().any(char::is_whitespace) && self.seen.insert(value.into()) {
            self.out.push(Arg::new(value, label.trim()));
        }
    }
}

fn json(p: &Path) -> Option<serde_json::Value> {
    if fs::metadata(p).map(|m| m.len() > READ_MAX).unwrap_or(true) {
        return None;
    }
    serde_json::from_slice(&fs::read(p).ok()?).ok()
}

/// "claude-opus-5-5" → "Opus 5.5"; "claude-fable-5-1[1m]" → "Fable 5.1 (1M context)". Not a Claude id: "".
pub fn claude_label(id: &str) -> String {
    let (base, wide) = id.strip_suffix("[1m]").map(|b| (b, true)).unwrap_or((id, false));
    let Some(rest) = base.strip_prefix("claude-") else { return String::new() };
    let mut parts = rest.split('-');
    let Some(family) = parts.next().filter(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_alphabetic())) else { return String::new() };
    let version: Vec<&str> = parts.collect();
    if version.is_empty() || version.iter().any(|v| v.is_empty() || !v.chars().all(|c| c.is_ascii_digit()) || v.len() > 2) {
        return String::new();
    }
    let mut name = family[..1].to_uppercase() + &family[1..];
    name = format!("{name} {}", version.join("."));
    if wide {
        name.push_str(" (1M context)");
    }
    name
}

fn claude(home: &Path, l: &mut Values) {
    for (v, n) in CLAUDE_MODELS {
        l.add(v, n);
    }
    if let Some(s) = json(&home.join(".claude/settings.json")) {
        let mut ids: Vec<String> = s["modelSettings"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
        ids.extend(s["availableModels"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)));
        for id in ids {
            l.add(&id, &claude_label(&id));
        }
    }
    let extra = json(&home.join(".claude.json")).map(|c| c["additionalModelOptionsCache"].clone()).unwrap_or_default();
    for o in extra.as_array().into_iter().flatten() {
        let Some(v) = o["value"].as_str() else { continue };
        // Its description starts with the model's name ("Fable 5.1 · Most capable …").
        let label = o["description"].as_str().and_then(|d| d.split(" · ").next()).filter(|d| !d.is_empty());
        let label = label.map(String::from).unwrap_or_else(|| claude_label(v));
        l.add(v, &label);
    }
}

fn codex(home: &Path, l: &mut Values) {
    let Some(c) = json(&home.join(".codex/models_cache.json")) else { return };
    let mut models: Vec<&serde_json::Value> = c["models"].as_array().into_iter().flatten().filter(|m| m["visibility"] == "list").collect();
    models.sort_by_key(|m| m["priority"].as_i64().unwrap_or(i64::MAX));
    for m in models {
        if let Some(slug) = m["slug"].as_str() {
            l.add(slug, m["display_name"].as_str().unwrap_or(""));
        }
    }
}

fn pi(home: &Path, l: &mut Values) {
    let agent = home.join(".pi/agent");
    let custom = json(&agent.join("models.json")).map(|m| m["providers"].clone()).unwrap_or_default();
    let store = json(&agent.join("models-store.json")).unwrap_or_default();
    // Providers it can use: a key saved by its login, or one in models.json.
    let mut keyed: HashSet<String> = json(&agent.join("auth.json")).and_then(|a| a.as_object().map(|o| o.keys().cloned().collect())).unwrap_or_default();
    if let Some(o) = custom.as_object() {
        keyed.extend(o.iter().filter(|(_, p)| p["apiKey"].as_str().is_some_and(|k| !k.is_empty())).map(|(n, _)| n.clone()));
    }
    let enabled: Vec<String> = json(&agent.join("settings.json"))
        .and_then(|s| s["enabledModels"].as_array().cloned())
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(|p| p.split(':').next().unwrap_or(p).to_string()).collect())
        .unwrap_or_default();
    let mut providers: Vec<&String> = keyed.iter().collect();
    providers.sort();
    for p in providers {
        for src in [&store[p.as_str()]["models"], &custom[p.as_str()]["models"]] {
            for m in src.as_array().into_iter().flatten() {
                let Some(id) = m["id"].as_str() else { continue };
                let value = format!("{p}/{id}");
                if enabled.is_empty() || enabled.iter().any(|e| glob(e, &value) || glob(e, id)) {
                    l.add(&value, m["name"].as_str().unwrap_or(""));
                }
            }
        }
    }
}

/// `*` matches any run of characters; nothing else is special.
fn glob(pattern: &str, s: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == s;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !s.starts_with(first) || !s[first.len()..].ends_with(last) || s.len() < first.len() + last.len() {
        return false;
    }
    let mut rest = &s[first.len()..s.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }
    fn vals(v: &[Arg]) -> Vec<&str> {
        v.iter().map(|a| a.value.as_str()).collect()
    }

    #[test]
    fn claude_ids_aliases_and_the_users_own() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        assert_eq!(vals(&values("claude", home)), CLAUDE_MODELS.iter().map(|(v, _)| *v).collect::<Vec<_>>(), "no files: the table");
        put(&home.join(".claude/settings.json"), r#"{"modelSettings": {"claude-opus-5-5": {}, "claude-opus-6-0": {}}, "availableModels": ["claude-haiku-5-6"]}"#);
        put(&home.join(".claude.json"), r#"{"additionalModelOptionsCache": [{"value": "claude-fable-5-1[1m]", "label": "Fable", "description": "Fable 5.1 · Most capable"}]}"#);
        let v = values("claude", home);
        let extra: Vec<(&str, &str)> = v[CLAUDE_MODELS.len()..].iter().map(|a| (a.value.as_str(), a.label.as_str())).collect();
        assert_eq!(extra, vec![("claude-opus-6-0", "Opus 6.0"), ("claude-haiku-5-6", "Haiku 5.6"), ("claude-fable-5-1[1m]", "Fable 5.1")], "a known id is listed once");
        assert_eq!(v[0], Arg::new("claude-fable-5-2", "Fable 5.2"));
    }

    #[test]
    fn claude_labels() {
        assert_eq!(claude_label("claude-sonnet-5-5"), "Sonnet 5.5");
        assert_eq!(claude_label("claude-fable-5-1[1m]"), "Fable 5.1 (1M context)");
        for no in ["opus", "claude-", "claude-opus", "claude-opus-latest", "gpt-6"] {
            assert_eq!(claude_label(no), "", "{no}");
        }
    }

    #[test]
    fn codex_lists_its_picker_models_in_order() {
        let t = tempfile::tempdir().unwrap();
        assert!(values("codex", t.path()).is_empty(), "no cache: no values");
        put(
            &t.path().join(".codex/models_cache.json"),
            r#"{"models": [{"slug": "b", "display_name": "B", "visibility": "list", "priority": 2},
                {"slug": "hidden", "display_name": "H", "visibility": "hide", "priority": 0},
                {"slug": "a", "display_name": "A", "visibility": "list", "priority": 1}]}"#,
        );
        assert_eq!(values("codex", t.path()), vec![Arg::new("a", "A"), Arg::new("b", "B")]);
    }

    #[test]
    fn pi_models_of_providers_with_a_key() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join(".pi/agent");
        put(&a.join("auth.json"), r#"{"router": {"type": "api_key"}}"#);
        put(&a.join("models-store.json"), r#"{"router": {"models": [{"id": "x/one", "name": "One"}]}, "nokey": {"models": [{"id": "no"}]}}"#);
        put(&a.join("models.json"), r#"{"providers": {"router": {"models": [{"id": "x/two", "name": "Two"}]}, "local": {"apiKey": "!cmd", "models": [{"id": "small", "name": "Small"}]}}}"#);
        assert_eq!(
            values("pi", t.path()),
            vec![Arg::new("local/small", "Small"), Arg::new("router/x/one", "One"), Arg::new("router/x/two", "Two")],
            "a provider with no key is left out"
        );
        put(&a.join("settings.json"), r#"{"enabledModels": ["router/x/*", "small:high"]}"#);
        assert_eq!(vals(&values("pi", t.path())), vec!["local/small", "router/x/one", "router/x/two"]);
        put(&a.join("settings.json"), r#"{"enabledModels": ["router/x/one"]}"#);
        assert_eq!(vals(&values("pi", t.path())), vec!["router/x/one"], "only the enabled ones");
    }

    #[test]
    fn agy_and_others() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(values("agy", t.path()).len(), AGY_MODELS.len());
        assert!(values("cursor-agent", t.path()).is_empty());
    }

    #[test]
    fn globs() {
        assert!(glob("a/*", "a/b") && glob("*b", "ab") && glob("a*c*e", "abcde") && glob("x", "x"));
        assert!(!glob("a/*", "b/a") && !glob("a*a", "a") && !glob("x", "xy"));
    }
}
