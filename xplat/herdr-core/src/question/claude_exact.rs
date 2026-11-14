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

/// Terminal display width of `s`: combining/control scalars = 0, East Asian Wide/Fullwidth/emoji = 2, others = 1.
pub(crate) fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn char_width(c: char) -> usize {
    let u = c as u32;
    if is_control(c)
        || matches!(
            u,
            0x0300..=0x036F
                | 0x1160..=0x11FF
                | 0x1AB0..=0x1AFF
                | 0x1DC0..=0x1DFF
                | 0x20D0..=0x20FF
                | 0xFE00..=0xFE0F
                | 0xFE20..=0xFE2F
                | 0xE0100..=0xE01EF
        )
    {
        return 0;
    }
    if matches!(
        u,
        0x1100..=0x115F
            | 0x231A..=0x231B
            | 0x2329..=0x232A
            | 0x23E9..=0x23EC
            | 0x23F0
            | 0x23F3
            | 0x25FD..=0x25FE
            | 0x2614..=0x2615
            | 0x2648..=0x2653
            | 0x267F
            | 0x2693
            | 0x26A1
            | 0x26AA..=0x26AB
            | 0x26BD..=0x26BE
            | 0x26C4..=0x26C5
            | 0x26CE
            | 0x26D4
            | 0x26EA
            | 0x26F2..=0x26F3
            | 0x26F5
            | 0x26FA
            | 0x26FD
            | 0x2705
            | 0x270A..=0x270B
            | 0x2728
            | 0x274C
            | 0x274E
            | 0x2753..=0x2755
            | 0x2757
            | 0x2795..=0x2797
            | 0x27B0
            | 0x27BF
            | 0x2B1B..=0x2B1C
            | 0x2B50
            | 0x2B55
            | 0x2E80..=0x303E
            | 0x3041..=0x3247
            | 0x3250..=0x4DBF
            | 0x4E00..=0xA4C6
            | 0xA960..=0xA97C
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6B
            | 0xFF01..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x16FE0..=0x16FE4
            | 0x16FF0..=0x16FF1
            | 0x17000..=0x18CD5
            | 0x18D00..=0x18D08
            | 0x1AFF0..=0x1B2FB
            | 0x1F004
            | 0x1F0CF
            | 0x1F18E
            | 0x1F191..=0x1F19A
            | 0x1F1E6..=0x1F1FF
            | 0x1F200..=0x1F2FF
            | 0x1F300..=0x1F64F
            | 0x1F680..=0x1F6FF
            | 0x1F900..=0x1F9FF
            | 0x1FA70..=0x1FAFF
            | 0x20000..=0x2FFFD
            | 0x30000..=0x3FFFD
    ) {
        return 2;
    }
    1
}

/// The shortest question text Claude Code 2.1.322 draws behind a block-quote bar (it starts at about 90
/// terminal columns). A shorter text that starts with "│" is that text itself.
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
    if display_width(&want.text) >= QUOTE_MIN {
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

pub const MAX_REVIEW_OPTIONS: usize = 8;

/// Why `review_collision` blocked a pre-flight question card / answer set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionKind {
    Labels,
    Picks,
    Own,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Quoted,
    Unquoted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Cand {
    picks: Vec<usize>,
    other: Option<String>,
}

fn has_marks(s: &str) -> bool {
    s.contains('●') || s.contains('→')
}

fn push_unique(out: &mut Vec<String>, s: String) {
    if !out.contains(&s) {
        out.push(s);
    }
}

fn quote_if_comma(s: String, multi: bool, form: Form) -> String {
    if multi && form == Form::Quoted && s.contains(',') {
        format!("\"{s}\"")
    } else {
        s
    }
}

fn cand_parts(question: &Question, cand: &Cand, form: Form) -> Option<Vec<Vec<String>>> {
    let mut parts = Vec::with_capacity(cand.picks.len() + usize::from(cand.other.is_some()));
    for &p in &cand.picks {
        let opt = question.options.get(p)?;
        let mut alts = vec![];
        for s in shown_labels(opt) {
            if s.is_empty() || s.contains('"') || s.contains('│') {
                return None;
            }
            push_unique(&mut alts, quote_if_comma(s, question.multi_select, form));
        }
        parts.push(alts);
    }
    if let Some(o) = &cand.other {
        if o.is_empty() || o.contains('"') || o.contains('│') {
            return None;
        }
        let mut alts = vec![o.clone()];
        if question.multi_select && form == Form::Quoted && o.contains(',') {
            push_unique(&mut alts, format!("\"{o}\""));
        }
        parts.push(alts);
    }
    Some(parts)
}

fn cand_variants(question: &Question, other_bare: Option<&str>) -> Vec<Cand> {
    let n = question.options.len();
    let mut out = vec![];
    if question.multi_select {
        for mask in 1usize..(1usize << n) {
            let picks: Vec<usize> = (0..n).filter(|i| (mask & (1 << i)) != 0).collect();
            out.push(Cand { picks, other: None });
        }
        if let Some(o) = other_bare {
            for mask in 0usize..(1usize << n) {
                let picks: Vec<usize> = (0..n).filter(|i| (mask & (1 << i)) != 0).collect();
                out.push(Cand { picks, other: Some(o.to_string()) });
            }
        }
    } else {
        for p in 0..n {
            out.push(Cand { picks: vec![p], other: None });
        }
        if let Some(o) = other_bare {
            out.push(Cand { picks: vec![], other: Some(o.to_string()) });
        }
    }
    out
}

fn canonical_cand(a: &QuestionAnswer) -> Option<Cand> {
    let mut picks = a.picks.clone();
    picks.sort_unstable();
    picks.dedup();
    let other = other_text(a).map(|s| bare(&s));
    if picks.is_empty() && other.is_none() {
        return None;
    }
    Some(Cand { picks, other })
}

fn validate_question_caps(q: &Question) -> bool {
    let qt = bare(&q.text);
    if q.options.len() > MAX_REVIEW_OPTIONS || qt.is_empty() || qt.contains('│') {
        return false;
    }
    q.options.iter().all(|o| {
        let labels = shown_labels(o);
        !labels.is_empty() && labels.iter().all(|s| !s.is_empty() && !s.contains('"') && !s.contains('│'))
    })
}

fn validate_answer_caps(q: &Question, a: &QuestionAnswer) -> Result<(), CollisionKind> {
    if a.picks.iter().any(|&p| p >= q.options.len()) {
        return Err(CollisionKind::Picks);
    }
    let has_other = other_text(a);
    if !q.multi_select && a.picks.len() + usize::from(has_other.is_some()) > 1 {
        return Err(CollisionKind::Picks);
    }
    if let Some(o) = has_other {
        let b = bare(&o);
        if b.is_empty() || b.contains('"') || b.contains('│') {
            return Err(CollisionKind::Own);
        }
    }
    Ok(())
}

/// Match `parts[0] , parts[1] , ...` starting at `tail`, returning all matched byte lengths.
fn match_parts_at(tail: &str, parts: &[Vec<String>]) -> Vec<usize> {
    let mut offsets = vec![0usize];
    for (j, alts) in parts.iter().enumerate() {
        if j > 0 {
            let mut next = vec![];
            for off in offsets {
                if tail.get(off..).is_some_and(|t| t.starts_with(',')) && !next.contains(&(off + 1)) {
                    next.push(off + 1);
                }
            }
            offsets = next;
        }
        let mut next = vec![];
        for off in offsets {
            let Some(rest) = tail.get(off..) else { continue };
            for alt in alts {
                if rest.starts_with(alt.as_str()) {
                    let end = off + alt.len();
                    if !next.contains(&end) {
                        next.push(end);
                    }
                }
            }
        }
        if next.is_empty() {
            return vec![];
        }
        offsets = next;
    }
    offsets
}

fn to_tokens<'a>(parts: &'a [Vec<String>], comma: &'a [String]) -> Vec<&'a [String]> {
    let mut t = Vec::with_capacity(parts.len() * 2);
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            t.push(comma);
        }
        t.push(p.as_slice());
    }
    t
}

/// Exact language intersection `L(p1) ∩ L(p2) != ∅` where items in each list are joined by `","`.
fn parts_intersect(p1: &[Vec<String>], p2: &[Vec<String>]) -> bool {
    let comma = [",".to_string()];
    let t1 = to_tokens(p1, &comma);
    let t2 = to_tokens(p2, &comma);
    let mut visited: HashSet<(usize, usize, usize, usize, usize, usize)> = HashSet::new();
    let mut stack = vec![];
    for a1 in 0..t1.first().map_or(0, |s| s.len()) {
        for a2 in 0..t2.first().map_or(0, |s| s.len()) {
            stack.push((0usize, a1, 0usize, 0usize, a2, 0usize));
        }
    }
    while let Some(st @ (i1, a1, o1, i2, a2, o2)) = stack.pop() {
        if !visited.insert(st) {
            continue;
        }
        let s1 = t1[i1][a1].as_bytes();
        let s2 = t2[i2][a2].as_bytes();
        let rem1 = &s1[o1..];
        let rem2 = &s2[o2..];
        let common = rem1.len().min(rem2.len());
        if rem1[..common] != rem2[..common] {
            continue;
        }
        let done1 = common == rem1.len();
        let done2 = common == rem2.len();
        let ni1 = if done1 { i1 + 1 } else { i1 };
        let ni2 = if done2 { i2 + 1 } else { i2 };
        if ni1 == t1.len() && ni2 == t2.len() {
            return true;
        }
        if ni1 == t1.len() || ni2 == t2.len() {
            continue;
        }
        let a1_range = if done1 { 0..t1[ni1].len() } else { a1..a1 + 1 };
        let a2_range = if done2 { 0..t2[ni2].len() } else { a2..a2 + 1 };
        let no1 = if done1 { 0 } else { o1 + common };
        let no2 = if done2 { 0 } else { o2 + common };
        for na1 in a1_range {
            for na2 in a2_range.clone() {
                stack.push((ni1, na1, no1, ni2, na2, no2));
            }
        }
    }
    false
}

/// Run line-start-anchored DP over `body_str` for one `Form`. Returns `(chosen_matched, other_matched)`.
fn dp_form(
    q: &QuestionRequest,
    body_str: &str,
    body_starts: &HashSet<usize>,
    cands_per_q: &[Vec<Cand>],
    chosen_idx: &[usize],
    form: Form,
) -> (bool, bool) {
    let mut chosen_pos: HashSet<usize> = HashSet::from([0]);
    let mut other_pos: HashSet<usize> = HashSet::new();

    for (k, cands) in cands_per_q.iter().enumerate() {
        let mut next_chosen: HashSet<usize> = HashSet::new();
        let mut next_other: HashSet<usize> = HashSet::new();
        let qtext = bare(&q.questions[k].text);
        let mut positions: Vec<usize> = chosen_pos.iter().chain(other_pos.iter()).copied().collect();
        positions.sort_unstable();
        positions.dedup();

        for pos in positions {
            let from_chosen = chosen_pos.contains(&pos);
            let from_other = other_pos.contains(&pos);
            if !body_starts.contains(&pos) || !body_str[pos..].starts_with('●') {
                continue;
            }
            let after_dot = pos + '●'.len_utf8();
            if !body_str[after_dot..].starts_with(&qtext) {
                continue;
            }
            let arrow_pos = after_dot + qtext.len();
            if !body_starts.contains(&arrow_pos) || !body_str[arrow_pos..].starts_with('→') {
                continue;
            }
            let ans_start = arrow_pos + '→'.len_utf8();
            let tail = &body_str[ans_start..];
            for (ci, cand) in cands.iter().enumerate() {
                let Some(parts) = cand_parts(&q.questions[k], cand, form) else { continue };
                for clen in match_parts_at(tail, &parts) {
                    let end_pos = ans_start + clen;
                    if ci == chosen_idx[k] && from_chosen {
                        next_chosen.insert(end_pos);
                    }
                    if ci != chosen_idx[k] || from_other {
                        next_other.insert(end_pos);
                    }
                }
            }
        }
        chosen_pos = next_chosen;
        other_pos = next_other;
        if chosen_pos.is_empty() && other_pos.is_empty() {
            return (false, false);
        }
    }
    (chosen_pos.contains(&body_str.len()), other_pos.contains(&body_str.len()))
}

fn cands_for_question(q: &Question, a: Option<&QuestionAnswer>) -> Option<(Vec<Cand>, Option<usize>)> {
    let chosen = a.and_then(canonical_cand);
    let other_bare = chosen.as_ref().and_then(|c| c.other.as_deref());
    let cands = cand_variants(q, other_bare);
    let chosen_idx = match &chosen {
        None => None,
        Some(c) => Some(cands.iter().position(|x| x == c)?),
    };
    Some((cands, chosen_idx))
}

fn single_question_collides(q: &Question, a: Option<&QuestionAnswer>) -> Option<CollisionKind> {
    let (cands, chosen_idx) = cands_for_question(q, a)?;
    match chosen_idx {
        None => {
            for (i, c1) in cands.iter().enumerate() {
                let p1 = cand_parts(q, c1, Form::Quoted)?;
                for (j, c2) in cands.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let p2q = cand_parts(q, c2, Form::Quoted)?;
                    let p2u = cand_parts(q, c2, Form::Unquoted)?;
                    if parts_intersect(&p1, &p2q) || parts_intersect(&p1, &p2u) {
                        return Some(CollisionKind::Labels);
                    }
                }
            }
            None
        }
        Some(ci) => {
            let c1 = &cands[ci];
            let p1 = cand_parts(q, c1, Form::Quoted)?;
            let mut collides = false;
            for (j, c2) in cands.iter().enumerate() {
                if j == ci {
                    continue;
                }
                let p2q = cand_parts(q, c2, Form::Quoted)?;
                let p2u = cand_parts(q, c2, Form::Unquoted)?;
                if parts_intersect(&p1, &p2q) || parts_intersect(&p1, &p2u) {
                    collides = true;
                    break;
                }
            }
            if !collides {
                return None;
            }
            if c1.other.is_none() {
                return Some(CollisionKind::Picks);
            }
            if c1.picks.is_empty() {
                return Some(CollisionKind::Own);
            }
            // Multi-select with both picks and Other: classify as Picks if picks alone already collide.
            let picks_only = Cand { picks: c1.picks.clone(), other: None };
            if let Some(p_only_q) = cand_parts(q, &picks_only, Form::Quoted) {
                for c2 in cand_variants(q, None) {
                    if c2.picks == c1.picks {
                        continue;
                    }
                    if let (Some(p2q), Some(p2u)) = (cand_parts(q, &c2, Form::Quoted), cand_parts(q, &c2, Form::Unquoted)) {
                        if parts_intersect(&p_only_q, &p2q) || parts_intersect(&p_only_q, &p2u) {
                            return Some(CollisionKind::Picks);
                        }
                    }
                }
            }
            Some(CollisionKind::Own)
        }
    }
}

fn raw_body_variants_for(q: &Question, a: &QuestionAnswer, cap: usize) -> Option<Vec<String>> {
    let mut item_alts: Vec<Vec<String>> = vec![];
    let mut picks = a.picks.clone();
    picks.sort_unstable();
    picks.dedup();
    for p in picks {
        let opt = q.options.get(p)?;
        let alts = if opt.recommended {
            RECOMMENDED_SUFFIXES
                .iter()
                .map(|suf| {
                    let s = format!("{}{}", opt.label, suf);
                    if q.multi_select && s.contains(',') {
                        format!("\"{s}\"")
                    } else {
                        s
                    }
                })
                .collect()
        } else if q.multi_select && opt.label.contains(',') {
            vec![format!("\"{}\"", opt.label)]
        } else {
            vec![opt.label.clone()]
        };
        item_alts.push(alts);
    }
    if let Some(o) = other_text(a) {
        let mut alts = vec![o.clone()];
        if q.multi_select && o.contains(',') {
            push_unique(&mut alts, format!("\"{o}\""));
        }
        item_alts.push(alts);
    }
    let mut acc = vec![String::new()];
    for (j, alts) in item_alts.iter().enumerate() {
        let mut next = vec![];
        for cur in &acc {
            for alt in alts {
                let mut s = cur.clone();
                if j > 0 {
                    s.push(',');
                }
                s.push_str(alt);
                if !next.contains(&s) {
                    if next.len() >= cap {
                        return None;
                    }
                    next.push(s);
                }
            }
        }
        acc = next;
    }
    Some(acc)
}

fn push_rendered_lines(raw: &str, out: &mut String, starts: &mut HashSet<usize>) {
    for l in raw.split('\n') {
        let b = bare(l);
        if !b.is_empty() {
            starts.insert(out.len());
            out.push_str(&b);
        }
    }
}

fn dp_cross_collides(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Option<CollisionKind> {
    let mut cands_per_q = vec![];
    let mut chosen_idx = vec![];
    let mut per_q_bodies = vec![];
    for (k, qq) in q.questions.iter().enumerate() {
        let (cands, Some(ci)) = cands_for_question(qq, Some(&answers[k]))? else { return Some(CollisionKind::Picks) };
        cands_per_q.push(cands);
        chosen_idx.push(ci);
        let Some(bodies) = raw_body_variants_for(qq, &answers[k], 64).filter(|b| !b.is_empty()) else {
            return Some(CollisionKind::Picks);
        };
        per_q_bodies.push(bodies);
    }

    let mut tuples: Vec<Vec<usize>> = vec![vec![]];
    for bs in &per_q_bodies {
        let mut next = vec![];
        for t in &tuples {
            for i in 0..bs.len() {
                if next.len() >= 256 {
                    return Some(CollisionKind::Picks);
                }
                let mut nt = t.clone();
                nt.push(i);
                next.push(nt);
            }
        }
        tuples = next;
    }

    for tuple in tuples {
        let mut body_str = String::new();
        let mut body_starts = HashSet::new();
        for (k, qq) in q.questions.iter().enumerate() {
            push_rendered_lines(&format!("● {}", qq.text), &mut body_str, &mut body_starts);
            push_rendered_lines(&format!("→ {}", per_q_bodies[k][tuple[k]]), &mut body_str, &mut body_starts);
        }
        let (_, other_q) = dp_form(q, &body_str, &body_starts, &cands_per_q, &chosen_idx, Form::Quoted);
        let (_, other_u) = dp_form(q, &body_str, &body_starts, &cands_per_q, &chosen_idx, Form::Unquoted);
        if other_q || other_u {
            let own = answers.iter().filter_map(other_text).any(|o| has_marks(&o))
                || q.questions.iter().zip(answers).any(|(qq, a)| single_question_collides(qq, Some(a)) == Some(CollisionKind::Own));
            return Some(if own { CollisionKind::Own } else { CollisionKind::Picks });
        }
    }
    None
}

/// Structured pre-flight collision check over both `Form::Quoted` and `Form::Unquoted`, supporting empty,
/// partial, and complete `answers`. Returns `None` iff every allowed body variant uniquely identifies the choice.
pub fn review_collision(q: &QuestionRequest, answers: &[QuestionAnswer]) -> Option<CollisionKind> {
    if !(q.agent == "claude"
        && !q.questions.is_empty()
        && q.delivery == Delivery::Picker
        && (q.questions.len() > 1 || q.questions.last().is_some_and(|x| x.multi_select)))
    {
        return None;
    }
    for qq in &q.questions {
        if !validate_question_caps(qq) {
            return Some(CollisionKind::Labels);
        }
    }
    for (k, a) in answers.iter().enumerate() {
        let Some(qq) = q.questions.get(k) else { break };
        if let Err(kind) = validate_answer_caps(qq, a) {
            return Some(kind);
        }
    }

    let is_answered = |k: usize| answers.get(k).is_some_and(|a| !a.picks.is_empty() || other_text(a).is_some());
    let any_unanswered = (0..q.questions.len()).any(|k| !is_answered(k));
    let has_cross_marks = q.questions.len() > 1
        && q.questions.iter().enumerate().any(|(k, qq)| {
            has_marks(&qq.text)
                || qq.options.iter().any(|o| has_marks(&o.label))
                || answers.get(k).and_then(other_text).is_some_and(|o| has_marks(&o))
        });

    if any_unanswered {
        for (k, qq) in q.questions.iter().enumerate() {
            if is_answered(k) {
                if let Some(kind) = single_question_collides(qq, Some(&answers[k])) {
                    return Some(kind);
                }
            }
        }
        if has_cross_marks {
            let own_marks = answers.iter().filter_map(other_text).any(|o| has_marks(&o));
            let label_marks = q.questions.iter().any(|qq| has_marks(&qq.text) || qq.options.iter().any(|o| has_marks(&o.label)));
            if label_marks {
                return Some(CollisionKind::Labels);
            }
            if own_marks {
                return Some(CollisionKind::Own);
            }
        }
        for (k, qq) in q.questions.iter().enumerate() {
            if !is_answered(k) {
                if let Some(kind) = single_question_collides(qq, None) {
                    return Some(kind);
                }
            }
        }
        return None;
    }

    if !has_cross_marks {
        for (k, qq) in q.questions.iter().enumerate() {
            if let Some(kind) = single_question_collides(qq, Some(&answers[k])) {
                return Some(kind);
            }
        }
        return None;
    }

    dp_cross_collides(q, answers)
}

/// Claude's review screen of `q` with `answers`, enforcing line-start anchors and unique joint solution
/// `S_valid = Solutions(Quoted) ∪ Solutions(Unquoted) == {A_chosen}`.
pub fn claude_review_exact(q: &QuestionRequest, answers: &[QuestionAnswer], screen: &str) -> bool {
    if !(q.agent == "claude"
        && !q.questions.is_empty()
        && answers.len() == q.questions.len()
        && q.questions.iter().all(validate_question_caps)
        && q.questions
            .iter()
            .zip(answers)
            .all(|(k, a)| (!a.picks.is_empty() || other_text(a).is_some()) && validate_answer_caps(k, a).is_ok()))
    {
        return false;
    }
    let lines: Vec<&str> = screen.split('\n').collect();
    let Some(title) = lines.iter().rposition(|l| trim_ws(l) == "Review your answers") else { return false };
    let Some(head) = lines[..title].iter().rposition(|l| !trim_ws(l).is_empty()) else { return false };
    if !claude_bar(q, lines[head]) {
        return false;
    }
    let Some(ready) = lines.iter().rposition(|l| trim_ws(l) == "Ready to submit your answers?") else { return false };
    if ready <= title {
        return false;
    }

    // Strip quote bars ONLY inside confirmed quote-barred question-text sections (from "│ ● <q.text>" up to
    // the immediately following "→" line), never inside "→" answer continuations such as a wrapped "│●B".
    let body_lines = &lines[title + 1..ready];
    let mut body_str = String::new();
    let mut body_starts: HashSet<usize> = HashSet::new();
    let mut i = 0;
    while i < body_lines.len() {
        let t = trim_ws(body_lines[i]);
        if let Some(first_rest) = t.strip_prefix('│') {
            if trim_ws(first_rest).starts_with('●') {
                let mut j = i;
                let mut stripped_lines = Vec::new();
                let mut stripped_bare = String::new();
                while j < body_lines.len() {
                    let Some(rj) = trim_ws(body_lines[j]).strip_prefix('│') else { break };
                    if trim_ws(rj).starts_with('→') {
                        break;
                    }
                    stripped_bare.push_str(&bare(rj));
                    stripped_lines.push(rj);
                    j += 1;
                }
                let next_is_arrow = body_lines[j..].iter().find(|l| !trim_ws(l).is_empty()).is_some_and(|l| trim_ws(l).starts_with('→'));
                let is_quoted_question = next_is_arrow
                    && q.questions.iter().any(|qq| {
                        display_width(&qq.text) >= QUOTE_MIN
                            && !trim_ws(&qq.text).starts_with('│')
                            && stripped_bare == format!("●{}", bare(&qq.text))
                    });
                if is_quoted_question {
                    for rj in stripped_lines {
                        let b = bare(rj);
                        if !b.is_empty() {
                            body_starts.insert(body_str.len());
                            body_str.push_str(&b);
                        }
                    }
                    i = j;
                    continue;
                }
            }
        }
        let b = bare(body_lines[i]);
        if !b.is_empty() {
            body_starts.insert(body_str.len());
            body_str.push_str(&b);
        }
        i += 1;
    }

    let mut fchars: Vec<String> = vec![];
    let mut fstarts: HashSet<usize> = HashSet::new();
    for l in &lines[ready..] {
        let b = bare(l);
        if !b.is_empty() {
            fstarts.insert(fchars.len());
            fchars.extend(b.graphemes(true).map(String::from));
        }
    }

    let mut cands_per_q = vec![];
    let mut chosen_idx = vec![];
    for (k, a) in q.questions.iter().zip(answers) {
        let Some((cands, Some(ci))) = cands_for_question(k, Some(a)) else { return false };
        cands_per_q.push(cands);
        chosen_idx.push(ci);
    }

    let (chosen_q, other_q) = dp_form(q, &body_str, &body_starts, &cands_per_q, &chosen_idx, Form::Quoted);
    let (chosen_u, other_u) = dp_form(q, &body_str, &body_starts, &cands_per_q, &chosen_idx, Form::Unquoted);
    if !((chosen_q || chosen_u) && !other_q && !other_u) {
        return false;
    }

    let mut run = Run { chars: fchars, pos: 0 };
    if !(fstarts.contains(&run.pos) && run.take("Readytosubmityouranswers?")) {
        return false;
    }
    let mut marks = 0;
    let first = fstarts.contains(&run.pos);
    if run.take("❯") {
        marks += 1;
    }
    if !(first && run.take("1.Submitanswers")) {
        return false;
    }
    let second = fstarts.contains(&run.pos);
    if run.take("❯") {
        marks += 1;
    }
    second && run.take("2.Cancel") && run.pos == run.chars.len() && marks == 1
}
