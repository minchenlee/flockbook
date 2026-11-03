import Foundation
import HerdrKit

/// `status`: the tags over the composer (model, effort, ctx, cost, and the owner's own status line parts).
/// Claude Code's facts come from Flockbook's status line wrapper, when the owner turned it on; otherwise the
/// model named in the log is the only tag.
public enum FeedStatus {
    public struct Tag: Encodable, Sendable, Equatable {
        /// "model", "effort", "context", "cost", "own".
        public var kind: String
        public var text: String
        public var percent: Int?
        public var warn: Bool
    }

    /// The folder Flockbook's wrapper writes to: the `--dir` of a `flockbook-statusline` status line command in
    /// Claude Code's settings. Nil when the wrapper is not on.
    public static func folder(settings: URL) -> URL? {
        guard let d = try? Data(contentsOf: settings), let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any],
              let cmd = (o["statusLine"] as? [String: Any])?["command"] as? String, cmd.contains("flockbook-statusline") else { return nil }
        let words = shellWords(cmd)
        guard let i = words.firstIndex(of: "--dir"), words.indices.contains(i + 1) else { return nil }
        return URL(fileURLWithPath: words[i + 1], isDirectory: true)
    }

    /// Splits a command line into words: single quotes, double quotes and backslashes as a POSIX shell reads them.
    public static func shellWords(_ s: String) -> [String] {
        var out: [String] = [], cur = "", has = false, quote: Character? = nil, esc = false
        for ch in s {
            if esc { cur.append(ch); esc = false; continue }
            if let q = quote {
                if ch == q { quote = nil } else if ch == "\\" && q == "\"" { esc = true } else { cur.append(ch) }
                continue
            }
            switch ch {
            case "'", "\"": quote = ch; has = true
            case "\\": esc = true; has = true
            case " ", "\t", "\n": if has || !cur.isEmpty { out.append(cur) }; cur = ""; has = false
            default: cur.append(ch)
            }
        }
        if has || !cur.isEmpty { out.append(cur) }
        return out
    }

    public static func defaultSettings(env: [String: String] = ProcessInfo.processInfo.environment) -> URL {
        let dir = env["CLAUDE_CONFIG_DIR"].flatMap { $0.isEmpty ? nil : $0 } ?? NSHomeDirectory() + "/.claude"
        return URL(fileURLWithPath: dir + "/settings.json")
    }

    /// Prints one `status` line. `log`: the session log (its file name is Claude Code's session id).
    public static func run(agent: String, log: URL?, settings: URL, out: JSONLines, now: Date = Date()) {
        var tags: [Tag] = [], source = "none"
        if agent == "claude", let log, let dir = folder(settings: settings) {
            let sid = log.deletingPathExtension().lastPathComponent
            if let rec = StatusLineFolder(url: dir).record(session: sid, now: now), !rec.isEmpty {
                tags = rec.tags.map { Tag(kind: $0.kind.rawValue, text: $0.text, percent: $0.percent, warn: $0.warn) }
                source = "statusline"
            }
        }
        if tags.isEmpty, let log, let r = try? FeedTail.read(log, bytes: 256 * 1024), let m = FeedTail.model(r.lines) {
            tags = [Tag(kind: "model", text: m, percent: nil, warn: false)]
            source = "log"
        }
        struct Line: Encodable { var type = "status"; var agent: String; var source: String; var tags: [Tag] }
        out.emit(Line(agent: agent, source: source, tags: tags))
    }
}
