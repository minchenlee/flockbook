//! Agent-owned queues. Screen text is display evidence; only exact Claude log text can be pulled back.
use crate::AgentIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeQueueLog {
    pub items: Vec<Option<String>>,
}
impl ClaudeQueueLog {
    pub fn feed(&mut self, value: &Value) {
        if value["type"] != "queue-operation" {
            return;
        }
        let text = value["content"].as_str().map(str::to_owned);
        match value["operation"].as_str() {
            Some("enqueue") => self.items.push(text),
            Some("dequeue") if !self.items.is_empty() => {
                self.items.remove(0);
            }
            Some("remove" | "popAll") => {
                let i = self
                    .items
                    .iter()
                    .position(|s| s == &text)
                    .or_else(|| self.items.iter().position(Option::is_none));
                if let Some(i) = i {
                    self.items.remove(i);
                } else if text.is_none() && !self.items.is_empty() {
                    self.items.remove(0);
                }
            }
            _ => {}
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedInput {
    pub agent: String,
    pub items: Vec<Option<String>>,
    pub pullable: bool,
}
const CLAUDE_HINT: &str = "ctrl+enter to send now";
const PLACEHOLDER: &str = "Press up to edit queued messages";
fn compact(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}
fn same(entry: &Option<String>, row: &str) -> bool {
    entry.as_ref().is_none_or(|e| {
        let (e, r) = (compact(e), compact(row));
        e == r
            || r.strip_suffix('…')
                .is_some_and(|prefix| e.starts_with(prefix))
    })
}
fn runs(s: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut in_run = false;
    for line in s.split('\n') {
        let c = compact(line);
        if c.is_empty() {
            out.push(c);
            in_run = false;
        } else if in_run {
            out.last_mut().unwrap().push_str(&c);
        } else {
            out.push(c);
            in_run = true;
        }
    }
    out
}
fn same_lines(e: &str, row: &str) -> bool {
    let (e, r) = (runs(e), runs(row));
    e == r
        || (!r.is_empty()
            && e.len() >= r.len()
            && e[..r.len() - 1] == r[..r.len() - 1]
            && r.last()
                .unwrap()
                .strip_suffix('…')
                .is_some_and(|p| e[r.len() - 1].starts_with(p)))
}
fn rule(l: &str) -> bool {
    let t = l.trim();
    t.chars().count() >= 10 && t.chars().all(|c| c == '─')
}
pub fn raw_box(screen: &str) -> Vec<String> {
    let lines: Vec<_> = screen.split('\n').collect();
    let rules: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| rule(l))
        .map(|(i, _)| i)
        .collect();
    if rules.len() < 2 {
        return vec![];
    }
    lines[rules[rules.len() - 2] + 1..rules[rules.len() - 1]]
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                l.strip_prefix("❯ ").unwrap_or(l).to_string()
            } else {
                l.strip_prefix("  ").unwrap_or(l).to_string()
            }
        })
        .collect()
}
pub fn read(kind: &str, working: bool, screen: &str, log: &ClaudeQueueLog) -> Option<QueuedInput> {
    if !working {
        return None;
    }
    let lines: Vec<_> = screen.split('\n').collect();
    match kind {
        "claude" => {
            let hint = lines.iter().rposition(|l| l.trim() == CLAUDE_HINT)?;
            let mut rows: Vec<(String, bool)> = vec![];
            let mut cont: Vec<String> = vec![];
            for l in lines[..hint].iter().rev() {
                if let Some(t) = l.strip_prefix("❯ ") {
                    let mut parts = vec![t.trim_end().to_string()];
                    parts.extend(cont.iter().rev().cloned());
                    let gapped = parts.iter().any(String::is_empty);
                    if !rows.is_empty()
                        && parts.len() > 1
                        && parts.last().is_some_and(String::is_empty)
                    {
                        parts.pop();
                    }
                    rows.insert(0, (parts.join("\n"), gapped));
                    cont.clear();
                } else if l.trim().is_empty() && (!rows.is_empty() || !cont.is_empty()) {
                    cont.push(String::new());
                } else if l.starts_with("  ") && !l.trim().is_empty() {
                    cont.push(l[2..].trim_end().to_string());
                } else {
                    break;
                }
            }
            if rows.is_empty() {
                return None;
            }
            let mut m = 0;
            while m < rows.len() && m < log.items.len() {
                let (r, g) = &rows[rows.len() - 1 - m];
                let e = &log.items[log.items.len() - 1 - m];
                if !if *g {
                    e.as_ref().is_some_and(|e| same_lines(e, r))
                } else {
                    same(e, r)
                } {
                    break;
                }
                m += 1;
            }
            let items = if m == 0 {
                let start = rows.iter().rposition(|(_, g)| *g).map_or(0, |i| i + 1);
                rows[start..]
                    .iter()
                    .map(|(r, _)| Some(r.clone()))
                    .collect::<Vec<_>>()
            } else {
                let mut items = log.items[..log.items.len() - m].to_vec();
                for k in (0..m).rev() {
                    items.push(
                        log.items[log.items.len() - 1 - k]
                            .clone()
                            .or_else(|| Some(rows[rows.len() - 1 - k].0.clone())),
                    );
                }
                items
            };
            if items.is_empty() {
                return None;
            }
            let b = raw_box(screen);
            Some(QueuedInput {
                agent: kind.into(),
                items,
                pullable: b == [PLACEHOLDER],
            })
        }
        "codex" => {
            let head = lines
                .iter()
                .rposition(|l| l.trim() == "• Queued follow-up inputs")?;
            let mut items: Vec<Option<String>> = vec![];
            let mut margin = 4;
            for l in &lines[head + 1..] {
                let t = l.trim();
                if t == "⇧← edit last queued message" {
                    break;
                }
                if let Some(text) = t.strip_prefix("↳ ") {
                    margin = l.chars().count() - t.chars().count() + 2;
                    items.push(Some(text.trim_end().into()));
                } else if !t.is_empty() && l.starts_with(&" ".repeat(margin)) && !items.is_empty() {
                    items
                        .last_mut()
                        .unwrap()
                        .as_mut()
                        .unwrap()
                        .push_str(&format!("\n{}", l[margin..].trim_end()));
                } else {
                    break;
                }
            }
            (!items.is_empty()).then(|| QueuedInput {
                agent: kind.into(),
                items,
                pullable: false,
            })
        }
        "grok" => {
            if !lines
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty())?
                .contains("Ctrl+;:queue")
            {
                return None;
            }
            let box_at = lines.iter().rposition(|l| l.trim().starts_with('╭'))?;
            let re = regex::Regex::new(r"^    #(\d+) (.*)$").ok()?;
            let last = lines[..box_at].iter().rposition(|l| re.is_match(l))?;
            let mut first = last;
            while first > 0 && re.is_match(lines[first - 1]) {
                first -= 1;
            }
            let mut items = vec![];
            for (i, l) in lines[first..=last].iter().enumerate() {
                let c = re.captures(l)?;
                if c[1].parse::<usize>().ok()? != i + 1 {
                    return None;
                }
                items.push(Some(c[2].trim_end().into()));
            }
            Some(QueuedInput {
                agent: kind.into(),
                items,
                pullable: false,
            })
        }
        _ => None,
    }
}
/// Only widths we can prove without font/emoji/grapheme assumptions. Unknown is refused.
fn columns(s: &str) -> Option<usize> {
    s.chars().try_fold(0, |n, c| {
        Some(
            n + match c as u32 {
                0x20..=0x7e => 1,
                0x3000..=0x3029
                | 0x3030..=0x303e
                | 0x3041..=0x3098
                | 0x309b..=0x30ff
                | 0x3400..=0x4dbf
                | 0x4e00..=0x9fff
                | 0xac00..=0xd7a3
                | 0xf900..=0xfaff
                | 0xff01..=0xff60
                | 0xffe0..=0xffe6 => 2,
                _ => return None,
            },
        )
    })
}
fn width(screen: &str) -> Option<usize> {
    screen
        .lines()
        .rev()
        .find(|l| rule(l))
        .and_then(|l| l.trim().chars().count().checked_sub(4))
}
fn shows(text: &str, screen: &str) -> bool {
    let Some(w) = width(screen) else {
        return false;
    };
    let lines: Vec<_> = screen.split('\n').collect();
    let Some(bottom) = lines.iter().rposition(|l| rule(l)) else {
        return false;
    };
    let room = lines.len().min(60).saturating_sub(lines.len() - bottom + 2);
    !text.is_empty()
        && !text.ends_with('\n')
        && text.split('\n').count() <= room
        && text.split('\n').all(|l| {
            columns(l).is_some_and(|n| n < w)
                && !l.ends_with(' ')
                && !(!l.is_empty() && l.chars().all(|c| ('\u{2500}'..='\u{257f}').contains(&c)))
        })
}
pub trait PullIO {
    fn identity(&self) -> Option<AgentIdentity>;
    fn working(&self) -> bool;
    fn active(&self) -> bool {
        self.working()
    }
    fn screen(&self) -> Result<String, String>;
    /// Checks the identity and input permission again immediately before sending the key.
    fn key(&self, key: &str, expect: &AgentIdentity) -> Result<(), String>;
    /// Durable write MUST finish before Ctrl-U. False means no clearing.
    fn keep(&self, text: &str) -> Result<(), String>;
    fn settle(&self) {
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullOutcome {
    Pulled(String),
    Interrupted(String),
    Partial(String, String),
    Refused(String),
    Failed(String),
}
pub fn pull(io: &impl PullIO, expect: &AgentIdentity, log: &ClaudeQueueLog) -> PullOutcome {
    let run = || -> Result<PullOutcome, String> {
        if expect.kind != "claude" {
            return Err(
                "Edit this queue in the agent terminal; its screen cannot prove the exact text"
                    .into(),
            );
        }
        if expect.session_id.as_deref().is_none_or(str::is_empty)
            || io.identity().as_ref() != Some(expect)
            || !io.working()
        {
            return Err("Agent session changed or is not working".into());
        }
        let before = io.screen()?;
        let q = read("claude", true, &before, log).ok_or("No queue on screen")?;
        if !q.pullable || log.items.is_empty() || log.items.iter().any(Option::is_none) {
            return Err("Input is not empty or exact log text is unavailable".into());
        }
        let expected = log
            .items
            .iter()
            .flatten()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        if q.items.iter().any(Option::is_none)
            || q.items
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
                != expected
            || !shows(&expected, &before)
        {
            return Err("Queued text cannot be checked exactly at this terminal width".into());
        }
        if io.identity().as_ref() != Some(expect) {
            return Err("Agent changed".into());
        }
        io.key("Up", expect)?;
        io.settle();
        let shown = match io.screen() {
            Ok(s) => s,
            Err(e) => return Ok(PullOutcome::Failed(e)),
        };
        if width(&shown) != width(&before) || raw_box(&shown).join("\n") != expected {
            return Ok(PullOutcome::Failed(
                "Pulled input differs from the exact log text; nothing cleared".into(),
            ));
        }
        // From this point return the kept text even when a later read/write fails.
        if let Err(e) = io.keep(&expected) {
            return Ok(PullOutcome::Partial(expected, e));
        }
        let mut cur = raw_box(&shown);
        let mut sent = false;
        let original_width = width(&shown);
        for _ in 0..(4 * cur.len() + 5) {
            let step = || -> Result<(Vec<String>, String), String> {
                if io.identity().as_ref() != Some(expect) {
                    return Err("Agent changed while clearing".into());
                }
                let s = io.screen()?;
                if width(&s) != original_width {
                    return Err("Terminal width changed while clearing".into());
                }
                Ok((raw_box(&s), s))
            };
            let (rows, after) = match step() {
                Ok(v) => v,
                Err(e) => return Ok(PullOutcome::Partial(expected, e)),
            };
            if sent {
                let mut next = cur.clone();
                if next.len() <= 1 {
                    next.clear();
                } else if next.last().is_some_and(String::is_empty) {
                    next.pop();
                } else {
                    *next.last_mut().unwrap() = String::new();
                }
                if next == [""] {
                    next.clear();
                }
                if next.is_empty()
                    && (rows.is_empty()
                        || rows == [""]
                        || (cur != [PLACEHOLDER] && rows == [PLACEHOLDER]))
                {
                    if after.to_lowercase().matches("interrupted").count()
                        > before.to_lowercase().matches("interrupted").count()
                        || !io.active()
                    {
                        return Ok(PullOutcome::Interrupted(expected));
                    }
                    return Ok(PullOutcome::Pulled(expected));
                }
                if !next.is_empty() && rows == next {
                    cur = next;
                    sent = false;
                } else if rows == cur {
                    io.settle();
                    continue;
                } else {
                    return Ok(PullOutcome::Partial(
                        expected,
                        "Input changed while clearing".into(),
                    ));
                }
            } else if rows != cur {
                return Ok(PullOutcome::Partial(
                    expected,
                    "Input changed while clearing".into(),
                ));
            }
            if io.identity().as_ref() != Some(expect) {
                return Ok(PullOutcome::Partial(expected, "Agent changed".into()));
            }
            if let Err(e) = io.key("ctrl+u", expect) {
                return Ok(PullOutcome::Partial(expected, e));
            }
            sent = true;
            io.settle();
        }
        Ok(PullOutcome::Partial(
            expected,
            "Input did not finish clearing; check the terminal before sending".into(),
        ))
    };
    run().unwrap_or_else(PullOutcome::Refused)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn log_preserves_blank_lines_and_scalars() {
        let mut q = ClaudeQueueLog::default();
        q.feed(&serde_json::json!({"type":"queue-operation","operation":"enqueue","content":" a\n\ne\u{301}\n"}));
        assert_eq!(q.items, [Some(" a\n\ne\u{301}\n".into())]);
        q.feed(&serde_json::json!({"type":"queue-operation","operation":"remove","content":" a\n\ne\u{301}\n"}));
        assert!(q.items.is_empty());
    }
    #[test]
    fn uncertain_width_or_trailing_blank_is_refused() {
        let screen = "──────────────\n❯ Press up to edit queued messages\n──────────────";
        assert!(!shows("e\u{301}", screen));
        assert!(!shows("😀", screen));
        assert!(!shows("x\n", screen));
        assert!(!shows("x ", screen));
        assert!(!shows("x", "unknown"));
    }
    #[test]
    fn codex_and_grok_are_display_only() {
        let q = read(
            "codex",
            true,
            "• Queued follow-up inputs\n  ↳ hello\n⇧← edit last queued message",
            &ClaudeQueueLog::default(),
        )
        .unwrap();
        assert_eq!(q.items, [Some("hello".into())]);
        assert!(!q.pullable);
        let q = read(
            "grok",
            true,
            "    #1 hello\n╭───\n│ ❯\nCtrl+;:queue",
            &ClaudeQueueLog::default(),
        )
        .unwrap();
        assert_eq!(q.items, [Some("hello".into())]);
        assert!(!q.pullable);
    }
    #[test]
    fn blank_line_from_screen_requires_log_confirmation() {
        let screen="❯ old\n\n❯ newest\nctrl+enter to send now\n──────────────\n❯ Press up to edit queued messages\n──────────────";
        assert_eq!(
            read("claude", true, screen, &ClaudeQueueLog::default())
                .unwrap()
                .items,
            [Some("newest".into())]
        );
    }
}

#[cfg(test)]
mod pull_tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
    };
    struct IO {
        screens: RefCell<VecDeque<String>>,
        calls: RefCell<Vec<String>>,
        changed: Cell<bool>,
        change_on_keep: bool,
    }
    fn expect() -> AgentIdentity {
        AgentIdentity {
            pane_id: "pane".into(),
            name: "agent".into(),
            kind: "claude".into(),
            session_id: Some("session".into()),
        }
    }
    impl PullIO for IO {
        fn identity(&self) -> Option<AgentIdentity> {
            if self.changed.get() {
                None
            } else {
                Some(expect())
            }
        }
        fn working(&self) -> bool {
            true
        }
        fn screen(&self) -> Result<String, String> {
            let mut screens = self.screens.borrow_mut();
            Ok(if screens.len() > 1 {
                screens.pop_front().unwrap()
            } else {
                screens.front().cloned().unwrap_or_default()
            })
        }
        fn key(&self, key: &str, _: &AgentIdentity) -> Result<(), String> {
            self.calls.borrow_mut().push(key.into());
            Ok(())
        }
        fn keep(&self, text: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("keep:{text}"));
            if self.change_on_keep {
                self.changed.set(true);
            }
            Ok(())
        }
        fn settle(&self) {}
    }
    fn screen(input: &str) -> String {
        format!("❯ hello\nctrl+enter to send now\n────────────────────────────────────────\n❯ {input}\n────────────────────────────────────────\nfooter")
    }
    fn io(after: Vec<String>, change: bool) -> IO {
        let mut screens = VecDeque::from([screen(PLACEHOLDER), screen("hello")]);
        screens.extend(after);
        IO {
            screens: RefCell::new(screens),
            calls: RefCell::default(),
            changed: Cell::new(false),
            change_on_keep: change,
        }
    }
    fn log() -> ClaudeQueueLog {
        ClaudeQueueLog {
            items: vec![Some("hello".into())],
        }
    }
    #[test]
    fn saves_before_clearing_without_interrupt_keys() {
        let io = io(vec![screen("hello"), screen(PLACEHOLDER)], false);
        assert_eq!(
            pull(&io, &expect(), &log()),
            PullOutcome::Pulled("hello".into())
        );
        assert_eq!(*io.calls.borrow(), ["Up", "keep:hello", "ctrl+u"]);
    }
    #[test]
    fn changing_agent_after_keep_returns_recovery_without_clearing() {
        let io = io(vec![screen("hello")], true);
        assert!(matches!(pull(&io,&expect(),&log()),PullOutcome::Partial(t,_) if t=="hello"));
        assert_eq!(*io.calls.borrow(), ["Up", "keep:hello"]);
    }
    #[test]
    fn changed_text_or_width_stops_clearing() {
        for after in [
            screen("owner typed"),
            screen("hello").replace(
                "────────────────────────────────────────",
                "────────────────────",
            ),
        ] {
            let io = io(vec![after], false);
            assert!(matches!(pull(&io,&expect(),&log()),PullOutcome::Partial(t,_) if t=="hello"));
            assert_eq!(*io.calls.borrow(), ["Up", "keep:hello"]);
        }
    }
    #[test]
    fn exact_blank_lines_are_not_collapsed() {
        let before="❯ a\n\n  b\nctrl+enter to send now\n────────────────────────────────────────\n❯ Press up to edit queued messages\n────────────────────────────────────────\nfooter";
        let log = ClaudeQueueLog {
            items: vec![Some("a\n\nb".into())],
        };
        assert_eq!(read("claude", true, before, &log).unwrap().items, log.items);
    }
}

#[cfg(test)]
mod partial_clear_test {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};
    struct IO {
        screens: RefCell<VecDeque<String>>,
        calls: RefCell<Vec<String>>,
    }
    fn identity() -> AgentIdentity {
        AgentIdentity {
            pane_id: "p".into(),
            name: "agent".into(),
            kind: "claude".into(),
            session_id: Some("s".into()),
        }
    }
    fn screen(input: &str) -> String {
        format!("❯ one\n  two\nctrl+enter to send now\n────────────────────────────────────────\n❯ {input}\n────────────────────────────────────────\nfooter")
    }
    impl PullIO for IO {
        fn identity(&self) -> Option<AgentIdentity> {
            Some(identity())
        }
        fn working(&self) -> bool {
            true
        }
        fn screen(&self) -> Result<String, String> {
            let mut q = self.screens.borrow_mut();
            Ok(if q.len() > 1 {
                q.pop_front().unwrap()
            } else {
                q.front().unwrap().clone()
            })
        }
        fn key(&self, key: &str, _: &AgentIdentity) -> Result<(), String> {
            self.calls.borrow_mut().push(key.into());
            Ok(())
        }
        fn keep(&self, text: &str) -> Result<(), String> {
            self.calls.borrow_mut().push(format!("keep:{text}"));
            Ok(())
        }
        fn settle(&self) {}
    }
    #[test]
    fn partial_clear_keeps_the_entire_original_message() {
        let io = IO {
            screens: RefCell::new(
                [
                    screen(PLACEHOLDER),
                    screen("one\n  two"),
                    screen("one\n  two"),
                    screen("one\n  "),
                    screen("owner changed input"),
                ]
                .into(),
            ),
            calls: RefCell::default(),
        };
        let result = pull(
            &io,
            &identity(),
            &ClaudeQueueLog {
                items: vec![Some("one\ntwo".into())],
            },
        );
        assert!(matches!(result,PullOutcome::Partial(t,_) if t=="one\ntwo"));
        assert_eq!(
            *io.calls.borrow(),
            ["Up", "keep:one\ntwo", "ctrl+u", "ctrl+u"]
        );
    }
}
