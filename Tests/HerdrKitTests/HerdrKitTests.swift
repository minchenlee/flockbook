import Testing
@testable import HerdrKit

@Suite struct SnapshotTests {
    @Test func parsesTree() throws {
        let ws = try Support.parseSnapshot()
        #expect(ws.count == 1)
        let tabs = ws[0].tabs
        #expect(tabs.map(\.label) == ["Main", "Shell"])
        #expect(tabs[0].panes.count == 2)
        #expect(tabs[0].status == .blocked)         // rollup: blocked beats working
        #expect(tabs[1].panes[0].agent == nil)      // shell pane
        #expect(tabs[1].status == .unknown)
        let a = try #require(tabs[0].panes[0].agent)
        #expect(a.name == "demo-dev" && a.kind == "claude" && a.sessionId == "sess-1" && a.title == "Fix the bug")
        #expect(tabs[0].panes[0].terminalId == "term_a")
    }

    @Test func rollupOrder() {
        #expect(AgentStatus.rollup([.idle, .done, .working]) == .working)
        #expect(AgentStatus.rollup([.idle, .done]) == .done)
        #expect(AgentStatus.rollup([AgentStatus]()) == .unknown)
    }
}

@Suite struct SessionTests {
    @Test func launchArguments() {
        #expect(HerdrSession.fromLaunch(arguments: ["app", "--session", "x"], environment: [:]).name == "x")
        #expect(HerdrSession.fromLaunch(arguments: ["app"], environment: ["HERDR_DESK_SESSION": "y"]).name == "y")
        #expect(HerdrSession.fromLaunch(arguments: ["app"], environment: [:]).name == nil)
        #expect(HerdrSession.named("c9-input-test").socketPath.hasSuffix("sessions/c9-input-test/herdr.sock"))
    }
}

@Suite struct ConversationTests {
    @Test func mapsLines() throws {
        let got = Support.describe(SessionLog.parse(lines: try Support.sessionLines()))
        #expect(got == [
            "user:Fix the login bug", "asst:Looking at it now.", "chips:Read 2 files|Ran a command",
            "asst:Tests pass.", "perm:rm -rf build",
        ])
    }

    @Test func unansweredToolBecomesPendingPermission() throws {
        let items = SessionLog.parse(lines: try Support.sessionLines())
        let got = Support.describe(items)
        #expect(got.last == "perm:rm -rf build")
        #expect(got.count == 5)
        guard case .permissionRequest(let r) = try #require(items.last).kind else { Issue.record("not a permissionRequest"); return }
        #expect(r.state == .pending && r.toolName == "Bash" && r.toolUseId == "t9")
        #expect(!Support.describe(SessionLog.parse(lines: Array(try Support.sessionLines().prefix(5)))).contains { $0.hasPrefix("perm:") })
    }

    @Test func codexRolloutMapsLines() throws {
        let got = Support.describe(SessionLog.parse(lines: try Support.codexLines()))
        #expect(got == ["user:Review the parser", "asst:Checking the files.", "chips:Ran 2 commands", "asst:No P1 found.", "perm:rm -rf build"])
    }

    @Test func injectedUserLines() throws {
        let inj = String(decoding: try Support.fixtureData("injected.jsonl"), as: UTF8.self).split(separator: "\n").map(String.init)
        #expect(Support.describe(SessionLog.parse(lines: inj)) == [
            "user:Real question", "status", "status", "chips:Read a file",
            "user:[herdr-send from orchard-main-orch] please rebuild", "asst:On it.",
        ])
    }

    @Test func promptMarker() {
        // The question alone is not a prompt: the numbered choices must be on screen too.
        #expect(!PermissionGate.screenShowsPrompt("Do you want to proceed?"))
        #expect(PermissionGate.screenShowsPrompt("Do you want to proceed?\n❯ 1. Yes\n  2. No"))
        #expect(!PermissionGate.screenShowsPrompt("Running tests"))
    }

    @Test func stableIds() throws {
        let a = SessionLog.parse(lines: try Support.sessionLines()).map(\.id)
        let b = SessionLog.parse(lines: try Support.sessionLines()).map(\.id)
        #expect(a == b)
    }

    @Test func locatesAndTails() async throws {
        let r = try await Support.tailRoundTrip()
        #expect(r.found)
        #expect(!r.escaped)
        #expect(r.counts == [0, 1, 2])
    }
}
