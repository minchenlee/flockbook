import Foundation
@testable import HerdrKit

// The third review of the Since you left card (each failed before its fix).

/// The app's copy shows no turn yet when the read decides (a mirror still empty, a log not loaded): nothing is marked
/// until the copy shows a turn, so the newer turn is not marked seen before it is on screen.
func checksSinceArrivalNoCopy() {
    let now: Int64 = 100 * 60_000, seen = SinceLeft.Seen(id: "a1", at: now - 47 * 60_000)
    var a = SinceLeft.Arrival()
    let d = a.first("a2", shown: nil, seen: seen, now: now)
    check(d.decided && d.away != nil, "since arrival no copy: the read decides: a card")
    check(a.mark(nil) == nil, "since arrival no copy: the app's copy shows nothing yet: nothing is marked (\(a.mark(nil) ?? "nil"))")
    check(a.mark("a2") == "a2", "since arrival no copy: once the read's turn shows, it is marked")
    check(a.mark(nil) == "a2", "since arrival no copy: after that, a copy that shows nothing keeps the decided turn")
    var older = SinceLeft.Arrival()
    _ = older.first("a2", shown: nil, seen: seen, now: now)
    check(older.mark("a1") == "a1", "since arrival no copy: the copy shows an older turn: that turn is marked (the card comes again)")
    var same = SinceLeft.Arrival()
    _ = same.first("a2", shown: "a2", seen: seen, now: now)
    check(same.mark(nil) == "a2", "since arrival no copy: decided on the turn shown: a copy that shows nothing keeps it")
}

