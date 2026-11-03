import Foundation

/// Text pulled back from an agent's queue (`QueuePull`). Kept from before the first Ctrl-U until the composer of the
/// device that pulled it has it, so a failed read, a lost connection or a quit never loses it.
public struct PulledText: Codable, Hashable, Sendable {
    public var id: String
    public var pane: String
    /// Who pulled it: `PulledText.mac`, or a hub device id. Only that one gets it.
    public var device: String
    public var text: String
    public var at: Date
    /// What the device is told with the text when it gets it again (a partial pull's reason), and whether the turn
    /// stopped. Until the pull ends: `unfinished` (a quit mid-pull never looks like a clean one).
    public var note: String?
    public var interrupted: Bool
    public init(id: String = UUID().uuidString, pane: String, device: String, text: String, at: Date = Date(),
                note: String? = nil, interrupted: Bool = false) {
        self.id = id; self.pane = pane; self.device = device; self.text = text; self.at = at; self.note = note; self.interrupted = interrupted
    }
    /// The Mac app's own composer.
    public static let mac = "mac"
    /// The note of a kept text whose pull has not ended (a quit mid-pull): the terminal may still hold some of it.
    public static let unfinished = "Flockbook may not have finished clearing the agent's input box. Check the agent's terminal before you send it."

    private enum K: String, CodingKey { case id, pane, device, text, at, note, interrupted }
    /// Files from before `note` and `interrupted` read back too.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: K.self)
        id = try c.decode(String.self, forKey: .id); pane = try c.decode(String.self, forKey: .pane)
        device = try c.decode(String.self, forKey: .device); text = try c.decode(String.self, forKey: .text)
        at = try c.decode(Date.self, forKey: .at)
        note = try c.decodeIfPresent(String.self, forKey: .note)
        interrupted = try c.decodeIfPresent(Bool.self, forKey: .interrupted) ?? false
    }
}

/// Pulled texts not in a composer yet. `keep` before the Ctrl-U; `take` once the composer has the text. At most
/// `maxCount` texts and `maxBytes` of text: full, `keep` refuses (and the pull clears nothing).
@MainActor public class MemoryPulledStore {
    public private(set) var all: [PulledText] = []
    public let maxCount: Int
    public let maxBytes: Int
    public init(maxCount: Int = 100, maxBytes: Int = 1 << 20) { self.maxCount = maxCount; self.maxBytes = maxBytes }

    /// Keeps `p`. False: full, or it could not be saved (then nothing must clear the agent's input box).
    public func keep(_ p: PulledText) -> Bool {
        guard all.count < maxCount, all.reduce(p.text.utf8.count, { $0 + $1.text.utf8.count }) <= maxBytes else { return false }
        all.append(p)
        guard save() else { all.removeLast(); return false }
        return true
    }

    /// The pull of `id` ended: what the device is told when it gets the text again.
    public func finish(id: String, note: String?, interrupted: Bool) {
        guard let i = all.firstIndex(where: { $0.id == id }) else { return }
        all[i].note = note; all[i].interrupted = interrupted
        _ = save()   // not saved: the warning from `keep` stays (more caution, nothing lost)
    }

    /// A removed device's texts: no composer will ever confirm them.
    public func drop(device: String) { remove { $0.device == device } }
    /// Texts of panes that are gone (`panes`: the panes there are now): no composer of theirs is left.
    public func prune(panes: Set<String>) { remove { !panes.contains($0.pane) } }
    /// Removes the texts of panes `gone` says are gone (`HerdrStore.paneGone`: per backend).
    public func prune(gone: (String) -> Bool) { remove { gone($0.pane) } }

    private func remove(_ gone: (PulledText) -> Bool) {
        let before = all.count
        all.removeAll(where: gone)
        if all.count != before { _ = save() }
    }

    /// The text `id` of `device`, removed: its composer has it now. Nil: not kept (or taken already).
    public func take(id: String, device: String) -> PulledText? {
        guard let i = all.firstIndex(where: { $0.id == id && $0.device == device }) else { return nil }
        let p = all.remove(at: i)
        _ = save()   // not saved: a restart brings it back once more (twice in a composer, never lost)
        return p
    }

    /// The device's texts not taken yet, oldest first.
    public func pending(device: String) -> [PulledText] { all.filter { $0.device == device } }

    func load(_ ps: [PulledText]) { all = ps }
    /// Saves; false when that failed.
    func save() -> Bool { true }
}

/// Pulled texts in a JSON file, mode 600 (it holds what the owner typed).
@MainActor public final class FilePulledStore: MemoryPulledStore {
    public let url: URL
    public init(url: URL, maxCount: Int = 100, maxBytes: Int = 1 << 20) {
        self.url = url
        super.init(maxCount: maxCount, maxBytes: maxBytes)
        if let data = try? Data(contentsOf: url) {
            let d = JSONDecoder(); d.dateDecodingStrategy = .iso8601
            load((try? d.decode([PulledText].self, from: data)) ?? [])
        }
    }
    override func save() -> Bool {
        let e = JSONEncoder(); e.dateEncodingStrategy = .iso8601; e.outputFormatting = [.prettyPrinted, .sortedKeys]
        return (try? PrivateFile.write(try e.encode(all), to: url)) != nil
    }
}
