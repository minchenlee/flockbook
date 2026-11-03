import Testing
@testable import HerdrKit

@Suite struct HCFrameTests {
    @Test func headerIsLittleEndian() {
        let f = HCFrame(channel: 0x0102_0304, kind: .credit, payload: Support.bytes([9, 8]))
        #expect(Support.array(f.encoded()) == [4, 3, 2, 1, 1, 2, 0, 0, 0, 9, 8])
    }

    @Test func decoderReassembles() throws {
        let a = HCFrame(channel: 0, kind: .data, payload: Support.bytes([1, 2, 3]))
        let b = HCFrame(channel: 7, kind: .close, payload: Support.bytes([]))
        let all = Support.array(a.encoded()) + Support.array(b.encoded())
        var dec = HCFrameDecoder()
        var got: [HCFrame] = []
        for byte in all { got += try dec.feed([byte][...]) }
        #expect(got == [a, b])
    }

    @Test func oversizeRejected() {
        var dec = HCFrameDecoder()
        let bad: [UInt8] = [0, 0, 0, 0, 0, 0x01, 0x00, 0x00, 0x01]
        #expect(throws: (any Error).self) { _ = try dec.feed(bad[...]) }
    }
}

@Suite struct HCSessionTests {
    @Test func socketRules() {
        #expect(HCSession.resolve(environment: ["HC_SOCKET": "/x.sock"]).socketPath == "/x.sock")
        #expect(HCSession.resolve(environment: ["XDG_RUNTIME_DIR": "/r"]).socketPath == "/r/herdr-core/default.sock")
        #expect(HCSession.resolve(environment: ["XDG_STATE_HOME": "/s", "HC_SESSION": "q"]).socketPath == "/s/herdr-core/q/q.sock")
    }
}
