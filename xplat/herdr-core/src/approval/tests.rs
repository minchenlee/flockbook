//! Parity with Swift's `HerdrKitChecks` for the approval rules (ChecksNotice, ChecksCore, ChecksAgy,
//! ChecksGrokApproval, ChecksGrokRemote, ChecksHCGrok, ChecksHCCards, ChecksHCCardFlow, ChecksHubReview4/5,
//! ChecksHubSecurity): the same synthetic screens and expected values. Anything that sends keys runs on the scripted
//! `FakeScreen` and asserts the exact keys, or that nothing at all was sent. Each `assert!` names the Swift check.

use super::*;
use crate::fake::{hc_pane, FakeScreen};
use crate::keygate::{Allowed, KeyGate};
use std::cell::RefCell;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

// MARK: helpers

fn j(lines: &[&str]) -> String {
    lines.join("\n")
}

fn rule(c: char, n: usize) -> String {
    c.to_string().repeat(n)
}

fn st(kind: &str, blocked: bool, id: Option<&str>, screen: &str, tool: Option<&str>, command: Option<&str>) -> PaneState {
    PaneState {
        agent_kind: kind.into(),
        blocked,
        pending_tool_use_id: id.map(String::from),
        screen: screen.into(),
        pending_tool: tool.map(String::from),
        pending_command: command.map(String::from),
    }
}

fn req(title: &str, command: &str, tool: &str, id: &str) -> PermissionRequest {
    new_permission(title, command, tool, id, PermState::Pending)
}

fn yes() -> Allowed {
    Arc::new(|| true)
}

fn no() -> Allowed {
    Arc::new(|| false)
}

/// `allowed` says yes `n` times, then no (Swift: `Budget`, `left -= 1; return left >= 0`).
fn budget(n: i64) -> Allowed {
    let left = Arc::new(AtomicI64::new(n));
    Arc::new(move || left.fetch_sub(1, Ordering::SeqCst) > 0)
}

/// An agent as the TUI `herdr` server reports it (bound by its session id).
fn tui_agent(kind: &str, session: Option<&str>, status: AgentStatus) -> Agent {
    let mut a = Agent::new(kind, kind, status);
    a.backend = BackendKind::Tui;
    a.session_id = session.map(String::from);
    a
}

fn tui_pane(agent: Option<Agent>) -> Pane {
    Pane {
        id: "w1:p1".into(),
        terminal_id: "term_1".into(),
        cwd: "/w".into(),
        agent,
        label: None,
        backend: BackendKind::Tui,
        process_ids: vec![],
    }
}

/// The calls a `PaneScreen` gets, in order, to see that the screen read comes right before the key.
struct Rec {
    fake: FakeScreen,
    log: RefCell<Vec<String>>,
}

impl Rec {
    fn new(fake: FakeScreen) -> Rec {
        Rec { fake, log: RefCell::new(vec![]) }
    }
    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }
}

impl PaneScreen for Rec {
    fn backend(&self) -> BackendKind {
        self.fake.backend()
    }
    fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError> {
        self.log.borrow_mut().push("snapshot".into());
        self.fake.snapshot()
    }
    fn visible_text(&self, pane_id: &str) -> Result<String, HerdrError> {
        self.log.borrow_mut().push("read".into());
        self.fake.visible_text(pane_id)
    }
    fn respond(&self, pane_id: &str, allow: bool) -> Result<(), HerdrError> {
        self.log.borrow_mut().push("respond".into());
        self.fake.respond(pane_id, allow)
    }
    fn send_named_keys(&self, keys: &[String], pane_id: &str) -> Result<(), HerdrError> {
        self.log.borrow_mut().push("keys".into());
        self.fake.send_named_keys(keys, pane_id)
    }
    fn send_keys(&self, bytes: &[u8], pane_id: &str) -> Result<(), HerdrError> {
        self.log.borrow_mut().push("bytes".into());
        self.fake.send_keys(bytes, pane_id)
    }
    fn send_text(&self, text: &str, pane_id: &str) -> Result<(), HerdrError> {
        self.log.borrow_mut().push("text".into());
        self.fake.send_text(text, pane_id)
    }
    fn pause(&self, _ms: u64) {}
}

// MARK: ChecksNotice.swift

fn notice_prompt() -> String {
    j(&[
        &rule('─', 40),
        " Bash command",
        " Tip: auto mode handles these prompts for you — choose \"switch to auto mode\" below",
        " Build the web app",
        &rule('╌', 40),
        " npm run build",
        &rule('╌', 40),
        " Do you want to proceed?",
        " ❯ 1. Yes",
        "   2. Yes, and always allow access to /repo from this project",
        "   3. Yes, and switch to auto mode · auto mode handles these prompts for you",
        "   4. No",
        "",
        " Esc to cancel · Tab to amend",
    ])
}

fn notice_picker() -> String {
    j(&[
        " ←  ☐ Deploy  ✔ Submit  →",
        " Do you want to deploy now?",
        " ❯ 1. Yes",
        "   2. No",
        "   3. Type something.",
        " ────────────────",
        "   Chat about this",
    ])
}

fn notice_blocked() -> PaneState {
    st("claude", true, Some("toolu_1"), &format!("old output\n{}", notice_prompt()), Some("Bash"), Some("npm run build"))
}

#[test]
fn notice_buttons() {
    let prompt = notice_prompt();
    let picker = notice_picker();
    let blocked = notice_blocked();

    // Which notifications get buttons.
    assert_eq!(super::prompt(&blocked, true).as_deref(), Some("toolu_1"), "notice: Claude permission prompt gets buttons, bound to the pending call");
    assert_eq!(super::prompt(&blocked, false), None, "notice: input off -> no buttons");
    let mut s = blocked.clone();
    s.screen = picker.clone();
    s.pending_tool_use_id = None;
    assert_eq!(super::prompt(&s, true), None, "notice: question picker -> no buttons");
    s.pending_tool_use_id = Some("toolu_1".into());
    assert_eq!(super::prompt(&s, true), None, "notice: question picker with an open call -> no buttons");
    let mut s = blocked.clone();
    s.screen = "Running tests…".into();
    assert_eq!(super::prompt(&s, true), None, "notice: no prompt on screen (tool running) -> no buttons");
    // No transcript (or the log lags the screen): bind to the prompt on screen instead.
    let mut nolog = blocked.clone();
    nolog.pending_tool_use_id = None;
    let fp = super::prompt(&nolog, true);
    assert!(
        fp.is_some() && fp.as_deref() != Some("toolu_1") && fp == screen_prompt(&prompt),
        "notice: no pending call in the log -> buttons bound to the prompt on screen"
    );
    assert!(
        screen_prompt(&format!("{prompt}\n")) == screen_prompt(&prompt) && screen_prompt("Running tests…").is_none(),
        "notice: screen prompt id is stable, nil without a prompt"
    );
    let other = prompt.replace("npm run build", "rm -rf build");
    assert_ne!(screen_prompt(&other), screen_prompt(&prompt), "notice: another command -> another screen prompt id");
    assert_eq!(screen_prompt(&format!("old output\n{prompt}")), screen_prompt(&prompt), "notice: text above the prompt box does not change the id");
    assert_eq!(outcome(true, fp.as_deref(), Some(&nolog), true), Outcome::Send { allow: true }, "notice: screen-bound Allow, same prompt -> send");
    assert_eq!(
        outcome(true, fp.as_deref(), Some(&blocked), true),
        Outcome::Send { allow: true },
        "notice: screen-bound, log caught up with the same prompt -> send"
    );
    let mut other_now = nolog.clone();
    other_now.screen = other;
    assert_eq!(outcome(true, fp.as_deref(), Some(&other_now), true), Outcome::Open, "notice: screen-bound, another prompt now -> open");
    let mut s = blocked.clone();
    s.blocked = false;
    assert_eq!(super::prompt(&s, true), None, "notice: agent not blocked -> no buttons");
    let mut s = blocked.clone();
    s.agent_kind = "codex".into();
    assert_eq!(super::prompt(&s, true), None, "notice: approval keys are verified for Claude only -> no buttons for Codex");
}

#[test]
fn notice_screen_request() {
    let prompt = notice_prompt();
    let picker = notice_picker();
    // The prompt box read from the screen: the card's request when the log has none.
    let r = screen_request(&prompt, "claude").expect("notice: screen request");
    assert!(
        r.title == "Bash command" && r.tool_name == "Bash" && r.command == "npm run build" && r.state == PermState::Pending,
        "notice: screen request has the tool and the command ({r:?})"
    );
    assert_eq!(Some(r.tool_use_id), screen_prompt(&prompt), "notice: screen request id = screen prompt id");
    assert!(
        screen_request("Running tests…", "claude").is_none() && screen_request(&picker, "claude").is_none(),
        "notice: no screen request without a permission prompt"
    );
    let edit = prompt.replace(" Bash command", " Edit file");
    assert_ne!(screen_prompt(&edit), screen_prompt(&prompt), "notice: screen id includes the tool name");
    let opts = prompt.replace("access to /repo", "access to /other");
    assert_ne!(screen_prompt(&opts), screen_prompt(&prompt), "notice: screen id includes the whole box (options too)");
    let moved = prompt.replace(" ❯ 1. Yes\n   2.", "   1. Yes\n ❯ 2.");
    assert_ne!(moved, prompt, "(setup) the cursor moved");
    assert_eq!(screen_prompt(&moved), screen_prompt(&prompt), "notice: the option cursor does not change the screen id");
}

fn workflow_prompt(left: &str) -> String {
    let deny = format!(" ⚠ Claude Code will automatically deny this request in {left}, to avoid blocking progress on an unattended session");
    j(&[
        "⏺ Bash(cleanup)",
        &rule('─', 40),
        " Bash command · from the \"build-r1-t2\" workflow",
        " Remove probe files",
        &rule('╌', 40),
        " cd /work/app &&",
        "   rm -f /tmp_probe_check.json &&",
        "   ls",
        &rule('╌', 40),
        " Dangerous rm operation on critical path: /tmp_probe_check.json",
        &deny,
        "",
        " Do you want to proceed?",
        " ❯ 1. Yes",
        "   2. No",
        "",
        " Esc to cancel · Tab to amend",
    ])
}

#[test]
fn notice_workflow() {
    // A workflow's call (its transcript is not the main log): 2 options, a danger note and a countdown that changes every second.
    let wf = workflow_prompt("1:23");
    let wf_later = workflow_prompt("1:22");
    let r = screen_request(&wf, "claude");
    assert!(
        r.as_ref().is_some_and(|r| r.tool_name == "Bash"
            && r.title.starts_with("Bash command")
            && r.command == "cd /work/app &&\nrm -f /tmp_probe_check.json &&\nls"),
        "notice: workflow prompt -> card request ({r:?})"
    );
    let mut wf_state = st("claude", true, None, &wf, None, None);
    assert!(
        super::prompt(&wf_state, true) == screen_prompt(&wf) && screen_prompt(&wf).is_some(),
        "notice: workflow prompt without a log row -> buttons"
    );
    assert_eq!(screen_prompt(&wf), screen_prompt(&wf_later), "notice: the auto-deny countdown is not part of the screen id");
    wf_state.screen = wf_later;
    assert_eq!(
        outcome(true, screen_prompt(&wf).as_deref(), Some(&wf_state), true),
        Outcome::Send { allow: true },
        "notice: Allow a second later (countdown moved) -> send"
    );
    assert_ne!(screen_prompt(&wf.replace("rm -f", "rm -rf")), screen_prompt(&wf), "notice: another command in the workflow prompt -> another id");
}

#[test]
fn notice_outcome() {
    // Action -> outcome, with the stale-prompt guard.
    let blocked = notice_blocked();
    assert_eq!(outcome(true, Some("toolu_1"), Some(&blocked), true), Outcome::Send { allow: true }, "notice: Allow on the same prompt sends allow");
    assert_eq!(outcome(false, Some("toolu_1"), Some(&blocked), true), Outcome::Send { allow: false }, "notice: Deny on the same prompt sends deny");
    assert_eq!(outcome(true, Some("toolu_0"), Some(&blocked), true), Outcome::Open, "notice: another prompt now -> open, send nothing");
    let mut s = blocked.clone();
    s.screen = "Running tests…".into();
    assert_eq!(outcome(true, Some("toolu_1"), Some(&s), true), Outcome::Open, "notice: prompt gone from screen -> open");
    let mut s = blocked.clone();
    s.blocked = false;
    assert_eq!(outcome(false, Some("toolu_1"), Some(&s), true), Outcome::Open, "notice: agent no longer blocked -> open");
    assert_eq!(outcome(true, Some("toolu_1"), None, true), Outcome::Open, "notice: pane gone -> open");
    assert_eq!(outcome(true, Some("toolu_1"), Some(&blocked), false), Outcome::Open, "notice: input turned off since -> open");
    assert_eq!(outcome(true, None, Some(&blocked), true), Outcome::Open, "notice: notification without a bound prompt -> open");
}

#[test]
fn notice_state_read() {
    // Reading the state sends nothing; a changed screen after posting opens the tab. (Swift: against FakeHerdr.)
    let prompt = notice_prompt();
    let fake = FakeScreen::new(vec![], vec![&prompt]);
    let pending = req("Bash", "npm run build", "Bash", "toolu_1");
    let s = approval_state(&fake, "w1:p1", "claude", true, Some(&pending));
    assert!(s.as_ref().is_some_and(|s| s.screen.contains("Do you want to proceed?")), "notice: state read from the pane's visible screen");
    assert!(fake.sent().is_empty() && *fake.reads.borrow() > 0, "notice: reading the state only reads ({:?})", fake.sent());
    assert_eq!(outcome(true, Some("toolu_1"), s.as_ref(), true), Outcome::Send { allow: true }, "notice: fresh state allows the send");
    let fake = FakeScreen::new(vec![], vec!["Running tests…"]);
    let stale = approval_state(&fake, "w1:p1", "claude", true, Some(&pending));
    assert_eq!(outcome(true, Some("toolu_1"), stale.as_ref(), true), Outcome::Open, "notice: screen changed after posting -> open");
    // A screen that cannot be read: no state, so no send.
    let fake = FakeScreen::new(vec![], vec![&prompt]);
    *fake.fail_reads.borrow_mut() = 1;
    assert_eq!(approval_state(&fake, "w1:p1", "claude", true, Some(&pending)), None, "notice: unreadable screen -> no state");
}

#[test]
fn notice_clipped() {
    // A box taller than the pane: its top rule, header and the start of the command scrolled off.
    let dashed = rule('╌', 40);
    let tail = format!("\n{dashed}\n Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel · Tab to amend");
    let clipped = (21..=40).map(|i| format!(" │ line {i} of a long heredoc")).collect::<Vec<_>>().join("\n") + &tail;
    let r = screen_request(&clipped, "claude");
    assert!(
        r.as_ref().is_some_and(|r| r.title == CLIPPED_TITLE && r.command.is_empty() && r.tool_name.is_empty()),
        "notice clipped: no header or command from the screen top ({r:?})"
    );
    assert!(
        screen_prompt(&clipped).is_some() && screen_prompt(&clipped) == r.map(|r| r.tool_use_id),
        "notice clipped: the prompt still has its screen id"
    );
    let plain = (21..=40).map(|i| format!(" line {i} of a long heredoc")).collect::<Vec<_>>().join("\n") + &tail;
    assert_eq!(
        screen_request(&plain, "claude").map(|r| r.title).as_deref(),
        Some(CLIPPED_TITLE),
        "notice clipped: no gutter, only the command's closing rule left -> clipped too"
    );
    let all = (1..=40).map(|i| format!("line {i} of a long heredoc")).collect::<Vec<_>>().join("\n");
    let s = st("claude", true, Some("toolu_7"), &clipped, Some("Bash"), Some(&all));
    assert_eq!(super::prompt(&s, true), screen_prompt(&clipped), "notice clipped: the buttons answer the prompt on screen");
}

#[test]
fn notice_wrap() {
    // Claude 2.1.296 draws a "│" gutter on a wrapped command and breaks it mid-word (seen live).
    let dashed = rule('╌', 40);
    let wrapped = |l1: &str, l2: &str| {
        format!(
            "{}\n Bash command\n Tip: auto mode handles these prompts for you\n Create the file\n{dashed}\n │ {l1}\n │ {l2}\n{dashed}\n Do you want to proceed?\n ❯ 1. Yes\n   2. No\n Esc to cancel · Tab to amend",
            rule('─', 40)
        )
    };
    let mid_word = wrapped("touch /work/project/build/output/scrat", "chpad/notes.txt");
    let r = screen_request(&mid_word, "claude");
    assert_eq!(
        r.map(|r| r.command).as_deref(),
        Some("touch /work/project/build/output/scrat\nchpad/notes.txt"),
        "notice wrap: gutter removed from the command"
    );
    let s = |screen: &str, command: &str| st("claude", true, Some("toolu_9"), screen, Some("Bash"), Some(command));
    assert_eq!(
        super::prompt(&s(&mid_word, "touch /work/project/build/output/scratchpad/notes.txt"), true).as_deref(),
        Some("toolu_9"),
        "notice wrap: the log's call matches a box wrapped mid-word"
    );
    let at_space = wrapped("npm run build --workspace", "web");
    assert_eq!(
        super::prompt(&s(&at_space, "npm run build --workspace web"), true).as_deref(),
        Some("toolu_9"),
        "notice wrap: the log's call matches a box wrapped at a space"
    );
    assert_eq!(
        super::prompt(&s(&mid_word, "touch /work/project/build/output/scratchpad/other.txt"), true),
        screen_prompt(&mid_word),
        "notice wrap: another command still binds the screen id"
    );
    assert_eq!(
        super::prompt(&s(&mid_word, "touch /work/project/build/output/scrat"), true),
        screen_prompt(&mid_word),
        "notice wrap: a log command that is only the first line does not match"
    );
    // Not in the Swift checks: a log command that runs on past the box's last line does not match either.
    assert_eq!(
        super::prompt(&s(&at_space, "npm run build --workspace web && rm -rf /"), true),
        screen_prompt(&at_space),
        "(rust extra) notice wrap: a log command longer than the box does not match"
    );
}

#[test]
fn notice_p3_15() {
    // The older Claude Code layout: the description under the command, no dashed rules. The command read from the
    // box then holds the description too, and the log's call never matches it.
    let older = j(&[
        &rule('─', 40),
        " Bash command",
        "",
        "   npm run build",
        "   Build the web app",
        "",
        " Do you want to proceed?",
        " ❯ 1. Yes",
        "   2. No",
        "",
        " Esc to cancel · Tab to amend",
    ]);
    let logged = req("Bash", "npm run build", "Bash", "toolu_7");
    let on_screen = screen_request(&older, "claude");
    let card = card_request(Some(&logged), on_screen.as_ref());
    assert_eq!(card.as_ref().map(|c| c.tool_use_id.clone()), screen_prompt(&older), "p3-15: older layout -> the card binds the screen id");
    let now = st("claude", true, Some("toolu_7"), &older, Some("Bash"), Some("npm run build"));
    assert_eq!(
        outcome(true, card.as_ref().map(|c| c.tool_use_id.as_str()), Some(&now), true),
        Outcome::Send { allow: true },
        "p3-15: the composer card's id passes the re-check before the keys"
    );
    assert_eq!(outcome(true, Some("toolu_7"), Some(&now), true), Outcome::Open, "p3-15: the log's id would send nothing (why it gets no buttons)");
    // Current layout: the log's call is the box -> the card is the log's call.
    let current = card_request(Some(&logged), screen_request(&notice_prompt(), "claude").as_ref());
    assert_eq!(current.map(|c| c.tool_use_id).as_deref(), Some("toolu_7"), "p3-15: log matches the box -> the conversation card");
    assert_eq!(card_request(None, on_screen.as_ref()), on_screen, "p3-15: no log call -> the screen card");
    assert_eq!(card_request(None, None), None, "p3-15: no card -> none");
}

// MARK: ChecksCore.swift, ChecksHCCardFlow.swift (4: the marker)

#[test]
fn prompt_detection() {
    assert!(
        screen_shows_prompt(" Do you want to proceed?\n ❯ 1. Yes") && !screen_shows_prompt("Running tests…"),
        "screen marker"
    );
    assert!(screen_shows_prompt(" Do you want to proceed?\n ❯ 1. Yes\n   2. Yes, and don't ask again\n   3. No (esc)"), "4: the approval prompt");
    assert!(
        screen_shows_prompt("│ Do you want to make this edit to main.swift?\n│ ❯ 1. Yes\n│   2. No, and tell Claude what to do differently (esc)"),
        "4: edit prompt inside a box"
    );
    let question = j(&[
        "←  ☐ Deploy  ✔ Submit  →",
        "Do you want to deploy now?",
        "❯ 1. Yes",
        "  2. Later",
        "  3. Type something.",
        "────────────────────",
        "  4. Chat about this",
        "Enter to select · ↑/↓ to navigate · Esc to cancel",
    ]);
    assert!(!screen_shows_prompt(&question), "4: a question menu that asks 'Do you want' is not an approval");
    assert!(!screen_shows_prompt("⏺ Do you want me to refactor this too?\n> "), "4: the agent's own text with 'Do you want' is not an approval");
    assert!(!screen_shows_prompt("Do you want to proceed?\n(no options drawn yet)"), "4: no '1. Yes' option, no approval");
    assert!(
        screen_shows_prompt(" Do you want to make this edit to\n Sources/HerdrKit/SomeVeryLongFileName.swift?\n ❯ 1. Yes\n   2. No"),
        "4: a wrapped question still counts"
    );
}

// MARK: ChecksHCCards.swift (which checks run)

#[test]
fn hc_cards_which_checks() {
    let agent = |b: BackendKind, s: AgentStatus, k: Option<&str>, kind: &str| {
        let mut a = Agent::new("a", kind, s);
        a.backend = b;
        a.status_kind = k.map(String::from);
        a
    };
    use AgentStatus::*;
    use BackendKind::*;
    assert!(check_approval(Some(&agent(Hc, Blocked, Some("permission"), "claude")), false), "cards: hc blocked/permission checks the approval screen");
    assert!(!check_approval(Some(&agent(Hc, Blocked, Some("question"), "claude")), true), "cards: hc blocked/question never checks approval");
    assert!(check_approval(Some(&agent(Hc, Blocked, None, "claude")), false), "cards: hc blocked without kind: the screen decides");
    assert!(!check_approval(Some(&agent(Hc, Working, None, "claude")), true), "cards: hc not blocked: no check (7501 is the source)");
    assert!(
        check_approval(Some(&agent(Tui, Working, None, "claude")), true) && !check_approval(Some(&agent(Tui, Idle, None, "claude")), false),
        "cards: TUI unchanged (blocked or a pending tool call)"
    );
    assert!(
        read_question(Some(&agent(Hc, Blocked, Some("question"), "claude"))) && read_question(Some(&agent(Hc, Blocked, None, "claude"))),
        "cards: hc question read on question / no kind"
    );
    assert!(
        !read_question(Some(&agent(Hc, Blocked, Some("permission"), "claude"))) && !read_question(Some(&agent(Hc, Idle, Some("question"), "claude"))),
        "cards: no question read on permission or when not blocked"
    );
    assert!(
        read_question(Some(&agent(Tui, Blocked, None, "claude"))) && !read_question(Some(&agent(Tui, Blocked, None, "codex"))),
        "cards: TUI question read unchanged (Claude, blocked)"
    );
}

// MARK: ChecksAgy.swift (approval)

const AGY_PERM_SCREEN: &str = "● Bash(ls -d demo*) (ctrl+o to expand)

Command
──────────────────────────────────────────

Requesting permission for:
   ls -d demo*

Run this command?
> 1. Yes, run command
  2. Yes, and always allow in this conversation for commands that start with 'ls'
  3. Yes, and always allow for commands that start with 'ls' (Persist to settings.json)
  4. No, cancel

  ↑/↓ Navigate · tab Amend · ctrl+g edit/expand command
esc to cancel                                                     accept-edits · Gemini 4 Argon · medium";

const AGY_QUESTION_SCREEN: &str = "> Use your ask_question tool: which color do I prefer, red or blue?

? Which color do you prefer, red or blue?

Question
──────────────────────────────────────────

Question 1/1: Which color do you prefer, red or blue?

> 1. Red
  2. Blue
  3. Write-in...

  ↑/↓ Navigate · enter Select · esc Skip
esc to cancel                                                     accept-edits · Gemini 4 Argon · medium";

#[test]
fn agy_approval() {
    assert!(screen_shows_prompt(AGY_PERM_SCREEN), "agy: permission prompt on screen");
    assert!(!screen_shows_prompt(AGY_QUESTION_SCREEN), "agy: question picker is not a permission prompt");
    let state = |screen: &str, kind: &str, cmd: Option<&str>, blocked: bool| {
        st(kind, blocked, cmd.map(|_| "ag1-0"), screen, cmd.map(|_| "Bash"), cmd)
    };
    let cmd = Some("ls -d demo*");
    let ids = prompt_ids(&state(AGY_PERM_SCREEN, "agy", cmd, true), true);
    assert!(ids.len() == 2 && ids[0] == "ag1-0" && ids[1].starts_with("screen-"), "agy: the log's call is the box on screen {ids:?}");
    let last = vec![ids[1].clone()];
    assert_eq!(prompt_ids(&state(AGY_PERM_SCREEN, "agy", Some("rm -rf demo"), true), true), last, "agy: another command in the log: the box only");
    assert_eq!(prompt_ids(&state(AGY_PERM_SCREEN, "agy", None, true), true), last, "agy: no log call: the box");
    assert!(
        prompt_ids(&state(AGY_PERM_SCREEN, "agy", cmd, true), false).is_empty() && prompt_ids(&state(AGY_PERM_SCREEN, "agy", cmd, false), true).is_empty(),
        "agy: no buttons without input or block"
    );
    assert!(prompt_ids(&state(AGY_QUESTION_SCREEN, "agy", cmd, true), true).is_empty(), "agy: a question picker gets no approval buttons");
    assert!(prompt_ids(&state(AGY_PERM_SCREEN, "claude", cmd, true), true).is_empty(), "agy: the agy box is not read for Claude");
    let moved = AGY_PERM_SCREEN.replace("> 1. Yes", "  1. Yes").replace("  4. No", "> 4. No");
    assert_eq!(prompt_ids(&state(&moved, "agy", None, true), true), last, "agy: the cursor moving keeps the prompt id");
    let two = AGY_PERM_SCREEN.replace("  2. Yes", "> 2. Yes");
    assert!(prompt_ids(&state(&two, "agy", cmd, true), true).is_empty(), "agy: two cursors: not read");
    let clipped = AGY_PERM_SCREEN.split('\n').skip(4).collect::<Vec<_>>().join("\n");
    assert!(prompt_ids(&state(&clipped, "agy", cmd, true), true).is_empty(), "agy: box top off screen: not read");
    let r = screen_request(AGY_PERM_SCREEN, "agy");
    assert!(
        r.as_ref().is_some_and(|r| r.command == "ls -d demo*" && r.tool_name == "Bash" && r.tool_use_id == ids[1]),
        "agy: card request from the screen {r:?}"
    );
    let mut agy = Agent::new("a", "agy", AgentStatus::Blocked);
    agy.session_id = Some("/x.jsonl".into());
    agy.status_kind = Some("permission".into());
    let mut q = agy.clone();
    q.status_kind = Some("question".into());
    assert!(check_approval(Some(&agy), true) && !check_approval(Some(&q), true), "agy: hc 7501 kind gates the approval card");
    // ChecksAgy (questions): hc question kind: no approval, no screen read.
    assert!(!check_approval(Some(&q), true) && !read_question(Some(&q)), "agy: hc question kind: no approval, no screen read");
}

// MARK: ChecksGrokApproval.swift

const REJECT: &str = "No, reject (type to add feedback)";

/// The prompt as Grok Build 1.0.67 draws it (shape captured live; text synthetic). `focus`: the row with (●).
fn grok_prompt(command: &str, focus: usize, reject: &str) -> String {
    let rows = ["Yes, and don't ask again for anything (always-approve mode)", "Yes, proceed", reject, "Never allow: echo"];
    let mut l: Vec<String> = ["  ❯ Write a file.", "", "  ◆ Write hi into out.txt… 3.0s", "", "  ┃", "  ┃  ›  Write hi into out.txt"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    l.push(format!("  ┃  │ {command}"));
    l.extend(["  ┃  ← → narrow scope".to_string(), "  ┃".to_string()]);
    for (i, r) in rows.iter().enumerate() {
        l.push(format!("  ┃  {} ({}) {r}", i + 1, if i + 1 == focus { "●" } else { "○" }));
    }
    l.extend([
        "  ┃".to_string(),
        String::new(),
        "  1/4:select  │  Tab:next option  │  ←/→:scope  │  Ctrl+o:always-approve  │  Ctrl+c:cancel  │  Esc:scrollback".to_string(),
    ]);
    l.join("\n")
}

fn gp(focus: usize) -> String {
    grok_prompt("echo hi > out.txt", focus, REJECT)
}

const GROK_IDLE: &str = "  ╭──────╮\n  │ ❯    │\n  ╰──────╯\n  Shift+Tab:mode  │  Ctrl+.:shortcuts";

#[test]
fn grok_approval() {
    let s = gp(2);
    let b = grok_box(&s);
    assert!(
        b.as_ref().is_some_and(|b| b.title == "Write hi into out.txt" && b.command == "echo hi > out.txt" && b.focus == 1),
        "grok approval: the prompt read from the screen"
    );
    assert!(screen_shows_prompt(&s), "grok approval: a prompt on screen takes keys");
    assert!(grok_box(GROK_IDLE).is_none() && !screen_shows_prompt(GROK_IDLE), "grok approval: no prompt on the idle screen");
    let question = "  ┃\n  ┃  Which color?\n  ┃\n  ┃  1 (○) Red  Warm.\n  ┃  2 (○) Blue  Cool.\n  ┃  z (○) Type your answer here\n  ┃\n  ┃  [1/1] ↑/↓ navigate · ←/→ question · y copy  Enter:submit\n  ┃\n\n  Tab:next answer  │  Esc:scrollback  │  Shift+x:dismiss";
    assert!(grok_box(question).is_none(), "grok approval: a question picker is not a prompt");

    // The id: the prompt on screen, the same while the focus moves; the log's open call too when it is the same command.
    let gst = |screen: &str, kind: &str, blocked: bool, logged: Option<&PermissionRequest>| {
        st(
            kind,
            blocked,
            logged.map(|l| l.tool_use_id.as_str()),
            screen,
            logged.map(|l| l.tool_name.as_str()),
            logged.map(|l| l.command.as_str()),
        )
    };
    let on_screen = prompt_ids(&gst(&s, "grok", true, None), true);
    assert!(on_screen.len() == 1 && on_screen[0].starts_with("screen-"), "grok approval: the screen's id {on_screen:?}");
    assert_eq!(prompt_ids(&gst(&gp(3), "grok", true, None), true), on_screen, "grok approval: the focus moving keeps the id");
    assert_ne!(
        prompt_ids(&gst(&grok_prompt("echo b > out.txt", 2, REJECT), "grok", true, None), true),
        on_screen,
        "grok approval: another command, another id"
    );
    let call = req("Bash", "echo hi > out.txt", "Bash", "call-1");
    assert_eq!(
        prompt_ids(&gst(&s, "grok", true, Some(&call)), true),
        vec!["call-1".to_string(), on_screen[0].clone()],
        "grok approval: the log's open call is the prompt"
    );
    let mut other = call.clone();
    other.command = "ls".into();
    assert_eq!(prompt_ids(&gst(&s, "grok", true, Some(&other)), true), on_screen, "grok approval: a lagging log call does not name the prompt");
    assert!(
        prompt_ids(&gst(&s, "grok", false, None), true).is_empty()
            && prompt_ids(&gst(&s, "grok", true, None), false).is_empty()
            && prompt_ids(&gst(&s, "claude", true, None), true).is_empty(),
        "grok approval: blocked grok with input on only"
    );
    let r = screen_request(&s, "grok");
    assert!(
        r.as_ref().is_some_and(|r| r.tool_use_id == on_screen[0] && r.command == "echo hi > out.txt" && r.tool_name == "Bash")
            && screen_request(&s, "claude").is_none(),
        "grok approval: the card from the screen"
    );

    // Keys (live): Enter answers the row with focus. Allow: only from "Yes, proceed". Deny: Down from it to "No, reject"
    // right below, then Enter. The always-approve and never-allow rows are never chosen; any other focus sends nothing.
    assert!(
        grok_key(&s, true) == Some("Enter") && grok_key(&gp(3), true).is_none() && grok_key(&gp(1), true).is_none(),
        "grok approval keys: allow from Yes, proceed only"
    );
    assert!(
        grok_key(&s, false) == Some("Down")
            && grok_key(&gp(3), false) == Some("Enter")
            && grok_key(&gp(4), false).is_none()
            && grok_key(&gp(1), false).is_none(),
        "grok approval keys: deny via No, reject"
    );
    assert!(
        grok_key(&grok_prompt("echo hi > out.txt", 3, "No, reject: too risky"), false).is_none(),
        "grok approval keys: typed feedback is the user's: no key"
    );
    // Not in the Swift checks: Down only when "No, reject" is the row right below "Yes, proceed".
    let far = gp(2).replace("3 (○) No, reject (type to add feedback)", "3 (○) Never allow: rm").replace("4 (○) Never allow: echo", "4 (○) No, reject (type to add feedback)");
    assert!(grok_box(&far).is_some_and(|b| b.no() == Some(3) && b.yes() == Some(1)), "(setup) No, reject two rows below");
    assert_eq!(grok_key(&far, false), None, "(rust extra) grok approval keys: no Down past another row");
}

// MARK: ChecksGrokRemote.swift (the permission card)

/// ChecksGrokRemote's prompt: the block without the lines above it.
fn grok_remote_prompt(focus: usize) -> String {
    let rows = ["Yes, and don't ask again for anything (always-approve mode)", "Yes, proceed", REJECT, "Never allow: echo"];
    let mut l: Vec<String> = ["  ┃", "  ┃  ›  Write hi into out.txt", "  ┃  │ echo hi > out.txt", "  ┃"].iter().map(|s| s.to_string()).collect();
    for (i, r) in rows.iter().enumerate() {
        l.push(format!("  ┃  {} ({}) {r}", i + 1, if i + 1 == focus { "●" } else { "○" }));
    }
    l.extend([
        "  ┃".to_string(),
        String::new(),
        "  1/4:select  │  Tab:next option  │  ←/→:scope  │  Ctrl+o:always-approve  │  Ctrl+c:cancel  │  Esc:scrollback".to_string(),
    ]);
    l.join("\n")
}

fn grok_agent(session: &str) -> Agent {
    tui_agent("grok", Some(session), AgentStatus::Blocked)
}

#[test]
fn grok_remote_permission() {
    let agent = grok_agent("g1");
    let gate = |allowed: Allowed| KeyGate::new(Some(agent.clone()), Some(allowed));
    let p2 = grok_remote_prompt(2);
    let p3 = grok_remote_prompt(3);
    let id = prompt_ids(&st("grok", true, None, &p2, None, None), true).first().cloned().unwrap_or_default();
    let fake = |screens: Vec<&str>| FakeScreen::with_panes(vec![tui_pane(Some(grok_agent("g1")))], screens);

    // Deny = Down, then Enter once the focus is on "No, reject"; each key gated.
    let f = fake(vec![&p2, &p3, GROK_IDLE]);
    let denied = respond_if_open(&f, "w1:p1", &id, false, "grok", true, None, Some(&gate(yes())));
    assert_eq!(denied, Ok(true), "grok remote: Deny answered");
    assert_eq!(f.sent(), ["Down", "Enter"], "grok remote: Deny = Down, then Enter");

    let f = fake(vec![&p2, GROK_IDLE]);
    let allowed = respond_if_open(&f, "w1:p1", &id, true, "grok", true, None, Some(&gate(yes())));
    assert!(allowed == Ok(true) && f.sent() == ["Enter"], "grok remote: Allow = Enter {:?}", f.sent());

    let p1 = grok_remote_prompt(1);
    let f = fake(vec![&p1]);
    let off = respond_if_open(&f, "w1:p1", &id, true, "grok", true, None, Some(&gate(yes())));
    assert!(off == Ok(false) && f.sent().is_empty(), "grok remote: the focus on always-approve: nothing sent {:?}", f.sent());

    // The level goes after Down (checks: the open prompt, Down, then Enter): no Enter.
    let f = fake(vec![&p2, &p3, GROK_IDLE]);
    let cut = respond_if_open(&f, "w1:p1", &id, false, "grok", true, None, Some(&gate(budget(2))));
    assert!(cut == Ok(false) && f.sent() == ["Down"], "grok remote: the level gone between keys: no Enter {:?}", f.sent());
}

// MARK: ChecksHCGrok.swift (respondIfOpen on an hc pane)

#[test]
fn hc_grok_respond() {
    let rows = ["Yes, and don't ask again for anything (always-approve mode)", "Yes, proceed", REJECT, "Never allow: echo"];
    let bx = |focus: usize| {
        let mut l: Vec<String> = [
            "  ◆ Write hi into out.txt… 3.0s",
            "",
            "  ┃",
            "  ┃  ›  Write hi into out.txt",
            "  ┃  │ echo hi > out.txt",
            "  ┃  ← → narrow scope",
            "  ┃",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        for (i, r) in rows.iter().enumerate() {
            l.push(format!("  ┃  {} ({}) {r}", i + 1, if i + 1 == focus { "●" } else { "○" }));
        }
        l.extend([
            "  ┃".to_string(),
            String::new(),
            "  1/4:select  │  Tab:next option  │  ←/→:scope  │  Ctrl+o:always-approve  │  Ctrl+c:cancel  │  Esc:scrollback".to_string(),
        ]);
        l.join("\n")
    };
    let b2 = bx(2);
    let b3 = bx(3);
    let panes = || vec![hc_pane("t1", "grok", AgentStatus::Blocked)];
    let f = FakeScreen::with_panes(panes(), vec![&b2]);
    assert!(screen_shows_prompt(&f.visible_text("hc:t1").unwrap()), "hc grok: the box on the hc screen is a prompt");
    let s = approval_state(&f, "hc:t1", "grok", true, None).expect("hc grok: state");
    let id = prompt_ids(&s, true).last().cloned().expect("hc grok: the box has an id");

    let f = FakeScreen::with_panes(panes(), vec![&b2, GROK_IDLE]);
    let allowed = respond_if_open(&f, "hc:t1", &id, true, "grok", true, None, None);
    assert!(allowed == Ok(true) && f.sent() == ["Enter"], "hc grok: Allow = Enter ({:?})", f.sent());
    // Deny: Down to "No, reject", then Enter once the focus is there.
    let f = FakeScreen::with_panes(panes(), vec![&b2, &b3, GROK_IDLE]);
    let denied = respond_if_open(&f, "hc:t1", &id, false, "grok", true, None, None);
    assert!(denied == Ok(true) && f.sent() == ["Down", "Enter"], "hc grok: Deny = Down, Enter ({:?})", f.sent());
    // Another box on screen: the bound prompt is gone, nothing is sent.
    let other = b2.replace("echo hi > out.txt", "rm -rf out");
    let f = FakeScreen::with_panes(panes(), vec![&other]);
    let stale = respond_if_open(&f, "hc:t1", &id, true, "grok", true, None, None);
    assert!(stale == Ok(false) && f.sent().is_empty(), "hc grok: another box: nothing sent");

    // The hub on hc: a device's gate is asked before the keys; a refusal sends nothing.
    let f = FakeScreen::with_panes(panes(), vec![&b2]);
    let refused = respond_if_open(&f, "hc:t1", &id, true, "grok", true, None, Some(&KeyGate::new(None, Some(no()))));
    assert!(refused == Ok(false) && f.sent().is_empty(), "hc hub: a gate that says no sends nothing");
}

// MARK: ChecksHubSecurity.swift (the approval parts)

fn security_box(command: &str) -> String {
    j(&[
        &rule('─', 40),
        " Bash command",
        " Run it",
        &rule('╌', 40),
        &format!(" {command}"),
        &rule('╌', 40),
        " Do you want to proceed?",
        " ❯ 1. Yes",
        "   2. No",
        "",
        " Esc to cancel · Tab to amend",
    ])
}

#[test]
fn security_log_lag() {
    // (3) Log lag: the log's pending call is A, the screen shows prompt B. An id is good only for the prompt on screen.
    let screen_b = security_box("rm -rf build");
    let lagging = st("claude", true, Some("toolu_A"), &screen_b, Some("Bash"), Some("npm run build"));
    assert_eq!(super::prompt(&lagging, true), screen_prompt(&screen_b), "security: log lags the screen -> buttons bound to the screen prompt");
    assert_eq!(outcome(true, Some("toolu_A"), Some(&lagging), true), Outcome::Open, "security: an id for the log's call A does not answer screen prompt B");
    let caught_up = st("claude", true, Some("toolu_B"), &screen_b, Some("Bash"), Some("rm   -rf build"));
    assert_eq!(super::prompt(&caught_up, true).as_deref(), Some("toolu_B"), "security: the log's call matches the box (tool + command) -> its id is good");
    assert_eq!(
        outcome(true, screen_prompt(&screen_b).as_deref(), Some(&caught_up), true),
        Outcome::Send { allow: true },
        "security: the screen id stays good when the log matches"
    );
    let unknown_call = st("claude", true, Some("toolu_B"), &screen_b, None, None);
    assert_eq!(super::prompt(&unknown_call, true), screen_prompt(&screen_b), "security: a log call without tool and command is not trusted for the box");
}

#[test]
fn security_respond_if_open() {
    // (3) The re-check before the keys: reads, then sends only when the bound prompt is still open.
    let screen_a = security_box("npm run build");
    let id_a = screen_prompt(&screen_a).unwrap_or_default();
    let f = FakeScreen::new(vec![], vec![&screen_a]);
    let sent = respond_if_open(&f, "w1:p1", &id_a, true, "claude", true, None, None);
    assert!(sent == Ok(true) && f.sent() == ["respond:allow"], "security: respondIfOpen sends when the prompt is still open {:?}", f.sent());
    let other = security_box("rm -rf /");
    let f = Rec::new(FakeScreen::new(vec![], vec![&other]));
    let again = respond_if_open(&f, "w1:p1", &id_a, true, "claude", true, None, None);
    assert!(
        again == Ok(false) && f.fake.sent().is_empty() && f.log().iter().all(|c| c == "read"),
        "security: respondIfOpen sends nothing when another prompt is open {:?}",
        f.log()
    );
    // Deny on the same prompt: the deny input, once.
    let f = FakeScreen::new(vec![], vec![&screen_a]);
    assert!(
        respond_if_open(&f, "w1:p1", &id_a, false, "claude", true, None, None) == Ok(true) && f.sent() == ["respond:deny"],
        "security: Deny on the open prompt sends the deny input"
    );
}

#[test]
fn security_unreadable_prompt() {
    // (5) A permission prompt the parser cannot read gets no id (the hub then refuses input: not ported here).
    let odd = format!("{}\n Do you want to make this edit?", rule('─', 16));
    assert_eq!(screen_prompt(&odd), None, "security: (setup) the parser does not read this prompt");
    let f = FakeScreen::new(vec![], vec![&odd]);
    assert!(
        respond_if_open(&f, "w1:p1", "x", true, "claude", true, None, None) == Ok(false) && f.sent().is_empty(),
        "security: unreadable prompt -> approve sends nothing"
    );
    // (b) A screen that cannot be read refuses (fail closed), with nothing sent.
    let f = FakeScreen::new(vec![], vec![&odd]);
    *f.fail_reads.borrow_mut() = 1;
    assert!(
        respond_if_open(&f, "w1:p1", "x", true, "claude", true, None, None) == Ok(false) && f.sent().is_empty(),
        "review2: unreadable screen -> approve sends nothing"
    );
}

#[test]
fn security_review2_card() {
    // (a) The in-app card: it binds the request it shows, and shows the screen prompt when the log lags.
    let screen_b = format!(
        "{}\n Bash command\n Run it\n{d}\n rm -rf build\n{d}\n Do you want to proceed?\n ❯ 1. Yes\n   2. No\n\n Esc to cancel · Tab to amend",
        rule('─', 40),
        d = rule('╌', 10)
    );
    let on_screen = screen_request(&screen_b, "claude");
    let lagging = req("Bash", "npm run build", "Bash", "toolu_A");
    let caught_up = req("Bash", "rm -rf build", "Bash", "toolu_B");
    let id = |r: Option<PermissionRequest>| r.map(|r| r.tool_use_id);
    assert_eq!(
        id(card_request(Some(&lagging), on_screen.as_ref())),
        screen_prompt(&screen_b),
        "review2: log lags the screen -> the card shows (and binds) the screen prompt"
    );
    assert_eq!(
        id(card_request(Some(&caught_up), on_screen.as_ref())).as_deref(),
        Some("toolu_B"),
        "review2: log matches the box -> the card shows the log's call"
    );
    assert_eq!(
        id(card_request(Some(&lagging), None)).as_deref(),
        Some("toolu_A"),
        "review2: no box read -> the log's call (the keys still need the box)"
    );
    let s = st("claude", true, Some("toolu_A"), &screen_b, Some("Bash"), Some("npm run build"));
    let bound = id(card_request(Some(&lagging), on_screen.as_ref()));
    assert_eq!(outcome(true, bound.as_deref(), Some(&s), true), Outcome::Send { allow: true }, "review2: the card's bound id passes the re-check");
}

// MARK: ChecksHubReview4.swift, ChecksHubReview5.swift (approve)

const R4_BOX: &str = " Bash command
 npm run build
 Do you want to proceed?
 ❯ 1. Yes
   2. No

 Esc to cancel · Tab to amend";

#[test]
fn review4_approve() {
    // Approve: no key when the agent changed or the level went; sent when all holds.
    let seen = tui_agent("claude", Some("s1"), AgentStatus::Blocked);
    let id = screen_prompt(R4_BOX).unwrap_or_default();
    let f = FakeScreen::with_panes(vec![tui_pane(Some(tui_agent("claude", Some("s2"), AgentStatus::Blocked)))], vec![R4_BOX]);
    let r1 = respond_if_open(&f, "w1:p1", &id, true, "claude", true, None, Some(&KeyGate::new(Some(seen.clone()), Some(yes()))));
    assert!(r1 == Ok(false) && f.sent().is_empty(), "r4 approve: agent swapped -> nothing sent");
    let pane = || tui_pane(Some(tui_agent("claude", Some("s1"), AgentStatus::Blocked)));
    let f = FakeScreen::with_panes(vec![pane()], vec![R4_BOX]);
    let r2 = respond_if_open(&f, "w1:p1", &id, true, "claude", true, None, Some(&KeyGate::new(Some(seen.clone()), Some(no()))));
    assert!(r2 == Ok(false) && f.sent().is_empty(), "r4 approve: level gone -> nothing sent");
    let f = Rec::new(FakeScreen::with_panes(vec![pane()], vec![R4_BOX]));
    let r3 = respond_if_open(&f, "w1:p1", &id, true, "claude", true, None, Some(&KeyGate::new(Some(seen.clone()), Some(yes()))));
    assert!(r3 == Ok(true) && f.fake.sent() == ["respond:allow"], "r4 approve: all holds -> one key sent");
    let log = f.log();
    assert!(
        log.len() >= 2 && log[log.len() - 2] == "read" && log[log.len() - 1] == "respond",
        "r4 approve: the screen read is right before the key ({log:?})"
    );
}

#[test]
fn review5_approve_fresh_status() {
    // (5) Approve reads the agent's status fresh: not blocked now -> no key.
    let seen = tui_agent("claude", Some("s1"), AgentStatus::Idle);
    let f = FakeScreen::with_panes(vec![tui_pane(Some(tui_agent("claude", Some("s1"), AgentStatus::Working)))], vec![R4_BOX]);
    let id = screen_prompt(R4_BOX).unwrap_or_default();
    let ap = respond_if_open(&f, "w1:p1", &id, true, "claude", true, None, Some(&KeyGate::new(Some(seen), Some(yes()))));
    assert!(ap == Ok(false) && f.sent().is_empty(), "r5 approve: the agent is not blocked now (stale 'blocked' from the check) -> no key");
}
