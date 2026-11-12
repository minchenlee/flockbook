//! Parity with Swift's `HerdrKitChecks` (ChecksQuestions, ChecksGrokQuestion, ChecksAgy, ChecksClaudeHook*,
//! ChecksHubReview4–7, ChecksGrokRemote, ChecksHCCards, ChecksModFeedCards): the same synthetic screens, the same
//! expected values and key sequences. Each `assert!` names the Swift check it ports.

use super::*;
use crate::fake::hc_pane;
use crate::keygate::{Allowed, KeyGate};
use serde_json::json;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

// MARK: helpers

/// A JSON object literal (`[String: Any]`).
macro_rules! o {
    ($($t:tt)*) => {
        json!($($t)*).as_object().unwrap().clone()
    };
}

/// A Swift multi-line literal: the first newline dropped, the closing line's indentation removed from every line.
fn sw(s: &str) -> String {
    let s = s.strip_prefix('\n').unwrap_or(s);
    let (body, indent) = s.rsplit_once('\n').expect("closing line");
    assert!(indent.chars().all(|c| c == ' '), "closing line is indentation only");
    body.split('\n')
        .map(|l| match l.strip_prefix(indent) {
            Some(r) => r,
            None if l.trim().is_empty() => "",
            None => panic!("line not indented: {l:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn opt(l: &str) -> QOption {
    option(l, "", false)
}

fn optd(l: &str, d: &str) -> QOption {
    option(l, d, false)
}

fn question(header: &str, text: &str, options: Vec<QOption>, multi: bool) -> Question {
    new_question(None, header, text, options, multi)
}

fn request(id: &str, agent: &str, questions: Vec<Question>) -> QuestionRequest {
    new_request(id, agent, Delivery::Picker, questions)
}

fn ans(picks: &[usize]) -> QuestionAnswer {
    QuestionAnswer { picks: picks.to_vec(), other: None }
}

fn other(picks: &[usize], text: &str) -> QuestionAnswer {
    QuestionAnswer { picks: picks.to_vec(), other: Some(text.into()) }
}

fn text(s: &str) -> Step {
    Step::Text(s.into())
}

fn key(k: &[&str]) -> Step {
    Step::Keys(k.iter().map(|s| s.to_string()).collect())
}

fn wait(s: &str) -> Step {
    Step::WaitFor(s.into())
}

fn yes() -> Allowed {
    Arc::new(|| true)
}

fn no() -> Allowed {
    Arc::new(|| false)
}

/// `allowed` says yes `n` times, then no.
fn budget(n: i64) -> Allowed {
    let left = Arc::new(AtomicI64::new(n));
    Arc::new(move || left.fetch_sub(1, Ordering::SeqCst) - 1 >= 0)
}

/// The agent as a TUI `herdr` pane shows it (bound by its session id).
fn tui_agent(kind: &str, session: Option<&str>) -> Agent {
    let mut a = Agent::new(kind, kind, AgentStatus::Blocked);
    a.backend = BackendKind::Tui;
    a.session_id = session.map(String::from);
    a
}

fn tui_pane(agent: Agent) -> Pane {
    Pane {
        id: "w1:p1".into(),
        terminal_id: "t1".into(),
        cwd: "/w".into(),
        agent: Some(agent),
        label: None,
        backend: BackendKind::Tui,
        process_ids: vec![],
    }
}

fn gate(a: &Agent, allowed: Allowed) -> KeyGate {
    KeyGate::new(Some(a.clone()), Some(allowed))
}

#[derive(Clone, Debug, PartialEq)]
enum Input {
    /// `send_named_keys`
    Keys(Vec<String>),
    /// `send_keys` (typed as is)
    Bytes(String),
    /// `send_text` (text + Enter)
    Text(String),
}

type OnInput = Box<dyn FnMut(&Input, &Live)>;

/// A pane whose screen reacts to input (Swift: `FakeHerdr.onInput` / `FakeHC.onKeys`). Records every input.
struct Live {
    panes: RefCell<Vec<Pane>>,
    screen: RefCell<String>,
    inputs: RefCell<Vec<Input>>,
    on_input: RefCell<Option<OnInput>>,
    /// A slow render: the screen shown after this many more reads.
    later: RefCell<Option<(usize, String)>>,
}

impl Live {
    fn new(pane: Pane, screen: &str) -> Live {
        Live { panes: RefCell::new(vec![pane]), screen: RefCell::new(screen.into()), inputs: RefCell::new(vec![]), on_input: RefCell::new(None), later: RefCell::new(None) }
    }
    /// Shows `s` only after `reads` more screen reads (the TUI renders slowly on a busy machine).
    fn set_later(&self, reads: usize, s: &str) {
        *self.later.borrow_mut() = Some((reads, s.into()));
    }
    fn set(&self, s: &str) {
        *self.screen.borrow_mut() = s.into();
    }
    fn get(&self) -> String {
        self.screen.borrow().clone()
    }
    fn set_agent(&self, a: Agent) {
        self.panes.borrow_mut()[0].agent = Some(a);
    }
    fn on(&self, f: impl FnMut(&Input, &Live) + 'static) {
        *self.on_input.borrow_mut() = Some(Box::new(f));
    }
    fn off(&self) {
        *self.on_input.borrow_mut() = None;
    }
    fn mark(&self) -> usize {
        self.inputs.borrow().len()
    }
    fn after(&self, n: usize) -> Vec<Input> {
        self.inputs.borrow()[n..].to_vec()
    }
    /// Swift `sent(after:)`: named keys joined by ",", text as typed.
    fn sent(&self, n: usize) -> Vec<String> {
        self.after(n)
            .into_iter()
            .map(|i| match i {
                Input::Keys(k) => k.join(","),
                Input::Bytes(s) | Input::Text(s) => s,
            })
            .collect()
    }
    /// Named keys joined, any text as "text" (ChecksHubReview6/7).
    fn sent_or_text(&self, n: usize) -> Vec<String> {
        self.after(n)
            .into_iter()
            .map(|i| match i {
                Input::Keys(k) => k.join(","),
                _ => "text".into(),
            })
            .collect()
    }
    /// The calls as hc's `terminal.keys` names them (ChecksHCCards, ChecksModFeedCards).
    fn hc_keys(&self, n: usize) -> Vec<Vec<String>> {
        self.after(n)
            .into_iter()
            .map(|i| match i {
                Input::Keys(k) => k.iter().map(|x| hc_translate(x)).collect(),
                Input::Bytes(s) | Input::Text(s) => hc_names(&s),
            })
            .collect()
    }
    fn input(&self, i: Input) -> Result<(), HerdrError> {
        self.inputs.borrow_mut().push(i.clone());
        let f = self.on_input.borrow_mut().take();
        if let Some(mut f) = f {
            f(&i, self);
            let mut slot = self.on_input.borrow_mut();
            if slot.is_none() {
                *slot = Some(f);
            }
        }
        Ok(())
    }
}

/// `HCKeys.translate` for the names these checks use.
fn hc_translate(k: &str) -> String {
    if gcount(k) == 1 {
        return k.into();
    }
    match k.to_lowercase().as_str() {
        "escape" => "esc".into(),
        "return" => "enter".into(),
        l => l.into(),
    }
}

/// `HCKeys.names(for:)` for plain text.
fn hc_names(s: &str) -> Vec<String> {
    s.chars()
        .map(|c| match c {
            '\r' | '\n' => "enter".into(),
            '\u{1b}' => "esc".into(),
            '\t' => "tab".into(),
            ' ' => "space".into(),
            c => c.to_string(),
        })
        .collect()
}

impl PaneScreen for Live {
    fn backend(&self) -> BackendKind {
        BackendKind::Hc
    }
    fn snapshot(&self) -> Result<Vec<Workspace>, HerdrError> {
        Ok(vec![Workspace {
            id: "w1".into(),
            label: "w".into(),
            tabs: vec![Tab { id: "w1:t1".into(), label: "t".into(), panes: self.panes.borrow().clone(), ..Default::default() }],
        }])
    }
    fn visible_text(&self, _pane_id: &str) -> Result<String, HerdrError> {
        let mut later = self.later.borrow_mut();
        if let Some((n, s)) = later.as_mut() {
            if *n == 0 {
                *self.screen.borrow_mut() = std::mem::take(s);
                *later = None;
            } else {
                *n -= 1;
            }
        }
        drop(later);
        Ok(self.get())
    }
    fn respond(&self, _pane_id: &str, allow: bool) -> Result<(), HerdrError> {
        self.input(Input::Keys(vec![if allow { "1".into() } else { "Escape".into() }]))
    }
    fn send_named_keys(&self, keys: &[String], _pane_id: &str) -> Result<(), HerdrError> {
        self.input(Input::Keys(keys.to_vec()))
    }
    fn send_keys(&self, bytes: &[u8], _pane_id: &str) -> Result<(), HerdrError> {
        self.input(Input::Bytes(String::from_utf8_lossy(bytes).into_owned()))
    }
    fn send_text(&self, text: &str, _pane_id: &str) -> Result<(), HerdrError> {
        self.input(Input::Text(text.into()))
    }
    // As hc: key names type only some characters (crate::hc::keys::typeable).
    fn can_type(&self, text: &str) -> bool {
        text.chars().all(crate::hc::keys::typeable)
    }
    fn pause(&self, _ms: u64) {}
}

fn is_gate_stop<T>(r: &Result<T, HerdrError>) -> bool {
    matches!(r, Err(HerdrError::KeyGateStop))
}

// MARK: Claude Code 2.1.296's picker, as captured live (Swift: `ClaudePickerSim`)

#[derive(Clone)]
struct SimQ {
    header: String,
    text: String,
    options: Vec<(String, String)>,
    multi: bool,
}

fn simq(header: &str, text: &str, options: &[(&str, &str)], multi: bool) -> SimQ {
    SimQ { header: header.into(), text: text.into(), options: options.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), multi }
}

struct Sim {
    qs: Vec<SimQ>,
    at: usize,
    focus: usize,
    review: bool,
    done: bool,
    typing: bool,
    checks: Vec<BTreeSet<usize>>,
    typed: Vec<Option<String>>,
    above: String,
    single: std::collections::HashMap<usize, usize>,
}

impl Sim {
    fn new(qs: Vec<SimQ>) -> Sim {
        let n = qs.len();
        Sim {
            qs,
            at: 0,
            focus: 0,
            review: false,
            done: false,
            typing: false,
            checks: vec![BTreeSet::new(); n],
            typed: vec![None; n],
            above: "❯ Use AskUserQuestion\n────────────────────────────────────────".into(),
            single: Default::default(),
        }
    }

    fn answered(&self, k: usize) -> bool {
        self.single.contains_key(&k) || !self.checks[k].is_empty() || (!self.qs[k].multi && self.typed[k].is_some() && k < self.at)
    }

    fn bar(&self) -> String {
        if self.qs.len() == 1 && !self.qs[0].multi {
            let b = if self.checks[0].is_empty() && self.typed[0].is_none() { "☐" } else { "☒" };
            return format!(" {} {}", b, self.qs[0].header);
        }
        let tabs: Vec<String> =
            (0..self.qs.len()).map(|k| format!("{} {}", if self.answered(k) { "☒" } else { "☐" }, self.qs[k].header)).collect();
        format!("←  {}  ✔ Submit  →", tabs.join("  "))
    }

    fn render(&self) -> String {
        if self.done {
            return format!("{}\n⏺ User answered Claude's questions:\n❯ ", self.above);
        }
        if self.review {
            let mut l = vec![self.above.clone(), self.bar(), "".into(), "Review your answers".into(), "".into()];
            for (k, q) in self.qs.iter().enumerate() {
                let a: Vec<String> = if q.multi {
                    let mut v: Vec<String> = self.checks[k].iter().filter(|&&i| i < q.options.len()).map(|&i| q.options[i].0.clone()).collect();
                    if self.checks[k].contains(&q.options.len()) {
                        v.push(self.typed[k].clone().unwrap_or_default());
                    }
                    v
                } else {
                    match self.single.get(&k) {
                        Some(&i) => vec![q.options[i].0.clone()],
                        None => vec![self.typed[k].clone().unwrap_or_default()],
                    }
                };
                l.push(format!(" ● {}", q.text));
                l.push(format!("   → {}", a.join(", ")));
            }
            l.extend(["", "Ready to submit your answers?", "", "❯ 1. Submit answers", "  2. Cancel"].map(String::from));
            return l.join("\n");
        }
        let q = &self.qs[self.at];
        let n = q.options.len();
        let mut l = vec![self.above.clone(), self.bar(), "".into(), q.text.clone(), "".into()];
        let mark = |i: usize| if self.focus == i { "❯" } else { " " };
        for (i, o) in q.options.iter().enumerate() {
            let b = if q.multi { if self.checks[self.at].contains(&i) { "[✔] " } else { "[ ] " } } else { "" };
            l.push(format!("{} {}. {}{}", mark(i), i + 1, b, o.0));
            if !o.1.is_empty() {
                l.push(format!("{}{}", if q.multi { "         " } else { "     " }, o.1));
            }
        }
        let b = if q.multi { if self.checks[self.at].contains(&n) { "[✔] " } else { "[ ] " } } else { "" };
        let empty = if q.multi { "Type something" } else { "Type something." };
        l.push(format!("{} {}. {}{}", mark(n), n + 1, b, self.typed[self.at].clone().unwrap_or(empty.into())));
        if q.multi {
            l.push(format!("{}    Submit", mark(n + 1)));
        }
        l.push("─".repeat(60));
        l.push(format!("  {}. Chat about this", n + 2));
        l.push("".into());
        let nav = if self.qs.len() > 1 { "Tab/Arrow keys to navigate" } else { "↑/↓ to navigate" };
        let vim = if self.focus >= n && (self.typing || self.typed[self.at].is_some()) { " · ctrl+g to edit in Vim" } else { "" };
        l.push(format!("Enter to select · {nav}{vim} · Esc to cancel"));
        l.join("\n")
    }

    fn next(&mut self) {
        self.focus = 0;
        self.typing = false;
        if self.at + 1 < self.qs.len() {
            self.at += 1;
        } else if self.qs.len() > 1 || self.qs[self.at].multi {
            self.review = true;
        } else {
            self.done = true;
        }
    }

    fn input(&mut self, i: &Input) {
        match i {
            Input::Keys(ks) => {
                for k in ks {
                    self.key(k);
                }
            }
            Input::Bytes(t) => self.text(t),
            Input::Text(_) => {}
        }
    }

    fn text(&mut self, t: &str) {
        if self.review {
            if t == "1" {
                self.review = false;
                self.done = true;
            }
            return;
        }
        let multi = self.qs[self.at].multi;
        let n = self.qs[self.at].options.len();
        if self.typing && self.focus == n {
            let s = self.typed[self.at].get_or_insert_with(String::new);
            s.push_str(t);
            return;
        }
        let Ok(d) = t.parse::<usize>() else { return };
        if !(1..=n + 1).contains(&d) {
            return;
        }
        if multi {
            if !self.checks[self.at].remove(&(d - 1)) {
                self.checks[self.at].insert(d - 1);
            }
            if d == n + 1 {
                self.typing = true;
            }
            return;
        }
        if d == n + 1 {
            self.focus = n;
            self.typing = true;
            return;
        }
        self.single.insert(self.at, d - 1);
        self.next();
    }

    fn key(&mut self, k: &str) {
        if self.review || self.done {
            return;
        }
        let multi = self.qs[self.at].multi;
        let n = self.qs[self.at].options.len();
        match k {
            "Down" => self.focus = (self.focus + 1).min(if multi { n + 1 } else { n }),
            "Tab" => {
                if multi && self.focus == n && self.typed[self.at].is_some() {
                    self.focus = n + 1;
                } else {
                    self.next();
                }
            }
            "Enter" => {
                if multi {
                    if self.focus == n + 1 {
                        self.next();
                    }
                } else if self.focus == n && self.typed[self.at].is_some() {
                    self.next();
                } else if self.focus < n {
                    self.single.insert(self.at, self.focus);
                    self.next();
                }
            }
            "Left" => {
                if self.at > 0 {
                    self.at -= 1;
                    self.focus = 0;
                }
            }
            _ => {}
        }
    }
}

/// A hooked request (Swift: `ClaudeQuestionHook.record(...).request`: `QuestionAdapter.claude`, headers cleared).
fn hooked(id: &str, questions: Value) -> QuestionRequest {
    let mut q = claude(id, json!({ "questions": questions }).as_object().unwrap()).expect("questions");
    q.headers = vec![];
    q
}

/// Runs `answer_question` on a live `Sim` (Swift: `run` in checksClaudeHook).
fn run_sim(sim: Rc<RefCell<Sim>>, live: &Live, req: &QuestionRequest, a: &[QuestionAnswer], g: &KeyGate) -> (Vec<String>, Result<(), HerdrError>) {
    live.set(&sim.borrow().render());
    let s = sim.clone();
    live.on(move |i, l| {
        s.borrow_mut().input(i);
        l.set(&s.borrow().render());
    });
    let n = live.mark();
    let r = answer_question(live, "w1:p1", req, a, Some(g));
    live.off();
    (live.sent(n), r)
}

// MARK: ChecksQuestions.swift

/// checksAskUserQuestion: the parser parts (the `AskUserQuestion` input and its result).
#[test]
fn ask_user_question_parse() {
    let q1 = claude("toolu_q1", &o!({"questions": [{"question": "Which branch should the worktree start from?", "header": "Branch", "multiSelect": false,
            "options": [{"label": "integration (Recommended)", "description": "Has the latest merged work."}, {"label": "main", "description": "Stable."}]}]}),
    )
    .unwrap();
    let labels: Vec<&str> = q1.questions[0].options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["integration", "main"], "question: (Recommended) suffix removed and flagged");
    assert_eq!(q1.questions[0].options.iter().map(|o| o.recommended).collect::<Vec<_>>(), [true, false]);
    assert!(q1.questions[0].options[0].detail == "Has the latest merged work." && !q1.questions[0].multi_select, "question: detail, single select");
    let result = json!({"answers": {"Which branch should the worktree start from?": "integration (Recommended)"}});
    assert_eq!(
        claude_state(&q1, false, Some(&result), "User has answered your questions: ..."),
        QuestionState::Answered(Some(vec!["integration (Recommended)".into()])),
        "question: answers from toolUseResult"
    );
    let q2 = claude("toolu_q2", &o!({"questions": [{"question": "Which items to clean?", "header": "Cleanup", "multiSelect": true,
            "options": [{"label": "Old caches", "description": "Rebuildable."}, {"label": "Build output（建議）", "description": "Rebuildable."}, {"label": "Logs", "description": ""}]},
            {"question": "Report \"numbers\" after?", "header": "Report", "multiSelect": false, "options": [{"label": "Yes", "description": ""}, {"label": "No", "description": ""}]}]}),
    )
    .unwrap();
    assert!(
        q2.questions[0].multi_select && q2.questions[0].options[1].label == "Build output" && q2.questions[0].options[1].recommended,
        "question: multiSelect, （建議） suffix"
    );
    let t = r#"Your questions have been answered: "Which items to clean?"="Old caches, Build output（建議）", "Report \"numbers\" after?"="Yes". You can now continue with these answers in mind."#;
    assert_eq!(
        claude_state(&q2, false, None, t),
        QuestionState::Answered(Some(vec!["Old caches, Build output（建議）".into(), "Yes".into()])),
        "question: answers parsed from result text, quotes escaped"
    );
    let q3 = claude("toolu_q3", &o!({"questions": [{"question": "Proceed?", "header": "Go", "multiSelect": false, "options": [{"label": "Yes"}, {"label": "No"}]}]})).unwrap();
    assert_eq!(
        claude_state(&q3, true, Some(&json!("Error: The user doesn't want to proceed with this tool use.")), "rejected"),
        QuestionState::Declined,
        "question: error result is .declined"
    );
    assert!(claude("x", &o!({"questions": []})).is_none() && claude("x", &o!({"questions": [1]})).is_none(), "no questions: no request");
}

/// checksCodexQuestions: the parser parts.
#[test]
fn codex_questions_parse() {
    let q1 = codex(
        "request_user_input",
        "call_c1",
        r#"{"questions":[{"header":"Color","id":"color","question":"Pick a color?","options":[{"label":"Red (Recommended)","description":"Warm."},{"label":"Blue","description":"Cool."}]}]}"#,
    )
    .unwrap();
    assert!(
        q1.agent == "codex" && q1.delivery == Delivery::Picker && q1.questions[0].id == "color"
            && q1.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && q1.questions[0].options[0].recommended,
        "codex question: picker, id, (Recommended) flagged"
    );
    assert_eq!(codex_state(&q1, r#"{"answers":{"color":{"answers":["Blue"]}}}"#), Some(QuestionState::Answered(Some(vec!["Blue".into()]))), "codex question: answer by id");
    let q2 = codex(
        "request_user_input",
        "call_c2",
        r#"{"questions":[{"header":"Size","id":"size","question":"Which size?","options":[{"label":"Small","description":"S"},{"label":"Large","description":"L"}]},{"header":"Pet","id":"pet","question":"Cat or dog?","options":[{"label":"Cat","description":"c"},{"label":"Dog","description":"d"}]}]}"#,
    )
    .unwrap();
    assert_eq!(
        codex_state(&q2, r#"{"answers":{"size":{"answers":["Large"]},"pet":{"answers":["None of the above","user_note: 一隻魚 fish"]}}}"#),
        Some(QuestionState::Answered(Some(vec!["Large".into(), "一隻魚 fish".into()]))),
        "codex question: notes replace None of the above"
    );
    let q3 = codex("request_user_input", "call_c3", r#"{"questions":[{"header":"Go","id":"go","question":"Go on?","options":[{"label":"Yes","description":"y"}]}]}"#).unwrap();
    assert_eq!(codex_state(&q3, "request_user_input is unavailable in Default mode"), Some(QuestionState::Declined), "codex question: unavailable output is .declined");
    let q4 = codex("request_user_input_async", "call_c4", r#"{"questions":[{"title":"Which plan?","options":["Fast","Safe"]}]}"#).unwrap();
    assert!(
        q4.delivery == Delivery::Message && q4.questions[0].text == "Which plan?"
            && q4.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Fast", "Safe"],
        "codex question: async title and string options"
    );
    assert_eq!(codex_state(&q4, r#"{"accepted":true}"#), None, "codex question: async stays pending after {{accepted:true}}");
    assert!(codex("shell", "c", r#"{"questions":[{"title":"x"}]}"#).is_none(), "another tool: no question");
}

/// checksQuestionScreen
#[test]
fn question_screen() {
    let single = sw(r#"
        ❯ Ask me.
        ────────────────────
         ☐ Color
        Pick a color?
        ❯ 1. Red (Recommended)
             warm
          2. Blue
             cool
          3. Type something.
        ────────────────────
          4. Chat about this
        Enter to select · ↑/↓ to navigate · Esc to cancel
        "#);
    let (h, q) = claude_screen(&single).expect("single");
    assert!(
        h == ["Color"] && q.text == "Pick a color?"
            && q.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && q.options.iter().map(|o| o.recommended).collect::<Vec<_>>() == [true, false]
            && q.options.iter().map(|o| o.detail.as_str()).collect::<Vec<_>>() == ["warm", "cool"]
            && !q.multi_select,
        "screen question: single {q:?}"
    );
    let multi = sw(r#"
        ←  ☐ Size  ☒ Tops  ☐ Pay  ✔ Submit  →
        Which toppings would you like
        on top?
        ❯ 1. [✔] Cheese
             Melted
          2. [ ] Olive
          3. [ ] Type something
             Submit
        ────────────────────
          4. Chat about this
        "#);
    let (h, m) = claude_screen(&multi).expect("multi");
    assert!(
        h == ["Size", "Tops", "Pay"] && m.text == "Which toppings would you like on top?" && m.multi_select
            && m.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Cheese", "Olive"],
        "screen question: multiSelect, wrapped text, tab bar {m:?}"
    );
    assert!(
        claude_screen("⏺ User answered Claude's questions:\n  ⎿  · Pick a color? → Blue").is_none()
            && claude_screen("Do you want to proceed?\n❯ 1. Yes\n  2. No").is_none(),
        "screen question: none without a picker"
    );
}

/// checksQuestionKeys
#[test]
fn question_keys() {
    let opts2 = || vec![opt("A"), opt("B")];
    let single = request("t", "claude", vec![question("H", "Pick one?", opts2(), false)]);
    let multi = request("t", "claude", vec![question("H", "Pick some?", opts2(), true)]);
    let three = request(
        "t",
        "claude",
        vec![question("1", "First?", opts2(), false), question("2", "Second?", opts2(), true), question("3", "Third?", opts2(), false)],
    );
    assert_eq!(steps(&single, &[ans(&[1])]).ok(), Some(vec![text("2")]), "keys: claude single = digit");
    assert_eq!(steps(&single, &[other(&[], "a fish")]).ok(), Some(vec![text("3"), text("a fish"), key(&["Enter"])]), "keys: claude single Other");
    assert_eq!(
        steps(&multi, &[other(&[1, 0], "tart")]).ok(),
        Some(vec![text("1"), text("2"), text("3"), key(&["Down", "Down"]), text("tart"), key(&["Tab"]), key(&["Enter"]), wait("Review your answers"), text("1")]),
        "keys: claude multiSelect + Other"
    );
    assert_eq!(
        steps(&three, &[ans(&[0]), ans(&[1]), other(&[], "x")]).ok(),
        Some(vec![
            text("1"),
            wait("Second?"),
            text("2"),
            key(&["Tab"]),
            wait("Third?"),
            text("3"),
            text("x"),
            key(&["Enter"]),
            wait("Review your answers"),
            text("1")
        ]),
        "keys: claude three questions"
    );
    let mut cx = three.clone();
    cx.agent = "codex".into();
    cx.questions[1].multi_select = false;
    assert_eq!(
        steps(&cx, &[ans(&[0]), ans(&[1]), other(&[], "一隻魚")]).ok(),
        Some(vec![text("1"), wait("Second?"), text("2"), wait("Third?"), key(&["Down", "Down"]), key(&["Tab"]), text("一隻魚"), key(&["Enter"])]),
        "keys: codex digits, notes on None of the above"
    );
    let mut cxm = multi.clone();
    cxm.agent = "codex".into();
    let fails = |q: &QuestionRequest, a: &[QuestionAnswer]| steps(q, a).is_err();
    assert!(
        fails(&cxm, &[ans(&[0])])
            && fails(&single, &[ans(&[0, 1])])
            && fails(&single, &[ans(&[])])
            && fails(&single, &[ans(&[5])])
            && fails(&single, &[])
            && fails(&single, &[other(&[], "two\nlines")]),
        "keys: unsupported answers throw before sending"
    );
    let screen = "←  ☐ H  ✔ Submit  →\nPick one?\n❯ 1. A\n  2. B\n  3. Type something.\n  4. Chat about this\nEnter to select";
    let echoed = format!("❯ Ask: question 'Pick some?' and 'Pick one?'\n{screen}");
    assert!(
        !screen_shows(&multi, &echoed) && picker_region("claude", &echoed).is_some_and(|r| !r.contains("Pick some?")),
        "keys: only the picker region counts, not the echoed prompt"
    );
    let cx_screen = "› Ask 'Cat or dog?' then 'Which size?'\n  Question 2/2 (1 unanswered)\n  Which size?\n  › 1. S\n  tab to add notes | enter to submit all";
    assert!(
        picker_region("codex", cx_screen).is_some_and(|r| !r.contains("Cat or dog?"))
            && picker_region("codex", cx_screen).is_some_and(|r| r.contains("Which size?"))
            && picker_region("codex", "• Questions 1/1 answered\n  • Cat or dog?").is_none(),
        "keys: codex picker region"
    );
    assert!(
        screen_shows(&single, screen)
            && !screen_shows(&multi, screen)
            && !screen_shows(&single, "⏺ User answered Claude's questions:\n  ⎿  · Pick one? → B"),
        "keys: screen check needs picker and question"
    );
    let mut asy = single.clone();
    asy.agent = "codex".into();
    asy.delivery = Delivery::Message;
    assert!(
        message_text(&asy, &[ans(&[1])]) == "B" && message_text(&three, &[ans(&[0]), ans(&[0, 1]), other(&[], "x")]) == "First? A\nSecond? A, B\nThird? x",
        "keys: async message text"
    );
}

// MARK: ChecksGrokQuestion.swift

fn grok_two() -> QuestionRequest {
    grok("call-q-0", &o!({"questions":[{"question":"Which color?","options":[{"label":"Red (Recommended)","description":"Warm."},{"label":"Blue","description":"Cool."}]},
            {"question":"Which size?","options":[{"label":"Small","description":"Little."},{"label":"Large","description":"Big."}]}]}),
    )
    .unwrap()
}

/// The picker on screen (Grok Build 1.0.67, the shape captured live; text synthetic).
fn grok_screen(text: &str, rows: &[(&str, &str)], k: usize, n: usize, mark: &str) -> String {
    let pad = " ".repeat(60);
    let mut l: Vec<String> = vec![
        "  ❯ Pick for me. ".into(),
        "".into(),
        format!("  ◆ Ask {n} questions"),
        "".into(),
        "  ┃".into(),
        format!("  ┃  {text}"),
        "  ┃".into(),
        "  ┃".into(),
    ];
    for (i, r) in rows.iter().enumerate() {
        l.push(format!("  ┃  {} ({}) {}  {}{}█", i + 1, mark, r.0, r.1, pad));
    }
    l.push("  ┃  z (○) Type your answer here".into());
    l.push("  ┃".into());
    l.push(format!("  ┃  [{k}/{n}] ↑/↓ navigate · ←/→ question · y copy{pad}{}", if k == n { "Enter:submit" } else { "Enter:select" }));
    l.push("  ┃".into());
    l.push("".into());
    l.push("  Tab:next answer  │  Esc:scrollback  │  Shift+x:dismiss".into());
    l.join("\n")
}

/// checksGrokQuestion
#[test]
fn grok_question() {
    let q = grok_two();
    assert!(q.agent == "grok" && q.delivery == Delivery::Picker && q.state == QuestionState::Pending && q.tool_use_id == "call-q-0", "grok question: a pending picker card");
    assert!(
        q.questions.iter().map(|x| x.text.as_str()).collect::<Vec<_>>() == ["Which color?", "Which size?"]
            && q.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && q.questions[0].options[0].recommended
            && q.questions[0].options[0].detail == "Warm."
            && q.questions.iter().all(|x| !x.multi_select),
        "grok question: questions, labels, (Recommended)"
    );
    let raw = json!({"type":"AskUserQuestion","UserAnswered":{"answers":[{"question":"Which color?","options":["Red","Blue"],"picked":["Red"]},{"question":"Which size?","options":["Small","Large"],"picked":[],"typed":"huge"}]}});
    assert_eq!(grok_state(&q, false, Some(&raw), ""), QuestionState::Answered(Some(vec!["Red".into(), "huge".into()])), "grok question: picked and typed answers");
    let t = r#"User has answered your questions: "Which color?"="Blue", "Which size?"="Small"."#;
    assert_eq!(grok_state(&q, false, None, t), QuestionState::Answered(Some(vec!["Blue".into(), "Small".into()])), "grok question: answers from the result text");
    assert_eq!(grok_state(&q, true, None, "dismissed"), QuestionState::Declined, "grok question: a failed call is declined");
    let m = grok("call-m", &o!({"questions":[{"question":"Which fruits?","options":[{"label":"Apple"},{"label":"Pear"}],"multi_select":true}]})).unwrap();
    assert!(m.questions[0].multi_select, "grok question: multi_select");

    let color = [("Red", "Warm."), ("Blue", "Cool.")];
    let size = [("Small", "Little."), ("Large", "Big.")];
    let s1 = grok_screen("Which color?", &color, 1, 2, "○");
    let s2 = grok_screen("Which size?", &size, 2, 2, "○");
    assert!(picker_region("grok", &s1).is_some() && screen_shows(&q, &s1), "grok picker: on screen");
    assert!(
        shows_plain(&q, 0, &s1) && !shows_plain(&q, 1, &s1) && shows_plain(&q, 1, &s2) && !shows_plain(&q, 0, &s2),
        "grok picker: the question shown, exactly"
    );
    assert!(
        !shows_plain(&q, 0, &grok_screen("Which color?", &color, 1, 3, "○"))
            && !shows_plain(&q, 0, &grok_screen("Which color?", &[("Red", "Warm."), ("Green", "Cool.")], 1, 2, "○"))
            && !shows_plain(&q, 0, &grok_screen("Which color?", &[("Red", "Warm.")], 1, 2, "○"))
            && !shows_plain(&q, 0, &grok_screen("Which color?", &[("Red", "Hot."), ("Blue", "Cool.")], 1, 2, "○"))
            && !shows_plain(&q, 0, &grok_screen("Which colour?", &color, 1, 2, "○")),
        "grok picker: another count, option, detail or text: no match"
    );
    assert!(!shows_plain(&q, 0, &grok_screen("Which color?", &color, 1, 2, "●")), "grok picker: a row already picked: no key");
    let no_picker = "  ❯ Which color? Red or Blue\n\n  ◆ Thinking\n\n  Shift+Tab:mode  │  Ctrl+.:shortcuts";
    assert!(picker_region("grok", no_picker).is_none() && !screen_shows(&q, no_picker), "grok picker: the prompt repeating the question is not a picker");
    assert_eq!(steps(&q, &[ans(&[0]), ans(&[1])]).ok(), Some(vec![text("1"), wait("Which size?"), text("2")]), "grok picker: a digit per question; the last one submits");
    assert!(steps(&q, &[ans(&[0]), other(&[], "huge")]).is_err(), "grok picker: free text is not sent");
    assert!(steps(&m, &[ans(&[0])]).is_err(), "grok picker: multi_select is not sent");
}

// MARK: ChecksAgy.swift (checksAgyQuestions)

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

/// checksAgyQuestions
#[test]
fn agy_questions() {
    let q = agy("ag1-0", &o!({"questions":[{"is_multi_select":false,"options":["Red","Blue"],"question":"Which color do you prefer, red or blue?"}]}),
    )
    .unwrap();
    assert!(
        q.agent == "agy" && q.delivery == Delivery::Picker && q.state == QuestionState::Pending && q.questions.len() == 1
            && q.questions[0].text == "Which color do you prefer, red or blue?"
            && q.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]
            && !q.questions[0].multi_select,
        "agy: question parsed"
    );
    assert_eq!(agy_state(&q, "Created At: x\nCompleted At: y\nA1: Blue"), QuestionState::Answered(Some(vec!["Blue".into()])), "agy: answered");
    assert_eq!(agy_state(&q, "Created At: x\nCompleted At: y\nA1: User Skipped"), QuestionState::Declined, "agy: skipped is declined");
    assert!(screen_shows(&q, AGY_QUESTION_SCREEN) && !screen_shows(&q, AGY_PERM_SCREEN), "agy: picker on screen");
    assert!(shows_plain(&q, 0, AGY_QUESTION_SCREEN), "agy: the exact question on screen");
    let swapped = AGY_QUESTION_SCREEN.replace("1. Red", "1. Blau");
    let two_cursors = AGY_QUESTION_SCREEN.replace("  2. Blue", "> 2. Blue");
    let other_text_ = AGY_QUESTION_SCREEN.replace("Question 1/1: Which color", "Question 1/1: Which colour");
    let no_foot = AGY_QUESTION_SCREEN.replace(" · esc Skip", "");
    let no_write_in = AGY_QUESTION_SCREEN.replace("  3. Write-in...\n", "");
    assert!(
        ![swapped, two_cursors, other_text_, no_foot, no_write_in, AGY_PERM_SCREEN.to_string()].iter().any(|s| shows_plain(&q, 0, s)),
        "agy: another option, two cursors, another text, no picker footer, no write-in row: not this question"
    );
    assert_eq!(steps(&q, &[ans(&[1])]).ok(), Some(vec![text("2")]), "agy: digit picks and submits");
    assert!(steps(&q, &[other(&[], "Green")]).is_err(), "agy: write-in not proven: refused");
    let mut multi = q.clone();
    multi.questions[0].multi_select = true;
    assert!(steps(&multi, &[ans(&[0, 1])]).is_err(), "agy: multi-select not proven: refused");
    let mut two = q.clone();
    two.questions.push(q.questions[0].clone());
    two.questions[1].text = "And tea or coffee?".into();
    assert!(steps(&two, &[ans(&[0]), ans(&[1])]).is_err(), "agy: several questions not proven: refused");
}

// MARK: ChecksClaudeHook.swift

fn color_record() -> QuestionRequest {
    hooked(
        "toolu_01ABC",
        json!([{"question":"Pick a color?","header":"Color","multiSelect":false,"options":[
            {"label":"Red (Recommended)","description":"warm and a long description that should wrap on a narrow terminal screen for sure because it is long"},
            {"label":"Blue","description":"cool"}]}]),
    )
}

fn two_record() -> QuestionRequest {
    hooked(
        "toolu_two",
        json!([{"question":"Which size?","header":"Size","multiSelect":false,"options":[{"label":"Small","description":"small one"},{"label":"Large","description":"big one"}]},
            {"question":"Which toppings?","header":"Tops","multiSelect":true,"options":[{"label":"Cheese","description":"yellow"},{"label":"Ham","description":"pink"},{"label":"Egg","description":""}]}]),
    )
}

fn ex(q: &QuestionRequest, k: usize, s: &str, other: Option<&str>, focus: FieldFocus, checks: Option<&[usize]>) -> bool {
    let c: Option<BTreeSet<usize>> = checks.map(|c| c.iter().copied().collect());
    claude_exact(q, k, s, other, focus, c.as_ref())
}

/// checksClaudeHook: the exact rendering (Claude Code 2.1.296, live).
#[test]
fn claude_hook_exact() {
    let q = color_record();
    assert!(q.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"] && q.questions[0].options[0].recommended && q.unread() == 0);
    let single = sw(r#"
    ❯ Call the AskUserQuestion tool once with one question
    ────────────────────────────────────────────────────────────────
     ☐ Color

    Pick a color?

    ❯ 1. Red (Recommended)
         warm and a long description that should wrap on a narrow terminal screen for sure because it is long
      2. Blue
         cool
      3. Type something.
    ────────────────────────────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    "#);
    let e = |s: &str| ex(&q, 0, s, None, FieldFocus::Auto, None);
    let eo = |s: &str, o: &str| ex(&q, 0, s, Some(o), FieldFocus::Auto, None);
    assert!(e(&single), "hookB exact: the live single picker is the record's question");
    assert!(shows_plain(&q, 0, &single), "hookB shows: a hooked request uses the exact compare");
    assert!(e(&single.replace("screen for sure because it is long", "screen for sure\n     because it is long")), "hookB exact: a wrapped description still matches");
    let other_empty = single
        .replace("❯ 1. Red", "  1. Red")
        .replace("  3. Type something.", "❯ 3. Type something.")
        .replace("↑/↓ to navigate · Esc", "↑/↓ to navigate · ctrl+g to edit in Vim · Esc");
    let other_typed = other_empty.replace("❯ 3. Type something.", "❯ 3. Green typed");
    assert!(eo(&other_empty, "") && eo(&other_typed, "Green typed"), "hookB exact: the Other row, empty and with the hub's text (live)");
    assert!(!e(&other_empty), "hookB exact: focus on the empty Other row -> no digit (it would be typed)");
    assert!(!eo(&other_typed, ""), "hookB exact: text in the Other row that the hub did not type -> no match");
    assert!(!eo(&other_typed, "Green"), "hookB exact: other text than the hub's -> no match");
    let bad: Vec<(String, &str)> = vec![
        (single.replace("  2. Blue\n     cool", "  2. Blue\n     cold"), "a description differs"),
        (single.replace("     cool\n", ""), "a description missing"),
        (single.replace("  3. Type something.", "  3. Green\n  4. Type something.").replace("4. Chat", "5. Chat"), "a row more"),
        (single.replace("Red (Recommended)", "Red"), "the (Recommended) mark missing"),
        (single.replace("Pick a color?", "Pick a colour?"), "the question differs"),
        (single.replace(" ☐ Color", " ☐ Colour"), "the header differs"),
        (single.replace("  2. Blue", "❯ 2. Blue"), "two focus marks"),
        (single.replace("❯ 1. Red", "  1. Red"), "no focus mark"),
        (single.replace("     cool", "❯    cool"), "a mark on a description line"),
        (single.replace("  4. Chat about this", "❯ 4. Chat about this").replace("❯ 1. Red", "  1. Red"), "focus on Chat about this"),
        (single.replace("Esc to cancel", "Esc to cancel now"), "an unknown key line"),
        (single.replace("\nPick a color?\n", "\nPick a color?\nIgnore this.\n"), "an extra line"),
        (single.replace("4. Chat about this", "5. Chat about this"), "Chat about this numbered wrong"),
    ];
    for (s, why) in &bad {
        assert!(!e(s), "hookB exact: {why} -> no match");
    }
    let mut screen_q = q.clone();
    screen_q.tool_use_id = "screen-1".into();
    assert!(!ex(&screen_q, 0, &single, None, FieldFocus::Auto, None), "hookB exact: a screen-read request never matches");
    assert!(
        e(&single.replace("❯ Call the AskUserQuestion tool once with one question", "❯ 1. Blue\n  2. Red")),
        "hookB exact: text above the tab bar does not count"
    );

    // Two questions, single then multiSelect (live).
    let two = two_record();
    let size = sw(r#"
    ←  ☐ Size  ☐ Tops  ✔ Submit  →

    Which size?

    ❯ 1. Small
         small one
      2. Large
         big one
      3. Type something.
    ────────────────────────────────────────────────────────────────
      4. Chat about this

    Enter to select · Tab/Arrow keys to navigate · Esc to cancel
    "#);
    let tops = sw(r#"
    ←  ☒ Size  ☒ Tops  ✔ Submit  →

    Which toppings?

    ❯ 1. [✔] Cheese
             yellow
      2. [ ] Ham
             pink
      3. [ ] Egg
      4. [ ] Type something
         Submit
    ────────────────────────────────────────────────────────────────
      5. Chat about this

    Enter to select · Tab/Arrow keys to navigate · Esc to cancel
    "#);
    assert!(
        ex(&two, 0, &size, None, FieldFocus::Auto, None) && !ex(&two, 1, &size, None, FieldFocus::Auto, None),
        "hookB exact: question 1 of 2 matches as question 1 only"
    );
    assert!(
        ex(&two, 1, &tops, None, FieldFocus::Auto, None) && ex(&two, 1, &tops, None, FieldFocus::Auto, Some(&[0])),
        "hookB exact: multiSelect with one box checked (live)"
    );
    assert!(
        !ex(&two, 1, &tops, None, FieldFocus::Auto, Some(&[])) && !ex(&two, 1, &tops, None, FieldFocus::Auto, Some(&[0, 1])),
        "hookB exact: other [✔] boxes than expected -> no match"
    );
    let tops_other = tops
        .replace("❯ 1. [✔] Cheese", "  1. [✔] Cheese")
        .replace("  4. [ ] Type something", "❯ 4. [✔] olive")
        .replace("navigate · Esc", "navigate · ctrl+g to edit in Vim · Esc");
    let tops_submit = tops_other.replace("❯ 4. [✔] olive", "  4. [✔] olive").replace("     Submit", "❯    Submit");
    assert!(ex(&two, 1, &tops_other, Some("olive"), FieldFocus::Other, Some(&[0, 3])), "hookB exact: multiSelect Other with the hub's text, focus on it (live)");
    assert!(
        ex(&two, 1, &tops_submit, Some("olive"), FieldFocus::Submit, Some(&[0, 3]))
            && !ex(&two, 1, &tops_submit, Some("olive"), FieldFocus::Other, Some(&[0, 3])),
        "hookB exact: focus on Submit (live) only where Submit is expected"
    );
    assert!(!ex(&two, 1, &tops.replace("  3. [ ] Egg", "  3. [x] Egg"), None, FieldFocus::Auto, None), "hookB exact: an unknown box -> no match");
    let review_screen = sw(r#"
    ←  ☒ Size  ☒ Tops  ✔ Submit  →

    Review your answers

     ● Which size?
       → Large
     ● Which toppings?
       → Cheese, olive

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    "#);
    let answers = [ans(&[1]), other(&[0], "olive")];
    assert!(claude_review_exact(&two, &answers, &review_screen), "hookB review: the live review of the hub's answers");
    assert!(!claude_review_exact(&two, &[ans(&[0]), answers[1].clone()], &review_screen), "hookB review: other answers than the hub's -> no match");
    assert!(!claude_review_exact(&two, &answers, &review_screen.replace("  2. Cancel", "❯ 2. Cancel")), "hookB review: two marks -> no match");
    assert!(!claude_review_exact(&two, &answers, &review_screen.replace("1. Submit answers", "1. Cancel")), "hookB review: rows swapped -> no match");
    assert!(!shows_review(&two, &review_screen, None), "hookB review: a hooked request without the answers never matches");
}

/// checksClaudeHook: answering end to end against the live layout.
#[test]
fn claude_hook_answer() {
    let q = color_record();
    let two = two_record();
    let agent = tui_agent("claude", Some("sess-1"));
    let live = Live::new(tui_pane(agent.clone()), "");
    let g = gate(&agent, yes());
    let color_sim = || {
        Rc::new(RefCell::new(Sim::new(vec![simq(
            "Color",
            "Pick a color?",
            &[("Red (Recommended)", "warm and a long description that should wrap on a narrow terminal screen for sure because it is long"), ("Blue", "cool")],
            false,
        )])))
    };
    let s = color_sim();
    let (keys, err) = run_sim(s.clone(), &live, &q, &[ans(&[1])], &g);
    assert!(err.is_ok() && keys == ["2"] && s.borrow().done && s.borrow().single.get(&0) == Some(&1), "hookB answer: single pick -> '2' ({keys:?}, {err:?})");
    let s = color_sim();
    let (keys, err) = run_sim(s.clone(), &live, &q, &[other(&[], "Green typed!")], &g);
    assert!(keys.is_empty() && matches!(&err, Err(HerdrError::Question(QuestionFailure::Unsupported(m))) if m.contains("cannot type")), "hc cannot type '!': refused before any key, {keys:?} {err:?}");
    let (keys, err) = run_sim(s.clone(), &live, &q, &[other(&[], "Green typed")], &g);
    assert!(
        err.is_ok() && keys == ["3", "Green typed", "Enter"] && s.borrow().done && s.borrow().typed[0].as_deref() == Some("Green typed"),
        "hookB answer: single Other -> 3, text, Enter ({keys:?}, {err:?})"
    );
    let s = color_sim();
    s.borrow_mut().typed[0] = Some("mine".into());
    let (keys, err) = run_sim(s, &live, &q, &[other(&[], "Green typed")], &g);
    assert!(keys.is_empty() && err.is_err(), "hookB answer: text the user typed in the Other row -> nothing sent ({keys:?})");
    let two_sim = || {
        Rc::new(RefCell::new(Sim::new(vec![
            simq("Size", "Which size?", &[("Small", "small one"), ("Large", "big one")], false),
            simq("Tops", "Which toppings?", &[("Cheese", "yellow"), ("Ham", "pink"), ("Egg", "")], true),
        ])))
    };
    let answers = [ans(&[1]), other(&[0], "olive")];
    let s = two_sim();
    let (keys, err) = run_sim(s.clone(), &live, &two, &answers, &g);
    assert!(
        err.is_ok() && keys == ["2", "1", "4", "Down", "Down", "Down", "olive", "Tab", "Enter", "1"] && s.borrow().done,
        "hookB answer: 2 questions, multiSelect + Other, review submit ({keys:?}, {err:?})"
    );
    let s = two_sim();
    let (keys, err) = run_sim(s.clone(), &live, &two, &[ans(&[1]), ans(&[0, 2])], &g);
    assert!(
        err.is_ok() && keys == ["2", "1", "3", "Tab", "1"] && s.borrow().done && s.borrow().checks[1] == BTreeSet::from([0, 2]),
        "hookB answer: multiSelect picks, Tab, review ({keys:?})"
    );
    // The user checked a box on question 2 before: the hub's digits would toggle it off, so no key on question 2.
    let s = two_sim();
    s.borrow_mut().checks[1] = BTreeSet::from([0]);
    let (keys, err) = run_sim(s, &live, &two, &[ans(&[1]), ans(&[0])], &g);
    assert!(
        !keys.contains(&"1".to_string()) && !keys.contains(&"Tab".to_string()) && err.is_err(),
        "hookB answer: a box checked by someone else -> stops before toggling ({keys:?})"
    );
    // Another request on screen (same layout, other options): nothing sent.
    let s = Rc::new(RefCell::new(Sim::new(vec![simq("Color", "Pick a color?", &[("Red (Recommended)", "warm"), ("Blue", "cool")], false)])));
    let (keys, err) = run_sim(s, &live, &q, &[ans(&[1])], &g);
    assert!(keys.is_empty() && err.is_err(), "hookB answer: a picker that is not the record's exact rendering -> nothing sent ({keys:?})");
    // The picker changes mid-way: after question 1 another request opens; no key goes to it.
    let s = two_sim();
    let swap = Rc::new(RefCell::new(Sim::new(vec![
        simq("Size", "Which size?", &[("Small", "small one"), ("Large", "big one")], false),
        simq("Tops", "Which toppings?", &[("Cheese", "yellow"), ("Ham", "pink"), ("Bacon", "")], true),
    ])));
    swap.borrow_mut().at = 1;
    swap.borrow_mut().single.insert(0, 1);
    live.set(&s.borrow().render());
    let (s2, sw2) = (s.clone(), swap.clone());
    live.on(move |i, l| {
        s2.borrow_mut().input(i);
        let screen = if s2.borrow().at == 1 { sw2.borrow().render() } else { s2.borrow().render() };
        l.set(&screen);
    });
    let n0 = live.mark();
    let err = answer_question(&live, "w1:p1", &two, &answers, Some(&g));
    live.off();
    assert!(live.sent(n0) == ["2"] && err.is_err(), "hookB answer: another request after question 1 -> stops, nothing more sent ({:?})", live.sent(n0));
}

/// Issue #11 item 1: a gated answer (the sidecar, a paired device) on a busy machine. Each key's render comes after
/// 16 reads (2.4 s at 150 ms; the old wait gave up after 1.5 s): the gate waits for the exact screen and sends every key. The same agent with another
/// question on screen stops with `ScreenStop` (keys sent so far told); another agent is still `KeyGateStop`.
#[test]
fn claude_hook_slow_render() {
    let two = two_record();
    let agent = tui_agent("claude", Some("sess-1"));
    let live = Live::new(tui_pane(agent.clone()), "");
    let g = gate(&agent, yes());
    let two_sim = || {
        Rc::new(RefCell::new(Sim::new(vec![
            simq("Size", "Which size?", &[("Small", "small one"), ("Large", "big one")], false),
            simq("Tops", "Which toppings?", &[("Cheese", "yellow"), ("Ham", "pink"), ("Egg", "")], true),
        ])))
    };
    let s = two_sim();
    live.set(&s.borrow().render());
    let s2 = s.clone();
    live.on(move |i, l| {
        s2.borrow_mut().input(i);
        l.set("");
        l.set_later(16, &s2.borrow().render());
    });
    let n = live.mark();
    let err = answer_question(&live, "w1:p1", &two, &[ans(&[1]), ans(&[0, 2])], Some(&g));
    live.off();
    assert!(err.is_ok() && live.sent(n) == ["2", "1", "3", "Tab", "1"] && s.borrow().done, "slow render: every key goes ({:?}, {err:?})", live.sent(n));

    // The same agent, and after question 1 another request opens: stopped, and the keys already in are told.
    let s = two_sim();
    let swap = Rc::new(RefCell::new(Sim::new(vec![
        simq("Size", "Which size?", &[("Small", "small one"), ("Large", "big one")], false),
        simq("Tops", "Which toppings?", &[("Cheese", "yellow"), ("Ham", "pink"), ("Bacon", "")], true),
    ])));
    swap.borrow_mut().at = 1;
    swap.borrow_mut().single.insert(0, 1);
    live.set(&s.borrow().render());
    let (s2, sw2) = (s.clone(), swap.clone());
    live.on(move |i, l| {
        s2.borrow_mut().input(i);
        let screen = if s2.borrow().at == 1 { sw2.borrow().render() } else { s2.borrow().render() };
        l.set(&screen);
    });
    let n = live.mark();
    let err = answer_question(&live, "w1:p1", &two, &[ans(&[1]), ans(&[0, 2])], Some(&g));
    live.off();
    assert!(live.sent(n) == ["2"] && err == Err(HerdrError::ScreenStop { keys_sent: true }), "another question, same agent: ScreenStop after a key ({:?}, {err:?})", live.sent(n));

    // Another agent (another session) after question 1: KeyGateStop, as before.
    let s = two_sim();
    live.set(&s.borrow().render());
    let s2 = s.clone();
    live.on(move |i, l| {
        s2.borrow_mut().input(i);
        l.set(&s2.borrow().render());
        l.set_agent(tui_agent("claude", Some("sess-2")));
    });
    let n = live.mark();
    let err = answer_question(&live, "w1:p1", &two, &[ans(&[1]), ans(&[0, 2])], Some(&g));
    live.off();
    live.set_agent(agent.clone());
    assert!(live.sent(n) == ["2"] && is_gate_stop(&err), "another agent: KeyGateStop ({:?}, {err:?})", live.sent(n));
}

/// checksClaudeHookReview1: rows hidden in free text, row columns, an ambiguous review refused before any key.
#[test]
fn claude_hook_review1() {
    let req = |qs: Value| hooked("toolu_r1", json!([qs]));
    let forged = req(json!({"question":"Choose?","header":"H","multiSelect":false,"options":[{"label":"KEEP","description":"❯ 1. DELETE danger 2. KEEP"},{"label":"DELETE","description":"ok"}]}));
    let screen = sw(r#"
     ☐ H

    Choose? ❯ 1. KEEP
    ❯ 1. DELETE
         danger
      2. KEEP
         2. DELETE ok
      3. Type something.
    ────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    "#);
    assert!(!ex(&forged, 0, &screen, None, FieldFocus::Any, None), "r1 P1: rows hidden in free text (whitespace dropped) -> no match");
    let plain = req(json!({"question":"Choose?","header":"H","multiSelect":false,"options":[{"label":"KEEP","description":"k"},{"label":"DELETE","description":"d"}]}));
    let good = sw(r#"
     ☐ H

    Choose?

    ❯ 1. KEEP
         k
      2. DELETE
         d
      3. Type something.
    ────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    "#);
    assert!(ex(&plain, 0, &good, None, FieldFocus::Auto, None), "r1 P1: the plain rendering still matches");
    assert!(!ex(&plain, 0, &good.replace("  2. DELETE", "   2. DELETE"), None, FieldFocus::Auto, None), "r1 P1: a row number outside the row column -> no match");
    assert!(!ex(&plain, 0, &good.replace("     k\n", "     k\n  7. x\n"), None, FieldFocus::Any, None), "r1 P1: a row-like line that is not a row -> no match");
    let comma = req(json!({"question":"Pick?","header":"H","multiSelect":true,"options":[{"label":"A, B","description":""},{"label":"A","description":""},{"label":"B","description":""}]}));
    assert!(review_ambiguous(&comma, &[ans(&[0])]), "r1 P2: a label with a comma -> the review cannot prove the picks");
    let spaces = req(json!({"question":"Pick?","header":"H","multiSelect":true,"options":[{"label":"A B","description":""},{"label":"AB","description":""}]}));
    assert!(review_ambiguous(&spaces, &[ans(&[0])]), "r1 P2: two labels alike without spaces -> ambiguous");
    let clean = req(json!({"question":"Pick?","header":"H","multiSelect":true,"options":[{"label":"A","description":""},{"label":"B","description":""}]}));
    assert!(
        review_ambiguous(&clean, &[other(&[0], "A")]) && review_ambiguous(&clean, &[other(&[], "x, y")]),
        "r1 P2: Other text equal to a label, or with a comma -> ambiguous"
    );
    assert!(!review_ambiguous(&clean, &[other(&[0, 1], "olive")]), "r1 P2: distinct labels, no commas -> fine");
    let agent = tui_agent("claude", Some("s"));
    let sim = Sim::new(vec![simq("H", "Pick?", &[("A, B", ""), ("A", ""), ("B", "")], true)]);
    let live = Live::new(tui_pane(agent.clone()), &sim.render());
    let r = answer_question(&live, "w1:p1", &comma, &[ans(&[0])], Some(&gate(&agent, yes())));
    assert!(r.is_err() && live.mark() == 0, "r1 P2: an ambiguous review -> refused before any key");
}

// MARK: ChecksClaudeHookReview2.swift, ChecksClaudeHookReview3.swift

#[test]
fn claude_hook_review2() {
    let req = |id: &str, qs: Value| hooked(id, qs);
    let marks = req(
        "toolu_M",
        json!([{"question":"First?","header":"One","multiSelect":false,"options":[{"label":"A","description":""},{"label":"A●Second?→B","description":""}]},
            {"question":"Second?","header":"Two","multiSelect":false,"options":[{"label":"B●Second?→C","description":""},{"label":"C","description":""}]}]),
    );
    let want = [ans(&[0]), ans(&[0])];
    let review = sw(r#"
    ←  ☒ One  ☒ Two  ✔ Submit  →

    Review your answers

     ● First?
       → A●Second?→B
     ● Second?
       → C

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    "#);
    assert!(review_ambiguous(&marks, &want), "r2 P2: a label with the review marks ● or → -> ambiguous");
    assert!(!claude_review_exact(&marks, &want, &review), "r2 P2: the forged review does not match");
    let plain = req(
        "toolu_P",
        json!([{"question":"First?","header":"One","multiSelect":false,"options":[{"label":"A","description":""},{"label":"B","description":""}]},
            {"question":"Second?","header":"Two","multiSelect":false,"options":[{"label":"C","description":""},{"label":"D","description":""}]}]),
    );
    let plain_want = [ans(&[0]), ans(&[1])];
    let plain_review = review.replace("A●Second?→B", "A").replace("→ C", "→ D");
    assert!(claude_review_exact(&plain, &plain_want, &plain_review), "r2 P2: the plain review still matches");
    assert!(
        !claude_review_exact(&plain, &plain_want, &plain_review.replace("→ A\n ● Second?", "→ A ● Second?")),
        "r2 P2: a review mark not at a line start -> no match"
    );
    let del = req("toolu_D", json!([{"question":"Do it?","header":"Act","multiSelect":false,"options":[{"label":"Delete nothing","description":""},{"label":"Keep","description":"k"}]}]));
    let del_screen = Sim::new(vec![simq("Act", "Do it?", &[("Delete", "nothing"), ("Keep", "k")], false)]).render();
    assert!(!ex(&del, 0, &del_screen, None, FieldFocus::Any, None), "r2 P2: label and description moved across their boundary -> no match");
    let del_true = Sim::new(vec![simq("Act", "Do it?", &[("Delete nothing", ""), ("Keep", "k")], false)]).render();
    assert!(ex(&del, 0, &del_true, None, FieldFocus::Any, None), "r2 P2: the true rendering still matches");
    let rev = req("toolu_V", json!([{"question":"Do it?","header":"Act","multiSelect":false,"options":[{"label":"Delete","description":"nothing"},{"label":"Keep","description":"k"}]}]));
    assert!(!ex(&rev, 0, &del_true, None, FieldFocus::Any, None), "r2 P2: the other way round -> no match");
    // P3: an ambiguous review with the picker on question 2: refused before the Left that goes back to question 1.
    let amb = req(
        "toolu_Q",
        json!([{"question":"First?","header":"One","multiSelect":false,"options":[{"label":"A B","description":""},{"label":"AB","description":""}]},
            {"question":"Second?","header":"Two","multiSelect":false,"options":[{"label":"C","description":""},{"label":"D","description":""}]}]),
    );
    let mut sim = Sim::new(vec![simq("One", "First?", &[("A B", ""), ("AB", "")], false), simq("Two", "Second?", &[("C", ""), ("D", "")], false)]);
    sim.at = 1;
    let agent = tui_agent("claude", Some("sess-2"));
    let live = Live::new(tui_pane(agent.clone()), &sim.render());
    let r = answer_question(&live, "w1:p1", &amb, &[ans(&[0]), ans(&[0])], Some(&gate(&agent, yes())));
    assert!(r.is_err() && live.mark() == 0, "r2 P3: ambiguous review, picker on question 2 -> refused before any key (no Left)");
}

// A single-choice question's review line shows ONE answer (a label or the Other text), never picks joined by
// commas, so a comma in its labels or Other text cannot make two answers look alike there. ("Yes, apply it" /
// "No / not yet" is the usual yes-no question; refusing it sent every such card to the terminal.)
#[test]
fn claude_review_comma_single_choice() {
    let q = hooked(
        "toolu_C",
        json!([{"question":"Remind you?","header":"Nudge","multiSelect":false,"options":[{"label":"Yes, remind me Dec 28","description":"a"},{"label":"No / not yet","description":"b"}]},
            {"question":"Second?","header":"Two","multiSelect":false,"options":[{"label":"C","description":""},{"label":"D","description":""}]}]),
    );
    let want = [ans(&[0]), ans(&[1])];
    assert!(!review_ambiguous(&q, &want), "single choice: a comma in a label is not ambiguous");
    assert!(!review_ambiguous(&q, &[other(&[], "Later, in January"), ans(&[1])]), "single choice: Other text with a comma is not ambiguous");
    assert!(review_ambiguous(&q, &[other(&[], "No / not yet"), ans(&[1])]), "single choice: Other text alike a label is still ambiguous");
    let review = sw(r#"
    ←  ☒ Nudge  ☒ Two  ✔ Submit  →

    Review your answers

     ● Remind you?
       → Yes, remind me Dec 28
     ● Second?
       → D

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    "#);
    assert!(claude_review_exact(&q, &want, &review), "single choice: the review of a comma label matches");
    assert!(!claude_review_exact(&q, &[ans(&[1]), ans(&[1])], &review), "single choice: another pick -> no match");
    assert!(!claude_review_exact(&q, &want, &review.replace("→ Yes, remind me Dec 28", "→ No / not yet")), "single choice: another answer shown -> no match");
    let own = review.replace("→ Yes, remind me Dec 28", "→ Later, in January");
    assert!(claude_review_exact(&q, &[other(&[], "Later, in January"), ans(&[1])], &own), "single choice: Other text with a comma matches");
    // Multi-choice keeps the rule: its picks ARE joined by commas.
    let multi = hooked(
        "toolu_M2",
        json!([{"question":"Which?","header":"Many","multiSelect":true,"options":[{"label":"Yes, remind me","description":""},{"label":"Yes","description":""},{"label":"remind me","description":""}]}]),
    );
    assert!(review_ambiguous(&multi, &[ans(&[0])]), "multi choice: a comma in a label is still ambiguous");
    assert!(review_ambiguous(&multi, &[other(&[1], "x, y")]), "multi choice: Other text with a comma is still ambiguous");
}

#[test]
fn claude_hook_review3() {
    let q = hooked(
        "toolu_r3",
        json!([{"question":"Deploy A B?","header":"Spaced","multiSelect":false,"options":[{"label":"Yes","description":"go"},{"label":"No","description":"stop"}]},
            {"question":"Deploy AB?","header":"Joined","multiSelect":false,"options":[{"label":"Yes","description":"go"},{"label":"No","description":"stop"}]}]),
    );
    let mut sim = Sim::new(vec![
        simq("Spaced", "Deploy A B?", &[("Yes", "go"), ("No", "stop")], false),
        simq("Joined", "Deploy AB?", &[("Yes", "go"), ("No", "stop")], false),
    ]);
    sim.at = 1;
    assert!(!shows_plain(&q, 0, &sim.render()), "r3 P2: question 2 on screen does not pass as question 1 when they look alike");
    let at1 = sim.render();
    sim.at = 0;
    assert!(!shows_plain(&q, 0, &sim.render()), "r3 P2: two questions alike -> neither is proven on screen");
    let agent = tui_agent("claude", Some("s3"));
    let live = Live::new(tui_pane(agent.clone()), &sim.render());
    let r = answer_question(&live, "w1:p1", &q, &[ans(&[0]), ans(&[1])], Some(&gate(&agent, yes())));
    assert!(r.is_err() && live.mark() == 0, "r3 P2: questions alike -> refused before any key");
    // The picker on question 2 (Swift's sim is left there by the first check): no Left either.
    live.set(&at1);
    let r = answer_question(&live, "w1:p1", &q, &[ans(&[0]), ans(&[1])], Some(&gate(&agent, yes())));
    // The same agent, the screen never proves question 1 (issue #11 item 1: a screen stop, not another agent).
    assert!(r == Err(HerdrError::ScreenStop { keys_sent: false }) && live.mark() == 0, "r3 P2: questions alike, picker on question 2 -> no Left ({r:?})");
    let ok = Sim::new(vec![simq("Spaced", "Deploy A?", &[("Yes", "go"), ("No", "stop")], false), simq("Joined", "Deploy B?", &[("Yes", "go"), ("No", "stop")], false)]);
    let mut q2 = q.clone();
    q2.questions[0].text = "Deploy A?".into();
    q2.questions[1].text = "Deploy B?".into();
    assert!(shows_plain(&q2, 0, &ok.render()), "r3 P2: distinct questions -> question 1 is proven");
}

// MARK: ChecksHubReview4.swift, ChecksHubReview5.swift (the question parts)

#[test]
fn hub_review4_answer_gated() {
    let seen = tui_agent("claude", Some("s1"));
    let multi = sw(r#"
        ←  ☒ Tops  ✔ Submit  →
        Which toppings?
        ❯ 1. [ ] Cheese
          2. [ ] Olive
          3. [ ] Type something
             Submit
        ────────────────────
          4. Chat about this
        "#);
    let live = Live::new(tui_pane(seen.clone()), &multi);
    let (h, shown) = claude_screen(&multi).expect("r4 answer: fixture parses");
    let q = screen_request(&h, vec![shown]);
    let a = [ans(&[0, 1])];
    let total = steps(&q, &a).map(|s| s.iter().filter(|x| x.sends_keys()).count()).unwrap_or(0);
    assert!(total >= 2, "r4 answer: the fixture needs {total} key steps");
    let n = live.mark();
    let r = answer_question(&live, "w1:p1", &q, &a, Some(&gate(&seen, budget(1))));
    assert!(is_gate_stop(&r) && live.after(n).len() == 1, "r4 answer: level gone after the first key -> stops, 1 key sent ({:?})", live.after(n));
    live.set_agent(tui_agent("claude", Some("s9")));
    let n = live.mark();
    let r = answer_question(&live, "w1:p1", &q, &a, Some(&gate(&seen, yes())));
    assert!(is_gate_stop(&r) && live.after(n).is_empty(), "r4 answer: another agent session -> nothing sent");
    live.set_agent(seen.clone());
    live.set(&sw(r#"
        ←  ☐ Color  ☐ Size  ✔ Submit  →
        Pick a color?
        ❯ 1. Red
          2. Blue
          3. Type something.
        ────────────────────
          4. Chat about this
        "#));
    let n = live.mark();
    let r = load_all_questions(&live, "w1:p1", Some(&gate(&seen, no())), None);
    assert!(is_gate_stop(&r) && live.after(n).is_empty(), "r4 load: level gone -> no navigation keys");
}

fn color_two_screens() -> (String, String, String, String) {
    let color = sw(r#"
    ←  ☐ Color  ☐ Size  ✔ Submit  →
    Pick a color?
    ❯ 1. Red
         Choose red
      2. Blue
         Choose blue
      3. Type something.
    ────────────────────
      4. Chat about this
    Enter to select · Tab/Arrow keys to navigate · Esc to cancel
    "#);
    let size = color
        .replace("Pick a color?", "Pick a size?")
        .replace("Red", "Small")
        .replace("Blue", "Large")
        .replace("Choose red", "Choose small")
        .replace("Choose blue", "Choose large");
    let lookalike = color.replace("1. Red", "1. Green");
    let review = sw(r#"
    ←  ☒ Color  ☒ Size  ✔ Submit  →

    Review your answers

     ● Pick a color?
       → Red
     ● Pick a size?
       → Small

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    "#);
    (color, size, lookalike, review)
}

#[test]
fn hub_review5_bind() {
    let (color, size, lookalike, review) = color_two_screens();
    let q = screen_request(
        &["Color".into(), "Size".into()],
        vec![question("Color", "Pick a color?", vec![opt("Red"), opt("Blue")], false), question("Size", "Pick a size?", vec![opt("Small"), opt("Large")], false)],
    );
    assert!(shows_plain(&q, 0, &color) && !shows_plain(&q, 1, &color), "r5 bind: question 1 shown, not question 2");
    assert!(!shows_plain(&q, 0, &lookalike), "r5 bind: same text, another option -> not this question");
    assert!(!shows_plain(&q, 0, &color.replace("☐ Color", "☐ Hue")), "r5 bind: other headers -> not this request");
    assert!(shows_any(&q, &size) && !shows_any(&q, &lookalike), "r5 bind: any question of the request, exactly");
    assert!(
        shows_review(&q, &review, None) && !shows_review(&q, &review.replace("Pick a size?", "Pick a shape?"), None) && !shows_review(&q, &color, None),
        "r5 bind: the review screen lists every question of the request"
    );
    let seen = tui_agent("claude", Some("s1"));
    let open = gate(&seen, yes());
    let one = screen_request(&["Color".into()], vec![q.questions[0].clone()]);
    let live = Live::new(tui_pane(seen.clone()), &lookalike.replace("☐ Color  ☐ Size  ✔ Submit", "☐ Color  ✔ Submit"));
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &one, &[ans(&[0])], Some(&open));
    assert!(live.after(n).is_empty(), "r5 bind: a picker with the same question text but other options gets no key");
    // (3) One key per call for a paired device: the committing key is its own gated call.
    let multi = sw(r#"
    ←  ☐ Tops  ✔ Submit  →
    Which toppings?
    ❯ 1. [ ] Cheese
      2. [ ] Olive
      3. [ ] Type something
         Submit
    ────────────────────
      4. Chat about this
    "#);
    live.set(&multi);
    let (h, shown) = claude_screen(&multi).expect("r5 batch: fixture parses");
    let mq = screen_request(&h, vec![shown]);
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &mq, &[other(&[0], "ham")], Some(&open));
    let named: Vec<Vec<String>> = live.after(n).into_iter().filter_map(|i| if let Input::Keys(k) = i { Some(k) } else { None }).collect();
    assert!(!named.is_empty() && named.iter().all(|k| k.len() == 1), "r5 batch: every named key its own gated call ({named:?})");
}

// MARK: ChecksHubReview6.swift

fn codex_fixtures() -> (QuestionRequest, String, String, String) {
    let codex_s = sw(r#"
    › Ask me two questions
      Question 1/2 (2 unanswered)
      Pick a color?

      › 1. Red                r
        2. Blue               b
        3. None of the above  Optionally, add details in notes (tab)

      tab to add notes | enter to submit answer | ←/→ to navigate questions | esc to interrupt
    "#);
    let notes = sw(r#"
      Question 1/2 (2 unanswered)
      Pick a color?

        1. Red                r
        2. Blue               b
      › 3. None of the above  Optionally, add details in notes (tab)

      › fish note

      tab or esc to clear notes | enter to submit answer
    "#);
    let mut cq = request(
        "call_1",
        "codex",
        vec![question("Color", "Pick a color?", vec![optd("Red", "r"), optd("Blue", "b")], false), question("Size", "Pick a size?", vec![opt("Small"), opt("Large")], false)],
    );
    cq.state = QuestionState::Pending;
    let empty_notes = notes.replace("› fish note", "› Add notes");
    (cq, codex_s, notes, empty_notes)
}

#[test]
fn hub_review6_codex() {
    let (cq, codex_s, notes, empty_notes) = codex_fixtures();
    let seen = tui_agent("codex", Some("s1"));
    let open = gate(&seen, yes());
    let swapped = codex_s.replace("1. Red                r", "1. Blue               b").replace("2. Blue               b", "2. Red                r");
    let added = codex_s.replace("3. None of the above", "3. Green              g\n    4. None of the above");
    assert!(shows_plain(&cq, 0, &codex_s), "r6 codex: the captured picker matches");
    assert!(!shows_plain(&cq, 0, &notes) && !shows_plain(&cq, 0, &empty_notes), "r6 codex: notes open (typed or empty) -> not a pick screen");
    let sh = |s: &str, o: &str| shows(&cq, 0, s, Some(o), FieldFocus::Auto, None, None);
    assert!(sh(&empty_notes, "") && sh(&notes, "fish note") && !sh(&notes, "cat"), "r6 codex: the notes the hub opened or typed match");
    let live = Live::new(tui_pane(seen.clone()), &notes);
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &cq, &[ans(&[0]), ans(&[0])], Some(&open));
    assert!(live.after(n).is_empty(), "r6 codex: notes open -> a pick gets no key");
    live.set(&codex_s);
    let on_none = codex_s.replace("  › 1. Red", "    1. Red").replace("    3. None", "  › 3. None");
    let (en, nt) = (empty_notes.clone(), notes.replace("fish note", "cat"));
    live.on(move |i, l| match i {
        Input::Keys(k) if k == &["Down"] => l.set(&on_none),
        Input::Keys(k) if k == &["Tab"] => l.set(&en),
        Input::Bytes(_) | Input::Text(_) if l.get() == en => l.set(&nt),
        _ => {}
    });
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &cq, &[other(&[], "cat"), ans(&[0])], Some(&open));
    let sent = live.sent_or_text(n);
    assert!(sent.ends_with(&["Tab".to_string(), "text".to_string(), "Enter".to_string()]), "r6 codex: notes answer goes Tab, text, Enter through the gate ({sent:?})");
    live.off();
    assert!(!shows_plain(&cq, 0, &swapped), "r6 codex: options in another order -> not this question");
    assert!(!shows_plain(&cq, 0, &added), "r6 codex: a new option -> not this question");
    let mut cm = cq.clone();
    cm.questions[0].multi_select = true;
    assert!(!shows_plain(&cm, 0, &codex_s), "r6 codex: another mode (multiSelect) -> not this question");
    live.set(&swapped);
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &cq, &[ans(&[0]), ans(&[0])], Some(&open));
    assert!(live.after(n).is_empty(), "r6 codex: swapped options get no key");
    // Review 7: Codex marks a recommended option on screen.
    let recommended = codex_s.replace("1. Red                r", "1. Red (Recommended)  r");
    let mut cr = cq.clone();
    cr.questions[0].options[0].recommended = true;
    assert!(
        shows_plain(&cr, 0, &recommended) && !shows_plain(&cr, 0, &recommended.replace("Red (Recommended)", "Rose (Recommended)")),
        "r7 codex: a recommended option matches with its mark"
    );
    // Review 8: the key line is the last line, not any text holding its words.
    let footer_note = notes.replace("› fish note", "› enter to submit");
    assert!(!shows_plain(&cq, 0, &footer_note) && sh(&footer_note, "enter to submit"), "r8 codex: notes holding 'enter to submit' are notes, not the key line");
    let refused = |q: &QuestionRequest, o: &str| steps(q, &q.questions.iter().map(|_| other(&[], o)).collect::<Vec<_>>()).is_err();
    let one = screen_request(&["Color".into()], vec![question("Color", "Pick a color?", vec![opt("Red"), opt("Blue")], false)]);
    assert!(
        refused(&cq, "Add notes") && refused(&one, "Type something.") && refused(&one, "Type something") && !refused(&one, "fish"),
        "r9 both: an Other text equal to the field's placeholder is refused before any key"
    );
}

fn claude_r6() -> String {
    sw(r#"
    ←  ☐ Color  ✔ Submit  →
    Pick a color?
    ❯ 1. Red
         Choose Red
      2. Blue
         Choose Blue
      3. Type something.
    ────────────────────
      4. Chat about this
    Enter to select · ↑/↓ to navigate · Esc to cancel
    "#)
}

fn labels(s: &str) -> Option<Vec<String>> {
    claude_screen(s).map(|(_, q)| q.options.into_iter().map(|o| o.label).collect())
}

#[test]
fn hub_review6_claude() {
    let claude_s = claude_r6();
    let typed = claude_s.replace("❯ 1. Red", "  1. Red").replace("  3. Type something.", "❯ 3. Green typed");
    let extra = claude_s.replace("3. Type something.", "3. Green\n  4. Type something.");
    let one = screen_request(&["Color".into()], vec![question("Color", "Pick a color?", vec![opt("Red"), opt("Blue")], false)]);
    let sh = |q: &QuestionRequest, s: &str, o: Option<&str>, f: FieldFocus| shows(q, 0, s, o, f, None, None);
    assert!(shows_plain(&one, 0, &claude_s), "r6 claude: the picker matches");
    assert!(!shows_plain(&one, 0, &extra), "r6 claude: a new trailing option -> not this question");
    assert!(!shows_plain(&one, 0, &typed) && sh(&one, &typed, Some("Green typed"), FieldFocus::Auto), "r6 claude: the typed row only with our own text");
    assert_eq!(labels(&typed), Some(vec!["Red".into(), "Blue".into()]), "r6 claude: the typed row is not read as an option");
    let multi_typed = "←  ☐ Tops  ✔ Submit  →\nWhich toppings?\n  1. [ ] Cheese\n  2. [ ] Olive\n❯ 3. [ ] ham please\n     Submit\n────\n  4. Chat about this";
    assert_eq!(labels(multi_typed), Some(vec!["Cheese".into(), "Olive".into()]), "r6 claude: the typed row (multi) is not an option");
    let seen = tui_agent("claude", Some("s1"));
    let g = gate(&seen, yes());
    let live = Live::new(tui_pane(seen.clone()), &typed);
    let (h, shown) = claude_screen(&typed).expect("r6 claude: the typed screen parses");
    let from_screen = screen_request(&h, vec![shown]);
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &from_screen, &[ans(&[0])], Some(&g));
    assert!(live.after(n).is_empty(), "r6 claude: text in the Other row -> a pick gets no key");
    let prefixed = claude_s.replace("  3. Type something.", "  3. Type something else\n  4. Type something.");
    assert_eq!(labels(&prefixed), Some(vec!["Red".into(), "Blue".into(), "Type something else".into()]), "r7 claude: an option starting with 'Type something' is an option");
    assert!(claude_screen(&claude_s.replace("────────────────────\n", "")).is_none(), "r7 claude: no line above 'Chat about this' -> not read");
    // Review 8
    let rec_typed = claude_s.replace("3. Type something.", "3. Type something. (Recommended)");
    assert!(
        claude_parse(&rec_typed).and_then(|p| p.typed).as_deref() == Some("Type something. (Recommended)") && !shows_plain(&one, 0, &rec_typed),
        "r8 claude: Other text with a (Recommended) mark is text, not the empty row"
    );
    assert!(
        !sh(&one, &claude_s.replace("3. Type something.", "3. fish (Recommended)"), Some("fish"), FieldFocus::Auto),
        "r8 claude: Other text 'fish (Recommended)' is not our 'fish'"
    );
    live.set(&claude_s);
    let count = Rc::new(RefCell::new(0));
    let (c, base) = (count.clone(), claude_s.clone());
    live.on(move |i, l| match i {
        Input::Bytes(_) => {
            *c.borrow_mut() += 1;
            let focused = base.replace("❯ 1. Red", "  1. Red");
            l.set(&focused.replace("  3. Type something.", if *c.borrow() == 1 { "❯ 3. Type something." } else { "❯ 3. 3" }));
        }
        Input::Keys(_) => l.set("❯ \n"),
        Input::Text(_) => {}
    });
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &one, &[other(&[], "3")], Some(&g));
    live.off();
    assert_eq!(live.sent_or_text(n), ["text", "text", "Enter"], "r8 claude: Other text equal to its digit -> digit, text, Enter");
    // Review 9
    let focused_other = claude_s.replace("❯ 1. Red", "  1. Red").replace("  3. Type something.", "❯ 3. Type something.");
    assert!(
        claude_parse(&focused_other).is_some_and(|p| p.other_focused) && claude_parse(&claude_s).is_some_and(|p| !p.other_focused),
        "r9 claude: focus on the Other row is read"
    );
    assert!(
        !shows_plain(&one, 0, &focused_other) && sh(&one, &focused_other, Some(""), FieldFocus::Auto),
        "r9 claude: Other row with focus -> no pick; matches only 'opened by us, empty'"
    );
    live.set(&focused_other);
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &one, &[ans(&[0])], Some(&g));
    assert!(live.after(n).is_empty(), "r9 claude: the user edits the Other row ('Type something.' typed) -> a pick gets no key");
    // Review 10
    assert!(
        !sh(&one, &typed.replace("❯ 3. Green typed", "  3. Green typed").replace("  1. Red", "❯ 1. Red"), Some("Green typed"), FieldFocus::Auto),
        "r10 claude: our text there but focus moved to an option -> no Enter"
    );
    let tops = sw(r#"
    ←  ☐ Tops  ✔ Submit  →
    Which toppings?
    ❯ 1. [ ] Cheese
             Add cheese
      2. [ ] Olive
             Add olive
      3. [ ] Type something
         Submit
    ────────────────────
      4. Chat about this
    Enter to select · ↑/↓ to navigate · Esc to cancel
    "#);
    let tops_checked = tops.replace("[ ] Type something", "[✔] Type something");
    let tops_on_row = tops_checked.replace("❯ 1. [ ] Cheese", "  1. [ ] Cheese").replace("  3. [✔] Type something", "❯ 3. [✔] Type something");
    let tops_ham = tops_on_row.replace("Type something", "ham");
    let tops_submit = tops_ham.replace("❯ 3. [✔] ham", "  3. [✔] ham").replace("     Submit", "❯    Submit");
    let mreq = screen_request(&["Tops".into()], vec![question("Tops", "Which toppings?", vec![opt("Cheese"), opt("Olive")], true)]);
    assert!(
        claude_parse(&tops_submit).is_some_and(|p| p.submit_focused) && claude_parse(&tops_ham).is_some_and(|p| !p.submit_focused),
        "r10 claude: focus on Submit is read"
    );
    for move_away in [false, true] {
        live.set(&tops);
        let downs = Rc::new(RefCell::new(0));
        let d = downs.clone();
        let (tc, tor, th, ts) = (tops_checked.clone(), tops_on_row.clone(), tops_ham.clone(), tops_submit.clone());
        live.on(move |i, l| match i {
            Input::Bytes(t) if t == "3" => l.set(&tc),
            Input::Bytes(t) if t == "ham" => {
                if move_away {
                    l.set(&th.replace("❯ 3. [✔] ham", "  3. [✔] ham").replace("  1. [ ] Cheese", "❯ 1. [ ] Cheese"));
                } else {
                    l.set(&th);
                }
            }
            Input::Keys(k) if k == &["Down"] => {
                *d.borrow_mut() += 1;
                if *d.borrow() == 2 {
                    l.set(&tor);
                }
            }
            Input::Keys(k) if k == &["Tab"] => l.set(&ts),
            Input::Keys(k) if k == &["Enter"] => l.set("❯ \n"),
            _ => {}
        });
        let n = live.mark();
        let _ = answer_question(&live, "w1:p1", &mreq, &[other(&[], "ham")], Some(&g));
        live.off();
        let sent = live.sent(n);
        if move_away {
            assert_eq!(sent, ["3", "Down", "Down", "ham"], "r10 claude multi: focus moved off the row after our text -> no Tab, no Enter");
        } else {
            assert_eq!(sent, ["3", "Down", "Down", "ham", "Tab", "Enter"], "r10 claude multi: Other flow with focus checked at each key");
        }
    }
    // Review 11
    let fake_submit = tops_ham
        .replace("❯ 3. [✔] ham", "  3. [✔] ham")
        .replace("  1. [ ] Cheese", "❯ 1. [ ] Cheese")
        .replace("Add cheese", "❯ Submit");
    assert!(
        claude_parse(&fake_submit).is_some_and(|p| !p.submit_focused) && !sh(&mreq, &fake_submit, Some("ham"), FieldFocus::Submit),
        "r11 claude: '❯ Submit' in an option's text is not the Submit control"
    );
    let two_marks = tops_submit.replace("  1. [ ] Cheese", "❯ 1. [ ] Cheese");
    assert!(
        !sh(&mreq, &two_marks, Some("ham"), FieldFocus::Submit) && !sh(&one, &typed.replace("  2. Blue", "❯ 2. Blue"), Some("Green typed"), FieldFocus::Auto),
        "r11 claude: two focus marks -> focus unknown, no key"
    );
    // Review 12, 13
    let violet = claude_s.replace("     Choose Red", "     2. Violet");
    assert!(labels(&violet) == Some(vec!["Red".into(), "Blue".into()]) && shows_plain(&one, 0, &violet), "r12 claude: '2. Violet' in an option's text is text, not a row");
    let gap = claude_s.replace("  2. Blue", "  5. Blue");
    assert!(claude_screen(&gap).is_none() && !shows_plain(&one, 0, &gap), "r12 claude: rows not numbered 1, 2, 3 -> no picker read");
    let listed = claude_s.replace("Pick a color?", "1. Pick a color?");
    assert_eq!(labels(&listed), Some(vec!["Red".into(), "Blue".into()]), "r12 claude: a numbered question line is not row 1");
    let indented = claude_s.replace("Pick a color?", "Pick a color?\n    1. Violet\n    2. Type something.");
    let violet_req = screen_request(&["Color".into()], vec![question("Color", "Pick a color?", vec![opt("Violet")], false)]);
    assert!(
        !shows_plain(&violet_req, 0, &indented) && !shows_plain(&one, 0, &indented),
        "r13 claude: a focus mark on a line that is not a row or Submit -> focus unknown, no key"
    );
    let mut om = one.clone();
    om.questions[0].multi_select = true;
    assert!(!shows_plain(&om, 0, &claude_s), "r6 claude: another mode (multiSelect) -> not this question");
}

#[test]
fn hub_review6_full_read() {
    let claude_s = claude_r6();
    let a = screen_request(&["Color".into(), "Size".into()], vec![question("", "Pick a color?", vec![opt("Red"), opt("Blue")], false)]);
    let b_full = screen_request(
        &["Color".into(), "Size".into()],
        vec![question("Color", "Pick a hue?", vec![opt("Red"), opt("Blue")], false), question("Size", "Pick a size?", vec![opt("Small"), opt("Large")], false)],
    );
    let a_full = screen_request(
        &["Color".into(), "Size".into()],
        vec![question("Color", "Pick a color?", vec![opt("Red"), opt("Blue")], false), question("Size", "Pick a size?", vec![opt("Small"), opt("Large")], false)],
    );
    assert!(same_request(&a_full, &a) && !same_request(&b_full, &a), "r6 full: same headers, another question -> another request");
    let seen = tui_agent("claude", Some("s1"));
    let live = Live::new(tui_pane(seen.clone()), &claude_s.replace("☐ Color  ✔ Submit", "☐ Color  ☐ Size  ✔ Submit").replace("Pick a color?", "Pick a hue?"));
    let r = load_all_questions(&live, "w1:p1", Some(&gate(&seen, yes())), Some(&a));
    assert!(matches!(r, Err(HerdrError::Question(QuestionFailure::NotOnScreen))) && live.mark() == 0, "r6 full: the picker now shows another request -> refused, no key");
}

// MARK: ChecksHubReview7.swift

#[test]
fn hub_review7_codex() {
    let logged = |qs: &str| codex("request_user_input", "call_c", &format!(r#"{{"questions":[{qs}]}}"#)).unwrap();
    let red = r#"{"label":"Red","description":"Choose Red"}"#;
    let blue = r#"{"label":"Blue","description":"Choose Blue"}"#;
    let q = logged(&format!(r#"{{"id":"color","header":"Color","question":"Pick a color?","options":[{red},{blue}]}}"#));
    let base = sw(r#"
    › Use request_user_input once with one question
      Question 1/1 (1 unanswered)
      Pick a color?

      › 1. Red                Choose Red
        2. Blue               Choose Blue
        3. None of the above  Optionally, add details in notes (tab)

      tab to add notes | enter to submit answer | esc to interrupt
    "#);
    let on_other = base.replace("  › 1. Red", "    1. Red").replace("    3. None", "  › 3. None");
    let empty = on_other.replace("tab to add notes | enter to submit answer | esc to interrupt", "› Add notes\n\n  tab or esc to clear notes | enter to submit answer");
    let typed = empty.replace("› Add notes", "  › ham and eggs");
    let sp = |q: &QuestionRequest, s: &str| shows_plain(q, 0, s);
    let sr = |q: &QuestionRequest, k: usize, s: &str, o: Option<&str>, row: Option<usize>| shows(q, k, s, o, FieldFocus::Auto, row, None);

    assert!(sp(&q, &base), "r14 codex: the live picker is the logged question");
    assert!(sp(&q, &base.replace(" (1 unanswered)", "")), "r14 codex: without the unanswered count");
    assert!(!sp(&q, &base.replace("Question 1/1", "Question 1/2")), "r14 codex: another question count -> no match");
    assert!(!sp(&q, &base.replace("Choose Red", "Choose Rot")), "r14 codex: a description differs -> no match");
    assert!(
        !sp(&q, &base.replace("1. Red                Choose Red", "1. Blue               Choose Blue").replace("2. Blue               Choose Blue", "2. Red                Choose Red")),
        "r14 codex: rows in another order -> no match"
    );
    assert!(!sp(&q, &base.replace("    3. None", "    3. Green\n    4. None")), "r14 codex: a new row -> no match");
    assert!(!sp(&q, &base.replace("Choose Red", "2. Violet")), "r14 codex: '2. Violet' on screen but not in the log -> no match");
    let violet = logged(&format!(r#"{{"id":"color","header":"Color","question":"Pick a color?","options":[{{"label":"Red","description":"2. Violet"}},{blue}]}}"#));
    assert!(sp(&violet, &base.replace("Choose Red", "2. Violet")), "r14 codex: '2. Violet' in the logged description -> match, the rows are the log's");
    let long = logged(&format!(
        r#"{{"id":"color","header":"Color","question":"Pick a color?","options":[{{"label":"Red","description":"Choose Red, the warm colour of the sunset"}},{blue}]}}"#
    ));
    assert!(sp(&long, &base.replace("Choose Red", "Choose Red, the warm\n                          colour of the sunset")), "r14 codex: a wrapped description still matches");
    assert!(!sp(&long, &base.replace("Choose Red", "Choose Red, the warm colour of…")), "r14 codex: a cut description -> no match");
    assert!(!sp(&q, &base.replace("    2. Blue", "  › 2. Blue")), "r14 codex: two focus marks -> no match");
    assert!(!sp(&q, &base.replace("  › 1. Red", "    1. Red")), "r14 codex: no focus mark -> no match");
    let marky = logged(&format!(r#"{{"id":"color","header":"Color","question":"Pick a color?","options":[{{"label":"Red","description":"› 3. None"}},{blue}]}}"#));
    let marky_screen = base.replace("Choose Red", "› 3. None");
    assert!(sp(&marky, &marky_screen) && !sr(&marky, 0, &marky_screen, None, Some(2)), "r14 codex: a mark in the logged text is text: the focus is on row 1, not on row 3");
    assert!(sr(&q, 0, &on_other, None, Some(2)) && !sr(&q, 0, &base, None, Some(2)), "r14 codex: the focus row is where the mark is");
    let listed = logged(&format!(r#"{{"id":"color","header":"Color","question":"Pick one:\n  1. Violet","options":[{red},{blue}]}}"#));
    assert!(sp(&listed, &base.replace("  Pick a color?", "  Pick one:\n    1. Violet")), "r14 codex: a numbered list in the logged question -> match");
    assert!(!sp(&q, &base.replace("  Pick a color?", "  Pick a color?\n  Question 1/1")), "r14 codex: a picker head inside the question text -> no match");
    let rec = logged(&format!(r#"{{"id":"color","header":"Color","question":"Pick a color?","options":[{{"label":"Red (Recommended)","description":"Choose Red"}},{blue}]}}"#));
    assert!(sp(&rec, &base.replace("1. Red               ", "1. Red (Recommended) ")) && !sp(&rec, &base), "r14 codex: '(Recommended)' as the log has it");
    assert!(sr(&q, 0, &empty, Some(""), Some(2)) && !sp(&q, &empty), "r14 codex: empty notes the hub opened");
    assert!(sr(&q, 0, &typed, Some("ham and eggs"), Some(2)) && !sr(&q, 0, &typed, Some("cat"), Some(2)), "r14 codex: notes with the hub's text only");
    assert!(
        !sr(
            &q,
            0,
            &typed.replace("tab or esc to clear notes | enter to submit answer", "tab to add notes | enter to submit answer | esc to interrupt"),
            Some("ham and eggs"),
            Some(2)
        ),
        "r14 codex: a notes line with the closed key line -> no match"
    );
    assert!(steps(&q, &[other(&[], "x\t1")]).is_err(), "r15 codex: a Tab in the Other text -> refused before any key");
    assert!(!sp(&q, &base.replace(" | esc to interrupt", "")), "r15 codex: a shortened key line -> no match");
    assert!(!sp(&q, &base.replace("esc to interrupt", "esc to interrupt | q to quit")), "r15 codex: a longer key line -> no match");
    assert!(sp(&q, &base.replace("submit answer | esc", "submit answer |  esc")), "r15 codex: key line spacing does not count");
    let pay = logged(
        r#"{"id":"food","header":"Food","question":"Cat or dog?","options":[{"label":"Cat","description":"Choose Cat."},{"label":"Dog","description":"Choose Dog."}]},{"id":"pay","header":"Pay","question":"How to pay?","options":[{"label":"Card","description":"Pay by Card."},{"label":"Cash","description":"Pay by Cash."}]}"#,
    );
    let second = sw(r#"
      Question 2/2 (1 unanswered)
      How to pay?
      › 1. Card               Pay by Card.
        2. Cash               Pay by Cash.
        3. None of the above  Optionally, add details in notes (tab)
      tab to add notes | enter to submit all | ←/→ to navigate questions | esc to interrupt
    "#);
    assert!(shows_plain(&pay, 1, &second) && !shows_plain(&pay, 0, &second), "r16 codex: the last question of a set ('enter to submit all') matches, as question 2 only");
    let second_notes = second
        .replace("  › 1. Card", "    1. Card")
        .replace("    3. None", "  › 3. None")
        .replace("tab to add notes | enter to submit all | ←/→ to navigate questions | esc to interrupt", "› ham\n  tab or esc to clear notes | enter to submit all");
    assert!(sr(&pay, 1, &second_notes, Some("ham"), Some(2)), "r16 codex: notes open on the last question of a set");
    let mut screen_q = q.clone();
    screen_q.tool_use_id = "screen-1".into();
    assert!(!sp(&screen_q, &base), "r14 codex: a screen-read request never matches for keys");

    // Tab (opens notes on the chosen row) waits for the focus on that row: Down keys that did not move it stop there.
    let seen = tui_agent("codex", Some("s1"));
    let g = gate(&seen, yes());
    let live = Live::new(tui_pane(seen.clone()), &base);
    let n = live.mark();
    let r = answer_question(&live, "w1:p1", &q, &[other(&[], "ham and eggs")], Some(&g));
    let sent = live.sent_or_text(n);
    assert!(
        !sent.contains(&"Tab".to_string()) && !sent.contains(&"text".to_string()) && r == Err(HerdrError::ScreenStop { keys_sent: true }),
        "r14 codex: focus not on the Other row -> no Tab, no text; a screen stop after the Down keys ({sent:?}, {r:?})"
    );
    // The whole notes answer, the focus moving as Codex moves it.
    live.set(&base);
    let downs = Rc::new(RefCell::new(0));
    let d = downs.clone();
    let mid = base.replace("  › 1. Red", "    1. Red").replace("    2. Blue", "  › 2. Blue");
    let (oo, em, ty) = (on_other.clone(), empty.clone(), typed.clone());
    live.on(move |i, l| match i {
        Input::Keys(k) => {
            if k == &["Down"] {
                *d.borrow_mut() += 1;
                l.set(if *d.borrow() >= 2 { &oo } else { &mid });
            }
            if k == &["Tab"] {
                l.set(&em);
            }
        }
        _ => {
            if l.get() == em {
                l.set(&ty);
            }
        }
    });
    let n = live.mark();
    let _ = answer_question(&live, "w1:p1", &q, &[other(&[], "ham and eggs")], Some(&g));
    live.off();
    assert_eq!(live.sent_or_text(n), ["Down", "Down", "Tab", "text", "Enter"], "r14 codex: notes answer: Down, Down, Tab, text, Enter");
}

// MARK: ChecksGrokRemote.swift (the question parts)

fn grok_picker(text: &str, rows: &[(&str, &str)], k: usize) -> String {
    let mut l: Vec<String> = vec!["  ┃".into(), format!("  ┃  {text}"), "  ┃".into()];
    for (i, r) in rows.iter().enumerate() {
        l.push(format!("  ┃  {} (○) {}  {}", i + 1, r.0, r.1));
    }
    l.push("  ┃  z (○) Type your answer here".into());
    l.push("  ┃".into());
    l.push(format!("  ┃  [{k}/2] ↑/↓ navigate · ←/→ question · y copy  {}", if k == 2 { "Enter:submit" } else { "Enter:select" }));
    l.push("  ┃".into());
    l.push("".into());
    l.push("  Tab:next answer  │  Esc:scrollback  │  Shift+x:dismiss".into());
    l.join("\n")
}

#[test]
fn grok_remote() {
    let q = request(
        "call-q-0",
        "grok",
        vec![
            question("", "Which color?", vec![optd("Red", "Warm."), optd("Blue", "Cool.")], false),
            question("", "Which size?", vec![optd("Small", "Little."), optd("Large", "Big.")], false),
        ],
    );
    let agent = tui_agent("grok", Some("g1"));
    let idle = "  ╭──────╮\n  │ ❯    │\n  ╰──────╯\n  Shift+Tab:mode  │  Ctrl+.:shortcuts";
    let q1 = grok_picker("Which color?", &[("Red", "Warm."), ("Blue", "Cool.")], 1);
    let q2 = grok_picker("Which size?", &[("Small", "Little."), ("Large", "Big.")], 2);
    let live = Live::new(tui_pane(agent.clone()), &q1);
    let two_steps = |l: &Live, q1: String, q2: String| {
        l.on(move |_, l| {
            let next = if l.get() == q1 { q2.clone() } else { idle.to_string() };
            l.set(&next);
        })
    };
    two_steps(&live, q1.clone(), q2.clone());
    let n = live.mark();
    let r = answer_question(&live, "w1:p1", &q, &[ans(&[1]), ans(&[0])], Some(&gate(&agent, yes())));
    assert!(r.is_ok() && live.sent(n) == ["2", "1"], "grok remote: the phone's answer, one digit per question {:?} {r:?}", live.sent(n));
    live.set(&q1);
    let q2c = q2.clone();
    live.on(move |_, l| {
        l.set_agent(tui_agent("grok", Some("g2")));
        l.set(&q2c);
    });
    let n = live.mark();
    let r = answer_question(&live, "w1:p1", &q, &[ans(&[1]), ans(&[0])], Some(&gate(&agent, yes())));
    assert!(r.is_err() && live.sent(n) == ["2"], "grok remote: another session in the pane: no next key {:?}", live.sent(n));
    live.off();
    // The Mac's own answer (no gate): the exact question before each key too, not only its first words.
    live.set_agent(agent.clone());
    live.set(&grok_picker("Which color?", &[("Green", "Fresh."), ("Blue", "Cool.")], 1));
    let n = live.mark();
    let r = answer_question(&live, "w1:p1", &q, &[ans(&[1]), ans(&[0])], None);
    assert!(r.is_err() && live.after(n).is_empty(), "grok mac: another question with the same words on screen: no key");
    live.set(&q1);
    two_steps(&live, q1.clone(), q2.clone());
    let n = live.mark();
    let r = answer_question(&live, "w1:p1", &q, &[ans(&[1]), ans(&[0])], None);
    assert!(r.is_ok() && live.sent(n) == ["2", "1"], "grok mac: the exact questions: one digit each {:?} {r:?}", live.sent(n));
    live.off();

    // One question: Grok shows no "[1/1]" count in its footer.
    let one = request("call-q-1", "grok", vec![question("", "Tea or coffee?", vec![optd("Tea", "Pick tea."), optd("Coffee", "Pick coffee.")], false)]);
    let single = |foot: &str| {
        let pad = " ".repeat(60);
        [
            "  ┃".to_string(),
            "  ┃  Tea or coffee?".into(),
            "  ┃".into(),
            "  ┃".into(),
            format!("  ┃  1 (○) Tea     Pick tea.{pad}█"),
            format!("  ┃  2 (○) Coffee  Pick coffee.{pad}█"),
            "  ┃  z (○) Type your answer here".into(),
            "  ┃".into(),
            format!("  ┃  {foot}"),
            "  ┃".into(),
            "  Tab:next answer  │  Esc:scrollback  │  Shift+x:dismiss".into(),
        ]
        .join("\n")
    };
    let s1 = single(&format!("↑/↓ navigate · y copy{}Enter:submit", " ".repeat(60)));
    assert!(grok_exact(&one, 0, &s1), "grok one question: no count in the footer: the exact question");
    assert!(grok_exact(&one, 0, &single("[1/1] ↑/↓ navigate · y copy  Enter:submit")), "grok one question: with a count too");
    assert!(!grok_exact(&one, 0, &single("↑/↓ navigate · y copy  Enter:select")), "grok one question: Enter:select refused");
    assert!(
        !grok_exact(&q, 0, &grok_picker("Which color?", &[("Red", "Warm."), ("Blue", "Cool.")], 1).replace("[1/2] ", "")),
        "grok two questions: no count refused"
    );
    for (name, g) in [("phone", Some(gate(&agent, yes()))), ("mac", None)] {
        live.set_agent(agent.clone());
        live.set(&s1);
        live.on(move |_, l| l.set(idle));
        let n = live.mark();
        let r = answer_question(&live, "w1:p1", &one, &[ans(&[1])], g.as_ref());
        live.off();
        assert!(r.is_ok() && live.sent(n) == ["2"], "grok one question ({name}): the digit sent {:?} {r:?}", live.sent(n));
    }
}

// MARK: ChecksHCCards.swift, ChecksModFeedCards.swift (the question parts, on hc panes)

const HC_PICKER: &str = "❯ Ask me.
────────────────────
 ☐ Color
Pick a color?
❯ 1. Red (Recommended)
     warm
  2. Blue
     cool
  3. Type something.
────────────────────
  4. Chat about this
Enter to select · ↑/↓ to navigate · Esc to cancel";

#[test]
fn hc_cards_question() {
    let live = Live::new(hc_pane("t1", "claude", AgentStatus::Blocked), HC_PICKER);
    let q = pending_question(&live, "hc:t1").unwrap();
    assert!(
        q.as_ref().is_some_and(|q| q.questions[0].text == "Pick a color?" && q.questions[0].options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>() == ["Red", "Blue"]),
        "cards: question read from terminal.read"
    );
    assert!(live.mark() == 0, "cards: reading sends no keys");
    live.on(|i, l| {
        if *i == Input::Bytes("2".into()) {
            l.set("⏺ User answered: Blue\n> ");
        }
    });
    answer_question(&live, "hc:t1", q.as_ref().unwrap(), &[ans(&[1])], None).unwrap();
    assert_eq!(live.hc_keys(0), [["2"]], "cards: answer = the picker keys, nothing else");
}

/// ChecksModFeedCards: the walk with a gate, as the Mac and the sidecar do it.
#[test]
fn modfeed_cards_question_gate() {
    let claude_pane = hc_pane("t1", "claude", AgentStatus::Blocked);
    let mut codex_pane = hc_pane("t1", "codex", AgentStatus::Blocked);
    codex_pane.agent.as_mut().unwrap().process = Some("5151@1".into());
    let picker = |i: usize, other: bool| -> String {
        let tabs = if other { "←  ☐ Lunch  ☐ Drink  ✔ Submit  →" } else { "←  ☐ Color  ☐ Size  ✔ Submit  →" };
        let q = match (other, i) {
            (true, 0) => "Pick a lunch?\n❯ 1. Soup\n  2. Rice",
            (true, _) => "Pick a drink?\n❯ 1. Tea\n  2. Milk",
            (false, 0) => "Pick a color?\n❯ 1. Red\n  2. Blue",
            (false, _) => "Which size?\n❯ 1. S\n  2. M",
        };
        format!("{tabs}\n{q}\n  3. Type something.\n────────────────────\n  4. Chat about this\nEnter to select · Tab/Arrow keys to navigate · Esc to cancel")
    };
    let mpane = claude_pane.clone();
    let live = Live::new(codex_pane.clone(), &picker(0, false));
    // One question on screen: read all sends no keys.
    {
        let one = Live::new(claude_pane.clone(), HC_PICKER);
        let g = KeyGate::new(mpane.agent.clone(), Some(yes()));
        let all = load_all_questions(&one, "hc:t1", Some(&g), None).unwrap();
        assert!(all.is_some_and(|q| q.questions.len() == 1) && one.mark() == 0, "card --all: one question, same card, no keys");
    }
    // The Mac card's Show all goes through the same gate: another agent, KeyGateStop, no keys.
    let r = load_all_questions(&live, "hc:t1", Some(&KeyGate::on_mac(mpane.agent.clone(), Some(yes()))), None);
    assert!(is_gate_stop(&r) && live.mark() == 0, "mac card load all: gated, another agent, stopped before any key");
    // The Mac: an agent with no identity is not refused; each key checks the screen instead.
    let mut bare_agent = mpane.agent.clone().unwrap();
    bare_agent.process = None;
    bare_agent.session_id = None;
    live.set_agent(claude_pane.agent.clone().unwrap());
    let at = Rc::new(RefCell::new(0usize));
    let a2 = at.clone();
    live.on(move |i, l| {
        if let Input::Keys(k) = i {
            if k == &["Left"] {
                let v = a2.borrow().saturating_sub(1);
                *a2.borrow_mut() = v;
            } else if k == &["Right"] {
                let v = (*a2.borrow() + 1).min(1);
                *a2.borrow_mut() = v;
            }
        }
        l.set(&picker(*a2.borrow(), false));
    });
    let one = pending_question(&live, &mpane.id).unwrap();
    let mac_all = load_all_questions(&live, &mpane.id, Some(&KeyGate::on_mac(Some(bare_agent.clone()), Some(yes()))), one.as_ref()).unwrap();
    assert!(mac_all.as_ref().is_some_and(|q| q.questions.len() == 2), "mac load all: no agent identity, still reads every question ({mac_all:?})");
    assert_eq!(live.hc_keys(0), [vec!["left"], vec!["right"], vec!["left"]], "mac load all: Left to question 1, Right, back with Left");
    // Another request comes up after the first key: the next key's screen check stops the walk.
    live.set(&picker(0, false));
    live.on(move |_, l| l.set(&picker(0, true)));
    let ka = live.mark();
    let r = load_all_questions(&live, &mpane.id, Some(&KeyGate::on_mac(Some(bare_agent.clone()), Some(yes()))), one.as_ref());
    assert!(r.is_err() && live.mark() - ka <= 1, "mac load all: another question on screen, stopped");
    // The Mac's answer keys: another agent (with an identity) in the pane, not one key.
    live.off();
    live.set(&picker(0, false));
    live.set_agent(codex_pane.agent.clone().unwrap());
    let kb = live.mark();
    let full = mac_all.unwrap();
    let r = answer_question(&live, &mpane.id, &full, &[ans(&[0]), ans(&[0])], Some(&KeyGate::on_mac(mpane.agent.clone(), Some(yes()))));
    assert!(is_gate_stop(&r) && live.mark() == kb, "mac answer: gated, another agent, no keys");
    // The same agent: the gate lets the walk start (the screen does not move, so it stops after that).
    live.set_agent(claude_pane.agent.clone().unwrap());
    let kc = live.mark();
    let _ = load_all_questions(&live, &mpane.id, Some(&KeyGate::new(mpane.agent.clone(), Some(yes()))), None);
    assert_eq!(live.hc_keys(kc).first(), Some(&vec!["left".to_string()]), "card --all: gated, same agent, the walk starts");
}

// MARK: messages, answered state, ids (no Swift check names them; Swift behaviour)

#[test]
fn message_delivery() {
    let mut q = request("c", "codex", vec![question("", "Which plan?", vec![opt("Fast"), opt("Safe")], false)]);
    q.delivery = Delivery::Message;
    let agent = tui_agent("codex", Some("s1"));
    let live = Live::new(tui_pane(agent.clone()), "› Ask Codex to do anything");
    answer_question(&live, "w1:p1", &q, &[ans(&[1])], Some(&gate(&agent, yes()))).unwrap();
    assert_eq!(live.after(0), [Input::Text("Safe".into())], "async: the answer as a message");
    let r = answer_question(&live, "w1:p1", &q, &[ans(&[])], None);
    assert_eq!(r, Err(HerdrError::Question(QuestionFailure::Unsupported("no answer".into()))));
    let r = answer_question(&live, "w1:p1", &q, &[ans(&[1])], Some(&gate(&agent, no())));
    assert!(is_gate_stop(&r) && live.mark() == 1, "async: the gate before the text");
    let mut done = q.clone();
    done.state = QuestionState::Declined;
    assert!(matches!(answer_question(&live, "w1:p1", &done, &[ans(&[1])], None), Err(HerdrError::Question(QuestionFailure::Unsupported(_)))));
    let mut unread = q.clone();
    unread.headers = vec!["A".into(), "B".into()];
    assert_eq!(
        answer_question(&live, "w1:p1", &unread, &[ans(&[1])], None),
        Err(HerdrError::Question(QuestionFailure::Unsupported("1 questions not read yet (loadAllQuestions)".into())))
    );
}

#[test]
fn screen_ids_and_text() {
    // djb2 over the UTF-8 of "Color|Pick a color?", as Swift's `screenRequest` computes it.
    let key = "Color|Pick a color?";
    let h = key.bytes().fold(5381u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
    let r = screen_request(&["Color".into()], vec![question("", "Pick a color?", vec![opt("Red")], false)]);
    assert_eq!(r.tool_use_id, format!("screen-{h:x}"));
    assert_eq!(r.questions[0].header, "Color");
    let two = screen_request(&["A".into(), "B".into()], vec![question("", "x?", vec![opt("1")], false)]);
    assert!(two.questions[0].header.is_empty() && two.unread() == 1, "headers go on the questions only when all are read");
    assert_eq!(flat("  a \n\n  b\tc  \n"), "a b c");
    assert_eq!(flat("   \nfoo"), " foo", "Swift keeps the space from a blank first line");
    assert_eq!(needle(&question("", "一二三四五六七八九十一二三四五六七八九十一二三四五六", vec![], false)), "一二三四五六七八九十一二三四五六七八九十一二三四");
    assert_eq!(strip_recommended(" Red (Recommended) "), ("Red".into(), true));
    assert_eq!(strip_recommended("Blue（推薦）"), ("Blue".into(), true));
    assert_eq!(answered(vec![None, None]), QuestionState::Answered(None));
    assert_eq!(answered(vec![Some("a".into()), None]), QuestionState::Answered(Some(vec!["a".into(), "".into()])));
    assert!(steps(&request("t", "claude", vec![question("", "Q?", vec![opt("A")], false)]), &[other(&[], "zero\u{200B}width")]).is_err(), "Cf is a control character");
}

/// Claude Code 2.1.322: a long question's text drawn as a block quote ("│ " before each of its lines, wrapped
/// lines too; seen live 2026-11-11 from about 90 characters on). The bar is drawing, not text.
#[test]
fn claude_hook_exact_quote_bar() {
    let long = "The throwaway \"Greenhouse badge test\" is back in greenhouse (last tab). Did it move to Orbit?";
    let q = hooked(
        "toolu_bar",
        json!([{"question": long,"header":"Final try","multiSelect":false,"options":[
            {"label":"Moved + notice","description":"It's in Orbit now and the notice showed."},
            {"label":"Didn't move","description":"It's still in greenhouse."}]}]),
    );
    let screen = |text: &str| sw(&format!(r#"
    ❯ the prompt
    ────────────────────────────────────────────────────────────────
     ☐ Final try

    {text}

    ❯ 1. Moved + notice
         It's in Orbit now and the notice showed.
      2. Didn't move
         It's still in greenhouse.
      3. Type something.
    ────────────────────────────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    "#));
    let e = |s: &str| ex(&q, 0, s, None, FieldFocus::Auto, None);
    let one = screen(&format!("│ {long}"));
    let wrapped = screen("│ The throwaway \"Greenhouse badge test\" is back in greenhouse\n    │ (last tab). Did it move to Orbit?");
    assert!(e(&screen(long)), "quote bar: the plain rendering still matches");
    assert!(e(&one), "quote bar: one barred line matches");
    assert!(e(&wrapped), "quote bar: every wrapped line barred matches");
    assert!(shows_plain(&q, 0, &one), "quote bar: shows() uses the same exact compare");
    let half = screen("│ The throwaway \"Greenhouse badge test\" is back in greenhouse\n    (last tab). Did it move to Orbit?");
    assert!(!e(&half), "quote bar: a bar on some lines only is text, not drawing -> no match");
    assert!(!e(&screen(&format!("│ {}", long.replace("Orbit?", "orbit?")))), "quote bar: other text behind the bar -> no match");
    assert!(!e(&one.replace("  2. Didn't move", "❯ 2. Didn't move")), "quote bar: two focus marks -> no match");
    let barred_q = hooked("toolu_bar2", json!([{"question": "│ starts with a bar?","header":"Final try","multiSelect":false,"options":[
        {"label":"Moved + notice","description":"It's in Orbit now and the notice showed."},{"label":"Didn't move","description":"It's still in greenhouse."}]}]));
    assert!(!ex(&barred_q, 0, &screen("│ │ starts with a bar?"), None, FieldFocus::Auto, None), "quote bar: a short question is never quoted: another text, refused");
    assert!(ex(&barred_q, 0, &screen("│ starts with a bar?"), None, FieldFocus::Auto, None), "quote bar: a short text that itself starts with │ is that text");
    let p = claude_parse(&wrapped).expect("parse");
    assert_eq!(p.question.text, long, "quote bar: the screen parse drops the bar too");
    // A narrow pane wraps inside the first 24 characters (the needle): the bar between them is drawing too.
    let narrow = screen("│ The throwaway\n    │ \"Greenhouse badge test\" is back in\n    │ greenhouse (last tab). Did it move to\n    │ Orbit?");
    assert!(e(&narrow), "quote bar: a narrow wrap matches");
    assert!(screen_shows(&q, &narrow), "quote bar: the entry check finds the needle across a barred wrap");
    let parsed = request("screen-1", "claude", vec![claude_parse(&narrow).expect("parse").question]);
    assert!(screen_shows(&parsed, &narrow), "quote bar: a screen-derived question's needle is found too");
    let review = sw(r#"
    ────────────────────────────────────────────────────────────────
    ←  ☒ Final try  ✔ Submit  →

    Review your answers

     │ ● The throwaway
     │ "Greenhouse badge test" is back in greenhouse (last tab). Did it move to Orbit?
       → Moved + notice

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    "#);
    let parsed_long = request("screen-2", "claude", vec![question("Final try", long, vec![], false)]);
    assert!(shows_review(&parsed_long, &review, None), "quote bar: a screen-derived review across a barred wrap");
    // A long question whose own text starts with "│", drawn without the quote bar (before 2.1.322) and with it.
    let own = format!("│ {long}");
    let own_q = hooked("toolu_bar3", json!([{"question": own,"header":"Final try","multiSelect":false,"options":[
        {"label":"Moved + notice","description":"It's in Orbit now and the notice showed."},{"label":"Didn't move","description":"It's still in greenhouse."}]}]));
    // "│ long" on screen is also `long` behind the bar (2.1.322): which question it is cannot be told, so a long
    // text's own "│" is refused (answer in the terminal), never a match for two requests.
    assert!(!ex(&own_q, 0, &screen(&own), None, FieldFocus::Auto, None), "quote bar: a long text's own │, not quoted: ambiguous, refused");
    assert!(!ex(&own_q, 0, &screen(&format!("│ {own}")), None, FieldFocus::Auto, None), "quote bar: a long text's own │, quoted: refused");
    assert!(ex(&q, 0, &screen(&own), None, FieldFocus::Auto, None), "quote bar: the same screen is the plain long question");
    assert!(!ex(&q, 0, &screen(&format!("│ │ {long}")), None, FieldFocus::Auto, None), "quote bar: one bar is drawing, the second is text");
}

/// Claude Code names the person's editor in the key line ("ctrl+g to edit in VS Code"): the same line as "Vim".
#[test]
fn claude_key_line_any_editor() {
    for l in [
        "Enter to select · ↑/↓ to navigate · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in VS Code · Esc to cancel",
        "  Enter to select  ·  ↑/↓ to navigate · ctrl+g to edit in Sublime Text · Esc to cancel",
    ] {
        assert!(claude_key_line(l), "key line: {l}");
    }
    for l in [
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel now",
        "Enter to select · ctrl+g to edit in Vim · Esc to cancel",
        "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in A · B · Esc to cancel",
    ] {
        assert!(!claude_key_line(l), "not a key line: {l}");
    }
    let q = color_record();
    let single = sw(r#"
    ────────────────────────────────────────────────────────────────
     ☐ Color

    Pick a color?

      1. Red (Recommended)
         warm and a long description that should wrap on a narrow terminal screen for sure because it is long
      2. Blue
         cool
    ❯ 3. Type something.
    ────────────────────────────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · ctrl+g to edit in VS Code · Esc to cancel
    "#);
    assert!(ex(&q, 0, &single, Some(""), FieldFocus::Auto, None), "key line: the Other row with VS Code named matches");
}

/// 2.1.322: the review screen draws a long question's "● text" with the block-quote bar too (live 2026-11-11).
#[test]
fn claude_review_quote_bar() {
    let q = hooked("toolu_rb", json!([
        {"question":"This is a deliberately long first question so the picker draws its text with a side bar, which size?","header":"Size","multiSelect":false,"options":[{"label":"Small","description":"s"},{"label":"Large","description":"l"}]},
        {"question":"Which toppings?","header":"Tops","multiSelect":true,"options":[{"label":"Cheese","description":"y"},{"label":"Ham","description":"p"},{"label":"Egg","description":"r"}]}]));
    let a = [ans(&[1]), other(&[0, 2], "Olives Please")];
    let review = |first: &str| sw(&format!(r#"
    ────────────────────────────────────────────────────────────────
    ←  ☒ Size  ☒ Tops  ✔ Submit  →

    Review your answers

    {first}
       → Large
     ● Which toppings?
       → Cheese, Egg, Olives Please

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    "#));
    assert!(claude_review_exact(&q, &a, &review(" ● This is a deliberately long first question so the picker draws its text with a side bar, which size?")), "review: plain");
    assert!(claude_review_exact(&q, &a, &review(" │ ● This is a deliberately long first question so the picker draws its text with a side bar, which size?")), "review: barred");
    assert!(claude_review_exact(&q, &a, &review(" │ ● This is a deliberately long first question so the picker\n     │ draws its text with a side bar, which size?")), "review: barred and wrapped");
    assert!(!claude_review_exact(&q, &a, &review(" │ ● This is a deliberately long first question so the picker draws its text with a side bar, which colour?")), "review: other text behind the bar");
    assert!(!claude_review_exact(&q, &[ans(&[0]), other(&[0, 2], "Olives Please")], &review(" │ ● This is a deliberately long first question so the picker draws its text with a side bar, which size?")), "review: another pick");
}

#[test]
fn claude_quote_bar_only_on_long_questions() {
    // Claude 2.1.322 draws the bar only for a long question (about 90 characters on). A short question shown as
    // "│ text" is that text itself: it is another question than "text", never the same one drawn with a bar.
    let opts = json!([{"label":"Yes","description":"y"},{"label":"No","description":"n"}]);
    let q = hooked("toolu_short", json!([{"question":"starts with a bar?","header":"Bar","multiSelect":false,"options":opts}]));
    let screen = |text: &str| sw(&format!(r#"
    ❯ the prompt
    ────────────────────────────────────────────────────────────────
     ☐ Bar

    {text}

    ❯ 1. Yes
         y
      2. No
         n
      3. Type something.
    ────────────────────────────────────────────────────────────────
      4. Chat about this

    Enter to select · ↑/↓ to navigate · Esc to cancel
    "#));
    assert!(ex(&q, 0, &screen("starts with a bar?"), None, FieldFocus::Auto, None), "short question, plain: matches");
    assert!(!ex(&q, 0, &screen("│ starts with a bar?"), None, FieldFocus::Auto, None), "short question behind a bar is another question's text: no match");
    let p = claude_parse(&screen("│ starts with a bar?")).expect("parse");
    assert_eq!(p.question.text, "│ starts with a bar?", "parse keeps a short question's own bar");
}

#[test]
fn claude_review_bar_only_in_question_text() {
    // Options A, B and "│B"; the review wraps the picks so "│B" starts a line. That bar is the label's own text.
    let q = hooked("toolu_rv", json!([
        {"question":"This is a deliberately long first question so the picker draws its text with a side bar, which size?","header":"Size","multiSelect":true,"options":[{"label":"A","description":"a"},{"label":"B","description":"b"},{"label":"│B","description":"c"}]}]));
    let review = sw(r#"
    ────────────────────────────────────────────────────────────────
    ←  ☒ Size  ✔ Submit  →

    Review your answers

     │ ● This is a deliberately long first question so the picker draws its text with a side bar, which size?
       → A,
       │B

    Ready to submit your answers?

    ❯ 1. Submit answers
      2. Cancel
    "#);
    assert!(claude_review_exact(&q, &[ans(&[0, 2])], &review), "picks A and │B: the wrapped │B is the label");
    assert!(!claude_review_exact(&q, &[ans(&[0, 1])], &review), "picks A and B: the screen shows │B, not B");
}
