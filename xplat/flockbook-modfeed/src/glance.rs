//! `glance`: what each agent's last turn did, read from its log by rules. No model and no network. Rust only.
//!
//! One `glance` line per agent on this machine. The facts are counts from the log's tool calls (files edited,
//! commands run, whether the last command failed, whether a test command ran after the last edit) and the first
//! sentence of the last message. The mod puts them on the chat's "Since you left" card.

use crate::{locate, rows, tail};
use herdr_core::resolver::SessionResolver;
use herdr_core::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// The last turn: everything after the newest message of the owner (or of another agent).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Facts {
    /// The id of the turn's newest item. Another id: the agent wrote more.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub id: Option<String>,
    /// When the turn's newest item came (milliseconds since 1970).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ended: Option<i64>,
    /// Files the edit tools changed (each file once). An edit made by a shell command is not seen.
    pub edited: usize,
    /// Shell commands with a result.
    pub commands: usize,
    /// The newest finished command, when it failed: its first line.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failed: Option<String>,
    /// The newest test command after the last edit: "passed", "failed", or "ran" (its exit code went into a pipe).
    /// "none": code files were edited and no test command finished after the last edit. Absent: no test, and no
    /// edit but to files of words (`.md`, `.txt`, `.csv`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tests: Option<String>,
    /// That test command's short name ("pytest", "cargo test").
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub test: Option<String>,
    /// The first sentence of the newest message.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub said: Option<String>,
}

pub const SAID_CAP: usize = 160;
pub const COMMAND_CAP: usize = 120;
/// A change to what a line holds: lines kept by an older helper are read again.
const KEPT_VERSION: u32 = 4;

fn is_edit(tool: &str) -> bool {
    matches!(tool, "Edit" | "MultiEdit" | "NotebookEdit" | "Write" | "apply_patch")
}

fn is_command(tool: &str) -> bool {
    matches!(tool, "Bash" | "exec" | "exec_command" | "shell" | "local_shell" | "container.exec")
}

/// The files one edit call names. A patch names each in its own header line; other tools have the path as summary.
fn edit_files(c: &ToolCall) -> Vec<String> {
    if c.tool_name == "apply_patch" {
        let text = c.input.replace("\\n", "\n");
        let files: Vec<String> = text
            .lines()
            .filter_map(|l| ["*** Update File: ", "*** Add File: ", "*** Delete File: "].iter().find_map(|m| l.trim().strip_prefix(m)))
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty())
            .collect();
        if !files.is_empty() {
            return files;
        }
    }
    vec![if c.summary.trim().is_empty() { format!("call {}", c.id) } else { c.summary.trim().to_string() }]
}

/// A file of words or data rows, by its name: no test is expected after an edit to one.
fn prose(file: &str) -> bool {
    let name = file.rsplit(['/', '\\']).next().unwrap_or(file).to_lowercase();
    matches!(name.rsplit_once('.').map(|x| x.1), Some("md" | "mdx" | "markdown" | "txt" | "rst" | "adoc" | "org" | "csv" | "tsv" | "log"))
}

/// A finished command failed. Claude Code, pi and Grok logs say so. Codex's log has only the output, which starts
/// with the exit code.
fn failed(c: &ToolCall, agent: &str) -> bool {
    if c.is_error {
        return true;
    }
    if agent == "claude" {
        return false;
    }
    let head: String = c.output.as_deref().unwrap_or("").chars().take(400).collect();
    for mark in ["Process exited with code ", "Exit code: ", "exit code: ", "\"exit_code\":"] {
        if let Some(i) = head.find(mark) {
            let n: String = head[i + mark.len()..].trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = n.parse::<i64>() {
                return n != 0;
            }
        }
    }
    false
}

/// The command split where a shell starts the next one (`&&`, `||`, `;`, `|`, a new line), outside quotes. Each part
/// with whether its output goes into a pipe. Text after a here-document mark is data, not commands: left out.
fn parts(cmd: &str) -> Vec<(String, bool)> {
    let cmd = cmd.split("<<").next().unwrap_or("");
    let (mut out, mut cur, mut quote, mut esc): (Vec<(String, bool)>, String, Option<char>, bool) = (vec![], String::new(), None, false);
    let mut it = cmd.chars().peekable();
    while let Some(ch) = it.next() {
        if esc {
            cur.push(ch);
            esc = false;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            } else if ch == '\\' && q == '"' {
                esc = true;
            }
            cur.push(ch);
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                cur.push(ch);
            }
            '\\' => esc = true,
            ';' | '\n' => out.push((std::mem::take(&mut cur), false)),
            '&' if it.peek() == Some(&'&') => {
                it.next();
                out.push((std::mem::take(&mut cur), false));
            }
            '|' => {
                let or = it.peek() == Some(&'|');
                if or {
                    it.next();
                }
                out.push((std::mem::take(&mut cur), !or));
            }
            _ => cur.push(ch),
        }
    }
    out.push((cur, false));
    out.into_iter().filter(|(p, _)| !p.trim().is_empty()).collect()
}

/// Programs that run tests themselves: their own `--` passes arguments on, it does not start another command.
const TESTS: [&str; 12] = ["cargo", "npm", "yarn", "pnpm", "bun", "go", "swift", "deno", "dotnet", "node", "make", "pytest"];

/// The test runner one command part starts, by name.
fn runner(part: &str) -> Option<String> {
    let mut w: Vec<&str> = part.split_whitespace().collect();
    // Leading `A=b` settings and the words that only start another program.
    loop {
        let n = match w.as_slice() {
            [first, ..] if first.contains('=') && !first.starts_with('-') => 1,
            ["sudo" | "time" | "env" | "npx" | "bunx" | "command" | "exec", ..] => 1,
            ["pnpm" | "bundle" | "yarn", "exec", ..] | ["uv" | "poetry" | "pipenv" | "pdm" | "hatch", "run", ..] => 2,
            ["python" | "python3" | "py", "-m", ..] => 2,
            ["nice" | "caffeinate" | "nohup", ..] => 1,
            ["timeout" | "gtimeout", _, ..] => 2,
            // A runner that takes the real command after `--` (a queue, a sandbox, a secrets wrapper).
            [first, rest @ ..] if !first.starts_with('-') && rest.contains(&"--") && TESTS.iter().all(|t| t != first) => rest.iter().position(|a| *a == "--").unwrap_or(0) + 2,
            _ => break,
        };
        w.drain(..n);
    }
    let first = w.first()?.rsplit('/').next().unwrap_or("");
    let rest = &w[1..];
    let has = |x: &str| rest.contains(&x);
    let script = |r: &[&str]| match r {
        ["test" | "t", ..] => true,
        ["run", s, ..] => s.starts_with("test"),
        [s, ..] => s.starts_with("test:"),
        [] => false,
    };
    let name = match first {
        "pytest" | "py.test" => "pytest",
        "unittest" => "unittest",
        "tox" | "nox" | "jest" | "vitest" | "mocha" | "ava" | "rspec" | "phpunit" | "ctest" => first,
        "npm" | "yarn" | "pnpm" | "bun" if script(rest) => return Some(format!("{first} test")),
        "go" if rest.first() == Some(&"test") => "go test",
        "cargo" if rest.iter().find(|a| !a.starts_with(['+', '-'])) == Some(&"test") => "cargo test",
        "cargo" if rest.iter().find(|a| !a.starts_with(['+', '-'])) == Some(&"nextest") => "cargo nextest",
        "swift" if rest.first() == Some(&"test") => "swift test",
        "deno" if rest.first() == Some(&"test") => "deno test",
        "dotnet" if rest.first() == Some(&"test") => "dotnet test",
        "node" if has("--test") => "node --test",
        "playwright" if rest.first() == Some(&"test") => "playwright test",
        "mvn" | "mvnw" if has("test") || has("verify") => "mvn test",
        "gradle" | "gradlew" if rest.iter().any(|a| a.to_lowercase().ends_with("test") || *a == "check") => "gradle test",
        "make" | "just" if rest.iter().any(|a| a.starts_with("test") || *a == "check") => return Some(format!("{first} test")),
        "xcodebuild" if has("test") => "xcodebuild test",
        _ => return None,
    };
    Some(name.to_string())
}

/// The test command in `cmd`: its short name, and whether its output goes into a pipe (the shell then gives the
/// exit code of the last program, not of the tests).
fn test_run(cmd: &str) -> Option<(String, bool)> {
    let pipefail = cmd.contains("pipefail");
    parts(cmd).into_iter().find_map(|(p, piped)| runner(&p).map(|n| (n, piped && !pipefail)))
}

/// The short name of the test runner a shell command starts ("pytest", "npm test"), by the program's name.
pub fn test_name(cmd: &str) -> Option<String> {
    test_run(cmd).map(|t| t.0)
}

/// The first sentence of a message, without markdown marks: the first line with text outside a code block, up to
/// its first full stop, question or exclamation mark that ends a sentence.
pub fn first_sentence(text: &str) -> Option<String> {
    let mut code = false;
    for raw in text.lines() {
        let l = raw.trim();
        if l.starts_with("```") || l.starts_with("~~~") {
            code = !code;
            continue;
        }
        if code {
            continue;
        }
        let mut l = l.trim_start_matches(['#', '>', ' ']);
        for bullet in ["- ", "* ", "+ ", "• "] {
            l = l.strip_prefix(bullet).unwrap_or(l);
        }
        if let Some((n, after)) = l.split_once(". ") {
            if !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()) {
                l = after;
            }
        }
        let l = short_paths(&unlinked(&l.replace("**", "").replace("__", "").replace('`', "")));
        let l = l.trim();
        if !l.chars().any(char::is_alphanumeric) {
            continue;
        }
        let cs: Vec<(usize, char)> = l.char_indices().collect();
        let mut end = l.len();
        for (n, (i, c)) in cs.iter().enumerate() {
            let next = cs.get(n + 1).map(|x| x.1);
            let stop = match c {
                '。' | '！' | '？' => true,
                '.' | '!' | '?' => next.is_some_and(char::is_whitespace),
                _ => false,
            };
            if stop {
                end = i + c.len_utf8();
                break;
            }
        }
        return Some(rows::cut(&l[..end], SAID_CAP));
    }
    None
}

/// A path as its last name only: "/w/src/a.rs" and "~/w/a.rs" read "a.rs". A path here starts with "/" or "~/"
/// and has two or more "/"; a web address stays whole.
fn short_paths(s: &str) -> String {
    let part = |c: char| c.is_ascii_alphanumeric() || "/._-~+@".contains(c);
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        let before = i.checked_sub(1).map(|b| cs[b]);
        if (cs[i] == '/' || (cs[i] == '~' && cs.get(i + 1) == Some(&'/'))) && !before.is_some_and(|b| part(b) || b == ':') {
            let mut j = i;
            while j < cs.len() && part(cs[j]) {
                j += 1;
            }
            let run: String = cs[i..j].iter().collect();
            // A full stop or a "/" at the end is not part of the name.
            let path = run.trim_end_matches(['.', '/']);
            if path.matches('/').count() >= 2 {
                out.push_str(path.rsplit('/').next().unwrap_or(""));
                out.push_str(&run[path.len()..]);
                i = j;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    out
}

/// Markdown links as their text only: `[notes](file:///a/notes.md)` reads "notes".
fn unlinked(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('[') {
        let after = &rest[i + 1..];
        let link = after.find("](").and_then(|j| after[j + 2..].find(')').map(|k| (j, j + 2 + k)));
        match link {
            Some((j, end)) if !after[..j].contains('[') => {
                out.push_str(&rest[..i]);
                out.push_str(&after[..j]);
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[..i + 1]);
                rest = after;
            }
        }
    }
    out + rest
}

/// The facts of the last turn in `items` (one agent's conversation, oldest first). `agent`: its kind ("claude").
pub fn facts(items: &[ConversationItem], agent: &str) -> Facts {
    let from = items.iter().rposition(|it| matches!(it.kind, ItemKind::UserText(_)));
    let turn = &items[from.map_or(0, |i| i + 1)..];
    let mut f = Facts::default();
    f.id = turn.last().or(from.map(|i| &items[i])).map(|it| it.id.clone());
    f.ended = turn.iter().rev().find_map(|it| rows::ms(it.timestamp));
    let mut files: Vec<String> = vec![];
    // The newest test command after the last edit: (state, name).
    let mut test: Option<(&'static str, String)> = None;
    let mut text: Option<&str> = None;
    for it in turn {
        match &it.kind {
            ItemKind::AssistantText(s) => text = Some(s),
            ItemKind::ToolChips(chips) => {
                let mut calls: Vec<&ToolCall> = chips.iter().flat_map(|c| &c.calls).collect();
                calls.sort_by_key(|c| c.seq);
                for c in calls {
                    if is_edit(&c.tool_name) {
                        for file in edit_files(c) {
                            if !files.contains(&file) {
                                files.push(file);
                            }
                        }
                        test = None;
                    } else if is_command(&c.tool_name) && c.output.is_some() {
                        f.commands += 1;
                        let bad = failed(c, agent);
                        f.failed = bad.then(|| rows::cut(c.summary.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or(""), COMMAND_CAP));
                        if let Some((name, piped)) = test_run(&c.summary) {
                            test = Some((if bad { "failed" } else if piped { "ran" } else { "passed" }, name));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    f.edited = files.len();
    let code = files.iter().any(|f| !prose(f));
    match test {
        Some((state, name)) => {
            f.tests = Some(state.into());
            f.test = Some(name);
        }
        None if code => f.tests = Some("none".into()),
        None => {}
    }
    f.said = text.and_then(first_sentence);
    f
}

/// What `glance` kept for one terminal: the line it printed for a log of this size and time.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Kept {
    path: String,
    size: u64,
    /// The log's change time, milliseconds since 1970.
    modified: i64,
    line: Value,
    #[serde(default)]
    v: u32,
}

fn line(terminal: &str, agent: &str, path: Option<&str>, facts: &Facts) -> Value {
    let mut v = serde_json::to_value(facts).unwrap_or_else(|_| json!({}));
    v["type"] = json!("glance");
    v["terminal"] = json!(terminal);
    v["agent"] = json!(agent);
    v["path"] = json!(path);
    v
}

/// One `glance` line for every agent pane on this machine, in herdr-core's order. `cache`: the sidecar's cache file
/// (where each agent's log is); the lines are kept beside it in `glance.json` and reused while a log is unchanged.
pub fn all(ws: &[Workspace], resolver: &mut SessionResolver, cache: Option<&Path>, now_s: f64) -> Vec<Value> {
    let kept_at = cache.map(|c| c.with_file_name("glance.json"));
    let kept: BTreeMap<String, Kept> = kept_at.as_deref().and_then(|p| std::fs::read(p).ok()).and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default();
    let mut next: BTreeMap<String, Kept> = BTreeMap::new();
    let mut out = vec![];
    for found in locate::all(ws, resolver, cache, now_s) {
        let agent = found.pane.agent.as_ref().map(|a| a.kind.clone()).unwrap_or_else(|| "agent".into());
        let tid = found.pane.terminal_id.clone();
        let Some(path) = found.path else {
            out.push(line(&tid, &agent, None, &Facts::default()));
            continue;
        };
        let meta = std::fs::metadata(&path).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
        let l = match kept.get(&tid).filter(|k| k.v == KEPT_VERSION && k.path == path && k.size == size && k.modified == modified && k.line["agent"] == json!(agent)) {
            Some(k) => k.line.clone(),
            None => {
                let f = tail::read(Path::new(&path), None, tail::BYTES).map(|r| facts(&tail::items(&r.lines), &agent)).unwrap_or_default();
                line(&tid, &agent, Some(&path), &f)
            }
        };
        next.insert(tid, Kept { path, size, modified, line: l.clone(), v: KEPT_VERSION });
        out.push(l);
    }
    if next != kept {
        locate::write_json(&next, kept_at.as_deref());
    }
    out
}
