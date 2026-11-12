//! Swift: `QuestionAdapter+ClaudeExact.swift`. Claude Code 2.1.296's picker for a request the hub knows from the
//! AskUserQuestion hook (`ClaudeQuestionHook`). Like `codex_exact`: the screen must be the request's own rendering,
//! compared with all whitespace dropped (wrapping does not count). Only the focus mark, the [✔] boxes, the tab-bar
//! boxes and the hub's own Other text may vary, each at a known place. Anything else (a new Claude Code version, the
//! user's own text) refuses: no key.

use super::*;
use std::collections::{BTreeSet, HashMap, HashSet};

/// Claude Code 2.1.296's key lines under a question (spacing aside), as captured live.
pub const CLAUDE_KEY_LINES: [&str; 4] = [
    "Enter to select · ↑/↓ to navigate · Esc to cancel",
    "Enter to select · ↑/↓ to navigate · ctrl+g to edit in Vim · Esc to cancel",
    "Enter to select · Tab/Arrow keys to navigate · Esc to cancel",
    "Enter to select · Tab/Arrow keys to navigate · ctrl+g to edit in Vim · Esc to cancel",
];

re!(editor_re, r"^(Enter to select · (?:↑/↓|Tab/Arrow keys) to navigate · ctrl\+g to edit in )[^·]+?( · Esc to cancel)$");

/// One of Claude's key lines (spacing aside). The "ctrl+g" part names the person's own editor ($VISUAL/$EDITOR):
/// "edit in Vim", "edit in VS Code" (seen live 2026-11-11, 2.1.322) and so on; any name counts as that line.
pub fn claude_key_line(line: &str) -> bool {
    let s = spacing_aside(line);
    if CLAUDE_KEY_LINES.contains(&s.as_str()) {
        return true;
    }
    editor_re().captures(&s).is_some_and(|m| CLAUDE_KEY_LINES.contains(&format!("{}Vim{}", &m[1], &m[2]).as_str()))
}

/// The text with every whitespace and newline scalar dropped.
pub fn bare(s: &str) -> String {
    s.chars().filter(|c| !is_ws_nl(*c)).collect()
}

fn take_prefix(s: &mut &str, lit: &str) -> bool {
    match s.strip_prefix(lit) {
        Some(rest) => {
            *s = rest;
            true
        }
        None => false,
    }
}

/// The tab bar of `q`, whitespace dropped: one question, single choice: "☐Header"; else "←☐A☐B✔Submit→". A
/// question's box is ☐ or ☒ (answered or not: Claude's own state, no key depends on it).
pub fn claude_bar(q: &QuestionRequest, line: &str) -> bool {
    let b = bare(line);
    let mut s: &str = &b;
    let tabs = q.questions.len() > 1 || q.questions.first().is_some_and(|x| x.multi_select);
    if tabs && !take_prefix(&mut s, "←") {
        return false;
    }
    for k in &q.questions {
        if !((take_prefix(&mut s, "☐") || take_prefix(&mut s, "☒")) && take_prefix(&mut s, &bare(&k.header))) {
            return false;
        }
    }
    if tabs && !take_prefix(&mut s, "✔Submit→") {
        return false;
    }
    s.is_empty()
}

/// The label as Claude shows it: the log's label, with its "(Recommended)" when it had one.
pub fn shown_labels(o: &QOption) -> Vec<String> {
    if o.recommended {
        RECOMMENDED_SUFFIXES.iter().map(|s| bare(&format!("{}{}", o.label, s))).collect()
    } else {
        vec![bare(&o.label)]
    }
}

/// Text with whitespace dropped, as grapheme clusters, read from `pos` on (Swift's `[Character]` and `take`).
struct Run {
    chars: Vec<String>,
    pos: usize,
}

impl Run {
    fn has(&self, lit: &str) -> bool {
        let a: Vec<&str> = lit.graphemes(true).collect();
        self.pos + a.len() <= self.chars.len() && self.chars[self.pos..self.pos + a.len()].iter().zip(&a).all(|(x, y)| x == y)
    }
    fn take(&mut self, lit: &str) -> bool {
        if !self.has(lit) {
            return false;
        }
        self.pos += gcount(lit);
        true
    }
}

re!(row_re, r"^(❯ |  )(\d+)\.(?: |$)");

/// The shortest question text Claude Code 2.1.322 draws behind a block-quote bar (it starts at about 90
/// characters). A shorter text that starts with "│" is that text itself.
pub(crate) const QUOTE_MIN: usize = 80;

/// Claude Code 2.1.322 draws a long question (about 90 characters or more) as a block quote: "│ " in front of
/// every line of its text, wrapped lines too. The bar is drawing, not text. `lines`: the picker's lines between
/// the tab bar and the first row. When every line with text there starts with "│", returns them without it;
/// otherwise (a short question, or a "│" that is part of the text) unchanged.
pub fn drop_quote_bar<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let text: Vec<&&str> = lines.iter().filter(|l| !trim_ws(l).is_empty()).collect();
    if text.is_empty() || !text.iter().all(|l| trim_ws(l).starts_with('│')) {
        return lines.to_vec();
    }
    lines.iter().map(|l| { let t = trim_ws(l); if t.is_empty() { *l } else { &t['│'.len_utf8()..] } }).collect()
}
re!(submit_re, r"^(?:❯|\s)\s*Submit\s*$");

/// The rows read so far in `claude_exact`: which region lines are rows (or Submit), and where the current row's
/// line ends.
struct Rows<'a> {
    run: Run,
    starts: HashMap<usize, usize>,
    region: &'a [&'a str],
    row_lines: HashSet<usize>,
    line_end: usize,
}

impl Rows<'_> {
    /// A row line starts here: its mark (taken) or None when it is not row `number`'s line. `line_end`: where
    /// that line ends (the label must end there: the label/description boundary is a line break, never dropped).
    fn row(&mut self, number: usize) -> Option<bool> {
        let l = *self.starts.get(&self.run.pos)?;
        let m = row_re().captures(self.region[l])?;
        if int(&m[2]) != Some(number as i64) {
            return None;
        }
        self.row_lines.insert(l);
        self.line_end = self.run.pos + gcount(&bare(self.region[l]));
        let marked = &m[1] == "❯ ";
        if marked && !self.run.take("❯") {
            return None;
        }
        if !self.run.take(&format!("{number}.")) {
            return None;
        }
        Some(marked)
    }
}

/// The picker on screen is question `k` of the hooked request `q`, exactly. From the bottom: the key line (the
/// last line), "n+2. Chat about this" alone after the separator line, then from the last tab-bar line: the bar,
/// the question text, per row an optional focus mark "❯", "i.", the box (multiSelect), the label and the
/// description, then the Other row ("Type something." / "Type something" while empty, else `other`: the text
/// the hub typed) and, multiSelect, the Submit row. Exactly one focus mark, on a row or Submit; `focus` says
/// which one it must be (`Auto`: from `other`, as in `shows`). `checks`: the [✔] rows (Other = index n), when given.
pub fn claude_exact(
    q: &QuestionRequest,
    k: usize,
    screen: &str,
    other: Option<&str>,
    focus: FieldFocus,
    checks: Option<&BTreeSet<usize>>,
) -> bool {
    if !(q.agent == "claude" && !q.tool_use_id.starts_with("screen-") && k < q.questions.len()) {
        return false;
    }
    let want = &q.questions[k];
    let n = want.options.len();
    let lines: Vec<&str> = screen.split('\n').collect();
    let Some(foot) = lines.iter().rposition(|l| !trim_ws(l).is_empty()) else { return false };
    if !claude_key_line(lines[foot]) {
        return false;
    }
    let Some(sep) = lines[..foot].iter().rposition(|l| {
        let t = trim_ws(l);
        gcount(t) >= 3 && t.graphemes(true).all(|g| g == "─")
    }) else {
        return false;
    };
    if bare(&lines[sep + 1..foot].concat()) != format!("{}.Chataboutthis", n + 2) {
        return false;
    }
    let Some(head) = lines[..sep].iter().rposition(|l| {
        let t = trim_ws(l);
        t.starts_with('☐') || t.starts_with('☒') || t.starts_with('←')
    }) else {
        return false;
    };
    if !claude_bar(q, lines[head]) {
        return false;
    }
    // The text between the bar and the separator, whitespace dropped, with where each line starts. Structure is
    // read from lines, never from the dropped-whitespace text: a row is a line of its own in Claude's
    // two-column focus slot ("❯ " or "  ", then "N."), Submit a line of its own; a mark or a number anywhere
    // else is text. Every row-like line must be one of the rows read (free text cannot hide or add a row).
    let mut region: Vec<&str> = lines[head + 1..sep].to_vec();
    // The question text's lines (above the first row) without a block-quote bar (2.1.322, long questions).
    let first_row = region.iter().position(|l| row_re().is_match(l)).unwrap_or(region.len());
    // A short question is never quoted: its lines are checked as drawn. A long one without the bar. A long text
    // that itself starts with "│" is refused: "│ text" on screen is also `text` behind the bar, so which
    // question is shown cannot be told.
    if gcount(&want.text) >= QUOTE_MIN {
        if trim_ws(&want.text).starts_with('│') {
            return false;
        }
        let unquoted = drop_quote_bar(&region[..first_row]);
        region.splice(..first_row, unquoted);
    }
    let mut chars: Vec<String> = vec![];
    let mut starts: HashMap<usize, usize> = HashMap::new();
    for (i, l) in region.iter().enumerate() {
        let b = bare(l);
        if !b.is_empty() {
            starts.insert(chars.len(), i);
            chars.extend(b.graphemes(true).map(String::from));
        }
    }
    let mut r = Rows { run: Run { chars, pos: 0 }, starts, region: &region, row_lines: HashSet::new(), line_end: 0 };
    if !r.run.take(&bare(&want.text)) {
        return false;
    }
    let mut marks: Vec<usize> = vec![];
    let mut checked: BTreeSet<usize> = BTreeSet::new();
    let boxed = |run: &mut Run, checked: &mut BTreeSet<usize>, i: usize| -> bool {
        if !want.multi_select {
            return true;
        }
        if run.take("[✔]") {
            checked.insert(i);
            return true;
        }
        run.take("[]")
    };
    for (i, o) in want.options.iter().enumerate() {
        let Some(marked) = r.row(i + 1) else { return false };
        if !boxed(&mut r.run, &mut checked, i) {
            return false;
        }
        if marked {
            marks.push(i);
        }
        let fits: Vec<String> = shown_labels(o).into_iter().filter(|l| r.run.has(l)).collect();
        // The label is the rest of the row line, the description starts a line (a wrapped label refuses).
        if !(fits.len() == 1 && r.run.take(&fits[0]) && r.run.pos == r.line_end && r.run.take(&bare(&o.detail))) {
            return false;
        }
    }
    let Some(other_marked) = r.row(n + 1) else { return false };
    if !boxed(&mut r.run, &mut checked, n) {
        return false;
    }
    if other_marked {
        marks.push(n);
    }
    let empty = if want.multi_select { "Typesomething" } else { "Typesomething." };
    let field = match other {
        Some(o) if !o.is_empty() => bare(o),
        _ => empty.to_string(),
    };
    if !r.run.take(&field) {
        return false;
    }
    if want.multi_select {
        let Some(&l) = r.starts.get(&r.run.pos) else { return false };
        if !submit_re().is_match(region[l]) {
            return false;
        }
        r.row_lines.insert(l);
        if r.run.take("❯") {
            marks.push(n + 1);
        }
        if !r.run.take("Submit") {
            return false;
        }
    }
    if !(r.run.pos == r.run.chars.len() && marks.len() == 1) {
        return false;
    }
    let mark = marks[0];
    // No other line looks like a row, Submit or a focus mark.
    for (i, l) in region.iter().enumerate() {
        if r.row_lines.contains(&i) {
            continue;
        }
        if row_re().is_match(l) || submit_re().is_match(l) || trim_ws(l).starts_with('❯') {
            return false;
        }
    }
    if let Some(checks) = checks {
        if want.multi_select && *checks != checked {
            return false;
        }
    }
    // `Auto` as in `shows`: no hub text -> not on the Other row; "" -> on it (multiSelect: anywhere, Down moves
    // it there); text -> on it.
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
        FieldFocus::Auto => mark < n,
        FieldFocus::Any => true,
        FieldFocus::Other => mark == n,
        FieldFocus::Submit => mark == n + 1,
    }
}

/// The answer as Claude's review shows it: the picked labels in row order, then the Other text, ", " between.
pub fn review_parts(question: &Question, a: &QuestionAnswer) -> Vec<Vec<String>> {
    let mut picks = a.picks.clone();
    picks.sort();
    let mut out: Vec<Vec<String>> =
        picks.iter().filter_map(|&p| question.options.get(p)).map(shown_labels).collect();
    out.extend(other_text(a).map(|o| vec![bare(&o)]));
    out
}

/// Claude's review screen of the hooked request `q` with the hub's `answers`, exactly: the tab bar, "Review your
/// answers", per question "● text" and "→ answer", "Ready to submit your answers?", then "1. Submit answers" and
/// "2. Cancel" with one focus mark. The last key ("1") goes only here.
pub fn claude_review_exact(q: &QuestionRequest, answers: &[QuestionAnswer], screen: &str) -> bool {
    if !(q.agent == "claude"
        && !q.tool_use_id.starts_with("screen-")
        && answers.len() == q.questions.len()
        && !review_ambiguous(q, answers)
        && q.questions.iter().zip(answers).all(|(k, a)| a.picks.iter().all(|&p| p < k.options.len())))
    {
        return false;
    }
    let lines: Vec<&str> = screen.split('\n').collect();
    let Some(title) = lines.iter().rposition(|l| trim_ws(l) == "Review your answers") else { return false };
    let Some(head) = lines[..title].iter().rposition(|l| !trim_ws(l).is_empty()) else { return false };
    if !claude_bar(q, lines[head]) {
        return false;
    }
    // Whitespace dropped, with where each line starts: "●", "→", "Ready…" and the two rows must each start a
    // line (a mark inside a label is text).
    let mut chars: Vec<String> = vec![];
    let mut starts: HashSet<usize> = HashSet::new();
    // 2.1.322 draws a long question's "● text" lines with "│ " in front, as in the picker (drop_quote_bar): the
    // bar is drawing. Only in a question's text: from a "│ ●" line to its "→" line. The "→" answer lines (a label
    // can start with "│") and the rows below "Ready to submit your answers?" are checked as they are.
    let ready = lines.iter().rposition(|l| trim_ws(l) == "Ready to submit your answers?").unwrap_or(lines.len());
    let mut barred = false;
    for (i, l) in lines.iter().enumerate().skip(title + 1) {
        let t = trim_ws(l);
        let mut l: &str = l;
        if i < ready {
            if let Some(rest) = t.strip_prefix('│') {
                if trim_ws(rest).starts_with('●') {
                    barred = true;
                }
                if barred {
                    l = rest;
                }
            } else if t.starts_with('→') || t.starts_with('●') {
                barred = false;
            }
            if trim_ws(l).starts_with('→') {
                barred = false;
            }
        }
        let b = bare(l);
        if !b.is_empty() {
            starts.insert(chars.len());
            chars.extend(b.graphemes(true).map(String::from));
        }
    }
    let mut run = Run { chars, pos: 0 };
    let line = |run: &mut Run, lit: &str| starts.contains(&run.pos) && run.take(lit);
    for (k, a) in q.questions.iter().zip(answers) {
        if !(line(&mut run, "●") && run.take(&bare(&k.text)) && line(&mut run, "→")) {
            return false;
        }
        for (j, alts) in review_parts(k, a).iter().enumerate() {
            if j > 0 && !run.take(",") {
                return false;
            }
            let fits: Vec<&String> = alts.iter().filter(|x| run.has(x)).collect();
            if !(fits.len() == 1 && run.take(fits[0])) {
                return false;
            }
        }
    }
    if !line(&mut run, "Readytosubmityouranswers?") {
        return false;
    }
    let mut marks = 0;
    let first = starts.contains(&run.pos);
    if run.take("❯") {
        marks += 1;
    }
    if !(first && run.take("1.Submitanswers")) {
        return false;
    }
    let second = starts.contains(&run.pos);
    if run.take("❯") {
        marks += 1;
    }
    second && run.take("2.Cancel") && run.pos == run.chars.len() && marks == 1
}

/// The review screen cannot prove the hub's answers: a shown label with a comma or a review mark (● →), two
/// labels alike once whitespace is dropped, a question text with a review mark, or Other text with a comma, a
/// review mark or alike a label. Then two answer sets look the same there, so the last key would go blind; the
/// answer is refused before any key. The comma counts only in a multi-choice question: a single-choice one's
/// review line shows one answer (a label or the Other text), never picks joined by commas.
pub fn review_ambiguous(q: &QuestionRequest, answers: &[QuestionAnswer]) -> bool {
    let marks = |s: &str| s.contains('●') || s.contains('→');
    for (k, question) in q.questions.iter().enumerate() {
        let comma = |s: &str| question.multi_select && s.contains(',');
        let all: Vec<String> = question.options.iter().flat_map(|o| shown_labels(o).into_iter().collect::<BTreeSet<_>>()).collect();
        let unique: HashSet<&String> = all.iter().collect();
        if marks(&question.text) || all.iter().any(|x| comma(x) || marks(x)) || unique.len() != all.len() {
            return true;
        }
        if let Some(o) = answers.get(k).and_then(other_text).map(|o| bare(&o)) {
            if comma(&o) || marks(&o) || all.contains(&o) {
                return true;
            }
        }
    }
    false
}
