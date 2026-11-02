import Foundation

/// Layout writes for the app layer, the same calls on both backends. Ids are the backend's own (`HerdrClient`:
/// herdr ids; `HCClient`: `hc:space:<s>`, `hc:tab:<s>/<t>`, `hc:<address>`).
public protocol LayoutBackend: HerdrBackend {
    /// Returns the new workspace's id.
    func createWorkspace(label: String?, cwd: String?, focus: Bool) async throws -> String
    func renameWorkspace(id: String, to label: String) async throws
    func closeWorkspace(id: String) async throws
    func createTab(workspaceId: String, label: String?, cwd: String?) async throws
    func closeTab(_ tabId: String) async throws
    /// Final 0-based position among its workspace's tabs.
    func moveTab(_ tabId: String, toIndex: Int) async throws
    func renameTab(_ tabId: String, to label: String) async throws
    func split(paneId: String, direction: SplitDirection) async throws
    func renamePane(paneId: String, to label: String?) async throws
    func closePane(paneId: String) async throws
}

extension HerdrClient: LayoutBackend {}

/// hc layout: terminals run with `keep: true` and the `herdr.space` / `herdr.tab` labels, and the `herdr-app`
/// document holds spaces, tabs and splits as the official app writes them (`docs/research/hc-api/herdr-app-doc.md`).
/// When the official app runs on this core, its own `herdr-app do` commands are used instead.
extension HCClient: LayoutBackend {
    // MARK: - Ids

    /// Whether the app may offer layout writes for this hc workspace or tab id: spaces and tabs, here or on another
    /// machine (that machine's document; an op on one that is not connected and kept is refused then).
    public static func layoutWritable(id: String) -> Bool {
        if id.hasPrefix("hc:space:") { return (try? spaceLabel(id)) != nil }
        if id.hasPrefix("hc:tab:") { return (try? tabLabels(id)) != nil }
        return false
    }

    static func spaceLabel(_ workspaceId: String) throws -> String {
        guard workspaceId.hasPrefix("hc:space:"), case let s = String(workspaceId.dropFirst(9)), !s.isEmpty else {
            throw HerdrError.protocolError("not an hc space: \(workspaceId)")
        }
        return s
    }

    /// `hc:tab:<space>/<tab>`. A remote space is qualified (`box/sp1`) and its tab part is local to that machine, so the
    /// tab comes back qualified too (`box/tb1`).
    static func tabLabels(_ tabId: String) throws -> (space: String, tab: String) {
        guard tabId.hasPrefix("hc:tab:"), case let rest = tabId.dropFirst(7), let slash = rest.lastIndex(of: "/") else {
            throw HerdrError.protocolError("not an hc tab in a space: \(tabId)")
        }
        let s = String(rest[..<slash]), t = String(rest[rest.index(after: slash)...])
        guard !s.isEmpty, !t.isEmpty else { throw HerdrError.protocolError("not an hc tab in a space: \(tabId)") }
        return (s, HCMapper.machinePrefix(s) + t)
    }

    /// The machine a layout write goes to: this one (`prefix` "") or another (`box/`). Inside a write, ids and addresses
    /// are that machine's own (mirror.rs local); the official app and hc calls on terminals take them qualified.
    struct Machine: Equatable, Sendable {
        let prefix: String
        static let here = Machine(prefix: "")
        init(prefix: String) { self.prefix = prefix }
        init(of id: String) { prefix = HCMapper.machinePrefix(id) }
        /// `on` for doc and terminal.run calls; nil here.
        var on: String? { prefix.isEmpty ? nil : String(prefix.dropLast()) }
        func local(_ id: String) -> String { id.hasPrefix(prefix) ? String(id.dropFirst(prefix.count)) : id }
        func qualify(_ id: String) -> String { prefix + id }
    }

    /// The machine and its own space id of a workspace id.
    static func machineSpace(_ workspaceId: String) throws -> (Machine, String) {
        let s = try spaceLabel(workspaceId)
        let m = Machine(of: s)
        return (m, m.local(s))
    }

    /// The machine and its own space and tab ids of a tab id.
    static func machineTab(_ tabId: String) throws -> (Machine, String, String) {
        let t = try tabLabels(tabId)
        let m = Machine(of: t.space)
        return (m, m.local(t.space), m.local(t.tab))
    }

    /// Another machine's layout is written only when the official app would show it: connected and not temporary
    /// (window.rs documents_updated). Asked before anything is written.
    func requireShown(_ m: Machine) async throws {
        guard let on = m.on else { return }
        let list = try await call("machine.list")
        let ms = list["machines"] as? [[String: Any]] ?? []
        guard let rec = ms.first(where: { $0["name"] as? String == on }) else {
            throw HerdrError.server(code: "not_found", message: "no machine \(on)")
        }
        if rec["temporary"] as? Bool == true {
            throw HerdrError.server(code: "refused", message: "\(on) is a temporary machine: Herdr keeps no layout for it")
        }
        guard rec["state"] as? String == "connected" else {
            throw HerdrError.server(code: "unavailable", message: "\(on) is not connected (\(rec["state"] as? String ?? "?"))")
        }
    }

    // MARK: - Document

    /// `doc.get` → edit → `doc.set` with the version read. On `conflict` (someone wrote in between) it reads again and
    /// re-applies the edit, a few times. An unreadable document or an unknown shape throws `HCAppDoc.Refusal` and
    /// nothing is written. No write when the edit changed nothing.
    @discardableResult
    func editAppDoc<T>(on m: Machine = .here, _ edit: (inout HCAppDoc) throws -> T) async throws -> T {
        var attempt = 0
        while true {
            let (doc0, version) = try await readAppDoc(on: m)
            var doc = doc0
            let out = try edit(&doc)
            if NSDictionary(dictionary: doc.value).isEqual(to: doc0.value) { return out }
            do {
                var p: [String: Any] = ["name": Self.appDocName, "version": version, "value": doc.value]
                if let on = m.on { p["on"] = on }
                _ = try await call("doc.set", json: HCJSON(p))
                markDocStale()
                await writeWhere(doc, on: m)
                return out
            } catch HerdrError.server(code: "conflict", _) where attempt < 4 {
                attempt += 1
            }
        }
    }

    /// The `where` labels to write (address → `<space> / <tab>`), as the official app does after each change
    /// (window.rs write_where): every live terminal the document shows, a sub-agent its nearest shown ancestor's, and
    /// only where the label differs. Hc shows it in its picker and `ls`; the ids stay what groups.
    /// `tops`: a folder → its git checkout top (an unnamed space is called after its first terminal's, else that
    /// terminal's folder; window.rs folder_info).
    static func whereWrites(_ doc: HCAppDoc, terms: [[String: Any]], home: String, tops: [String: String] = [:]) -> [String: String] {
        var byAddr: [String: [String: Any]] = [:]
        // Only this machine's terminals: a remote one's parent names a terminal on its own machine.
        for t in terms { if let a = t["address"] as? String, !a.contains("/") { byAddr[a] = t } }
        var want = doc.wherePlaces(home: home) { a in byAddr[a].flatMap(Self.cwd).map { tops[$0] ?? $0 } }
        for a in byAddr.keys where want[a] == nil {
            var cur = a
            for _ in 0..<32 {
                guard let p = byAddr[cur]?["parent"] as? String else { break }
                if let place = want[p] { want[a] = place; break }
                cur = p
            }
        }
        return want.filter { a, place in
            guard let t = byAddr[a] else { return false }
            return (t["labels"] as? [String: Any])?["where"] as? String != place
        }
    }

    /// `git -C folder rev-parse --show-toplevel`; nil outside a checkout, when git fails or takes over 3 s.
    public static func gitTopLevel(_ folder: String) -> String? {
        let p = Process(), out = Pipe()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        p.arguments = ["-C", folder, "rev-parse", "--show-toplevel"]
        p.standardOutput = out
        p.standardError = FileHandle.nullDevice
        p.standardInput = FileHandle.nullDevice
        let done = DispatchSemaphore(value: 0)
        p.terminationHandler = { _ in done.signal() }
        do { try p.run() } catch { return nil }
        if done.wait(timeout: .now() + 3) == .timedOut { p.terminate(); return nil }
        guard p.terminationStatus == 0 else { return nil }
        let top = String(decoding: out.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return top.isEmpty ? nil : top
    }

    /// Writes the changed `where` labels. A failed write is left for the next change, as the official app does.
    /// On another machine its own home and git (run there); with no home known there, nothing is written.
    func writeWhere(_ doc: HCAppDoc, on m: Machine = .here) async {
        guard let terms = try? await liveTerminals(on: m), let home = await machineHome(m) else { return }
        // Git is asked only for the folders that name a space: each unnamed space's first live terminal, not home.
        var cwds: [String: String] = [:]
        for t in terms { if let a = t["address"] as? String, !a.contains("/"), let c = Self.cwd(t) { cwds[a] = c } }
        var ask: [String] = []
        for a in doc.unnamedSpaceLeads() {
            guard let c = cwds[a], !ask.contains(c), HCAppDoc.folderName(c, home: home) != "~" else { continue }
            ask.append(c)
        }
        var tops: [String: String] = [:]
        if let on = m.on {
            if !ask.isEmpty, let out = try? await exec(on: on, "for d in " + ask.map(Self.shellQuote).joined(separator: " ")
                + #"; do printf '%s\t' "$d"; git -C "$d" rev-parse --show-toplevel 2>/dev/null || echo; done"#) {
                for l in out.split(separator: "\n") {
                    let p = l.split(separator: "\t", maxSplits: 1).map(String.init)
                    if p.count == 2, ask.contains(p[0]), p[1].hasPrefix("/") { tops[p[0]] = p[1] }
                }
            }
        } else {
            let gitTop = self.gitTop
            for c in ask { if let top = await Task.detached(operation: { gitTop(c) }).value { tops[c] = top } }
        }
        for (a, place) in Self.whereWrites(doc, terms: terms, home: home, tops: tops).sorted(by: { $0.key < $1.key }) {
            _ = try? await call("terminal.label", ["terminal": m.qualify(a), "labels": ["where": place]])
        }
    }

    /// That machine's home: this Mac's here; there `$HOME`, asked once. Nil when it cannot be asked.
    func machineHome(_ m: Machine) async -> String? {
        guard let on = m.on else { return NSHomeDirectory() }
        if let h = machineHomes[on] { return h }
        guard let out = try? await exec(on: on, #"printf '%s\n' "$HOME""#),
              let h = out.split(separator: "\n").first.map(String.init), h.hasPrefix("/") else { return nil }
        machineHomes[on] = h
        return h
    }

    /// The document and its version. Throws `HCAppDoc.Refusal` for a shape Flockbook does not know.
    func readAppDoc(on m: Machine = .here) async throws -> (HCAppDoc, NSNumber) {
        var p: [String: Any] = ["name": Self.appDocName]
        if let on = m.on { p["on"] = on }
        let r = try await call("doc.get", json: HCJSON(p))
        guard let version = r["version"] as? NSNumber, CFGetTypeID(version) != CFBooleanGetTypeID() else {
            throw HCAppDoc.Refusal(reason: "the document has no version")
        }
        return (try HCAppDoc(value: r["value"]), version)
    }

    /// Live terminal records.
    func liveTerminals() async throws -> [[String: Any]] {
        let list = try await call("terminal.list")
        return (list["terminals"] as? [[String: Any]] ?? []).filter { $0["state"] as? String == "live" }
    }

    /// Live terminals of one machine, with that machine's own addresses (`box/t3` → `t3`; a `parent` already is).
    func liveTerminals(on m: Machine) async throws -> [[String: Any]] {
        try await liveTerminals().compactMap { t in
            guard let a = t["address"] as? String, HCMapper.machinePrefix(a) == m.prefix else { return nil }
            var o = t
            o["address"] = m.local(a)
            return o
        }
    }

    /// `herdr.space` and `herdr.tab`; nil without a space. A space without a tab gets a fresh tab on adoption.
    static func labels(_ t: [String: Any]) -> (space: String, tab: String?)? {
        let l = t["labels"] as? [String: Any] ?? [:]
        guard let s = l["herdr.space"] as? String, !s.isEmpty else { return nil }
        return (s, (l["herdr.tab"] as? String).flatMap { $0.isEmpty ? nil : $0 })
    }

    /// The document tab for a tab id part: `@t7` (a terminal labeled with a space only, shown as its own tab) is the
    /// tab the document shows t7 in once adopted.
    static func docTab(_ doc: HCAppDoc, _ tb: String) -> String {
        guard tb.hasPrefix("@") else { return tb }
        return doc.place(ofTerminal: String(tb.dropFirst()))?.tab ?? tb
    }

    static func cwd(_ t: [String: Any]) -> String? {
        ((t["foreground"] as? [String: Any])?["cwd"] as? String) ?? (t["cwd"] as? String)
    }

    /// Adopts every live terminal labeled for `space` that no pane shows (what the official app does on each listing),
    /// so the document matches what Flockbook shows before it is changed. Local terminals only. One without a tab
    /// label gets a new tab with a fresh id, as the official app does (window.rs `adopt`).
    static func adoptSpace(_ doc: inout HCAppDoc, _ space: String, terms: [[String: Any]], home: String? = NSHomeDirectory()) {
        var fresh: [String: String] = [:]
        adoptSpace(&doc, space, terms: terms, fresh: &fresh, home: home)
    }

    /// `fresh`: the new tab id per terminal, kept across calls so a probe and the write that follows agree.
    /// `home`: that machine's, for a terminal with no cwd.
    static func adoptSpace(_ doc: inout HCAppDoc, _ space: String, terms: [[String: Any]], fresh: inout [String: String],
                           home: String? = NSHomeDirectory()) {
        for t in terms {
            guard let a = t["address"] as? String, !a.contains("/"), let (s, tb) = labels(t), s == space else { continue }
            let tab = tb ?? fresh[a] ?? HCAppDoc.freshId("tb")
            if tb == nil { fresh[a] = tab }
            doc.adopt(terminal: a, space: s, tab: tab, folder: cwd(t) ?? home ?? doc.spaceFolder(s) ?? "")
        }
    }

    /// `terminal.run` the way the official app starts a pane's terminal: the core picks the shell (empty argv), kept,
    /// labeled for its space and tab. Returns the address.
    /// Returns the machine's own address.
    func runPaneTerminal(space: String, tab: String, cwd: String?, on m: Machine = .here) async throws -> String {
        var p: [String: Any] = ["argv": [String](), "keep": true,
                                "env": ["TERM": "xterm-256color", "COLORTERM": "truecolor", "TERM_PROGRAM": "herdr"],
                                "labels": ["herdr.space": space, "herdr.tab": tab]]
        if let cwd, !cwd.isEmpty { p["cwd"] = cwd }
        if let on = m.on { p["on"] = on }
        let r = try await call("terminal.run", json: HCJSON(p))
        guard let a = r["address"] as? String, !a.isEmpty else { throw HerdrError.protocolError("terminal.run returned no address") }
        return m.local(a)
    }

    /// Ends terminals and their sub-agents all the way down, as the official app does.
    /// Every kill is tried; the ones that failed are reported after (the layout already changed: such a kept, labeled
    /// terminal would come back as an unnamed tab).
    /// `addresses` and `terms`: the machine's own.
    func endTerminals(_ addresses: [String], terms: [[String: Any]], on m: Machine = .here) async throws {
        var failed: [String] = []
        for a in Self.doomed(addresses, terms: terms).map(m.qualify) {
            do { _ = try await call("terminal.kill", ["terminal": a]) } catch { failed.append("\(a) (\(error))") }
        }
        if !failed.isEmpty {
            throw HerdrError.server(code: "kill_failed", message: "closed in the layout, but still running: \(failed.joined(separator: ", "))")
        }
    }

    /// Ends a terminal just started whose document write failed, so no kept, labeled shell is left to come back as an
    /// unnamed tab. The caller throws the write's error.
    func abandon(_ address: String, on m: Machine = .here) async { _ = try? await call("terminal.kill", ["terminal": m.qualify(address)]) }

    /// `addresses` and their unlabeled sub-agents, recursively. A sub-agent with its own `herdr.space` label (kept as a
    /// tab) is the person's and is spared. A child's `parent` names a terminal on the child's own machine
    /// (`host2/t9` with parent `t3` is under `host2/t3`).
    static func doomed(_ addresses: [String], terms: [[String: Any]]) -> [String] {
        var doomed = addresses
        var i = 0
        while i < doomed.count {
            for t in terms {
                guard let a = t["address"] as? String, let p = t["parent"] as? String, Self.labels(t) == nil, !doomed.contains(a),
                      HCMapper.machinePrefix(a) + p == doomed[i] else { continue }
                doomed.append(a)
            }
            i += 1
        }
        return doomed
    }

    // MARK: - The official app's commands

    /// `herdr-app do` when the official app runs on this same core; nil otherwise (then the document is written).
    func officialApp() async -> HerdrAppCLI? {
        let found = appCLI ?? (session == HCSession.defaultSession ? HerdrAppCLI.find() : nil)
        guard let cli = found, await cli.isRunning() else { return nil }
        return cli
    }

    // MARK: - LayoutBackend

    public func createWorkspace(label: String?, cwd: String?, focus: Bool) async throws -> String {
        let name = label?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let folder = cwd.flatMap { $0.isEmpty ? nil : $0 } ?? NSHomeDirectory()
        if let app = await officialApp() {
            let r = try await app.do("space.new", ["name": name, "folder": folder])
            guard let s = r["space"] as? String else { throw HerdrError.protocolError("herdr-app space.new gave no space") }
            return "hc:space:\(s)"
        }
        _ = try await readAppDoc()   // refuse before starting anything
        let s = HCAppDoc.freshId("sp"), tb = HCAppDoc.freshId("tb")
        let addr = try await runPaneTerminal(space: s, tab: tb, cwd: folder)
        do { try await editAppDoc { $0.addSpace(id: s, name: name, folder: folder, tab: tb, terminal: addr) } }
        catch { await abandon(addr); throw error }
        return "hc:space:\(s)"
    }

    public func renameWorkspace(id: String, to label: String) async throws {
        let (m, s) = try Self.machineSpace(id)
        try await requireShown(m)
        if let app = await officialApp() { _ = try await app.do("space.rename", ["id": m.qualify(s), "name": label]); return }
        let terms = try await liveTerminals(on: m)
        let mhome = await machineHome(m)
        try await editAppDoc(on: m) { doc in
            Self.adoptSpace(&doc, s, terms: terms, home: mhome)
            guard doc.renameSpace(s, to: label) else { throw HerdrError.server(code: "not_found", message: "no space \(s) in the layout") }
        }
    }

    public func closeWorkspace(id: String) async throws {
        let (m, s) = try Self.machineSpace(id)
        try await requireShown(m)
        if let app = await officialApp() { _ = try await app.do("space.close", ["id": m.qualify(s), "confirmed": true]); return }
        let terms = try await liveTerminals(on: m)
        let mhome = await machineHome(m)
        let shown = try await editAppDoc(on: m) { doc -> [String] in
            Self.adoptSpace(&doc, s, terms: terms, home: mhome)
            return doc.removeSpace(s)
        }
        // Only what left the tree ends (every labeled terminal of the space was adopted into it first).
        try await endTerminals(shown, terms: terms, on: m)
    }

    public func createTab(workspaceId: String, label: String?, cwd: String?) async throws {
        let (m, s) = try Self.machineSpace(workspaceId)
        try await requireShown(m)
        let name = label?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        if let app = await officialApp() {
            var a: [String: Any] = ["space": m.qualify(s)]
            if !name.isEmpty { a["name"] = name }
            if let cwd, !cwd.isEmpty { a["folder"] = cwd }
            _ = try await app.do("tab.new", a)
            return
        }
        let terms = try await liveTerminals(on: m)
        let mhome = await machineHome(m)
        let doc0 = try await readAppDoc(on: m).0
        let tb = HCAppDoc.freshId("tb")
        // Another machine's home is not this one's: its space folder, else its own home.
        let folder = cwd.flatMap { $0.isEmpty ? nil : $0 } ?? (m.on == nil ? NSHomeDirectory() : doc0.spaceFolder(s) ?? mhome)
        let addr = try await runPaneTerminal(space: s, tab: tb, cwd: folder, on: m)
        do {
            try await editAppDoc(on: m) { doc in
                Self.adoptSpace(&doc, s, terms: terms, home: mhome)
                doc.addTab(space: s, folder: folder ?? "", id: tb, name: name, terminal: addr)
            }
        } catch { await abandon(addr, on: m); throw error }
    }

    public func closeTab(_ tabId: String) async throws {
        let (m, s, tb) = try Self.machineTab(tabId)
        try await requireShown(m)
        if let app = await officialApp() {
            _ = try await app.do("tab.close", ["id": m.qualify(try await officialTab(tb, on: m)), "confirmed": true])
            return
        }
        let terms = try await liveTerminals(on: m)
        let mhome = await machineHome(m)
        let shown = try await editAppDoc(on: m) { doc -> [String] in
            Self.adoptSpace(&doc, s, terms: terms, home: mhome)
            return doc.removeTab(Self.docTab(doc, tb)).terminals
        }
        // Only what left the tree ends: a terminal labeled for this tab but shown in another stays.
        try await endTerminals(shown, terms: terms, on: m)
    }

    /// `toIndex`: the place among the tabs Flockbook shows (those with live terminals). The document also holds mod-only
    /// and empty tabs, so the move goes next to the shown neighbour there.
    public func moveTab(_ tabId: String, toIndex: Int) async throws {
        let (m, s, tb) = try Self.machineTab(tabId)
        try await requireShown(m)
        let terms = try await liveTerminals(on: m)
        let mhome = await machineHome(m)
        let live = Set(terms.compactMap { $0["address"] as? String })
        if let app = await officialApp() {
            let doc = try await readAppDoc(on: m).0
            let id = Self.docTab(doc, tb)
            guard let place = doc.movePlace(tab: id, toShown: toIndex, live: live) else { return }
            _ = try await app.do("tab.move", ["id": m.qualify(id), "index": place + 1])
            return
        }
        try await editAppDoc(on: m) { doc in
            Self.adoptSpace(&doc, s, terms: terms, home: mhome)
            let id = Self.docTab(doc, tb)
            guard doc.tabPosition(id) != nil else { throw HerdrError.server(code: "not_found", message: "no tab \(tb) in the layout") }
            if let place = doc.movePlace(tab: id, toShown: toIndex, live: live) { doc.moveTab(id, to: place) }
        }
    }

    public func renameTab(_ tabId: String, to label: String) async throws {
        let (m, s, tb) = try Self.machineTab(tabId)
        try await requireShown(m)
        if let app = await officialApp() {
            _ = try await app.do("tab.rename", ["id": m.qualify(try await officialTab(tb, on: m)), "name": label])
            return
        }
        let terms = try await liveTerminals(on: m)
        let mhome = await machineHome(m)
        try await editAppDoc(on: m) { doc in
            Self.adoptSpace(&doc, s, terms: terms, home: mhome)
            guard doc.renameTab(Self.docTab(doc, tb), to: label) else { throw HerdrError.server(code: "not_found", message: "no tab \(tb) in the layout") }
        }
    }

    static func notClosable(_ a: String) -> Error { HerdrError.server(code: "refused", message: "the pane showing \(a) is marked not closable") }

    static func appOwns(_ what: String) -> Error {
        HerdrError.server(code: "refused", message: "the official Herdr app is running and has no command for \(what); change it there")
    }

    /// The pane id the running official app knows for a terminal; refused when it shows none (a sub-agent): writing
    /// the document behind the app's back is not done.
    static func officialPane(_ doc: HCAppDoc, _ a: String) throws -> String {
        guard let pid = doc.paneId(terminal: a) else { throw appOwns("\(a), which has no pane") }
        return pid
    }

    /// The tab id the running official app knows for a tab id part (`@t7` → the tab it adopted t7 into).
    private func officialTab(_ tb: String, on m: Machine) async throws -> String { Self.docTab(try await readAppDoc(on: m).0, tb) }

    /// The pane's machine, its terminal record, its labeled self-or-ancestor (`home`) and that one's labels, with that
    /// machine's own addresses; `terms` are that machine's live terminals.
    private func placed(_ paneId: String) async throws -> (m: Machine, terms: [[String: Any]], address: String, record: [String: Any], home: String, space: String, tab: String?) {
        let q = try address(paneId)
        let m = Machine(of: q), a = m.local(q)
        try await requireShown(m)
        let terms = try await liveTerminals(on: m)
        guard let rec = terms.first(where: { $0["address"] as? String == a }) else { throw HerdrError.server(code: "not_found", message: "no live terminal \(q)") }
        var cur: [String: Any]? = rec
        var seen = Set<String>()
        while let c = cur, let ca = c["address"] as? String, seen.insert(ca).inserted {
            if let (s, tb) = Self.labels(c) { return (m, terms, a, rec, ca, s, tb) }
            cur = (c["parent"] as? String).flatMap { p in terms.first { $0["address"] as? String == p } }
        }
        throw HerdrError.protocolError("\(q) is not in a space")
    }

    public func split(paneId: String, direction: SplitDirection) async throws {
        let (m, terms, a, rec, home, s0, _) = try await placed(paneId)
        let mhome = await machineHome(m)
        if let app = await officialApp() {
            let pid = try await Self.officialPane(readAppDoc(on: m).0, a)
            _ = try await app.do("pane.split", ["direction": direction == .down ? "down" : "right", "pane": m.qualify(pid)])
            return
        }
        // The new terminal is labeled with the document tab it goes into (after adoption), as the official app does.
        var probe = try await readAppDoc(on: m).0
        var fresh: [String: String] = [:]
        Self.adoptSpace(&probe, s0, terms: terms, fresh: &fresh, home: mhome)
        guard let (s, tb) = probe.place(ofTerminal: a) ?? probe.place(ofTerminal: home) else {
            throw HerdrError.server(code: "not_found", message: "\(a) has no pane in the layout")
        }
        let addr = try await runPaneTerminal(space: s, tab: tb, cwd: Self.cwd(rec), on: m)
        do {
            try await editAppDoc(on: m) { doc in
                Self.adoptSpace(&doc, s0, terms: terms, fresh: &fresh, home: mhome)
                // A sub-agent has no pane of its own: the new one goes right of its tab's last pane (adoption).
                if !doc.split(target: a, down: direction == .down, terminal: addr) {
                    doc.adopt(terminal: addr, space: s, tab: tb, folder: Self.cwd(rec) ?? mhome ?? "")
                }
            }
        } catch { await abandon(addr, on: m); throw error }
    }

    /// The official app has no pane names: Flockbook keeps its own in the pane node's `title`, which the official app
    /// reads and writes back unchanged.
    /// While the official app runs it owns the document and has no pane rename: refused then.
    public func renamePane(paneId: String, to label: String?) async throws {
        let (m, terms, a, _, _, s, _) = try await placed(paneId)
        let mhome = await machineHome(m)
        if await officialApp() != nil { throw Self.appOwns("pane names") }
        try await editAppDoc(on: m) { doc in
            Self.adoptSpace(&doc, s, terms: terms, home: mhome)
            guard doc.setPaneTitle(terminal: a, label) else { throw HerdrError.server(code: "not_found", message: "\(a) has no pane in the layout") }
        }
    }

    public func closePane(paneId: String) async throws {
        let (m, terms, a, _, _, s, _) = try await placed(paneId)
        let mhome = await machineHome(m)
        if let app = await officialApp() {
            let doc = try await readAppDoc(on: m).0
            let pid = try Self.officialPane(doc, a)
            guard doc.paneClosable(terminal: a) else { throw Self.notClosable(a) }
            _ = try await app.do("pane.close", ["pane": m.qualify(pid)])
            return
        }
        try await editAppDoc(on: m) { doc in
            Self.adoptSpace(&doc, s, terms: terms, home: mhome)
            guard doc.paneClosable(terminal: a) else { throw Self.notClosable(a) }
            doc.removePane(terminal: a)
        }
        try await endTerminals([a], terms: terms, on: m)
    }
}

/// The official app's command line (`herdr-app do <command> '<JSON object>'`). It prints a JSON object, or
/// `{"error": …}` and exits 1.
public struct HerdrAppCLI: Sendable {
    public let path: String
    public init(path: String) { self.path = path }

    /// `herdr-app` on PATH or in the app bundle.
    public static func find(environment env: [String: String] = ProcessInfo.processInfo.environment) -> HerdrAppCLI? {
        let dirs = (env["PATH"] ?? "").split(separator: ":").map(String.init)
        let candidates = dirs.map { "\($0)/herdr-app" } + ["/Applications/Herdr.app/Contents/MacOS/herdr-app"]
        return candidates.first { FileManager.default.isExecutableFile(atPath: $0) }.map(HerdrAppCLI.init)
    }

    /// `herdr-app status` succeeds only while the window runs.
    public func isRunning() async -> Bool { (try? await run(["status"]))?.status == 0 }

    /// The arguments go as one JSON object: `--k v` values are JSON-parsed there (control.rs `parse_args`), so a name
    /// like "2" or "true" would not stay a string.
    public func `do`(_ command: String, _ args: [String: Any]) async throws -> [String: Any] {
        let obj = try JSONSerialization.data(withJSONObject: args, options: [.sortedKeys, .withoutEscapingSlashes])
        let argv = ["do", command, String(decoding: obj, as: UTF8.self)]
        let (status, out) = try await run(argv)
        let o = (try? JSONSerialization.jsonObject(with: out)) as? [String: Any] ?? [:]
        if let e = o["error"] as? String { throw HerdrError.server(code: "herdr-app", message: e) }
        guard status == 0 else { throw HerdrError.server(code: "herdr-app", message: "\(command) exited \(status)") }
        return o
    }

    func run(_ argv: [String], timeout: TimeInterval = 15) async throws -> (status: Int32, out: Data) {
        let path = path
        return try await Task.detached {
            let p = Process()
            p.executableURL = URL(fileURLWithPath: path)
            p.arguments = argv
            let pipe = Pipe()
            p.standardOutput = pipe
            p.standardError = FileHandle.nullDevice
            try p.run()
            let killer = DispatchWorkItem { if p.isRunning { p.terminate() } }
            DispatchQueue.global().asyncAfter(deadline: .now() + timeout, execute: killer)
            let out = pipe.fileHandleForReading.readDataToEndOfFile()
            p.waitUntilExit()
            killer.cancel()
            return (p.terminationStatus, out)
        }.value
    }
}
