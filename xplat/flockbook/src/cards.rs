//! Approval card and question card: what the conversation column shows at the bottom while the selected pane is
//! blocked. DESIGN.md section 8 (approval card, e2, radius 14).
//!
//! # Placement (same as the Swift app)
//!
//! - Approval: a floating card ABOVE the composer. Allow = `ALLOW_ONCE` (or the digit of the "always allow" row when
//!   the box is ticked), Deny = Escape. NO keyboard shortcuts, like Swift: a permission is answered only by
//!   clicking a button.
//! - Question: a card IN PLACE of the composer. Digits 1-9 pick, arrows move, Enter sends (or goes to the next
//!   question), Esc goes back one question or leaves "Other".
//!
//! # Detection (same as Swift `PermissionGate` and `TabScreen`)
//!
//! - Approval: the agent is blocked, the last conversation item is a pending permission request, and the pane's
//!   screen shows the prompt marker (`herdr_core::screen_shows_prompt`). Only the screen tells a prompt from a
//!   running tool. The card binds only to a lone Bash call of a Claude agent (no other call still open), and the screen must show
//!   exactly its command as a Bash prompt ([`actions::request_shows`]); else the card is `unbound` (buttons off, a
//!   notice to open the terminal). With no [`PaneActions`] installed (read-only) the screen cannot be read: the card
//!   shows with its buttons off. When the log has no pending request (the agent logs the call late, or not at all), a
//!   prompt on screen makes the card by itself ([`actions::screen_request`], Grok's framed box), through the same binder.
//! - Question: Claude's open picker read from the screen (`question::claude_screen`; Claude logs the call only after
//!   the answer), else a pending question item in the log (Codex).
//!
//! # Mutating calls
//!
//! Every key goes out through [`PaneActions`] (see `actions.rs`). The live implementation is installed only by
//! `main.rs` through [`install_live`]. Tests use a recording fake. Live write tests target `c9-input-test` only.
//!
//! The screen is read off the UI thread (every 1.5 s while the selected pane is blocked) and picked up by
//! [`on_refresh`], which `App::refresh` calls. The detected card is cached in `state.cards.card`, so `view` does no
//! parsing.

pub mod flow;
pub mod keys;
mod view;

use crate::actions::{self, AlwaysAllow, PaneActions};
use crate::{App, Message};
use flow::Flow;
use herdr_core::{AgentStatus, ItemKind, PermState, QuestionRequest, QuestionState};
use iced::Task;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub use view::{bottom, detail};

/// The id of the "Other" text field (focus target).
pub const OTHER_ID: &str = "cards-other";
/// How often a blocked pane's screen is read.
const POLL: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenRead {
    pub pane: String,
    pub text: String,
    pub identity: Option<herdr_core::AgentIdentity>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Approval {
    pub key: String,
    pub pane: String,
    pub agent: String,
    pub title: String,
    pub command: String,
    /// The call's own description, which the prompt may show under the command.
    pub detail: String,
    pub always: Option<AlwaysAllow>,
    /// The prompt as the screen showed it when the card was made. A send checks the screen still shows exactly this.
    pub identity: Option<actions::PromptIdentity>,
    /// The screen shows another prompt than the one the owner first saw on this card: the buttons are off and the card
    /// says so. They come back if the screen shows the first prompt again.
    pub stale: bool,
    /// The log's command is not in the screen's request region (a plan prompt, another tool, a lagging log): the buttons
    /// are off and the card says so. It goes live when the screen shows this request.
    pub unbound: bool,
    /// False when no actions are installed (read-only): the buttons are off.
    pub can_act: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Card {
    Approval(Approval),
    Question { key: String, pane: String, agent: String, q: QuestionRequest, can_send: bool },
    /// The answer went out; the card is replaced by one line until the agent moves on.
    Sent { agent: String },
}

#[derive(Default)]
pub struct CardsState {
    /// `None`: read-only (no card can send keys).
    pub actions: Option<Arc<dyn PaneActions>>,
    /// The latest read of the selected pane's screen.
    pub screen: Option<ScreenRead>,
    inbox: Arc<Mutex<Option<ScreenRead>>>,
    inflight: Arc<AtomicBool>,
    last_poll: Option<Instant>,
    /// The card to show, recomputed on refresh and after each change.
    pub card: Option<Card>,
    /// Question answers in progress, with the card key they belong to.
    pub flow: Option<(String, Flow)>,
    /// The "Always allow" box.
    pub always: bool,
    /// A send is on its way.
    pub busy: bool,
    pub problem: Option<String>,
    /// Card keys already answered from here.
    pub sent: HashSet<String>,
    /// The prompt the owner first saw on the current approval card (card key, identity). Sends check against this one.
    pub pinned: Option<(String, actions::PromptIdentity)>,
    /// The "Other" field has the keyboard: digits and arrows type instead of pick.
    pub other_active: bool,
    pub agent_pin: Option<(String, herdr_core::AgentIdentity)>,
}

impl CardsState {
    pub fn flow(&self) -> Option<&Flow> {
        match (&self.flow, &self.card) {
            (Some((k, f)), Some(Card::Question { key, .. })) if k == key => Some(f),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKey {
    /// `chord`: Cmd or Ctrl is held.
    Enter { chord: bool },
    Escape,
    Up,
    Down,
    Space,
    /// 1..=9.
    Digit(usize),
}

#[derive(Debug, Clone)]
pub enum CardsMsg {
    Key(CardKey),
    Allow,
    Deny,
    Always(bool),
    Pick(usize),
    ActivateOther,
    OtherInput(String),
    Submit,
    Back,
    /// Result of a send: `error` is `None` when the keys went out.
    Done { key: String, error: Option<String> },
}

/// Installs the live actions for `session`. MUTATING: only called from the live path in `main.rs`.
#[cfg(unix)]
pub fn install_live(app: &mut App, session: &herdr_core::HerdrSession) {
    app.state.cards.actions = Some(Arc::new(actions::LiveActions::new(session)));
}

// ---------------------------------------------------------------- detection

/// The last conversation item that is not a status line.
fn last_item(app: &App) -> Option<&ItemKind> {
    app.state.conversation_items().iter().rev().map(|i| &i.kind).find(|k| !matches!(k, ItemKind::Status(_)))
}

/// What the bottom of the conversation should show for the selected pane. Pure: reads state only.
pub fn detect(app: &App) -> Option<Card> {
    let st = &app.state.cards;
    let pane = app.state.selected_pane()?;
    let agent = pane.agent.as_ref()?;
    let last = last_item(app);
    let screen = st.screen.as_ref().filter(|s| s.pane == pane.id).map(|s| s.text.as_str());
    let card = (|| {
        if agent.status == AgentStatus::Blocked {
            if agent.kind == "grok" {
                if let Some((b, identity)) = screen.and_then(|s| Some((herdr_core::approval::grok_box(s)?, actions::grok_identity(s)?))) {
                    return Some(Card::Approval(Approval {
                        key: format!("{}:{}", pane.id, actions::screen_key(&identity.request)), pane: pane.id.clone(), agent: agent.name.clone(),
                        title: b.title, command: b.command, detail: String::new(), always: None, identity: Some(identity), stale: false, unbound: false, can_act: st.actions.is_some(),
                    }));
                }
            }
            // A pending request in the log is the request the owner is shown. The screen only says whether that request
            // is on screen ([`actions::request_shows`]): a screen showing a different command never binds it, and it is
            // never replaced by a request made up from the screen. Screen-only prompts are the fallback for the modes where
            // the agent logs the call late (or not at all).
            let logged = match last {
                Some(ItemKind::PermissionRequest(p)) if p.state == PermState::Pending => Some(p),
                _ => None,
            };
            if let Some(p) = logged {
                let (can_act, always) = match (&st.actions, screen) {
                    (None, _) => (false, None),
                    (Some(_), Some(text)) if herdr_core::screen_shows_prompt(text) => (true, actions::always_allow(text)),
                    // Not read yet, or no prompt on screen (a running tool): no card.
                    (Some(_), _) => return None,
                };
                let identity = screen.filter(|_| can_act).and_then(actions::prompt_identity);
                // Only a lone Bash call can bind: with parallel calls the screen may show another call's prompt.
                let bindable = agent.kind == "claude" && p.tool_name == "Bash" && p.open_calls == 1;
                let unbound = can_act && !(bindable && identity.as_ref().is_some_and(|id| actions::request_shows(id, &p.command, &p.detail)));
                return Some(Card::Approval(Approval {
                    key: format!("{}:{}", pane.id, p.tool_use_id),
                    pane: pane.id.clone(),
                    agent: agent.name.clone(),
                    title: p.title.clone(),
                    command: p.command.clone(),
                    detail: p.detail.clone(),
                    always,
                    identity,
                    stale: false,
                    unbound,
                    can_act: can_act && !unbound,
                }));
            }
            if agent.kind == "claude" {
                if let Some((p, identity)) = screen.and_then(|s| Some((actions::screen_request(s)?, actions::prompt_identity(s)?))) {
                    return Some(Card::Approval(Approval {
                        key: format!("{}:{}", pane.id, p.tool_use_id), pane: pane.id.clone(), agent: agent.name.clone(),
                        title: p.title, command: p.command, detail: p.detail, always: screen.and_then(actions::always_allow), identity: Some(identity), stale: false, unbound: false, can_act: st.actions.is_some(),
                    }));
                }
            }
            if agent.kind == "claude" {
                if let Some(shown) = screen.and_then(herdr_core::question::claude_screen) {
                    let q = herdr_core::question::screen_request(shown.headers, vec![shown.question]);
                    let can_send = st.actions.is_some() && q.unread() == 0;
                    return Some(Card::Question { key: format!("{}:{}", pane.id, q.tool_use_id), pane: pane.id.clone(), agent: agent.name.clone(), q, can_send });
                }
            }
        }
        if agent.status != AgentStatus::Working {
            if let Some(ItemKind::Question(q)) = last {
                if q.state == QuestionState::Pending {
                    let can_send = st.actions.is_some() && q.delivery == herdr_core::Delivery::Picker && q.unread() == 0;
                    return Some(Card::Question { key: format!("{}:{}", pane.id, q.tool_use_id), pane: pane.id.clone(), agent: agent.name.clone(), q: q.clone(), can_send });
                }
            }
        }
        None
    })();
    match card {
        Some(Card::Approval(a)) if st.sent.contains(&a.key) => Some(Card::Sent { agent: a.agent }),
        Some(Card::Question { key, agent, .. }) if st.sent.contains(&key) => Some(Card::Sent { agent }),
        other => other,
    }
}

/// Recomputes the cached card and keeps the question flow in step with it.
pub fn recompute(app: &mut App) {
    let mut card = detect(app);
    let st = &mut app.state.cards;
    // The identity of an approval card is the prompt seen when its key first appeared. A later, different screen under the
    // same key (the log lags the screen) turns the card stale: no key may go to a prompt the owner did not review.
    if let Some(Card::Approval(a)) = card.as_mut() {
        // An unbound card pins nothing: the first prompt the owner can act on is the one that counts.
        if let Some(fresh) = a.identity.clone().filter(|_| !a.unbound) {
            let seen = match &st.pinned {
                Some((k, id)) if *k == a.key => id.clone(),
                _ => {
                    st.pinned = Some((a.key.clone(), fresh.clone()));
                    fresh.clone()
                }
            };
            if fresh != seen {
                a.stale = true;
                a.can_act = false;
            }
            a.identity = Some(seen);
        }
    } else if !matches!(card, Some(Card::Sent { .. })) {
        st.pinned = None;
    }
    match &card {
        Some(Card::Question { key, q, .. }) => {
            if st.flow.as_ref().map(|(k, _)| k) != Some(key) {
                st.flow = Some((key.clone(), Flow::new(&q.questions, &q.headers)));
                st.other_active = false;
                st.problem = None;
            }
        }
        _ => {
            st.flow = None;
            st.other_active = false;
        }
    }
    if !matches!(card, Some(Card::Approval(_))) {
        st.always = false;
    }
    if let Some(c) = &card {
        let key = card_key(c);
        if st.agent_pin.as_ref().map(|(k, _)| k.as_str()) != Some(key) {
            st.agent_pin = st.screen.as_ref().and_then(|s| s.identity.clone()).map(|id| (key.to_string(), id));
        }
    } else { st.agent_pin = None; }
    if st.card != card {
        if st.card.as_ref().map(card_key) != card.as_ref().map(card_key) {
            // Another request: its "always allow" starts unticked, whatever the owner chose on the last card.
            st.problem = None;
            st.always = false;
        }
        st.card = card;
    }
}

fn card_key(c: &Card) -> &str {
    match c {
        Card::Approval(a) => &a.key,
        Card::Question { key, .. } => key,
        Card::Sent { agent } => agent,
    }
}

/// Called by `App::refresh`: picks up a finished screen read, starts the next one, recomputes the card.
pub fn on_refresh(app: &mut App) {
    let pane = app.state.selected_pane().map(|p| (p.id.clone(), p.agent.as_ref().is_some_and(|a| a.status == AgentStatus::Blocked)));
    let st = &mut app.state.cards;
    if let Some(r) = st.inbox.lock().ok().and_then(|mut g| g.take()) {
        st.screen = Some(r);
    }
    match (&pane, st.actions.clone()) {
        (Some((id, true)), Some(actions)) => {
            if st.screen.as_ref().is_some_and(|s| &s.pane != id) {
                st.screen = None;
            }
            if actions.instant() {
                if let Ok(text) = actions.read_screen(id) {
                    st.screen = Some(ScreenRead { pane: id.clone(), text, identity: actions.agent_identity(id).ok().flatten() });
                }
            } else if !st.inflight.load(Ordering::SeqCst) && st.last_poll.is_none_or(|t| t.elapsed() >= POLL) {
                st.last_poll = Some(Instant::now());
                st.inflight.store(true, Ordering::SeqCst);
                let (inbox, inflight, id) = (st.inbox.clone(), st.inflight.clone(), id.clone());
                std::thread::spawn(move || {
                    let identity = actions.agent_identity(&id).ok().flatten();
                    if let Ok(text) = actions.read_screen(&id) {
                        let after = actions.agent_identity(&id).ok().flatten();
                        if identity == after {
                          if let Ok(mut g) = inbox.lock() {
                            *g = Some(ScreenRead { pane: id, text, identity });
                          }
                        }
                    }
                    inflight.store(false, Ordering::SeqCst);
                });
            }
        }
        _ => {
            // Not blocked (or read-only): forget the screen and the answers sent for this pane.
            st.screen = None;
            st.last_poll = None;
            if let Some((id, _)) = &pane {
                let prefix = format!("{id}:");
                st.sent.retain(|k| !k.starts_with(&prefix));
            }
        }
    }
    recompute(app);
}

/// Blocking read of the selected pane's screen (tests and demo set-up). Does nothing without actions.
pub fn read_now(app: &mut App) {
    let Some(id) = app.state.selected_pane().map(|p| p.id.clone()) else { return };
    if let Some(a) = app.state.cards.actions.clone() {
        if let Ok(text) = a.read_screen(&id) {
            app.state.cards.screen = Some(ScreenRead { pane: id.clone(), text, identity: a.agent_identity(&id).ok().flatten() });
        }
    }
    recompute(app);
}

// ---------------------------------------------------------------- update

pub fn update(app: &mut App, msg: CardsMsg) -> Task<Message> {
    let task = match msg {
        CardsMsg::Key(k) => on_key(app, k),
        CardsMsg::Allow => approve(app, true),
        CardsMsg::Deny => approve(app, false),
        CardsMsg::Always(on) => {
            app.state.cards.always = on;
            Task::none()
        }
        CardsMsg::Pick(i) => {
            if let Some(f) = flow_mut(app) {
                f.pick(i);
            }
            app.state.cards.other_active = false;
            iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
        }
        CardsMsg::ActivateOther => activate_other(app),
        CardsMsg::OtherInput(s) => {
            if let Some(f) = flow_mut(app) {
                f.set_other(&s);
            }
            app.state.cards.other_active = true;
            Task::none()
        }
        CardsMsg::Submit => submit(app),
        CardsMsg::Back => {
            if let Some(f) = flow_mut(app) {
                f.back();
            }
            app.state.cards.other_active = false;
            Task::none()
        }
        CardsMsg::Done { key, error } => {
            let st = &mut app.state.cards;
            st.busy = false;
            match error {
                None => {
                    st.problem = None;
                    st.sent.insert(key);
                }
                Some(e) => {
                    let changed = e == actions::PROMPT_CHANGED || e == actions::PROMPT_MISMATCH;
                    st.problem = Some(e);
                    if changed {
                        // The screen is not what the card showed: read it again now, so the card shows what is there.
                        st.last_poll = None;
                        if st.actions.as_ref().is_some_and(|a| a.instant()) {
                            read_now(app);
                        }
                    }
                }
            }
            Task::none()
        }
    };
    recompute(app);
    task
}

fn flow_mut(app: &mut App) -> Option<&mut Flow> {
    let key = match &app.state.cards.card {
        Some(Card::Question { key, .. }) => key.clone(),
        _ => return None,
    };
    match &mut app.state.cards.flow {
        Some((k, f)) if *k == key => Some(f),
        _ => None,
    }
}

fn activate_other(app: &mut App) -> Task<Message> {
    if flow_mut(app).is_none() {
        return Task::none();
    }
    app.state.cards.other_active = true;
    iced::widget::operation::focus(OTHER_ID)
}

fn on_key(app: &mut App, k: CardKey) -> Task<Message> {
    // The cards live on the Conversation and Needs you screens only. A key message from anywhere else is ignored.
    if !matches!(app.state.screen, crate::state::Screen::Conversation | crate::state::Screen::NeedsYou) {
        return Task::none();
    }
    match app.state.cards.card.clone() {
        // An approval card takes no keys: a permission is answered only by clicking Allow or Deny.
        Some(Card::Question { .. }) => match k {
            // Cmd/Ctrl+Enter is not "Enter" for a question (the key map drops it; this keeps the rule if it slips by).
            CardKey::Enter { chord: true } => Task::none(),
            CardKey::Enter { chord: false } => submit(app),
            CardKey::Escape => {
                if let Some(f) = flow_mut(app) {
                    f.back();
                }
                app.state.cards.other_active = false;
                Task::none()
            }
            // Up moves the highlight. From the "Other" row the field lets go of the keyboard.
            CardKey::Up => {
                let leaving = std::mem::replace(&mut app.state.cards.other_active, false);
                let landed_other = flow_mut(app).map(|f| f.move_cursor(-1)).unwrap_or(false);
                if landed_other {
                    activate_other(app)
                } else if leaving {
                    iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
                } else {
                    Task::none()
                }
            }
            CardKey::Down => match flow_mut(app).map(|f| f.move_cursor(1)) {
                Some(true) => activate_other(app),
                _ => Task::none(),
            },
            CardKey::Digit(n) => match flow_mut(app).map(|f| f.digit(n)) {
                Some(flow::Digit::Other) => activate_other(app),
                _ => Task::none(),
            },
            CardKey::Space => {
                if let Some(f) = flow_mut(app) {
                    if f.question().multi_select {
                        let c = f.cursor;
                        f.pick(c);
                    }
                }
                Task::none()
            }
        },
        _ => Task::none(),
    }
}

fn approve(app: &mut App, allow: bool) -> Task<Message> {
    let grok = app.state.selected_pane().and_then(|p| p.agent.as_ref()).is_some_and(|a| a.kind == "grok");
    let st = &mut app.state.cards;
    let (Some(Card::Approval(a)), Some(actions)) = (st.card.clone(), st.actions.clone()) else { return Task::none() };
    let Some(seen) = a.identity.clone() else { return Task::none() };
    if !a.can_act || st.busy {
        return Task::none();
    }
    st.busy = true;
    st.problem = None;
    let expected = st.agent_pin.as_ref().filter(|(k, _)| k == &a.key).map(|(_, id)| id.clone());
    let always = (allow && st.always).then(|| a.always.clone()).flatten();
    let (pane, key, command, detail) = (a.pane.clone(), a.key.clone(), a.command.clone(), a.detail.clone());
    spawn_send(key, move || {
        let guarded = actions::GuardedActions::pinned(actions, &pane, expected)?;
        if grok { actions::respond_grok(&guarded, &pane, allow, &seen) }
        else { actions::respond(&guarded, &pane, allow, always.as_ref(), &seen, &command, &detail) }
    })
}

fn submit(app: &mut App) -> Task<Message> {
    let st = &mut app.state.cards;
    let (Some(Card::Question { key, pane, q, can_send, .. }), Some(actions)) = (st.card.clone(), st.actions.clone()) else { return Task::none() };
    let Some((fk, flow)) = st.flow.as_mut() else { return Task::none() };
    if *fk != key || st.busy || !flow.advance() || !can_send {
        return Task::none();
    }
    let answers = flow.values();
    let expected = st.agent_pin.as_ref().filter(|(k, _)| k == &key).map(|(_, id)| id.clone());
    st.busy = true;
    st.problem = None;
    spawn_send(key, move || {
        let guarded = actions::GuardedActions::pinned(actions, &pane, expected)?;
        actions::answer_question(&guarded, &pane, &q, &answers)
    })
}

/// Runs a blocking send on its own thread and reports back as `CardsMsg::Done`.
fn spawn_send(key: String, job: impl FnOnce() -> Result<(), String> + Send + 'static) -> Task<Message> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(job().err());
    });
    Task::perform(async move { rx.await.unwrap_or_else(|_| Some("send thread stopped".to_string())) }, move |error| Message::Cards(CardsMsg::Done { key: key.clone(), error }))
}

// ---------------------------------------------------------------- demo

/// Canned screens for the demo's blocked panes, so `--demo` and snapshots show working cards without a socket.
/// The actions only record: nothing is sent anywhere.
pub fn install_demo(app: &mut App) {
    let mut screens = std::collections::HashMap::new();
    for ws in &app.state.workspaces {
        for tab in &ws.tabs {
            for pane in &tab.panes {
                if let Some(text) = app.source.conversation(pane).last().and_then(demo_screen) {
                    screens.insert(pane.id.clone(), text);
                }
            }
        }
    }
    app.state.cards.actions = Some(Arc::new(demo_actions(screens)));
    app.refresh();
}

fn demo_screen(item: &herdr_core::ConversationItem) -> Option<String> {
    match &item.kind {
        ItemKind::PermissionRequest(p) if p.state == PermState::Pending => Some(format!(
            "\u{25CF} Bash({})\n\n Bash command\n\n   {}\n\n Do you want to proceed?\n \u{276F} 1. Yes\n   2. Yes, and always allow access to the deploy scripts from this project\n   3. No\n",
            p.command, p.command
        )),
        ItemKind::Question(q) if q.state == QuestionState::Pending => {
            let q0 = q.questions.first()?;
            let mut s = format!("\u{2190}  \u{2610} {}  \u{2714} Submit  \u{2192}\n\n{}\n\n", q0.header, q0.text);
            for (i, o) in q0.options.iter().enumerate() {
                s.push_str(&format!(" {}. {}{}\n    {}\n", i + 1, o.label, if o.recommended { " (Recommended)" } else { "" }, o.detail));
            }
            s.push_str(&format!(" {}. Type something.\n\u{2500}\u{2500}\u{2500}\u{2500}\n Chat about this\n", q0.options.len() + 1));
            Some(s)
        }
        _ => None,
    }
}

/// Serves one canned screen per pane and records sends. Every send succeeds.
struct DemoPane {
    screens: std::collections::HashMap<String, String>,
    inner: actions::RecordingActions,
}

fn demo_actions(screens: std::collections::HashMap<String, String>) -> DemoPane {
    DemoPane { screens, inner: actions::RecordingActions::new("") }
}

impl PaneActions for DemoPane {
    fn read_screen(&self, pane_id: &str) -> Result<String, String> {
        Ok(self.screens.get(pane_id).cloned().unwrap_or_default())
    }
    fn send_text(&self, pane_id: &str, text: &str) -> Result<(), String> {
        self.inner.send_text(pane_id, text)
    }
    fn send_keys(&self, pane_id: &str, keys: &[String]) -> Result<(), String> {
        self.inner.send_keys(pane_id, keys)
    }
    fn pacing(&self) -> actions::Pacing {
        actions::Pacing::instant()
    }
    fn instant(&self) -> bool {
        true
    }
}

/// The demo app with the tab labelled `label` selected (snapshots, `--tab`).
pub fn demo_app_on_tab(mode: fb_ui::Mode, screen: crate::state::Screen, label: &str) -> App {
    let mut app = App::demo(mode, screen);
    let found = app.state.workspaces.iter().find_map(|w| w.tabs.iter().find(|t| t.label == label).map(|t| (w.id.clone(), t.id.clone())));
    if let Some((w, t)) = found {
        app.state.select_tab(&w, &t);
        app.refresh();
    }
    app
}
