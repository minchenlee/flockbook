//! The parse half of Swift's `QuestionAdapter` (+Agy): question cards from tool calls, and their state from results.
//! The screen and key half lives in `question_adapter`.

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

use super::json::{self, Obj};
use super::text;
use crate::models::{Delivery, Question, QuestionOption, QuestionRequest, QuestionState};

// MARK: Claude Code AskUserQuestion

pub const RECOMMENDED_SUFFIXES: [&str; 6] = ["(Recommended)", "（Recommended）", "（建議）", "(建議)", "（推薦）", "(推薦)"];

/// Codex's own last picker row; picking it with notes is how free text is given.
pub const CODEX_OTHER_LABEL: &str = "None of the above";

pub fn strip_recommended(raw: &str) -> (String, bool) {
    let label = text::trim_ws(raw);
    for suf in RECOMMENDED_SUFFIXES {
        if text::has_suffix(label, suf) {
            return (text::trim_ws(&label[..label.len() - suf.len()]).to_string(), true);
        }
    }
    (label.to_string(), false)
}

fn option(o: &Obj) -> QuestionOption {
    let (label, rec) = strip_recommended(json::str_(o.get("label")).unwrap_or(""));
    QuestionOption::new(&label, json::str_(o.get("description")).unwrap_or(""), rec)
}

pub fn claude(id: &str, input: &Obj) -> Option<QuestionRequest> {
    let qs = json::objs(input.get("questions"))?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .iter()
        .map(|q| {
            let opts = json::objs(q.get("options")).unwrap_or_default().into_iter().map(option).collect();
            Question::new(
                None,
                json::str_(q.get("header")).unwrap_or(""),
                json::str_(q.get("question")).unwrap_or(""),
                opts,
                json::bool_(q.get("multiSelect")).unwrap_or(false),
            )
        })
        .collect();
    Some(QuestionRequest::new(id, "claude", Delivery::Picker, questions))
}

fn pair_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r#"(?R)"((?:[^"\\]|\\.)*)"="((?:[^"\\]|\\.)*)""#).unwrap())
}

/// Answers come from the line's `toolUseResult.answers` (question text -> answer), else from the result text
/// (`"question"="answer", …`). An error result is a rejection.
pub fn claude_state(q: &QuestionRequest, is_error: bool, result: Option<&Value>, text: &str) -> QuestionState {
    if is_error {
        return QuestionState::Declined;
    }
    let mut by_question: HashMap<String, String> = HashMap::new();
    let given = json::obj(result).and_then(|r| {
        let a = json::obj(r.get("answers"))?;
        a.iter().map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect::<Option<HashMap<_, _>>>()
    });
    if let Some(a) = given {
        by_question = a;
    } else {
        let un = |s: &str| s.replace("\\\"", "\"").replace("\\\\", "\\");
        for m in pair_re().captures_iter(text) {
            by_question.insert(un(&m[1]), un(&m[2]));
        }
    }
    answered(q.questions.iter().map(|x| by_question.get(&x.id).cloned()).collect())
}

// MARK: Codex request_user_input / request_user_input_async

pub const CODEX_TOOLS: [&str; 2] = ["request_user_input", "request_user_input_async"];

/// `request_user_input` (Plan mode picker): `{questions:[{id, header, question, options:[{label, description}]}]}`.
/// `request_user_input_async`: `{questions:[{title, options:[String]}]}`; it returns at once, the user answers in a message.
pub fn codex(name: &str, call_id: &str, arguments: &str) -> Option<QuestionRequest> {
    if !CODEX_TOOLS.contains(&name) {
        return None;
    }
    let o = json::parse_obj(arguments)?;
    let qs = json::objs(o.get("questions"))?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .iter()
        .enumerate()
        .map(|(k, q)| {
            let opts = json::arr(q.get("options"))
                .map(|a| a.as_slice())
                .unwrap_or(&[])
                .iter()
                .filter_map(|o| {
                    if let Some(s) = o.as_str() {
                        let (l, r) = strip_recommended(s);
                        return Some(QuestionOption::new(&l, "", r));
                    }
                    o.as_object().map(option)
                })
                .collect();
            let text = json::str_(q.get("question")).or(json::str_(q.get("title"))).unwrap_or("");
            let id = json::str_(q.get("id")).map(str::to_string).unwrap_or_else(|| format!("q{k}"));
            let multi = json::bool_(q.get("multiSelect")).or(json::bool_(q.get("multi_select"))).unwrap_or(false);
            Question::new(Some(&id), json::str_(q.get("header")).unwrap_or(""), text, opts, multi)
        })
        .collect();
    let delivery = if name == "request_user_input" { Delivery::Picker } else { Delivery::Message };
    Some(QuestionRequest::new(call_id, "codex", delivery, questions))
}

/// Codex picker output: `{"answers":{"<id>":{"answers":["Blue"]}}}`; the notes come as `"user_note: …"`.
/// Anything else (e.g. "unavailable in Default mode", an interrupt) means no answer. The async tool's
/// `{"accepted":true}` returns nil: the question stays pending until the next user message.
pub fn codex_state(q: &QuestionRequest, output: &str) -> Option<QuestionState> {
    let o = json::parse_obj(output);
    if q.delivery == Delivery::Message && o.as_ref().and_then(|o| json::bool_(o.get("accepted"))) == Some(true) {
        return None;
    }
    let Some(by_id) = o.as_ref().and_then(|o| json::obj(o.get("answers"))) else { return Some(QuestionState::Declined) };
    Some(answered(
        q.questions
            .iter()
            .map(|question| {
                let list = json::strs(json::obj(by_id.get(&question.id))?.get("answers"))?;
                let notes: Vec<&str> =
                    list.iter().filter(|s| text::has_prefix(s, "user_note: ")).map(|s| text::drop_first(s, 11)).collect();
                let mut picks: Vec<&str> = list.iter().filter(|s| !text::has_prefix(s, "user_note: ")).copied().collect();
                if !notes.is_empty() {
                    picks.retain(|p| *p != CODEX_OTHER_LABEL);
                }
                picks.extend(notes);
                Some(picks.join(", "))
            })
            .collect(),
    ))
}

// MARK: Grok Build ask_user_question

/// `{questions:[{question, options:[{label, description}], multi_select?}]}`. Grok Build logs the call while its
/// picker is open, and the result when it is answered.
pub fn grok(call_id: &str, input: &Obj) -> Option<QuestionRequest> {
    let qs = json::objs(input.get("questions"))?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .iter()
        .map(|q| {
            let opts = json::objs(q.get("options")).unwrap_or_default().into_iter().map(option).collect();
            let multi = json::bool_(q.get("multi_select")).or(json::bool_(q.get("multiSelect"))).unwrap_or(false);
            Question::new(None, "", json::str_(q.get("question")).unwrap_or(""), opts, multi)
        })
        .collect();
    Some(QuestionRequest::new(call_id, "grok", Delivery::Picker, questions))
}

/// Grok Build 1.0.67's result: `rawOutput.UserAnswered.answers[{question, picked:[label], typed?}]`, else the
/// result text (`"question"="answer", …`). A failed call or any other result (dismissed) is no answer.
pub fn grok_state(q: &QuestionRequest, failed: bool, output: Option<&Value>, text: &str) -> QuestionState {
    if failed {
        return QuestionState::Declined;
    }
    let list = json::obj(output).and_then(|o| json::obj(o.get("UserAnswered"))).and_then(|u| json::objs(u.get("answers")));
    if let Some(list) = list {
        return answered(
            q.questions
                .iter()
                .map(|question| {
                    let a = list.iter().find(|a| json::str_(a.get("question")) == Some(question.text.as_str()))?;
                    let typed = json::str_(a.get("typed")).map(text::trim).unwrap_or("");
                    let mut parts: Vec<&str> = json::strs(a.get("picked")).unwrap_or_default();
                    if !typed.is_empty() {
                        parts.push(typed);
                    }
                    Some(parts.join(", "))
                })
                .collect(),
        );
    }
    if text::has_prefix(text, "User has answered") {
        return claude_state(q, false, None, text);
    }
    QuestionState::Declined
}

pub fn answered(answers: Vec<Option<String>>) -> QuestionState {
    if !answers.iter().any(Option::is_some) {
        return QuestionState::Answered(None);
    }
    QuestionState::Answered(Some(answers.into_iter().map(Option::unwrap_or_default).collect()))
}

// MARK: agy ask_question

/// `{questions:[{question, options:[String], is_multi_select}]}`. Labels are kept as agy shows them.
pub fn agy(id: &str, input: &Obj) -> Option<QuestionRequest> {
    let qs = json::objs(input.get("questions"))?;
    if qs.is_empty() {
        return None;
    }
    let questions = qs
        .iter()
        .enumerate()
        .map(|(k, q)| {
            let opts = json::arr(q.get("options"))
                .map(|a| a.as_slice())
                .unwrap_or(&[])
                .iter()
                .filter_map(Value::as_str)
                .map(|l| QuestionOption::new(l, "", false))
                .collect();
            Question::new(
                Some(&format!("q{k}")),
                "",
                json::str_(q.get("question")).unwrap_or(""),
                opts,
                json::bool_(q.get("is_multi_select")).unwrap_or(false),
            )
        })
        .collect();
    Some(QuestionRequest::new(id, "agy", Delivery::Picker, questions))
}

fn agy_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?mR)^A(\d+): (.*)$").unwrap())
}

/// Result lines `A1: Blue`, `A2: …` (one per question). `User Skipped` everywhere (Esc) is a decline.
pub fn agy_state(q: &QuestionRequest, text: &str) -> QuestionState {
    let mut by_index: HashMap<i64, String> = HashMap::new();
    for m in agy_re().captures_iter(text) {
        if let Ok(i) = m[1].parse::<i64>() {
            by_index.insert(i - 1, text::trim_ws(&m[2]).to_string());
        }
    }
    if !by_index.is_empty() && by_index.values().all(|v| v == "User Skipped") {
        return QuestionState::Declined;
    }
    answered(
        (0..q.questions.len() as i64)
            .map(|i| by_index.get(&i).filter(|v| v.as_str() != "User Skipped").cloned())
            .collect(),
    )
}
