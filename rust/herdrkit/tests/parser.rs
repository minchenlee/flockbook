//! Parity with the Swift checks (`Sources/HerdrKitChecks`) that exercise `ConversationParser` and `LogImage`.
//! The fixtures are the Swift checks' own (`Tests/HerdrKitTests/Fixtures`), synthetic. One `check!` per Swift `check`.

use std::collections::HashSet;

use base64::Engine;
use chrono::{DateTime, Utc};
use herdrkit::log_image::{self, Format, Ref};
use herdrkit::parser::{self, ClaudeQueueLog, ConversationParser};
use herdrkit::{ConversationItem, Delivery, ItemKind, PermissionState, QuestionRequest, QuestionState, ToolCall};
use serde_json::{json, Value};

macro_rules! check {
    ($cond:expr, $($msg:tt)+) => { assert!($cond, $($msg)+) };
}

fn fixture(name: &str) -> Vec<String> {
    let path = format!("{}/../../Tests/HerdrKitTests/Fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    text.split('\n').filter(|l| !l.is_empty()).map(str::to_string).collect()
}

fn parse<S: AsRef<str>>(lines: &[S]) -> Vec<ConversationItem> {
    let mut p = ConversationParser::new();
    for l in lines {
        p.feed_line(l.as_ref());
    }
    p.items()
}

fn describe(items: &[ConversationItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| match &i.kind {
            ItemKind::UserText(t) => format!("user:{t}"),
            ItemKind::AssistantText(t) => format!("asst:{t}"),
            ItemKind::ToolChips(c) => format!("chips:{}", c.iter().map(|x| x.text.as_str()).collect::<Vec<_>>().join("|")),
            ItemKind::PermissionRequest(p) => format!("perm:{}", p.command),
            ItemKind::Question(q) => format!("q:{}", q.questions.iter().map(|x| x.header.as_str()).collect::<Vec<_>>().join("|")),
            ItemKind::Status(_) => "status".into(),
        })
        .collect()
}

fn chip_calls(items: &[ConversationItem]) -> Vec<ToolCall> {
    items
        .iter()
        .flat_map(|i| match &i.kind {
            ItemKind::ToolChips(c) => c.iter().flat_map(|x| x.calls.clone()).collect(),
            _ => vec![],
        })
        .collect()
}

fn ids(items: &[ConversationItem]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

fn unique(ids: &[String]) -> bool {
    ids.iter().collect::<HashSet<_>>().len() == ids.len()
}

fn has_perm(items: &[ConversationItem]) -> bool {
    items.iter().any(|i| matches!(i.kind, ItemKind::PermissionRequest(_)))
}

fn question_at(items: &[ConversationItem], i: usize) -> Option<QuestionRequest> {
    match &items.get(i)?.kind {
        ItemKind::Question(q) => Some(q.clone()),
        _ => None,
    }
}

fn first_question(items: &[ConversationItem]) -> Option<QuestionRequest> {
    items.iter().find_map(|i| if let ItemKind::Question(q) = &i.kind { Some(q.clone()) } else { None })
}

fn answered(a: &[&str]) -> QuestionState {
    QuestionState::Answered(Some(a.iter().map(|s| s.to_string()).collect()))
}

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn out(c: &ToolCall) -> String {
    c.output.clone().unwrap_or_else(|| "nil".into())
}

// MARK: ChecksCore

#[test]
fn conversation() {
    let lines = fixture("session.jsonl");
    check!(
        describe(&parse(&lines))
            == ["user:Fix the login bug", "asst:Looking at it now.", "chips:Read 2 files|Ran a command", "asst:Tests pass.", "perm:rm -rf build"],
        "maps lines {:?}",
        describe(&parse(&lines))
    );
    let tail = parse(&lines);
    check!(describe(&tail).last().map(String::as_str) == Some("perm:rm -rf build") && tail.len() == 5, "unanswered tail tool_use becomes permissionRequest");
    match &tail.last().unwrap().kind {
        ItemKind::PermissionRequest(r) => check!(
            r.state == PermissionState::Pending && r.tool_name == "Bash" && r.tool_use_id == "t9",
            "permissionRequest is pending with tool info"
        ),
        _ => check!(false, "last item is permissionRequest"),
    }
    let answered = parse(&lines[..5]);
    check!(!describe(&answered).iter().any(|d| d.starts_with("perm:")), "answered tool_use emits no permissionRequest");
    check!(ids(&parse(&lines)) == ids(&parse(&lines)), "stable ids");
}

#[test]
fn injected() {
    let inj = fixture("injected.jsonl");
    check!(
        describe(&parse(&inj))
            == [
                "user:Real question",
                "status",
                "status",
                "chips:Read a file",
                "user:[herdr-send from orchard-main-orch] please rebuild",
                "asst:On it.",
            ],
        "injected user lines {:?}",
        describe(&parse(&inj))
    );
    match &parse(&inj)[1].kind {
        ItemKind::Status(t) => check!(t == "Context compacted", "compaction status text"),
        _ => check!(false, "compaction is a status item"),
    }
}

#[test]
fn queued() {
    let q = fixture("queued.jsonl");
    let got = describe(&parse(&q));
    check!(
        got == [
            "user:Run the tests, then summarize.",
            "chips:Ran a command",
            "user:Also check the lint step.",
            "user:Here is the failing screen.",
            "user:An older client logs no origin.",
            "asst:Tests pass and lint is clean.",
            "user:Thanks.",
        ],
        "queued: messages taken mid-turn show as user text, in log order ({got:?})"
    );
    let i = ids(&parse(&q));
    check!(i == ids(&parse(&q)) && unique(&i), "queued: ids stable and unique");
}

#[test]
fn pasted() {
    let p = fixture("pasted.jsonl");
    check!(
        describe(&parse(&p))
            == [
                "user:Ship the Orchard banner today.",
                "user:Please review this:\n\nline A\nline B\n\nThanks",
                "user:first\n\nsecond",
                "user:Relay: <pasted_content id=\"3\">inner</pasted_content id=\"3\"> end",
                "user:block form",
                "user:cut off at the end",
            ],
        "pasted: shown as user text, wrappers stripped, order kept, injected tags still hidden {:?}",
        describe(&parse(&p))
    );
    check!(
        parser::user_visible("<environment_context>cwd</environment_context>").is_none()
            && parser::user_visible("[Request interrupted by user]").is_none()
            && parser::user_visible("typed <b>text</b>").as_deref() == Some("typed <b>text</b>"),
        "pasted: allowlist of visible tags"
    );
    let cxp = parse(&[
        r#"{"timestamp":"2026-10-10T01:00:00.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<pasted_content id=\"1\">codex paste</pasted_content id=\"1\">"}]}}"#,
        r#"{"timestamp":"2026-10-10T01:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>cwd</environment_context>"}]}}"#,
    ]);
    check!(describe(&cxp) == ["user:codex paste"], "pasted: codex user text uses the same rule");
}

#[test]
fn codex_rollout() {
    let cx = fixture("codex.jsonl");
    check!(
        describe(&parse(&cx))
            == ["user:Review the parser", "asst:Checking the files.", "chips:Ran 2 commands", "asst:No P1 found.", "perm:rm -rf build"],
        "codex maps lines {:?}",
        describe(&parse(&cx))
    );
}

#[test]
fn tool_calls() {
    let lines = fixture("session.jsonl");
    let calls = chip_calls(&parse(&lines));
    check!(calls.iter().map(|c| c.tool_name.as_str()).collect::<Vec<_>>() == ["Read", "Read", "Bash"], "calls: tool names");
    check!(
        calls[0].summary == "/a.swift" && calls[0].output.as_deref() == Some("x") && !calls[0].is_error,
        "calls: Read summary + output"
    );
    check!(calls[0].input.contains("file_path"), "calls: input is full JSON");
    let mut late = ConversationParser::new();
    late.feed_line(r#"{"type":"assistant","uuid":"l1","message":{"content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"ls","description":"List"}},{"type":"tool_use","id":"g1","name":"Task","input":{"description":"Sub job","prompt":"p"}}]}}"#);
    late.feed_line(r#"{"type":"assistant","uuid":"l2","message":{"content":[{"type":"text","text":"Later text"}]}}"#);
    check!(
        chip_calls(&late.items()).iter().all(|c| c.output.is_none()) && chip_calls(&late.items()).len() == 2,
        "calls: no output before result"
    );
    late.feed_line(&format!(
        r#"{{"type":"user","uuid":"l3","message":{{"content":[{{"type":"tool_result","tool_use_id":"b1","is_error":true,"content":[{{"type":"text","text":"boom"}}]}},{{"type":"tool_result","tool_use_id":"g1","content":"{}"}}]}}}}"#,
        "x".repeat(9000)
    ));
    let lc = chip_calls(&late.items());
    check!(
        lc[0].summary == "ls" && lc[0].output.as_deref() == Some("boom") && lc[0].is_error,
        "calls: late result updates earlier chip"
    );
    let last = lc.last().unwrap();
    check!(
        last.summary == "Sub job"
            && last.output.as_ref().map(|o| o.len()).unwrap_or(0) <= 4200
            && last.output.as_ref().map(|o| o.ends_with('…')) == Some(true),
        "calls: Task summary, output truncated"
    );
    let cx = chip_calls(&parse(&fixture("codex.jsonl")));
    check!(
        cx.len() == 2 && cx[0].output.as_deref() == Some("ok") && cx[1].summary == "bash -lc swift test" && cx[1].output.as_deref() == Some("done"),
        "calls: codex mapped {cx:?}"
    );
    check!(parse(&lines) == parse(&lines), "items() equal across calls (stable chip ids)");
}

#[test]
fn thinking_before_tool() {
    let mut p = ConversationParser::new();
    for l in [
        r#"{"type":"assistant","uuid":"k0","message":{"id":"m1","content":[{"type":"thinking","thinking":"","signature":"s"}]}}"#,
        r#"{"type":"assistant","uuid":"k1","message":{"id":"m1","content":[{"type":"thinking","thinking":"Shown before the tool","signature":"s"}]}}"#,
        r#"{"type":"assistant","uuid":"k2","message":{"id":"m1","content":[{"type":"tool_use","id":"tm","name":"mcp__memes__meme","input":{"meme":"x"}}]}}"#,
        r#"{"type":"assistant","uuid":"k3","message":{"id":"m2","content":[{"type":"thinking","thinking":"Private reasoning","signature":"s"}]}}"#,
        r#"{"type":"assistant","uuid":"k4","message":{"id":"m2","content":[{"type":"text","text":"Answer"}]}}"#,
        r#"{"type":"assistant","uuid":"k5","message":{"id":"m3","content":[{"type":"thinking","thinking":"Other message's reasoning","signature":"s"}]}}"#,
        r#"{"type":"assistant","uuid":"k6","message":{"id":"m4","content":[{"type":"tool_use","id":"tb","name":"Bash","input":{"command":"ls"}}]}}"#,
        r#"{"type":"assistant","uuid":"k7","message":{"id":"m5","content":[{"type":"thinking","thinking":"Same line","signature":"s"},{"type":"tool_use","id":"tc","name":"Bash","input":{"command":"pwd"}}]}}"#,
    ] {
        p.feed_line(l);
    }
    let i = p.items();
    let texts: Vec<&str> =
        i.iter().filter_map(|x| if let ItemKind::AssistantText(t) = &x.kind { Some(t.as_str()) } else { None }).collect();
    check!(texts == ["Shown before the tool", "Answer", "Same line"], "thinking: shown only right before its message's tool call {texts:?}");
    let shown = i.iter().position(|x| x.kind == ItemKind::AssistantText("Shown before the tool".into()));
    check!(
        shown.is_some_and(|s| s + 1 < i.len() && matches!(i[s + 1].kind, ItemKind::ToolChips(_))),
        "thinking: the text comes before the tool chips"
    );
}

#[test]
fn timestamps() {
    let items = parse(&[
        r#"{"type":"user","uuid":"t1","timestamp":"2026-10-09T01:02:03.456Z","message":{"content":"hello"}}"#,
        r#"{"type":"assistant","uuid":"t2","timestamp":"2026-10-09T01:02:10.000Z","message":{"content":[{"type":"tool_use","id":"x1","name":"Read","input":{"file_path":"/a"}}]}}"#,
        r#"{"type":"assistant","uuid":"t3","timestamp":"2026-10-09T01:02:12Z","message":{"content":[{"type":"tool_use","id":"x2","name":"Read","input":{"file_path":"/b"}}]}}"#,
        r#"{"type":"user","uuid":"t4","timestamp":"2026-10-09T01:02:13.000Z","message":{"content":[{"type":"tool_result","tool_use_id":"x1","content":"ok"}]}}"#,
        r#"{"type":"assistant","uuid":"t5","timestamp":"2026-10-09T01:20:00.000Z","message":{"content":[{"type":"text","text":"done"}]}}"#,
        r#"{"type":"user","uuid":"t6","message":{"content":"no stamp"}}"#,
    ]);
    check!(items.len() == 4, "timestamp: fixture parses to 4 items ({})", items.len());
    let d = items[0].timestamp.map(|t| (t - at("2026-10-09T01:02:03.456Z")).num_microseconds().unwrap().abs());
    check!(d.is_some_and(|d| d < 2000), "timestamp: user text carries the line time (fraction)");
    check!(items[1].timestamp == Some(at("2026-10-09T01:02:10.000Z")), "timestamp: chips item uses the first call's time");
    check!(items[2].timestamp == Some(at("2026-10-09T01:20:00.000Z")), "timestamp: assistant text carries the line time");
    check!(items.last().unwrap().timestamp.is_none(), "timestamp: nil when the line has none");
    let cx = parse(&[
        r#"{"timestamp":"2026-10-09T03:00:00.250Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi codex"}]}}"#,
        r#"{"timestamp":"2026-10-09T03:00:05.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}}"#,
    ]);
    check!(
        cx.len() == 2 && cx[0].timestamp == Some(at("2026-10-09T03:00:00.250Z")) && cx[1].timestamp == Some(at("2026-10-09T03:00:05.000Z")),
        "timestamp: codex lines"
    );
    let pend = parse(&[
        r#"{"type":"assistant","uuid":"p1","timestamp":"2026-10-09T04:00:00.000Z","message":{"content":[{"type":"tool_use","id":"pp","name":"Bash","input":{"command":"ls"}}]}}"#,
    ]);
    check!(
        matches!(pend[0].kind, ItemKind::PermissionRequest(_)) && pend[0].timestamp == Some(at("2026-10-09T04:00:00.000Z")),
        "timestamp: pending permission request"
    );
}

#[test]
fn prompt_detection() {
    let lines = fixture("session.jsonl");
    let mut p = ConversationParser::new();
    for l in &lines {
        p.feed_line(l);
    }
    check!(p.has_open_tool_use(), "parser: open tool use at end of fixture");
    let mut closed = ConversationParser::new();
    for l in &lines[..5] {
        closed.feed_line(l);
    }
    check!(!closed.has_open_tool_use(), "parser: no open tool use after results");
}

#[test]
fn cut() {
    let c1 = parser::cut(&"é".repeat(3000));
    let c2 = parser::cut(&"a😀".repeat(2000));
    let b1 = c1.strip_suffix('…').unwrap_or("");
    check!(c1.ends_with('…') && b1.len() == 4096 && b1.chars().all(|c| c == 'é'), "cut: 2-byte chars end on a boundary");
    let b2 = c2.strip_suffix('…').unwrap_or(&c2);
    check!(b2.len() <= 4096 && b2.len() > 4090 && !c2.contains('\u{FFFD}'), "cut: 4-byte chars end on a boundary");
    check!(parser::cut("short") == "short", "cut: short text unchanged");
}

// MARK: ChecksAgents

#[test]
fn pi() {
    let pl = fixture("pi.jsonl");
    let before = parse(&pl[..14]);
    check!(
        describe(&before)
            == ["user:List the files.", "asst:Looking.", "chips:Ran a command|Read a file|Edited a file", "asst:Done with a.txt.", "user:Say something wrong.", "status"],
        "pi: items before the branch {:?}",
        describe(&before)
    );
    check!(before.last().unwrap().kind == ItemKind::Status("Error: Rate limited.".into()), "pi: error stop as status");
    let items = parse(&pl);
    check!(
        describe(&items)
            == [
                "user:List the files.",
                "asst:Looking.",
                "chips:Ran a command|Read a file|Edited a file",
                "asst:Done with a.txt.",
                "user:Run pwd yourself.",
                "chips:Ran a command",
                "status",
                "status",
                "asst:Building.",
                "perm:make",
            ],
        "pi: newest branch only, edited turn dropped {:?}",
        describe(&items)
    );
    let chips = |i: usize| match &items[i].kind {
        ItemKind::ToolChips(c) => c.clone(),
        _ => vec![],
    };
    let calls: Vec<ToolCall> = chips(2).into_iter().flat_map(|c| c.calls).collect();
    check!(
        calls.iter().map(|c| c.tool_name.as_str()).collect::<Vec<_>>() == ["Bash", "Read", "Edit"]
            && calls.iter().map(out).collect::<Vec<_>>() == ["a.txt", "hello", "no match"]
            && calls.iter().map(|c| c.is_error).collect::<Vec<_>>() == [false, false, true]
            && calls[1].summary == "/tmp/demo/a.txt",
        "pi: tool calls with results and errors"
    );
    let c5 = chips(5);
    check!(
        c5[0].calls[0].output.as_deref() == Some("/tmp/demo") && c5[0].calls[0].summary == "pwd",
        "pi: user ! command as a chip"
    );
    check!(
        !items.iter().any(|i| matches!(&i.kind, ItemKind::AssistantText(t) if t.contains("Hidden"))),
        "pi: thinking hidden"
    );
    check!(ids(&items) == ids(&parse(&pl)) && items[0].id == "u1-0", "pi: stable ids from entry ids");
    let mut p = ConversationParser::new();
    for l in &pl {
        p.feed_line(l);
    }
    check!(p.has_open_tool_use() && ids(&p.items()) == ids(&items), "pi: incremental feed = batch, open tool call");
}

#[test]
fn cursor() {
    let cl = fixture("cursor.jsonl");
    let items = parse(&cl);
    check!(
        describe(&items)
            == [
                "user:List the files.",
                "asst:Looking.",
                "chips:Ran a command|Read a file|Edited a file",
                "asst:Done.",
                "user:What is this?",
                "chips:Used a connector",
                "status",
                "user:Plain prompt.",
                "status",
            ],
        "cursor: items {:?}",
        describe(&items)
    );
    check!(
        items[0].timestamp == Some(at("2026-09-12T08:09:00Z")) && items[4].timestamp == Some(at("2026-09-12T06:00:00Z")),
        "cursor: <timestamp> with UTC offset"
    );
    let mut p = ConversationParser::new();
    let mut off = 0u64;
    for l in &cl {
        p.feed(l, Some(off));
        off += l.len() as u64 + 1;
    }
    check!(!p.has_open_tool_use() && !has_perm(&p.items()), "cursor: calls never pending");
    let i = ids(&p.items());
    check!(unique(&i) && i.iter().all(|x| x.contains("cu@")), "cursor: ids from byte offsets");
    if let ItemKind::ToolChips(c) = &items[2].kind {
        check!(
            c.iter().flat_map(|x| x.calls.iter().map(|c| c.summary.as_str())).collect::<Vec<_>>() == ["ls", "/tmp/demo/a.txt", "/tmp/demo/a.txt"],
            "cursor: call summaries"
        );
    }
}

// MARK: ChecksAgy

#[test]
fn agy_parser() {
    let al = fixture("agy.jsonl");
    let items = parse(&al);
    check!(
        describe(&items)
            == [
                "user:List the files.",
                "asst:Looking.",
                "chips:Ran a command|Read a file|Edited a file",
                "asst:Done with a.txt.",
                "status",
                "user:Build it.",
                "asst:Building.",
                "perm:make",
            ],
        "agy: items {:?}",
        describe(&items)
    );
    let calls = chip_calls(&items);
    check!(
        calls.iter().map(|c| c.tool_name.as_str()).collect::<Vec<_>>() == ["Bash", "Read", "Edit"]
            && calls.iter().map(|c| c.summary.as_str()).collect::<Vec<_>>() == ["ls", "/tmp/demo/a.txt", "/tmp/demo/a.txt"],
        "agy: tool names and summaries"
    );
    check!(
        calls.iter().map(|c| c.is_error).collect::<Vec<_>>() == [false, false, true]
            && out(&calls[0]).ends_with("a.txt")
            && out(&calls[1]).ends_with("hello")
            && calls[2].output.as_deref() == Some("no match")
            && !out(&calls[0]).contains("Created At"),
        "agy: results by order, header lines cut {:?}",
        calls.iter().map(out).collect::<Vec<_>>()
    );
    check!(
        !items.iter().any(|i| matches!(&i.kind, ItemKind::AssistantText(t) if t.contains("Hidden") || t.contains("SYSTEM_MESSAGE"))),
        "agy: thinking and system messages hidden"
    );
    check!(items[0].timestamp == Some(at("2026-11-02T02:00:00Z")), "agy: created_at as time");
    if let ItemKind::Status(s) = &items[4].kind {
        check!(s == "Context compacted", "agy: checkpoint is a compaction marker");
    }
    if let ItemKind::PermissionRequest(r) = &items.last().unwrap().kind {
        check!(r.tool_name == "Bash" && r.title == "Wants to run a command", "agy: pending call as a request");
    }
    let mut p = ConversationParser::new();
    for l in &al {
        p.feed_line(l);
    }
    check!(p.has_open_tool_use() && ids(&p.items()) == ids(&items), "agy: incremental feed = batch, open call");
    check!(ids(&parse(&al)) == ids(&items) && unique(&ids(&items)), "agy: stable unique ids");
    let bg = r#"{"step_index":12,"source":"MODEL","type":"GENERIC","status":"RUNNING","created_at":"2026-11-02T02:00:12Z","content":"Created At: x\nTool is running as a background task with task id: c/task-12"}"#;
    let mut with_bg = al.clone();
    with_bg.push(bg.into());
    check!(!has_perm(&parse(&with_bg)), "agy: background task closes the call");
    let plain = r#"{"step_index":0,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-02T02:00:00Z","content":"hi"}"#;
    let tagged = r#"{"step_index":1,"source":"USER_EXPLICIT","type":"USER_INPUT","status":"DONE","created_at":"2026-11-02T02:00:00Z","content":"<USER_SETTINGS_CHANGE>x</USER_SETTINGS_CHANGE>"}"#;
    check!(describe(&parse(&[plain, tagged])) == ["user:hi"], "agy: plain prompt kept, tagged-only dropped");
}

#[test]
fn agy_order() {
    let al = fixture("agy-order.jsonl");
    let items = parse(&al);
    let calls = chip_calls(&items);
    check!(
        calls.iter().map(out).collect::<Vec<_>>() == ["A", "B", "Not run", "D", "task", "X", "Y", "Not run"]
            && calls.iter().map(|c| c.is_error).collect::<Vec<_>>() == [false, false, true, false, false, false, false, true],
        "agy: results by step_index {:?}",
        calls.iter().map(out).collect::<Vec<_>>()
    );
    check!(!has_perm(&items), "agy: no false pending call {:?}", describe(&items));
    let qs: Vec<QuestionState> =
        items.iter().filter_map(|i| if let ItemKind::Question(q) = &i.kind { Some(q.state.clone()) } else { None }).collect();
    check!(qs == [answered(&["Blue"])], "agy: the question takes its own result {qs:?}");
    let mut p = ConversationParser::new();
    for l in &al {
        p.feed_line(l);
    }
    check!(!p.has_open_tool_use() && ids(&p.items()) == ids(&items), "agy: incremental feed = batch, nothing open");
    let mut late_lines = al.clone();
    late_lines.push(r#"{"step_index":6,"source":"MODEL","type":"GENERIC","status":"DONE","created_at":"2026-11-02T03:00:19Z","content":"C"}"#.into());
    let late = chip_calls(&parse(&late_lines));
    check!(late[2].output.as_deref() == Some("C") && !late[2].is_error, "agy: late result fills the call");

    let il = fixture("agy-injected.jsonl");
    let outs = |lines: &[String]| -> Vec<String> {
        chip_calls(&parse(lines)).iter().map(|c| format!("{}{}", out(c), if c.is_error { "!" } else { "" })).collect()
    };
    let orders: [(&str, [usize; 10]); 4] = [
        ("in order", [0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
        ("injected step before the call", [0, 2, 1, 3, 4, 5, 7, 6, 8, 9]),
        ("result before the call", [0, 1, 2, 3, 4, 5, 7, 8, 6, 9]),
        ("result, call, then injected step", [0, 1, 2, 3, 4, 5, 8, 6, 7, 9]),
    ];
    for (name, order) in orders {
        let lines: Vec<String> = order.iter().map(|&i| il[i].clone()).collect();
        check!(outs(&lines) == ["a", "b", "c"], "agy: injected steps: {name} {:?}", outs(&lines));
    }
    let mut ip = ConversationParser::new();
    for l in &il {
        ip.feed_line(l);
    }
    check!(!ip.has_open_tool_use() && !has_perm(&ip.items()), "agy: injected steps: nothing left open");
}

fn agy_step(i: i64, ty: &str, extra: &str) -> String {
    let source = if ty == "USER_INPUT" { "USER_EXPLICIT" } else { "MODEL" };
    format!(r#"{{"step_index":{i},"source":"{source}","type":"{ty}","status":"DONE","created_at":"2026-11-02T02:00:2{i}Z",{extra}}}"#)
}

#[test]
fn agy_questions() {
    let ask = agy_step(
        5,
        "PLANNER_RESPONSE",
        r#""tool_calls":[{"name":"ask_question","args":{"questions":[{"is_multi_select":false,"options":["Red","Blue"],"question":"Which color do you prefer, red or blue?"}]}}]"#,
    );
    let head = vec![agy_step(4, "USER_INPUT", r#""content":"<USER_REQUEST>\nAsk me.\n</USER_REQUEST>""#), ask];
    let pending = parse(&head);
    let Some(ItemKind::Question(q)) = pending.last().map(|i| &i.kind) else { panic!("agy: ask_question is a question item {:?}", describe(&pending)) };
    check!(
        q.agent == "agy"
            && q.delivery == Delivery::Picker
            && q.state == QuestionState::Pending
            && q.questions.len() == 1
            && q.questions[0].text == "Which color do you prefer, red or blue?"
            && q.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && !q.questions[0].multi_select,
        "agy: question parsed"
    );
    check!(!has_perm(&pending), "agy: a question is never a permission request");
    let state = |result: &str| -> Option<QuestionState> {
        let mut lines = head.clone();
        lines.push(agy_step(6, "GENERIC", &format!(r#""content":"Created At: x\nCompleted At: y\n{result}""#)));
        first_question(&parse(&lines)).map(|q| q.state)
    };
    check!(state("A1: Blue") == Some(answered(&["Blue"])), "agy: answered");
    check!(state("A1: User Skipped") == Some(QuestionState::Declined), "agy: skipped is declined");
    let deny = vec![
        agy_step(8, "USER_INPUT", r#""content":"<USER_REQUEST>\nRun pwd\n</USER_REQUEST>""#),
        agy_step(9, "PLANNER_RESPONSE", r#""tool_calls":[{"name":"run_command","args":{"CommandLine":"pwd"}}]"#),
    ];
    check!(describe(&parse(&deny)).last().map(String::as_str) == Some("perm:pwd"), "agy: call pending before the next prompt");
    let mut more = deny.clone();
    more.push(agy_step(10, "USER_INPUT", r#""content":"<USER_REQUEST>\nNext.\n</USER_REQUEST>""#));
    let after = parse(&more);
    check!(!has_perm(&after) && chip_calls(&after)[0].is_error, "agy: next prompt closes the denied call {:?}", describe(&after));
}

// MARK: ChecksGrok

#[test]
fn grok() {
    let gl = fixture("grok.jsonl");
    let items = parse(&gl);
    check!(
        describe(&items)
            == [
                "user:List the files in /tmp/demo.",
                "asst:Looking.",
                "chips:Searched the code 2 times|Ran a command|Read a file|Edited a file",
                "asst:Done with a.txt.",
                "user:Line one.\nLine two.",
                "asst:Working.",
                "chips:Ran a command",
                "status",
                "user:Again.",
                "perm:make test",
            ],
        "grok: items {:?}",
        describe(&items)
    );
    check!(
        !items.iter().any(|i| matches!(&i.kind, ItemKind::AssistantText(t) if t.contains("Hidden"))),
        "grok: thinking hidden"
    );
    check!(items[0].timestamp == chrono::TimeZone::timestamp_opt(&Utc, 1_792_000_000, 500_000_000).single(), "grok: time from agentTimestampMs {:?}", items[0].timestamp);
    check!(items[7].kind == ItemKind::Status("Interrupted".into()), "grok: a cancelled turn says Interrupted");
    let calls = chip_calls(&items);
    check!(
        calls.iter().map(|c| c.tool_name.as_str()).collect::<Vec<_>>() == ["Grep", "Glob", "Bash", "Read", "Edit", "Bash"],
        "grok: tools under Claude Code's names"
    );
    check!(
        calls.iter().map(|c| c.summary.as_str()).collect::<Vec<_>>() == [".", "/tmp/demo", "ls", "/tmp/demo/a.txt", "/tmp/demo/a.txt", "make"],
        "grok: call summaries"
    );
    check!(
        calls.iter().map(|c| c.output.clone()).collect::<Vec<_>>()
            == [
                Some("found 1 matches".to_string()),
                Some("- /tmp/demo/\n  - a.txt".into()),
                Some("a.txt\n".into()),
                Some("hello".into()),
                Some("no match".into()),
                None
            ]
            && calls.iter().map(|c| c.is_error).collect::<Vec<_>>() == [false, false, false, false, true, false],
        "grok: results and a failed call"
    );
    check!(!calls[0].input.contains("variant"), "grok: the update's rawInput tag is not shown");

    let mut p = ConversationParser::new();
    let mut off = 0u64;
    let mut first_user = vec![];
    for (i, l) in gl.iter().enumerate() {
        p.feed(l, Some(off));
        off += l.len() as u64 + 1;
        if i == 1 || i == 2 {
            first_user.push(p.items().first().map(|x| x.id.clone()).unwrap_or_default());
        }
    }
    check!(ids(&p.items()) == ids(&items) && unique(&ids(&items)), "grok: incremental feed = batch, ids unique");
    check!(
        first_user.len() == 2
            && first_user[0] == first_user[1]
            && describe(&p.items()).first().map(String::as_str) == Some("user:List the files in /tmp/demo."),
        "grok: merged chunks keep the first id"
    );
    check!(p.has_open_tool_use(), "grok: the last call is open");
    let mut ended = gl.clone();
    ended.push(r#"{"timestamp":1792000009,"method":"_x.ai/session/update","params":{"sessionId":"s","update":{"sessionUpdate":"turn_completed","stop_reason":"end_turn"},"_meta":{"eventId":"s-99"}}}"#.into());
    check!(!has_perm(&parse(&ended)), "grok: turn end closes open calls");
}

fn grok_line(n: i64, update: &str) -> String {
    format!(
        r#"{{"timestamp":1792000100,"method":"session/update","params":{{"sessionId":"s","update":{update},"_meta":{{"eventId":"s-q{n}","agentTimestampMs":{}}}}}}}"#,
        1_792_000_100_000 + n
    )
}

#[test]
fn grok_question() {
    let user = grok_line(1, r#"{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Pick for me."}}"#);
    let ask = grok_line(2, r#"{"sessionUpdate":"tool_call","toolCallId":"call-q-0","title":"ask_user_question","rawInput":{"questions":[{"question":"Which color?","options":[{"label":"Red (Recommended)","description":"Warm."},{"label":"Blue","description":"Cool."}]},{"question":"Which size?","options":[{"label":"Small","description":"Little."},{"label":"Large","description":"Big."}]}]},"_meta":{"x.ai/tool":{"name":"ask_user_question","kind":"ask_user"}}}"#);
    let tagged = grok_line(3, r#"{"sessionUpdate":"tool_call_update","toolCallId":"call-q-0","kind":"other","rawInput":{"variant":"AskUserQuestion","questions":[]}}"#);
    let done = |n: i64, status: &str, rest: &str| {
        grok_line(n, &format!(r#"{{"sessionUpdate":"tool_call_update","toolCallId":"call-q-0","status":"{status}"{rest}}}"#))
    };
    let answered_line = done(4, "completed", r#","content":[{"type":"content","content":{"type":"text","text":"User has answered your questions: \"Which color?\"=\"Red\", \"Which size?\"=\"Other\" user notes: huge."}}],"rawOutput":{"type":"AskUserQuestion","UserAnswered":{"answers":[{"question":"Which color?","options":["Red","Blue"],"picked":["Red"]},{"question":"Which size?","options":["Small","Large"],"picked":[],"typed":"huge"}]}}"#);
    let text_only = done(4, "completed", r#","content":[{"type":"content","content":{"type":"text","text":"User has answered your questions: \"Which color?\"=\"Blue\", \"Which size?\"=\"Small\"."}}]"#);
    let failed = done(4, "failed", r#","content":[{"type":"content","content":{"type":"text","text":"dismissed"}}]"#);
    let cancelled = grok_line(5, r#"{"sessionUpdate":"turn_completed","stop_reason":"cancelled"}"#);

    let open = parse(&[&user, &ask, &tagged]);
    let q = first_question(&open).unwrap();
    check!(
        q.agent == "grok"
            && q.delivery == Delivery::Picker
            && q.state == QuestionState::Pending
            && q.tool_use_id == "call-q-0"
            && open.last().unwrap().id == "q-call-q-0",
        "grok question: a pending picker card, last item {:?}",
        describe(&open)
    );
    check!(
        q.questions.iter().map(|x| x.text.as_str()).collect::<Vec<_>>() == ["Which color?", "Which size?"]
            && q.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && q.questions[0].options[0].recommended
            && q.questions[0].options[0].detail == "Warm."
            && q.questions.iter().all(|x| !x.multi_select),
        "grok question: questions, labels, (Recommended)"
    );
    check!(chip_calls(&open).is_empty(), "grok question: no tool chip for the question");
    let st = |lines: &[&String]| first_question(&parse(lines)).map(|q| q.state);
    check!(st(&[&user, &ask, &tagged, &answered_line]) == Some(answered(&["Red", "huge"])), "grok question: picked and typed answers");
    check!(st(&[&user, &ask, &text_only]) == Some(answered(&["Blue", "Small"])), "grok question: answers from the result text");
    check!(st(&[&user, &ask, &failed]) == Some(QuestionState::Declined), "grok question: a failed call is declined");
    check!(st(&[&user, &ask, &cancelled]) == Some(QuestionState::Declined), "grok question: a cancelled turn declines it");
    let multi = parse(&[grok_line(2, r#"{"sessionUpdate":"tool_call","toolCallId":"call-m","title":"ask_user_question","rawInput":{"questions":[{"question":"Which fruits?","options":[{"label":"Apple"},{"label":"Pear"}],"multi_select":true}]},"_meta":{"x.ai/tool":{"name":"ask_user_question","kind":"ask_user"}}}"#)]);
    check!(first_question(&multi).is_some_and(|q| q.questions[0].multi_select), "grok question: multi_select");
}

// MARK: ChecksQuestions

#[test]
fn ask_user_question() {
    let ql = fixture("question.jsonl");
    let items = parse(&ql);
    check!(
        describe(&items) == ["asst:One question first.", "q:Branch", "q:Cleanup|Report", "q:Go", "asst:Last one.", "q:Ship"],
        "question: items {:?}",
        describe(&items)
    );
    let q1 = question_at(&items, 1).unwrap();
    let q2 = question_at(&items, 2).unwrap();
    check!(
        q1.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["integration", "main"]
            && q1.questions[0].options.iter().map(|o| o.recommended).collect::<Vec<_>>() == [true, false],
        "question: (Recommended) suffix removed and flagged"
    );
    check!(
        q1.questions[0].options[0].detail == "Has the latest merged work." && !q1.questions[0].multi_select,
        "question: detail, single select"
    );
    check!(q1.state == answered(&["integration (Recommended)"]), "question: answers from toolUseResult");
    check!(
        q2.questions[0].multi_select && q2.questions[0].options[1].label == "Build output" && q2.questions[0].options[1].recommended,
        "question: multiSelect, （建議） suffix"
    );
    check!(
        q2.state == answered(&["Old caches, Build output（建議）", "Yes"]),
        "question: answers parsed from result text, quotes escaped ({:?})",
        q2.state
    );
    check!(question_at(&items, 3).unwrap().state == QuestionState::Declined, "question: error result is .declined");
    let q5 = question_at(&items, 5).unwrap();
    check!(q5.state == QuestionState::Pending && q5.tool_use_id == "toolu_q4", "question: no result yet is .pending");
    check!(!has_perm(&items), "question: never a permissionRequest");
    check!(items[5].id == "q-toolu_q4" && ids(&parse(&ql)) == ids(&items), "question: stable ids");
}

#[test]
fn codex_questions() {
    let cl = fixture("question-codex.jsonl");
    let items = parse(&cl);
    check!(
        describe(&items) == ["user:Ask me things.", "q:Color", "q:Size|Pet", "q:Go", "q:", "asst:Waiting for your pick.", "user:Safe", "q:Ship"],
        "codex question: items {:?}",
        describe(&items)
    );
    let q = |i: usize| question_at(&items, i).unwrap();
    check!(
        q(1).agent == "codex"
            && q(1).delivery == Delivery::Picker
            && q(1).questions[0].id == "color"
            && q(1).questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && q(1).questions[0].options[0].recommended,
        "codex question: picker, id, (Recommended) flagged"
    );
    check!(q(1).state == answered(&["Blue"]), "codex question: answer by id");
    check!(q(2).state == answered(&["Large", "一隻魚 fish"]), "codex question: notes replace None of the above ({:?})", q(2).state);
    check!(q(3).state == QuestionState::Declined, "codex question: unavailable output is .declined");
    check!(
        q(4).delivery == Delivery::Message
            && q(4).questions[0].text == "Which plan?"
            && q(4).questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Fast", "Safe"],
        "codex question: async title and string options"
    );
    check!(q(4).state == QuestionState::Answered(None), "codex question: async answered by the next user message");
    check!(q(7).state == QuestionState::Pending, "codex question: no output yet is .pending");
    check!(
        !items.iter().any(|i| matches!(i.kind, ItemKind::PermissionRequest(_) | ItemKind::ToolChips(_))),
        "codex question: no permission, no tool chip"
    );
    let mut p = ConversationParser::new();
    for l in &cl[..10] {
        p.feed_line(l);
    }
    check!(
        p.items()
            .iter()
            .any(|i| matches!(&i.kind, ItemKind::Question(r) if r.delivery == Delivery::Message && r.state == QuestionState::Pending)),
        "codex question: async stays pending after {{accepted:true}}"
    );
}

// MARK: ChecksQueuedInput (the log part)

#[test]
fn queue_log() {
    let mut log = ClaudeQueueLog::default();
    for o in [
        json!({"type": "queue-operation", "operation": "enqueue", "content": "QA"}),
        json!({"type": "queue-operation", "operation": "enqueue", "content": "QB"}),
        json!({"type": "queue-operation", "operation": "remove", "content": "QA"}),
        json!({"type": "queue-operation", "operation": "enqueue", "content": "QC"}),
        json!({"type": "queue-operation", "operation": "dequeue"}),
        json!({"type": "queue-operation", "operation": "enqueue"}),
    ] {
        log.feed(o.as_object().unwrap());
    }
    check!(log.items == [Some("QC".to_string()), None], "queue: log enqueue/remove/dequeue; an enqueue without content is a nil ({:?})", log.items);
    log.feed(json!({"type": "queue-operation", "operation": "popAll", "content": "QC"}).as_object().unwrap());
    log.feed(json!({"type": "queue-operation", "operation": "popAll"}).as_object().unwrap());
    check!(log.items.is_empty(), "queue: popAll empties it");
    let fl = fixture("queued.jsonl");
    let mut p = ConversationParser::new();
    for l in &fl {
        p.feed_line(l);
    }
    check!(p.queue().items.is_empty(), "queue: the fixture's queue ends empty (enqueue, remove, dequeue)");
    let mut open = ConversationParser::new();
    for l in &fl[..3] {
        open.feed_line(l);
    }
    check!(open.queue().items == [Some("Also check the lint step.".to_string())], "queue: the parser keeps the log's open queue");
}

// MARK: ChecksLogImage

fn be16(v: usize) -> Vec<u8> {
    vec![(v >> 8 & 0xff) as u8, (v & 0xff) as u8]
}
fn le16(v: usize) -> Vec<u8> {
    vec![(v & 0xff) as u8, (v >> 8 & 0xff) as u8]
}
fn le24(v: usize) -> Vec<u8> {
    vec![(v & 0xff) as u8, (v >> 8 & 0xff) as u8, (v >> 16 & 0xff) as u8]
}
fn be32(v: usize) -> Vec<u8> {
    vec![(v >> 24 & 0xff) as u8, (v >> 16 & 0xff) as u8, (v >> 8 & 0xff) as u8, (v & 0xff) as u8]
}
fn le32(v: usize) -> Vec<u8> {
    [le16(v & 0xffff), le16(v >> 16)].concat()
}

fn png(w: usize, h: usize) -> Vec<u8> {
    [vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], be32(13), b"IHDR".to_vec(), be32(w), be32(h), vec![8, 6, 0, 0, 0], be32(0)].concat()
}

/// A JPEG whose SOF comes after an APP1 segment of `app` bytes (EXIF puts it there), `sof` = 0xC0 baseline, 0xC2 progressive.
fn jpeg(w: usize, h: usize, app: usize, sof: u8) -> Vec<u8> {
    [
        vec![0xff, 0xd8, 0xff, 0xe1],
        be16(app + 2),
        vec![0x41; app],
        vec![0xff, 0xc4],
        be16(4),
        vec![0, 0], // a DHT before the frame: not a size
        vec![0xff, sof],
        be16(17),
        vec![8],
        be16(h),
        be16(w),
        vec![3],
        vec![0; 9],
    ]
    .concat()
}

fn riff(chunk: &str, body: Vec<u8>) -> Vec<u8> {
    [b"RIFF".to_vec(), le32(4 + 8 + body.len()), b"WEBP".to_vec(), chunk.as_bytes().to_vec(), le32(body.len()), body].concat()
}
fn webp_x(w: usize, h: usize) -> Vec<u8> {
    riff("VP8X", [vec![0, 0, 0, 0], le24(w - 1), le24(h - 1)].concat())
}
fn webp_l(w: usize, h: usize) -> Vec<u8> {
    let bits = (w - 1) | (h - 1) << 14;
    riff("VP8L", [vec![0x2f], le32(bits), vec![0, 0, 0]].concat())
}
fn webp_lossy(w: usize, h: usize) -> Vec<u8> {
    riff("VP8 ", [vec![0, 0, 0, 0x9d, 0x01, 0x2a], le16(w), le16(h), vec![0, 0]].concat())
}
fn gif(w: usize, h: usize) -> Vec<u8> {
    [b"GIF89a".to_vec(), le16(w), le16(h), vec![0, 0, 0]].concat()
}

fn b64(d: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(d)
}

fn line(v: Value) -> String {
    serde_json::to_string(&v).unwrap()
}

fn ref_desc(r: &Ref) -> String {
    format!("{} {}x{} @{}#{}", log_image::format_raw(r.format), r.width, r.height, r.offset.map(|o| o.to_string()).unwrap_or("-".into()), r.index)
}

#[test]
fn log_image_headers() {
    let size = |d: &[u8]| {
        log_image::header(d).map(|(f, w, h)| format!("{} {w}x{h}", log_image::format_raw(f))).unwrap_or("nil".into())
    };
    check!(size(&png(1280, 720)) == "png 1280x720", "image header: png");
    check!(size(&jpeg(4032, 3024, 600, 0xc0)) == "jpeg 4032x3024", "image header: jpeg after APP1 and DHT");
    check!(size(&jpeg(800, 600, 40_000, 0xc2)) == "jpeg 800x600", "image header: progressive jpeg, SOF 40 KB in");
    check!(size(&webp_x(1920, 1080)) == "webp 1920x1080", "image header: webp VP8X");
    check!(size(&webp_l(300, 200)) == "webp 300x200", "image header: webp VP8L");
    check!(size(&webp_lossy(640, 480)) == "webp 640x480", "image header: webp VP8");
    check!(size(&gif(32, 16)) == "gif 32x16", "image header: gif");
    check!(
        size(b"not a picture") == "nil"
            && size(&[]) == "nil"
            && size(&png(10, 10)[..20]) == "nil"
            && size(&jpeg(10, 10, 600, 0xc0)[..300]) == "nil"
            && size(&png(0, 10)) == "nil",
        "image header: junk, cut or empty → nil"
    );
    check!(
        log_image::header_base64(&b64(&png(12, 34))).map(|(_, w, h)| format!("{w}x{h}")).as_deref() == Some("12x34")
            && log_image::header_base64("%%%").is_none(),
        "image header: from base64, reads only the start"
    );
}

#[test]
fn log_image_items() {
    let shot_a = b64(&png(1440, 900));
    let shot_b = b64(&jpeg(640, 480, 600, 0xc0));

    // Claude Code: the owner's pasted image is an `image` block beside the text.
    let mut p = ConversationParser::new();
    p.feed(
        &line(json!({"type": "user", "uuid": "u1", "message": {"role": "user", "content": [
            {"type": "text", "text": "What is wrong here? [Image #1] [Image #2]"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": shot_a}},
            {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": shot_b}},
        ]}})),
        Some(4_000),
    );
    p.feed(
        &line(json!({"type": "user", "uuid": "u2", "message": {"role": "user", "content": [
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": shot_a}},
        ]}})),
        Some(9_000),
    );
    let items = p.items();
    check!(describe(&items) == ["user:What is wrong here? [Image #1] [Image #2]", "user:"], "claude images: user items {:?}", describe(&items));
    check!(
        items[0].images.iter().map(ref_desc).collect::<Vec<_>>() == ["png 1440x900 @4000#0", "jpeg 640x480 @4000#1"],
        "claude images: refs on the user item"
    );
    let last = items.last().unwrap();
    check!(
        last.images.iter().map(|r| r.index).collect::<Vec<_>>() == [0] && last.images[0].offset == Some(9_000),
        "claude images: a picture alone is a user item"
    );
    let dbg = format!("{items:?}");
    check!(!dbg.contains(&shot_a[..40]) && !dbg.contains(&shot_b[..40]), "claude images: no base64 in the items");

    // Claude Code: a tool's picture in tool_result content.
    let mut p = ConversationParser::new();
    p.feed(r#"{"type":"assistant","uuid":"a1","message":{"content":[{"type":"tool_use","id":"r1","name":"Read","input":{"file_path":"/tmp/x.png"}}]}}"#, Some(0));
    p.feed(
        &line(json!({"type": "user", "uuid": "t1", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "r1", "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": shot_a}}]},
        ]}})),
        Some(200),
    );
    let calls = chip_calls(&p.items());
    check!(
        calls[0].images.iter().map(|r| format!("{}x{} @{}#{}", r.width, r.height, r.offset.unwrap_or(0), r.index)).collect::<Vec<_>>() == ["1440x900 @200#0"],
        "claude images: tool result picture on its call"
    );
    check!(describe(&p.items()) == ["chips:Read a file"], "claude images: a tool picture is no user item");
    check!(!format!("{:?}", p.items()).contains(&shot_a[..40]), "claude images: no base64 in the call");

    // Codex: input_image data URLs, in the owner's message and in a tool's output.
    let mut p = ConversationParser::new();
    p.feed(
        &line(json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [
            {"type": "input_text", "text": "Match this layout"},
            {"type": "input_image", "image_url": format!("data:image/webp;base64,{}", b64(&webp_x(1200, 800)))},
        ]}})),
        Some(50),
    );
    p.feed(&line(json!({"type": "response_item", "payload": {"type": "function_call", "name": "view_image", "call_id": "c1", "arguments": "{}"}})), Some(900));
    p.feed(
        &line(json!({"type": "response_item", "payload": {"type": "function_call_output", "call_id": "c1", "output": [
            {"type": "input_text", "text": "ok"}, {"type": "input_image", "image_url": format!("data:image/jpeg;base64,{shot_b}")},
        ]}})),
        Some(1_000),
    );
    let items = p.items();
    check!(items[0].images.iter().map(ref_desc).collect::<Vec<_>>() == ["webp 1200x800 @50#0"], "codex images: on the user item");
    check!(
        chip_calls(&items)[0].images.iter().map(|r| format!("{}x{} @{}#{}", r.width, r.height, r.offset.unwrap_or(0), r.index)).collect::<Vec<_>>()
            == ["640x480 @1000#0"],
        "codex images: tool output picture on its call"
    );
    check!(!format!("{items:?}").contains(&shot_b[..40]), "codex images: no base64 in the items");

    // pi: a tool result's `image` block (data + mimeType). Kept across a branch rebuild.
    let lines = [
        line(json!({"type": "message", "id": "u1", "parentId": null, "message": {"role": "user", "content": [{"type": "text", "text": "Screenshot it"}]}})),
        line(json!({"type": "message", "id": "a1", "parentId": "u1", "message": {"role": "assistant", "content": [{"type": "toolCall", "id": "k1", "name": "read", "arguments": {"path": "s.png"}}]}})),
        line(json!({"type": "message", "id": "r1", "parentId": "a1", "message": {"role": "toolResult", "toolCallId": "k1", "content": [
            {"type": "text", "text": "Read image"}, {"type": "image", "data": b64(&gif(64, 48)), "mimeType": "image/gif"}]}})),
        line(json!({"type": "message", "id": "a2", "parentId": "r1", "message": {"role": "assistant", "content": [{"type": "text", "text": "Done."}]}})),
        line(json!({"type": "message", "id": "a3", "parentId": "r1", "message": {"role": "assistant", "content": [{"type": "text", "text": "Retried."}]}})),
    ];
    let mut p = ConversationParser::new();
    let mut off = 0u64;
    for l in &lines {
        p.feed(l, Some(off));
        off += l.len() as u64 + 1;
    }
    let off2 = (lines[0].len() + lines[1].len() + 2) as u64;
    let refs: Vec<String> = chip_calls(&p.items()).first().map(|c| c.images.iter().map(ref_desc).collect()).unwrap_or_default();
    check!(refs == [format!("gif 64x48 @{off2}#0")], "pi images: tool result picture, offset kept after the rebuild {refs:?} {:?}", describe(&p.items()));
}

#[test]
fn log_image_read_back() {
    let shot_a = b64(&png(1440, 900));
    let shot_b = b64(&jpeg(640, 480, 600, 0xc0));
    let l = line(json!({"type": "user", "uuid": "u1", "message": {"role": "user", "content": [
        {"type": "text", "text": "two"},
        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": shot_a}},
        {"type": "tool_result", "tool_use_id": "x", "content": [{"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": shot_b}}]},
    ]}}));
    check!(
        log_image::data(&l, 0) == Some(png(1440, 900))
            && log_image::data(&l, 1) == Some(jpeg(640, 480, 600, 0xc0))
            && log_image::data(&l, 2).is_none()
            && log_image::data("{", 0).is_none(),
        "image read back: by index, in log order"
    );
    let cx = line(json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [
        {"type": "input_image", "image_url": format!("data:image/gif;base64,{}", b64(&gif(5, 6)))}]}}));
    check!(log_image::data(&cx, 0) == Some(gif(5, 6)), "image read back: codex data URL");
    let remote = line(json!({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [
        {"type": "input_image", "image_url": "https://example.com/a.png"}]}}));
    let mut p = ConversationParser::new();
    p.feed(&remote, Some(0));
    check!(p.items().iter().all(|i| i.images.is_empty()) && log_image::data(&remote, 0).is_none(), "image read back: a web URL is never fetched");

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("s.jsonl");
    let head = r#"{"type":"user","uuid":"h","message":{"content":"hi"}}"#;
    std::fs::write(&file, format!("{head}\n{l}\n")).unwrap();
    let r = Ref { format: Format::Jpeg, width: 640, height: 480, offset: Some(head.len() as u64 + 1), index: 1, digest: log_image::fingerprint(&shot_b) };
    check!(log_image::read(&r, &file) == Some(jpeg(640, 480, 600, 0xc0)), "image read back: from the log file at the line's offset");
    check!(
        log_image::read(&Ref { format: Format::Png, width: 1, height: 1, offset: Some(3), index: 0, digest: 0 }, &file).is_none()
            && log_image::read(&Ref { format: Format::Png, width: 1, height: 1, offset: None, index: 0, digest: 0 }, &file).is_none(),
        "image read back: an offset mid-line or none reads nothing"
    );

    // The ref carries the picture's fingerprint: a rewritten log with another picture of the same format and size at
    // the same line and index reads nothing.
    let mut pf = ConversationParser::new();
    pf.feed(&l, Some(0));
    let fref = pf.items().iter().flat_map(|i| i.images.clone()).next();
    let other_a = b64(&[png(1440, 900), vec![1, 2, 3]].concat());
    check!(
        fref.as_ref().map(|r| r.digest) == Some(log_image::fingerprint(&shot_a))
            && fref.as_ref().map(|r| r.digest) != Some(0)
            && log_image::fingerprint(&shot_a) != log_image::fingerprint(&other_a)
            && log_image::fingerprint(&shot_a) == log_image::fingerprint(&shot_a.clone()),
        "image fingerprint: on the ref, differs for other bytes {fref:?}"
    );
    let ref_a = Ref { format: Format::Png, width: 1440, height: 900, offset: Some(head.len() as u64 + 1), index: 0, digest: log_image::fingerprint(&shot_a) };
    let before = log_image::read(&ref_a, &file);
    std::fs::write(&file, format!("{head}\n{}\n", l.replace(&shot_a, &other_a))).unwrap();
    check!(
        before == Some(png(1440, 900)) && log_image::read(&ref_a, &file).is_none(),
        "image fingerprint: a same-size picture rewritten at the same place reads nothing"
    );
}

// MARK: chip ids (Swift `stableUUID`)

#[test]
fn stable_chip_ids() {
    // Same input, same id; the uuidString form (uppercase, dashed); another key, another id.
    let a = parser::stable_uuid("u1-0|Read a file");
    check!(a == parser::stable_uuid("u1-0|Read a file") && a != parser::stable_uuid("u1-0|Ran a command"), "stable uuid: same text, same id");
    check!(
        a.len() == 36
            && a.chars().enumerate().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() && !c.is_ascii_lowercase() }),
        "stable uuid: uuidString form {a}"
    );
}
