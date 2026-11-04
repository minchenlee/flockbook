//! Swift: `QuestionAdapter+Agy.swift`. agy's `ask_question` (agy 1.4.12, live): the call's args name the
//! questions; the picker is answered with keys.

use super::*;

/// `{questions:[{question, options:[String], is_multi_select}]}`. Labels are kept as agy shows them.
pub fn agy(id: &str, input: &Map<String, Value>) -> Option<QuestionRequest> {
    let qs = as_objects(input.get("questions"))?;
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
                .filter_map(Value::as_str)
                .map(|l| option(l, "", false))
                .collect();
            new_question(
                Some(&format!("q{k}")),
                "",
                as_str(q.get("question")).unwrap_or(""),
                opts,
                as_bool(q.get("is_multi_select")).unwrap_or(false),
            )
        })
        .collect();
    Some(new_request(id, "agy", Delivery::Picker, questions))
}

re!(agy_answer_re, r"(?mR)^A(\d+): (.*)$");

/// Result lines `A1: Blue`, `A2: …` (one per question). `User Skipped` everywhere (Esc) is a decline.
pub fn agy_state(q: &QuestionRequest, text: &str) -> QuestionState {
    let mut by_index: HashMap<i64, String> = HashMap::new();
    for m in agy_answer_re().captures_iter(text) {
        if let Some(i) = int(&m[1]) {
            by_index.insert(i - 1, trim_ws(&m[2]).to_string());
        }
    }
    if !by_index.is_empty() && by_index.values().all(|v| v == "User Skipped") {
        return QuestionState::Declined;
    }
    answered(
        (0..q.questions.len() as i64)
            .map(|i| by_index.get(&i).filter(|v| *v != "User Skipped").cloned())
            .collect(),
    )
}

/// Verified for one single-choice question: the digit picks and submits (no Enter). Write-in and multi-select are
/// not verified, and neither are several questions in a row, so they are refused before anything is sent.
pub(super) fn agy_steps(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Result<Vec<Step>, HerdrError> {
    if q.questions.len() > 1 {
        return Err(unsupported("agy questions in a row are not supported yet: answer in the terminal"));
    }
    let mut out = vec![];
    for (k, question) in q.questions.iter().enumerate() {
        if question.multi_select {
            return Err(unsupported("agy multi-select is not supported yet: answer in the terminal"));
        }
        if other_text(&answers[k]).is_some() {
            return Err(unsupported("agy write-in is not supported yet: answer in the terminal"));
        }
        out.push(Step::Text(format!("{}", answers[k].picks[0] + 1)));
        if k + 1 < q.questions.len() {
            out.push(Step::WaitFor(needle(&q.questions[k + 1])));
        }
    }
    Ok(out)
}

re!(agy_head_re, r"^Question \d+/\d+: ");
re!(agy_head_whole_re, r"(?R)^Question (\d+)/(\d+): (.*)$");
re!(agy_row_re, r"(?R)^(>\s*)?(\d+)\.\s+(.*)$");

/// The picker shows question `k` exactly: "Question k/n: <text>" (it may wrap), the options as rows "1. …" in
/// order, then agy's own "Write-in..." row, one ">" cursor, and the key line below. Single choice only.
pub fn agy_exact(q: &QuestionRequest, k: usize, screen: &str) -> bool {
    if !(q.agent == "agy" && k < q.questions.len() && !q.questions[k].multi_select) {
        return false;
    }
    let want = &q.questions[k];
    let lines: Vec<&str> = screen.split('\n').map(trim_ws).collect();
    let Some(head) = lines.iter().rposition(|l| agy_head_re().is_match(l)) else { return false };
    let Some(m) = agy_head_whole_re().captures(lines[head]) else { return false };
    if int(&m[1]) != Some(k as i64 + 1) || int(&m[2]) != Some(q.questions.len() as i64) {
        return false;
    }
    let mut at = head + 1;
    let mut text: Vec<&str> = vec![m.get(3).map_or("", |x| x.as_str())];
    while at < lines.len() && !agy_row_re().is_match(lines[at]) {
        text.push(lines[at]);
        at += 1;
    }
    if bare(&text.concat()) != bare(&want.text) {
        return false;
    }
    let mut rows: Vec<String> = vec![];
    let mut cursors = 0;
    while at < lines.len() {
        let Some(r) = agy_row_re().captures(lines[at]) else { break };
        if int(&r[2]) != Some(rows.len() as i64 + 1) {
            return false;
        }
        if r.get(1).is_some() {
            cursors += 1;
        }
        rows.push(r[3].to_string());
        at += 1;
    }
    let mut expected: Vec<String> = want.options.iter().map(|o| o.label.clone()).collect();
    expected.push("Write-in...".into());
    if !(cursors == 1 && rows == expected) {
        return false;
    }
    let Some(foot) = lines[at..].iter().find(|l| !l.is_empty()) else { return false };
    *foot == "↑/↓ Navigate · enter Select · esc Skip"
}
