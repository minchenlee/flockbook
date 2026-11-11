import Foundation
@testable import HerdrKit

// The "/" command list (SlashCommands, SlashList) and the agent's input box (AgentBox). The same cases as the
// helper's tests (xplat/flockbook-modfeed: commands.rs, models.rs, queue.rs box_tests) and the mod's
// (herdr-app/test/composer.test.mjs).

/// A fresh folder for one check, removed after `body`.
private func scratch(_ body: (URL) throws -> Void) rethrows {
    let root = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hkslash-\(UUID().uuidString)")
    try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    try body(root)
}

private func put(_ p: URL, _ s: String) {
    try? FileManager.default.createDirectory(at: p.deletingLastPathComponent(), withIntermediateDirectories: true)
    try? Data(s.utf8).write(to: p)
}

private typealias Arg = SlashCommands.Arg
private func vals(_ a: [Arg]) -> [String] { a.map(\.value) }

func checksSlashTables() throws {
    // The tables are the helper's: both sides compare theirs with this file.
    let o = try JSONSerialization.jsonObject(with: fixture("slash-tables.json")) as! [String: Any]
    check(o["efforts"] as? [String] == SlashCommands.efforts && o["pi_thinking"] as? [String] == SlashCommands.piThinking, "slash tables: effort and thinking values")
    let built = o["builtins"] as! [String: [[String: Any]]]
    for kind in ["claude", "codex", "pi", "agy"] {
        let want = (built[kind] ?? []).map { b in
            SlashCommands.Builtin(name: b["name"] as! String, description: b["description"] as! String, hint: b["hint"] as! String,
                                  args: b["args"] as! [String], screen: b["screen"] as! Bool)
        }
        check(!want.isEmpty && SlashCommands.builtins(kind) == want, "slash tables: \(kind) built-ins as the fixture")
    }
    check(Set(built.keys) == ["claude", "codex", "pi", "agy"] && SlashCommands.builtins("cursor-agent").isEmpty, "slash tables: four kinds, no other")
    let models = o["models"] as! [String: [[String]]]
    check(models["claude"] == SlashCommands.claudeModels.map { [$0.0, $0.1] } && models["agy"] == SlashCommands.agyModels.map { [$0.0, $0.1] }, "slash tables: model values as the fixture")
}

func checksSlashCommands() throws {
    try scratch { t in
        let v = SlashCommands.list("claude", home: t.appendingPathComponent("home"), cwd: t.appendingPathComponent("proj"))
        check(v.map(\.name) == ["add-dir", "clear", "compact", "context", "effort", "export", "init", "model", "resume", "review"],
              "slash: Claude built-ins without files; screens with no value are left out")
        check(v.allSatisfy { $0.source == "builtin" }, "slash: all built-in")
        func c(_ n: String) -> SlashCommands.Command { v.first { $0.name == n }! }
        check(Array(vals(c("model").args).prefix(2)) == ["claude-fable-5-2", "claude-opus-5-5"] && c("model").args[1].label == "Opus 5.5", "slash: /model full ids first, with labels")
        check(vals(c("model").args).contains("opus") && vals(c("effort").args) == ["low", "medium", "high", "xhigh", "max"], "slash: aliases; /effort values")
        check(c("model").hint == "<model>" && c("add-dir").hint == "<path>", "slash: hints")
        check(c("model").screen && c("resume").screen && !c("clear").screen && !c("context").screen, "slash: screen marks")
    }

    try scratch { t in
        let home = t.appendingPathComponent("home"), proj = t.appendingPathComponent("proj")
        put(proj.appendingPathComponent(".claude/commands/deploy.md"), "---\ndescription: \"Deploy the app\"\n---\nRun the deploy.")
        put(proj.appendingPathComponent(".claude/commands/ops/restart.md"), "# Restart the server\n\nThen check.")
        put(home.appendingPathComponent(".claude/commands/deploy.md"), "---\ndescription: the user's deploy\n---\n")
        put(home.appendingPathComponent(".claude/commands/lint.md"), "Lint the diff.")
        put(home.appendingPathComponent(".claude/commands/bad name.md"), "a name with a space")
        put(home.appendingPathComponent(".claude/commands/crlf.md"), "---\r\ndescription: Windows lines\r\n---\r\nbody\r\n")
        put(home.appendingPathComponent(".claude/skills/notes/SKILL.md"), "---\nname: take-notes\ndescription: Write notes\n---\nbody")
        put(home.appendingPathComponent(".claude/skills/hidden/SKILL.md"), "---\ndescription: model only\nuser-invocable: false\n---\n")
        put(home.appendingPathComponent(".claude/skills/empty-dir/README.md"), "not a skill")
        let plug = t.appendingPathComponent("plug"), off = t.appendingPathComponent("off"), other = t.appendingPathComponent("other")
        put(plug.appendingPathComponent("commands/hello.md"), "---\ndescription: Say hello\n---\n")
        put(plug.appendingPathComponent("skills/tidy/SKILL.md"), "---\nname: tidy\ndescription: Tidy up\n---\n")
        put(off.appendingPathComponent("commands/nope.md"), "x")
        put(other.appendingPathComponent("commands/elsewhere.md"), "x")
        let installed: [String: Any] = ["version": 2, "plugins": [
            "kit@market": [["scope": "user", "installPath": plug.path]],
            "off@market": [["scope": "user", "installPath": off.path]],
            "proj@market": [["scope": "project", "projectPath": "/somewhere/else", "installPath": other.path]],
        ]]
        put(home.appendingPathComponent(".claude/plugins/installed_plugins.json"), String(decoding: try JSONSerialization.data(withJSONObject: installed), as: UTF8.self))
        put(home.appendingPathComponent(".claude/settings.json"), #"{"enabledPlugins": {"off@market": false, "kit@market": true}}"#)
        let v = SlashCommands.list("claude", home: home, cwd: proj)
        let own = v.filter { $0.source != "builtin" }.map { "\($0.name) \($0.source)" }
        check(own == ["crlf user", "deploy project", "kit:hello plugin", "kit:tidy plugin", "lint user", "restart project", "take-notes skill"],
              "slash: custom commands, skills and plugins \(own)")
        func d(_ n: String) -> String { v.first { $0.name == n }?.description ?? "?" }
        check(d("deploy") == "Deploy the app", "slash: the project's command wins over the user's, quotes off")
        check(d("restart") == "Restart the server", "slash: no front matter: the first line, without its #")
        check(d("lint") == "Lint the diff." && d("kit:hello") == "Say hello" && d("crlf") == "Windows lines", "slash: descriptions")
        // A plugin installed for this project counts here.
        let mine: [String: Any] = ["plugins": ["proj@market": [["scope": "project", "projectPath": proj.path + "/.", "installPath": other.path]]]]
        put(home.appendingPathComponent(".claude/plugins/installed_plugins.json"), String(decoding: try JSONSerialization.data(withJSONObject: mine), as: UTF8.self))
        check(SlashCommands.list("claude", home: home, cwd: proj).contains { $0.name == "proj:elsewhere" }, "slash: a project's plugin in that project")
        put(proj.appendingPathComponent(".claude/settings.local.json"), #"{"enabledPlugins": {"proj@market": false}}"#)
        check(!SlashCommands.list("claude", home: home, cwd: proj).contains { $0.name == "proj:elsewhere" }, "slash: turned off in the project's settings")
    }

    try scratch { t in
        let home = t.appendingPathComponent("home")
        put(home.appendingPathComponent(".claude/commands/long.md"), String(repeating: "word ", count: 100))
        put(home.appendingPathComponent(".claude/commands/status.md"), "My own status")
        let v = SlashCommands.list("claude", home: home, cwd: t.appendingPathComponent("proj"))
        let d = v.first { $0.name == "long" }?.description ?? ""
        check(d.unicodeScalars.count <= SlashCommands.desc && d.hasSuffix("…") && !d.hasSuffix(" …"), "slash: long descriptions are cut")
        check(!v.contains { $0.name == "status" }, "slash: a left-out screen keeps its name (the built-in runs, not the file)")
        check(SlashCommands.list("cursor-agent", home: home, cwd: t).isEmpty && SlashCommands.list("agent", home: home, cwd: t).isEmpty, "slash: other agents get nothing")
    }

    try scratch { t in
        let home = t.appendingPathComponent("home"), proj = t.appendingPathComponent("proj")
        put(home.appendingPathComponent(".codex/prompts/mine.md"), "a prompt")
        put(home.appendingPathComponent(".codex/skills/s/SKILL.md"), "---\nname: s\n---\n")
        put(home.appendingPathComponent(".claude/commands/claude-only.md"), "x")
        var v = SlashCommands.list("codex", home: home, cwd: proj)
        check(v.allSatisfy { $0.source == "builtin" }, "slash codex: no prompts, no skills, nothing of Claude's")
        check(v.count == SlashCommands.codex.count - SlashCommands.screens("codex", home: home).count, "slash codex: built-ins less the screens")
        check(v.first { $0.name == "status" }?.screen == false && v.first { $0.name == "rename" }?.hint == "<name>", "slash codex: /status prints in the chat; hints")
        check(SlashCommands.screens("codex", home: home) == ["config", "model", "permissions", "resume", "skills"], "slash codex: no model cache: /model takes no value")
        put(home.appendingPathComponent(".codex/models_cache.json"), #"{"models": [{"slug": "gpt-x", "display_name": "GPT-X", "visibility": "list", "priority": 1}]}"#)
        v = SlashCommands.list("codex", home: home, cwd: proj)
        let m = v.first { $0.name == "model" }
        check(m?.hint == "<model>" && m?.screen == true && m?.args == [Arg("gpt-x", "GPT-X")], "slash codex: with the cache, /model is listed with its values")
        check(!SlashCommands.screens("codex", home: home).contains("model"), "slash codex: then /model is not a left-out screen")
    }

    try scratch { t in
        let home = t.appendingPathComponent("home"), repo = t.appendingPathComponent("repo")
        let proj = repo.appendingPathComponent("app/web")
        try FileManager.default.createDirectory(at: repo.appendingPathComponent(".git"), withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: proj, withIntermediateDirectories: true)
        put(home.appendingPathComponent(".pi/agent/prompts/review.md"), "---\ndescription: Review staged changes\nargument-hint: \"[focus]\"\n---\nReview.")
        put(home.appendingPathComponent(".pi/agent/prompts/nested/deep.md"), "not loaded: top level only")
        put(proj.appendingPathComponent(".pi/prompts/project.md"), "not loaded: needs the project's trust")
        put(home.appendingPathComponent(".pi/agent/skills/notes/SKILL.md"), "---\nname: notes\ndescription: Take notes\n---\n")
        put(home.appendingPathComponent(".pi/agent/skills/group/inner/SKILL.md"), "---\nname: inner\n---\nFound in a subfolder")
        put(home.appendingPathComponent(".agents/skills/shared/SKILL.md"), "---\nname: shared\n---\n")
        put(proj.appendingPathComponent(".pi/skills/local/SKILL.md"), "---\nname: local\n---\n")
        put(repo.appendingPathComponent("app/.agents/skills/mid/SKILL.md"), "---\nname: mid\n---\n")
        put(repo.appendingPathComponent(".agents/skills/top/SKILL.md"), "---\nname: top\n---\n")
        put(t.appendingPathComponent(".agents/skills/outside/SKILL.md"), "---\nname: outside\n---\n")
        let v = SlashCommands.list("pi", home: home, cwd: proj)
        let own = v.filter { $0.source != "builtin" }.map { "\($0.name) \($0.source)" }
        check(own == ["review user", "skill:inner skill", "skill:local skill", "skill:mid skill", "skill:notes skill", "skill:shared skill", "skill:top skill"],
              "slash pi: no nested or project prompts; .agents/skills up to the repository root, not past it \(own)")
        func c(_ n: String) -> SlashCommands.Command { v.first { $0.name == n }! }
        check(c("review").hint == "[focus]" && c("review").description == "Review staged changes", "slash pi: a prompt's hint and description")
        check(vals(c("thinking").args) == ["off", "minimal", "low", "medium", "high", "xhigh", "max"], "slash pi: /thinking values")
        check(c("thinking").screen && c("model").screen && !c("session").screen, "slash pi: screen marks")
        check(SlashCommands.screens("pi", home: home) == ["fork", "resume", "scoped-models", "settings", "tree"], "slash pi: screens")
        put(home.appendingPathComponent(".pi/agent/settings.json"), #"{"enableSkillCommands": false}"#)
        check(SlashCommands.list("pi", home: home, cwd: proj).allSatisfy { !$0.name.hasPrefix("skill:") }, "slash pi: skill commands turned off")
    }

    try scratch { t in
        let home = t.appendingPathComponent("home")
        put(home.appendingPathComponent(".gemini/skills/one/SKILL.md"), "---\nname: one\ndescription: First\n---\n")
        put(home.appendingPathComponent(".gemini/antigravity/skills/two/SKILL.md"), "---\nname: two\n---\nSecond")
        put(home.appendingPathComponent(".gemini/antigravity/skills/one/SKILL.md"), "---\nname: one\n---\nlisted once")
        let v = SlashCommands.list("agy", home: home, cwd: t.appendingPathComponent("proj"))
        check(v.filter { $0.source != "builtin" }.map { "\($0.name) \($0.description)" } == ["one First", "two Second"], "slash agy: its two skill folders, a name once")
        check(v.allSatisfy { !["new", "settings", "switch", "undo", "quota", "cs", "branch"].contains($0.name) }, "slash agy: other names are not rows")
        func c(_ n: String) -> SlashCommands.Command { v.first { $0.name == n }! }
        check(c("clear").description.contains("/new") && vals(c("effort").args) == ["low", "medium", "high", "xhigh", "max"], "slash agy: other names in the description; /effort")
        check(c("model").args[1] == Arg("gemini-4-argon-medium", "Gemini 4 Argon (Medium)"), "slash agy: /model values with labels")
    }

    try scratch { t in
        func opens(_ kind: String, _ text: String) -> String? { SlashCommands.opensScreen(kind, text: text, home: t) }
        check(opens("claude", "/model") == "/model opens a menu in the terminal. Add a value, like /model claude-fable-5-2.", "screen: /model bare")
        check(opens("claude", "  /status \n") == "/status opens a screen in the terminal. Use it there.", "screen: /status, spaces trimmed")
        check(opens("claude", "/resume") == "/resume opens a menu in the terminal. Add a value, like /resume [session].", "screen: no fixed values: the hint")
        for ok in ["/model sonnet", "/status please", "/clear", "/compact", "/modelx", "model", "hello /model", "/"] {
            check(opens("claude", ok) == nil, "screen: \(ok) goes out")
        }
        check(opens("codex", "/model") == "/model opens a screen in the terminal. Use it there.", "screen: per agent; Codex with no model cache")
        check(opens("codex", "/status") == nil && opens("pi", "/thinking") != nil && opens("agy", "/usage") != nil && opens("cursor-agent", "/model") == nil, "screen: per agent")
        check(SlashCommands.screens("claude", home: t) == ["config", "help", "hooks", "mcp", "memory", "permissions", "plugin", "rewind", "status", "usage"], "screen: Claude's left-out screens")
        // Every screen command is either listed (it takes a value) or named in `screens`.
        for kind in ["claude", "codex", "pi", "agy"] {
            let listed = SlashCommands.list(kind, home: t, cwd: t), left = SlashCommands.screens(kind, home: t)
            check(SlashCommands.builtins(kind).filter(\.screen).allSatisfy { b in listed.contains { $0.name == b.name } != left.contains(b.name) }, "screen: \(kind) listed or left out, never both")
            // The composer's note (from the listing) is the send path's (from the tables).
            let listing = SlashCommands.listing(kind, home: t, cwd: t)
            check(SlashCommands.builtins(kind).allSatisfy { SlashList.screenNote(listing, text: "/\($0.name)") == SlashCommands.opensScreen(kind, text: "/\($0.name)", home: t) },
                  "screen: \(kind) the composer's note is the send path's")
        }
    }
}

func checksSlashModels() throws {
    try scratch { home in
        check(vals(SlashCommands.modelValues("claude", home: home)) == SlashCommands.claudeModels.map(\.0), "models claude: no files: the table")
        put(home.appendingPathComponent(".claude/settings.json"), #"{"modelSettings": {"claude-opus-5-5": {}, "claude-opus-6-0": {}}, "availableModels": ["claude-haiku-5-6"]}"#)
        put(home.appendingPathComponent(".claude.json"), #"{"additionalModelOptionsCache": [{"value": "claude-fable-5-1[1m]", "label": "Fable", "description": "Fable 5.1 · Most capable"}]}"#)
        let v = SlashCommands.modelValues("claude", home: home)
        let extra = v.dropFirst(SlashCommands.claudeModels.count).map { "\($0.value)=\($0.label)" }
        check(extra == ["claude-opus-6-0=Opus 6.0", "claude-haiku-5-6=Haiku 5.6", "claude-fable-5-1[1m]=Fable 5.1"], "models claude: the user's own, a known id once \(extra)")
        check(v.first == Arg("claude-fable-5-2", "Fable 5.2"), "models claude: the table first")
    }
    check(SlashCommands.claudeLabel("claude-sonnet-5-5") == "Sonnet 5.5" && SlashCommands.claudeLabel("claude-fable-5-1[1m]") == "Fable 5.1 (1M context)", "models: Claude labels")
    check(["opus", "claude-", "claude-opus", "claude-opus-latest", "gpt-6"].allSatisfy { SlashCommands.claudeLabel($0).isEmpty }, "models: no label for other ids")

    try scratch { home in
        check(SlashCommands.modelValues("codex", home: home).isEmpty, "models codex: no cache: no values")
        put(home.appendingPathComponent(".codex/models_cache.json"), """
            {"models": [{"slug": "b", "display_name": "B", "visibility": "list", "priority": 2},
              {"slug": "hidden", "display_name": "H", "visibility": "hide", "priority": 0},
              {"slug": "a", "display_name": "A", "visibility": "list", "priority": 1}]}
            """)
        check(SlashCommands.modelValues("codex", home: home) == [Arg("a", "A"), Arg("b", "B")], "models codex: its picker's models in its order")
    }

    try scratch { home in
        let a = home.appendingPathComponent(".pi/agent")
        put(a.appendingPathComponent("auth.json"), #"{"router": {"type": "api_key"}}"#)
        put(a.appendingPathComponent("models-store.json"), #"{"router": {"models": [{"id": "x/one", "name": "One"}]}, "nokey": {"models": [{"id": "no"}]}}"#)
        put(a.appendingPathComponent("models.json"), #"{"providers": {"router": {"models": [{"id": "x/two", "name": "Two"}]}, "local": {"apiKey": "!cmd", "models": [{"id": "small", "name": "Small"}]}}}"#)
        check(SlashCommands.modelValues("pi", home: home) == [Arg("local/small", "Small"), Arg("router/x/one", "One"), Arg("router/x/two", "Two")], "models pi: a provider with no key is left out")
        put(a.appendingPathComponent("settings.json"), #"{"enabledModels": ["router/x/*", "small:high"]}"#)
        check(vals(SlashCommands.modelValues("pi", home: home)) == ["local/small", "router/x/one", "router/x/two"], "models pi: enabled by glob and by id")
        put(a.appendingPathComponent("settings.json"), #"{"enabledModels": ["router/x/one"]}"#)
        check(vals(SlashCommands.modelValues("pi", home: home)) == ["router/x/one"], "models pi: only the enabled ones")
        check(SlashCommands.modelValues("agy", home: home).count == SlashCommands.agyModels.count && SlashCommands.modelValues("cursor-agent", home: home).isEmpty, "models: agy's table; none for others")
    }
    let g = SlashCommands.glob
    check(g("a/*", "a/b") && g("*b", "ab") && g("a*c*e", "abcde") && g("x", "x"), "glob: matches")
    check(!g("a/*", "b/a") && !g("a*a", "a") && !g("x", "xy"), "glob: no match")
}

func checksSlashList() {
    typealias Command = SlashCommands.Command
    let cmds = ["clear", "compact", "context", "deploy", "kit:hello"].map { Command(name: $0) }
    check(SlashList.slashQuery("/") == "" && SlashList.slashQuery("/de") == "de" && SlashList.slashQuery("/kit:he") == "kit:he", "slashQuery: the name after a leading /")
    check(["", "de", " /de", "/de ", "/deploy now", "/a\nb", "/a\r\nb"].allSatisfy { SlashList.slashQuery($0) == nil }, "slashQuery: nil with a space or no /")
    check(SlashList.matchCommands(cmds, "c").map(\.name) == ["clear", "compact", "context"] && SlashList.matchCommands(cmds, "CO").map(\.name) == ["compact", "context"]
          && SlashList.matchCommands(cmds, "zz").isEmpty, "matchCommands: by prefix, any case")
    let many = (0..<20).map { Command(name: String(format: "cmd%02d", $0)) }
    check(SlashList.matchCommands(many, "").count == 20, "matchCommands: all matches (the list scrolls)")
    let a = SlashList.argQuery("/model so")
    check(a?.name == "model" && a?.query == "so" && SlashList.argQuery("/model ")?.query == "", "argQuery: the command and the part of a value")
    check(["/model", "/model opus 5", "/ x", "model x", "/model\tx", "/model  x"].allSatisfy { SlashList.argQuery($0) == nil }, "argQuery: one space, no other")
    check(SlashList.windowTop(0, pick: 10, count: 20) == 3 && SlashList.windowTop(3, pick: 5, count: 20) == 3 && SlashList.windowTop(3, pick: 1, count: 20) == 1
          && SlashList.windowTop(15, pick: 0, count: 4) == 0, "windowTop: follows the pick, still while it can")

    // ↑ ↓ choose and stop at the ends; Tab completes the full name and a space; Esc closes until the text changes.
    var p = SlashPicker()
    p.textChanged("/c")
    _ = p.move(-1, text: "/c", commands: cmds); _ = p.move(1, text: "/c", commands: cmds)
    check(p.completion(text: "/c", commands: cmds) == "/compact ", "list: up on the first row stays there; down: the second")
    p.textChanged("/compact ")
    check(p.list(text: "/compact ", commands: cmds) == nil, "list: a completed name closes it")
    p.textChanged("/c")
    for _ in 0..<5 { _ = p.move(1, text: "/c", commands: cmds) }
    check(p.completion(text: "/c", commands: cmds) == "/context ", "list: down on the last row stays there (no wrap)")
    p.textChanged("/c")
    check(p.close(text: "/c", commands: cmds) && p.list(text: "/c", commands: cmds) == nil, "list: Esc closes it")
    check(!p.move(1, text: "/c", commands: cmds) && p.completion(text: "/c", commands: cmds) == nil && !p.close(text: "/c", commands: cmds),
          "list: closed: the keys are the text's own again")
    p.textChanged("/cl")
    check(p.list(text: "/cl", commands: cmds)?.items.count == 1, "list: typing opens it again")
    var none = SlashPicker()
    check(!none.move(1, text: "hello", commands: cmds) && none.completion(text: "", commands: cmds) == nil, "list: no list, no key taken")

    // Return: a part of a name completes; a whole name goes out.
    check(SlashPicker().enter(text: "/dep", listing: .init(commands: cmds), hasFiles: false) == .complete("/deploy "), "enter: a part of a name is completed, not sent")
    check(SlashPicker().enter(text: "/clear", listing: .init(commands: cmds), hasFiles: false) == .send, "enter: a whole name goes out")
    check(SlashPicker().enter(text: "hello", listing: .init(commands: cmds), hasFiles: false) == .send
          && SlashPicker().enter(text: "/zz", listing: .init(commands: cmds), hasFiles: false) == .send
          && SlashPicker().enter(text: "/dep", listing: nil, hasFiles: false) == .send, "enter: other text, an unknown name, or no list yet: sent")
    var closed = SlashPicker()
    _ = closed.close(text: "/dep", commands: cmds)
    check(closed.enter(text: "/dep", listing: .init(commands: cmds), hasFiles: false) == .send, "enter: a closed list completes nothing")

    // 20 matches: 8 rows show; down past the last row moves the window; the count says there are more.
    p = SlashPicker()
    p.textChanged("/cmd")
    var s = p.list(text: "/cmd", commands: many)
    check(s?.shown.map(\.name) == many[0..<8].map(\.name) && s?.count == "1–8 of 20" && s.map { $0.title($0.items[0]) } == "/cmd00", "window: the first 8 of 20")
    for _ in 0..<10 { _ = p.move(1, text: "/cmd", commands: many) }
    s = p.list(text: "/cmd", commands: many)
    check(s?.shown.map(\.name) == many[3..<11].map(\.name) && s?.count == "4–11 of 20" && s?.pick == 10, "window: pick 10 is its last row")
    for _ in 0..<20 { _ = p.move(1, text: "/cmd", commands: many) }
    check(p.list(text: "/cmd", commands: many)?.shown.last?.name == "cmd19", "window: down stops at the last match")
    for _ in 0..<19 { _ = p.move(-1, text: "/cmd", commands: many) }
    check(p.list(text: "/cmd", commands: many)?.shown.map(\.name) == many[0..<8].map(\.name), "window: up to the first: it follows back")
    for _ in 0..<12 { _ = p.move(1, text: "/cmd", commands: many) }
    p.textChanged("/cmd1")
    check(p.list(text: "/cmd1", commands: many)?.shown.map(\.name) == many[10..<18].map(\.name) && p.completion(text: "/cmd1", commands: many) == "/cmd10 ",
          "window: a new filter starts at its first match")
    check(p.list(text: "/c", commands: cmds)?.count == nil, "window: no count when all rows show")

    // The wheel moves the window a row per 30 points; the pick stays on a shown row.
    p = SlashPicker()
    p.textChanged("/cmd")
    p.scroll(deltaY: -90, text: "/cmd", commands: many)
    check(p.list(text: "/cmd", commands: many)?.shown.map(\.name) == many[3..<11].map(\.name) && p.completion(text: "/cmd", commands: many) == "/cmd03 ",
          "wheel: down 90: three rows, the pick moved into the window")
    p.textChanged("/cmd")
    p.scroll(deltaY: -15, text: "/cmd", commands: many); p.scroll(deltaY: -15, text: "/cmd", commands: many)
    check(p.list(text: "/cmd", commands: many)?.shown.first?.name == "cmd01", "wheel: small trackpad steps add up")
    p.scroll(deltaY: 1000, text: "/cmd", commands: many)
    check(p.list(text: "/cmd", commands: many)?.shown.first?.name == "cmd00", "wheel: up stops at the top")
    p.scroll(deltaY: -10000, text: "/cmd", commands: many)
    check(p.list(text: "/cmd", commands: many)?.shown.last?.name == "cmd19", "wheel: down stops at the bottom")
    var few = SlashPicker()
    few.scroll(deltaY: -300, text: "/c", commands: cmds)
    check(few.list(text: "/c", commands: cmds)?.top == 0 && few.list(text: "/c", commands: cmds)?.pick == 0, "wheel: nothing to scroll with 8 rows or fewer")

    // Screen commands and values.
    let built = [
        Command(name: "clear", description: "Start a new conversation"),
        Command(name: "effort", description: "Set effort level", hint: "low|medium|high|xhigh|max", args: ["low", "medium", "high", "xhigh", "max"].map { Arg($0) }, screen: true),
        Command(name: "model", description: "Set the AI model", hint: "<model>", args: ["opus", "sonnet", "haiku", "fable", "default"].map { Arg($0) }, screen: true),
    ]
    let listing = SlashCommands.Listing(commands: built, screens: ["status", "config"])
    check(SlashList.screenNote(listing, text: "/model") == "/model opens a menu in the terminal. Add a value, like /model opus.", "screenNote: a command that takes a value")
    check(SlashList.screenNote(listing, text: "  /status\n") == "/status opens a screen in the terminal. Use it there.", "screenNote: one that takes none")
    check(SlashList.screenNote(.init(commands: [Command(name: "resume", hint: "[session]", screen: true)]), text: "/resume")
          == "/resume opens a menu in the terminal. Add a value, like /resume [session].", "screenNote: no values: the hint")
    check(["/model sonnet", "/status now", "/clear", "/mode", "model", "hi /model", "/", ""].allSatisfy { SlashList.screenNote(listing, text: $0) == nil }, "screenNote: anything else is nil")
    check(SlashList.screenNote(nil, text: "/model") == nil, "screenNote: no list yet: the send path checks")
    check(SlashPicker().enter(text: "/model", listing: listing, hasFiles: false) == .blocked("/model opens a menu in the terminal. Add a value, like /model opus."),
          "enter: a bare screen command is not sent")
    check(SlashPicker().enter(text: "/model", listing: listing, hasFiles: true) == .send && SlashPicker().enter(text: "/model sonnet", listing: listing, hasFiles: false) == .send,
          "enter: with files, or with a value, it goes out")
    let model = built[2], row = SlashPicker().list(text: "/mo", commands: built)?.items.first
    check(row == SlashList.Item(name: "model", hint: "<model>", description: "Set the AI model", screen: true), "list: a built-in's hint and its \"needs a value\" mark")

    p = SlashPicker()
    check(p.completion(text: "/mo", commands: built) == "/model ", "values: Tab completes the command")
    var v = p.list(text: "/model ", commands: built)
    check(v?.command == "model" && v?.items.map(\.name) == vals(model.args) && v.map { $0.title($0.items[0]) } == "opus", "values: after \"/model \" its values, not commands")
    check(p.list(text: "/model s", commands: built)?.items.map(\.name) == ["opus", "sonnet"], "values: any part of a value")
    check(p.completion(text: "/model so", commands: built) == "/model sonnet", "values: Tab completes one")
    check(p.enter(text: "/model sonnet", listing: listing, hasFiles: false) == .send, "values: a whole value goes out")
    check(p.enter(text: "/effort x", listing: listing, hasFiles: false) == .complete("/effort xhigh"), "values: Return on a part completes it")
    check(p.list(text: "/clear x", commands: built) == nil && p.list(text: "/model opus 5", commands: built) == nil, "values: none for a command without values; a space ends the value")

    let labeled = [Command(name: "model", hint: "<model>", args: [Arg("claude-opus-5-5", "Opus 5.5"), Arg("claude-sonnet-5-5", "Sonnet 5.5"), Arg("sonnet", "The latest Sonnet")], screen: true)]
    check(SlashList.matchArgs(labeled[0].args, "SONNET").map(\.value) == ["claude-sonnet-5-5", "sonnet"]
          && SlashList.matchArgs(labeled[0].args, "5.5").map(\.value) == ["claude-opus-5-5", "claude-sonnet-5-5"] && SlashList.matchArgs(labeled[0].args, "").count == 3,
          "matchArgs: by a part of the value or the label")
    v = p.list(text: "/model ", commands: labeled)
    check(v?.items.first == SlashList.Item(name: "claude-opus-5-5", description: "Opus 5.5"), "values: full ids with their labels")
    check(p.enter(text: "/model op", listing: .init(commands: labeled), hasFiles: false) == .complete("/model claude-opus-5-5"), "values: a part of an id completes")
    check(SlashList.screenNote(.init(commands: labeled), text: "/model") == "/model opens a menu in the terminal. Add a value, like /model claude-opus-5-5.", "screenNote: the example is the first value")
}

func checksAgentBox() {
    let rule = String(repeating: "─", count: 40)
    func claude(_ row: String) -> String { "● Done.\n\n\(rule)\n\(row)\n\(rule)\n  ~/proj  ⎇main\n  Opus 5.5 ·medium\n" }
    func box(_ kind: String, _ screen: String) -> Bool? { AgentBox.hasText(kind: kind, screen: screen, claudeSuggests: false) }
    check(box("claude", claude("❯\u{a0}Try \"fix typecheck errors\"")) == false, "box claude: its placeholder")
    check(box("claude", claude("❯\u{a0}")) == false && box("claude", claude("❯ Press up to edit queued messages")) == false, "box claude: empty, or the queue's placeholder")
    check(box("claude", claude("❯\u{a0}/")) == true && box("claude", claude("❯\u{a0}hi there")) == true, "box claude: a half-typed command; text")
    check(box("claude", claude("❯ \n  second line")) == true, "box claude: text on a later row")
    check(box("claude", claude(" Do you want to proceed?\n ❯ 1. Yes")) == nil && box("claude", "no box here") == nil, "box claude: a dialog, or no box: a doubt")

    func suggests(_ row: String) -> Bool? { AgentBox.hasText(kind: "claude", screen: claude(row), claudeSuggests: true) }
    check(suggests("❯\u{a0}run the tests") == nil, "box claude, suggestions on: other text may be its dim suggestion (a doubt, sent as before)")
    check(suggests("❯\u{a0}/") == true && suggests("❯\u{a0}/sta") == true, "box claude, suggestions on: a command being typed is never a suggestion")
    check(suggests("❯\u{a0}") == false && suggests("❯\u{a0}Try \"fix typecheck errors\"") == false, "box claude, suggestions on: empty")

    func codex(_ row: String) -> String { "• Ran ls\n\n\(row)\n\n  GPT-6.1-Sol high · Context 100% left · 0 in\n  ← for agents · ? for shortcuts\n" }
    check(box("codex", codex("› Ask Codex to do anything")) == false && box("codex", codex("› hi there")) == true && box("codex", codex("› /")) == true
          && box("codex", "nothing") == nil, "box codex")
    // A newer Codex footer: a blank line, then model · folder · thread title (no "% left"), then the shortcuts.
    func titled(_ row: String) -> String { "• Ran ls\n\n\(row)\n\n  GPT-6.1-Sol high · ~/src/app · Fix the login form · Main [default]\n  ← for agents · ? for shortcuts\n" }
    check(box("codex", titled("› Ask Codex to do anything")) == false, "box codex, footer with no \"% left\": the empty box is empty")
    check(box("codex", titled("› hi there")) == true, "box codex, footer with no \"% left\": one line of text")
    check(box("codex", titled("› first line\n  second line")) == true, "box codex, footer with no \"% left\": two lines of text")

    func pi(_ row: String) -> String { "\(rule)\n\(rule)\n\(row)\n\(rule)\n~/proj  ⎇main\n" }
    check(box("pi", pi("")) == false && box("pi", pi("hi there")) == true && box("pi", pi("\nsecond")) == true && box("pi", "no rules") == nil, "box pi")

    func agy(_ row: String) -> String { "  Gemini 3.8 Flash (Medium)\n\n\(rule)\n\(row)\n\(rule)\n? for shortcuts\n" }
    check(box("agy", agy(">")) == false && box("agy", agy("> Auto mode: edits & safe commands auto-approved (shift+tab to cycle)")) == false, "box agy: empty, or a mode's note")
    check(box("agy", agy("> hi there")) == true && box("agy", agy("Switch Model")) == nil, "box agy: text; a menu is a doubt")

    func grok(_ row: String) -> String { "  Update: available\n\n  ╭──────────────\n  │ \(row)   \n  ╰──────────────\n  Enter:send\n" }
    check(box("grok", grok("❯ ")) == false && box("grok", grok("❯ hi there")) == true && box("grok", "  ╭──\n  │ Pick one\n  ╰──\n") == nil, "box grok")
    check(box("cursor-agent", "❯ hi") == nil, "box: other agents are not read")
    check(AgentBox.notEmpty == "The agent's box has text in it. Clear it in the terminal first. Nothing was sent.", "box: the note")

    let home = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("hkbox-\(UUID().uuidString)")
    defer { try? FileManager.default.removeItem(at: home) }
    check(AgentBox.claudeSuggests(home: home) && AgentBox.claudeSuggests(home: nil), "box: prompt suggestions are on by default")
    put(home.appendingPathComponent(".claude/settings.json"), #"{"promptSuggestionEnabled": false}"#)
    check(!AgentBox.claudeSuggests(home: home), "box: prompt suggestions turned off in the settings")
    put(home.appendingPathComponent(".claude/settings.json"), #"{"promptSuggestionEnabled": 0}"#)
    check(AgentBox.claudeSuggests(home: home), "box: only a JSON false turns them off")
}
