//! `commands`: the "/" commands of an agent's terminal, read from files (no key goes to the agent). The mod shows them
//! above its reply box when the text starts with "/". Per agent kind (`list`), as each agent finds them:
//! - Claude Code: its common built-ins (`CLAUDE`); custom commands, `.claude/commands/**/*.md` in the terminal's
//!   folder, then in the user's home; skills, `.claude/skills/<name>/SKILL.md` there and in the home; plugins
//!   (`~/.claude/plugins/installed_plugins.json`, not turned off in `enabledPlugins`) as `<plugin>:<name>`.
//! - Codex: a short set of its built-ins (`CODEX`). Its skills are not "/" commands, and it has no prompt files here.
//! - pi: its built-ins (`PI`); prompt templates, `~/.pi/agent/prompts/*.md` (top level; the project's need its trust,
//!   so they are left out); skills as `skill:<name>` from `~/.pi/agent/skills`, `~/.agents/skills`, the project's
//!   `.pi/skills`, and `.agents/skills` from the folder up to the repository root (off with `enableSkillCommands`).
//! - agy (Antigravity CLI): its built-ins (`AGY`, one row a command, other names in the description); skills from
//!   `~/.gemini/skills` and `~/.gemini/antigravity/skills`.
//! - Any other agent: nothing. Commands an agent's extensions add (pi's npm packages) cannot be read from files.
//!
//! Built-ins carry an argument hint, fixed argument values where the agent has them (/model, /effort, /thinking),
//! and a mark on those that open a screen in the terminal when sent bare (spike, Claude Code 2.1, Codex 0.167,
//! pi 1.3, agy 1.5). A name seen once is not listed again (the first source wins).
//!
//! A screen built-in sent bare ("/model") is not sent (`opens_screen`, in the mod and in `send`): the chat cannot
//! show the screen. One that takes a value stays in the list ("/model sonnet" goes out); one that takes none
//! (/status, /config) can never be sent from the chat, so `list` leaves it out and `screens` names it.

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
pub const PI_THINKING: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/// Claude Code 2.1 (spike: /agents is gone; /cost and /review are other names of /usage and /code-review).
pub const CLAUDE: [Builtin; 20] = [
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

/// Codex 0.167: a short set of its about 60 (descriptions as its menu says them).
pub const CODEX: [Builtin; 20] = [
    b("clear", "Clear the terminal and start a new chat", "", &[], false),
    b("compact", "Summarize conversation to prevent hitting the context limit", "", &[], false),
    b("config", "Configure Codex preferences", "", &[], true),
    b("copy", "Copy the last response or part of it", "", &[], false),
    b("diff", "Show git diff (including untracked files)", "", &[], false),
    b("export", "Export the conversation as markdown", "", &[], false),
    b("fork", "Fork the current chat", "", &[], false),
    b("goal", "Set or view the goal for a long-running task", "[goal]", &[], false),
    b("init", "Create an AGENTS.md file with instructions for Codex", "", &[], false),
    b("mcp", "List MCP tools", "[verbose]", &[], false),
    b("model", "Choose what model and reasoning effort to use", "", &[], true),
    b("new", "Start a new chat during a conversation", "", &[], false),
    b("permissions", "Choose what Codex is allowed to do", "", &[], true),
    b("plan", "Switch to Plan mode", "", &[], false),
    b("rename", "Rename the current thread", "<name>", &[], false),
    b("resume", "Resume a saved chat", "", &[], true),
    b("review", "Review my current changes and find issues", "", &[], false),
    b("status", "Show current session configuration and token usage", "", &[], false),
    b("usage", "View account usage or use a usage limit reset", "", &[], false),
    b("skills", "Use skills to improve how Codex performs specific tasks", "", &[], true),
];

/// pi 1.3 (docs/slash-commands.md and its menu).
pub const PI: [Builtin; 17] = [
    b("changelog", "Show changelog entries", "", &[], false),
    b("clone", "Duplicate the current session at the current position", "", &[], false),
    b("compact", "Compact the session context, optionally with instructions", "[instructions]", &[], false),
    b("copy", "Copy the last agent message to the clipboard", "", &[], false),
    b("export", "Export the session as HTML or JSONL", "[path]", &[], false),
    b("fork", "Create a new fork from a previous user message", "", &[], true),
    b("hotkeys", "Show all keyboard shortcuts", "", &[], false),
    b("model", "Select a model", "[provider/model]", &[], true),
    b("name", "Set the session display name", "[name]", &[], false),
    b("new", "Start a new session", "", &[], false),
    b("reload", "Reload keybindings, extensions, skills, prompts, themes and context files", "", &[], false),
    b("resume", "Resume a different session", "", &[], true),
    b("scoped-models", "Enable or disable models for Ctrl+P cycling", "", &[], true),
    b("session", "Show session info and stats", "", &[], false),
    b("settings", "Open the settings menu", "", &[], true),
    b("thinking", "Set the thinking level", "[level]", &PI_THINKING, true),
    b("tree", "Navigate the session tree", "", &[], true),
];

/// agy 1.5 (Antigravity CLI): one row a command; its other names are in the description.
pub const AGY: [Builtin; 28] = [
    b("add-dir", "Add a directory to the workspace", "<path>", &[], false),
    b("agents", "List available custom agents", "", &[], false),
    b("artifact", "View and review artifacts", "", &[], true),
    b("btw", "Ask a side question without interrupting the current task", "<question>", &[], false),
    b("changelog", "Show release notes and changes", "", &[], false),
    b("clear", "Clear conversation and start a new one (also /new)", "", &[], false),
    b("codesearch", "Search code in the workspace (also /cs)", "<query>", &[], false),
    b("compact", "Compact the conversation now", "", &[], false),
    b("config", "Open settings panel (also /settings)", "", &[], true),
    b("context", "Visualize current context usage", "", &[], true),
    b("copy", "Copy the last planner response to the clipboard", "[n]", &[], false),
    b("credits", "Show remaining credits", "", &[], false),
    b("diff", "View uncommitted changes and per-turn diffs", "", &[], true),
    b("effort", "Set the reasoning effort", "[level]", &EFFORTS, true),
    b("fork", "Create a branch of the conversation at this point (also /branch)", "[name]", &[], false),
    b("goal", "Run until the specified goal is completely finished", "<goal>", &[], false),
    b("help", "Show available commands and keybindings", "", &[], true),
    b("hooks", "Manage hook configurations for tool events", "", &[], true),
    b("mcp", "Manage MCP servers", "", &[], true),
    b("model", "Set a model, or run a single prompt on another model", "[model]", &[], true),
    b("permissions", "Manage tool permissions", "", &[], true),
    b("plan", "Plan carefully before executing a task", "[task]", &[], false),
    b("plugin", "Manage plugins", "", &[], true),
    b("rename", "Rename the current conversation", "<name>", &[], false),
    b("resume", "Browse and resume past conversations (also /switch)", "", &[], true),
    b("rewind", "Rewind conversation to a previous message (also /undo)", "", &[], true),
    b("skills", "List available skills", "", &[], false),
    b("usage", "View model quota usage (also /quota)", "", &[], true),
];

/// The built-ins of an agent of `kind` (none for an unknown kind).
pub fn builtins(kind: &str) -> &'static [Builtin] {
    match kind {
        "claude" => &CLAUDE,
        "codex" => &CODEX,
        "pi" => &PI,
        "agy" => &AGY,
        _ => &[],
    }
}

impl Builtin {
    /// It takes a value after its name (a hint or fixed values).
    pub fn takes_value(&self) -> bool {
        !self.hint.is_empty() || !self.args.is_empty()
    }
}

/// The screen built-ins of `kind` that take no value: never sent from the chat, so not in `list`. Sorted by name.
pub fn screens(kind: &str) -> Vec<&'static str> {
    let mut v: Vec<&str> = builtins(kind).iter().filter(|b| b.screen && !b.takes_value()).map(|b| b.name).collect();
    v.sort_unstable();
    v
}

/// `text` is a screen built-in of `kind` sent bare ("/model", spaces around it trimmed): why it is not sent, for the
/// person. Anything after the name ("/model sonnet"), or any other text: `None`.
pub fn opens_screen(kind: &str, text: &str) -> Option<String> {
    let name = text.trim().strip_prefix('/')?;
    let b = builtins(kind).iter().find(|b| b.screen && b.name == name)?;
    Some(screen_note(b.name, b.args.first().copied().unwrap_or(b.hint), b.takes_value()))
}

/// The note for a bare screen command. The mod builds the same text (composer.js `screenNote`).
pub fn screen_note(name: &str, example: &str, takes_value: bool) -> String {
    if takes_value {
        format!("/{name} opens a menu in the terminal. Add a value, like /{name} {example}.")
    } else {
        format!("/{name} opens a screen in the terminal. Use it there.")
    }
}

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
    /// Built-ins: sent bare, it opens a screen in the terminal, so it needs a value to be sent (`opens_screen`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub screen: bool,
}

/// The commands of an agent of `kind` in `cwd`, with the user's home `home`. Sorted by name. Unknown kinds: none.
pub fn list(kind: &str, home: &Path, cwd: &Path) -> Vec<Command> {
    let mut l = Lister::default();
    match kind {
        "claude" => {
            l.builtins(&CLAUDE);
            let claude = home.join(".claude");
            for (dir, source) in [(cwd.join(".claude/commands"), "project"), (claude.join("commands"), "user")] {
                for (name, desc, hint) in commands_in(&dir, 3) {
                    l.add(name, desc, source, hint);
                }
            }
            for dir in [cwd.join(".claude/skills"), claude.join("skills")] {
                for (name, desc) in skills_in(&dir, 0) {
                    l.add(name, desc, "skill", String::new());
                }
            }
            for (plugin, path) in plugins(&claude, cwd) {
                for (name, desc, hint) in commands_in(&path.join("commands"), 3) {
                    l.add(format!("{plugin}:{name}"), desc, "plugin", hint);
                }
                for (name, desc) in skills_in(&path.join("skills"), 0) {
                    l.add(format!("{plugin}:{name}"), desc, "plugin", String::new());
                }
            }
        }
        "codex" => l.builtins(&CODEX),
        "pi" => {
            l.builtins(&PI);
            let agent = home.join(".pi/agent");
            for (name, desc, hint) in commands_in(&agent.join("prompts"), 0) {
                l.add(name, desc, "user", hint);
            }
            if pi_skill_commands(&agent) {
                let mut dirs = vec![cwd.join(".pi/skills")];
                dirs.extend(up_to_repo(cwd).into_iter().map(|d| d.join(".agents/skills")));
                dirs.extend([agent.join("skills"), home.join(".agents/skills")]);
                for d in dirs {
                    for (name, desc) in skills_in(&d, 3) {
                        l.add(format!("skill:{name}"), desc, "skill", String::new());
                    }
                }
            }
        }
        "agy" => {
            l.builtins(&AGY);
            for d in [home.join(".gemini/skills"), home.join(".gemini/antigravity/skills")] {
                for (name, desc) in skills_in(&d, 0) {
                    l.add(name, desc, "skill", String::new());
                }
            }
        }
        _ => {}
    }
    let mut out = l.out;
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[derive(Default)]
struct Lister {
    out: Vec<Command>,
    seen: HashSet<String>,
}

impl Lister {
    fn add(&mut self, name: String, description: String, source: &'static str, hint: String) -> Option<&mut Command> {
        if self.out.len() >= MAX || !valid(&name) || !self.seen.insert(name.clone()) {
            return None;
        }
        self.out.push(Command { name, description: cut(&description), source, hint: cut(&hint), args: vec![], screen: false });
        self.out.last_mut()
    }

    fn builtins(&mut self, list: &[Builtin]) {
        for b in list {
            // A screen with no value can never be sent from the chat: left out, and its name stays the built-in's.
            if b.screen && !b.takes_value() {
                self.seen.insert(b.name.into());
                continue;
            }
            if let Some(c) = self.add(b.name.into(), b.description.into(), "builtin", b.hint.into()) {
                c.args = b.args.iter().map(|a| a.to_string()).collect();
                c.screen = b.screen;
            }
        }
    }
}

/// pi's `enableSkillCommands` (default on), from its settings.
fn pi_skill_commands(agent: &Path) -> bool {
    let v = fs::read(agent.join("settings.json")).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    v.and_then(|v| v.get("enableSkillCommands").and_then(|e| e.as_bool())).unwrap_or(true)
}

/// `cwd` and its parents, up to the first one with a `.git` (the repository root), or only `cwd` outside one.
fn up_to_repo(cwd: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    for d in cwd.ancestors().take(20) {
        out.push(d.to_path_buf());
        if d.join(".git").exists() {
            return out;
        }
    }
    vec![cwd.to_path_buf()]
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

/// `<dir>/*.md`, and the `.md` files in its subfolders down to `depth` (the folder is not part of the name, as in
/// Claude Code): (name, description, argument hint).
fn commands_in(dir: &Path, depth_max: usize) -> Vec<(String, String, String)> {
    let mut out = vec![];
    let mut walk = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = walk.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth < depth_max {
                    walk.push((p, depth + 1));
                }
            } else if p.extension().is_some_and(|e| e == "md") {
                let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
                let (front, body) = read_md(&p);
                let desc = field(&front, "description").unwrap_or_else(|| first_line(&body));
                out.push((stem.to_string(), desc, field(&front, "argument-hint").unwrap_or_default()));
            }
        }
    }
    out
}

/// `<dir>/<name>/SKILL.md`, and down to `depth` more folders (a folder with a SKILL.md is not looked into): the name
/// in its front matter, else the folder's. A skill with `user-invocable: false` is not offered after "/", so it is
/// left out.
fn skills_in(dir: &Path, depth: usize) -> Vec<(String, String)> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    let mut dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    dirs.sort();
    let mut out = vec![];
    for d in dirs {
        let f = d.join("SKILL.md");
        if !f.is_file() {
            if depth > 0 {
                out.extend(skills_in(&d, depth - 1));
            }
            continue;
        }
        let (front, body) = read_md(&f);
        if field(&front, "user-invocable").is_some_and(|v| v == "false") {
            continue;
        }
        let Some(name) = field(&front, "name").or_else(|| d.file_name().and_then(|n| n.to_str()).map(String::from)) else { continue };
        out.push((name, field(&front, "description").unwrap_or_else(|| first_line(&body))));
    }
    out
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
        let v = list("claude", &t.path().join("home"), &t.path().join("proj"));
        let got: Vec<&str> = v.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            got,
            vec!["add-dir", "clear", "compact", "context", "effort", "export", "init", "model", "resume", "review"],
            "screens with no value (/status, /config …) are left out"
        );
        assert!(v.iter().all(|c| c.source == "builtin"));
        let c = |n: &str| v.iter().find(|c| c.name == n).unwrap();
        assert_eq!(c("model").args, vec!["opus", "sonnet", "haiku", "fable", "default"]);
        assert_eq!(c("effort").args, vec!["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(c("model").hint, "<model>");
        assert!(c("model").screen && c("resume").screen && !c("clear").screen && !c("context").screen);
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
        let v = list("claude", &home, &proj);
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
        let v = list("claude", &home, &t.path().join("proj"));
        let d = &v.iter().find(|c| c.name == "long").unwrap().description;
        assert!(d.chars().count() <= DESC && d.ends_with('…'), "{d}");
    }

    #[test]
    fn codex_gets_its_builtins_only() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        put(&home.join(".codex/prompts/mine.md"), "a prompt");
        put(&home.join(".codex/skills/s/SKILL.md"), "---\nname: s\n---\n");
        put(&home.join(".claude/commands/claude-only.md"), "x");
        let v = list("codex", &home, &t.path().join("proj"));
        assert!(v.iter().all(|c| c.source == "builtin"), "no prompts, no skills, nothing of Claude's");
        assert_eq!(v.len(), CODEX.len() - screens("codex").len());
        let c = |n: &str| v.iter().find(|c| c.name == n).unwrap();
        assert!(!c("status").screen, "Codex's /status prints in the chat");
        assert_eq!(screens("codex"), vec!["config", "model", "permissions", "resume", "skills"]);
        assert_eq!(c("rename").hint, "<name>");
    }

    #[test]
    fn pi_builtins_prompts_and_skill_commands() {
        let t = tempfile::tempdir().unwrap();
        let (home, repo) = (t.path().join("home"), t.path().join("repo"));
        let proj = repo.join("app/web");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(&proj).unwrap();
        put(&home.join(".pi/agent/prompts/review.md"), "---\ndescription: Review staged changes\nargument-hint: \"[focus]\"\n---\nReview.");
        put(&home.join(".pi/agent/prompts/nested/deep.md"), "not loaded: top level only");
        put(&proj.join(".pi/prompts/project.md"), "not loaded: needs the project's trust");
        put(&home.join(".pi/agent/skills/notes/SKILL.md"), "---\nname: notes\ndescription: Take notes\n---\n");
        put(&home.join(".pi/agent/skills/group/inner/SKILL.md"), "---\nname: inner\n---\nFound in a subfolder");
        put(&home.join(".agents/skills/shared/SKILL.md"), "---\nname: shared\n---\n");
        put(&proj.join(".pi/skills/local/SKILL.md"), "---\nname: local\n---\n");
        put(&repo.join("app/.agents/skills/mid/SKILL.md"), "---\nname: mid\n---\n");
        put(&repo.join(".agents/skills/top/SKILL.md"), "---\nname: top\n---\n");
        put(&t.path().join(".agents/skills/outside/SKILL.md"), "---\nname: outside\n---\n");
        let v = list("pi", &home, &proj);
        let own: Vec<(&str, &str)> = v.iter().filter(|c| c.source != "builtin").map(|c| (c.name.as_str(), c.source)).collect();
        assert_eq!(
            own,
            vec![
                ("review", "user"),
                ("skill:inner", "skill"),
                ("skill:local", "skill"),
                ("skill:mid", "skill"),
                ("skill:notes", "skill"),
                ("skill:shared", "skill"),
                ("skill:top", "skill"),
            ],
            "no nested or project prompts; .agents/skills up to the repository root, not past it"
        );
        let c = |n: &str| v.iter().find(|c| c.name == n).unwrap();
        assert_eq!(c("review").hint, "[focus]");
        assert_eq!(c("review").description, "Review staged changes");
        assert_eq!(c("thinking").args, vec!["off", "minimal", "low", "medium", "high", "xhigh", "max"]);
        assert!(c("thinking").screen && c("model").screen && !c("session").screen);
        assert_eq!(screens("pi"), vec!["fork", "resume", "scoped-models", "settings", "tree"]);
        put(&home.join(".pi/agent/settings.json"), r#"{"enableSkillCommands": false}"#);
        assert!(list("pi", &home, &proj).iter().all(|c| !c.name.starts_with("skill:")), "skill commands turned off");
    }

    #[test]
    fn agy_builtins_and_its_two_skill_folders() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        put(&home.join(".gemini/skills/one/SKILL.md"), "---\nname: one\ndescription: First\n---\n");
        put(&home.join(".gemini/antigravity/skills/two/SKILL.md"), "---\nname: two\n---\nSecond");
        put(&home.join(".gemini/antigravity/skills/one/SKILL.md"), "---\nname: one\n---\nlisted once");
        let v = list("agy", &home, &t.path().join("proj"));
        let own: Vec<(&str, &str)> = v.iter().filter(|c| c.source != "builtin").map(|c| (c.name.as_str(), c.description.as_str())).collect();
        assert_eq!(own, vec![("one", "First"), ("two", "Second")]);
        assert!(v.iter().all(|c| !["new", "settings", "switch", "undo", "quota", "cs", "branch"].contains(&c.name.as_str())), "other names are not rows");
        let c = |n: &str| v.iter().find(|c| c.name == n).unwrap();
        assert!(c("clear").description.contains("/new"));
        assert_eq!(c("effort").args, vec!["low", "medium", "high", "xhigh", "max"]);
    }

    #[test]
    fn bare_screen_commands_are_not_sent() {
        assert_eq!(opens_screen("claude", "/model").as_deref(), Some("/model opens a menu in the terminal. Add a value, like /model opus."));
        assert_eq!(opens_screen("claude", "  /status \n").as_deref(), Some("/status opens a screen in the terminal. Use it there."));
        assert_eq!(
            opens_screen("claude", "/resume").as_deref(),
            Some("/resume opens a menu in the terminal. Add a value, like /resume [session]."),
            "no fixed values: the hint"
        );
        for ok in ["/model sonnet", "/status please", "/clear", "/compact", "/modelx", "model", "hello /model", "/"] {
            assert_eq!(opens_screen("claude", ok), None, "{ok}");
        }
        assert!(opens_screen("codex", "/model").is_some() && opens_screen("codex", "/status").is_none(), "per agent");
        assert!(opens_screen("pi", "/thinking").is_some() && opens_screen("agy", "/usage").is_some());
        assert_eq!(opens_screen("cursor-agent", "/model"), None);
        assert_eq!(screens("claude"), vec!["config", "help", "hooks", "mcp", "memory", "permissions", "plugin", "rewind", "status", "usage"]);
        // Every screen command is either listed (it takes a value) or named in `screens`.
        for kind in ["claude", "codex", "pi", "agy"] {
            for b in builtins(kind).iter().filter(|b| b.screen) {
                assert!(b.takes_value() != screens(kind).contains(&b.name), "{kind} /{}", b.name);
            }
        }
    }

    #[test]
    fn a_left_out_screen_keeps_its_name() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        put(&home.join(".claude/commands/status.md"), "My own status");
        let v = list("claude", &home, &t.path().join("proj"));
        assert!(v.iter().all(|c| c.name != "status"), "the built-in runs, not the file: not listed");
    }

    #[test]
    fn other_agents_get_nothing() {
        let t = tempfile::tempdir().unwrap();
        assert!(list("cursor-agent", &t.path().join("home"), &t.path().join("proj")).is_empty());
        assert!(list("agent", &t.path().join("home"), &t.path().join("proj")).is_empty());
    }
}
