//! `commands`: the "/" commands of a Claude Code terminal, read from files (no key goes to the agent). The mod shows
//! them above its reply box when the text starts with "/". Sources, as Claude Code finds them:
//! - the built-ins people use most (`BUILTINS`), with an argument hint, the values of /model and /effort, and a mark
//!   on those that open a screen in the terminal when sent bare;
//! - custom commands: `.claude/commands/*.md` in the terminal's folder, then in the user's home;
//! - skills: `.claude/skills/<name>/SKILL.md` in the terminal's folder, then in the home;
//! - plugins (`~/.claude/plugins/installed_plugins.json`, not turned off in `enabledPlugins`): their commands and
//!   skills, as `<plugin>:<name>`.
//!
//! A name seen once is not listed again (the first source wins). Only names Claude Code can be sent are kept.

use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A built-in: name, description (as Claude Code's own menu says it), argument hint, argument values (the mod offers
/// them after "/name "), and whether sent bare it opens a screen in the terminal (a picker, a dialog: the chat cannot
/// show it, the owner answers it in the terminal). Checked against Claude Code 2.1 (spike): /agents is gone, /cost
/// and /review are other names of /usage and /code-review.
pub struct Builtin {
    pub name: &'static str,
    pub description: &'static str,
    pub hint: &'static str,
    pub args: &'static [&'static str],
    pub screen: bool,
}

const fn b(name: &'static str, description: &'static str, hint: &'static str, args: &'static [&'static str], screen: bool) -> Builtin {
    Builtin { name, description, hint, args, screen }
}

pub const MODELS: [&str; 5] = ["opus", "sonnet", "haiku", "fable", "default"];
pub const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub const BUILTINS: [Builtin; 20] = [
    b("add-dir", "Add a new working directory", "<path>", &[], true),
    b("clear", "Start a new conversation (clears the history)", "", &[], false),
    b("compact", "Summarise the conversation so far, to free context", "[instructions]", &[], false),
    b("config", "Open the settings", "", &[], true),
    b("context", "Show how full the context is", "", &[], false),
    b("effort", "Set effort level for model usage", "low|medium|high|xhigh|max", &EFFORTS, true),
    b("export", "Export the current conversation to a file or clipboard", "[file]", &[], true),
    b("help", "Show help and the keys", "", &[], true),
    b("hooks", "Show the hooks", "", &[], true),
    b("init", "Initialize a new CLAUDE.md file with codebase documentation", "", &[], false),
    b("mcp", "Manage MCP servers", "", &[], true),
    b("memory", "Edit the memory and instruction files", "", &[], true),
    b("model", "Set the AI model for Claude Code", "<model>", &MODELS, true),
    b("permissions", "Show and edit the tool permission rules", "", &[], true),
    b("resume", "Resume an earlier conversation", "[session]", &[], true),
    b("review", "Review the current diff or a PR for bugs", "", &[], false),
    b("rewind", "Restore the code and/or conversation to a previous point", "", &[], true),
    b("status", "Show the version, model, account and connections", "", &[], true),
    b("usage", "Show session cost, plan usage, and activity stats", "", &[], true),
    b("plugin", "Manage plugins", "", &[], true),
];

/// At most this many entries, and description characters.
const MAX: usize = 500;
const DESC: usize = 120;
/// A file larger than this is not read for its description (its name is still listed).
const READ_MAX: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Command {
    pub name: String,
    pub description: String,
    /// "builtin", "project", "user", "skill", or "plugin".
    pub source: &'static str,
    /// Built-ins: what may follow the name ("<model>", "[file]").
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hint: String,
    /// Built-ins: the values offered after "/name " (/model, /effort).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Built-ins: sent bare, it opens a screen in the terminal.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub screen: bool,
}

/// The commands for a Claude Code terminal in `cwd`, with the user's home `home`. Sorted by name.
pub fn list(home: &Path, cwd: &Path) -> Vec<Command> {
    let mut out: Vec<Command> = vec![];
    let mut seen = HashSet::new();
    let mut add = |out: &mut Vec<Command>, name: String, description: String, source: &'static str| {
        if out.len() < MAX && valid(&name) && seen.insert(name.clone()) {
            out.push(Command { name, description: cut(&description), source, hint: String::new(), args: vec![], screen: false });
        }
    };
    for b in &BUILTINS {
        add(&mut out, b.name.into(), b.description.into(), "builtin");
        if let Some(c) = out.last_mut().filter(|c| c.name == b.name) {
            c.hint = b.hint.into();
            c.args = b.args.iter().map(|a| a.to_string()).collect();
            c.screen = b.screen;
        }
    }
    let claude = home.join(".claude");
    for (dir, source) in [(cwd.join(".claude/commands"), "project"), (claude.join("commands"), "user")] {
        for (name, desc) in commands_in(&dir) {
            add(&mut out, name, desc, source);
        }
    }
    for dir in [cwd.join(".claude/skills"), claude.join("skills")] {
        for (name, desc) in skills_in(&dir) {
            add(&mut out, name, desc, "skill");
        }
    }
    for (plugin, path) in plugins(&claude, cwd) {
        for (name, desc) in commands_in(&path.join("commands")).into_iter().chain(skills_in(&path.join("skills"))) {
            add(&mut out, format!("{plugin}:{name}"), desc, "plugin");
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A name Claude Code takes after "/": letters, digits and `-_.:`, not empty, at most 100 characters.
fn valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 100 && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c))
}

fn cut(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= DESC {
        return s.to_string();
    }
    format!("{}…", s.chars().take(DESC - 1).collect::<String>().trim_end())
}

/// `<dir>/*.md` and the `.md` files in its subfolders (the folder is not part of the name, as in Claude Code).
fn commands_in(dir: &Path) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut walk = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = walk.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth < 3 {
                    walk.push((p, depth + 1));
                }
            } else if p.extension().is_some_and(|e| e == "md") {
                let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
                let (front, body) = read_md(&p);
                let desc = field(&front, "description").unwrap_or_else(|| first_line(&body));
                out.push((stem.to_string(), desc));
            }
        }
    }
    out
}

/// `<dir>/<name>/SKILL.md`: the name in its front matter, else the folder's. A skill with `user-invocable: false`
/// is not offered after "/" in Claude Code, so it is left out.
fn skills_in(dir: &Path) -> Vec<(String, String)> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.into_iter()
        .filter_map(|d| {
            let f = d.join("SKILL.md");
            if !f.is_file() {
                return None;
            }
            let (front, body) = read_md(&f);
            if field(&front, "user-invocable").is_some_and(|v| v == "false") {
                return None;
            }
            let name = field(&front, "name").or_else(|| d.file_name().and_then(|n| n.to_str()).map(String::from))?;
            Some((name, field(&front, "description").unwrap_or_else(|| first_line(&body))))
        })
        .collect()
}

/// Installed plugins that are on: (name, folder). Turned off with `"name@market": false` in `enabledPlugins` of the
/// user's or the project's settings. A plugin installed for a project counts in that project only.
fn plugins(claude: &Path, cwd: &Path) -> Vec<(String, PathBuf)> {
    let read = |p: &Path| fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let Some(installed) = read(&claude.join("plugins/installed_plugins.json")) else { return vec![] };
    let mut off = HashSet::new();
    for s in [claude.join("settings.json"), cwd.join(".claude/settings.json"), cwd.join(".claude/settings.local.json")] {
        if let Some(m) = read(&s).and_then(|v| v.get("enabledPlugins").and_then(|e| e.as_object().cloned())) {
            for (k, v) in m {
                if v == serde_json::Value::Bool(false) {
                    off.insert(k);
                } else {
                    off.remove(&k);
                }
            }
        }
    }
    let Some(map) = installed.get("plugins").and_then(|p| p.as_object()) else { return vec![] };
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    let mut out = vec![];
    for key in keys {
        if off.contains(key) {
            continue;
        }
        let name = key.split('@').next().unwrap_or(key).to_string();
        let entries = map[key].as_array().cloned().unwrap_or_default();
        let here = entries.iter().find(|e| {
            let scope = e.get("scope").and_then(|s| s.as_str()).unwrap_or("user");
            scope == "user" || e.get("projectPath").and_then(|p| p.as_str()).is_some_and(|p| Path::new(p) == cwd)
        });
        if let Some(path) = here.and_then(|e| e.get("installPath")).and_then(|p| p.as_str()) {
            out.push((name, PathBuf::from(path)));
        }
    }
    out
}

/// A Markdown file's front matter lines (between the first two `---` lines) and its body.
fn read_md(p: &Path) -> (Vec<String>, String) {
    if fs::metadata(p).map(|m| m.len() > READ_MAX).unwrap_or(true) {
        return (vec![], String::new());
    }
    let text = fs::read_to_string(p).unwrap_or_default();
    let mut lines = text.lines();
    if lines.next().map(str::trim) == Some("---") {
        let mut front = vec![];
        for l in lines.by_ref() {
            if l.trim() == "---" {
                return (front, lines.collect::<Vec<_>>().join("\n"));
            }
            front.push(l.to_string());
        }
        return (vec![], text);
    }
    (vec![], text)
}

/// `key: value` in front matter, its quotes taken off.
fn field(front: &[String], key: &str) -> Option<String> {
    front.iter().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        (k.trim() == key).then(|| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string()).filter(|v| !v.is_empty())
    })
}

/// The body's first line with text, without a heading's `#`.
fn first_line(body: &str) -> String {
    body.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    fn names(v: &[Command]) -> Vec<(&str, &str)> {
        v.iter().map(|c| (c.name.as_str(), c.source)).collect()
    }

    #[test]
    fn builtins_only_without_files() {
        let t = tempfile::tempdir().unwrap();
        let v = list(&t.path().join("home"), &t.path().join("proj"));
        let got: Vec<&str> = v.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            got,
            vec![
                "add-dir", "clear", "compact", "config", "context", "effort", "export", "help", "hooks", "init", "mcp", "memory", "model",
                "permissions", "plugin", "resume", "review", "rewind", "status", "usage"
            ]
        );
        assert!(v.iter().all(|c| c.source == "builtin"));
        let c = |n: &str| v.iter().find(|c| c.name == n).unwrap();
        assert_eq!(c("model").args, vec!["opus", "sonnet", "haiku", "fable", "default"]);
        assert_eq!(c("effort").args, vec!["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(c("model").hint, "<model>");
        assert!(c("model").screen && c("config").screen && !c("clear").screen && !c("context").screen);
        // On the wire: hint, args and screen only where they say something.
        assert_eq!(serde_json::to_value(c("clear")).unwrap(), serde_json::json!({"name": "clear", "description": "Start a new conversation (clears the history)", "source": "builtin"}));
        assert_eq!(serde_json::to_value(c("add-dir")).unwrap()["hint"], "<path>");
    }

    #[test]
    fn custom_commands_skills_and_plugins() {
        let t = tempfile::tempdir().unwrap();
        let (home, proj) = (t.path().join("home"), t.path().join("proj"));
        put(&proj.join(".claude/commands/deploy.md"), "---\ndescription: \"Deploy the app\"\n---\nRun the deploy.");
        put(&proj.join(".claude/commands/ops/restart.md"), "# Restart the server\n\nThen check.");
        put(&home.join(".claude/commands/deploy.md"), "---\ndescription: the user's deploy\n---\n");
        put(&home.join(".claude/commands/lint.md"), "Lint the diff.");
        put(&home.join(".claude/commands/bad name.md"), "a name with a space");
        put(&home.join(".claude/skills/notes/SKILL.md"), "---\nname: take-notes\ndescription: Write notes\n---\nbody");
        put(&home.join(".claude/skills/hidden/SKILL.md"), "---\ndescription: model only\nuser-invocable: false\n---\n");
        put(&home.join(".claude/skills/empty-dir/README.md"), "not a skill");
        let plug = t.path().join("plug");
        put(&plug.join("commands/hello.md"), "---\ndescription: Say hello\n---\n");
        put(&plug.join("skills/tidy/SKILL.md"), "---\nname: tidy\ndescription: Tidy up\n---\n");
        let off = t.path().join("off");
        put(&off.join("commands/nope.md"), "x");
        let other = t.path().join("other");
        put(&other.join("commands/elsewhere.md"), "x");
        put(
            &home.join(".claude/plugins/installed_plugins.json"),
            &serde_json::json!({"version": 2, "plugins": {
                "kit@market": [{"scope": "user", "installPath": plug}],
                "off@market": [{"scope": "user", "installPath": off}],
                "proj@market": [{"scope": "project", "projectPath": "/somewhere/else", "installPath": other}],
            }})
            .to_string(),
        );
        put(&home.join(".claude/settings.json"), r#"{"enabledPlugins": {"off@market": false, "kit@market": true}}"#);
        let v = list(&home, &proj);
        let own: Vec<Command> = v.iter().filter(|c| c.source != "builtin").cloned().collect();
        assert_eq!(
            names(&own),
            vec![
                ("deploy", "project"),
                ("kit:hello", "plugin"),
                ("kit:tidy", "plugin"),
                ("lint", "user"),
                ("restart", "project"),
                ("take-notes", "skill"),
            ]
        );
        let d = |n: &str| v.iter().find(|c| c.name == n).unwrap().description.clone();
        assert_eq!(d("deploy"), "Deploy the app", "the project's command wins over the user's, quotes off");
        assert_eq!(d("restart"), "Restart the server", "no front matter: the first line, without its #");
        assert_eq!(d("lint"), "Lint the diff.");
        assert_eq!(d("kit:hello"), "Say hello");
    }

    #[test]
    fn long_descriptions_are_cut() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        put(&home.join(".claude/commands/long.md"), &"word ".repeat(100));
        let v = list(&home, &t.path().join("proj"));
        let d = &v.iter().find(|c| c.name == "long").unwrap().description;
        assert!(d.chars().count() <= DESC && d.ends_with('…'), "{d}");
    }
}
