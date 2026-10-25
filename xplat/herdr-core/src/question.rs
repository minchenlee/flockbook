//! Everything that differs per agent for question cards (port of QuestionAdapter.swift, pure parts):
//! (a) parsing the tool call and its result, (b) the screen markers and key sequence that answer it.
//! The pane I/O half of the Swift file (`answerQuestion`, `pendingQuestion`, ...) sends keys and is out of this gate.

use crate::model::*;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

const RECOMMENDED_SUFFIXES: [&str; 6] = ["(Recommended)", "（Recommended）", "（建議）", "(建議)", "（推薦）", "(推薦)"];

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("static regex"))
}

pub(crate) fn strip_recommended(raw: &str) -> (String, bool) {
    let label = raw.trim_matches(|c| c == ' ' || c == '\t');
    for suf in RECOMMENDED_SUFFIXES {
        if let Some(rest) = label.strip_suffix(suf) {
            return (rest.trim_matches(|c| c == ' ' || c == '\t').to_string(), true);
        }
    }
    (label.to_string(), false)
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

// MARK: (a) Claude Code AskUserQuestion

pub(crate) fn claude(id: &str, input: &Map<String, Value>) -> Option<QuestionRequest> {
    let qs = input.get("questions")?.as_array()?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .iter()
        .map(|q| {
            let options = q
                .get("options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|o| {
                            let (label, rec) = strip_recommended(str_of(o, "label"));
                            QOption { label, detail: str_of(o, "description").into(), recommended: rec }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let text = str_of(q, "question").to_string();
            Question {
                id: text.clone(),
                header: str_of(q, "header").into(),
                text,
                options,
                multi_select: q.get("multiSelect").and_then(Value::as_bool).unwrap_or(false),
            }
        })
        .collect();
    Some(QuestionRequest::new(id, questions))
}

/// Answers come from the line's `toolUseResult.answers` (question text -> answer), else from the result text
/// (`"question"="answer", …`). An error result is a rejection.
pub(crate) fn claude_state(q: &QuestionRequest, is_error: bool, result: Option<&Value>, text: &str) -> QuestionState {
    if is_error {
        return QuestionState::Declined;
    }
    let mut by_question: HashMap<String, String> = HashMap::new();
    let from_result: Option<HashMap<String, String>> = result
        .and_then(|r| r.get("answers"))
        .and_then(Value::as_object)
        .and_then(|a| a.iter().map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect());
    if let Some(a) = from_result {
        by_question = a;
    } else {
        static R: OnceLock<Regex> = OnceLock::new();
        let un = |s: &str| s.replace("\\\"", "\"").replace("\\\\", "\\");
        for m in re(&R, r#""((?:[^"\\]|\\.)*)"="((?:[^"\\]|\\.)*)""#).captures_iter(text) {
            by_question.insert(un(&m[1]), un(&m[2]));
        }
    }
    answered(q.questions.iter().map(|x| by_question.get(&x.id).cloned()).collect())
}

// MARK: (a) Codex request_user_input / request_user_input_async

const CODEX_TOOLS: [&str; 2] = ["request_user_input", "request_user_input_async"];

/// `request_user_input` (Plan mode picker): `{questions:[{id, header, question, options:[{label, description}]}]}`.
/// `request_user_input_async`: `{questions:[{title, options:[String]}]}`; it returns at once, the user answers in a message.
pub(crate) fn codex(name: &str, call_id: &str, arguments: &str) -> Option<QuestionRequest> {
    if !CODEX_TOOLS.contains(&name) {
        return None;
    }
    let o: Value = serde_json::from_str(arguments).ok()?;
    let qs = o.get("questions")?.as_array()?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .iter()
        .enumerate()
        .map(|(k, q)| {
            let options = q
                .get("options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|o| {
                            if let Some(s) = o.as_str() {
                                let (label, recommended) = strip_recommended(s);
                                return Some(QOption { label, detail: String::new(), recommended });
                            }
                            o.as_object()?;
                            let (label, recommended) = strip_recommended(str_of(o, "label"));
                            Some(QOption { label, detail: str_of(o, "description").into(), recommended })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let text = q.get("question").and_then(Value::as_str).or_else(|| q.get("title").and_then(Value::as_str)).unwrap_or("");
            Question {
                id: q.get("id").and_then(Value::as_str).map(String::from).unwrap_or_else(|| format!("q{k}")),
                header: str_of(q, "header").into(),
                text: text.into(),
                options,
                multi_select: q
                    .get("multiSelect")
                    .and_then(Value::as_bool)
                    .or_else(|| q.get("multi_select").and_then(Value::as_bool))
                    .unwrap_or(false),
            }
        })
        .collect();
    let mut r = QuestionRequest::new(call_id, questions);
    r.agent = "codex".into();
    r.delivery = if name == "request_user_input" { Delivery::Picker } else { Delivery::Message };
    Some(r)
}

/// Codex's own last picker row; picking it with notes is how free text is given.
const CODEX_OTHER_LABEL: &str = "None of the above";

/// Codex picker output: `{"answers":{"<id>":{"answers":["Blue"]}}}`; the notes come as `"user_note: …"`.
/// Anything else means no answer. The async tool's `{"accepted":true}` returns `None`: the question stays
/// pending until the next user message.
pub(crate) fn codex_state(q: &QuestionRequest, output: &str) -> Option<QuestionState> {
    let o: Option<Value> = serde_json::from_str(output).ok();
    if q.delivery == Delivery::Message && o.as_ref().and_then(|o| o.get("accepted")).and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let Some(by_id) = o.as_ref().and_then(|o| o.get("answers")).and_then(Value::as_object) else {
        return Some(QuestionState::Declined);
    };
    let answers = q
        .questions
        .iter()
        .map(|question| {
            let list: Vec<String> = by_id
                .get(&question.id)?
                .get("answers")?
                .as_array()?
                .iter()
                .map(|v| v.as_str().map(String::from))
                .collect::<Option<_>>()?;
            let notes: Vec<String> = list.iter().filter_map(|s| s.strip_prefix("user_note: ").map(String::from)).collect();
            let mut picks: Vec<String> = list.iter().filter(|s| !s.starts_with("user_note: ")).cloned().collect();
            if !notes.is_empty() {
                picks.retain(|p| p != CODEX_OTHER_LABEL);
            }
            picks.extend(notes);
            Some(picks.join(", "))
        })
        .collect();
    Some(answered(answers))
}

pub(crate) fn answered(answers: Vec<Option<String>>) -> QuestionState {
    if answers.iter().all(Option::is_none) {
        return QuestionState::Answered(None);
    }
    QuestionState::Answered(Some(answers.into_iter().map(Option::unwrap_or_default).collect()))
}

// MARK: (b) Answer: screen markers and keys

/// Text that is on screen only while the agent's question picker is open.
pub fn markers(agent: &str) -> &'static [&'static str] {
    if agent == "grok" {
        &["Shift+x:dismiss"]
    } else if agent == "codex" {
        &["enter to submit"]
    } else {
        &["Chat about this", "Review your answers"]
    }
}

/// One step of an answer. `WaitFor` blocks until the picker (not the rest of the screen) shows the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Text(String),
    Keys(Vec<String>),
    WaitFor(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// The pane does not show this question's picker: nothing was sent.
    NotOnScreen,
    /// This answer cannot be given with keys. Nothing was sent.
    Unsupported(String),
    /// Keys were sent, but the expected screen did not come.
    Stuck(String),
    /// All keys were sent, but the picker is still on screen.
    StillOnScreen,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::NotOnScreen => write!(f, "the question is not on screen"),
            Failure::Unsupported(m) => write!(f, "cannot answer: {m}"),
            Failure::Stuck(m) => write!(f, "stopped waiting for: {m}"),
            Failure::StillOnScreen => write!(f, "the question is still on screen after answering"),
        }
    }
}
impl std::error::Error for Failure {}

/// Screen text with lines trimmed and joined, so a wrapped question still matches.
pub(crate) fn flat(s: &str) -> String {
    let joined = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Start of a question's text (short, so a wrap inside CJK text does not break the match).
fn needle(q: &Question) -> String {
    flat(&q.text).chars().take(24).collect::<String>().trim().to_string()
}

fn is_tab_bar(l: &str) -> bool {
    let t = l.trim();
    t.starts_with('☐') || t.starts_with('☒') || t.starts_with('✔') || t.starts_with('←')
}

/// The picker part of the screen only, flattened: Claude from its tab bar down, Codex from its "Question k/n" line
/// down. Text above it (the user's own prompt often repeats the question) must not count. `None` = no picker.
pub fn picker_region(agent: &str, screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.split('\n').collect();
    if agent == "grok" { return grok_region(screen); }
    let start = if agent == "codex" {
        static R: OnceLock<Regex> = OnceLock::new();
        let r = re(&R, r"^\s*Question \d+/\d+");
        lines.iter().rposition(|l| r.is_match(l))
    } else {
        lines.iter().rposition(|l| is_tab_bar(l))
    }?;
    let region = flat(&lines[start..].join("\n"));
    markers(agent).iter().any(|m| region.contains(m)).then_some(region)
}

pub fn picker_shown(q: &QuestionRequest, screen: &str) -> bool {
    picker_region(&q.agent, screen).is_some()
}

/// The picker is open and shows the first question.
pub fn screen_shows(q: &QuestionRequest, screen: &str) -> bool {
    if q.agent == "grok" { return grok_exact(q, 0, screen); }
    let Some(first) = q.questions.first() else { return false };
    picker_region(&q.agent, screen).is_some_and(|r| r.contains(&needle(first)))
}

fn other_text(a: &QuestionAnswer) -> Option<String> {
    let s = a.other.as_deref().unwrap_or("").trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// The key sequence for a picker question. `Unsupported` is returned before anything is sent.
pub fn steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Result<Vec<Step>, Failure> {
    if answers.len() != q.questions.len() {
        return Err(Failure::Unsupported(format!("{} answers for {} questions", answers.len(), q.questions.len())));
    }
    for (k, a) in answers.iter().enumerate() {
        let n = q.questions[k].options.len();
        if !a.picks.iter().all(|p| *p < n) || n > 8 {
            return Err(Failure::Unsupported(format!("question {}: option out of range", k + 1)));
        }
        let other = a.other.as_deref().unwrap_or("").trim();
        if other.chars().any(char::is_control) {
            return Err(Failure::Unsupported(format!("question {}: line break in the text", k + 1)));
        }
        if a.picks.is_empty() && other.is_empty() {
            return Err(Failure::Unsupported(format!("question {}: no answer", k + 1)));
        }
        if !q.questions[k].multi_select && a.picks.len() > 1 {
            return Err(Failure::Unsupported(format!("question {}: one choice only", k + 1)));
        }
    }
    if q.agent == "grok" {
        grok_steps(q, answers)
    } else if q.agent == "codex" {
        codex_steps(q, answers)
    } else {
        Ok(claude_steps(q, answers))
    }
}

fn keys(k: &[&str]) -> Step {
    Step::Keys(k.iter().map(|s| s.to_string()).collect())
}
fn downs(n: usize) -> Vec<String> {
    vec!["Down".to_string(); n]
}

/// Claude Code 2.1.296. Single choice: digit picks and moves on ("Other" = digit n+1, text, Enter).
/// multiSelect: digits toggle; "Other" = digit n+1, Down to the row, text, Tab, Enter; without it Tab moves on.
/// After the last question of a multi-question or multiSelect card: review screen, `1` submits.
fn claude_steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Vec<Step> {
    let mut out = Vec::new();
    for (k, question) in q.questions.iter().enumerate() {
        let a = &answers[k];
        let n = question.options.len();
        if !question.multi_select {
            if let Some(other) = other_text(a) {
                out.extend([Step::Text(format!("{}", n + 1)), Step::Text(other), keys(&["Enter"])]);
            } else {
                out.push(Step::Text(format!("{}", a.picks[0] + 1)));
            }
        } else {
            let mut picks = a.picks.clone();
            picks.sort_unstable();
            out.extend(picks.iter().map(|p| Step::Text(format!("{}", p + 1))));
            if let Some(other) = other_text(a) {
                out.extend([Step::Text(format!("{}", n + 1)), Step::Keys(downs(n)), Step::Text(other), keys(&["Tab"]), keys(&["Enter"])]);
            } else {
                out.push(keys(&["Tab"]));
            }
        }
        if let Some(next) = q.questions.get(k + 1) {
            out.push(Step::WaitFor(needle(next)));
        }
    }
    if q.questions.len() > 1 || q.questions.last().is_some_and(|x| x.multi_select) {
        out.extend([Step::WaitFor("Review your answers".into()), Step::Text("1".into())]);
    }
    out
}

/// Codex 0.162 (Plan mode picker). Digit picks and moves on; on the last question it submits all.
/// Free text: Down to "None of the above" (row n+1), Tab opens the notes, text, Enter. No multiSelect.
fn codex_steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Result<Vec<Step>, Failure> {
    let mut out = Vec::new();
    for (k, question) in q.questions.iter().enumerate() {
        if question.multi_select {
            return Err(Failure::Unsupported("Codex has no multiSelect picker".into()));
        }
        let a = &answers[k];
        let n = question.options.len();
        if let Some(other) = other_text(a) {
            let mut ks = downs(a.picks.first().copied().unwrap_or(n));
            ks.push("Tab".into());
            out.extend([Step::Keys(ks), Step::Text(other), keys(&["Enter"])]);
        } else {
            out.push(Step::Text(format!("{}", a.picks[0] + 1)));
        }
        if let Some(next) = q.questions.get(k + 1) {
            out.push(Step::WaitFor(needle(next)));
        }
    }
    Ok(out)
}

/// `Message` delivery: the text sent as a user message. One question: the answer; several: "question answer" lines.
pub fn message_text(q: &QuestionRequest, answers: &[QuestionAnswer]) -> String {
    q.questions
        .iter()
        .zip(answers)
        .map(|(question, a)| {
            let mut picks = a.picks.clone();
            picks.sort_unstable();
            let mut parts: Vec<String> = picks.iter().map(|i| question.options[*i].label.clone()).collect();
            parts.extend(other_text(a));
            let v = parts.join(", ");
            if q.questions.len() == 1 { v } else { format!("{} {v}", question.text) }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// MARK: Claude Code: the pending question comes from the screen

/// A screen-read request. Headers go on the questions when all are read; the id depends only on the tab bar
/// (plus the text for a single question), so the read-only and the full read give the same id.
pub fn screen_request(headers: Vec<String>, mut questions: Vec<Question>) -> QuestionRequest {
    if questions.len() == headers.len() {
        for (q, h) in questions.iter_mut().zip(&headers) {
            q.header = h.clone();
        }
    }
    let mut key = headers.join("|");
    if headers.len() == 1 {
        key.push('|');
        key.push_str(questions.first().map(|q| q.text.as_str()).unwrap_or(""));
    }
    // djb2 over UTF-8 bytes, 32-bit wrapping: stable across calls and runs (same as the Swift version).
    let hash = key.bytes().fold(5381u32, |h, b| h.wrapping_shl(5).wrapping_add(h).wrapping_add(b as u32));
    let mut r = QuestionRequest::new(&format!("screen-{hash:x}"), questions);
    r.headers = headers;
    r
}

/// Claude Code 2.1.296 writes the AskUserQuestion call to its log only after it is answered, so while the
/// picker is open the question exists only on screen. This reads the visible question: the tab-bar headers
/// (one per question) and the current question with its options. `None` when no Claude picker is shown.
pub struct ScreenQuestion {
    pub headers: Vec<String>,
    pub question: Question,
}

pub fn claude_screen(screen: &str) -> Option<ScreenQuestion> {
    let lines: Vec<&str> = screen.split('\n').collect();
    let end = lines.iter().rposition(|l| l.contains("Chat about this"))?;
    let bar = lines[..end].iter().rposition(|l| is_tab_bar(l))?;
    let headers: Vec<String> = lines[bar]
        .split(|c| "☐☒✔←→".contains(c))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "Submit")
        .collect();
    static OPT: OnceLock<Regex> = OnceLock::new();
    let opt_re = re(&OPT, r"^\s*(?:❯\s*)?(\d+)\.\s+(\[[ ✔]\]\s+)?(.*)$");
    let (mut text, mut options, mut multi, mut in_options) = (Vec::<String>::new(), Vec::<QOption>::new(), false, false);
    for l in &lines[bar + 1..end] {
        let t = l.trim();
        if t.starts_with("──") {
            break;
        }
        if let Some(m) = opt_re.captures(l) {
            let label = m[3].trim();
            if label.starts_with("Type something") {
                break;
            }
            in_options = true;
            if m.get(2).is_some() {
                multi = true;
            }
            let (clean, rec) = strip_recommended(label);
            options.push(QOption { label: clean, detail: String::new(), recommended: rec });
        } else if in_options {
            if !t.is_empty() {
                if let Some(last) = options.last_mut() {
                    if !last.detail.is_empty() {
                        last.detail.push(' ');
                    }
                    last.detail.push_str(t);
                }
            }
        } else if !t.is_empty() {
            text.push(t.to_string());
        }
    }
    if options.is_empty() || text.is_empty() {
        return None;
    }
    let q = text.join(" ");
    let header = if headers.len() == 1 { headers[0].clone() } else { String::new() };
    Some(ScreenQuestion {
        headers,
        question: Question { id: q.clone(), header, text: q, options, multi_select: multi },
    })
}

pub(crate) fn grok(id: &str, input: &Map<String, Value>) -> Option<QuestionRequest> {
    let mut q = claude(id, input)?;
    q.agent = "grok".into();
    for (question, raw) in q.questions.iter_mut().zip(input.get("questions")?.as_array()?) {
        question.multi_select = raw.get("multi_select").and_then(Value::as_bool).unwrap_or(question.multi_select);
    }
    Some(q)
}

pub(crate) fn grok_state(q: &QuestionRequest, failed: bool, output: Option<&Value>, text: &str) -> QuestionState {
    if failed { return QuestionState::Declined; }
    if let Some(list) = output.and_then(|v| v.pointer("/UserAnswered/answers")).and_then(Value::as_array) {
        return answered(q.questions.iter().map(|question| {
            let a = list.iter().find(|a| a.get("question").and_then(Value::as_str) == Some(question.text.as_str()))?;
            let mut parts: Vec<&str> = a.get("picked").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
            if let Some(t) = a.get("typed").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) { parts.push(t.trim()); }
            Some(parts.join(", "))
        }).collect());
    }
    if text.starts_with("User has answered") { claude_state(q, false, None, text) } else { QuestionState::Declined }
}

fn grok_steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Result<Vec<Step>, Failure> {
    let mut out = Vec::new();
    for (k, (question, a)) in q.questions.iter().zip(answers).enumerate() {
        if question.multi_select || other_text(a).is_some() {
            return Err(Failure::Unsupported("Grok free text and multi-choice require the terminal".into()));
        }
        out.push(Step::Text((a.picks[0] + 1).to_string()));
        if k + 1 < q.questions.len() { out.push(Step::WaitFor(needle(&q.questions[k + 1]))); }
    }
    Ok(out)
}

fn grok_body(screen: &str) -> Option<Vec<String>> {
    let lines: Vec<_> = screen.lines().collect();
    let end = lines.iter().rposition(|l| l.trim_start().starts_with('┃'))?;
    let mut start = end;
    while start > 0 && lines[start - 1].trim_start().starts_with('┃') { start -= 1; }
    Some(lines[start..=end].iter().map(|l| l.trim_start().trim_start_matches('┃').trim_end_matches(|c: char| c == '█' || c.is_whitespace()).trim().to_string()).filter(|l| !l.is_empty()).collect())
}
fn grok_region(screen: &str) -> Option<String> {
    let body = grok_body(screen)?.join("\n");
    body.contains("Shift+x:dismiss").then(|| flat(&body))
}
/// Exact untouched Grok picker. A selected row, edited field or changed option disables input.
pub fn grok_exact(q: &QuestionRequest, k: usize, screen: &str) -> bool {
    if q.agent != "grok" || q.tool_use_id.starts_with("screen-") { return false; }
    let Some(want) = q.questions.get(k).filter(|q| !q.multi_select) else { return false };
    let Some(body) = grok_body(screen) else { return false };
    if grok_region(screen).is_none() { return false; }
    let bare = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let Some(first) = body.iter().position(|l| l.starts_with("1 (○) ")) else { return false };
    if first == 0 || body.len() != first + want.options.len() + 2 || bare(&body[..first].join("")) != bare(&want.text) { return false; }
    for (i, o) in want.options.iter().enumerate() {
        let Some(label) = body[first + i].strip_prefix(&format!("{} (○) ", i + 1)) else { return false };
        let labels = std::iter::once(o.label.clone()).chain(RECOMMENDED_SUFFIXES.iter().filter(|_| o.recommended).map(|s| format!("{}{s}", o.label)));
        if !labels.into_iter().any(|l| bare(label) == bare(&format!("{l}{}", o.detail))) { return false; }
    }
    if body[first + want.options.len()] != "z (○) Type your answer here" { return false; }
    let foot = body.last().unwrap();
    let count = foot.starts_with(&format!("[{}/{}] ", k + 1, q.questions.len())) || (q.questions.len() == 1 && !foot.starts_with('['));
    count && foot.ends_with(if k + 1 == q.questions.len() { "Enter:submit" } else { "Enter:select" })
}

#[cfg(test)]
mod grok_tests {
    use super::*;
    #[test]
    fn grok_picker_requires_exact_untouched_rows() {
        let mut q = QuestionRequest::new("q", vec![Question::new("", "Pick?", vec![QOption::new("A")], false)]);
        q.agent = "grok".into();
        let screen = "┃ Pick?\n┃ 1 (○) A\n┃ z (○) Type your answer here\n┃ Shift+x:dismiss Enter:submit";
        assert!(grok_exact(&q, 0, screen));
        assert!(!grok_exact(&q, 0, &screen.replace("○) A", "●) A")));
        assert!(!grok_exact(&q, 0, &screen.replace("Pick?", "Changed?")));
    }
}

/// Revalidated before every mutating step. Grok requires its exact untouched layout.
pub fn question_on_screen(q: &QuestionRequest, k: usize, screen: &str) -> bool {
    if q.agent == "grok" { return grok_exact(q, k, screen); }
    let Some(want) = q.questions.get(k) else { return false };
    let Some(region) = picker_region(&q.agent, screen) else { return false };
    region.contains(&flat(&want.text)) && want.options.iter().all(|o| region.contains(&flat(&o.label)))
}

pub fn review_on_screen(q: &QuestionRequest, answers: &[QuestionAnswer], screen: &str) -> bool {
    if q.agent != "claude" || answers.len() != q.questions.len() { return false; }
    let Some(region) = picker_region(&q.agent, screen) else { return false };
    region.contains("Review your answers") && q.headers.iter().all(|h| region.contains(h))
        && q.questions.iter().zip(answers).all(|(question, a)| {
            region.contains(&flat(&question.text)) && a.picks.iter().all(|i| question.options.get(*i).is_some_and(|o| region.contains(&flat(&o.label))))
                && a.other.as_ref().is_none_or(|s| region.contains(&flat(s)))
        })
}
