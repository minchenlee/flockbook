//! Ports of the conversation checks in ChecksCore.swift (maps lines, injected, pasted, codex rollout,
//! tool calls, timestamps, prompt detection, cut) and ChecksAgents.swift (pi, cursor).
mod common;
use common::*;
use herdr_core::conversation::{cut, user_visible, ConversationParser};
use herdr_core::{ItemKind, PermState, QuestionState};

fn at(secs: f64) -> Option<f64> {
    Some(secs)
}
fn near(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 0.002)
}

#[test]
fn claude_maps_lines() {
    let lines = fixture_lines("session.jsonl");
    assert_eq!(
        describe(&parse(&lines)),
        ["user:Fix the login bug", "asst:Looking at it now.", "chips:Read 2 files|Ran a command", "asst:Tests pass.", "perm:rm -rf build"],
        "maps lines"
    );
    let tail = parse(&lines);
    assert!(describe(&tail).last().unwrap() == "perm:rm -rf build" && tail.len() == 5, "unanswered tail tool_use becomes permissionRequest");
    match &tail.last().unwrap().kind {
        ItemKind::PermissionRequest(r) => {
            assert!(r.state == PermState::Pending && r.tool_name == "Bash" && r.tool_use_id == "t9", "permissionRequest is pending with tool info");
        }
        _ => panic!("last item is permissionRequest"),
    }
    let answered = parse(&lines[..5]);
    assert!(!describe(&answered).iter().any(|d| d.starts_with("perm:")), "answered tool_use emits no permissionRequest");
    let ids = |v: &[herdr_core::ConversationItem]| v.iter().map(|i| i.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&parse(&lines)), ids(&parse(&lines)), "stable ids");
}

#[test]
fn injected_user_lines() {
    let inj = fixture_lines("injected.jsonl");
    assert_eq!(
        describe(&parse(&inj)),
        ["user:Real question", "status", "status", "chips:Read a file", "user:[herdr-send from orchard-main-orch] please rebuild", "asst:On it."],
        "injected user lines"
    );
    match &parse(&inj)[1].kind {
        ItemKind::Status(t) => assert_eq!(t, "Context compacted", "compaction status text"),
        _ => panic!("compaction is a status item"),
    }
}

#[test]
fn pasted_text() {
    let p = fixture_lines("pasted.jsonl");
    assert_eq!(
        describe(&parse(&p)),
        [
            "user:Ship the Orchard banner today.",
            "user:Please review this:\n\nline A\nline B\n\nThanks",
            "user:first\n\nsecond",
            "user:Relay: <pasted_content id=\"3\">inner</pasted_content id=\"3\"> end",
            "user:block form",
            "user:cut off at the end",
        ],
        "pasted: shown as user text, wrappers stripped, order kept, injected tags still hidden"
    );
    assert!(
        user_visible("<environment_context>cwd</environment_context>").is_none()
            && user_visible("[Request interrupted by user]").is_none()
            && user_visible("typed <b>text</b>").as_deref() == Some("typed <b>text</b>"),
        "pasted: allowlist of visible tags"
    );
    let cxp = parse(&[
        r#"{"timestamp":"2026-10-10T01:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<pasted_content id=\"1\">codex paste</pasted_content id=\"1\">"}]}}"#.to_string(),
        r#"{"timestamp":"2026-10-10T01:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}"#.to_string(),
    ]);
    assert_eq!(describe(&cxp), ["user:codex paste"], "pasted: codex user text uses the same rule");
}

#[test]
fn hc_notes_are_shown() {
    // What `hc send` types into a lead agent: one `<hc from="<terminal> [name]">…</hc>` envelope. It is the
    // conversation, not injected context, so it stays visible with its tag (the row decides how to draw it).
    let note = r#"<hc from="t2 docs-worker">Done: notes updated.\nSee notes.md</hc>"#;
    let shown = note.replace("\\n", "\n");
    assert_eq!(user_visible(&shown).as_deref(), Some(shown.as_str()), "hc: a note is visible");
    let lines = [
        format!(
            r#"{{"timestamp":"2026-10-10T01:00:00.000Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{}"}}]}}}}"#,
            note.replace('"', "\\\"")
        ),
        r#"{"type":"user","uuid":"u1","timestamp":"2026-10-10T01:00:01.000Z","message":{"role":"user","content":"<pasted_content id=\"a1\">\n<hc from=\"t3\">Short note</hc>\n</pasted_content id=\"a1\">"}}"#.to_string(),
        r#"{"type":"user","uuid":"u2","timestamp":"2026-10-10T01:00:02.000Z","message":{"role":"user","content":"<hc from=\"t4 review\" reach=\"hc send t1 when done or stuck\">Review x.rs</hc>"}}"#.to_string(),
    ];
    assert_eq!(
        describe(&parse(&lines)),
        [
            format!("user:{shown}"),
            "user:<hc from=\"t3\">Short note</hc>".to_string(),
            "user:<hc from=\"t4 review\" reach=\"hc send t1 when done or stuck\">Review x.rs</hc>".to_string(),
        ],
        "hc: Codex and Claude Code notes, pasted or not, are user items"
    );
}

#[test]
fn codex_rollout() {
    let cx = fixture_lines("codex.jsonl");
    assert_eq!(
        describe(&parse(&cx)),
        ["user:Review the parser", "asst:Checking the files.", "chips:Ran 2 commands", "asst:No P1 found.", "perm:rm -rf build"],
        "codex maps lines"
    );
}

#[test]
fn tool_call_details() {
    let lines = fixture_lines("session.jsonl");
    let calls = chip_calls(&parse(&lines));
    assert_eq!(calls.iter().map(|c| c.tool_name.as_str()).collect::<Vec<_>>(), ["Read", "Read", "Bash"], "calls: tool names");
    assert!(calls[0].summary == "/a.swift" && calls[0].output.as_deref() == Some("x") && !calls[0].is_error, "calls: Read summary + output");
    assert!(calls[0].input.contains("file_path"), "calls: input is full JSON");
    let mut late = ConversationParser::new();
    late.feed(r#"{"type":"assistant","uuid":"l1","message":{"content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls","description":"List"}},{"type":"tool_use","id":"g1","name":"Task","input":{"description":"Sub job","prompt":"p"}}]}}"#);
    late.feed(r#"{"type":"assistant","uuid":"l2","message":{"content":[{"type":"text","text":"Later text"}]}}"#);
    let early = chip_calls(&late.items());
    assert!(early.iter().all(|c| c.output.is_none()) && early.len() == 2, "calls: no output before result");
    let big = "x".repeat(9000);
    late.feed(&format!(
        r#"{{"type":"user","uuid":"l3","message":{{"content":[{{"type":"tool_result","tool_use_id":"b1","is_error":true,"content":[{{"type":"text","text":"boom"}}]}},{{"type":"tool_result","tool_use_id":"g1","content":"{big}"}}]}}}}"#
    ));
    let lc = chip_calls(&late.items());
    assert!(lc[0].summary == "ls" && lc[0].output.as_deref() == Some("boom") && lc[0].is_error, "calls: late result updates earlier chip");
    let out = lc[1].output.as_deref().unwrap();
    assert!(lc[1].summary == "Sub job" && out.len() <= 4200 && out.ends_with('…'), "calls: Task summary, output truncated");
    let cxc = chip_calls(&parse(&fixture_lines("codex.jsonl")));
    assert!(
        cxc.len() == 2 && cxc[0].output.as_deref() == Some("ok") && cxc[1].summary == "bash -lc swift test" && cxc[1].output.as_deref() == Some("done"),
        "calls: codex mapped"
    );
    assert_eq!(parse(&lines), parse(&lines), "items() equal across calls (stable chip ids)");
}

#[test]
fn timestamps() {
    let items = parse(&[
        r#"{"type":"user","uuid":"t1","timestamp":"2026-10-09T01:02:03.456Z","message":{"content":"hello"}}"#.into(),
        r#"{"type":"assistant","uuid":"t2","timestamp":"2026-10-09T01:02:10.000Z","message":{"content":[{"type":"tool_use","id":"x1","name":"Read","input":{"file_path":"/a"}}]}}"#.into(),
        r#"{"type":"assistant","uuid":"t3","timestamp":"2026-10-09T01:02:12Z","message":{"content":[{"type":"tool_use","id":"x2","name":"Read","input":{"file_path":"/b"}}]}}"#.into(),
        r#"{"type":"user","uuid":"t4","timestamp":"2026-10-09T01:02:13.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"x1","content":"ok"}]}}"#.into(),
        r#"{"type":"assistant","uuid":"t5","timestamp":"2026-10-09T01:20:00.000Z","message":{"content":[{"type":"text","text":"done"}]}}"#.into(),
        r#"{"type":"user","uuid":"t6","message":{"content":"no stamp"}}"#.into(),
    ]);
    assert_eq!(items.len(), 4, "timestamp: fixture parses to 4 items");
    assert!(near(items[0].timestamp, 1791507723.456), "timestamp: user text carries the line time (fraction)");
    assert_eq!(items[1].timestamp, at(1791507730.0), "timestamp: chips item uses the first call's time");
    assert_eq!(items[2].timestamp, at(1791508800.0), "timestamp: assistant text carries the line time");
    assert!(items.last().unwrap().timestamp.is_none(), "timestamp: none when the line has none");
    let cx = parse(&[
        r#"{"timestamp":"2026-10-09T03:00:00.250Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi codex"}]}}"#.into(),
        r#"{"timestamp":"2026-10-09T03:00:05.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}}"#.into(),
    ]);
    assert!(cx.len() == 2 && near(cx[0].timestamp, 1791514800.25) && near(cx[1].timestamp, 1791514805.0), "timestamp: codex lines");
    let pend = parse(&[r#"{"type":"assistant","uuid":"p1","timestamp":"2026-10-09T04:00:00.000Z","message":{"content":[{"type":"tool_use","id":"pp","name":"Bash","input":{"command":"ls"}}]}}"#.into()]);
    assert!(matches!(pend[0].kind, ItemKind::PermissionRequest(_)) && pend[0].timestamp == at(1791518400.0), "timestamp: pending permission request");
}

#[test]
fn open_tool_use_and_prompt_marker() {
    let lines = fixture_lines("session.jsonl");
    assert!(herdr_core::screen_shows_prompt(" Do you want to proceed?\n ❯ 1. Yes") && !herdr_core::screen_shows_prompt("Running tests…"), "screen marker");
    let mut p = ConversationParser::new();
    for l in &lines {
        p.feed(l);
    }
    assert!(p.has_open_tool_use(), "parser: open tool use at end of fixture");
    let mut closed = ConversationParser::new();
    for l in &lines[..5] {
        closed.feed(l);
    }
    assert!(!closed.has_open_tool_use(), "parser: no open tool use after results");
}

#[test]
fn cut_on_char_boundary() {
    let c1 = cut(&"é".repeat(3000));
    let body1 = c1.strip_suffix('…').expect("cut: suffix");
    assert!(body1.len() == 4096 && body1.chars().all(|c| c == 'é'), "cut: 2-byte chars end on a boundary");
    let c2 = cut(&"a😀".repeat(2000));
    let body2 = c2.strip_suffix('…').unwrap();
    assert!(body2.len() <= 4096 && body2.len() > 4090 && !c2.contains('\u{FFFD}'), "cut: 4-byte chars end on a boundary");
    assert_eq!(cut("short"), "short", "cut: short text unchanged");
}

#[test]
fn pi_tree() {
    let pl = fixture_lines("pi.jsonl");
    let before = parse(&pl[..14]);
    assert_eq!(
        describe(&before),
        ["user:List the files.", "asst:Looking.", "chips:Ran a command|Read a file|Edited a file", "asst:Done with a.txt.", "user:Say something wrong.", "status"],
        "pi: items before the branch"
    );
    assert!(matches!(&before.last().unwrap().kind, ItemKind::Status(s) if s == "Error: Rate limited."), "pi: error stop as status");
    let items = parse(&pl);
    assert_eq!(
        describe(&items),
        ["user:List the files.", "asst:Looking.", "chips:Ran a command|Read a file|Edited a file", "asst:Done with a.txt.", "user:Run pwd yourself.", "chips:Ran a command", "status", "status", "asst:Building.", "perm:make"],
        "pi: newest branch only, edited turn dropped"
    );
    let chips = |i: usize| match &items[i].kind {
        ItemKind::ToolChips(c) => c.clone(),
        _ => vec![],
    };
    let calls: Vec<_> = chips(2).into_iter().flat_map(|c| c.calls).collect();
    assert_eq!(calls.iter().map(|c| c.tool_name.as_str()).collect::<Vec<_>>(), ["Bash", "Read", "Edit"]);
    assert_eq!(calls.iter().map(|c| c.output.as_deref().unwrap()).collect::<Vec<_>>(), ["a.txt", "hello", "no match"]);
    assert_eq!(calls.iter().map(|c| c.is_error).collect::<Vec<_>>(), [false, false, true]);
    assert_eq!(calls[1].summary, "/tmp/demo/a.txt", "pi: tool calls with results and errors");
    let c5 = &chips(5)[0].calls[0];
    assert!(c5.output.as_deref() == Some("/tmp/demo") && c5.summary == "pwd", "pi: user ! command as a chip");
    assert!(!items.iter().any(|i| matches!(&i.kind, ItemKind::AssistantText(t) if t.contains("Hidden"))), "pi: thinking hidden");
    let ids: Vec<_> = items.iter().map(|i| i.id.clone()).collect();
    assert!(ids == parse(&pl).iter().map(|i| i.id.clone()).collect::<Vec<_>>() && items[0].id == "u1-0", "pi: stable ids from entry ids");
    let mut p = ConversationParser::new();
    for l in &pl {
        p.feed(l);
    }
    assert!(p.has_open_tool_use() && p.items().iter().map(|i| i.id.clone()).collect::<Vec<_>>() == ids, "pi: incremental feed = batch, open tool call");
}

#[test]
fn cursor_transcript() {
    let cl = fixture_lines("cursor.jsonl");
    let items = parse(&cl);
    assert_eq!(
        describe(&items),
        ["user:List the files.", "asst:Looking.", "chips:Ran a command|Read a file|Edited a file", "asst:Done.", "user:What is this?", "chips:Used a connector", "status", "user:Plain prompt.", "status"],
        "cursor: items"
    );
    assert!(items[0].timestamp == at(1789200540.0) && items[4].timestamp == at(1789192800.0), "cursor: <timestamp> with UTC offset");
    let mut p = ConversationParser::new();
    let mut off = 0u64;
    for l in &cl {
        p.feed_at(l, Some(off));
        off += l.len() as u64 + 1;
    }
    assert!(!p.has_open_tool_use() && !p.items().iter().any(|i| matches!(i.kind, ItemKind::PermissionRequest(_))), "cursor: calls never pending");
    let ids: Vec<_> = p.items().iter().map(|i| i.id.clone()).collect();
    let uniq: std::collections::HashSet<_> = ids.iter().collect();
    assert!(uniq.len() == ids.len() && ids.iter().all(|i| i.contains("cu@")), "cursor: ids from byte offsets");
    if let ItemKind::ToolChips(c) = &items[2].kind {
        let s: Vec<_> = c.iter().flat_map(|c| c.calls.iter().map(|c| c.summary.clone())).collect();
        assert_eq!(s, ["ls", "/tmp/demo/a.txt", "/tmp/demo/a.txt"], "cursor: call summaries");
    } else {
        panic!("chips expected");
    }
}

#[test]
fn question_state_type_is_reexported() {
    // Compile-time check that the public surface the app needs is reachable from the crate root.
    let _ = QuestionState::Pending;
}

#[test]
fn pending_bash_carries_its_description() {
    let detail = |line: &str| match &parse(&[line.into()]).last().unwrap().kind {
        ItemKind::PermissionRequest(r) => r.detail.clone(),
        _ => panic!("pending permission request"),
    };
    let with = r#"{"type":"assistant","uuid":"d1","message":{"content":[{"type":"tool_use","id":"dd","name":"Bash","input":{"command":"npm test","description":"Run the tests"}}]}}"#;
    assert_eq!(detail(with), "Run the tests");
    let without = r#"{"type":"assistant","uuid":"d2","message":{"content":[{"type":"tool_use","id":"de","name":"Bash","input":{"command":"npm test"}}]}}"#;
    assert_eq!(detail(without), "");
    let edit = r#"{"type":"assistant","uuid":"d3","message":{"content":[{"type":"tool_use","id":"df","name":"Edit","input":{"file_path":"a.rs","description":"x"}}]}}"#;
    assert_eq!(detail(edit), "", "only Bash shows a description under the command");
}

#[test]
fn pending_request_counts_the_open_calls() {
    let open = |lines: &[&str]| match &parse(&lines.iter().map(|l| l.to_string()).collect::<Vec<_>>()).last().unwrap().kind {
        ItemKind::PermissionRequest(r) => r.open_calls,
        _ => panic!("pending permission request"),
    };
    let one = r#"{"type":"assistant","uuid":"o1","message":{"content":[{"type":"tool_use","id":"oa","name":"Bash","input":{"command":"ls"}}]}}"#;
    let two = r#"{"type":"assistant","uuid":"o2","message":{"content":[{"type":"tool_use","id":"oa","name":"Bash","input":{"command":"ls"}},{"type":"tool_use","id":"ob","name":"Bash","input":{"command":"pwd"}}]}}"#;
    assert_eq!(open(&[one]), 1);
    assert_eq!(open(&[two]), 2, "parallel calls");
}
