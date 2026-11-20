import Foundation

/// The finished copies of each mirrored log (an agent's log on another machine, copied to a file here), by the local
/// file's path. A read of a mirror shows the log as it is only after a copy that finished after it was asked for.
/// Both mirrors count: herdr over ssh (`RemoteLogMirror`) and hc's other machines (`HCClient` mirror pollers).
public enum MirrorSyncs {
    private static let counts = Locked<[String: Int]>([:])

    /// A new mirror at `path`: no copy finished yet (also when an earlier mirror used the same path).
    static func register(_ path: String) { counts.update { $0[path] = 0 } }
    /// A copy finished: the file now has the log as the host had it then. A path that is no mirror (stopped: a
    /// poller's late copy) stays none.
    static func bump(_ path: String) { counts.update { if let n = $0[path] { $0[path] = n + 1 } } }
    static func drop(_ path: String) { counts.update { $0[path] = nil } }

    /// Copies finished so far; nil: `path` is no mirror (a log on this Mac).
    public static func count(_ path: String) -> Int? { counts.get()[path] }
}
