import Foundation
#if canImport(AppKit)
import AppKit
#endif

/// `paste-image`: the clipboard's image as a PNG file, so the mod can attach it like a picked file.
/// herdr-app's mod API reads only clipboard text.
public enum FeedPaste {
    /// Files older than this in the paste folder are removed on each paste.
    public static let keep: TimeInterval = 24 * 3600

    /// The clipboard's image as PNG data, or nil when it holds none.
    public static func clipboardPNG() -> Data? {
        #if canImport(AppKit)
        let pb = NSPasteboard.general
        if let png = pb.data(forType: .png) { return png }
        if let tiff = pb.data(forType: .tiff), let rep = NSBitmapImageRep(data: tiff) { return rep.representation(using: .png, properties: [:]) }
        #endif
        return nil
    }

    /// The name `save` gives a paste: "paste-YYYYMMDD-HHMMSS-xxxxxx.png". Only these are ever removed.
    static func isPaste(_ name: String) -> Bool {
        name.wholeMatch(of: /paste-\d{8}-\d{6}-[0-9A-Fa-f]{6}\.png/) != nil
    }

    /// Writes `png` into `dir` as a new file and returns its path; removes its own pastes older than `keep`. Other
    /// files in `dir` are never touched (`--dir` can name any folder).
    public static func save(_ png: Data, dir: URL, now: Date = Date()) throws -> String {
        let fm = FileManager.default
        try fm.createDirectory(at: dir, withIntermediateDirectories: true)
        for f in (try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: [.contentModificationDateKey])) ?? []
        where isPaste(f.lastPathComponent) {
            if let d = try? f.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate, now.timeIntervalSince(d) > keep {
                try? fm.removeItem(at: f)
            }
        }
        let f = DateFormatter(); f.locale = Locale(identifier: "en_US_POSIX"); f.dateFormat = "yyyyMMdd-HHmmss"
        let url = dir.appendingPathComponent("paste-\(f.string(from: now))-\(UUID().uuidString.prefix(6)).png")
        try png.write(to: url, options: .atomic)
        return url.path
    }
}
