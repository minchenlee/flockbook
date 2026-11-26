import Foundation

/// What an agent's last turn did, read from its log by fixed rules: no model and no network. For the chat's
/// "Since you left" card (`SinceLeft`). The Swift side of the helper's `glance` (xplat/flockbook-modfeed/src/glance.rs)
/// and of the mod's `facts` (herdr-app/mods/flockbook/glance.js): the same rules, and the same cases in
/// `Tests/HerdrKitTests/Fixtures/since-vectors.json`.
public enum Glance {
    /// The last turn: everything after the newest user message.
    public struct Facts: Sendable, Hashable, Codable {
        /// The id of the turn's newest item. Another id: the agent wrote more.
        public var id: String?
        /// When the turn's newest item came (ms since 1970).
        public var ended: Int64?
        /// Files the edit tools changed (each file once). An edit made by a shell command is not seen.
        public var edited = 0
        /// Shell commands with a result.
        public var commands = 0
        /// The newest finished command, when it failed: its first line.
        public var failed: String?
        /// The newest test command after the last edit: "passed", "failed", "ran" (its exit code went into a pipe), or
        /// "none" (code files were edited and no test command finished after the last edit). Nil: no test, and no edit
        /// but to files of words.
        public var tests: String?
        /// That test command's short name ("pytest", "cargo test").
        public var test: String?
        /// The first sentence of the newest message.
        public var said: String?
        public init(id: String? = nil, ended: Int64? = nil, edited: Int = 0, commands: Int = 0, failed: String? = nil,
                    tests: String? = nil, test: String? = nil, said: String? = nil) {
            self.id = id; self.ended = ended; self.edited = edited; self.commands = commands; self.failed = failed
            self.tests = tests; self.test = test; self.said = said
        }
    }

    public static let saidCap = 160
    public static let commandCap = 120

    /// The text cut at `n` characters (graphemes), with "…" when it was cut. The helper's `rows::cut`.
    public static func cut(_ s: String, _ n: Int) -> String {
        s.count > n ? String(s.prefix(n)) + "…" : s
    }

    static func ms(_ d: Date?) -> Int64? { d.map { Int64(($0.timeIntervalSince1970 * 1000).rounded()) } }

    static func isEdit(_ tool: String) -> Bool { ["Edit", "MultiEdit", "NotebookEdit", "Write", "apply_patch"].contains(tool) }
    static func isCommand(_ tool: String) -> Bool { ["Bash", "exec", "exec_command", "shell", "local_shell", "container.exec"].contains(tool) }

    /// The files one edit call names. A patch names each in its own header line; other tools have the path as summary.
    static func editFiles(_ c: ToolCall) -> [String] {
        if c.toolName == "apply_patch" {
            let marks = ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
            let files = c.input.replacingOccurrences(of: "\\n", with: "\n").split(separator: "\n", omittingEmptySubsequences: false)
                .compactMap { l -> String? in
                    let t = l.trimmingCharacters(in: .whitespaces)
                    guard let m = marks.first(where: { t.hasPrefix($0) }) else { return nil }
                    return String(t.dropFirst(m.count)).trimmingCharacters(in: .whitespaces)
                }
                .filter { !$0.isEmpty }
            if !files.isEmpty { return files }
        }
        let s = c.summary.trimmingCharacters(in: .whitespacesAndNewlines)
        return [s.isEmpty ? "call \(c.id)" : s]
    }

    /// A file of words or data rows, by its name: no test is expected after an edit to one.
    static func prose(_ file: String) -> Bool {
        let name = (file.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? file).lowercased()
        guard let dot = name.lastIndex(of: ".") else { return false }
        return ["md", "mdx", "markdown", "txt", "rst", "adoc", "org", "csv", "tsv", "log"].contains(String(name[name.index(after: dot)...]))
    }

    /// A finished command failed. Claude Code, pi and Grok logs say so; Codex's log has only the output, which starts
    /// with the exit code.
    static func failed(_ c: ToolCall, agent: String) -> Bool {
        if c.isError { return true }
        if agent == "claude" { return false }
        let head = String((c.output ?? "").prefix(400))
        for mark in ["Process exited with code ", "Exit code: ", "exit code: ", "\"exit_code\":"] {
            guard let r = head.range(of: mark) else { continue }
            let digits = head[r.upperBound...].drop(while: { $0.isWhitespace }).prefix(while: { $0.isASCII && $0.isNumber })
            if let n = Int(digits) { return n != 0 }
        }
        return false
    }

    /// The command split where a shell starts the next one (`&&`, `||`, `;`, `|`, a new line), outside quotes. Each part
    /// with whether its output goes into a pipe. Text after a here-document mark is data: left out.
    static func parts(_ cmd: String) -> [(String, Bool)] {
        let cmd = cmd.components(separatedBy: "<<").first ?? ""
        var out: [(String, Bool)] = [], cur = "", quote: Character? = nil, esc = false
        let cs = Array(cmd)
        var i = 0
        while i < cs.count {
            let ch = cs[i]; i += 1
            if esc { cur.append(ch); esc = false; continue }
            if let q = quote {
                if ch == q { quote = nil } else if ch == "\\" && q == "\"" { esc = true }
                cur.append(ch); continue
            }
            switch ch {
            case "'", "\"": quote = ch; cur.append(ch)
            case "\\": esc = true
            case ";", "\n": out.append((cur, false)); cur = ""
            case "&" where i < cs.count && cs[i] == "&": i += 1; out.append((cur, false)); cur = ""
            case "|":
                let or = i < cs.count && cs[i] == "|"
                if or { i += 1 }
                out.append((cur, !or)); cur = ""
            default: cur.append(ch)
            }
        }
        out.append((cur, false))
        return out.filter { !$0.0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    /// Programs that run tests themselves: their own `--` passes arguments on, it does not start another command.
    static let testers: Set<String> = ["cargo", "npm", "yarn", "pnpm", "bun", "go", "swift", "deno", "dotnet", "node", "make", "pytest"]

    /// The test runner one command part starts, by name.
    static func runner(_ part: String) -> String? {
        var w = part.split(whereSeparator: { $0.isWhitespace }).map(String.init)
        // Leading `A=b` settings and the words that only start another program.
        while let first = w.first {
            let n: Int
            if first.contains("=") && !first.hasPrefix("-") { n = 1 }
            else if ["sudo", "time", "env", "npx", "bunx", "command", "exec"].contains(first) { n = 1 }
            else if w.count >= 2, (["pnpm", "bundle", "yarn"].contains(first) && w[1] == "exec") || (["uv", "poetry", "pipenv", "pdm", "hatch"].contains(first) && w[1] == "run") { n = 2 }
            else if w.count >= 2, ["python", "python3", "py"].contains(first), w[1] == "-m" { n = 2 }
            else if ["nice", "caffeinate", "nohup"].contains(first) { n = 1 }
            else if w.count >= 2, ["timeout", "gtimeout"].contains(first) { n = 2 }
            // A runner that takes the real command after `--` (a queue, a sandbox, a secrets wrapper).
            else if !first.hasPrefix("-"), let p = w.dropFirst().firstIndex(of: "--"), !testers.contains(first) { n = p + 1 }
            else { break }
            w.removeFirst(n)
        }
        guard let head = w.first else { return nil }
        let first = head.split(separator: "/", omittingEmptySubsequences: false).last.map(String.init) ?? ""
        let rest = Array(w.dropFirst())
        let has = { (x: String) in rest.contains(x) }
        let firstArg = rest.first { !$0.hasPrefix("+") && !$0.hasPrefix("-") }
        func script(_ r: [String]) -> Bool {
            guard let a = r.first else { return false }
            if a == "test" || a == "t" { return true }
            if a == "run", r.count >= 2 { return r[1].hasPrefix("test") }
            return a.hasPrefix("test:")
        }
        let one = { (names: String...) in names.contains(first) }
        if one("pytest", "py.test") { return "pytest" }
        if one("unittest") { return "unittest" }
        if one("tox", "nox", "jest", "vitest", "mocha", "ava", "rspec", "phpunit", "ctest") { return first }
        if one("npm", "yarn", "pnpm", "bun"), script(rest) { return "\(first) test" }
        if one("go"), rest.first == "test" { return "go test" }
        if one("cargo"), firstArg == "test" { return "cargo test" }
        if one("cargo"), firstArg == "nextest" { return "cargo nextest" }
        if one("swift", "deno", "dotnet", "playwright"), rest.first == "test" { return "\(first) test" }
        if one("node"), has("--test") { return "node --test" }
        if one("mvn", "mvnw"), has("test") || has("verify") { return "mvn test" }
        if one("gradle", "gradlew"), rest.contains(where: { $0.lowercased().hasSuffix("test") || $0 == "check" }) { return "gradle test" }
        if one("make", "just"), rest.contains(where: { $0.hasPrefix("test") || $0 == "check" }) { return "\(first) test" }
        if one("xcodebuild"), has("test") { return "xcodebuild test" }
        return nil
    }

    /// The test command in `cmd`: its short name, and whether its output goes into a pipe (the shell then gives the
    /// exit code of the last program, not of the tests).
    static func testRun(_ cmd: String) -> (String, Bool)? {
        let pipefail = cmd.contains("pipefail")
        for (p, piped) in parts(cmd) { if let n = runner(p) { return (n, piped && !pipefail) } }
        return nil
    }

    /// The short name of the test runner a shell command starts ("pytest", "npm test"), by the program's name.
    public static func testName(_ cmd: String) -> String? { testRun(cmd)?.0 }

    /// The first sentence of a message, without markdown marks: the first line with text outside a code block, up to
    /// its first full stop, question or exclamation mark that ends a sentence.
    public static func firstSentence(_ text: String) -> String? {
        var code = false
        for raw in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let t = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            if t.hasPrefix("```") || t.hasPrefix("~~~") { code.toggle(); continue }
            if code { continue }
            var l = Substring(t).drop(while: { $0 == "#" || $0 == ">" || $0 == " " })
            for bullet in ["- ", "* ", "+ ", "• "] where l.hasPrefix(bullet) { l = l.dropFirst(bullet.count) }
            if let r = l.range(of: ". ") {
                let n = l[..<r.lowerBound]
                if !n.isEmpty, n.count <= 3, n.allSatisfy({ $0.isASCII && $0.isNumber }) { l = l[r.upperBound...] }
            }
            let plain = String(l).replacingOccurrences(of: "**", with: "").replacingOccurrences(of: "__", with: "").replacingOccurrences(of: "`", with: "")
            let s = shortPaths(unlinked(plain)).trimmingCharacters(in: .whitespacesAndNewlines)
            guard s.unicodeScalars.contains(where: { $0.properties.isAlphabetic || $0.properties.numericType != nil }) else { continue }
            let cs = Array(s.unicodeScalars)
            var end = cs.count
            for (n, c) in cs.enumerated() {
                let next = n + 1 < cs.count ? cs[n + 1] : nil
                let stop: Bool
                switch c {
                case "。", "！", "？": stop = true
                case ".", "!", "?": stop = next?.properties.isWhitespace ?? false
                default: stop = false
                }
                if stop { end = n + 1; break }
            }
            var head = String.UnicodeScalarView()
            head.append(contentsOf: cs[0..<end])
            return cut(String(head), saidCap)
        }
        return nil
    }

    /// A path as its last name only: "/w/src/a.rs" and "~/w/a.rs" read "a.rs". A path here starts with "/" or "~/"
    /// and has two or more "/"; a web address stays whole.
    static func shortPaths(_ s: String) -> String {
        func part(_ c: Character) -> Bool { (c.isASCII && (c.isLetter || c.isNumber)) || "/._-~+@".contains(c) }
        let cs = Array(s)
        var out = "", i = 0
        while i < cs.count {
            let before: Character? = i > 0 ? cs[i - 1] : nil
            if (cs[i] == "/" || (cs[i] == "~" && i + 1 < cs.count && cs[i + 1] == "/")) && !(before.map { part($0) || $0 == ":" } ?? false) {
                var j = i
                while j < cs.count && part(cs[j]) { j += 1 }
                let run = String(cs[i..<j])
                var path = Substring(run)
                while let last = path.last, last == "." || last == "/" { path = path.dropLast() }
                if path.filter({ $0 == "/" }).count >= 2 {
                    out += path.split(separator: "/", omittingEmptySubsequences: false).last.map(String.init) ?? ""
                    out += run.dropFirst(path.count)
                    i = j
                    continue
                }
            }
            out.append(cs[i]); i += 1
        }
        return out
    }

    /// Markdown links as their text only: `[notes](file:///a/notes.md)` reads "notes".
    static func unlinked(_ s: String) -> String {
        var out = "", rest = Substring(s)
        while let i = rest.firstIndex(of: "[") {
            let after = rest[rest.index(after: i)...]
            if let j = after.range(of: "](")?.lowerBound, let k = after[after.index(j, offsetBy: 2)...].firstIndex(of: ")"), !after[..<j].contains("[") {
                out += rest[..<i]; out += after[..<j]
                rest = after[after.index(after: k)...]
            } else {
                out += rest[...i]
                rest = after
            }
        }
        return out + rest
    }

    /// The facts of the last turn in `items` (one agent's conversation, oldest first). `agent`: its kind ("claude").
    public static func facts(_ items: [ConversationItem], agent: String) -> Facts {
        let from = items.lastIndex { if case .userText = $0.kind { return true } else { return false } }
        let turn = items[(from.map { $0 + 1 } ?? 0)...]
        var f = Facts()
        f.id = (turn.last ?? from.map { items[$0] })?.id
        f.ended = turn.reversed().lazy.compactMap { ms($0.timestamp) }.first
        var files: [String] = []
        var test: (String, String)?
        var text: String?
        for it in turn {
            switch it.kind {
            case .assistantText(let s): text = s
            case .toolChips(let chips):
                // The chips group a run's calls by kind: `seq` puts them back in the order of the log.
                for c in chips.flatMap(\.calls).enumerated().sorted(by: { ($0.element.seq, $0.offset) < ($1.element.seq, $1.offset) }).map(\.element) {
                    if isEdit(c.toolName) {
                        for file in editFiles(c) where !files.contains(file) { files.append(file) }
                        test = nil
                    } else if isCommand(c.toolName), c.output != nil {
                        f.commands += 1
                        let bad = failed(c, agent: agent)
                        let line = c.summary.split(separator: "\n", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? ""
                        f.failed = bad ? cut(line, commandCap) : nil
                        if let (name, piped) = testRun(c.summary) { test = (bad ? "failed" : piped ? "ran" : "passed", name) }
                    }
                }
            default: break
            }
        }
        f.edited = files.count
        if let (state, name) = test { f.tests = state; f.test = name }
        else if files.contains(where: { !prose($0) }) { f.tests = "none" }
        f.said = text.flatMap(firstSentence)
        return f
    }

    /// The line from the log: "Edited 5 files · pytest passed after the last edit". "" when there is nothing.
    /// The mod's `facts` (glance.js).
    public static func line(_ g: Facts?) -> String {
        guard let g else { return "" }
        func count(_ n: Int, _ one: String, _ many: String) -> String { "\(n) \(n == 1 ? one : many)" }
        var out: [String] = []
        if g.edited > 0 { out.append("edited \(count(g.edited, "file", "files"))") }
        let after = g.edited > 0 ? " after the last edit" : ""
        let test = g.test.flatMap { $0.isEmpty ? nil : $0 } ?? "a test command"
        switch g.tests {
        case "passed": out.append("\(test) passed\(after)")
        case "failed": out.append("\(test) failed\(after)")
        case "ran": out.append("\(test) ran\(after), result not known")
        case "none": out.append("no test run after the last edit")
        default: break
        }
        if let failed = g.failed, !failed.isEmpty, g.tests != "failed" { out.append("the last command failed: \(failed)") }
        if out.isEmpty && g.commands > 0 { out.append("ran \(count(g.commands, "command", "commands"))") }
        let s = out.joined(separator: " · ")
        return s.isEmpty ? "" : s.prefix(1).uppercased() + s.dropFirst()
    }
}
