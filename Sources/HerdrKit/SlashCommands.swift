import Foundation

/// The "/" commands of an agent's terminal, read from files (no key goes to the agent). The composer shows them above
/// the reply box when the text starts with "/". Rust: `xplat/flockbook-modfeed/src/commands.rs` and `models.rs` (the
/// Herdr app mod's helper): the same tables, sources and rules; `Tests/HerdrKitTests/Fixtures/slash-tables.json`
/// holds the tables for both sides' tests.
///
/// Per agent kind, as each agent finds them:
/// - Claude Code: its common built-ins; custom commands, `.claude/commands/**/*.md` in the terminal's folder, then in
///   the user's home; skills, `.claude/skills/<name>/SKILL.md` there and in the home; plugins
///   (`~/.claude/plugins/installed_plugins.json`, not turned off in `enabledPlugins`) as `<plugin>:<name>`.
/// - Codex: a short set of its built-ins. Its skills are not "/" commands.
/// - pi: its built-ins; prompt templates, `~/.pi/agent/prompts/*.md` (top level); skills as `skill:<name>` from
///   `~/.pi/agent/skills`, `~/.agents/skills`, the project's `.pi/skills`, and `.agents/skills` from the folder up to
///   the repository root (off with `enableSkillCommands`).
/// - agy (Antigravity CLI): its built-ins (one row a command, other names in the description); skills from
///   `~/.gemini/skills` and `~/.gemini/antigravity/skills`.
/// - Any other agent: nothing.
///
/// A built-in carries an argument hint, fixed argument values where the agent has them (/model, /effort, /thinking),
/// and a mark when, sent bare, it opens a screen in the terminal. A screen built-in sent bare ("/model") is not sent
/// (`opensScreen`): the chat cannot show the screen. One that takes a value stays in the list ("/model sonnet" goes
/// out); one that takes none (/status, /config) can never be sent from the chat, so `list` leaves it out and
/// `screens` names it. A name seen once is not listed again (the first source wins).
public enum SlashCommands {
    /// A value offered after "/name ", with a label for people ("Opus 5.5"); no label: the value says it all.
    public struct Arg: Hashable, Sendable {
        public var value: String
        public var label: String
        public init(_ value: String, _ label: String = "") { self.value = value; self.label = label }
    }

    /// A built-in: name, description (as the agent's own menu says it), argument hint, argument values, and whether
    /// sent bare it opens a screen in the terminal (a picker, a dialog).
    public struct Builtin: Hashable, Sendable {
        public var name: String, description: String, hint: String
        public var args: [String]
        public var screen: Bool
    }

    public struct Command: Hashable, Sendable {
        public var name: String
        public var description: String
        /// "builtin", "project", "user", "skill", or "plugin".
        public var source: String
        /// What may follow the name ("<model>", "[file]"), or "".
        public var hint: String
        /// Built-ins: the values offered after "/name " (/model, /effort), each with a label where one helps.
        public var args: [Arg]
        /// Built-ins: sent bare, it opens a screen in the terminal, so it needs a value to be sent.
        public var screen: Bool
        public init(name: String, description: String = "", source: String = "builtin", hint: String = "", args: [Arg] = [], screen: Bool = false) {
            self.name = name; self.description = description; self.source = source; self.hint = hint; self.args = args; self.screen = screen
        }
    }

    /// What the composer needs for one agent: its commands, and the screen built-ins left out of them.
    public struct Listing: Hashable, Sendable {
        public var commands: [Command]
        public var screens: [String]
        public init(commands: [Command] = [], screens: [String] = []) { self.commands = commands; self.screens = screens }
    }

    private static func b(_ name: String, _ description: String, _ hint: String, _ args: [String], _ screen: Bool) -> Builtin {
        Builtin(name: name, description: description, hint: hint, args: args, screen: screen)
    }

    static let efforts = ["low", "medium", "high", "xhigh", "max"]
    static let piThinking = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]

    // The tables: Claude Code 2.1, Codex 0.167 (a short set of its about 60), pi 1.3, agy 1.5 (as commands.rs), then
    // the /model values of Claude Code 2.1.322 (full ids first, then the aliases) and agy 1.5.3 (as models.rs).
    static let claude: [Builtin] = [
        b("add-dir", "Add a new working directory", "<path>", [], true),
        b("clear", "Start a new conversation (clears the history)", "", [], false),
        b("compact", "Summarise the conversation so far, to free context", "[instructions]", [], false),
        b("config", "Open the settings", "", [], true),
        b("context", "Show how full the context is", "", [], false),
        b("effort", "Set effort level for model usage", "low|medium|high|xhigh|max", efforts, true),
        b("export", "Export the current conversation to a file or clipboard", "[file]", [], true),
        b("help", "Show help and the keys", "", [], true),
        b("hooks", "Show the hooks", "", [], true),
        b("init", "Initialize a new CLAUDE.md file with codebase documentation", "", [], false),
        b("mcp", "Manage MCP servers", "", [], true),
        b("memory", "Edit the memory and instruction files", "", [], true),
        b("model", "Set the AI model for Claude Code", "<model>", [], true),
        b("permissions", "Show and edit the tool permission rules", "", [], true),
        b("resume", "Resume an earlier conversation", "[session]", [], true),
        b("review", "Review the current diff or a PR for bugs", "", [], false),
        b("rewind", "Restore the code and/or conversation to a previous point", "", [], true),
        b("status", "Show the version, model, account and connections", "", [], true),
        b("usage", "Show session cost, plan usage, and activity stats", "", [], true),
        b("plugin", "Manage plugins", "", [], true),
    ]
    static let codex: [Builtin] = [
        b("clear", "Clear the terminal and start a new chat", "", [], false),
        b("compact", "Summarize conversation to prevent hitting the context limit", "", [], false),
        b("config", "Configure Codex preferences", "", [], true),
        b("copy", "Copy the last response or part of it", "", [], false),
        b("diff", "Show git diff (including untracked files)", "", [], false),
        b("export", "Export the conversation as markdown", "", [], false),
        b("fork", "Fork the current chat", "", [], false),
        b("goal", "Set or view the goal for a long-running task", "[goal]", [], false),
        b("init", "Create an AGENTS.md file with instructions for Codex", "", [], false),
        b("mcp", "List MCP tools", "[verbose]", [], false),
        b("model", "Choose what model and reasoning effort to use", "", [], true),
        b("new", "Start a new chat during a conversation", "", [], false),
        b("permissions", "Choose what Codex is allowed to do", "", [], true),
        b("plan", "Switch to Plan mode", "", [], false),
        b("rename", "Rename the current thread", "<name>", [], false),
        b("resume", "Resume a saved chat", "", [], true),
        b("review", "Review my current changes and find issues", "", [], false),
        b("status", "Show current session configuration and token usage", "", [], false),
        b("usage", "View account usage or use a usage limit reset", "", [], false),
        b("skills", "Use skills to improve how Codex performs specific tasks", "", [], true),
    ]
    static let pi: [Builtin] = [
        b("changelog", "Show changelog entries", "", [], false),
        b("clone", "Duplicate the current session at the current position", "", [], false),
        b("compact", "Compact the session context, optionally with instructions", "[instructions]", [], false),
        b("copy", "Copy the last agent message to the clipboard", "", [], false),
        b("export", "Export the session as HTML or JSONL", "[path]", [], false),
        b("fork", "Create a new fork from a previous user message", "", [], true),
        b("hotkeys", "Show all keyboard shortcuts", "", [], false),
        b("model", "Select a model", "[provider/model]", [], true),
        b("name", "Set the session display name", "[name]", [], false),
        b("new", "Start a new session", "", [], false),
        b("reload", "Reload keybindings, extensions, skills, prompts, themes and context files", "", [], false),
        b("resume", "Resume a different session", "", [], true),
        b("scoped-models", "Enable or disable models for Ctrl+P cycling", "", [], true),
        b("session", "Show session info and stats", "", [], false),
        b("settings", "Open the settings menu", "", [], true),
        b("thinking", "Set the thinking level", "[level]", piThinking, true),
        b("tree", "Navigate the session tree", "", [], true),
    ]
    static let agy: [Builtin] = [
        b("add-dir", "Add a directory to the workspace", "<path>", [], false),
        b("agents", "List available custom agents", "", [], false),
        b("artifact", "View and review artifacts", "", [], true),
        b("btw", "Ask a side question without interrupting the current task", "<question>", [], false),
        b("changelog", "Show release notes and changes", "", [], false),
        b("clear", "Clear conversation and start a new one (also /new)", "", [], false),
        b("codesearch", "Search code in the workspace (also /cs)", "<query>", [], false),
        b("compact", "Compact the conversation now", "", [], false),
        b("config", "Open settings panel (also /settings)", "", [], true),
        b("context", "Visualize current context usage", "", [], true),
        b("copy", "Copy the last planner response to the clipboard", "[n]", [], false),
        b("credits", "Show remaining credits", "", [], false),
        b("diff", "View uncommitted changes and per-turn diffs", "", [], true),
        b("effort", "Set the reasoning effort", "[level]", efforts, true),
        b("fork", "Create a branch of the conversation at this point (also /branch)", "[name]", [], false),
        b("goal", "Run until the specified goal is completely finished", "<goal>", [], false),
        b("help", "Show available commands and keybindings", "", [], true),
        b("hooks", "Manage hook configurations for tool events", "", [], true),
        b("mcp", "Manage MCP servers", "", [], true),
        b("model", "Set a model, or run a single prompt on another model", "[model]", [], true),
        b("permissions", "Manage tool permissions", "", [], true),
        b("plan", "Plan carefully before executing a task", "[task]", [], false),
        b("plugin", "Manage plugins", "", [], true),
        b("rename", "Rename the current conversation", "<name>", [], false),
        b("resume", "Browse and resume past conversations (also /switch)", "", [], true),
        b("rewind", "Rewind conversation to a previous message (also /undo)", "", [], true),
        b("skills", "List available skills", "", [], false),
        b("usage", "View model quota usage (also /quota)", "", [], true),
    ]
    static let claudeModels: [(String, String)] = [
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
    ]
    static let agyModels: [(String, String)] = [
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
    ]

    /// A folder with nothing in it, for an agent whose files are out of reach (on another machine): the lists then
    /// hold what the tables give (Claude Code's and agy's models; no Codex or pi model, no custom command).
    public static let noFiles = URL(fileURLWithPath: "/var/empty")

    /// The built-ins of an agent of `kind` (none for an unknown kind).
    public static func builtins(_ kind: String) -> [Builtin] {
        switch kind {
        case "claude": claude
        case "codex": codex
        case "pi": pi
        case "agy": agy
        default: []
        }
    }

    /// A built-in with its values for this user: `/model`'s come from the agent's files (`modelValues`); a value list
    /// with no hint gets "<model>".
    private struct Resolved {
        var b: Builtin, hint: String, args: [Arg]
        /// It takes a value after its name (a hint or values).
        var takesValue: Bool { !hint.isEmpty || !args.isEmpty }
    }

    private static func resolve(_ kind: String, home: URL?) -> [Resolved] {
        builtins(kind).map { b in
            var args = b.args.map { Arg($0) }
            if b.name == "model" { args = home.map { modelValues(kind, home: $0) } ?? [] }
            return Resolved(b: b, hint: b.hint.isEmpty && !args.isEmpty ? "<model>" : b.hint, args: args)
        }
    }

    /// The screen built-ins of `kind` that take no value: never sent from the chat, so not in `list`. Sorted by name.
    public static func screens(_ kind: String, home: URL?) -> [String] {
        resolve(kind, home: home).filter { $0.b.screen && !$0.takesValue }.map(\.b.name).sorted(by: bytes)
    }

    /// `text` is a screen built-in of `kind` sent bare ("/model", spaces around it trimmed): why it is not sent, for
    /// the person. Anything after the name ("/model sonnet"), or any other text: nil.
    public static func opensScreen(_ kind: String, text: String, home: URL?) -> String? {
        let t = trim(text)
        guard t.hasPrefix("/") else { return nil }
        let name = String(t.dropFirst())
        guard let r = resolve(kind, home: home).first(where: { $0.b.screen && $0.b.name == name }) else { return nil }
        return screenNote(name: r.b.name, example: r.args.first?.value ?? r.hint, takesValue: r.takesValue)
    }

    /// The note for a bare screen command (the mod and the helper build the same text).
    public static func screenNote(name: String, example: String, takesValue: Bool) -> String {
        takesValue ? "/\(name) opens a menu in the terminal. Add a value, like /\(name) \(example)."
                   : "/\(name) opens a screen in the terminal. Use it there."
    }

    /// At most this many entries, and description characters.
    static let max = 500, desc = 120
    /// A file larger than this is not read for its description (its name is still listed).
    static let readMax = 256 * 1024

    /// The commands and left-out screens of an agent of `kind` in `cwd`, with the user's home `home`.
    public static func listing(_ kind: String, home: URL, cwd: URL) -> Listing {
        Listing(commands: list(kind, home: home, cwd: cwd), screens: screens(kind, home: home))
    }

    /// The commands of an agent of `kind` in `cwd`, with the user's home `home`. Sorted by name. Unknown kinds: none.
    public static func list(_ kind: String, home: URL, cwd: URL) -> [Command] {
        var l = Lister()
        switch kind {
        case "claude":
            l.builtins(kind, home)
            let claude = home.appendingPathComponent(".claude")
            for (dir, source) in [(cwd.appendingPathComponent(".claude/commands"), "project"), (claude.appendingPathComponent("commands"), "user")] {
                for c in commandsIn(dir, depthMax: 3) { l.add(c.name, c.desc, source, c.hint) }
            }
            for dir in [cwd.appendingPathComponent(".claude/skills"), claude.appendingPathComponent("skills")] {
                for s in skillsIn(dir, depth: 0) { l.add(s.name, s.desc, "skill", "") }
            }
            for (plugin, path) in plugins(claude: claude, cwd: cwd) {
                for c in commandsIn(path.appendingPathComponent("commands"), depthMax: 3) { l.add("\(plugin):\(c.name)", c.desc, "plugin", c.hint) }
                for s in skillsIn(path.appendingPathComponent("skills"), depth: 0) { l.add("\(plugin):\(s.name)", s.desc, "plugin", "") }
            }
        case "codex": l.builtins(kind, home)
        case "pi":
            l.builtins(kind, home)
            let agent = home.appendingPathComponent(".pi/agent")
            for c in commandsIn(agent.appendingPathComponent("prompts"), depthMax: 0) { l.add(c.name, c.desc, "user", c.hint) }
            if piSkillCommands(agent) {
                var dirs = [cwd.appendingPathComponent(".pi/skills")]
                dirs += upToRepo(cwd).map { $0.appendingPathComponent(".agents/skills") }
                dirs += [agent.appendingPathComponent("skills"), home.appendingPathComponent(".agents/skills")]
                for d in dirs {
                    for s in skillsIn(d, depth: 3) { l.add("skill:\(s.name)", s.desc, "skill", "") }
                }
            }
        case "agy":
            l.builtins(kind, home)
            for d in [home.appendingPathComponent(".gemini/skills"), home.appendingPathComponent(".gemini/antigravity/skills")] {
                for s in skillsIn(d, depth: 0) { l.add(s.name, s.desc, "skill", "") }
            }
        default: break
        }
        return l.out.sorted { bytes($0.name, $1.name) }
    }

    private struct Lister {
        var out: [Command] = []
        var seen: Set<String> = []

        /// The entry's index in `out`, or nil when it was not added (full, a bad name, or a name seen before).
        @discardableResult
        mutating func add(_ name: String, _ description: String, _ source: String, _ hint: String) -> Int? {
            guard out.count < SlashCommands.max, valid(name), seen.insert(name).inserted else { return nil }
            out.append(Command(name: name, description: cut(description), source: source, hint: cut(hint)))
            return out.count - 1
        }

        mutating func builtins(_ kind: String, _ home: URL) {
            for r in resolve(kind, home: home) {
                // A screen with no value can never be sent from the chat: left out, and its name stays the built-in's.
                if r.b.screen && !r.takesValue { seen.insert(r.b.name); continue }
                if let i = add(r.b.name, r.b.description, "builtin", r.hint) { out[i].args = r.args; out[i].screen = r.b.screen }
            }
        }
    }

    /// pi's `enableSkillCommands` (default on), from its settings.
    private static func piSkillCommands(_ agent: URL) -> Bool {
        bool(object(json(agent.appendingPathComponent("settings.json"), max: .max))?["enableSkillCommands"]) ?? true
    }

    /// `cwd` and its parents, up to the first one with a `.git` (the repository root), or only `cwd` outside one.
    private static func upToRepo(_ cwd: URL) -> [URL] {
        var out: [URL] = [], d = cwd.standardizedFileURL
        for _ in 0..<20 {
            out.append(d)
            if FileManager.default.fileExists(atPath: d.appendingPathComponent(".git").path) { return out }
            let up = d.deletingLastPathComponent()
            if up.path == d.path { break }
            d = up
        }
        return [cwd]
    }

    /// A name Claude Code takes after "/": letters, digits and `-_.:`, not empty, at most 100 characters.
    static func valid(_ name: String) -> Bool {
        !name.isEmpty && name.utf8.count <= 100 && name.unicodeScalars.allSatisfy { c in
            c.isASCII && (c.properties.isAlphabetic || ("0"..."9").contains(c) || "-_.:".unicodeScalars.contains(c))
        }
    }

    static func cut(_ s: String) -> String {
        let s = trim(s)
        guard s.unicodeScalars.count > desc else { return s }
        var head = String.UnicodeScalarView(s.unicodeScalars.prefix(desc - 1))
        while let last = head.last, last.properties.isWhitespace { head.removeLast() }
        return String(head) + "…"
    }

    /// `<dir>/*.md`, and the `.md` files in its subfolders down to `depthMax` (the folder is not part of the name, as
    /// in Claude Code): name, description, argument hint.
    private static func commandsIn(_ dir: URL, depthMax: Int) -> [(name: String, desc: String, hint: String)] {
        var out: [(name: String, desc: String, hint: String)] = []
        var walk = [(dir, 0)]
        while let (d, depth) = walk.popLast() {
            for p in entries(d) {
                if isDir(p) {
                    if depth < depthMax { walk.append((p, depth + 1)) }
                } else if p.pathExtension == "md" {
                    let stem = p.deletingPathExtension().lastPathComponent
                    let (front, body) = readMD(p)
                    out.append((stem, field(front, "description") ?? firstLine(body), field(front, "argument-hint") ?? ""))
                }
            }
        }
        return out
    }

    /// `<dir>/<name>/SKILL.md`, and down to `depth` more folders (a folder with a SKILL.md is not looked into): the
    /// name in its front matter, else the folder's. A skill with `user-invocable: false` is not offered after "/", so
    /// it is left out.
    private static func skillsIn(_ dir: URL, depth: Int) -> [(name: String, desc: String)] {
        var out: [(name: String, desc: String)] = []
        for d in entries(dir) where isDir(d) {
            let f = d.appendingPathComponent("SKILL.md")
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: f.path, isDirectory: &isDirectory), !isDirectory.boolValue else {
                if depth > 0 { out += skillsIn(d, depth: depth - 1) }
                continue
            }
            let (front, body) = readMD(f)
            if field(front, "user-invocable") == "false" { continue }
            out.append((field(front, "name") ?? d.lastPathComponent, field(front, "description") ?? firstLine(body)))
        }
        return out
    }

    /// Installed plugins that are on: (name, folder). Turned off with `"name@market": false` in `enabledPlugins` of
    /// the user's or the project's settings. A plugin installed for a project counts in that project only.
    private static func plugins(claude: URL, cwd: URL) -> [(String, URL)] {
        guard let installed = object(json(claude.appendingPathComponent("plugins/installed_plugins.json"), max: .max)) else { return [] }
        var off: Set<String> = []
        for s in [claude.appendingPathComponent("settings.json"), cwd.appendingPathComponent(".claude/settings.json"), cwd.appendingPathComponent(".claude/settings.local.json")] {
            guard let m = object(object(json(s, max: .max))?["enabledPlugins"]) else { continue }
            for (k, v) in m { if bool(v) == false { off.insert(k) } else { off.remove(k) } }
        }
        guard let map = object(installed["plugins"]) else { return [] }
        var out: [(String, URL)] = []
        for key in map.keys.sorted(by: bytes) where !off.contains(key) {
            let name = key.split(separator: "@", omittingEmptySubsequences: false).first.map(String.init) ?? key
            let here = (map[key] as? [Any] ?? []).compactMap(object).first { e in
                (e["scope"] as? String ?? "user") == "user" || (e["projectPath"] as? String).map { parts($0) == parts(cwd.path) } ?? false
            }
            if let path = here?["installPath"] as? String { out.append((name, URL(fileURLWithPath: path))) }
        }
        return out
    }

    /// A Markdown file's front matter lines (between the first two `---` lines) and its body.
    private static func readMD(_ p: URL) -> (front: [String], body: String) {
        guard let size = (try? FileManager.default.attributesOfItem(atPath: p.path))?[.size] as? Int, size <= readMax,
              let data = try? Data(contentsOf: p), let text = String(data: data, encoding: .utf8) else { return ([], "") }
        let all = lines(text)
        guard all.first.map(trim) == "---" else { return ([], text) }
        guard let end = all.dropFirst().firstIndex(where: { trim($0) == "---" }) else { return ([], text) }
        return (Array(all[1..<end]), all[(end + 1)...].joined(separator: "\n"))
    }

    /// `key: value` in front matter, its quotes taken off.
    private static func field(_ front: [String], _ key: String) -> String? {
        for l in front {
            guard let colon = l.firstIndex(of: ":"), trim(String(l[..<colon])) == key else { continue }
            var v = Substring(trim(String(l[l.index(after: colon)...])))
            while let c = v.first, c == "\"" || c == "'" { v.removeFirst() }
            while let c = v.last, c == "\"" || c == "'" { v.removeLast() }
            if !v.isEmpty { return String(v) }
        }
        return nil
    }

    /// The body's first line with text, without a heading's `#`.
    private static func firstLine(_ body: String) -> String {
        for l in lines(body) {
            let t = trim(String(trim(l).drop { $0 == "#" }))
            if !t.isEmpty { return t }
        }
        return ""
    }

    // MARK: /model values

    /// The values `/model <value>` takes for an agent of `kind`, read from the agent's own files where it keeps them
    /// (each switches with the value and no screen). None: the agent's `/model` takes no value known here, so it
    /// stays a screen with no value.
    /// - Claude Code: the full ids of its current models and their aliases, plus the ids in the user's settings
    ///   (`modelSettings`, `availableModels`) and the extra options it saved (`~/.claude.json`
    ///   `additionalModelOptionsCache`).
    /// - Codex: its model cache (`~/.codex/models_cache.json`, the ones its picker lists, in its order).
    /// - pi: `provider/id` of the models of the providers it has a key for (`auth.json`, or an `apiKey` in
    ///   `models.json`), from `models-store.json` and `models.json`; only `enabledModels` when its settings name some.
    /// - agy: a table (its list comes from its server, not a file).
    public static func modelValues(_ kind: String, home: URL) -> [Arg] {
        var l = Values()
        switch kind {
        case "claude": claudeValues(home, &l)
        case "codex": codexValues(home, &l)
        case "pi": piValues(home, &l)
        case "agy": for (v, n) in agyModels { l.add(v, n) }
        default: break
        }
        return l.out
    }

    /// At most this many values; a file larger than this is not read.
    static let valuesMax = 400, valuesReadMax = 16 * 1024 * 1024

    private struct Values {
        var out: [Arg] = []
        var seen: Set<String> = []
        mutating func add(_ value: String, _ label: String) {
            let value = trim(value)
            guard out.count < valuesMax, !value.isEmpty, !value.unicodeScalars.contains(where: \.properties.isWhitespace),
                  seen.insert(value).inserted else { return }
            out.append(Arg(value, trim(label)))
        }
    }

    /// "claude-opus-5-5" → "Opus 5.5"; "claude-fable-5-1[1m]" → "Fable 5.1 (1M context)". Not a Claude id: "".
    public static func claudeLabel(_ id: String) -> String {
        let wide = id.hasSuffix("[1m]")
        let base = wide ? String(id.dropLast(4)) : id
        guard base.hasPrefix("claude-") else { return "" }
        let split = base.dropFirst(7).split(separator: "-", omittingEmptySubsequences: false).map(String.init)
        guard let family = split.first, !family.isEmpty, family.unicodeScalars.allSatisfy({ $0.isASCII && $0.properties.isAlphabetic }) else { return "" }
        let version = split.dropFirst()
        guard !version.isEmpty, version.allSatisfy({ v in !v.isEmpty && v.utf8.count <= 2 && v.unicodeScalars.allSatisfy { ("0"..."9").contains($0) } }) else { return "" }
        return family.prefix(1).uppercased() + family.dropFirst() + " " + version.joined(separator: ".") + (wide ? " (1M context)" : "")
    }

    private static func claudeValues(_ home: URL, _ l: inout Values) {
        for (v, n) in claudeModels { l.add(v, n) }
        if let s = object(json(home.appendingPathComponent(".claude/settings.json"), max: valuesReadMax)) {
            var ids = (object(s["modelSettings"])?.keys.sorted(by: bytes)) ?? []
            ids += (s["availableModels"] as? [Any] ?? []).compactMap { $0 as? String }
            for id in ids { l.add(id, claudeLabel(id)) }
        }
        let extra = object(json(home.appendingPathComponent(".claude.json"), max: valuesReadMax))?["additionalModelOptionsCache"] as? [Any] ?? []
        for o in extra.compactMap(object) {
            guard let v = o["value"] as? String else { continue }
            // Its description starts with the model's name ("Fable 5.1 · Most capable …").
            let label = (o["description"] as? String).flatMap { $0.components(separatedBy: " · ").first }.flatMap { $0.isEmpty ? nil : $0 }
            l.add(v, label ?? claudeLabel(v))
        }
    }

    private static func codexValues(_ home: URL, _ l: inout Values) {
        guard let c = object(json(home.appendingPathComponent(".codex/models_cache.json"), max: valuesReadMax)) else { return }
        let models = (c["models"] as? [Any] ?? []).compactMap(object).filter { $0["visibility"] as? String == "list" }
        // A stable sort (the cache's order among equals), as the helper's.
        let sorted = models.enumerated().sorted { a, b in
            let (pa, pb) = (int(a.element["priority"]) ?? .max, int(b.element["priority"]) ?? .max)
            return pa != pb ? pa < pb : a.offset < b.offset
        }
        for m in sorted.map(\.element) {
            if let slug = m["slug"] as? String { l.add(slug, m["display_name"] as? String ?? "") }
        }
    }

    private static func piValues(_ home: URL, _ l: inout Values) {
        let agent = home.appendingPathComponent(".pi/agent")
        let custom = object(object(json(agent.appendingPathComponent("models.json"), max: valuesReadMax))?["providers"]) ?? [:]
        let store = object(json(agent.appendingPathComponent("models-store.json"), max: valuesReadMax)) ?? [:]
        // Providers it can use: a key saved by its login, or one in models.json.
        var keyed = Set(object(json(agent.appendingPathComponent("auth.json"), max: valuesReadMax))?.keys.map { $0 } ?? [])
        for (n, p) in custom where !((object(p)?["apiKey"] as? String) ?? "").isEmpty { keyed.insert(n) }
        let enabled = ((object(json(agent.appendingPathComponent("settings.json"), max: valuesReadMax))?["enabledModels"] as? [Any]) ?? [])
            .compactMap { $0 as? String }.map { $0.split(separator: ":", omittingEmptySubsequences: false).first.map(String.init) ?? $0 }
        for p in keyed.sorted(by: bytes) {
            for src in [object(store[p])?["models"], object(custom[p])?["models"]] {
                for m in (src as? [Any] ?? []).compactMap(object) {
                    guard let id = m["id"] as? String else { continue }
                    let value = "\(p)/\(id)"
                    if enabled.isEmpty || enabled.contains(where: { glob($0, value) || glob($0, id) }) { l.add(value, m["name"] as? String ?? "") }
                }
            }
        }
    }

    /// `*` matches any run of characters; nothing else is special.
    static func glob(_ pattern: String, _ s: String) -> Bool {
        let parts = pattern.utf8.split(separator: UInt8(ascii: "*"), omittingEmptySubsequences: false).map(Array.init)
        let s = Array(s.utf8)
        guard parts.count > 1 else { return Array(pattern.utf8) == s }
        let first = parts[0], last = parts[parts.count - 1]
        guard s.count >= first.count + last.count, s.starts(with: first), s.suffix(last.count).elementsEqual(last) else { return false }
        var rest = s[first.count..<(s.count - last.count)]
        for mid in parts[1..<(parts.count - 1)] {
            guard let i = find(mid, in: rest) else { return false }
            rest = rest[(i + mid.count)...]
        }
        return true
    }

    private static func find(_ needle: [UInt8], in hay: ArraySlice<UInt8>) -> Int? {
        if needle.isEmpty { return hay.startIndex }
        guard hay.count >= needle.count else { return nil }
        return (hay.startIndex...(hay.endIndex - needle.count)).first { hay[$0..<($0 + needle.count)].elementsEqual(needle) }
    }

    // MARK: text and file helpers (the helper's rules: Rust's `trim`, `lines`, byte order)

    /// Without Unicode white space at both ends.
    static func trim(_ s: String) -> String {
        var v = Substring.UnicodeScalarView(s.unicodeScalars)
        while let c = v.first, c.properties.isWhitespace { v.removeFirst() }
        while let c = v.last, c.properties.isWhitespace { v.removeLast() }
        return String(v)
    }

    /// Lines split at "\n" (a "\r" before it taken off); no empty line after a last "\n". By bytes: "\r\n" is one
    /// Character in Swift.
    static func lines(_ s: String) -> [String] {
        var out = s.utf8.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: false).map { l -> String in
            String(decoding: l.last == UInt8(ascii: "\r") ? l.dropLast() : l[...], as: UTF8.self)
        }
        if s.utf8.last == UInt8(ascii: "\n") || s.isEmpty { out.removeLast() }
        return out
    }

    /// Byte order (names are compared as the helper compares them).
    static func bytes(_ a: String, _ b: String) -> Bool { a.utf8.lexicographicallyPrecedes(b.utf8) }

    /// A path's parts, without "." and empty ones: two spellings of one folder compare equal.
    private static func parts(_ path: String) -> [String] {
        (path.hasPrefix("/") ? ["/"] : []) + path.split(separator: "/").map(String.init).filter { $0 != "." }
    }

    /// A folder's entries, sorted by name. A folder that cannot be read: none.
    private static func entries(_ dir: URL) -> [URL] {
        ((try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []).sorted(by: bytes).map { dir.appendingPathComponent($0) }
    }

    private static func isDir(_ p: URL) -> Bool {
        var d: ObjCBool = false
        return FileManager.default.fileExists(atPath: p.path, isDirectory: &d) && d.boolValue
    }

    private static func json(_ p: URL, max: Int) -> Any? {
        guard let size = (try? FileManager.default.attributesOfItem(atPath: p.path))?[.size] as? Int, size <= max,
              let data = try? Data(contentsOf: p) else { return nil }
        return try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
    }

    private static func object(_ v: Any?) -> [String: Any]? { v as? [String: Any] }

    /// A JSON true or false (a number is not one).
    private static func bool(_ v: Any?) -> Bool? {
        guard let n = v as? NSNumber, CFGetTypeID(n) == CFBooleanGetTypeID() else { return nil }
        return n.boolValue
    }

    /// A JSON whole number (not a boolean, not a fraction).
    private static func int(_ v: Any?) -> Int? {
        guard let n = v as? NSNumber, CFGetTypeID(n) != CFBooleanGetTypeID(), !CFNumberIsFloatType(n) else { return nil }
        return n.intValue
    }
}
