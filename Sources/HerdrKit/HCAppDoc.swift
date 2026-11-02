import Foundation

/// The official app's layout document (`herdr-app`, one per machine core), edited the way the official app edits it
/// (`docs/research/hc-api/herdr-app-doc.md`). It is kept as the raw JSON that was read: every field Flockbook does not
/// change stays exactly as read. A shape it does not know is refused at parse time, so nothing is ever written over it.
public struct HCAppDoc: @unchecked Sendable {
    /// Why a document cannot be edited. Nothing is written when this is thrown.
    public struct Refusal: Error, CustomStringConvertible, Equatable {
        public let reason: String
        public var description: String { "herdr-app layout not changed: \(reason)" }
    }

    /// Top-level fields other than `spaces`, kept as read.
    private var top: [String: Any]
    private(set) var spaces: [[String: Any]]

    static let nodeTypes: Set<String> = ["block", "row", "column", "stack"]

    /// `value` as `doc.get` returned it (`nil`/`NSNull`: no document yet).
    public init(value: Any?) throws {
        guard let value, !(value is NSNull) else { top = [:]; spaces = []; return }
        guard let o = value as? [String: Any] else { throw Refusal(reason: "the document is not an object") }
        top = o
        top["spaces"] = nil
        switch o["spaces"] {
        case nil, is NSNull: spaces = []
        case let a as [Any]:
            spaces = try a.enumerated().map { i, s in
                guard let s = s as? [String: Any] else { throw Refusal(reason: "space \(i) is not an object") }
                try Self.checkSpace(s, at: "space \(i)")
                return s
            }
        default: throw Refusal(reason: "`spaces` is not a list")
        }
    }

    /// The JSON to send with `doc.set`.
    public var value: [String: Any] {
        var o = top
        o["spaces"] = spaces
        return o
    }

    // MARK: - Shape check

    private static func checkString(_ o: [String: Any], _ k: String, at path: String) throws {
        if let v = o[k], !(v is String) { throw Refusal(reason: "\(path).\(k) is not a string") }
    }

    private static func checkSpace(_ s: [String: Any], at path: String) throws {
        for k in ["id", "name", "folder"] { try checkString(s, k, at: path) }
        guard let tabs = s["tabs"] else { return }
        guard let a = tabs as? [Any] else { throw Refusal(reason: "\(path).tabs is not a list") }
        for (i, t) in a.enumerated() {
            guard let t = t as? [String: Any] else { throw Refusal(reason: "\(path).tab \(i) is not an object") }
            for k in ["id", "name"] { try checkString(t, k, at: "\(path).tab \(i)") }
            if let r = t["root"] { try checkNode(r, at: "\(path).tab \(i).root", depth: 0) }
        }
    }

    private static func checkNode(_ v: Any, at path: String, depth: Int) throws {
        guard depth < 64 else { throw Refusal(reason: "\(path) is nested too deep") }
        guard let n = v as? [String: Any] else { throw Refusal(reason: "\(path) is not an object") }
        try checkString(n, "id", at: path)
        guard let type = n["type"] as? String, nodeTypes.contains(type) else {
            throw Refusal(reason: "\(path) has an unknown type \(String(describing: n["type"] ?? "none"))")
        }
        if type == "block" {
            guard n["content"] is [String: Any] else { throw Refusal(reason: "\(path).content is not an object") }
            if let c = n["content"] as? [String: Any], let t = c["terminal"], !(t is String) { throw Refusal(reason: "\(path).content.terminal is not a string") }
        } else {
            guard let ch = n["children"] as? [Any] else { throw Refusal(reason: "\(path).children is not a list") }
            for (i, c) in ch.enumerated() { try checkNode(c, at: "\(path).\(i)", depth: depth + 1) }
        }
        if let s = n["size"] {
            guard let s = s as? [String: Any], s["px"] is NSNumber || s["weight"] is NSNumber else { throw Refusal(reason: "\(path).size is not {px} or {weight}") }
        }
    }

    // MARK: - Reading

    public var spaceIds: [String] { spaces.compactMap { $0["id"] as? String } }

    func spaceIndex(_ id: String) -> Int? { spaces.firstIndex { $0["id"] as? String == id } }

    private func tabs(_ si: Int) -> [[String: Any]] { spaces[si]["tabs"] as? [[String: Any]] ?? [] }

    public func tabIds(space: String) -> [String] {
        guard let si = spaceIndex(space) else { return [] }
        return tabs(si).compactMap { $0["id"] as? String }
    }

    func tabIndex(_ id: String) -> (space: Int, tab: Int)? {
        for si in spaces.indices { if let ti = tabs(si).firstIndex(where: { $0["id"] as? String == id }) { return (si, ti) } }
        return nil
    }

    /// The folder a space's new terminals start in; nil when unset.
    public func spaceFolder(_ id: String) -> String? {
        spaceIndex(id).flatMap { spaces[$0]["folder"] as? String }.flatMap { $0.isEmpty ? nil : $0 }
    }

    public func spaceName(_ id: String) -> String? { spaceIndex(id).flatMap { spaces[$0]["name"] as? String } }

    /// The tab's 0-based place among its space's tabs (mod-only and empty ones count).
    public func tabPosition(_ id: String) -> Int? { tabIndex(id)?.tab }

    /// The final 0-based document place for moving `tab` to `shownIndex` among the tabs Flockbook shows (those with a
    /// terminal in `live`): before the shown tab now at that place, or right after the last shown one.
    public func movePlace(tab: String, toShown shownIndex: Int, live: Set<String>) -> Int? {
        guard let (si, _) = tabIndex(tab) else { return nil }
        let others = tabs(si).compactMap { $0["id"] as? String }.filter { $0 != tab }
        let shown = others.filter { id in terminals(tab: id).contains(where: live.contains) }
        guard !shown.isEmpty else { return nil }
        let i = max(shownIndex, 0)
        if i < shown.count { return others.firstIndex(of: shown[i]) }
        return others.firstIndex(of: shown[shown.count - 1]).map { $0 + 1 }
    }

    public func tabName(_ id: String) -> String? { tabIndex(id).flatMap { tabs($0.space)[$0.tab]["name"] as? String } }

    /// Terminal addresses of a tab's panes, in tree order (the official app's pane order).
    public func terminals(tab: String) -> [String] {
        guard let (si, ti) = tabIndex(tab), let root = tabs(si)[ti]["root"] as? [String: Any] else { return [] }
        return Self.blocks(root).compactMap(Self.terminal)
    }

    /// The tab (space, tab) whose tree shows this terminal.
    public func place(ofTerminal address: String) -> (space: String, tab: String)? {
        for si in spaces.indices {
            for t in tabs(si) {
                if let root = t["root"] as? [String: Any], Self.blocks(root).contains(where: { Self.terminal($0) == address }) {
                    return (spaces[si]["id"] as? String ?? "", t["id"] as? String ?? "")
                }
            }
        }
        return nil
    }

    public func showsTerminal(_ address: String) -> Bool { place(ofTerminal: address) != nil }

    /// Each shown terminal's place in words, `<space> / <tab>` (model.rs space_name / tab_name): a space without a
    /// name is called after its first terminal's folder (`folder(address)`), else its own folder; a tab without a name
    /// by its place (1, 2, ...).
    public func wherePlaces(home: String, folder: (String) -> String?) -> [String: String] {
        var out: [String: String] = [:]
        for si in spaces.indices {
            let ts = tabs(si)
            let roots = ts.map { $0["root"] as? [String: Any] ?? [:] }
            var space = spaces[si]["name"] as? String ?? ""
            if space.isEmpty {
                let first = roots.lazy.flatMap { Self.blocks($0).compactMap(Self.terminal) }.first
                space = Self.folderName(first.flatMap(folder) ?? spaces[si]["folder"] as? String ?? "", home: home)
            }
            for (ti, t) in ts.enumerated() {
                let named = t["name"] as? String ?? ""
                let place = "\(space) / \(named.isEmpty ? String(ti + 1) : named)"
                for a in Self.blocks(roots[ti]).compactMap(Self.terminal) { out[a] = place }
            }
        }
        return out
    }

    /// The first terminal of each space without a name: the one it is called after.
    public func unnamedSpaceLeads() -> [String] {
        spaces.indices.compactMap { si in
            guard (spaces[si]["name"] as? String ?? "").isEmpty else { return nil }
            return tabs(si).lazy.compactMap { $0["root"] as? [String: Any] }.flatMap { Self.blocks($0).compactMap(Self.terminal) }.first
        }
    }

    /// A folder's short name (model.rs folder_name): `~` for home or none, else its last part.
    public static func folderName(_ path: String, home: String) -> String {
        func trim(_ s: String) -> Substring {
            var s = Substring(s)
            while s.last == "/" { s = s.dropLast() }
            return s
        }
        let p = trim(path)
        if path.isEmpty || p == trim(home) { return "~" }
        if p.isEmpty { return String(path.prefix(1)) }
        return p.split(separator: "/").last.map(String.init) ?? String(p)
    }

    /// False when the pane showing this terminal is marked `closable: false` (the official app then keeps it).
    public func paneClosable(terminal address: String) -> Bool {
        for si in spaces.indices {
            for t in tabs(si) {
                guard let root = t["root"] as? [String: Any],
                      let n = Self.blocks(root).first(where: { Self.terminal($0) == address }) else { continue }
                return n["closable"] as? Bool != false
            }
        }
        return true
    }

    /// The `title` someone gave the pane showing this terminal (Flockbook's pane name; the official app keeps it).
    public func paneTitle(terminal address: String) -> String? {
        for si in spaces.indices {
            for t in tabs(si) {
                if let root = t["root"] as? [String: Any], let b = Self.blocks(root).first(where: { Self.terminal($0) == address }) {
                    return (b["title"] as? String).flatMap { $0.isEmpty ? nil : $0 }
                }
            }
        }
        return nil
    }

    /// The model pane id (`p…`) showing this terminal (for `herdr-app do pane.close`).
    public func paneId(terminal address: String) -> String? {
        for si in spaces.indices {
            for t in tabs(si) {
                if let root = t["root"] as? [String: Any], let b = Self.blocks(root).first(where: { Self.terminal($0) == address }) { return b["id"] as? String }
            }
        }
        return nil
    }

    static func terminal(_ n: [String: Any]) -> String? { (n["content"] as? [String: Any])?["terminal"] as? String }
    static func isPane(_ n: [String: Any]) -> Bool {
        guard n["type"] as? String == "block", let c = n["content"] as? [String: Any] else { return false }
        return c["terminal"] is String || c["mod"] is String
    }
    static func children(_ n: [String: Any]) -> [[String: Any]] { n["children"] as? [[String: Any]] ?? [] }

    /// Pane blocks (terminal or mod) in tree order.
    static func blocks(_ n: [String: Any]) -> [[String: Any]] {
        (isPane(n) ? [n] : []) + children(n).flatMap(blocks)
    }

    // MARK: - Ids

    /// The official app's ids: prefix + 32 random hex digits.
    public static func freshId(_ prefix: String) -> String {
        String(format: "%@%016llx%016llx", prefix, UInt64.random(in: .min ... .max), UInt64.random(in: .min ... .max))
    }

    static func paneNode(id: String, terminal: String) -> [String: Any] {
        ["id": id, "type": "block", "content": ["terminal": terminal]]
    }

    static func tabNode(id: String, name: String, paneId: String, terminal: String) -> [String: Any] {
        ["id": id, "name": name, "root": ["id": id, "type": "row", "children": [paneNode(id: paneId, terminal: terminal)]]]
    }

    // MARK: - Edits (the official app's rules)

    /// Puts a live labeled terminal where its labels say, as the official app's adoption does: an unknown space or tab
    /// is made under the label's id, unnamed; a known tab gets the pane split right of its last pane. No-op when a pane
    /// already shows the terminal.
    public mutating func adopt(terminal: String, space: String, tab: String, folder: String, paneId: String = freshId("p")) {
        guard !showsTerminal(terminal) else { return }
        guard let si = spaceIndex(space) else {
            spaces.append(["id": space, "name": "", "folder": folder, "tabs": [Self.tabNode(id: tab, name: "", paneId: paneId, terminal: terminal)]])
            return
        }
        // A known tab id (in any space, as `Model::tab`) gets the pane right of its last one.
        if let (tsi, ti) = tabIndex(tab) {
            var t = tabs(tsi)[ti]
            var root = t["root"] as? [String: Any] ?? ["id": tab, "type": "row", "children": [Any]()]
            if let last = Self.blocks(root).last, let lastId = last["id"] as? String {
                _ = Self.split(&root, target: lastId, axis: "row", new: Self.paneNode(id: paneId, terminal: terminal), containerId: Self.freshId("s"))
            } else {
                root["children"] = Self.children(root) + [Self.paneNode(id: paneId, terminal: terminal)]
            }
            t["root"] = root
            setTab(tsi, ti, t)
            return
        }
        var ts = tabs(si)
        ts.append(Self.tabNode(id: tab, name: "", paneId: paneId, terminal: terminal))
        spaces[si]["tabs"] = ts
    }

    private mutating func setTab(_ si: Int, _ ti: Int, _ t: [String: Any]) {
        var ts = tabs(si)
        ts[ti] = t
        spaces[si]["tabs"] = ts
    }

    /// `space.new`: a space with one tab showing `terminal`. If the official app adopted the terminal already, only the names are set.
    public mutating func addSpace(id: String, name: String, folder: String, tab: String, tabName: String = "", paneId: String = freshId("p"), terminal: String) {
        adopt(terminal: terminal, space: id, tab: tab, folder: folder, paneId: paneId)
        if let si = spaceIndex(id) { spaces[si]["name"] = name }
        if !tabName.isEmpty { renameTab(tab, to: tabName) }
    }

    /// `tab.new`: appended to the space (the selected tab is per screen, so Flockbook adds at the end).
    public mutating func addTab(space: String, folder: String, id: String, name: String = "", paneId: String = freshId("p"), terminal: String) {
        adopt(terminal: terminal, space: space, tab: id, folder: folder, paneId: paneId)
        if !name.isEmpty { renameTab(id, to: name) }
    }

    @discardableResult public mutating func renameSpace(_ id: String, to name: String) -> Bool {
        guard let si = spaceIndex(id) else { return false }
        spaces[si]["name"] = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return true
    }

    @discardableResult public mutating func renameTab(_ id: String, to name: String) -> Bool {
        guard let (si, ti) = tabIndex(id) else { return false }
        var t = tabs(si)[ti]
        t["name"] = name.trimmingCharacters(in: .whitespacesAndNewlines)
        setTab(si, ti, t)
        return true
    }

    /// Moves a tab to `index` (final 0-based position, clamped) inside its space.
    @discardableResult public mutating func moveTab(_ id: String, to index: Int) -> Bool {
        guard let (si, ti) = tabIndex(id) else { return false }
        var ts = tabs(si)
        let t = ts.remove(at: ti)
        ts.insert(t, at: min(max(index, 0), ts.count))
        spaces[si]["tabs"] = ts
        return true
    }

    /// Removes a space. Returns the terminals its tabs showed.
    @discardableResult public mutating func removeSpace(_ id: String) -> [String] {
        guard let si = spaceIndex(id) else { return [] }
        let addrs = tabs(si).flatMap { ($0["root"] as? [String: Any]).map(Self.blocks)?.compactMap(Self.terminal) ?? [] }
        spaces.remove(at: si)
        return addrs
    }

    /// Removes a tab; its space goes with its last tab. Returns the terminals it showed and whether the space closed.
    @discardableResult public mutating func removeTab(_ id: String) -> (terminals: [String], spaceClosed: Bool) {
        guard let (si, ti) = tabIndex(id) else { return ([], false) }
        var ts = tabs(si)
        let gone = ts.remove(at: ti)
        let addrs = (gone["root"] as? [String: Any]).map(Self.blocks)?.compactMap(Self.terminal) ?? []
        if ts.isEmpty { spaces.remove(at: si); return (addrs, true) }
        spaces[si]["tabs"] = ts
        return (addrs, false)
    }

    /// Removes the pane showing `terminal`. A tab left without panes closes, and so does a space left without tabs.
    @discardableResult public mutating func removePane(terminal: String) -> (found: Bool, tabClosed: Bool, spaceClosed: Bool) {
        guard let (sid, tid) = place(ofTerminal: terminal), let (si, ti) = tabIndex(tid), spaceIndex(sid) == si else { return (false, false, false) }
        var t = tabs(si)[ti]
        guard var root = t["root"] as? [String: Any], let pid = Self.blocks(root).first(where: { Self.terminal($0) == terminal })?["id"] as? String else { return (false, false, false) }
        if root["id"] as? String == pid {
            root = ["id": tid, "type": "row", "children": [Any]()]
        } else {
            _ = Self.remove(&root, id: pid)
            Self.collapse(&root)
        }
        if Self.blocks(root).isEmpty {
            let r = removeTab(tid)
            return (true, true, r.spaceClosed)
        }
        t["root"] = root
        setTab(si, ti, t)
        return (true, false, false)
    }

    /// Splits the pane showing `target` with a new pane for `terminal` (`down`: a column, else a row).
    @discardableResult public mutating func split(target: String, down: Bool, terminal: String, paneId: String = freshId("p")) -> Bool {
        guard let (_, tid) = place(ofTerminal: target), let (si, ti) = tabIndex(tid) else { return false }
        var t = tabs(si)[ti]
        guard var root = t["root"] as? [String: Any], let pid = Self.blocks(root).first(where: { Self.terminal($0) == target })?["id"] as? String else { return false }
        guard Self.split(&root, target: pid, axis: down ? "column" : "row", new: Self.paneNode(id: paneId, terminal: terminal), containerId: Self.freshId("s")) else { return false }
        t["root"] = root
        setTab(si, ti, t)
        return true
    }

    /// Names the pane showing `terminal` (`title`; nil or "" clears it).
    @discardableResult public mutating func setPaneTitle(terminal: String, _ title: String?) -> Bool {
        guard let (_, tid) = place(ofTerminal: terminal), let (si, ti) = tabIndex(tid) else { return false }
        var t = tabs(si)[ti]
        guard var root = t["root"] as? [String: Any] else { return false }
        let name = title?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        Self.edit(&root) { n in
            guard Self.terminal(n) == terminal else { return false }
            n["title"] = name.isEmpty ? nil : name
            return true
        }
        t["root"] = root
        setTab(si, ti, t)
        return true
    }

    // MARK: - Tree operations (layout.rs)

    /// Applies `f` to the first node (pre-order) it accepts.
    @discardableResult static func edit(_ n: inout [String: Any], _ f: (inout [String: Any]) -> Bool) -> Bool {
        if f(&n) { return true }
        guard var ch = n["children"] as? [[String: Any]] else { return false }
        for i in ch.indices where edit(&ch[i], f) { n["children"] = ch; return true }
        return false
    }

    enum Share { case weight(Double), px }
    static func share(_ n: [String: Any]) -> Share {
        guard let s = n["size"] as? [String: Any] else { return .weight(1) }
        if let w = s["weight"] as? NSNumber { return .weight(w.doubleValue) }
        return .px
    }

    static func axis(_ n: [String: Any]) -> String? {
        let t = n["type"] as? String
        return t == "row" || t == "column" ? t : nil
    }

    /// `Node::split`: beside `target` (after it) along `axis`. A parent on that axis takes it in, halving the target's
    /// weight for both (a px size stays); otherwise the target is wrapped in a new container that takes its slot size.
    static func split(_ root: inout [String: Any], target: String, axis along: String, new: [String: Any], containerId: String) -> Bool {
        // Parent on the same axis.
        let parentDone = edit(&root) { p in
            guard axis(p) == along, var c = p["children"] as? [[String: Any]], let i = c.firstIndex(where: { $0["id"] as? String == target }) else { return false }
            var n = new
            switch share(c[i]) {
            case .weight(let w): c[i]["size"] = ["weight": w / 2]; n["size"] = ["weight": w / 2]
            case .px: n["size"] = ["weight": 0.5]
            }
            c.insert(n, at: i + 1)
            p["children"] = c
            return true
        }
        if parentDone { return true }
        return edit(&root) { t in
            guard t["id"] as? String == target else { return false }
            var old = t
            let slot = old["size"]
            old["size"] = nil
            // The wrapper gets the new container id; the target keeps its own.
            var wrapper: [String: Any] = ["id": containerId, "type": along, "children": [old, new]]
            wrapper["size"] = slot
            t = wrapper
            return true
        }
    }

    /// `remove_inner`: removes `id`; in a row/column its weight goes to the sibling before it (else the next).
    static func remove(_ n: inout [String: Any], id: String) -> Bool {
        guard var c = n["children"] as? [[String: Any]] else { return false }
        if let i = c.firstIndex(where: { $0["id"] as? String == id }) {
            let gone = c.remove(at: i)
            if axis(n) != nil, case .weight(let g) = share(gone), !c.isEmpty {
                let j = max(i - 1, 0)
                if case .weight(let w) = share(c[j]) { c[j]["size"] = ["weight": w + g] }
            }
            if n["type"] as? String == "stack", n["active"] as? String == id {
                if let next = i < c.count ? c[i] : c.last { n["active"] = next["id"] } else { n["active"] = nil }
            }
            n["children"] = c
            return true
        }
        for i in c.indices where remove(&c[i], id: id) { n["children"] = c; return true }
        return false
    }

    /// `collapse`: empty rows/columns vanish, ones with one child give way to it (it takes their size); stacks stay.
    static func collapse(_ n: inout [String: Any]) {
        guard var c = n["children"] as? [[String: Any]] else { return }
        for i in c.indices { collapse(&c[i]) }
        if n["type"] as? String == "stack" { n["children"] = c; return }
        c.removeAll { axis($0) != nil && children($0).isEmpty }
        for i in c.indices where axis(c[i]) != nil && children(c[i]).count == 1 {
            let size = c[i]["size"]
            var only = children(c[i])[0]
            only["size"] = size
            c[i] = only
        }
        n["children"] = c
    }
}
