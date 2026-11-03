import Foundation

/// hc terminals → the app's Workspace / Tab / Pane / Agent model. Read-only grouping (owner's option A):
/// the official Herdr app labels its terminals `herdr.space=<id>` and `herdr.tab=<id>`. One space = one Workspace,
/// one tab = one Tab, each terminal = one Pane. A terminal without labels joins its parent's group (sub-agents sit
/// under their parent); otherwise it goes to "Other terminals", one tab each.
///
/// Ids: workspace `hc:space:<id>` / `hc:other`, tab `hc:tab:<space>/<tab>` / `hc:term:<address>`, pane `hc:<address>`.
/// They never collide with TUI ids (`w1`, `w1:t1`, `w1:p1`).
public enum HCMapper {
    public static let panePrefix = "hc:"
    public static let otherId = "hc:other"
    public static let otherLabel = "Other terminals"

    /// Pane id for an hc terminal address, and back.
    public static func paneId(_ address: String) -> String { panePrefix + address }
    public static func address(fromPaneId id: String) -> String? { id.hasPrefix(panePrefix) ? String(id.dropFirst(panePrefix.count)) : nil }

    /// One terminal record, only the fields the app uses. Unknown or missing fields fall back to empty values.
    struct Term {
        var address: String
        var labels: [String: String]
        var argv: [String]
        var cwd: String
        var name: String?
        var title: String
        var parent: String?
        var pid: Int?
        var state: String
        var status: String?
        var statusKind: String?
        var statusMsg: String
        var fgName: String?
        var fgArgv: [String]
        var fgCwd: String?
        var fgPid: Int?
        var created: Int

        init?(_ o: [String: Any]) {
            guard let a = o["address"] as? String, !a.isEmpty else { return nil }
            address = a
            labels = (o["labels"] as? [String: Any])?.compactMapValues { $0 as? String } ?? [:]
            argv = o["argv"] as? [String] ?? []
            cwd = o["cwd"] as? String ?? ""
            name = (o["name"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            title = o["title"] as? String ?? ""
            parent = (o["parent"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            pid = (o["pid"] as? NSNumber)?.intValue
            state = o["state"] as? String ?? "live"
            let st = o["status"] as? [String: Any]
            status = st?["state"] as? String
            statusKind = st?["kind"] as? String
            statusMsg = st?["msg"] as? String ?? ""
            let fg = o["foreground"] as? [String: Any]
            fgName = fg?["name"] as? String
            fgArgv = fg?["argv"] as? [String] ?? []
            fgCwd = (fg?["cwd"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            fgPid = (fg?["pid"] as? NSNumber)?.intValue
            created = (o["created_ms"] as? NSNumber)?.intValue ?? 0
        }
    }

    /// Agent programs Flockbook knows, by executable name. Anything else with no status report is a plain shell pane.
    static let knownKinds: [String: String] = [
        "claude": "claude", "codex": "codex", "pi": "pi", "cursor-agent": "cursor", "cursor": "cursor",
        "gemini": "gemini", "opencode": "opencode", "amp": "amp", "grok": "grok",
    ]
    static let wrappers: Set<String> = ["node", "bun", "deno", "python", "python3", "env", "npx", "bunx"]

    /// Agent kind from the foreground process (what runs now), then the start argv. Interpreters (`node …/codex`)
    /// are looked through: their first non-flag argument names the program.
    static func kind(_ t: Term) -> String? {
        for argv in [[t.fgName ?? ""] + t.fgArgv, t.argv] where !argv.isEmpty {
            var words = argv.filter { !$0.isEmpty }.map { ($0 as NSString).lastPathComponent }
            if let first = words.first, wrappers.contains(first) || wrappers.contains(words.dropFirst().first ?? "") {
                words = words.filter { !$0.hasPrefix("-") && !wrappers.contains($0) }
            }
            for w in words.prefix(2) { if let k = knownKinds[w] { return k } }
        }
        return nil
    }

    /// hc reports only what the program said (OSC 7501; pi too, live 2026-11-02). No report yet = unknown, never idle.
    /// `error` has no app state: the turn ended, so it shows as done.
    static func status(_ word: String?) -> AgentStatus {
        switch word {
        case "idle": .idle
        case "working": .working
        case "blocked": .blocked
        case "done", "error": .done
        default: .unknown
        }
    }

    static func agent(_ t: Term) -> Agent? {
        let k = kind(t)
        guard k != nil || t.status != nil else { return nil }   // a shell that never reported: plain pane
        let kind = k ?? (t.fgName ?? t.argv.first.map { ($0 as NSString).lastPathComponent } ?? "agent")
        let name = t.name ?? kind
        var title = t.title.isEmpty ? name : t.title
        var statusKind = t.status == "blocked" ? t.statusKind : nil
        // `error` shows as done (the turn ended) but must read as a failure: the subtitle says so, with 7501's msg.
        if t.status == "error" {
            title = t.statusMsg.isEmpty ? "error" : "error: \(t.statusMsg)"
            statusKind = "error"
        }
        var a = Agent(name: name, kind: kind, status: status(t.status), sessionId: nil, title: title,
                      backend: .hc, statusKind: statusKind)
        if let pid = t.fgPid ?? t.pid, pid > 0 { a.process = "\(pid)@\(t.created)" }
        return a
    }

    static func pane(_ t: Term, title: String? = nil) -> Pane {
        var pids: [Int32] = []
        for p in [t.fgPid, t.pid].compactMap({ $0 }) where p > 0 && !pids.contains(Int32(p)) { pids.append(Int32(p)) }
        // Remote terminals (`host2/t7`) run on another machine: their pids mean nothing here.
        if t.address.contains("/") { pids = [] }
        return Pane(id: paneId(t.address), terminalId: t.address, cwd: t.fgCwd ?? t.cwd, agent: agent(t), label: title ?? t.name,
                    backend: .hc, processIds: pids)
    }

    static func folderName(_ path: String) -> String? {
        let p = (path as NSString).lastPathComponent
        return p.isEmpty || p == "/" ? nil : p
    }

    /// `terminals`: `terminal.list` records. `appDoc`: the `herdr-app` document value, any shape (parsed defensively).
    /// `machineDocs`: each other machine's `herdr-app` document (`doc.get {on}`), in the order to show them. Its ids
    /// are local to that machine; they are shown qualified (`box/sp1`), after this machine's (mirror.rs qualify).
    public static func workspaces(terminals: [[String: Any]], appDoc: Any?, machineDocs: [(String, Any?)] = []) -> [Workspace] {
        // Per machine prefix ("" here, "box/" there): names, and the document when it parses (space, tab and pane
        // order, Flockbook's pane names).
        var namesBy: [String: AppDocNames] = ["": AppDocNames(appDoc)]
        var docsBy: [String: HCAppDoc] = [:]
        if let d = try? HCAppDoc(value: appDoc) { docsBy[""] = d }
        for (m, v) in machineDocs {
            namesBy[m + "/"] = AppDocNames(v)
            if let d = try? HCAppDoc(value: v) { docsBy[m + "/"] = d }
        }
        /// (machine prefix, local id) of a qualified id.
        func local(_ id: String) -> (String, String) { let m = machinePrefix(id); return (m, String(id.dropFirst(m.count))) }
        let terms = terminals.compactMap(Term.init).filter { $0.state == "live" }
        let byAddr = Dictionary(terms.map { ($0.address, $0) }, uniquingKeysWith: { a, _ in a })

        /// (space, tab) for a terminal: where the document shows it, else its own labels, else the nearest such
        /// ancestor's. A space label without a tab: a tab of its own (`@t7`) until the official app's adoption gives
        /// it a fresh one.
        func group(_ t: Term) -> (String, String)? {
            var cur: Term? = t
            var seen = Set<String>()
            while let c = cur, seen.insert(c.address).inserted {
                // Labels are local to the terminal's machine: `box/sp1` is not the local `sp1`.
                let m = machinePrefix(c.address)
                let own = String(c.address.dropFirst(m.count))
                if let p = docsBy[m]?.place(ofTerminal: own) { return (m + p.space, m + p.tab) }   // its machine's document
                if let s = c.labels["herdr.space"], !s.isEmpty {
                    return (m + s, m + (c.labels["herdr.tab"].flatMap { $0.isEmpty ? nil : $0 } ?? "@" + own))
                }
                // A parent is named on the child's own machine: try `host2/t3` before a local `t3`.
                cur = c.parent.flatMap { p in byAddr[machinePrefix(c.address) + p] ?? byAddr[p] }
            }
            return nil
        }

        var spaceOrder: [String] = []
        var tabOrder: [String: [String]] = [:]
        var panes: [String: [Term]] = [:]       // key "space\u{0}tab" (qualified ids hold "/")
        var others: [Term] = []
        for t in terms {
            guard let (s, tb) = group(t) else { others.append(t); continue }
            if tabOrder[s] == nil { spaceOrder.append(s); tabOrder[s] = [] }
            if !(tabOrder[s]!.contains(tb)) { tabOrder[s]!.append(tb) }
            panes["\(s)\u{0}\(tb)", default: []].append(t)
        }

        /// `items` reordered by their place in `order`; items not in it keep their order, after the others.
        func ordered<T>(_ items: [T], by order: [String], key: (T) -> String) -> [T] {
            guard !order.isEmpty else { return items }
            let rank = Dictionary(order.enumerated().map { ($1, $0) }, uniquingKeysWith: { a, _ in a })
            return items.enumerated().sorted { a, b in
                let ra = rank[key(a.element)] ?? Int.max, rb = rank[key(b.element)] ?? Int.max
                return ra != rb ? ra < rb : a.offset < b.offset
            }.map(\.element)
        }
        if !docsBy.isEmpty {
            // This machine first, then the others in the order given.
            let prefixes = [""] + machineDocs.map { $0.0 + "/" }
            spaceOrder = ordered(spaceOrder, by: prefixes.flatMap { m in docsBy[m]?.spaceIds.map { m + $0 } ?? [] }) { $0 }
            for s in spaceOrder {
                let (m, own) = local(s)
                guard let d = docsBy[m] else { continue }
                tabOrder[s] = ordered(tabOrder[s] ?? [], by: d.tabIds(space: own).map { m + $0 }) { $0 }
            }
            for (k, ts) in panes {
                let (m, tb) = local(String(k[k.index(after: k.firstIndex(of: "\u{0}")!)...]))
                guard let d = docsBy[m] else { continue }
                panes[k] = ordered(ts, by: d.terminals(tab: tb).map { m + $0 }) { $0.address }
            }
        }

        func makeTab(id: String, label: String, number: Int, terms: [Term]) -> Tab {
            let ps = terms.map { t in let (m, own) = local(t.address); return pane(t, title: docsBy[m]?.paneTitle(terminal: own)) }
            return Tab(id: id, label: label, number: number, panes: ps,
                       status: AgentStatus.rollup(ps.compactMap { $0.agent?.status }), backend: .hc)
        }

        var out: [Workspace] = spaceOrder.map { s in
            let tabs = (tabOrder[s] ?? []).enumerated().map { i, tb in
                // An unnamed tab is called by its place in the document, as in Herdr (model.rs tab_name); one only
                // labeled, by its shown number (an id the official app made is no name).
                // The tab part is local to the space's machine: `hc:tab:box/sp1/tb1`.
                let (m, own) = local(tb)
                return makeTab(id: "hc:tab:\(s)/\(own)", label: namesBy[m]?.tab(own) ?? docsBy[m]?.tabPosition(own).map { "\($0 + 1)" }
                                   ?? (isFreshId(own) || own.hasPrefix("@") ? "\(i + 1)" : own), number: i + 1,
                               terms: panes["\(s)\u{0}\(tb)"] ?? [])
            }
            let first = panes["\(s)\u{0}\(tabOrder[s]?.first ?? "")"]?.first
            let (m, own) = local(s)
            let label = namesBy[m]?.space(own) ?? first.flatMap { folderName($0.fgCwd ?? $0.cwd) } ?? s
            return Workspace(id: "hc:space:\(s)", label: label, tabs: tabs, backend: .hc)
        }
        if !others.isEmpty {
            let tabs = others.enumerated().map { i, t in
                let label = t.name ?? (t.title.isEmpty ? nil : t.title) ?? folderName(t.fgCwd ?? t.cwd) ?? t.address
                return makeTab(id: "hc:term:\(t.address)", label: label, number: i + 1, terms: [t])
            }
            out.append(Workspace(id: otherId, label: otherLabel, tabs: tabs, backend: .hc))
        }
        return out
    }

    /// An id the official app (or Flockbook) made: a short prefix and 32 hex digits.
    static func isFreshId(_ id: String) -> Bool {
        ["sp", "tb", "p", "s"].contains { p in
            id.hasPrefix(p) && id.count == p.count + 32 && id.dropFirst(p.count).allSatisfy(\.isHexDigit)
        }
    }

    /// `host2/` for `host2/t7`, else "". A remote child's `parent` is local to its machine.
    static func machinePrefix(_ address: String) -> String {
        guard let i = address.lastIndex(of: "/") else { return "" }
        return String(address[...i])
    }
}

/// Space and tab display names out of the `herdr-app` document. Its format is private to the official app and may
/// change, so this never trusts a shape: it walks the whole value and takes any object with a string id (or an object
/// keyed by id) that carries a string `name`, `label` or `title`. An ancestor key containing "tab" files it under tabs,
/// one containing "space" under spaces, neither under both. Anything else is ignored; a miss means "use the fallback".
struct AppDocNames {
    private(set) var spaces: [String: String] = [:]
    private(set) var tabs: [String: String] = [:]

    init(_ doc: Any?) {
        guard let doc else { return }
        walk(doc, context: .none, keyAsId: nil, depth: 0)
    }

    func space(_ id: String) -> String? { spaces[id] }
    func tab(_ id: String) -> String? { tabs[id] }

    private enum Ctx { case none, space, tab }

    private static func displayName(_ o: [String: Any]) -> String? {
        for k in ["name", "label", "title"] { if let s = o[k] as? String, !s.isEmpty { return s } }
        return nil
    }

    private mutating func walk(_ v: Any, context: Ctx, keyAsId: String?, depth: Int) {
        guard depth < 12 else { return }
        if let o = v as? [String: Any] {
            if let id = (o["id"] as? String) ?? keyAsId, let n = Self.displayName(o) {
                switch context {
                case .space: if spaces[id] == nil { spaces[id] = n }
                case .tab: if tabs[id] == nil { tabs[id] = n }
                case .none: if spaces[id] == nil { spaces[id] = n }; if tabs[id] == nil { tabs[id] = n }
                }
            }
            for (k, child) in o {
                let lk = k.lowercased()
                let ctx: Ctx = lk.contains("tab") ? .tab : lk.contains("space") ? .space : context
                // A container keyed by id ("spaces": {"s1": {...}}): its children's keys are ids.
                let isContainer = lk.contains("tab") || lk.contains("space")
                if isContainer, let m = child as? [String: Any], m["id"] == nil, Self.displayName(m) == nil {
                    for (id, item) in m { walk(item, context: ctx, keyAsId: id, depth: depth + 2) }
                } else {
                    walk(child, context: ctx, keyAsId: nil, depth: depth + 1)
                }
            }
        } else if let a = v as? [Any] {
            for item in a { walk(item, context: context, keyAsId: nil, depth: depth + 1) }
        }
    }
}
