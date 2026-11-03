import Foundation
@testable import HerdrKit

// Composer with a pasted image (live 2026-11-02): on hc a pasted image path inside the message reaches
// Claude as text and Claude reads the file (Read tool); a bare path becomes `[Image #1]`. A terminal on another machine
// cannot read this Mac's file: the image is first put beside it (`terminal.put`, on its machine) and the message names
// that copy.

@MainActor func checksHCImage() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("t1", argv: ["claude"]), FakeHC.terminal("box/t7", argv: ["claude"])]
    let dir = NSTemporaryDirectory() + "hkimg-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let png = dir + "/paste-1.png"
    FileManager.default.createFile(atPath: png, contents: Data([0x89, 0x50, 0x4E, 0x47]))
    let text = "what is this? \(png) thanks"
    check(HCClient.localFiles(in: text, under: dir) == [png], "image: the pasted file is found in the message")
    let hc = HCClient(session: HCSession(socketPath: fake.path))

    // Here: the text goes as it is (the file is on this machine).
    var r = await hc.deliver(text: text, to: "hc:t1", files: [png])
    var sent = fake.calls.last { $0.method == "terminal.send" }?.params
    check(r == .sent && sent?["text"] as? String == text && !fake.calls.contains { $0.method == "terminal.put" },
          "image: a local terminal gets the path as it is, nothing is put")

    // Another machine: put first, then the text with the path there.
    r = await hc.deliver(text: text, to: "hc:box/t7", files: [png])
    let put = fake.calls.last { $0.method == "terminal.put" }?.params
    sent = fake.calls.last { $0.method == "terminal.send" }?.params
    check(put?["terminal"] as? String == "box/t7" && put?["name"] as? String == "paste-1.png"
          && put?["data"] as? String == Data([0x89, 0x50, 0x4E, 0x47]).base64EncodedString(),
          "image: the file is put beside the remote terminal (\(put ?? [:]))")
    check(r == .sent && sent?["terminal"] as? String == "box/t7" && sent?["text"] as? String == "what is this? /home/b/.hc/put/box-t7/paste-1.png thanks",
          "image: the message names the copy on box (\(sent?["text"] ?? "nil"))")

    // a path there with a space (or quote) is quoted as the composer quotes paths.
    fake.putDir = "/home/b w/.hc/put"
    r = await hc.deliver(text: text, to: "hc:box/t7", files: [png])
    sent = fake.calls.last { $0.method == "terminal.send" }?.params
    check(r == .sent && sent?["text"] as? String == "what is this? '/home/b w/.hc/put/box-t7/paste-1.png' thanks",
          "image: a copy path with a space is quoted (\(sent?["text"] ?? "nil"))")
    fake.putDir = "/home/b/o'k"
    r = await hc.deliver(text: "see '\(png)'", to: "hc:box/t7", files: [png])
    sent = fake.calls.last { $0.method == "terminal.send" }?.params
    check(sent?["text"] as? String == #"see '/home/b/o'\''k/box-t7/paste-1.png'"#,
          "image: a quoted local path becomes one quoted path there (\(sent?["text"] ?? "nil"))")
    fake.putDir = "/home/b/.hc/put"

    // A file that cannot be put: nothing is sent, the composer keeps the text.
    let n = fake.calls.count
    r = await hc.deliver(text: "x \(dir)/gone.png", to: "hc:box/t7", files: [dir + "/gone.png"])
    check(r == .failed && !fake.calls[n...].contains { $0.method == "terminal.send" }, "image: a missing file fails before anything is sent")
}

/// A paired device's send to a terminal on another machine: the files go first (each `terminal.put` is a call to that
/// machine), then the key gate, then the text. A program that changed during the uploads gets nothing.
@MainActor func checksHCDeviceSend() async throws {
    let fake = FakeHC(path: fakeHCPath())
    try fake.start()
    defer { fake.kill() }
    fake.machines = [FakeHC.machine("box")]
    fake.terminals = [FakeHC.terminal("box/t7", argv: ["claude"], status: "idle")]
    let dir = NSTemporaryDirectory() + "hkdev-\(UUID().uuidString)"
    try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(atPath: dir) }
    let png = dir + "/paste-1.png"
    FileManager.default.createFile(atPath: png, contents: Data([0x89, 0x50, 0x4E, 0x47]))
    let hc = HCClient(session: HCSession(socketPath: fake.path))
    guard var seen = try await hc.snapshot().flatMap(\.tabs).flatMap(\.panes).first(where: { $0.id == "hc:box/t7" })?.agent else {
        check(false, "device send: the agent is there"); return
    }
    seen.sessionId = "sid-1"; seen.sessionResolved = true
    let gate = KeyGate(agent: seen, allowed: { true })
    func sends() -> Int { fake.calls.filter { $0.method == "terminal.send" }.count }

    var r = await hc.sendFromDevice(text: "look \(png)", to: "hc:box/t7", files: [png], gate: gate)
    let methods = fake.calls.map(\.method).filter { ["terminal.put", "terminal.list", "terminal.send"].contains($0) }
    check(r == .sent && methods.suffix(3) == ["terminal.put", "terminal.list", "terminal.send"],
          "device send: put, then the gate's fresh read, then the text (\(r), \(methods.suffix(3)))")
    check(fake.calls.last { $0.method == "terminal.send" }?.params["text"] as? String == "look /home/b/.hc/put/box-t7/paste-1.png",
          "device send: the text names the copy there")

    // Another program starts while the file uploads: nothing is typed.
    let n = sends()
    fake.onPut = { _ in fake.terminals = [FakeHC.terminal("box/t7", argv: ["claude"], pid: 5151, status: "idle")] }
    r = await hc.sendFromDevice(text: "look \(png)", to: "hc:box/t7", files: [png], gate: gate)
    check(r == .changed && sends() == n, "device send: a new program during the upload -> changed, nothing sent (\(r))")
    fake.onPut = nil
    // A file that cannot be put: failed, nothing sent.
    r = await hc.sendFromDevice(text: "x \(dir)/gone.png", to: "hc:box/t7", files: [dir + "/gone.png"], gate: gate)
    check(r == .failed && sends() == n, "device send: a file that cannot be put -> failed, nothing sent")
}
