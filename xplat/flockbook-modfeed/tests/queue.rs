//! The agents' own queues (`queue::read`) and pulling them back (`queue::run`). Ported from Swift's
//! ChecksQueuedInput and ChecksGrokQueue. Screens are shaped like Claude Code 2.1, Codex 0.163 and Grok Build 1.0.67
//! draw them; the text is made up.

use herdr_core::conversation::ClaudeQueueLog;
use herdr_core::{Agent, AgentStatus};
use modfeedkit::queue::{self, Outcome, PullIo};

fn rule() -> String {
    "─".repeat(60)
}

fn claude_screen(queued: &[&str], above: &str, input: &str) -> String {
    let mut l = vec![above.to_string()];
    l.extend(queued.iter().map(|q| format!("❯ {q}")));
    l.extend(["  ctrl+enter to send now".into(), "· Pouncing… (10s · ↓ 864 tokens)".into(), rule(), input.into(), rule(), "  ⏵⏵ auto mode on (shift+tab to cycle)".into()]);
    l.join("\n")
}
fn cs(queued: &[&str]) -> String {
    claude_screen(queued, "⏺ ALPHA", "❯ Press up to edit queued messages")
}

fn codex_screen(queued: &[&str], input: &str) -> String {
    let mut l: Vec<String> = vec!["› Run the shell command".into(), "• Working (30s • esc to interrupt)".into(), "• Queued follow-up inputs".into()];
    l.extend(queued.iter().map(|q| format!("  ↳ {q}")));
    l.extend(["    ⇧← edit last queued message".into(), input.into(), "  GPT-5.6-Luna max · max · Context 96% left".into(), "  ← for agents · ? for shortcuts".into()]);
    l.join("\n")
}

fn working(kind: &str) -> Agent {
    let mut a = Agent::new(kind, kind, AgentStatus::Working);
    a.process = Some(format!("4242@{kind}"));
    a
}

/// Claude Code's input box holding `rows` (the first after "❯ ", the rest indented two).
fn claude_box(rows: &[&str]) -> String {
    let body: Vec<String> = rows.iter().enumerate().map(|(i, r)| if i == 0 { format!("❯ {r}") } else { format!("  {r}") }).collect();
    format!("⏺ ALPHA\n· Pouncing… (12s)\n{}\n{}\n{}", rule(), body.join("\n"), rule())
}

fn log(items: &[Option<&str>]) -> ClaudeQueueLog {
    ClaudeQueueLog::new(items.iter().map(|i| i.map(String::from)).collect())
}
fn logs(items: &[&str]) -> ClaudeQueueLog {
    log(&items.iter().map(|s| Some(*s)).collect::<Vec<_>>())
}
fn items(q: &Option<queue::Queued>) -> Option<Vec<Option<String>>> {
    q.as_ref().map(|q| q.items.clone())
}
fn some(v: &[&str]) -> Option<Vec<Option<String>>> {
    Some(v.iter().map(|s| Some(s.to_string())).collect())
}

/// A pane for `queue::run`: screens and agents in the order the pull reads them (None: the read failed); every key
/// list it sends.
struct Fake {
    screens: Vec<Option<String>>,
    agents: Vec<Option<Agent>>,
    sent: Vec<Vec<String>>,
    keys_on: bool,
}
impl PullIo for Fake {
    fn agent(&mut self) -> Option<Agent> {
        if self.agents.len() > 1 { self.agents.remove(0) } else { self.agents.first().cloned().flatten() }
    }
    fn screen(&mut self) -> Option<String> {
        if self.screens.len() > 1 { self.screens.remove(0) } else { self.screens.first().cloned().flatten() }
    }
    fn keys(&mut self, k: &[&str], _: &Agent) -> bool {
        self.sent.push(k.iter().map(|s| s.to_string()).collect());
        self.keys_on
    }
    fn settle(&mut self) {}
}

struct Pull {
    r: Outcome,
    k: Vec<Vec<String>>,
    kept: Vec<(String, usize)>,
}

fn pull_with(kind: &str, screens: &[Option<String>], log: &ClaudeQueueLog, agents: Option<Vec<Option<Agent>>>, keys_on: bool, saves: bool, expect: Option<Agent>) -> Pull {
    let expect = expect.unwrap_or_else(|| working(kind));
    let mut io = Fake { screens: screens.to_vec(), agents: agents.unwrap_or_else(|| vec![Some(expect.clone())]), sent: vec![], keys_on };
    let mut kept = vec![];
    let sent = std::cell::RefCell::new(0usize);
    let r = {
        let io_ref = &mut io;
        // The count of key lists sent when the text was kept is read after the run (the closure cannot borrow io).
        let mut keep = |t: &str| {
            kept.push((t.to_string(), *sent.borrow()));
            saves
        };
        queue::run(&expect, log, &mut Counting { io: io_ref, sent: &sent }, &mut keep)
    };
    for k in &io.sent {
        for key in k {
            assert!(queue::ALLOWED_KEYS.contains(&key.as_str()), "only Up, shift+left, ctrl+u ever go out: {key}");
        }
    }
    Pull { r, k: io.sent, kept }
}
fn pull(kind: &str, screens: &[&str], log: &ClaudeQueueLog) -> Pull {
    pull_with(kind, &screens.iter().map(|s| Some(s.to_string())).collect::<Vec<_>>(), log, None, true, true, None)
}

struct Counting<'a> {
    io: &'a mut Fake,
    sent: &'a std::cell::RefCell<usize>,
}
impl PullIo for Counting<'_> {
    fn agent(&mut self) -> Option<Agent> {
        self.io.agent()
    }
    fn screen(&mut self) -> Option<String> {
        self.io.screen()
    }
    fn keys(&mut self, k: &[&str], a: &Agent) -> bool {
        *self.sent.borrow_mut() += 1;
        self.io.keys(k, a)
    }
    fn settle(&mut self) {}
}

fn up() -> Vec<String> {
    vec!["Up".into()]
}
fn u() -> Vec<String> {
    vec!["ctrl+u".into()]
}
fn refused(r: &Outcome) -> bool {
    matches!(r, Outcome::Refused(_))
}
fn failed(r: &Outcome) -> bool {
    matches!(r, Outcome::Failed(_))
}
fn partial(r: &Outcome) -> bool {
    matches!(r, Outcome::Partial(..))
}

#[test]
fn claude_screen_and_log() {
    let two = cs(&["QC reply GAMMA", "QD reply DELTA"]);
    assert_eq!(queue::claude_screen(&two), Some((vec!["QC reply GAMMA".into(), "QD reply DELTA".into()], true)));
    assert_eq!(queue::claude_screen(&claude_screen(&["QC"], "⏺ ALPHA", "❯ half typed")).map(|s| s.1), Some(false), "input with text is not empty");
    assert_eq!(queue::claude_screen(&format!("❯ hello\n⏺ Hi\n{}\n❯ \n{}", rule(), rule())), None, "no hint line, no queue");

    // The log: enqueue, remove, dequeue; an enqueue without content is a None.
    let ops = [
        r#"{"type":"queue-operation","operation":"enqueue","content":"QA"}"#,
        r#"{"type":"queue-operation","operation":"enqueue","content":"QB"}"#,
        r#"{"type":"queue-operation","operation":"remove","content":"QA"}"#,
        r#"{"type":"queue-operation","operation":"enqueue","content":"QC"}"#,
        r#"{"type":"queue-operation","operation":"dequeue"}"#,
        r#"{"type":"queue-operation","operation":"enqueue"}"#,
        r#"{"type":"user","message":{"content":"queue-operation"}}"#,
    ];
    assert_eq!(queue::log_of(ops).items, vec![Some("QC".to_string()), None]);
    let fixture = include_str!("../../../Tests/HerdrKitTests/Fixtures/queued.jsonl");
    assert!(queue::log_of(fixture.lines()).items.is_empty(), "the fixture's queue ends empty");
    assert_eq!(queue::log_of(fixture.lines().take(3)).items, vec![Some("Also check the lint step.".to_string())], "the open queue");

    // Log and screen together.
    let compact = claude_screen(&["/compact", "QH reply HOTEL"], "⏺ done", "❯ Press up to edit queued messages");
    assert_eq!(items(&queue::read(Some(&working("claude")), Some(&compact), &log(&[None]))), some(&["QH reply HOTEL"]), "compaction's enqueue has no text");
    assert_eq!(items(&queue::read(Some(&working("claude")), Some(&two), &logs(&["QC reply GAMMA", "QD reply DELTA"]))), some(&["QC reply GAMMA", "QD reply DELTA"]));
    assert_eq!(
        items(&queue::read(Some(&working("claude")), Some(&cs(&["QD reply DELTA"])), &logs(&["QC reply GAMMA", "QD reply DELTA"]))),
        some(&["QC reply GAMMA", "QD reply DELTA"]),
        "the screen shows only the newest row: the log's older entry stays"
    );
    assert_eq!(items(&queue::read(Some(&working("claude")), Some(&cs(&["QE new text"])), &logs(&["QE old text"]))), some(&["QE new text"]));
    assert_eq!(
        items(&queue::read(Some(&working("claude")), Some(&cs(&["QA reply ALPHA", "QZ from the screen"])), &log(&[Some("QA reply ALPHA"), None]))),
        some(&["QA reply ALPHA", "QZ from the screen"])
    );
    assert_eq!(queue::read(Some(&working("claude")), Some(&two), &log(&[])).map(|q| q.items.len()), Some(2));
    let mut idle = working("claude");
    idle.status = AgentStatus::Idle;
    assert_eq!(queue::read(Some(&idle), Some(&two), &logs(&["QC reply GAMMA"])), None, "none unless the agent works");
    assert_eq!(queue::read(Some(&working("pi")), Some(&two), &log(&[])), None);
    let line = queue::line(queue::read(Some(&working("claude")), Some(&two), &log(&[])).as_ref());
    assert_eq!(line["type"], "queued");
    assert_eq!(line["pullsAll"], true);
    assert_eq!(queue::line(None)["items"].as_array().map(Vec::len), Some(0));
}

fn live(rows: &[&str], indented: bool) -> String {
    let mut l: Vec<String> = vec!["❯ Run the tests in the foreground".into(), "  and reply DONE.".into(), "".into(),
        format!("{}Running the tests · 4s", if indented { "  " } else { "⏺ " }), "  ⎿  $ run-tests (4s)".into(), "     (ctrl+b to run in background)".into(), "".into()];
    l.extend(rows.iter().map(|r| r.to_string()));
    l.extend(["  ctrl+enter to send now".into(), "".into(), "✶ Stewing… (6s · ↓ 88 tokens)".into(), "".into(), rule(), "❯\u{a0}Press up to edit queued messages".into(), rule()]);
    l.join("\n")
}
fn bare(rows: &[&str]) -> String {
    let mut l: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    l.extend(["  ctrl+enter to send now".into(), "".into(), "✶ Stewing… (2s)".into(), "".into(), rule(), "❯\u{a0}Press up to edit queued messages".into(), rule()]);
    l.join("\n")
}

#[test]
fn claude_blank_lines() {
    let c = working("claude");
    let newest = live(&["❯ QF single", "", "❯ QG first", "", "  QG third"], false);
    let fg = queue::read(Some(&c), Some(&newest), &logs(&["QF single", "QG first\n\nQG third"]));
    assert_eq!(items(&fg), some(&["QF single", "QG first\n\nQG third"]));
    assert!(fg.unwrap().pullable);
    assert_eq!(queue::read(Some(&c), Some(&newest), &log(&[])), None, "no log for the blank-line row: nothing shown");
    assert_eq!(queue::read(Some(&c), Some(&newest), &log(&[Some("QF single"), None])), None, "a log entry without text does not vouch");
    let oldest = live(&["❯ QB first", "", "  QB third", "", "❯ QC single", "", "❯ QD a", "  QD b", "", "❯ QE single"], true);
    assert_eq!(items(&queue::read(Some(&c), Some(&oldest), &logs(&["QB first\n\nQB third", "QC single", "QD a\nQD b", "QE single"]))),
        some(&["QB first\n\nQB third", "QC single", "QD a\nQD b", "QE single"]));
    assert_eq!(items(&queue::read(Some(&c), Some(&oldest), &log(&[]))), some(&["QE single"]), "no log: only the newest");
    assert_eq!(queue::claude_screen(&live(&["❯ QC single", "", "❯ QE single"], false)).map(|s| s.0), Some(vec!["QC single".into(), "QE single".into()]));
    assert_eq!(items(&queue::read(Some(&c), Some(&live(&["❯ QC single"], true)), &log(&[]))), some(&["QC single"]), "the rows above are no message");
    let fg_only = live(&["❯ QG first", "", "  QG third"], false);
    for text in ["QG first QG third", "QG first\n\n\nQG third", "\nQG first\n\nQG third", "QG first\n\nQG third\n", "QG first\nQG third"] {
        assert_eq!(queue::read(Some(&c), Some(&fg_only), &logs(&[text])), None, "{text:?} does not vouch for the empty row");
        let p = pull("claude", &[&fg_only, &fg_only], &logs(&[text]));
        assert!(!p.k.contains(&up()), "{text:?}: no Up");
    }
    let sent = bare(&["❯ already sent", "", "❯ QQ queued"]);
    for l in [log(&[]), logs(&["QQ other"]), logs(&["QQ queued"])] {
        assert_eq!(items(&queue::read(Some(&c), Some(&sent), &l)), some(&["QQ queued"]), "the sent prompt above is not a message");
    }
    let tail = bare(&["❯ QR a", "", "", "❯ QS b"]);
    assert_eq!(items(&queue::read(Some(&c), Some(&tail), &log(&[]))), some(&["QS b"]));
    assert_eq!(items(&queue::read(Some(&c), Some(&tail), &logs(&["QR a\n", "QS b"]))), some(&["QR a\n", "QS b"]));

    let qg = live(&["❯ QG first", "", "  QG third"], false);
    let b = claude_box(&["QG first", "", "QG third"]);
    let p = pull("claude", &[&qg, &b, &b, &claude_box(&["QG first", "", ""]), &claude_box(&["QG first", ""]), &claude_box(&["QG first"]), &format!("{}\n❯ \n{}", rule(), rule())], &logs(&["QG first\n\nQG third"]));
    assert_eq!(p.r, Outcome::Pulled("QG first\n\nQG third".into()), "{:?}", p.k);
    assert_eq!(p.k.first(), Some(&up()));
    assert!(p.k[1..].iter().all(|k| *k == u()));
}

#[test]
fn codex_and_grok() {
    let cx = queue::read(Some(&working("codex")), Some(&codex_screen(&["QB reply BETA", "QD reply DELTA"], "› Ask Codex to do anything")), &log(&[])).unwrap();
    assert_eq!(cx.items, some(&["QB reply BETA", "QD reply DELTA"]).unwrap());
    assert!(cx.pullable && !cx.pulls_all());
    assert!(!queue::read(Some(&working("codex")), Some(&codex_screen(&["QB"], "› typing here")), &log(&[])).unwrap().pullable);

    let grok = |queued: &[&str], footer: Option<&str>| -> String {
        let mut l: Vec<String> = vec!["  ❯ Write a story.".into(), "".into(), "     Once upon a time.".into(), "".into()];
        for (i, q) in queued.iter().enumerate() {
            l.push(format!("    #{} {q}", i + 1));
        }
        l.extend(["".into(), "    ⠼ Thinking… 3.2s                                       5.6s ⇣1.46k [stop]".into(), "".into(),
            "  ╭──────────────────────────────────────────╮".into(), "  │ ❯   │".into(), "  ╰──────────────────────── Grok 4.7 (high) ─╯".into(), "".into()]);
        l.push(footer.map(String::from).unwrap_or_else(|| if queued.is_empty() { "  Shift+Tab:mode  │  Ctrl+c:cancel  │  Ctrl+.:shortcuts".into() }
            else { "  Enter:send now  │  Shift+Tab:mode  │  Ctrl+c:cancel  │  Ctrl+;:queue  │  Ctrl+.:shortcuts".into() }));
        l.join("\n")
    };
    let two = grok(&["Reply with alpha.", "Reply with beta, and this one is long enough to be cut on…"], None);
    assert_eq!(queue::grok_screen(&two), Some(vec!["Reply with alpha.".into(), "Reply with beta, and this one is long enough to be cut on…".into()]));
    let q = queue::read(Some(&working("grok")), Some(&two), &log(&[])).unwrap();
    assert!(q.agent == "grok" && q.items.len() == 2 && !q.pullable && !q.pulls_all());
    assert_eq!(queue::grok_screen(&grok(&[], None)), None);
    assert_eq!(queue::grok_screen(&grok(&["Reply with alpha."], Some("  Shift+Tab:mode  │  Ctrl+.:shortcuts"))), None, "no Ctrl+;:queue, no queue");
    let answer = grok(&["Reply with alpha."], None).replace("     Once upon a time.", "    #1 Step one\n    #2 Step two\n\n     Done.");
    assert_eq!(queue::grok_screen(&answer), Some(vec!["Reply with alpha.".into()]), "numbered lines in the answer do not count");
    assert_eq!(queue::grok_screen(&grok(&["A"], None).replace("#1 A", "#2 A")), None, "not numbered from 1");
    assert!(queue::reads("grok") && queue::reads("claude") && queue::reads("codex") && !queue::reads("pi"));

    // Codex and Grok: every pull is refused before any key; the owner edits it in the agent's terminal.
    let p = pull("codex", &[&codex_screen(&["QB reply BETA"], "› Ask Codex to do anything")], &log(&[]));
    assert!(refused(&p.r) && p.k.is_empty(), "{:?}", p.r);
    let p = pull("grok", &[&two], &log(&[]));
    assert!(matches!(&p.r, Outcome::Refused(w) if w.contains("Grok")) && p.k.is_empty());
}

#[test]
fn keys_are_only_up_shift_left_ctrl_u() {
    assert!(!queue::safe(&["Escape"]) && !queue::safe(&["esc"]) && !queue::safe(&["Up", "Escape"]) && !queue::safe(&["Enter"]) && !queue::safe(&["\u{1b}"]) && !queue::safe(&[]));
    assert!(queue::safe(&["Up"]) && queue::safe(&["ctrl+u"]));
    assert_eq!((queue::pull_key("claude"), queue::pull_key("codex"), queue::pull_key("pi"), queue::pull_key("grok")), (Some("Up"), Some("shift+left"), None, None));
    assert_eq!(queue::interrupts("• Working (2s • esc to interrupt)"), 0);
    assert_eq!(queue::interrupts("  ⎿  Interrupted · What should Claude do instead?"), 1);
    // hc's names for them.
    let hc: Vec<String> = queue::ALLOWED_KEYS.iter().map(|k| herdr_core::hc::keys::translate(k)).collect();
    assert_eq!(hc, vec!["up", "shift+left", "ctrl+u"]);
}

#[test]
fn claude_pull() {
    let two = cs(&["QC reply GAMMA", "QD reply DELTA"]);
    let empty = format!("{}\n❯ \n{}", rule(), rule());
    let pulled_in = claude_box(&["QC reply GAMMA", "QD reply DELTA"]);
    let one_row = claude_box(&["QC reply GAMMA"]);
    let qc_blank = claude_box(&["QC reply GAMMA", ""]);
    let gd_text = "QC reply GAMMA\nQD reply DELTA";
    let gd = logs(&["QC reply GAMMA", "QD reply DELTA"]);
    let end = format!("· Pouncing… (13s)\n{empty}");

    let p = pull("claude", &[&two, &pulled_in, &pulled_in, &qc_blank, &one_row, &end], &gd);
    assert_eq!(p.r, Outcome::Pulled(gd_text.into()));
    assert_eq!(p.k, vec![up(), u(), u(), u()]);
    assert_eq!(p.kept, vec![(gd_text.to_string(), 1)], "kept after Up, before the first Ctrl-U");

    // More came back than the screen showed: all of it.
    let more = claude_box(&["QK reply KILO", "QL reply LIMA"]);
    let p = pull("claude", &[&cs(&["QL reply LIMA"]), &more, &more, &claude_box(&["QK reply KILO", ""]), &claude_box(&["QK reply KILO"]), &empty], &logs(&["QK reply KILO", "QL reply LIMA"]));
    assert_eq!(p.r, Outcome::Pulled("QK reply KILO\nQL reply LIMA".into()));

    // Rows that are not the text character for character (wrapped): failed, nothing cleared.
    let wrapped = claude_box(&["QW one two three", "four five"]);
    let p = pull("claude", &[&cs(&["QW one two three four five"]), &wrapped, &wrapped, &claude_box(&["QW one two three"]), &empty], &logs(&["QW one two three four five"]));
    assert!(failed(&p.r) && p.kept.is_empty() && p.k == vec![up()], "{:?}", p.r);

    // Wider than a row, or what the screen cannot show exactly: refused before any key.
    let cjk = "請檢查資料庫連線".repeat(5);
    let rows: Vec<String> = vec![cjk.chars().take(28).collect(), cjk.chars().skip(28).collect()];
    let cb = claude_box(&[&rows[0], &rows[1]]);
    let p = pull("claude", &[&cs(&[&cjk]), &cb, &cb, &claude_box(&[&rows[0]]), &empty], &logs(&[&cjk]));
    assert!(refused(&p.r) && p.k.is_empty(), "CJK wider than a row: {:?}", p.r);
    for text in ["fix\n\n", "fix ", "a\tb", "if x:\n  run()  "] {
        let f = claude_box(&["fix"]);
        let p = pull("claude", &[&cs(&["fix"]), &f, &f, &empty], &logs(&[text]));
        assert!(refused(&p.r) && p.k.is_empty(), "{text:?}: {:?}", p.r);
    }
    let code = "if ready:\n    run()";
    let code_box = claude_box(&["if ready:", "    run()"]);
    let p = pull("claude", &[&cs(&[code]), &code_box, &code_box, &claude_box(&["if ready:", ""]), &claude_box(&["if ready:"]), &empty], &logs(&[code]));
    assert_eq!(p.r, Outcome::Pulled(code.into()), "indentation kept");

    // Other text came back; 'ab' for 'a b'; the box unreadable after Up: not kept, not cleared.
    let odd = claude_box(&["QZ something else"]);
    let p = pull("claude", &[&two, &odd, &odd], &gd);
    assert!(failed(&p.r) && p.kept.is_empty() && p.k == vec![up()]);
    let ab = claude_box(&["ab"]);
    let p = pull("claude", &[&cs(&["a b"]), &ab, &ab, &empty], &logs(&["a b"]));
    assert!(failed(&p.r) && p.kept.is_empty() && p.k == vec![up()]);
    let p = pull_with("claude", &[Some(two.clone()), None], &gd, None, true, true, None);
    assert!(failed(&p.r) && p.kept.is_empty() && p.k == vec![up()]);
    // Codex in the pane when Up would go: no key.
    let p = pull_with("claude", &[Some(two.clone()), Some(pulled_in.clone())], &gd, Some(vec![Some(working("claude")), Some(working("codex"))]), true, true, None);
    assert!(refused(&p.r) && p.k.is_empty());

    // The box as drawn: a no-break space or a trailing space is not the log's text.
    for row in ["a\u{a0}b", "a b "] {
        let c = claude_box(&[row]);
        let p = pull("claude", &[&cs(&["a b"]), &c, &c, &empty], &logs(&["a b"]));
        assert!(failed(&p.r) && p.kept.is_empty() && p.k == vec![up()], "{row:?}");
        // ...and the same change after the first read stops the clearing.
        let ab_box = claude_box(&["a b"]);
        let p = pull("claude", &[&cs(&["a b"]), &ab_box, &c, &c, &empty], &logs(&["a b"]));
        assert!(partial(&p.r) && p.k == vec![up()], "{row:?}: {:?}", p.r);
    }
    // An emoji, a line like the box's rule, more lines than the screen: refused before any key.
    let melt = "fix \u{1FAE0}";
    let mb = claude_box(&[melt]);
    let p = pull("claude", &[&cs(&[melt]), &mb, &mb, &empty], &logs(&[melt]));
    assert!(refused(&p.r) && p.k.is_empty());
    let ruled = format!("alpha\n{}\nbeta", "─".repeat(12));
    let p = pull("claude", &[&cs(&[&ruled.replace('\n', "\n  ")]), &claude_box(&["alpha"]), &empty], &logs(&[&ruled]));
    assert!(refused(&p.r) && p.k.is_empty());
    let tall: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
    let tall_text = tall.join("\n");
    let tall_box = claude_box(&tall.iter().map(String::as_str).collect::<Vec<_>>());
    let p = pull("claude", &[&cs(&["line 1\n  line 2…"]), &tall_box, &empty], &logs(&[&tall_text]));
    assert!(refused(&p.r) && p.k.is_empty());
    // Not the same scalars: U+F900 and U+8C48.
    let p = pull("claude", &[&cs(&["\u{F900}"]), &claude_box(&["\u{8C48}"]), &claude_box(&["\u{8C48}"]), &empty], &logs(&["\u{F900}"]));
    assert!(failed(&p.r) && p.k == vec![up()]);
    for marked in ["\u{3099}fix", "ok\n\u{302A}x"] {
        assert!(!queue::shows(marked, &two), "a combining mark in {marked:?}");
    }
    assert!(queue::shows("ok\n\u{8C48}x", &two));

    // Two blank rows added before the first Ctrl-U: stopped.
    let p = pull("claude", &[&cs(&["a"]), &claude_box(&["a"]), &claude_box(&["a", "", ""]), &claude_box(&["a", "", ""]), &empty], &logs(&["a"]));
    assert!(partial(&p.r) && p.k == vec![up()]);
    // A text that starts with a blank line: two Ctrl-U, then empty.
    let p = pull("claude", &[&cs(&["\n  a"]), &claude_box(&["", "a"]), &claude_box(&["", "a"]), &claude_box(&["", ""]), &empty, &empty], &logs(&["\na"]));
    assert_eq!((p.r, p.k), (Outcome::Pulled("\na".into()), vec![up(), u(), u()]));
    // The screen not redrawn after a Ctrl-U: no extra key.
    let p = pull("claude", &[&two, &pulled_in, &pulled_in, &pulled_in, &qc_blank, &one_row, &empty], &gd);
    assert_eq!((p.r, p.k), (Outcome::Pulled(gd_text.into()), vec![up(), u(), u(), u()]));
    // The placeholder's words left as text: one more Ctrl-U.
    let ph = queue::CLAUDE_PLACEHOLDER;
    let ph_text = format!("{ph}\nmore");
    let p = pull("claude", &[&cs(&[&format!("{ph}\n  more")]), &claude_box(&[ph, "more"]), &claude_box(&[ph, "more"]), &claude_box(&[ph, ""]), &claude_box(&[ph]), &empty], &logs(&[&ph_text]));
    assert_eq!((p.r, p.k), (Outcome::Pulled(ph_text.clone()), vec![up(), u(), u(), u()]));

    // The owner types while it is cleared: stopped, the text still goes to the composer.
    let p = pull("claude", &[&two, &pulled_in, &pulled_in, &claude_box(&["QC reply GAMMA", "QD reply DELTA and more"])], &gd);
    assert!(matches!(&p.r, Outcome::Partial(t, _) if t == gd_text) && p.k == vec![up(), u()]);
    let p = pull("claude", &[&two, &pulled_in, &pulled_in, &qc_blank, &claude_box(&["QC reply GAMMA typed"])], &gd);
    assert!(partial(&p.r) && p.k == vec![up(), u(), u()]);
    // Another agent after Up, or between two Ctrl-U: stopped.
    let mut other = working("claude");
    other.process = Some("9999@other".into());
    let w = || Some(working("claude"));
    let p = pull_with("claude", &[Some(two.clone()), Some(pulled_in.clone()), Some(pulled_in.clone())], &gd, Some(vec![w(), w(), Some(other.clone())]), true, true, None);
    assert!(partial(&p.r) && p.k == vec![up()], "{:?} {:?}", p.r, p.k);
    let p = pull_with("claude", &[Some(two.clone()), Some(pulled_in.clone()), Some(pulled_in.clone()), Some(one_row.clone())], &gd, Some(vec![w(), w(), w(), w(), Some(other.clone())]), true, true, None);
    assert!(partial(&p.r) && p.k == vec![up(), u()], "{:?}", p.k);
    // A failed read after a Ctrl-U: the text still comes back.
    let p = pull_with("claude", &[Some(two.clone()), Some(pulled_in.clone()), Some(pulled_in.clone()), None], &gd, None, true, true, None);
    assert!(p.r.text() == Some(gd_text) && p.k == vec![up(), u()]);
    // Not kept: no Ctrl-U.
    let p = pull_with("claude", &[two.clone(), pulled_in.clone(), pulled_in.clone(), one_row.clone(), one_row.clone(), empty.clone()].map(Some), &gd, None, true, false, None);
    assert!(partial(&p.r) && p.k == vec![up()]);
    // The box never empties: stopped after a few Ctrl-U.
    let p = pull("claude", &[&two, &pulled_in], &gd);
    assert!(partial(&p.r) && p.k.len() <= 7 && p.k[1..].iter().all(|k| *k == u()));

    // Claude Code without the log's text: refused before any key.
    let p = pull("claude", &[&two, &pulled_in, &pulled_in, &one_row, &empty], &log(&[]));
    assert!(refused(&p.r) && p.k.is_empty());
    let p = pull("claude", &[&cs(&["QD reply DELTA"]), &pulled_in, &pulled_in, &one_row, &empty], &logs(&["QD reply DELTA"]));
    assert!(failed(&p.r) && p.k == vec![up()] && p.kept.is_empty(), "more rows came back than the log gives");

    // No identity (no process): refused; lost after Up: no Ctrl-U.
    let mut anon = working("claude");
    anon.process = None;
    let p = pull_with("claude", &[two.clone(), pulled_in.clone()].map(Some), &gd, None, true, true, Some(anon.clone()));
    assert!(refused(&p.r) && p.k.is_empty());
    let p = pull_with("claude", &[two.clone(), pulled_in.clone(), pulled_in.clone()].map(Some), &gd, Some(vec![w(), w(), Some(anon)]), true, true, None);
    assert!(partial(&p.r) && p.k == vec![up()]);
    // Input box not empty, the turn ended, another agent, no queue, input off, nothing came back.
    let p = pull("claude", &[&claude_screen(&["QC"], "⏺ ALPHA", "❯ half typed")], &log(&[]));
    assert!(refused(&p.r) && p.k.is_empty());
    let mut done = working("claude");
    done.status = AgentStatus::Done;
    let p = pull_with("claude", &[Some(two.clone())], &gd, Some(vec![Some(done)]), true, true, None);
    assert!(refused(&p.r) && p.k.is_empty());
    let p = pull_with("claude", &[Some(two.clone())], &gd, Some(vec![Some(other)]), true, true, None);
    assert!(refused(&p.r) && p.k.is_empty());
    let p = pull("claude", &[&format!("⏺ nothing queued\n{empty}")], &log(&[]));
    assert!(refused(&p.r) && p.k.is_empty());
    let p = pull_with("claude", &[Some(two.clone())], &gd, None, false, true, None);
    assert!(refused(&p.r) && p.k == vec![up()]);
    let p = pull("claude", &[&two, &two], &gd);
    assert!(failed(&p.r) && p.k == vec![up()]);
    // An interrupt notice after the keys, or the agent blocked: interrupted.
    let p = pull("claude", &[&two, &pulled_in, &pulled_in, &qc_blank, &one_row, &format!("  ⎿  Interrupted · What should Claude do instead?\n{empty}")], &gd);
    assert_eq!(p.r, Outcome::Interrupted(gd_text.into()));
    let mut blocked = working("claude");
    blocked.status = AgentStatus::Blocked;
    let mut agents: Vec<Option<Agent>> = (0..9).map(|_| w()).collect();
    agents.push(Some(blocked));
    let p = pull_with("claude", &[two.clone(), pulled_in.clone(), pulled_in.clone(), qc_blank.clone(), one_row.clone(), empty.clone()].map(Some), &gd, Some(agents), true, true, None);
    assert_eq!(p.r, Outcome::Interrupted(gd_text.into()));
}
