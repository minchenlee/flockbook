//! Swift: `QuestionAdapter.swift` (plus `+ClaudeExact`, `+Agy`), ported 1:1 from `rust/herdrkit`'s `question_adapter`. Everything that differs per agent for question
//! cards: (a) parsing the tool call and its result, (b) the screen markers and key sequence that answer it.
//! Verified key sequences: docs/research/ask-user-question-keys.md.
//!
//! Swift's `extension PaneScreen` methods (`answerQuestion`, `pendingQuestion`, `loadAllQuestions`) are free
//! functions generic over the screen. Text is compared as Swift does: `trimmingCharacters(in: .whitespaces)` trims
//! spaces and tabs only (never newlines), `count`/`prefix` count grapheme clusters.

use crate::error::{HerdrError, QuestionFailure};
use crate::keygate::{fresh_agent, gated, passes, KeyGate};
use crate::model::*;
use crate::screen::PaneScreen;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// A lazily built regex: `re!(name, "pattern")` defines `fn name() -> &'static Regex`.
macro_rules! re {
    ($name:ident, $pat:expr) => {
        fn $name() -> &'static ::regex::Regex {
            static R: ::std::sync::OnceLock<::regex::Regex> = ::std::sync::OnceLock::new();
            R.get_or_init(|| ::regex::Regex::new($pat).expect("valid regex"))
        }
    };
}

mod agy;
mod claude_exact;

pub use agy::*;
pub use claude_exact::*;

// MARK: Text as Swift sees it

/// `CharacterSet.whitespaces`: Unicode Zs plus the tab.
pub(crate) fn is_ws(c: char) -> bool {
    matches!(c, '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}')
}

/// `CharacterSet.newlines` (and `Character.isNewline` for one scalar).
pub(crate) fn is_newline(c: char) -> bool {
    matches!(c, '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

/// `CharacterSet.whitespacesAndNewlines`.
pub(crate) fn is_ws_nl(c: char) -> bool {
    is_ws(c) || is_newline(c)
}

/// `CharacterSet.controlCharacters`: Unicode Cc and Cf.
pub(crate) fn is_control(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        0x00..=0x1F | 0x7F..=0x9F | 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x890..=0x891 | 0x8E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB
        | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}

/// `trimmingCharacters(in: .whitespaces)`.
pub(crate) fn trim_ws(s: &str) -> &str {
    s.trim_matches(is_ws)
}

/// `trimmingCharacters(in: .whitespacesAndNewlines)`.
pub(crate) fn trim_ws_nl(s: &str) -> &str {
    s.trim_matches(is_ws_nl)
}

/// Swift `String.count` (grapheme clusters).
pub(crate) fn gcount(s: &str) -> usize {
    s.graphemes(true).count()
}

/// Swift `dropFirst()` on a String: everything after the first grapheme cluster.
pub(crate) fn drop_first(s: &str) -> &str {
    match s.grapheme_indices(true).nth(1) {
        Some((i, _)) => &s[i..],
        None => "",
    }
}

/// Swift `Character.isWhitespace` for one grapheme cluster.
pub(crate) fn g_is_whitespace(g: &str) -> bool {
    if g == "\r\n" {
        return true;
    }
    let mut it = g.chars();
    matches!((it.next(), it.next()), (Some(c), None) if c.is_whitespace())
}

/// Swift `split(whereSeparator:)` over Characters: the pieces between separators, empty ones dropped.
pub(crate) fn split_graphemes<'a>(s: &'a str, sep: impl Fn(&str) -> bool) -> Vec<&'a str> {
    let mut out = vec![];
    let mut start = 0;
    for (i, g) in s.grapheme_indices(true) {
        if sep(g) {
            if i > start {
                out.push(&s[start..i]);
            }
            start = i + g.len();
        }
    }
    if s.len() > start {
        out.push(&s[start..]);
    }
    out
}

/// `split(whereSeparator: \.isWhitespace).joined(separator: " ")`: spacing aside.
pub(crate) fn spacing_aside(s: &str) -> String {
    split_graphemes(s, g_is_whitespace).join(" ")
}

/// Byte offset `i` starts a Character.
fn is_boundary(s: &str, i: usize) -> bool {
    if i == 0 || i == s.len() {
        return true;
    }
    if !s.is_char_boundary(i) {
        return false;
    }
    GraphemeCursor::new(i, s.len(), true).is_boundary(s, 0).unwrap_or(false)
}

/// Swift `hasSuffix`: compares whole Characters.
fn has_suffix(s: &str, p: &str) -> bool {
    s.ends_with(p) && is_boundary(s, s.len() - p.len())
}

/// Swift `Int(_:)` on a regex capture: ASCII digits (and a sign) only, nil on overflow.
pub(crate) fn int(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

fn unsupported(m: impl Into<String>) -> HerdrError {
    HerdrError::Question(QuestionFailure::Unsupported(m.into()))
}

fn not_on_screen() -> HerdrError {
    HerdrError::Question(QuestionFailure::NotOnScreen)
}

fn stuck(m: impl Into<String>) -> HerdrError {
    HerdrError::Question(QuestionFailure::Stuck(m.into()))
}

// MARK: Constructors with Swift's defaults

/// An option as the log or the screen gives it.
pub fn option(label: &str, detail: &str, recommended: bool) -> QOption {
    QOption { label: label.into(), detail: detail.into(), recommended }
}

/// Swift `Question(id:header:text:options:multiSelect:)`: `id` None is the text (Swift's default).
pub fn new_question(id: Option<&str>, header: &str, text: &str, options: Vec<QOption>, multi_select: bool) -> Question {
    Question { id: id.unwrap_or(text).into(), header: header.into(), text: text.into(), options, multi_select }
}

/// Swift `QuestionRequest(toolUseId:agent:delivery:questions:)`: pending, no headers.
pub fn new_request(tool_use_id: &str, agent: &str, delivery: Delivery, questions: Vec<Question>) -> QuestionRequest {
    let mut r = QuestionRequest::new(tool_use_id, questions);
    r.agent = agent.into();
    r.delivery = delivery;
    r
}

// MARK: JSON as `JSONSerialization` + `as?` casts see it

pub(crate) fn as_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// `as? Bool`: a JSON bool, or a number that is exactly 0 or 1 (NSNumber bridging).
pub(crate) fn as_bool(v: Option<&Value>) -> Option<bool> {
    match v? {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => match n.as_f64() {
            Some(x) if x == 0.0 => Some(false),
            Some(x) if x == 1.0 => Some(true),
            _ => None,
        },
        _ => None,
    }
}

/// `as? [[String: Any]]`: an array whose every element is an object.
pub(crate) fn as_objects(v: Option<&Value>) -> Option<Vec<&Map<String, Value>>> {
    v?.as_array()?.iter().map(Value::as_object).collect()
}

/// `as? [String]`: an array whose every element is a string.
pub(crate) fn as_strings(v: Option<&Value>) -> Option<Vec<&str>> {
    v?.as_array()?.iter().map(Value::as_str).collect()
}

// MARK: (a) Parse — Claude Code AskUserQuestion

pub const RECOMMENDED_SUFFIXES: [&str; 6] = ["(Recommended)", "（Recommended）", "（建議）", "(建議)", "（推薦）", "(推薦)"];

/// The label without its "(Recommended)" mark, and whether it had one.
pub fn strip_recommended(raw: &str) -> (String, bool) {
    let label = trim_ws(raw);
    for suf in RECOMMENDED_SUFFIXES {
        if has_suffix(label, suf) {
            return (trim_ws(&label[..label.len() - suf.len()]).to_string(), true);
        }
    }
    (label.to_string(), false)
}

fn labelled_options(q: &Map<String, Value>) -> Vec<QOption> {
    as_objects(q.get("options"))
        .unwrap_or_default()
        .into_iter()
        .map(|o| {
            let (label, rec) = strip_recommended(as_str(o.get("label")).unwrap_or(""));
            option(&label, as_str(o.get("description")).unwrap_or(""), rec)
        })
        .collect()
}

/// Claude Code's AskUserQuestion `input`: `{questions:[{question, header, options:[{label, description}], multiSelect}]}`.
pub fn claude(id: &str, input: &Map<String, Value>) -> Option<QuestionRequest> {
    let qs = as_objects(input.get("questions"))?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .into_iter()
        .map(|q| {
            new_question(
                None,
                as_str(q.get("header")).unwrap_or(""),
                as_str(q.get("question")).unwrap_or(""),
                labelled_options(q),
                as_bool(q.get("multiSelect")).unwrap_or(false),
            )
        })
        .collect();
    Some(new_request(id, "claude", Delivery::Picker, questions))
}

re!(answer_pair_re, r#""((?:[^"\\]|\\.)*)"="((?:[^"\\]|\\.)*)""#);

/// Answers come from the line's `toolUseResult.answers` (question text -> answer), else from the result text
/// (`"question"="answer", …`). An error result is a rejection.
pub fn claude_state(q: &QuestionRequest, is_error: bool, result: Option<&Value>, text: &str) -> QuestionState {
    if is_error {
        return QuestionState::Declined;
    }
    let mut by_question: HashMap<String, String> = HashMap::new();
    let from_result: Option<HashMap<String, String>> = result
        .and_then(Value::as_object)
        .and_then(|r| r.get("answers"))
        .and_then(Value::as_object)
        .and_then(|a| a.iter().map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect());
    if let Some(a) = from_result {
        by_question = a;
    } else {
        let un = |s: &str| s.replace("\\\"", "\"").replace("\\\\", "\\");
        for m in answer_pair_re().captures_iter(text) {
            by_question.insert(un(&m[1]), un(&m[2]));
        }
    }
    answered(q.questions.iter().map(|x| by_question.get(&x.id).cloned()).collect())
}

// MARK: (a) Parse — Codex request_user_input / request_user_input_async

pub const CODEX_TOOLS: [&str; 2] = ["request_user_input", "request_user_input_async"];

/// `request_user_input` (Plan mode picker): `{questions:[{id, header, question, options:[{label, description}]}]}`.
/// `request_user_input_async`: `{questions:[{title, options:[String]}]}`; it returns at once, the user answers in a message.
pub fn codex(name: &str, call_id: &str, arguments: &str) -> Option<QuestionRequest> {
    if !CODEX_TOOLS.contains(&name) {
        return None;
    }
    let o: Value = serde_json::from_str(arguments).ok()?;
    let qs = as_objects(o.as_object()?.get("questions"))?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .into_iter()
        .enumerate()
        .map(|(k, q)| {
            let opts: Vec<QOption> = q
                .get("options")
                .and_then(Value::as_array)
                .map(|a| a.as_slice())
                .unwrap_or(&[])
                .iter()
                .filter_map(|o| {
                    if let Some(s) = o.as_str() {
                        let (l, r) = strip_recommended(s);
                        return Some(option(&l, "", r));
                    }
                    let o = o.as_object()?;
                    let (l, r) = strip_recommended(as_str(o.get("label")).unwrap_or(""));
                    Some(option(&l, as_str(o.get("description")).unwrap_or(""), r))
                })
                .collect();
            let text = as_str(q.get("question")).or_else(|| as_str(q.get("title"))).unwrap_or("");
            let id = as_str(q.get("id")).map(String::from).unwrap_or_else(|| format!("q{k}"));
            let multi = as_bool(q.get("multiSelect")).or_else(|| as_bool(q.get("multi_select"))).unwrap_or(false);
            new_question(Some(&id), as_str(q.get("header")).unwrap_or(""), text, opts, multi)
        })
        .collect();
    let delivery = if name == "request_user_input" { Delivery::Picker } else { Delivery::Message };
    Some(new_request(call_id, "codex", delivery, questions))
}

/// Codex picker output: `{"answers":{"<id>":{"answers":["Blue"]}}}`; the notes come as `"user_note: …"`.
/// Anything else (e.g. "unavailable in Default mode", an interrupt) means no answer. The async tool's
/// `{"accepted":true}` returns None: the question stays pending until the next user message.
pub fn codex_state(q: &QuestionRequest, output: &str) -> Option<QuestionState> {
    let parsed: Option<Value> = serde_json::from_str(output).ok();
    let o = parsed.as_ref().and_then(Value::as_object);
    if q.delivery == Delivery::Message && as_bool(o.and_then(|o| o.get("accepted"))) == Some(true) {
        return None;
    }
    let Some(by_id) = o.and_then(|o| o.get("answers")).and_then(Value::as_object) else {
        return Some(QuestionState::Declined);
    };
    Some(answered(
        q.questions
            .iter()
            .map(|question| {
                let list = as_strings(by_id.get(&question.id).and_then(Value::as_object).and_then(|a| a.get("answers")))?;
                let notes: Vec<&str> = list.iter().filter(|s| s.starts_with("user_note: ")).map(|s| &s[11..]).collect();
                let mut picks: Vec<&str> = list.iter().copied().filter(|s| !s.starts_with("user_note: ")).collect();
                if !notes.is_empty() {
                    picks.retain(|s| *s != CODEX_OTHER_LABEL);
                }
                picks.extend(notes);
                Some(picks.join(", "))
            })
            .collect(),
    ))
}

// MARK: (a) Parse — Grok Build ask_user_question

/// `{questions:[{question, options:[{label, description}], multi_select?}]}`. Grok Build logs the call while its
/// picker is open, and the result when it is answered.
pub fn grok(call_id: &str, input: &Map<String, Value>) -> Option<QuestionRequest> {
    let qs = as_objects(input.get("questions"))?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .into_iter()
        .map(|q| {
            let multi = as_bool(q.get("multi_select")).or_else(|| as_bool(q.get("multiSelect"))).unwrap_or(false);
            new_question(None, "", as_str(q.get("question")).unwrap_or(""), labelled_options(q), multi)
        })
        .collect();
    Some(new_request(call_id, "grok", Delivery::Picker, questions))
}

/// Grok Build 1.0.67's result: `rawOutput.UserAnswered.answers[{question, picked:[label], typed?}]`, else the
/// result text (`"question"="answer", …`). A failed call or any other result (dismissed) is no answer.
pub fn grok_state(q: &QuestionRequest, failed: bool, output: Option<&Value>, text: &str) -> QuestionState {
    if failed {
        return QuestionState::Declined;
    }
    let list = output
        .and_then(Value::as_object)
        .and_then(|o| o.get("UserAnswered"))
        .and_then(Value::as_object)
        .and_then(|u| as_objects(u.get("answers")));
    if let Some(list) = list {
        return answered(
            q.questions
                .iter()
                .map(|question| {
                    let a = list.iter().find(|a| as_str(a.get("question")) == Some(question.text.as_str()))?;
                    let typed = as_str(a.get("typed")).map(trim_ws_nl).unwrap_or("");
                    let mut parts: Vec<&str> = as_strings(a.get("picked")).unwrap_or_default();
                    if !typed.is_empty() {
                        parts.push(typed);
                    }
                    Some(parts.join(", "))
                })
                .collect(),
        );
    }
    if text.starts_with("User has answered") {
        return claude_state(q, false, None, text);
    }
    QuestionState::Declined
}

/// One answer per question; `Answered(None)` when none of them was found.
pub fn answered(answers: Vec<Option<String>>) -> QuestionState {
    if !answers.iter().any(Option::is_some) {
        return QuestionState::Answered(None);
    }
    QuestionState::Answered(Some(answers.into_iter().map(Option::unwrap_or_default).collect()))
}

// MARK: (b) Answer — screen markers and keys

/// Codex's own last picker row; picking it with notes is how free text is given.
pub const CODEX_OTHER_LABEL: &str = "None of the above";

/// Text that is on screen only while the agent's question picker is open.
pub fn markers(agent: &str) -> &'static [&'static str] {
    match agent {
        "codex" => &["enter to submit"],
        "grok" => &["Shift+x:dismiss"],
        "agy" => &["esc Skip"],
        _ => &["Chat about this", "Review your answers"],
    }
}

/// One step of an answer. `WaitFor` blocks until the picker (not the rest of the screen) shows the text.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    Text(String),
    Keys(Vec<String>),
    WaitFor(String),
}

impl Step {
    pub fn sends_keys(&self) -> bool {
        !matches!(self, Step::WaitFor(_))
    }
}

fn keys(k: &[&str]) -> Step {
    Step::Keys(k.iter().map(|s| s.to_string()).collect())
}

re!(ws_run_re, r"\s+");

/// Screen text with lines trimmed and joined, so a wrapped question still matches.
pub fn flat(s: &str) -> String {
    let parts: Vec<&str> = s.split(is_newline).filter(|p| !p.is_empty()).map(trim_ws).collect();
    ws_run_re().replace_all(&parts.join(" "), " ").into_owned()
}

/// Start of a question's text (short, so a wrap inside CJK text does not break the match).
pub fn needle(q: &Question) -> String {
    let f = flat(&q.text);
    let end = f.grapheme_indices(true).nth(24).map(|(i, _)| i).unwrap_or(f.len());
    trim_ws(&f[..end]).to_string()
}

re!(question_head_re, r"^\s*Question \d+/\d+");

fn is_bar_line(l: &str) -> bool {
    let t = trim_ws(l);
    t.starts_with('☐') || t.starts_with('☒') || t.starts_with('✔') || t.starts_with('←')
}

/// The picker part of the screen only, flattened: Claude from its tab bar down, Codex from its "Question k/n" line
/// down (agy too), Grok from the top of its "┃" block down. Text above it (the user's own prompt often repeats the
/// question) must not count. None = no picker.
pub fn picker_region(agent: &str, screen: &str) -> Option<String> {
    let lines: Vec<String> = screen.split('\n').map(String::from).collect();
    let start = if agent == "grok" {
        grok_block(&lines).map(|r| r.start)
    } else if agent == "codex" || agent == "agy" {
        lines.iter().rposition(|l| question_head_re().is_match(l))
    } else {
        lines.iter().rposition(|l| is_bar_line(l))
    }?;
    let region = flat(&lines[start..].join("\n"));
    if markers(agent).iter().any(|m| region.contains(m)) {
        Some(region)
    } else {
        None
    }
}

/// `picker_region` holds `s`. For Claude also with the 2.1.322 quote bar ("│" at a line start) dropped: a long
/// question wrapped inside its needle has the bar between the two parts. A presence check only; the keys are
/// gated by the exact checks.
pub(crate) fn picker_has(agent: &str, screen: &str, s: &str) -> bool {
    if picker_region(agent, screen).is_some_and(|r| r.contains(s)) {
        return true;
    }
    agent == "claude" && {
        let unbarred: Vec<&str> = screen.split('\n').map(|l| trim_ws(l).strip_prefix('│').unwrap_or(l)).collect();
        picker_region(agent, &unbarred.join("\n")).is_some_and(|r| r.contains(s))
    }
}

/// Where Claude's focus must be for the next key (`shows`). `Auto`: from `other` (None: off the Other row;
/// "": on it, anywhere for multiSelect; text: on it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldFocus {
    Auto,
    Any,
    Other,
    Submit,
}

/// The picker shows question `k` of `q` exactly: its text, its options in order and no other, the same mode
/// (multiSelect), and for Claude the request's headers in the tab bar. `other`: what the hub put into the free
/// text field so far: None = nothing (Claude's "Other" row empty, Codex notes closed), "" = Codex notes opened
/// and empty, else the text typed. Any other state (the user's own text, notes left open) is no match: a digit
/// would be typed into that field. Codex: its rows are the options in order, then its own "None of the above".
/// `checks` (Claude multiSelect, a hooked request): the rows that must show [✔] now (the Other row is index n).
pub fn shows(
    q: &QuestionRequest,
    k: usize,
    screen: &str,
    other: Option<&str>,
    focus: FieldFocus,
    row: Option<usize>,
    checks: Option<&BTreeSet<usize>>,
) -> bool {
    if k >= q.questions.len() {
        return false;
    }
    let want = &q.questions[k];
    let labels: Vec<String> = want.options.iter().map(|o| flat(&o.label)).collect();
    if q.agent == "codex" {
        return codex_exact(q, k, screen, other, row);
    }
    if q.agent == "grok" {
        return other.is_none() && grok_exact(q, k, screen);
    }
    if q.agent == "agy" {
        return other.is_none() && agy_exact(q, k, screen);
    }
    // A request from the hook (not read from the screen) gets the exact compare; screen reads keep the parser.
    // The screen must prove question `k` only: when another question of the request also matches it (alike once
    // whitespace is dropped), the hub cannot know which one is open, so no key.
    if !q.tool_use_id.starts_with("screen-") {
        return claude_exact(q, k, screen, other, focus, checks)
            && !(0..q.questions.len()).any(|j| j != k && claude_exact(q, j, screen, other, FieldFocus::Any, None));
    }
    // Exactly one focus mark: with none or two, the hub cannot know where a key lands.
    let Some(s) = claude_parse(screen) else { return false };
    if !(s.focus_marks == 1
        && (q.headers.is_empty() || s.headers == q.headers)
        && flat(&s.question.text) == flat(&want.text)
        && s.question.multi_select == want.multi_select
        && s.question.options.iter().map(|o| flat(&o.label)).collect::<Vec<_>>() == labels)
    {
        return false;
    }
    // The "Other" row with focus is a text field: a digit there is typed, not a pick. Its empty look ("Type
    // something.") may be the user's own text, so a pick needs the row without focus and empty; "" (opened by
    // the hub's digit, live: focus on it) needs it empty (multiSelect: Down moves focus there); text, ours.
    let field = match other.map(flat) {
        None => s.typed.is_none(),
        Some(t) if t.is_empty() => s.typed.is_none(),
        Some(t) => s.typed.as_deref().map(flat) == Some(t),
    };
    let at = if focus != FieldFocus::Auto {
        focus
    } else if other.is_none() {
        FieldFocus::Auto
    } else if other == Some("") && want.multi_select {
        FieldFocus::Any
    } else {
        FieldFocus::Other
    };
    match at {
        FieldFocus::Auto => field && !s.other_focused,
        FieldFocus::Any => field,
        FieldFocus::Other => field && s.other_focused,
        FieldFocus::Submit => field && s.submit_focused,
    }
}

/// `shows` with no hub text, focus from it, any row, no box check (Swift's defaults).
pub fn shows_plain(q: &QuestionRequest, k: usize, screen: &str) -> bool {
    shows(q, k, screen, None, FieldFocus::Auto, None, None)
}

/// Codex 0.162.1's key lines (spacing aside), notes closed and open, as captured live.
pub const CODEX_KEY_LINES_CLOSED: [&str; 3] = [
    "tab to add notes | enter to submit answer | esc to interrupt",
    "tab to add notes | enter to submit answer | ←/→ to navigate questions | esc to interrupt",
    "tab to add notes | enter to submit all | ←/→ to navigate questions | esc to interrupt",
];
pub const CODEX_KEY_LINES_OPEN: [&str; 2] =
    ["tab or esc to clear notes | enter to submit answer", "tab or esc to clear notes | enter to submit all"];

re!(codex_head_re, r"^Question (\d+)/(\d+)(?: \(\d+ unanswered\))?$");

/// Codex 0.162.1's rendering of the logged question `k` is on screen, exactly. The picker head
/// ("Question k/n", maybe "(m unanswered)") and the key line (the screen's last line, closed or open notes) are
/// Codex's own; everything between them is compared with all whitespace dropped (wrapping does not count) to
/// the question text, then per row its number, label (with the log's "(Recommended)") and description, then
/// Codex's "None of the above" row, then the notes (`notes`: None closed, "" open and empty, else the hub's
/// text). The focus mark "›" may stand only right before a row number: one row must have it, the `row` one
/// when given (0-based). Text in between is matched as the log's text, so a mark or a number in it is text.
/// A request read from the screen (no log) never matches. Any other change (a new Codex version) refuses.
pub fn codex_exact(q: &QuestionRequest, k: usize, screen: &str, notes: Option<&str>, row: Option<usize>) -> bool {
    if !(q.agent == "codex" && !q.tool_use_id.starts_with("screen-") && k < q.questions.len()) {
        return false;
    }
    let want = &q.questions[k];
    if want.multi_select {
        return false;
    }
    let lines: Vec<&str> = screen.split('\n').collect();
    let Some(head) = lines.iter().rposition(|l| question_head_re().is_match(l)) else { return false };
    let Some(foot) = lines.iter().rposition(|l| !trim_ws(l).is_empty()) else { return false };
    if foot <= head {
        return false;
    }
    let Some(m) = codex_head_re().captures(trim_ws(lines[head])) else { return false };
    if int(&m[1]) != Some(k as i64 + 1) || int(&m[2]) != Some(q.questions.len() as i64) {
        return false;
    }
    // The key line, spacing aside, is one Codex 0.162.1 shows for this notes state (live captures).
    let key = spacing_aside(lines[foot]);
    let known = if notes.is_none() { CODEX_KEY_LINES_CLOSED.contains(&key.as_str()) } else { CODEX_KEY_LINES_OPEN.contains(&key.as_str()) };
    if !known {
        return false;
    }
    let all = bare(&lines[head + 1..foot].join("\n"));
    let mut body: &str = &all;
    fn take(body: &mut &str, lit: &str) -> bool {
        match body.strip_prefix(lit) {
            Some(rest) => {
                *body = rest;
                true
            }
            None => false,
        }
    }
    if !take(&mut body, &bare(&want.text)) {
        return false;
    }
    let mut marked: Vec<usize> = vec![];
    let mut rows: Vec<(Vec<String>, String)> = want
        .options
        .iter()
        .map(|o| {
            let labels = if o.recommended {
                RECOMMENDED_SUFFIXES.iter().map(|s| bare(&format!("{}{}", o.label, s))).collect()
            } else {
                vec![bare(&o.label)]
            };
            (labels, bare(&o.detail))
        })
        .collect();
    rows.push((vec![bare(CODEX_OTHER_LABEL)], bare("Optionally, add details in notes (tab)")));
    for (i, (labels, detail)) in rows.iter().enumerate() {
        if take(&mut body, "›") {
            marked.push(i); // a row starts with its number: a mark before it is the focus
        }
        if !take(&mut body, &format!("{}.", i + 1)) {
            return false;
        }
        let fits: Vec<&String> = labels.iter().filter(|l| body.starts_with(l.as_str())).collect();
        if !(fits.len() == 1 && take(&mut body, fits[0]) && take(&mut body, detail)) {
            return false;
        }
    }
    if let Some(notes) = notes {
        let lit = format!("›{}", if notes.is_empty() { "Addnotes".to_string() } else { bare(notes) });
        if !take(&mut body, &lit) {
            return false;
        }
    }
    if !(body.is_empty() && marked.len() == 1) {
        return false;
    }
    row.map_or(true, |r| marked == [r])
}

re!(codex_row_re, r"(?R)^\s*(?:›\s*)?(\d+)\.\s+(.*)$");

/// Codex's picker read from the screen (Swift `codexScreen`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexScreen {
    pub text: String,
    pub rows: Vec<String>,
    pub notes: Option<String>,
}

/// Codex's picker, read from the screen: the question text (flat) and its option rows in order, each row's
/// text after "N." (label, then its description after two spaces), and `notes`: None when closed, "" when open
/// and empty ("› Add notes"), else their text (Codex 0.162, live). None when no Codex picker is shown or the
/// rows are not numbered 1, 2, 3...
pub fn codex_screen(screen: &str) -> Option<CodexScreen> {
    let lines: Vec<&str> = screen.split('\n').collect();
    let start = lines.iter().rposition(|l| question_head_re().is_match(l))?;
    picker_region("codex", screen)?;
    // The key line is the last line of the picker, one of Codex's own two; text above it (notes too) may hold
    // the same words and never ends the parse.
    let foot = lines.iter().rposition(|l| !trim_ws(l).is_empty())?;
    if foot <= start {
        return None;
    }
    let key_line = trim_ws(lines[foot]);
    let notes_open = if key_line.starts_with("tab or esc to clear notes | enter to submit") {
        true
    } else if key_line.starts_with("tab to add notes | enter to submit") {
        false
    } else {
        return None;
    };
    let (mut text, mut rows, mut notes): (Vec<String>, Vec<String>, Vec<String>) = (vec![], vec![], vec![]);
    let (mut in_notes, mut after) = (false, false);
    for l in &lines[start + 1..foot] {
        let t = trim_ws(l);
        if in_notes {
            if !t.is_empty() {
                notes.push(t.to_string());
            }
        } else if let (false, Some(m)) = (after, codex_row_re().captures(l)) {
            if int(&m[1]) != Some(rows.len() as i64 + 1) {
                return None;
            }
            rows.push(trim_ws(&m[2]).to_string());
        } else if rows.is_empty() {
            if !t.is_empty() {
                text.push(t.to_string());
            }
        } else if t.is_empty() {
            after = true;
        } else if after && t.starts_with('›') {
            in_notes = true;
            notes.push(trim_ws(drop_first(t)).to_string());
        } else {
            return None; // a wrapped row or something unknown: cannot be compared exactly
        }
    }
    if rows.is_empty() || text.is_empty() || in_notes != notes_open {
        return None; // the key line and the field agree
    }
    let typed = flat(&notes.join("\n"));
    Some(CodexScreen {
        text: flat(&text.join(" ")),
        rows,
        notes: if in_notes { Some(if typed == "Add notes" { String::new() } else { typed }) } else { None },
    })
}

/// Two questions are the same: text, option labels in order, mode.
pub fn same_question(a: &Question, b: &Question) -> bool {
    flat(&a.text) == flat(&b.text)
        && a.options.iter().map(|o| flat(&o.label)).collect::<Vec<_>>() == b.options.iter().map(|o| flat(&o.label)).collect::<Vec<_>>()
        && a.multi_select == b.multi_select
}

/// `full` (every question read) is the request the client answered (`asked`, maybe with questions unread):
/// the same agent, headers and question count, and every question `asked` holds is in `full`, unchanged.
pub fn same_request(full: &QuestionRequest, asked: &QuestionRequest) -> bool {
    if !(full.agent == asked.agent
        && full.headers == asked.headers
        && full.unread() == 0
        && (asked.headers.is_empty() || full.questions.len() == asked.headers.len()))
    {
        return false;
    }
    if asked.questions.len() == full.questions.len() {
        return asked.questions.iter().zip(&full.questions).all(|(a, f)| same_question(a, f));
    }
    asked.questions.iter().all(|a| full.questions.iter().any(|f| same_question(a, f)))
}

/// The picker shows some question of `q` exactly (`shows`).
pub fn shows_any(q: &QuestionRequest, screen: &str) -> bool {
    (0..q.questions.len()).any(|k| shows_plain(q, k, screen))
}

/// Claude's review screen of `q`: the request's headers and every one of its questions listed. A hooked request
/// needs the exact review of `answers` (`claude_review_exact`).
pub fn shows_review(q: &QuestionRequest, screen: &str, answers: Option<&[QuestionAnswer]>) -> bool {
    if !q.tool_use_id.starts_with("screen-") {
        return answers.is_some_and(|a| claude_review_exact(q, a, screen));
    }
    let Some(region) = picker_region(&q.agent, screen) else { return false };
    if !region.contains("Review your answers") {
        return false;
    }
    q.headers.iter().all(|h| region.contains(h.as_str())) && q.questions.iter().all(|x| picker_has(&q.agent, screen, &flat(&x.text)))
}

pub fn picker_shown(q: &QuestionRequest, screen: &str) -> bool {
    picker_region(&q.agent, screen).is_some()
}

/// The picker is open and shows the first question.
pub fn screen_shows(q: &QuestionRequest, screen: &str) -> bool {
    let Some(first) = q.questions.first() else { return false };
    picker_has(&q.agent, screen, &needle(first))
}

/// The key sequence for a picker question. Errs with `Unsupported` before anything is sent.
pub fn steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Result<Vec<Step>, HerdrError> {
    if answers.len() != q.questions.len() {
        return Err(unsupported(format!("{} answers for {} questions", answers.len(), q.questions.len())));
    }
    for (k, a) in answers.iter().enumerate() {
        let n = q.questions[k].options.len();
        if !(a.picks.iter().all(|&p| p < n) && n <= 8) {
            return Err(unsupported(format!("question {}: option out of range", k + 1)));
        }
        let other = trim_ws_nl(a.other.as_deref().unwrap_or(""));
        if other.chars().any(is_newline) {
            return Err(unsupported(format!("question {}: line break in the text", k + 1)));
        }
        // A control key in the text (Tab closes Codex's notes) would act between two gated keys.
        if other.chars().any(is_control) {
            return Err(unsupported(format!("question {}: a control character in the text", k + 1)));
        }
        // The empty field shows these: the gate could not tell the answer from no answer.
        if ["Add notes", "Type something.", "Type something"].contains(&flat(other).as_str()) {
            return Err(unsupported(format!("question {}: this text is the field's placeholder", k + 1)));
        }
        if a.picks.is_empty() && other.is_empty() {
            return Err(unsupported(format!("question {}: no answer", k + 1)));
        }
        if !q.questions[k].multi_select && a.picks.len() > 1 {
            return Err(unsupported(format!("question {}: one choice only", k + 1)));
        }
    }
    match q.agent.as_str() {
        "codex" => codex_steps(q, answers),
        "grok" => grok_steps(q, answers),
        "agy" => agy_steps(q, answers),
        _ => Ok(claude_steps(q, answers)),
    }
}

/// The answer's free text, trimmed; None when empty.
pub fn other_text(a: &QuestionAnswer) -> Option<String> {
    let s = trim_ws_nl(a.other.as_deref().unwrap_or(""));
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn digit(p: usize) -> Step {
    Step::Text(format!("{}", p + 1))
}

/// Claude Code 2.1.296. Single choice: digit picks and moves on ("Other" = digit n+1, text, Enter).
/// multiSelect: digits toggle; "Other" = digit n+1, Down to the row, text, Tab, Enter; without it Tab moves on.
/// After the last question of a multi-question or multiSelect card: review screen, `1` submits.
fn claude_steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Vec<Step> {
    let mut out = vec![];
    for (k, question) in q.questions.iter().enumerate() {
        let a = &answers[k];
        let n = question.options.len();
        if !question.multi_select {
            if let Some(other) = other_text(a) {
                out.extend([Step::Text(format!("{}", n + 1)), Step::Text(other), keys(&["Enter"])]);
            } else {
                out.push(digit(a.picks[0]));
            }
        } else {
            let mut picks = a.picks.clone();
            picks.sort();
            for p in picks {
                out.push(digit(p));
            }
            if let Some(other) = other_text(a) {
                out.extend([
                    Step::Text(format!("{}", n + 1)),
                    Step::Keys(vec!["Down".to_string(); n]),
                    Step::Text(other),
                    keys(&["Tab"]),
                    keys(&["Enter"]),
                ]);
            } else {
                out.push(keys(&["Tab"]));
            }
        }
        if k + 1 < q.questions.len() {
            out.push(Step::WaitFor(needle(&q.questions[k + 1])));
        }
    }
    if q.questions.len() > 1 || q.questions.last().is_some_and(|x| x.multi_select) {
        out.extend([Step::WaitFor("Review your answers".into()), Step::Text("1".into())]);
    }
    out
}

/// Codex 0.162 (Plan mode picker). Digit picks and moves on; on the last question it submits all.
/// Free text: Down to "None of the above" (row n+1), Tab opens the notes, text, Enter. No multiSelect.
fn codex_steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Result<Vec<Step>, HerdrError> {
    let mut out = vec![];
    for (k, question) in q.questions.iter().enumerate() {
        if question.multi_select {
            return Err(unsupported("Codex has no multiSelect picker"));
        }
        let a = &answers[k];
        let n = question.options.len();
        if let Some(other) = other_text(a) {
            // Tab is its own step: it waits for the focus on the row it opens the notes for (`answer_question`).
            let row = a.picks.first().copied().unwrap_or(n);
            if row > 0 {
                out.push(Step::Keys(vec!["Down".to_string(); row]));
            }
            out.extend([keys(&["Tab"]), Step::Text(other), keys(&["Enter"])]);
        } else {
            out.push(digit(a.picks[0]));
        }
        if k + 1 < q.questions.len() {
            out.push(Step::WaitFor(needle(&q.questions[k + 1])));
        }
    }
    Ok(out)
}

/// Grok Build 1.0.67. Single choice: a digit picks and moves on; on the last question it submits all (live).
/// Its multi-choice rows and free-text row show no focus or typed state in plain text until a later redraw, so a
/// key could not be checked: those are answered in the terminal.
fn grok_steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Result<Vec<Step>, HerdrError> {
    let mut out = vec![];
    for (k, question) in q.questions.iter().enumerate() {
        if question.multi_select {
            return Err(unsupported("Grok's multi-choice picker is answered in the terminal"));
        }
        if other_text(&answers[k]).is_some() {
            return Err(unsupported("Grok's free text is answered in the terminal"));
        }
        out.push(digit(answers[k].picks[0]));
        if k + 1 < q.questions.len() {
            out.push(Step::WaitFor(needle(&q.questions[k + 1])));
        }
    }
    Ok(out)
}

/// `Message` delivery: the text sent as a user message. One question: the answer; several: "question: answer" lines.
/// (A pick out of range, which Swift would trap on, is left out; `answer_question` refuses it before sending.)
pub fn message_text(q: &QuestionRequest, answers: &[QuestionAnswer]) -> String {
    let parts: Vec<String> = q
        .questions
        .iter()
        .zip(answers)
        .map(|(question, a)| {
            let mut picks = a.picks.clone();
            picks.sort();
            let mut v: Vec<String> =
                picks.iter().filter_map(|&p| question.options.get(p)).map(|o| o.label.clone()).collect();
            v.extend(other_text(a));
            let v = v.join(", ");
            if q.questions.len() == 1 {
                v
            } else {
                format!("{} {}", question.text, v)
            }
        })
        .collect();
    parts.join("\n")
}

// MARK: Grok Build: the picker on screen

/// The lines of Grok's picker: the last run of lines that start with "┃" (its left bar). None when there is none.
pub fn grok_block(lines: &[String]) -> Option<std::ops::Range<usize>> {
    let bar = |s: &str| trim_ws(s).starts_with('┃');
    let last = lines.iter().rposition(|l| bar(l))?;
    let mut first = last;
    while first > 0 && bar(&lines[first - 1]) {
        first -= 1;
    }
    Some(first..last + 1)
}

/// A "┃" block's text lines: the bar, the scrollbar "█" at a line's end and blank lines dropped.
pub fn grok_lines(block: &[String]) -> Vec<String> {
    block
        .iter()
        .map(|l| {
            let mut t = drop_first(trim_ws(l)); // the "┃"
            while let Some((i, g)) = t.grapheme_indices(true).next_back() {
                if g == "█" || g_is_whitespace(g) {
                    t = &t[..i];
                } else {
                    break;
                }
            }
            trim_ws(t).to_string()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

re!(grok_row_re, r"(?R)^(\d+|z) \(([○●])\) (.*)$");

/// Grok Build 1.0.67's picker shows question `k` of `q` exactly, nothing picked yet: in its "┃" block the
/// question text, then rows "1 (○) label  description" for the options in order and no other, then its own
/// "z (○) Type your answer here", then "[k/n] …" with "Enter:select" ("Enter:submit" on the last question; a
/// single question shows no "[1/1]").
/// Text is compared with all whitespace dropped (wrapping does not count); the scrollbar "█" at a line's end is
/// not text. A picked row (●), a field being edited or the focus on the free-text row ("Enter:edit") refuses.
pub fn grok_exact(q: &QuestionRequest, k: usize, screen: &str) -> bool {
    if !(q.agent == "grok" && !q.tool_use_id.starts_with("screen-") && k < q.questions.len()) {
        return false;
    }
    let want = &q.questions[k];
    if want.multi_select {
        return false;
    }
    let lines: Vec<String> = screen.split('\n').map(String::from).collect();
    let Some(block) = grok_block(&lines) else { return false };
    if picker_region("grok", screen).is_none() {
        return false;
    }
    let body = grok_lines(&lines[block]);
    let Some(first_row) = body.iter().position(|l| grok_row_re().is_match(l)) else { return false };
    if !(first_row > 0 && body.len() == first_row + want.options.len() + 2) {
        return false;
    }
    if bare(&body[..first_row].concat()) != bare(&want.text) {
        return false;
    }
    for (i, o) in want.options.iter().enumerate() {
        let Some(m) = grok_row_re().captures(&body[first_row + i]) else { return false };
        if !(&m[1] == format!("{}", i + 1).as_str() && &m[2] == "○") {
            return false;
        }
        let mut labels: Vec<String> = vec![];
        if o.recommended {
            labels.extend(RECOMMENDED_SUFFIXES.iter().map(|s| bare(&format!("{}{}", o.label, s))));
        }
        labels.push(bare(&o.label));
        let shown = bare(&m[3]);
        if !labels.iter().any(|l| shown == format!("{}{}", l, bare(&o.detail))) {
            return false;
        }
    }
    if body[first_row + want.options.len()] != "z (○) Type your answer here" {
        return false;
    }
    let foot = &body[first_row + want.options.len() + 1];
    let last = k + 1 == q.questions.len();
    // One question: Grok shows no "[1/1]" count (live, 1.0.67); its footer is the keys and "Enter:submit".
    let counted = foot.starts_with(&format!("[{}/{}] ", k + 1, q.questions.len())) || (q.questions.len() == 1 && !foot.starts_with('['));
    counted && foot.ends_with(if last { "Enter:submit" } else { "Enter:select" })
}

// MARK: Claude Code: the pending question comes from the screen

/// A screen-read request. Headers go on the questions when all are read; the id depends only on the tab bar
/// (plus the text for a single question), so the read-only and the full read give the same id.
pub fn screen_request(headers: &[String], questions: Vec<Question>) -> QuestionRequest {
    let mut key = headers.join("|");
    if headers.len() == 1 {
        key.push('|');
        key.push_str(questions.first().map(|q| q.text.as_str()).unwrap_or(""));
    }
    let hash = key.bytes().fold(5381u32, |h, b| (h << 5).wrapping_add(h).wrapping_add(b as u32)); // stable across calls and runs
    let mut qs = questions;
    if qs.len() == headers.len() {
        for (q, h) in qs.iter_mut().zip(headers) {
            q.header = h.clone();
        }
    }
    let mut r = new_request(&format!("screen-{hash:x}"), "claude", Delivery::Picker, qs);
    r.headers = headers.to_vec();
    r
}

/// Claude Code 2.1.296 writes the AskUserQuestion call to its log only after it is answered, so while the
/// picker is open the question exists only on screen. This reads the visible question: the tab-bar headers
/// (one per question) and the current question with its options. None when no Claude picker is shown.
pub fn claude_screen(screen: &str) -> Option<(Vec<String>, Question)> {
    claude_parse(screen).map(|p| (p.headers, p.question))
}

/// `claude_screen` plus what decides where a key lands (Swift `claudeParse`'s tuple).
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeParse {
    pub headers: Vec<String>,
    pub question: Question,
    /// Text typed into the "Other" row; None while it shows its placeholder.
    pub typed: Option<String>,
    pub other_focused: bool,
    pub submit_focused: bool,
    /// Focus marks on rows and Submit; 0 when a mark stands anywhere else.
    pub focus_marks: usize,
}

re!(claude_opt_re, r"(?R)^(\s*(?:❯\s*)?)(\d+)\.\s+(\[[ ✔]\]\s+)?(.*)$");

/// `claude_screen` plus `typed`: text typed into the "Other" row. 2.1.296 always shows that row as the last
/// numbered row before the line above "Chat about this": "Type something." (multiSelect "Type something") when
/// empty, else the typed text. It is never an option. No such line: None (not a layout this reads).
pub fn claude_parse(screen: &str) -> Option<ClaudeParse> {
    let lines: Vec<&str> = screen.split('\n').collect();
    let end = lines.iter().rposition(|l| l.contains("Chat about this"))?;
    let bar = lines[..end].iter().rposition(|l| is_bar_line(l))?;
    let headers: Vec<String> = split_graphemes(lines[bar], |g| matches!(g, "☐" | "☒" | "✔" | "←" | "→"))
        .into_iter()
        .map(trim_ws)
        .filter(|h| !h.is_empty() && *h != "Submit")
        .map(String::from)
        .collect();
    let mut text: Vec<String> = vec![];
    let mut options: Vec<QOption> = vec![];
    let (mut multi, mut in_options, mut closed) = (false, false, false);
    let (mut last_raw, mut last_focused, mut submit_focused) = (String::new(), false, false);
    // The Submit control (multiSelect) is the line right after the last row and right before the separator;
    // the same text anywhere else is option text. `since_row`: lines with text since the last numbered row.
    // Rows: after the two-column focus slot ("❯ "), their number in the column of row 1, numbered 1, 2, 3…; a
    // numbered line in another column is question or option text.
    // `marks` counts the focus marks on rows and on Submit; a mark on any other line means the hub does not
    // know where the focus is (focus_marks 0: no key).
    let (mut marks, mut since_row, mut row_column, mut stray_mark) = (0usize, 0usize, None::<usize>, false);
    for l in &lines[bar + 1..end] {
        let t = trim_ws(l);
        if t.starts_with("──") {
            closed = true;
            break;
        }
        let marked = t.starts_with('❯');
        if !t.is_empty() && submit_focused {
            // that was not Submit
            submit_focused = false;
            stray_mark = true;
            since_row += 1;
        }
        if in_options && since_row == 0 && marked && trim_ws(drop_first(t)) == "Submit" {
            submit_focused = true;
            marks += 1;
            continue;
        }
        let row = claude_opt_re().captures(l).filter(|m| match row_column {
            None => &m[2] == "1" && gcount(&m[1]) >= 2,
            Some(c) => gcount(&m[1]) == c,
        });
        if let Some(m) = row {
            if row_column.is_none() {
                row_column = Some(gcount(&m[1]));
            }
            // A gap or a repeated number: the rows are not what they seem, so no picker is read.
            if int(&m[2]) != Some(options.len() as i64 + 1) {
                return None;
            }
            since_row = 0;
            if marked {
                marks += 1;
            }
            let label = trim_ws(&m[4]).to_string();
            in_options = true;
            if m.get(3).is_some() {
                multi = true;
            }
            let (clean, rec) = strip_recommended(&label);
            options.push(option(&clean, "", rec));
            last_raw = label;
            last_focused = trim_ws(l).starts_with('❯');
        } else if in_options {
            if marked {
                stray_mark = true;
            }
            if !t.is_empty() {
                since_row += 1;
            }
            if let (false, Some(o)) = (t.is_empty(), options.last_mut()) {
                if !o.detail.is_empty() {
                    o.detail.push(' ');
                }
                o.detail.push_str(t);
            }
        } else if !t.is_empty() {
            if marked {
                stray_mark = true;
            }
            text.push(t.to_string());
        }
    }
    // A long question's text is drawn with "│ " in front of each line (2.1.322): drop it when every line has it
    // and the text without it is long (claude_exact::QUOTE_MIN); a short text's own "│" stays.
    if !text.is_empty() && text.iter().all(|t| t.starts_with('│')) {
        let unquoted: Vec<String> = text.iter().map(|t| trim_ws(&t['│'.len_utf8()..]).to_string()).collect();
        if gcount(&unquoted.join(" ")) >= claude_exact::QUOTE_MIN {
            text = unquoted;
        }
    }
    // The last row is the "Other" row.
    if !closed || options.pop().is_none() {
        return None;
    }
    // The Other row keeps its raw text: "(Recommended)" there is typed text, not a mark.
    let typed = if last_raw == "Type something." || last_raw == "Type something" { None } else { Some(last_raw) };
    if options.is_empty() || text.is_empty() {
        return None;
    }
    let header = if headers.len() == 1 { headers[0].clone() } else { String::new() };
    let question = new_question(None, &header, &text.join(" "), options, multi);
    Some(ClaudeParse {
        headers,
        question,
        typed,
        other_focused: last_focused,
        submit_focused,
        focus_marks: if stray_mark { 0 } else { marks },
    })
}

// MARK: Keys (Swift: `extension PaneScreen`)

/// Reads the screen until `ok` holds, every 150 ms, for about `timeout_ms` (by the pauses taken, or by the
/// clock, whichever comes first: a fake that skips pauses still stops). A read that fails ends it with that error.
fn wait_screen<S: PaneScreen + ?Sized>(screen: &S, pane_id: &str, timeout_ms: u64, mut ok: impl FnMut(&str) -> bool) -> Result<bool, HerdrError> {
    let start = std::time::Instant::now();
    let mut slept = 0u64;
    loop {
        if ok(&screen.visible_text(pane_id)?) {
            return Ok(true);
        }
        if slept >= timeout_ms || start.elapsed() >= std::time::Duration::from_millis(timeout_ms) {
            return Ok(false);
        }
        screen.pause(150);
        slept += 150;
    }
}

/// How long a gated answer key waits for its exact screen (the render of the key before it) before it stops. A busy
/// machine renders slower than the 1.5 s the checks used to give.
const KEY_WAIT_MS: u64 = 5000;

/// The gate before one key of an answer. A gate with a screen check first waits (up to `KEY_WAIT_MS`) for `ok`, so a
/// slow render is waited for and not taken for a change; then the full check runs on a fresh read right before the
/// key, as `gated`. A stop names its cause: the agent or the input level changed (`KeyGateStop`, as before), or the
/// same agent shows another screen (`ScreenStop`, with `sent`: keys of this answer went in before).
fn answer_gated<S: PaneScreen + ?Sized>(screen: &S, gate: Option<&KeyGate>, pane_id: &str, ok: &dyn Fn(&str) -> bool, sent: bool) -> Result<(), HerdrError> {
    let Some(g) = gate else { return Ok(()) };
    let _ = wait_screen(screen, pane_id, KEY_WAIT_MS, ok); // read-only; a failed read stops below
    if passes(screen, Some(g), pane_id, Some(ok), None) {
        return Ok(());
    }
    let same_agent = g.agent.as_ref().is_none_or(|a| fresh_agent(screen, a, pane_id).is_some());
    let allowed = g.allowed.as_ref().is_none_or(|f| f());
    Err(if same_agent && allowed { HerdrError::ScreenStop { keys_sent: sent } } else { HerdrError::KeyGateStop })
}

/// Named keys of an answer, each behind `answer_gated` (one key per call, so no committing key rides in a batch past
/// a check). No gate: one call. `sent` is set once a key went.
fn answer_keys<S: PaneScreen + ?Sized>(screen: &S, keys: &[String], gate: Option<&KeyGate>, pane_id: &str, ok: &dyn Fn(&str) -> bool, sent: &mut bool) -> Result<(), HerdrError> {
    if gate.is_none() {
        screen.send_named_keys(keys, pane_id)?;
        *sent = true;
        return Ok(());
    }
    for k in keys {
        answer_gated(screen, gate, pane_id, ok, *sent)?;
        screen.send_named_keys(std::slice::from_ref(k), pane_id)?;
        *sent = true;
    }
    Ok(())
}

/// Why `review_blocked` refuses (`QuestionFailure::Unsupported`).
pub const REVIEW_BLOCKED: &str = "the review screen could not show which options were picked (a label with a comma or a review mark, or two alike)";

/// The refusal `answer_question` makes before any key when the review screen cannot prove the answers (a hooked
/// Claude picker with 2+ questions or a multi-choice last one, and `review_ambiguous`). A card shows it before Send.
pub fn review_blocked(q: &QuestionRequest, answers: &[QuestionAnswer]) -> bool {
    let hooked = q.agent == "claude" && !q.tool_use_id.starts_with("screen-");
    q.delivery == Delivery::Picker
        && hooked
        && (q.questions.len() > 1 || q.questions.last().is_some_and(|x| x.multi_select))
        && review_ambiguous(q, answers)
}

/// Answers a pending question in `pane_id`, one `QuestionAnswer` per question.
/// Picker: (1) checks the picker with the first question is on screen (Claude: another question of this request
/// is moved back with Left; anything else errs with `NotOnScreen`, nothing sent),
/// (2) sends the keys, waiting for each next question, (3) checks the picker is gone (else `StillOnScreen`).
/// Message (Codex async): sends the answer text and Enter, like the composer (`send_text`).
pub fn answer_question<S: PaneScreen + ?Sized>(
    screen: &S,
    pane_id: &str,
    q: &QuestionRequest,
    answers: &[QuestionAnswer],
    gate: Option<&KeyGate>,
) -> Result<(), HerdrError> {
    if q.state != QuestionState::Pending {
        return Err(unsupported("the question is not pending"));
    }
    if q.unread() != 0 {
        return Err(unsupported(format!("{} questions not read yet (loadAllQuestions)", q.unread())));
    }
    if q.delivery == Delivery::Message {
        if !(answers.len() == q.questions.len() && answers.iter().all(|a| !a.picks.is_empty() || other_text(a).is_some())) {
            return Err(unsupported("no answer"));
        }
        gated(screen, gate, pane_id, None)?;
        // Swift traps here on a pick out of range (`messageText`): nothing is sent either way.
        let in_range = q.questions.iter().zip(answers).all(|(x, a)| a.picks.iter().all(|&p| p < x.options.len()));
        if !in_range {
            return Err(unsupported("option out of range"));
        }
        return screen.send_text(&message_text(q, answers), pane_id); // the composer path: text + Enter
    }
    let steps = steps(q, answers)?;
    // Typed answers go key by key: refused before any key when the backend cannot type one of their characters.
    if let Some(bad) = answers.iter().filter_map(other_text).find(|t| !screen.can_type(t)) {
        let shown: String = bad.chars().filter(|c| !screen.can_type(&c.to_string())).collect::<BTreeSet<char>>().into_iter().collect();
        return Err(unsupported(format!("the typed answer has characters Herdr cannot type yet ({shown}): answer in the terminal")));
    }
    // `gate` (a remote device): before every key, the same agent, the exact question the key is for on screen
    // (its text, options, the request's headers; the review screen for the last key), input allowed.
    let any = |s: &str| shows_any(q, s);
    // A hooked request whose review cannot prove the answers: refused before any key (the Left below too).
    let hooked = q.agent == "claude" && !q.tool_use_id.starts_with("screen-");
    if review_blocked(q, answers) {
        return Err(unsupported(REVIEW_BLOCKED));
    }
    let now = screen.visible_text(pane_id)?;
    let mut sent = false; // a key of this answer went in
    if !screen_shows(q, &now) {
        // Claude: the user may have moved the picker to a later question of THIS request; go back to question 1.
        let back = q.agent == "claude"
            && q.questions.len() > 1
            && claude_screen(&now).is_some_and(|(_, shown)| q.questions.iter().skip(1).any(|x| x.text == shown.text));
        if !back {
            return Err(not_on_screen());
        }
        answer_keys(screen, &vec!["Left".to_string(); q.questions.len() - 1], gate, pane_id, &any, &mut sent)?;
        if !wait_screen(screen, pane_id, 2000, |s| screen_shows(q, s))? {
            return Err(stuck("back to question 1"));
        }
    }
    // `shown`: what the "Other" row of question `at` should hold now: empty until the hub typed its text there.
    // `focus`: Claude multiSelect "Other": Down keys may run with focus anywhere, the text and Tab need it on the
    // row, Enter needs it on Submit (live, 2.1.296).
    // `row` (Codex): the row whose notes the hub opens; Tab and every key after it need the focus there.
    // A hooked Claude request (`ClaudeQuestionHook`): the exact rendering before every key, the [✔] rows too
    // (`checked`: what the hub's digits toggled; the picker starts with none), and a short wait for the render
    // of the last key before that check (read-only).
    let (mut at, mut review, mut other_open, mut typed_in) = (0usize, false, false, false);
    let mut shown: Option<String> = None;
    let mut focus = FieldFocus::Auto;
    let mut row: Option<usize> = None;
    let mut checked: BTreeSet<usize> = BTreeSet::new();
    for step in &steps {
        if q.agent == "codex" && *step == keys(&["Tab"]) && at < answers.len() && at < q.questions.len() {
            row = Some(answers[at].picks.first().copied().unwrap_or(q.questions[at].options.len()));
        }
        let (k, r, in_row, f, fr) = (at, review, shown.clone(), focus, row);
        let multi = k < q.questions.len() && q.questions[k].multi_select;
        let ck: Option<BTreeSet<usize>> = if hooked && multi { Some(checked.clone()) } else { None };
        let same = |s: &str| -> bool {
            if r {
                shows_review(q, s, Some(answers))
            } else {
                shows(q, k, s, in_row.as_deref(), f, fr, ck.as_ref())
            }
        };
        // No gate: no check runs, but the render of the last key is still waited for a moment (with a gate,
        // `answer_gated` waits for the exact screen).
        if hooked && gate.is_none() && step.sends_keys() {
            let _ = wait_screen(screen, pane_id, 1500, same)?;
        }
        // Grok and agy on the Mac (no gate): the exact question before each key too, not only its first words.
        if (q.agent == "grok" || q.agent == "agy") && gate.is_none() && step.sends_keys() && !wait_screen(screen, pane_id, 1500, same)? {
            return Err(not_on_screen());
        }
        match step {
            Step::Text(s) => {
                answer_gated(screen, gate, pane_id, &same, sent)?;
                screen.send_keys(s.as_bytes(), pane_id)?; // typed as is: no Enter, not as a paste
                sent = true;
                // By step purpose, not by text (the typed text may equal a digit): Claude's first text step for
                // "Other" is its digit, which opens the row; the next text step is the typed text.
                let typed = if k < answers.len() { other_text(&answers[k]) } else { None };
                let typed_text = typed.is_some() && other_open && !typed_in;
                if hooked && multi && !r && !typed_text {
                    if let Some(d) = int(s).filter(|&d| d >= 1 && d <= q.questions[k].options.len() as i64 + 1) {
                        let i = (d - 1) as usize;
                        if !checked.remove(&i) {
                            checked.insert(i);
                        }
                    }
                }
                if typed.is_some() && !other_open && q.agent == "claude" && k < q.questions.len() && *s == format!("{}", q.questions[k].options.len() + 1) {
                    other_open = true;
                    shown = Some(String::new());
                } else if typed.is_some() && other_open && !typed_in {
                    shown = typed;
                    typed_in = true;
                }
            }
            Step::Keys(ks) => {
                answer_keys(screen, ks, gate, pane_id, &same, &mut sent)?;
                let multi_other = q.agent == "claude" && other_open && at < q.questions.len() && q.questions[at].multi_select;
                if multi_other && !typed_in && ks.iter().all(|x| x == "Down") {
                    focus = FieldFocus::Other; // now on the row
                }
                if multi_other && typed_in && *ks == ["Tab"] {
                    focus = FieldFocus::Submit;
                }
                // Codex: Tab on "None of the above" opens its notes, empty.
                if q.agent == "codex" && ks.last().is_some_and(|x| x == "Tab") && at < answers.len() && other_text(&answers[at]).is_some() {
                    other_open = true;
                    shown = Some(String::new());
                }
            }
            Step::WaitFor(s) => {
                if s == "Review your answers" {
                    review = true;
                } else {
                    at += 1;
                }
                other_open = false;
                typed_in = false;
                shown = None;
                focus = FieldFocus::Auto;
                row = None;
                checked.clear();
                if !wait_screen(screen, pane_id, 3000, |x| picker_has(&q.agent, x, s))? {
                    return Err(stuck(s.clone()));
                }
            }
        }
        screen.pause(120); // the TUI handles one key event per render
    }
    if !wait_screen(screen, pane_id, 5000, |s| !picker_shown(q, s))? {
        return Err(HerdrError::Question(QuestionFailure::StillOnScreen));
    }
    Ok(())
}

/// Claude Code's open question picker, read from the screen (its log has the call only after the answer).
/// READ-ONLY: sends nothing, so it is safe to call automatically. `questions` = the question on screen;
/// `headers` = all questions' headers from the tab bar; `unread` > 0 when there are more questions than the one
/// shown (then call `load_all_questions` on a user action). None when the pane shows no Claude picker.
/// `tool_use_id` is "screen-…", the same for both calls (the real id appears in the log with the answer).
pub fn pending_question<S: PaneScreen + ?Sized>(screen: &S, pane_id: &str) -> Result<Option<QuestionRequest>, HerdrError> {
    let Some((headers, question)) = claude_screen(&screen.visible_text(pane_id)?) else { return Ok(None) };
    Ok(Some(screen_request(&headers, vec![question])))
}

/// SENDS KEYS: call only on a user action (e.g. Next on question 1, or as the first step of Send), never on a
/// timer, refresh or render. Moves the picker with Left to question 1, reads each question with Right
/// (navigation only, nothing is chosen), then moves back to the question that was on screen and checks it is
/// there again (`Stuck` if not). One question: no keys, same as `pending_question`.
pub fn load_all_questions<S: PaneScreen + ?Sized>(
    screen: &S,
    pane_id: &str,
    gate: Option<&KeyGate>,
    expect: Option<&QuestionRequest>,
) -> Result<Option<QuestionRequest>, HerdrError> {
    let Some((headers, shown)) = claude_screen(&screen.visible_text(pane_id)?) else { return Ok(None) };
    // `expect`: the request the caller answers. The picker must still show it (same headers, a question it
    // holds), before any key and in the result; else `NotOnScreen`, nothing sent.
    if let Some(expect) = expect {
        if headers != expect.headers || !expect.questions.iter().any(|x| same_question(x, &shown)) {
            return Err(not_on_screen());
        }
    }
    let n = headers.len();
    if n <= 1 {
        let one = screen_request(&headers, vec![shown]);
        if let Some(expect) = expect {
            if !same_request(&one, expect) {
                return Err(not_on_screen());
            }
        }
        return Ok(Some(one));
    }
    let now = || -> Result<Option<Question>, HerdrError> { Ok(claude_screen(&screen.visible_text(pane_id)?).map(|(_, q)| q)) };
    // `gate` (a remote device): before every key, the same agent, this request's picker still open, input allowed.
    // Bound to what was read: the same headers and, before each key, the exact question now expected on screen.
    let exact = |want: Question| {
        let headers = headers.clone();
        move |s: &str| claude_screen(s).is_some_and(|(h, q)| h == headers && same_question(&q, &want))
    };
    // To question 1 (Left stops there). Navigation keys only; none of them chooses anything.
    gated(screen, gate, pane_id, Some(&exact(shown.clone())))?;
    screen.send_named_keys(&vec!["Left".to_string(); n - 1], pane_id)?;
    screen.pause(200);
    let Some(first) = now()? else { return Err(stuck("question 1")) };
    let mut questions = vec![first];
    for _ in 1..n {
        let last = questions[questions.len() - 1].clone();
        gated(screen, gate, pane_id, Some(&exact(last.clone())))?;
        screen.send_named_keys(&["Right".to_string()], pane_id)?;
        let mut next: Option<Question> = None;
        let _ = wait_screen(screen, pane_id, 2000, |s| match claude_screen(s) {
            Some((_, r)) if r.text != last.text => {
                next = Some(r);
                true
            }
            _ => false,
        })?;
        let Some(q) = next else { return Err(stuck(format!("reading question {}", questions.len() + 1))) };
        questions.push(q);
    }
    // Back to where the picker was.
    let back = n - 1 - questions.iter().position(|x| x.text == shown.text).unwrap_or(0);
    if back > 0 {
        gated(screen, gate, pane_id, Some(&exact(questions[questions.len() - 1].clone())))?;
        screen.send_named_keys(&vec!["Left".to_string(); back], pane_id)?;
    }
    if !wait_screen(screen, pane_id, 2000, |s| claude_screen(s).is_some_and(|(_, q)| q.text == shown.text))? {
        return Err(stuck("back to the question on screen"));
    }
    let full = screen_request(&headers, questions);
    if let Some(expect) = expect {
        if !same_request(&full, expect) {
            return Err(not_on_screen());
        }
    }
    Ok(Some(full))
}


#[cfg(test)]
mod tests;
