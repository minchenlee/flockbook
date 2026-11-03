import Foundation
@testable import HerdrKit

// Foundation lives here, not in the test file: importing Foundation and Testing together needs
// _Testing_Foundation, which Command Line Tools does not ship.

enum Support {
    static func fixtureData(_ name: String) throws -> Data {
        guard let url = Bundle.module.url(forResource: name, withExtension: nil, subdirectory: "Fixtures") else {
            throw HerdrError.protocolError("missing fixture \(name)")
        }
        return try Data(contentsOf: url)
    }

    static func sessionLines() throws -> [String] {
        String(decoding: try fixtureData("session.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    }

    static func codexLines() throws -> [String] {
        String(decoding: try fixtureData("codex.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
    }

    static func parseSnapshot() throws -> [Workspace] { try SnapshotParser.parse(try fixtureData("snapshot.json")) }

    static func describe(_ items: [ConversationItem]) -> [String] {
        items.map { i in
            switch i.kind {
            case .userText(let t): "user:\(t)"
            case .assistantText(let t): "asst:\(t)"
            case .toolChips(let c): "chips:" + c.map(\.text).joined(separator: "|")
            case .permissionRequest(let p): "perm:\(p.command)"
            case .question(let q): "question:" + q.questions.map(\.text).joined(separator: "|")
            case .status: "status"
            }
        }
    }

    /// Writes first line, locates, tails, appends line 5; returns observed item counts and lookups.
    static func tailRoundTrip() async throws -> (found: Bool, escaped: Bool, counts: [Int]) {
        let dir = NSTemporaryDirectory() + "hk-\(UUID().uuidString)"
        try FileManager.default.createDirectory(atPath: dir + "/proj", withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(atPath: dir) }
        let path = dir + "/proj/abc.jsonl"
        let lines = try sessionLines()
        try (lines[0] + "\n").write(toFile: path, atomically: true, encoding: .utf8)
        let found = SessionLog.locate(sessionId: "abc", projectsDir: dir)?.path == path
        let escaped = SessionLog.locate(sessionId: "../abc", projectsDir: dir) != nil

        var it = SessionLog(sessionId: "abc").items(projectsDir: dir, pollInterval: .milliseconds(30)).makeAsyncIterator()
        var counts = [(await it.next() ?? []).count]
        counts.append((await it.next() ?? []).count)
        let fh = try FileHandle(forWritingTo: URL(fileURLWithPath: path))
        try fh.seekToEnd()
        try fh.write(contentsOf: Data((lines[4] + "\n").utf8))
        try fh.close()
        counts.append((await it.next() ?? []).count)
        return (found, escaped, counts)
    }
}

extension Support {
    static func bytes(_ b: [UInt8]) -> Data { Data(b) }
    static func array(_ d: Data) -> [UInt8] { [UInt8](d) }
}
