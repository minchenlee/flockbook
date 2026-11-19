import Foundation

/// The Mac app's own files for the "Since you left" card and Summaries, in one folder (Application Support; a temp
/// folder in checks and snapshots). Nothing here reads or writes the Herdr app's home.
/// - `since-marks.json`: what the owner saw of each conversation (`SinceLeft.Seen`), so a restart does not forget it.
/// - `summaries.json`: the kept summaries (`Summary.Kept`), one per conversation.
/// - `summary-usage.jsonl`: a line for each run of a model: the time, the program, the model, the characters that
///   went and came, the time it took, and whether it worked. No word of the turn or of the answer.
/// Keys are "<herdr session>|<terminal>" (`key`), so two sessions do not mix.
@MainActor
public final class SinceStore {
    public let dir: URL
    public private(set) var seen: [String: SinceLeft.Seen] = [:]
    public private(set) var summaries: [String: Summary.Kept] = [:]

    public init(dir: URL) {
        self.dir = dir
        seen = Self.read([String: [String: SinceLeft.Seen]].self, dir.appendingPathComponent("since-marks.json"))?["seen"] ?? [:]
        summaries = Self.read([String: Summary.Kept].self, dir.appendingPathComponent("summaries.json")) ?? [:]
    }

    /// The key of one conversation: herdr's session name (empty: the default one) and the terminal.
    public nonisolated static func key(session: String?, terminal: String) -> String { "\(session ?? "")|\(terminal)" }

    static func read<T: Decodable>(_ t: T.Type, _ url: URL) -> T? {
        (try? Data(contentsOf: url)).flatMap { try? JSONDecoder().decode(T.self, from: $0) }
    }

    /// Written whole through a temp file. A failed write only loses that: the values stay for this run.
    func write<T: Encodable>(_ v: T, _ name: String) {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let e = JSONEncoder(); e.outputFormatting = [.sortedKeys]
        guard let d = try? e.encode(v) else { return }
        try? (d + Data("\n".utf8)).write(to: dir.appendingPathComponent(name), options: .atomic)
    }

    /// The conversation shows the turn `id` now (`now`: ms). Still the same turn: only the time moves, and not more
    /// than once a minute.
    public func markSeen(_ key: String, id: String?, now: Int64) {
        guard let id else { return }
        let was = seen[key]
        if was?.id == id {
            guard let was, now - was.at >= 60_000 else { return }
            seen[key] = SinceLeft.Seen(id: id, at: now, since: was.since)
        } else {
            seen[key] = SinceLeft.Seen(id: id, at: now, since: was?.at)
        }
        write(["seen": seen], "since-marks.json")
    }

    public func keep(_ key: String, _ k: Summary.Kept) {
        summaries[key] = k
        write(summaries, "summaries.json")
    }

    /// Drops the summaries of conversations that are gone.
    public func prune(keeping keys: Set<String>) {
        let next = summaries.filter { keys.contains($0.key) }
        guard next.count != summaries.count else { return }
        summaries = next
        write(summaries, "summaries.json")
    }

    var usageURL: URL { dir.appendingPathComponent("summary-usage.jsonl") }

    /// Adds a line for a run of a model.
    public func noteRun(at: Int64, by: String, model: String, sent: Int, got: Int, ms: Int, ok: Bool) {
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let o: [String: Any] = ["at": at, "by": by, "model": model, "sent": sent, "got": got, "ms": ms, "ok": ok]
        guard let d = try? JSONSerialization.data(withJSONObject: o, options: [.sortedKeys]) else { return }
        let line = d + Data("\n".utf8)
        if let fh = try? FileHandle(forWritingTo: usageURL) { defer { try? fh.close() }; _ = try? fh.seekToEnd(); try? fh.write(contentsOf: line) }
        else { try? line.write(to: usageURL) }
    }

    static let usageDays: Int64 = 35
    static let usageBytes = 512 * 1024

    /// For each time in `since` (ms), the runs of a model from then on. A file over 512 KB keeps the last 35 days.
    public func usage(since: [Int64], now: Int64) -> [Summary.Usage] {
        let text = (try? String(contentsOf: usageURL, encoding: .utf8)) ?? ""
        let rows = text.split(separator: "\n").compactMap { (try? JSONSerialization.jsonObject(with: Data($0.utf8))) as? [String: Any] }
        func n(_ r: [String: Any], _ k: String) -> Int { (r[k] as? NSNumber)?.intValue ?? 0 }
        if text.utf8.count > Self.usageBytes {
            let from = now - Self.usageDays * 86_400_000
            let kept = text.split(separator: "\n").filter { l in
                ((try? JSONSerialization.jsonObject(with: Data(l.utf8))) as? [String: Any]).map { Int64(n($0, "at")) >= from } ?? false
            }
            try? Data((kept.joined(separator: "\n") + (kept.isEmpty ? "" : "\n")).utf8).write(to: usageURL, options: .atomic)
        }
        return since.map { s in
            let part = rows.filter { Int64(n($0, "at")) >= s }
            return Summary.Usage(runs: part.count, failed: part.filter { ($0["ok"] as? Bool) != true }.count,
                                 sent: part.reduce(0) { $0 + n($1, "sent") }, got: part.reduce(0) { $0 + n($1, "got") }, ms: part.reduce(0) { $0 + n($1, "ms") })
        }
    }
}

/// What a program run gave.
public struct ExecResult: Sendable, Hashable {
    /// The exit code; nil when it did not run to an end (not found, timed out).
    public var code: Int32?
    public var stdout: String
    public var stderr: String
    public var timedOut = false
    /// The program was not found.
    public var missing = false
    public init(code: Int32?, stdout: String = "", stderr: String = "", timedOut: Bool = false, missing: Bool = false) {
        self.code = code; self.stdout = stdout; self.stderr = stderr; self.timedOut = timedOut; self.missing = missing
    }
}

/// Runs one program once, without a terminal and without a shell.
public protocol SummaryRunner: Sendable {
    func run(argv: [String], input: String, cwd: String, timeout: Int) async -> ExecResult
}

/// On this Mac, as a child of the app. The app may have a short PATH (started from Finder): the usual folders of
/// command line programs are added after it. A Claude Code session's markers are not passed on, so `claude -p` does not
/// take itself for a nested session (the rest of the environment stays, logins and providers too).
public struct ProcessRunner: SummaryRunner {
    public init() {}

    public static func environment(_ env: [String: String] = ProcessInfo.processInfo.environment, home: String = NSHomeDirectory()) -> [String: String] {
        var e = env
        // Only the markers of a running Claude Code session: its login and provider settings stay.
        e["CLAUDECODE"] = nil; e["CLAUDE_CODE_ENTRYPOINT"] = nil
        let extra = ["\(home)/.local/bin", "/opt/homebrew/bin", "/usr/local/bin", "\(home)/.bun/bin", "\(home)/.npm-global/bin", "\(home)/.cargo/bin", "/usr/bin", "/bin"]
        var path = (e["PATH"] ?? "").split(separator: ":").map(String.init)
        for p in extra where !path.contains(p) { path.append(p) }
        e["PATH"] = path.joined(separator: ":")
        return e
    }

    /// The program's file: a path as given, else the first match on PATH.
    public static func find(_ program: String, path: String) -> String? {
        if program.contains("/") { return FileManager.default.isExecutableFile(atPath: program) ? program : nil }
        return path.split(separator: ":").map { "\($0)/\(program)" }.first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// Writing to a program that closed its stdin must not end the app (SIGPIPE): the write fails instead.
    private static let noSigpipe: Void = { signal(SIGPIPE, SIG_IGN); return () }()

    /// Its output, filled from the pipes as it comes.
    private final class Output: @unchecked Sendable {
        let lock = NSLock()
        var out = Data(), err = Data()
        var timedOut = false
    }

    /// The program of one run, so the task that asked can end it.
    private final class Running: @unchecked Sendable {
        let lock = NSLock()
        var process: Process?
        var cancelled = false
        /// Ends the program (TERM, then KILL after 2 s), now or as soon as it starts.
        func stop() {
            let p: Process? = lock.withLock { cancelled = true; return process }
            guard let p, p.isRunning else { return }
            let pid = p.processIdentifier
            p.terminate()
            DispatchQueue.global().asyncAfter(deadline: .now() + 2) { if p.isRunning { kill(pid, SIGKILL) } }
        }
    }

    /// Ends with the task: a run called off (the app quits, Summaries is turned off) stops its program.
    public func run(argv: [String], input: String, cwd: String, timeout: Int) async -> ExecResult {
        let env = Self.environment()
        guard let first = argv.first, let exe = Self.find(first, path: env["PATH"] ?? "") else { return ExecResult(code: nil, missing: true) }
        _ = Self.noSigpipe
        let running = Running()
        return await withTaskCancellationHandler(operation: {
            await start(exe: exe, argv: argv, env: env, input: input, cwd: cwd, timeout: timeout, running: running)
        }, onCancel: { running.stop() })
    }

    private func start(exe: String, argv: [String], env: [String: String], input: String, cwd: String, timeout: Int, running: Running) async -> ExecResult {
        await withCheckedContinuation { (k: CheckedContinuation<ExecResult, Never>) in
            DispatchQueue.global().async {
                let p = Process(), out = Pipe(), err = Pipe(), inp = Pipe(), got = Output()
                p.executableURL = URL(fileURLWithPath: exe)
                p.arguments = Array(argv.dropFirst())
                p.environment = env
                try? FileManager.default.createDirectory(atPath: cwd, withIntermediateDirectories: true)
                p.currentDirectoryURL = URL(fileURLWithPath: cwd)
                p.standardOutput = out; p.standardError = err; p.standardInput = inp
                // Read as it comes. A program can leave a child that keeps the pipe open after it ends: the end of the
                // program, not the end of the pipe, decides when the run is over.
                let g = DispatchGroup()
                for (pipe, isOut) in [(out, true), (err, false)] {
                    g.enter()
                    pipe.fileHandleForReading.readabilityHandler = { h in
                        let d = h.availableData
                        if d.isEmpty { h.readabilityHandler = nil; g.leave(); return }
                        got.lock.withLock { if isOut { got.out.append(d) } else { got.err.append(d) } }
                    }
                }
                let off: Bool = running.lock.withLock {
                    if running.cancelled { return true }
                    do { try p.run() } catch { return true }
                    running.process = p
                    return false
                }
                if off {
                    out.fileHandleForReading.readabilityHandler = nil; err.fileHandleForReading.readabilityHandler = nil
                    k.resume(returning: ExecResult(code: nil, missing: !running.lock.withLock { running.cancelled })); return
                }
                let pid = p.processIdentifier
                let timer = DispatchWorkItem {
                    guard p.isRunning else { return }
                    got.lock.withLock { got.timedOut = true }
                    p.terminate()
                    DispatchQueue.global().asyncAfter(deadline: .now() + 2) { if p.isRunning { kill(pid, SIGKILL) } }
                }
                DispatchQueue.global().asyncAfter(deadline: .now() + .seconds(timeout), execute: timer)
                DispatchQueue.global().async { try? inp.fileHandleForWriting.write(contentsOf: Data(input.utf8)); try? inp.fileHandleForWriting.close() }
                p.waitUntilExit(); timer.cancel()
                _ = g.wait(timeout: .now() + 2)
                out.fileHandleForReading.readabilityHandler = nil; err.fileHandleForReading.readabilityHandler = nil
                let (o, e, timedOut) = got.lock.withLock { (got.out, got.err, got.timedOut) }
                let called = running.lock.withLock { running.cancelled }
                k.resume(returning: ExecResult(code: timedOut || called ? nil : p.terminationStatus, stdout: String(decoding: o, as: UTF8.self),
                                               stderr: String(decoding: e, as: UTF8.self), timedOut: timedOut))
            }
        }
    }
}

/// Through herdr-core's `machine.exec` on this machine, as the Herdr app mod's helper runs it (argv, no shell): the
/// program runs with herdr-core's environment. Only for the app's own work; checks use `ProcessRunner` with a stub.
public struct HCExecRunner: SummaryRunner {
    let client: HCClient
    public init(client: HCClient) { self.client = client }

    /// What goes to `machine.exec`: the helper's own fields (summary.rs), and no environment of the app's.
    public static func payload(argv: [String], input: String, cwd: String, timeout: Int) -> [String: Any] {
        ["argv": argv, "input": Data(input.utf8).base64EncodedString(), "timeout": timeout, "cwd": cwd]
    }

    public func run(argv: [String], input: String, cwd: String, timeout: Int) async -> ExecResult {
        try? FileManager.default.createDirectory(atPath: cwd, withIntermediateDirectories: true)
        let p = HCJSON(Self.payload(argv: argv, input: input, cwd: cwd, timeout: timeout))
        do {
            let r = try await client.call("machine.exec", json: p, timeout: TimeInterval(timeout + 5))
            let dec = { (k: String) in String(decoding: Data(base64Encoded: r[k] as? String ?? "") ?? Data(), as: UTF8.self) }
            if r["timed_out"] as? Bool == true { return ExecResult(code: nil, stdout: dec("stdout"), timedOut: true) }
            return ExecResult(code: (r["code"] as? NSNumber).map { Int32(truncating: $0) }, stdout: dec("stdout"), stderr: dec("stderr"))
        } catch {
            let missing = "\(error)".contains("spawn_failed")
            return ExecResult(code: nil, stderr: missing ? "" : "herdr-core: \(error)", missing: missing)
        }
    }
}

extension Summary {
    /// Why no summary came: the helper's error codes.
    public enum Failure: Error, Sendable, Hashable {
        case unsupported(String)
        case none(String)
        case failed(String)
    }

    /// Asks the model once for the last turn in `items`. `kind`: the agent that wrote it; `settings`: the person's
    /// choice; `program`: another path for the program (checks). Each run of a model is counted in `store`.
    @MainActor
    public static func ask(items: [ConversationItem], kind: String, settings: Settings, runner: any SummaryRunner, store: SinceStore,
                           program: String? = nil, cwd: String = NSTemporaryDirectory() + "flockbook-summary",
                           now: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970 * 1000) }) async -> Result<(id: String, state: String, needs: String?, model: String), Failure> {
        // A model is the person's choice for one program; each agent's own program runs its default.
        let (by, model) = settings.by == "own" ? (kind, nil) : (settings.by, modelName(settings.model))
        guard let (args, named) = argv(by, program: program, model: model) else {
            return .failure(.unsupported("No summary by \(by) yet: Claude Code, Codex and pi can write one. To choose one: Settings, Summaries, Written by."))
        }
        guard let (id, text) = input(items, agent: kind) else { return .failure(.none("The agent wrote nothing in its last turn.")) }
        let sent = stdin(by, text)
        let began = Date()
        let r = await runner.run(argv: args, input: sent, cwd: cwd, timeout: timeoutS)
        let used = { (got: Int, ok: Bool) in
            store.noteRun(at: now(), by: by, model: named, sent: sent.unicodeScalars.count + (by == "codex" ? 0 : system.unicodeScalars.count),
                          got: got, ms: Int(Date().timeIntervalSince(began) * 1000), ok: ok)
        }
        if r.missing { return .failure(.failed("The program \(args[0]) was not found.")) }
        if r.timedOut { used(0, false); return .failure(.failed("The model did not answer in \(timeoutS) s.")) }
        guard r.code == 0 else {
            used(r.stdout.unicodeScalars.count, false)
            let why = (r.stderr.split(separator: "\n") + r.stdout.split(separator: "\n")).map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty }.map { Glance.cut($0, lineCap) } ?? ""
            return .failure(.failed("The model's program failed. \(why)".trimmingCharacters(in: .whitespaces)))
        }
        guard let (state, needs) = parse(r.stdout) else { used(r.stdout.unicodeScalars.count, false); return .failure(.failed("The model's answer had no STATE line.")) }
        used(r.stdout.unicodeScalars.count, true)
        return .success((id, state, needs, named))
    }
}

/// The end of a log file read once (the last `bytes`, from a line start), as the app parses it. For Summaries of
/// agents whose conversation is not on screen. Kept while the file's size and change time are the same.
public final class LogTails: @unchecked Sendable {
    private let lock = NSLock()
    private var kept: [String: (size: UInt64, modified: Date, items: [ConversationItem])] = [:]
    public init() {}

    public func items(_ url: URL, bytes: Int = 2_000_000) -> [ConversationItem]? {
        guard let a = try? FileManager.default.attributesOfItem(atPath: url.path),
              let size = (a[.size] as? NSNumber)?.uint64Value, let modified = a[.modificationDate] as? Date else { return nil }
        if let k = lock.withLock({ kept[url.path] }), k.size == size, k.modified == modified { return k.items }
        guard let fh = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? fh.close() }
        let from = size > UInt64(bytes) ? size - UInt64(bytes) : 0
        try? fh.seek(toOffset: from)
        guard let data = try? fh.readToEnd() else { return nil }
        // Each line with the place where it starts in the file, as the app's tailer feeds it: Codex rows take their
        // ids from it, and a kept summary must match the card's turn.
        var p = ConversationParser()
        var start = data.startIndex
        if from > 0 { guard let nl = data.firstIndex(of: 0x0A) else { return [] }; start = nl + 1 }
        while start < data.endIndex {
            let end = data[start...].firstIndex(of: 0x0A) ?? data.endIndex
            if end == data.endIndex { break }   // a line still being written: the tailer waits for its end too
            if let line = String(data: data[start..<end], encoding: .utf8) {
                p.feed(line: line, offset: from + UInt64(start - data.startIndex))
            }
            start = end + 1
        }
        let items = p.items()
        lock.withLock { kept[url.path] = (size, modified, items) }
        return items
    }
}
