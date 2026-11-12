//! Swift: `Cards.swift`. `card`, `approve`, `answer`, `send`: the approval and question cards and the composer.
//!
//! Reading is safe any time. Keys go out only on the owner's action (`approve`, `answer`), and only after a fresh
//! read shows the same request the mod showed: the mod passes the card's `request` id back, and a different card on
//! screen is refused with `changed`, nothing sent.

use crate::answered::{Claim, FeedAnswered};
use crate::rows::{self, FeedQuestion};
use crate::tail;
use herdr_core::approval::{self, PaneState};
use herdr_core::hc::HcClient;
use herdr_core::keygate::{self, KeyGate};
use herdr_core::*;
use herdr_core::question;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Card {
    #[serde(rename = "type")]
    pub t: &'static str,
    /// "approval", "question", or None (no card).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The id to pass back to `approve` / `answer`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<FeedQuestion>,
    /// Questions known only by their header. `answer` reads them first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unread: Option<usize>,
    /// An approval this sidecar already answered that is still on screen: no buttons.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered: Option<bool>,
}

impl Card {
    pub fn none() -> Card {
        Card { t: "card", ..Default::default() }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Done {
    #[serde(rename = "type")]
    pub t: &'static str,
    /// "sent", "unconfirmed", "changed", "no-identity", "answered", "failed", "opens_screen" (`send`: a screen
    /// command sent bare; nothing was sent), "box_not_empty" (`send`: the agent's input box has text; nothing was
    /// sent).
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Done {
    pub fn new(result: &str, message: Option<&str>) -> Done {
        Done { t: "done", result: result.into(), message: message.map(String::from) }
    }
}

/// As the hub gates a paired device's keys: before each key the pane must still show the agent read now (same kind
/// and the same program). No agent: fail closed.
pub fn gate(pane: &Pane) -> KeyGate {
    let has = pane.agent.is_some();
    KeyGate::new(pane.agent.clone(), Some(Arc::new(move || has)))
}

pub fn agent_changed() -> Done {
    Done::new("changed", Some("another agent is in this pane now; nothing was sent"))
}
/// The same agent, in another log: the mod and the page show the new one.
pub fn new_conversation() -> Done {
    Done::new("changed", Some("the agent started a new conversation; the chat now shows it; nothing was sent"))
}
pub fn no_identity() -> Done {
    Done::new(
        "no-identity",
        Some("Flockbook cannot tell this agent from another program in this terminal, so it sends nothing. Use the terminal."),
    )
}
/// Claude Code's prompt suggestions are on (its default) unless its user settings say `"promptSuggestionEnabled": false`.
pub fn claude_suggests(home: Option<&Path>) -> bool {
    let v = home.and_then(|h| std::fs::read(h.join(".claude/settings.json")).ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    v.and_then(|v| v["promptSuggestionEnabled"].as_bool()).unwrap_or(true)
}

pub const BOX_NOT_EMPTY: &str = "The agent's box has text in it. Clear it in the terminal first. Nothing was sent.";
pub fn box_not_empty() -> Done {
    Done::new("box_not_empty", Some(BOX_NOT_EMPTY))
}
/// Why the gate said no: the agent changed, or it has no identity to check, which no retry fixes.
pub fn refused(pane: &Pane) -> Done {
    if pane.agent.as_ref().map(KeyGate::has_identity) == Some(true) {
        agent_changed()
    } else {
        no_identity()
    }
}
/// An answer the key gate stopped. Another agent, or no identity: as `refused`. The same agent with another screen
/// (a render that never came, or another question): its own result, and "nothing was sent" only when no key went.
pub fn answer_stop(pane: &Pane, e: &HerdrError) -> Done {
    match e {
        HerdrError::ScreenStop { keys_sent: false } => {
            Done::new("screen_changed", Some("The question on screen changed before the answer went in. Nothing was sent."))
        }
        HerdrError::ScreenStop { keys_sent: true } => Done::new(
            "screen_changed",
            Some("The question on screen changed while answering. Some keys may already be in: check the terminal."),
        ),
        _ => refused(pane),
    }
}
pub fn already_answered() -> Done {
    Done::new("answered", Some("this approval was already answered; waiting for the agent to move on"))
}
/// Another press of this prompt (another view, or a double key) was being handled: nothing went out from this one.
pub fn busy() -> Done {
    Done::new("busy", Some("Another press was being handled at the same moment, so this one sent nothing. Press again."))
}

pub enum Current {
    /// `pending`: the log's pending call, passed to `respond_if_open` (which matches it with the box again).
    Approval { id: String, title: String, command: String, tool: String, pending: Option<PermissionRequest> },
    Question(QuestionRequest),
}

pub fn current<S: PaneScreen + ?Sized>(pane: &Pane, screen: &S, log: Option<&Path>) -> Option<Current> {
    look(pane, screen, log).0
}

/// `.1`: the screen was read and the approval rules ran on it (a failed read is no evidence of anything).
pub fn look<S: PaneScreen + ?Sized>(pane: &Pane, screen: &S, log: Option<&Path>) -> (Option<Current>, bool) {
    let Some(agent) = &pane.agent else { return (None, false) };
    let items = log.and_then(|l| tail::read(l, None, tail::BYTES).ok()).map(|r| tail::items(&r.lines)).unwrap_or_default();
    let (mut perm, mut question): (Option<PermissionRequest>, Option<QuestionRequest>) = (None, None);
    for it in items.iter().rev() {
        match &it.kind {
            ItemKind::PermissionRequest(p) if perm.is_none() && p.state == PermState::Pending => perm = Some(p.clone()),
            ItemKind::Question(q) if question.is_none() && q.state == QuestionState::Pending => question = Some(q.clone()),
            _ => {}
        }
    }
    // The app's rules: the card is the prompt box on screen; the log's call only when it is that box (same tool and
    // command). A log that lags the screen still names the call before, and its id must never answer the next prompt.
    let mut read = false;
    if approval::check_approval(Some(agent), perm.is_some()) {
        if let Ok(text) = screen.visible_text(&pane.id) {
            read = true;
            let state = PaneState {
                agent_kind: agent.kind.clone(),
                blocked: agent.status == AgentStatus::Blocked,
                pending_tool_use_id: perm.as_ref().map(|p| p.tool_use_id.clone()),
                screen: text.clone(),
                pending_tool: perm.as_ref().map(|p| p.tool_name.clone()),
                pending_command: perm.as_ref().map(|p| p.command.clone()),
            };
            if approval::prompt(&state, true).is_some() {
                let on_screen = approval::screen_request(&text, &agent.kind);
                if let Some(card) = approval::card_request(perm.as_ref(), on_screen.as_ref()) {
                    let title = if card.title.is_empty() { "Approval needed".to_string() } else { card.title.clone() };
                    return (
                        Some(Current::Approval { id: card.tool_use_id, title, command: card.command, tool: card.tool_name, pending: perm }),
                        true,
                    );
                }
            }
        }
    }
    // A question in the log: answered by a message (Codex async) any time, or by keys while the agent waits.
    if let Some(q) = question {
        if q.delivery == Delivery::Message || agent.status == AgentStatus::Blocked {
            return (Some(Current::Question(q)), read);
        }
    }
    if approval::read_question(Some(agent)) {
        if let Ok(Some(q)) = question::pending_question(screen, &pane.id) {
            return (Some(Current::Question(q)), read);
        }
    }
    (None, read)
}

/// `all`: for a question with more behind it, walk the picker's tabs first (keys, on the owner's click).
pub fn card<S: PaneScreen + ?Sized>(pane: &Pane, screen: &S, log: Option<&Path>, all: bool, cache: Option<&Path>) -> Card {
    let (mut now, read) = look(pane, screen, log);
    if all {
        if let Some(Current::Question(q)) = &now {
            if q.unread() > 0 {
                if let Ok(Some(full)) = question::load_all_questions(screen, &pane.id, Some(&gate(pane)), Some(q)) {
                    now = Some(Current::Question(full));
                }
            }
        }
    }
    // The claims of prompts that surely left the screen go (only after a read that worked); one still there shows
    // as answered.
    let answered = FeedAnswered::new(cache);
    let blocked = pane.agent.as_ref().is_some_and(|a| a.status == AgentStatus::Blocked);
    if read {
        match &now {
            Some(Current::Approval { command, tool, .. }) => answered.seen(&pane.id, Some((tool, command))),
            _ => answered.seen(&pane.id, None),
        }
    }
    match now {
        Some(Current::Approval { id, title, command, tool, .. }) => {
            let claimed = answered.is_claimed(&pane.id, &id, &tool, &command, blocked, SystemTime::now());
            Card {
                t: "card",
                kind: Some("approval".into()),
                request: Some(id),
                title: Some(title),
                command: Some(rows::cut(&command, rows::TEXT_CAP)),
                tool: Some(tool),
                answered: claimed.then_some(true),
                ..Default::default()
            }
        }
        Some(Current::Question(q)) => {
            let row = rows::row(&ConversationItem { id: q.tool_use_id.clone(), kind: ItemKind::Question(q.clone()), ..Default::default() });
            Card {
                t: "card",
                kind: Some("question".into()),
                request: Some(q.tool_use_id.clone()),
                question: row.question,
                unread: Some(q.unread()),
                ..Default::default()
            }
        }
        None => Card::none(),
    }
}

pub fn approve<S: PaneScreen + ?Sized>(pane: &Pane, screen: &S, log: Option<&Path>, request: &str, allow: bool, cache: Option<&Path>) -> Done {
    let changed = || Done::new("changed", Some("the approval on screen is not the one shown"));
    let Some(agent) = &pane.agent else { return changed() };
    let Some(Current::Approval { id, command, tool, pending, .. }) = current(pane, screen, log) else { return changed() };
    if id != request {
        return changed();
    }
    if !keygate::passes(screen, Some(&gate(pane)), &pane.id, None, None) {
        return refused(pane);
    }
    // One answer per prompt, across runs and views: claimed before the key; kept when the key went out or may still
    // land; given back when nothing went out.
    let answered = FeedAnswered::new(cache);
    let blocked = agent.status == AgentStatus::Blocked;
    match answered.claim(&pane.id, &id, &tool, &command, blocked, SystemTime::now()) {
        Claim::Got => {}
        Claim::Taken => return already_answered(),
        Claim::Busy => return busy(),
    }
    // The screen is read again right before the key, and the key goes only while the box is still this id.
    match approval::respond_if_open(screen, &pane.id, &id, allow, &agent.kind, blocked, pending.as_ref(), Some(&gate(pane))) {
        Ok(true) => Done::new("sent", None),
        Ok(false) => {
            answered.release(&pane.id, &id);
            Done::new("changed", Some("the prompt on screen is not the one shown"))
        }
        Err(HerdrError::Timeout(_)) => {
            // hc may still hold the key for its typing-quiet rule: not "nothing sent". The claim stays while blocked.
            answered.hold(&pane.id, &id);
            Done::new("unconfirmed", Some("no answer in time; the key may still arrive"))
        }
        Err(_) => {
            answered.release(&pane.id, &id);
            Done::new("failed", Some("herdr-core did not take the keys"))
        }
    }
}

/// `[{"picks":[0]}, {"picks":[], "other":"text"}]`: one per question, option indexes from 0.
pub fn parse_answers(json: &str) -> Option<Vec<QuestionAnswer>> {
    let v: Value = serde_json::from_str(json).ok()?;
    let a = v.as_array()?;
    let mut out = vec![];
    for o in a {
        let o = o.as_object()?;
        let picks = match o.get("picks") {
            None => vec![],
            Some(p) => {
                let arr = p.as_array()?;
                let mut v = vec![];
                for x in arr {
                    // Swift `as? [Int]`: whole numbers only. A negative index is out of range, as Swift then finds it
                    // (no option has it): it maps to `usize::MAX`, which every answer path refuses. Nothing is sent.
                    let n = x.as_i64().or_else(|| x.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))?;
                    v.push(usize::try_from(n).unwrap_or(usize::MAX));
                }
                v
            }
        };
        let other = o.get("other").and_then(Value::as_str).map(String::from);
        out.push(QuestionAnswer { picks, other });
    }
    Some(out)
}

pub fn answer<S: PaneScreen + ?Sized>(pane: &Pane, screen: &S, log: Option<&Path>, request: &str, answers: &[QuestionAnswer]) -> Done {
    let Some(Current::Question(mut q)) = current(pane, screen, log) else {
        return Done::new("changed", Some("the question on screen is not the one shown"));
    };
    if q.tool_use_id != request {
        return Done::new("changed", Some("the question on screen is not the one shown"));
    }
    let g = gate(pane);
    let r = (|| -> Result<Option<Done>, HerdrError> {
        if q.unread() > 0 {
            match question::load_all_questions(screen, &pane.id, Some(&g), Some(&q))? {
                Some(all) => q = all,
                None => return Ok(Some(Done::new("changed", Some("the question went away")))),
            }
        }
        if answers.len() != q.questions.len() {
            return Ok(Some(Done::new("failed", Some(&format!("{} answers for {} questions", answers.len(), q.questions.len())))));
        }
        question::answer_question(screen, &pane.id, &q, answers, Some(&g))?;
        Ok(None)
    })();
    match r {
        Ok(Some(d)) => d,
        Ok(None) => Done::new("sent", None),
        Err(e @ (HerdrError::KeyGateStop | HerdrError::ScreenStop { .. })) => answer_stop(pane, &e),
        Err(HerdrError::Question(QuestionFailure::NotOnScreen)) => Done::new("changed", Some("the question is not on screen")),
        Err(e) => Done::new("failed", Some(&e.to_string())),
    }
}

/// Clears the line Claude Code's box shows (ctrl+e ctrl+u: a typed "/word" goes, a dim suggestion is untouched) and
/// reads the box again: true when it is down to one row, so the text pastes over any dim text and nothing typed is
/// left in front of it. Tested live on Claude Code 2.1: a typed "/clear" closed with Esc, then a send, gave "/clear…"
/// as one unknown command without this.
fn clear_box_line(client: &HcClient, pane_id: &str) -> bool {
    if client.send_named_keys(&["ctrl+e".to_string(), "ctrl+u".to_string()], pane_id).is_err() {
        return false;
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    client.visible_text(pane_id).ok().is_some_and(|s| crate::queue::box_one_line(&s))
}

/// The composer: the text, with each file in `files` named in it. `was`: the locate cache entry this terminal had
/// before the send looked its log up again.
#[allow(clippy::too_many_arguments)]
pub fn send(pane: &Pane, client: &HcClient, text: &str, files: &[String], log: Option<&Path>, expect_kind: Option<&str>, expect_path: Option<&str>, was: Option<&crate::locate::Entry>) -> Done {
    // The agent the owner was reading: another agent, or a new conversation, in this terminal now gets nothing.
    if let Some(k) = expect_kind {
        if Some(k) != pane.agent.as_ref().map(|a| a.kind.as_str()) {
            return agent_changed();
        }
    }
    if let Some(p) = expect_path {
        if Some(p.to_string()) != log.map(|l| l.to_string_lossy().into_owned()) {
            // The same agent process wrote the log the chat showed (`/clear`, `/resume`), and a log was found now: a new
            // conversation. Still nothing is sent: the text was written for the conversation the chat showed.
            let kind = pane.agent.as_ref().map(|a| a.kind.as_str()).unwrap_or("");
            if log.is_some() && was.is_some_and(|w| crate::locate::same_agent(w, pane, kind, p)) {
                return new_conversation();
            }
            return Done::new("changed", Some("another agent or conversation is in this pane now; nothing was sent"));
        }
    }
    // A screen command sent bare ("/model"): the chat cannot show its screen, so it is not sent (the mod checks too;
    // this covers the phone page and other callers). With files the text is not bare.
    if files.is_empty() {
        if let Some(note) = pane.agent.as_ref().and_then(|a| crate::commands::opens_screen(&a.kind, text, std::env::var_os("HOME").map(std::path::PathBuf::from).as_deref())) {
            return Done::new("opens_screen", Some(&note));
        }
    }
    if !keygate::passes(client, Some(&gate(pane)), &pane.id, None, None) {
        return refused(pane);
    }
    // The text would go after what is in the agent's box ("/" + "/status" = "//status"). A box not found, an agent
    // not read, or no screen: it goes as before.
    let kind = pane.agent.as_ref().map(|a| a.kind.as_str()).unwrap_or("");
    let suggests = kind == "claude" && claude_suggests(std::env::var_os("HOME").map(std::path::PathBuf::from).as_deref());
    let screen = client.visible_text(&pane.id).ok();
    if screen.as_deref().and_then(|s| crate::queue::box_has_text(kind, s, suggests)) == Some(true) {
        // Claude Code's dim suggestion can be a command ("/clear"): read as typed, it stopped every send. A lone
        // "/word" with no command list above the box is taken for it; its line is cleared and the text goes.
        let suggestion = suggests && screen.as_deref().and_then(crate::queue::suggested_slash).is_some();
        if !suggestion || !clear_box_line(client, &pane.id) {
            return box_not_empty();
        }
    }
    let r = if files.is_empty() { client.deliver(text, &pane.id) } else { client.deliver_files(text, &pane.id, files) };
    match r {
        DeliveryOutcome::Sent => Done::new("sent", None),
        DeliveryOutcome::Unconfirmed => Done::new("unconfirmed", Some("no answer in time; the text may still arrive")),
        DeliveryOutcome::Failed => Done::new("failed", Some("herdr-core did not take the text")),
    }
}
