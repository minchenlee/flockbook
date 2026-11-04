//! Claude Code's status line as Flockbook keeps it (Swift: the reading side of `StatusLine.swift`). Claude Code runs
//! Flockbook's wrapper as its statusLine command; the wrapper saves the JSON input and the owner's command's output
//! per session (`StatusLineFolder`). This reads a session's record and turns it into the row's tags. The wrapper
//! itself (`StatusLineWrapper`) and the settings switch (`ClaudeStatusLineSetting`) are not ported.

use crate::util::Date;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use unicode_segmentation::UnicodeSegmentation;

/// Swift's `String.count`: grapheme clusters.
fn count(s: &str) -> usize {
    s.graphemes(true).count()
}

/// Swift's `String(format: "%.2f", x)`.
fn two_places(x: f64) -> String {
    format!("{x:.2}")
}

/// A piece of a status line in one style. Colours are `#rrggbb`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StyledRun {
    pub text: String,
    pub fg: Option<String>,
    pub bg: Option<String>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

impl StyledRun {
    pub fn new(text: &str) -> StyledRun {
        StyledRun { text: text.into(), ..Default::default() }
    }
    pub fn fg(mut self, c: &str) -> StyledRun {
        self.fg = Some(c.into());
        self
    }
    pub fn bg(mut self, c: &str) -> StyledRun {
        self.bg = Some(c.into());
        self
    }
    pub fn bold(mut self) -> StyledRun {
        self.bold = true;
        self
    }
    pub fn dim(mut self) -> StyledRun {
        self.dim = true;
        self
    }
    pub fn italic(mut self) -> StyledRun {
        self.italic = true;
        self
    }
    pub fn underline(mut self) -> StyledRun {
        self.underline = true;
        self
    }

    fn style(&self) -> Style {
        Style {
            fg: self.fg.clone(),
            bg: self.bg.clone(),
            bold: self.bold,
            dim: self.dim,
            italic: self.italic,
            underline: self.underline,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Style {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

/// Terminal text with ANSI SGR colours to styled runs, line by line (Swift: `ANSIText`). Other escape sequences and
/// control characters are dropped (an OSC 8 link keeps its text); a style goes on over a newline, as in a terminal.
pub mod ansi_text {
    use super::{count, Style, StyledRun};
    use unicode_segmentation::UnicodeSegmentation;

    pub const MAX_LINES: usize = 8;
    /// Characters per line kept.
    pub const MAX_COLUMNS: usize = 400;

    /// The 16 basic colours (xterm's).
    pub const BASIC: [&str; 16] = [
        "#000000", "#cd0000", "#00cd00", "#cdcd00", "#0000ee", "#cd00cd", "#00cdcd", "#e5e5e5", "#7f7f7f", "#ff0000", "#00ff00",
        "#ffff00", "#5c5cff", "#ff00ff", "#00ffff", "#ffffff",
    ];

    fn hex(r: i64, g: i64, b: i64) -> String {
        format!("#{r:02x}{g:02x}{b:02x}")
    }

    /// xterm's 256 colours; None out of range.
    pub fn color256(n: i64) -> Option<String> {
        match n {
            0..=15 => Some(BASIC[n as usize].into()),
            16..=231 => {
                let levels = [0, 95, 135, 175, 215, 255];
                let i = (n - 16) as usize;
                Some(hex(levels[i / 36], levels[i / 6 % 6], levels[i % 6]))
            }
            232..=255 => {
                let v = 8 + 10 * (n - 232);
                Some(hex(v, v, v))
            }
            _ => None,
        }
    }

    /// Marks that change the text's direction or take no width. The joiner (U+200D) stays: emoji need it.
    fn hidden(c: u32) -> bool {
        matches!(c, 0x061C | 0x200B | 0x200E | 0x200F | 0x2060 | 0xFEFF | 0x202A..=0x202E | 0x2066..=0x2069)
    }

    pub fn lines(text: &str) -> Vec<Vec<StyledRun>> {
        let mut out: Vec<Vec<StyledRun>> = vec![];
        let mut line: Vec<StyledRun> = vec![];
        let mut style = Style::default();
        let mut buf = String::new();
        fn flush(buf: &mut String, line: &mut Vec<StyledRun>, style: &Style) {
            if buf.is_empty() {
                return;
            }
            match line.last_mut() {
                Some(last) if last.style() == *style => last.text.push_str(buf),
                _ => line.push(StyledRun {
                    text: buf.clone(),
                    fg: style.fg.clone(),
                    bg: style.bg.clone(),
                    bold: style.bold,
                    dim: style.dim,
                    italic: style.italic,
                    underline: style.underline,
                }),
            }
            buf.clear();
        }
        let s: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < s.len() {
            let c = s[i];
            if c == '\u{1B}' {
                i += 1;
                if i >= s.len() {
                    break;
                }
                match s[i] {
                    '[' => {
                        // CSI: parameters, intermediates, one final byte. Only `m` (SGR) changes anything.
                        let mut j = i + 1;
                        while j < s.len() && (0x30..=0x3F).contains(&(s[j] as u32)) {
                            j += 1;
                        }
                        let params: String = s[i + 1..j].iter().collect();
                        while j < s.len() && (0x20..=0x2F).contains(&(s[j] as u32)) {
                            j += 1;
                        }
                        if j >= s.len() || !(0x40..=0x7E).contains(&(s[j] as u32)) {
                            i = s.len();
                            continue;
                        }
                        if s[j] == 'm' && s[i + 1..j].iter().all(|c| !matches!(c, '?' | '>' | '<' | '=')) {
                            flush(&mut buf, &mut line, &style);
                            sgr(&params, &mut style);
                        }
                        i = j + 1;
                    }
                    ']' | 'P' | 'X' | '^' | '_' => {
                        // A string (OSC, DCS…) up to BEL or ESC \: dropped; the text after it stays.
                        let mut j = i + 1;
                        while j < s.len() && s[j] != '\u{07}' && !(s[j] == '\u{1B}' && j + 1 < s.len() && s[j + 1] == '\\') {
                            j += 1;
                        }
                        i = if j < s.len() && s[j] == '\u{07}' { j + 1 } else { j + 2 };
                    }
                    _ => {
                        let mut j = i;
                        while j < s.len() && (0x20..=0x2F).contains(&(s[j] as u32)) {
                            j += 1;
                        }
                        i = j + 1;
                    }
                }
                continue;
            }
            i += 1;
            let v = c as u32;
            match c {
                '\n' => {
                    flush(&mut buf, &mut line, &style);
                    out.push(std::mem::take(&mut line));
                }
                '\t' => buf.push(' '),
                _ if v < 0x20 || (0x7F..=0x9F).contains(&v) => {}
                _ if hidden(v) => {}
                _ => buf.push(c),
            }
        }
        if !buf.is_empty() || !line.is_empty() {
            flush(&mut buf, &mut line, &style);
            out.push(line);
        }
        out.into_iter().map(|l| cut(&l)).filter(|l| !l.is_empty()).take(MAX_LINES).collect()
    }

    /// The line's first `MAX_COLUMNS` characters.
    fn cut(line: &[StyledRun]) -> Vec<StyledRun> {
        let mut left = MAX_COLUMNS;
        let mut kept = vec![];
        for r in line {
            if left == 0 {
                break;
            }
            let mut r = r.clone();
            if count(&r.text) > left {
                r.text = r.text.graphemes(true).take(left).collect();
            }
            left -= count(&r.text);
            kept.push(r);
        }
        kept
    }

    fn sgr(params: &str, st: &mut Style) {
        let p: Vec<i64> = if params.is_empty() { vec![0] } else { params.split(';').map(|x| x.parse().unwrap_or(0)).collect() };
        let mut i = 0;
        while i < p.len() {
            let n = p[i];
            match n {
                0 => *st = Style::default(),
                1 => st.bold = true,
                2 => st.dim = true,
                3 => st.italic = true,
                4 => st.underline = true,
                22 => {
                    st.bold = false;
                    st.dim = false
                }
                23 => st.italic = false,
                24 => st.underline = false,
                30..=37 => st.fg = Some(BASIC[(n - 30) as usize].into()),
                39 => st.fg = None,
                40..=47 => st.bg = Some(BASIC[(n - 40) as usize].into()),
                49 => st.bg = None,
                90..=97 => st.fg = Some(BASIC[(n - 90 + 8) as usize].into()),
                100..=107 => st.bg = Some(BASIC[(n - 100 + 8) as usize].into()),
                38 | 48 => {
                    // 38;5;n or 38;2;r;g;b. Cut or out of range: the rest of the sequence is dropped.
                    let mut color = None;
                    if i + 2 < p.len() && p[i + 1] == 5 {
                        color = color256(p[i + 2]);
                        i += 2;
                    } else if i + 4 < p.len() && p[i + 1] == 2 && p[i + 2..=i + 4].iter().all(|c| (0..=255).contains(c)) {
                        color = Some(hex(p[i + 2], p[i + 3], p[i + 4]));
                        i += 4;
                    }
                    let Some(color) = color else { return };
                    if n == 38 {
                        st.fg = Some(color)
                    } else {
                        st.bg = Some(color)
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    pub fn plain(lines: &[Vec<StyledRun>]) -> String {
        lines.iter().map(|l| l.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
    }
}

/// What the status line command's JSON says about the session: model, effort, Claude Code's own cost estimate and
/// context use. None without a session id that is a plain name (letters, digits, `_`, `-`).
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeStatusInfo {
    pub session: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Claude Code's estimate of the session's cost so far, in US dollars.
    pub cost_usd: Option<f64>,
    pub context_percent: Option<i64>,
    pub context_size: Option<i64>,
}

/// An `NSNumber` from JSONSerialization: numbers, and booleans as 1 / 0.
fn number(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn non_empty(s: Option<&str>) -> Option<String> {
    s.filter(|s| !s.is_empty()).map(String::from)
}

impl ClaudeStatusInfo {
    pub const MAX_BYTES: usize = 262_144;

    pub fn from_json(json: &[u8]) -> Option<ClaudeStatusInfo> {
        if json.len() > Self::MAX_BYTES {
            return None;
        }
        let Ok(Value::Object(o)) = serde_json::from_slice::<Value>(json) else { return None };
        let sid = o.get("session_id")?.as_str()?;
        if !plain_name(sid) {
            return None;
        }
        let m = o.get("model").and_then(|v| v.as_object());
        let model = m.and_then(|m| m.get("display_name").and_then(|v| v.as_str()).or_else(|| m.get("id").and_then(|v| v.as_str())));
        let effort = o.get("effort").and_then(|v| v.as_object()).and_then(|e| e.get("level")).and_then(|v| v.as_str());
        let cost = number(o.get("cost").and_then(|v| v.as_object()).and_then(|c| c.get("total_cost_usd")));
        let ctx = o.get("context_window").and_then(|v| v.as_object());
        let pct = number(ctx.and_then(|c| c.get("used_percentage")))
            .filter(|d| d.is_finite())
            .map(|d| d.clamp(0.0, 100.0).round() as i64);
        // NSNumber.intValue: truncated.
        let size = number(ctx.and_then(|c| c.get("context_window_size"))).map(|d| d as i64);
        Some(ClaudeStatusInfo {
            session: sid.into(),
            model: non_empty(model),
            effort: non_empty(effort),
            cost_usd: cost,
            context_percent: pct,
            context_size: size,
        })
    }

    /// "Opus 5.5 · high · ctx 37% · $0.42": what is known.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = vec![];
        if let Some(m) = &self.model {
            parts.push(m.clone());
        }
        if let Some(e) = &self.effort {
            parts.push(e.clone());
        }
        if let Some(p) = self.context_percent {
            parts.push(format!("ctx {p}%"));
        }
        if let Some(c) = self.cost_usd {
            parts.push(format!("${}", two_places(c)));
        }
        parts.join(" · ")
    }

    /// The row's tags: model, effort, context, cost; a field Claude Code did not send has none.
    pub fn tags(&self) -> Vec<StatusTag> {
        let mut out = vec![];
        if let Some(m) = &self.model {
            out.push(StatusTag::plain(StatusTagKind::Model, m, &format!("Model {m}"), None, false));
        }
        if let Some(e) = &self.effort {
            out.push(StatusTag::plain(StatusTagKind::Effort, e, &format!("Effort {e}"), None, false));
        }
        if let Some(p) = self.context_percent {
            out.push(StatusTag::plain(
                StatusTagKind::Context,
                &format!("ctx {p}%"),
                &format!("Context {p}% used"),
                Some(p),
                p >= StatusTag::WARN_PERCENT,
            ));
        }
        if let Some(c) = self.cost_usd {
            let d = format!("${}", two_places(c));
            out.push(StatusTag::plain(StatusTagKind::Cost, &format!("{d} est."), &format!("Session cost about {d}, an estimate"), None, false));
        }
        out
    }
}

/// One session's saved status line: the owner's command's output as styled lines, and the JSON's facts.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusLineRecord {
    pub lines: Vec<Vec<StyledRun>>,
    pub info: Option<ClaudeStatusInfo>,
    pub time: Date,
}

impl StatusLineRecord {
    /// The row's tags: Claude Code's facts, then the owner's parts that do not repeat them.
    pub fn tags(&self) -> Vec<StatusTag> {
        let mut out = self.info.as_ref().map(|i| i.tags()).unwrap_or_default();
        for p in self.parts() {
            let text: String = p.iter().map(|r| r.text.as_str()).collect();
            out.push(StatusTag { kind: StatusTagKind::Own, text: text.clone(), spoken: text, percent: None, warn: false, runs: p });
        }
        out
    }

    /// The owner's lines as parts, without the ones that repeat the facts.
    pub fn parts(&self) -> Vec<Vec<StyledRun>> {
        status_segments::drop_repeats(&status_segments::split(&self.lines), self.info.as_ref())
    }

    /// No own line and no facts: nothing to show.
    pub fn is_empty(&self) -> bool {
        self.tags().is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusTagKind {
    Model,
    Effort,
    Context,
    Cost,
    Own,
}

impl StatusTagKind {
    /// Swift's raw value.
    pub fn raw(self) -> &'static str {
        match self {
            StatusTagKind::Model => "model",
            StatusTagKind::Effort => "effort",
            StatusTagKind::Context => "context",
            StatusTagKind::Cost => "cost",
            StatusTagKind::Own => "own",
        }
    }
}

/// One tag of the status row: what Claude Code's JSON says, one fact each, or one of the owner's parts.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StatusTag {
    pub kind: StatusTagKind,
    pub text: String,
    /// What a screen reader says.
    pub spoken: String,
    /// The context used, for its bar.
    pub percent: Option<i64>,
    /// The context is nearly full (`WARN_PERCENT` and up).
    pub warn: bool,
    /// An owner's part (`Own`): its text in its colours.
    pub runs: Vec<StyledRun>,
}

impl StatusTag {
    pub const WARN_PERCENT: i64 = 80;

    fn plain(kind: StatusTagKind, text: &str, spoken: &str, percent: Option<i64>, warn: bool) -> StatusTag {
        StatusTag { kind, text: text.into(), spoken: spoken.into(), percent, warn, runs: vec![] }
    }

    /// The orange of a full context, light and dark.
    pub fn warn_hex(dark: bool) -> &'static str {
        if dark {
            "#ffa04a"
        } else {
            "#a04b00"
        }
    }
    /// The tag's fill over the pane, light and dark.
    pub fn tag_hex(dark: bool) -> &'static str {
        if dark {
            "#2e2e2e"
        } else {
            "#f0f0f0"
        }
    }
    /// Dim text (ANSI 2) on the tag, light and dark.
    pub fn dim_hex(dark: bool) -> &'static str {
        if dark {
            "#969696"
        } else {
            "#6b6b6b"
        }
    }

    /// The colour a run of an owner's part is drawn in: dim, the fixed grey; its own colour where it reads on the
    /// tag; else None (the tag's own text colour).
    pub fn run_hex(run: &StyledRun, dark: bool) -> Option<String> {
        if run.dim {
            return Some(Self::dim_hex(dark).into());
        }
        run.fg.as_ref().filter(|fg| Self::readable(fg, dark)).cloned()
    }

    /// Whether a part's colour reads on the tag (4.5:1).
    pub fn readable(hex: &str, dark: bool) -> bool {
        Self::contrast(hex, Self::tag_hex(dark)) >= 4.5
    }

    /// WCAG contrast of two `#rrggbb` colours.
    pub fn contrast(a: &str, b: &str) -> f64 {
        fn lum(hex: &str) -> f64 {
            if count(hex) != 7 || !hex.starts_with('#') {
                return 0.0;
            }
            let Ok(v) = u32::from_str_radix(&hex[1..], 16) else { return 0.0 };
            let lin = |c: u32| {
                let x = (c & 0xff) as f64 / 255.0;
                if x <= 0.04045 {
                    x / 12.92
                } else {
                    ((x + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(v >> 16) + 0.7152 * lin(v >> 8) + 0.0722 * lin(v)
        }
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }
}

/// The owner's status line as parts, one tag each.
pub mod status_segments {
    use super::{two_places, ClaudeStatusInfo, StyledRun};
    use unicode_segmentation::UnicodeSegmentation;

    /// Parts kept, at most.
    pub const MAX_PARTS: usize = 40;

    /// Swift's `Character.isWhitespace` (and a regex `\s` on one character): its first scalar is White_Space.
    fn ws(g: &str) -> bool {
        g.chars().next().is_some_and(char::is_whitespace)
    }

    fn run_of(g: &[&str], i: usize) -> usize {
        g[i..].iter().take_while(|c| ws(c)).count()
    }

    /// The match of `\s+[·|•]\s+|\s*│\s*|\s{2,}` starting at `i` (its end), over characters.
    fn match_at(g: &[&str], i: usize) -> Option<usize> {
        let w = run_of(g, i);
        if w >= 1 && i + w < g.len() && matches!(g[i + w], "·" | "|" | "•") {
            let after = run_of(g, i + w + 1);
            if after >= 1 {
                return Some(i + w + 1 + after);
            }
        }
        if i + w < g.len() && g[i + w] == "│" {
            return Some(i + w + 1 + run_of(g, i + w + 1));
        }
        if w >= 2 {
            return Some(i + w);
        }
        None
    }

    /// Each line cut at " · ", " | ", " • " (spaces around), "│", or two spaces and more; each part trimmed, an empty
    /// one or a lone separator dropped; each part keeps its runs' styles.
    pub fn split(lines: &[Vec<StyledRun>]) -> Vec<Vec<StyledRun>> {
        let mut out: Vec<Vec<StyledRun>> = vec![];
        for line in lines {
            // One entry per character: the character and the run it came from.
            let chars: Vec<(&str, usize)> =
                line.iter().enumerate().flat_map(|(i, r)| r.text.graphemes(true).map(move |c| (c, i))).collect();
            let g: Vec<&str> = chars.iter().map(|c| c.0).collect();
            let mut cuts: Vec<(usize, usize)> = vec![];
            let mut i = 0;
            while i < g.len() {
                match match_at(&g, i) {
                    Some(end) => {
                        cuts.push((i, end));
                        i = end;
                    }
                    None => i += 1,
                }
            }
            cuts.push((g.len(), g.len()));
            let mut start = 0;
            for (lo, hi) in cuts {
                let mut part = &chars[start.min(lo)..lo];
                while part.first().is_some_and(|f| ws(f.0)) {
                    part = &part[1..];
                }
                while part.last().is_some_and(|l| ws(l.0)) {
                    part = &part[..part.len() - 1];
                }
                start = hi;
                if part.is_empty() || part.iter().all(|c| matches!(c.0, "·" | "|" | "│" | "•")) {
                    continue;
                }
                // Consecutive characters of one run stay one run.
                let mut runs: Vec<StyledRun> = vec![];
                let mut from: Vec<usize> = vec![];
                for (c, i) in part {
                    if from.last() == Some(i) {
                        runs.last_mut().unwrap().text.push_str(c);
                    } else {
                        let mut r = line[*i].clone();
                        r.text = c.to_string();
                        runs.push(r);
                        from.push(*i);
                    }
                }
                out.push(runs);
                if out.len() == MAX_PARTS {
                    return out;
                }
            }
        }
        out
    }

    /// The parts without those that say exactly what a JSON tag says: the model's name, "ctx|context N%" with its
    /// percent, "session $X" with its cost. Plain text only; anything else stays.
    pub fn drop_repeats(segments: &[Vec<StyledRun>], info: Option<&ClaudeStatusInfo>) -> Vec<Vec<StyledRun>> {
        let Some(info) = info else { return segments.to_vec() };
        fn norm(s: &str) -> String {
            s.to_lowercase().split(char::is_whitespace).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
        }
        let ctx = regex::Regex::new(r"^(?:ctx|context) ?:? ?(\d{1,3}) ?%$").expect("ctx pattern");
        let cost = regex::Regex::new(r"^session(?: cost)? ?:? ?\$(\d+(?:\.\d+)?)$").expect("cost pattern");
        segments
            .iter()
            .filter(|seg| {
                let t = norm(&seg.iter().map(|r| r.text.as_str()).collect::<String>());
                if info.model.as_ref().is_some_and(|m| t == norm(m)) {
                    return false;
                }
                if let (Some(p), Some(m)) = (info.context_percent, ctx.captures(&t)) {
                    if m[1].parse::<i64>().ok() == Some(p) {
                        return false;
                    }
                }
                if let (Some(c), Some(m)) = (info.cost_usd, cost.captures(&t)) {
                    if m[1].parse::<f64>().is_ok_and(|v| two_places(v) == two_places(c)) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }
}

/// The status row: one line of tags, then the chevron.
pub mod status_tag_row {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Fit {
        pub shown: usize,
        pub chevron: bool,
    }

    /// How many tags fit on one row, in order, and whether the chevron shows: when a tag is left out, or `more`.
    pub fn fit(widths: &[f64], chevron: f64, spacing: f64, available: f64, more: bool) -> Fit {
        let (mut used, mut all) = (0.0, 0usize);
        for w in widths {
            let next = used + if all > 0 { spacing } else { 0.0 } + w;
            if next > available {
                break;
            }
            used = next;
            all += 1;
        }
        if all == widths.len() && !more {
            return Fit { shown: all, chevron: false };
        }
        let mut shown = all;
        while shown > 0 {
            let row: f64 = widths[..shown].iter().sum::<f64>() + (shown - 1) as f64 * spacing;
            if row + spacing + chevron <= available {
                break;
            }
            shown -= 1;
        }
        Fit { shown, chevron: true }
    }

    /// The chevron's symbol name: closed, up; open, down.
    pub fn chevron_symbol(open: bool) -> &'static str {
        if open {
            "chevron.down"
        } else {
            "chevron.up"
        }
    }
}

fn plain_name(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The wrapper's folder: per session `<id>.json` (the input) and `<id>.out` (the owner's command's output).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusLineFolder {
    pub url: PathBuf,
}

impl StatusLineFolder {
    /// Saved output over this size is cut; files over it are not read.
    pub const MAX_BYTES: usize = 65_536;
    /// Records older than this (seconds) are not shown, and `clean` removes them.
    pub const MAX_AGE: f64 = 24.0 * 3600.0;

    pub fn new(dir: &Path) -> StatusLineFolder {
        StatusLineFolder { url: dir.to_path_buf() }
    }

    fn file(&self, session: &str, ext: &str) -> PathBuf {
        self.url.join(format!("{session}.{ext}"))
    }

    /// Written whole through a temp file (mode 600, the folder 700).
    fn write(&self, data: &[u8], to: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.url)?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&self.url)?;
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let tmp = self.url.join(format!(".tmp.{}.{:08x}", std::process::id(), nanos));
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let r = (|| {
            use std::io::Write;
            o.open(&tmp)?.write_all(data)?;
            std::fs::rename(&tmp, to)
        })();
        if r.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        r
    }

    /// Saves one run: the input and the output (cut at `MAX_BYTES`). Errors are ignored.
    pub fn save(&self, input: &[u8], output: &[u8], session: &str) {
        self.save_input(input, session);
        self.save_output(output, session);
    }

    /// Saves the input alone; the last output stays.
    pub fn save_input(&self, input: &[u8], session: &str) {
        if plain_name(session) {
            let _ = self.write(&input[..input.len().min(ClaudeStatusInfo::MAX_BYTES)], &self.file(session, "json"));
        }
    }

    pub fn save_output(&self, output: &[u8], session: &str) {
        if plain_name(session) {
            let _ = self.write(&output[..output.len().min(Self::MAX_BYTES)], &self.file(session, "out"));
        }
    }

    /// The session's saved line; None when none, or older than `MAX_AGE`. Links are not followed.
    pub fn record(&self, session: &str, now: SystemTime) -> Option<StatusLineRecord> {
        if !plain_name(session) {
            return None;
        }
        let (j, o) = (self.file(session, "json"), self.file(session, "out"));
        let a = std::fs::symlink_metadata(&j).ok()?;
        if !a.file_type().is_file() || a.len() > ClaudeStatusInfo::MAX_BYTES as u64 {
            return None;
        }
        let t = a.modified().ok()?;
        let age = match now.duration_since(t) {
            Ok(d) => d.as_secs_f64(),
            Err(e) => -e.duration().as_secs_f64(),
        };
        if age > Self::MAX_AGE {
            return None;
        }
        let input = std::fs::read(&j).ok()?;
        let mut lines = vec![];
        if let Ok(b) = std::fs::symlink_metadata(&o) {
            if b.file_type().is_file() && b.len() <= Self::MAX_BYTES as u64 {
                if let Ok(out) = std::fs::read(&o) {
                    lines = ansi_text::lines(&String::from_utf8_lossy(&out));
                }
            }
        }
        Some(StatusLineRecord { lines, info: ClaudeStatusInfo::from_json(&input), time: DateTime::<Utc>::from(t) })
    }

    /// Removes records older than `MAX_AGE` and stray temp files (over an hour old).
    pub fn clean(&self, now: SystemTime) {
        let Ok(rd) = std::fs::read_dir(&self.url) else { return };
        for e in rd.filter_map(|e| e.ok()) {
            let Some(n) = e.file_name().to_str().map(String::from) else { continue };
            if !(n.ends_with(".json") || n.ends_with(".out") || n.starts_with(".tmp.")) {
                continue;
            }
            let Some(t) = std::fs::symlink_metadata(e.path()).ok().and_then(|m| m.modified().ok()) else { continue };
            let age = now.duration_since(t).map(|d| d.as_secs_f64()).unwrap_or(0.0);
            if age > if n.starts_with(".tmp.") { 3600.0 } else { Self::MAX_AGE } {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests;
