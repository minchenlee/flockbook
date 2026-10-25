//! Composer: the floating reply card at the bottom of the conversation column. DESIGN.md section 8.
//!
//! Enter sends, Shift+Enter inserts a newline (same as the Swift app). The editor grows to `MAX_LINES` lines, then
//! scrolls. IME works through iced's own `text_editor`: preedit is shown but not committed, a commit lands at the caret.
//! Approval and question cards are not part of this module yet.
//!
//! # Sending
//!
//! Sending goes through the [`MessageSender`] trait. The real implementation, [`LiveSender`], writes to a herdr pane
//! (`pane.send_input`, text plus Enter). That is a MUTATING call. It is only installed by the live path in `main.rs`.
//! Tests never construct `LiveSender`: they use a fake that records calls. Live write tests target the named test
//! session `c9-input-test` only, never the default session.
//!
//! With no sender installed the composer is read-only ("Read-only: input is off"), like the Swift app.
//!
//! # Attach (+)
//!
//! The + button opens the system file dialog ([`FilePicker`]; live: [`NativePicker`]) and inserts the chosen paths
//! at the caret, shell-quoted, with a trailing space (Swift `attach()`). The agent reads a path as an attachment. In
//! read-only the button is dimmed and has no action. Files and folders have separate picker actions.

use crate::queued::Journal;
use crate::{style, App, Message};
use fb_ui::icons::{icon, Icon, SIZE_DEFAULT, SIZE_INLINE};
use fb_ui::theme::*;
use herdr_core::queued_input::{ClaudeQueueLog, PullIO, PullOutcome, QueuedInput};
use herdr_core::{AgentIdentity, AgentStatus};
use iced::keyboard::{key::Named, Key};
use iced::widget::text_editor::{self, Action, Binding, Content};
use iced::widget::{button, column, container, row, text, text_editor as text_editor_fn, Space};
use iced::{Alignment, Background, Border, Element, Length, Task};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The editor grows up to this many lines, then scrolls.
pub const MAX_LINES: usize = 8;
/// Widget id of the editor (tests and focus operations).
pub const EDITOR_ID: &str = "composer-editor";
/// Width cap of the card, same as the conversation column.
pub const CARD_MAX_W: f32 = 780.0;
/// Bubbles and the composer card: `Radius.bubble` in Swift (`Sources/HerdrDesk/Reading.swift:33`), not
/// the 18 pt the shared token uses.
const R_BUBBLE: f32 = 16.0;

/// Delivers a composer message to a pane. `Err` carries a short reason for the owner.
///
/// Implementations may block (the live one does a socket call), so `update` calls them off the UI thread.
pub trait MessageSender: Send + Sync {
    fn send(&self, pane_id: &str, text: &str) -> Result<(), String>;
    fn send_images(
        &self,
        pane_id: &str,
        text: &str,
        images: &[Attachment],
        _identity: Option<&AgentIdentity>,
    ) -> Result<(), String> {
        if images.is_empty() {
            self.send(pane_id, text)
        } else {
            Err("This sender cannot attach images".into())
        }
    }
    fn queue(
        &self,
        _identity: &AgentIdentity,
        _log: &ClaudeQueueLog,
    ) -> Result<Option<QueuedInput>, String> {
        Ok(None)
    }
    fn pull(&self, _identity: &AgentIdentity, _log: &ClaudeQueueLog) -> PullOutcome {
        PullOutcome::Refused("Queue pull is unavailable".into())
    }
    fn journal(&self) -> Option<Arc<Journal>> {
        None
    }
    fn revoke(&self) {}
    fn set_paused(&self, _paused: bool) {}
}

/// Sends text and Enter to a herdr pane. MUTATING: never used in tests; live write tests target the
/// `c9-input-test` session only.
#[cfg(unix)]
pub struct LiveSender {
    allowed: AtomicBool,
    paused: AtomicBool,
    journal: Arc<Journal>,
    client: herdr_core::client::HerdrClient<herdr_core::transport::UnixConnector>,
}

#[cfg(unix)]
impl LiveSender {
    pub fn new(session: &herdr_core::HerdrSession) -> LiveSender {
        let key: String = session
            .socket_path
            .bytes()
            .map(|b| format!("{b:02x}"))
            .collect();
        let path = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Flockbook/pulled")
            .join(format!("{key}.json"));
        LiveSender {
            allowed: AtomicBool::new(true),
            paused: AtomicBool::new(false),
            journal: Arc::new(Journal::new(path)),
            client: herdr_core::client::HerdrClient::new(session),
        }
    }
}

#[cfg(unix)]
impl MessageSender for LiveSender {
    fn send(&self, pane_id: &str, text: &str) -> Result<(), String> {
        if !self.allowed.load(Ordering::SeqCst) || self.paused.load(Ordering::SeqCst) {
            return Err("Input is off".into());
        }
        self.client
            .raw_request(
                "pane.send_input",
                serde_json::json!({"pane_id":pane_id,"text":text,"keys":["Enter"]}),
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    fn send_images(
        &self,
        pane: &str,
        text: &str,
        images: &[Attachment],
        identity: Option<&AgentIdentity>,
    ) -> Result<(), String> {
        if images.is_empty() {
            return self.send(pane, text);
        }
        let expect = identity.ok_or("Cannot identify this agent for image submission")?;
        if expect.pane_id != pane {
            return Err("Pane changed".into());
        }
        let paths = images
            .iter()
            .map(|i| {
                i.path
                    .to_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "Image path is not UTF-8".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        herdr_core::message_send::send(
            &LiveIO {
                sender: self,
                expect,
            },
            expect,
            text,
            &paths,
            Duration::from_secs(5),
        )
    }
    fn queue(
        &self,
        expect: &AgentIdentity,
        log: &ClaudeQueueLog,
    ) -> Result<Option<QueuedInput>, String> {
        let io = LiveIO {
            sender: self,
            expect,
        };
        if io.identity().as_ref() != Some(expect) {
            return Ok(None);
        }
        Ok(herdr_core::queued_input::read(
            &expect.kind,
            io.working(),
            &io.screen()?,
            log,
        ))
    }
    fn pull(&self, expect: &AgentIdentity, log: &ClaudeQueueLog) -> PullOutcome {
        herdr_core::queued_input::pull(
            &LiveIO {
                sender: self,
                expect,
            },
            expect,
            log,
        )
    }
    fn journal(&self) -> Option<Arc<Journal>> {
        Some(self.journal.clone())
    }
    fn revoke(&self) {
        self.allowed.store(false, Ordering::SeqCst);
    }
    fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }
}

#[cfg(unix)]
struct LiveIO<'a> {
    sender: &'a LiveSender,
    expect: &'a AgentIdentity,
}
#[cfg(unix)]
impl LiveIO<'_> {
    fn pane(&self) -> Option<herdr_core::Pane> {
        self.sender
            .client
            .snapshot()
            .ok()?
            .into_iter()
            .flat_map(|w| w.tabs)
            .flat_map(|t| t.panes)
            .find(|p| p.id == self.expect.pane_id)
    }
    fn allowed(&self) -> Result<(), String> {
        if self.sender.allowed.load(Ordering::SeqCst) && !self.sender.paused.load(Ordering::SeqCst)
        {
            Ok(())
        } else {
            Err("Input is off".into())
        }
    }
    fn request(&self, method: &str, params: serde_json::Value) -> Result<(), String> {
        self.allowed()?;
        self.sender
            .client
            .raw_request(method, params)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}
#[cfg(unix)]
impl PullIO for LiveIO<'_> {
    fn identity(&self) -> Option<AgentIdentity> {
        AgentIdentity::for_pane(&self.pane()?)
    }
    fn working(&self) -> bool {
        self.pane()
            .and_then(|p| p.agent)
            .is_some_and(|a| a.status == AgentStatus::Working)
    }
    fn active(&self) -> bool {
        self.pane().and_then(|p| p.agent).is_some_and(|a| {
            matches!(
                a.status,
                AgentStatus::Working | AgentStatus::Idle | AgentStatus::Done
            )
        })
    }
    fn screen(&self) -> Result<String, String> {
        let bytes=self.sender.client.raw_request("pane.read",serde_json::json!({"pane_id":self.expect.pane_id,"source":"visible","format":"text","lines":60})).map_err(|e|e.to_string())?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        value
            .pointer("/result/read/text")
            .and_then(|t| t.as_str())
            .map(str::to_owned)
            .ok_or_else(|| "Cannot read agent input".into())
    }
    fn key(&self, key: &str, expect: &AgentIdentity) -> Result<(), String> {
        if !matches!(key, "Up" | "ctrl+u") {
            return Err("Unsafe pull key".into());
        }
        self.allowed()?;
        if self.identity().as_ref() != Some(expect) {
            return Err("Agent changed".into());
        }
        self.request(
            "pane.send_keys",
            serde_json::json!({"pane_id":expect.pane_id,"keys":[key]}),
        )
    }
    fn keep(&self, text: &str) -> Result<(), String> {
        self.sender.journal.keep(&self.expect.pane_id, text)
    }
}
#[cfg(unix)]
impl herdr_core::message_send::SendIO for LiveIO<'_> {
    fn gate(&self, expect: &AgentIdentity) -> Result<(), String> {
        self.allowed()?;
        let pane = self.pane().ok_or("Pane disappeared")?;
        if AgentIdentity::for_pane(&pane).as_ref() != Some(expect)
            || expect.session_id.as_deref().is_none_or(str::is_empty)
        {
            return Err("Agent session changed or is unknown".into());
        }
        if pane.agent.is_some_and(|a| a.status == AgentStatus::Blocked) {
            return Err("Agent is waiting for a choice".into());
        }
        let screen = PullIO::screen(self)?;
        if herdr_core::screen_shows_prompt(&screen)
            || screen.contains("Enter to select")
            || screen.contains("Esc to cancel")
            || screen.contains("Type something.")
        {
            return Err("Agent input is occupied by a prompt".into());
        }
        self.allowed()
    }
    fn screen(&self) -> Result<String, String> {
        PullIO::screen(self)
    }
    fn input(&self, text: &str) -> Result<(), String> {
        self.request(
            "pane.send_input",
            serde_json::json!({"pane_id":self.expect.pane_id,"text":text,"keys":[]}),
        )
    }
    fn paste(&self, text: &str) -> Result<(), String> {
        self.request(
            "pane.send_text",
            serde_json::json!({"pane_id":self.expect.pane_id,"text":text}),
        )
    }
    fn enter(&self) -> Result<(), String> {
        self.request(
            "pane.send_keys",
            serde_json::json!({"pane_id":self.expect.pane_id,"keys":["Enter"]}),
        )
    }
}

/// What a file dialog gives back: the chosen paths, empty when cancelled.
pub type PickFuture = Pin<Box<dyn Future<Output = Vec<PathBuf>> + Send>>;

/// Opens a file dialog. Tests use a fake; the app uses [`NativePicker`].
pub trait FilePicker: Send + Sync {
    /// `start`: the folder the dialog opens in (the pane's working folder), if known.
    fn pick(&self, start: Option<PathBuf>) -> PickFuture;
    fn pick_folders(&self, _start: Option<PathBuf>) -> PickFuture {
        Box::pin(async { vec![] })
    }
}

/// The system file dialog (NSOpenPanel, the Windows file dialog, the XDG desktop portal on Linux).
pub struct NativePicker;

impl FilePicker for NativePicker {
    fn pick_folders(&self, start: Option<PathBuf>) -> PickFuture {
        let mut dialog = rfd::AsyncFileDialog::new().set_title("Attach folder");
        if let Some(dir) = start.filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        Box::pin(async move {
            dialog
                .pick_folders()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        })
    }
    fn pick(&self, start: Option<PathBuf>) -> PickFuture {
        let mut dialog = rfd::AsyncFileDialog::new().set_title("Attach");
        if let Some(dir) = start.filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        Box::pin(async move {
            dialog
                .pick_files()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        })
    }
}

/// A POSIX shell word, including dollar signs, backticks, newlines and Unicode scalars.
pub fn quoted(path: &Path) -> String {
    herdr_core::shell_quote::quoted_if_needed(&path.to_string_lossy())
}

#[derive(Debug, Clone)]
pub struct Attachment {
    pub path: PathBuf,
    pub preview: iced::widget::image::Handle,
}
#[derive(Default)]
pub struct PendingSend {
    pub images: Vec<Attachment>,
    pub recovery_ids: Vec<String>,
}

#[derive(Default)]
pub struct ComposerState {
    pub composing: Arc<AtomicBool>,
    pub image_jobs: usize,
    pub images: Vec<Attachment>,
    pub image_drafts: HashMap<String, Vec<Attachment>>,
    pub pending: HashMap<String, PendingSend>,
    pub queues: HashMap<String, QueuedInput>,
    pub queue_logs: HashMap<String, ClaudeQueueLog>,
    pub queue_reading: bool,
    pub pulling: bool,
    pub recovery: Vec<crate::queued::KeptText>,
    /// Per-pane failed submissions, retained even when the owner has already typed a new draft.
    pub failed: HashMap<String, Vec<(String, Vec<Attachment>)>>,

    pub content: Content,
    /// `None`: read-only.
    pub sender: Option<Arc<dyn MessageSender>>,
    pub queue_reader: Option<Arc<dyn MessageSender>>,
    /// Shown under the editor after a failed send; cleared on the next edit.
    pub error: Option<String>,
    /// Overrides the git lookup (tests and snapshots).
    pub branch_override: Option<String>,
    /// A send is on its way (blocks double sends of the same Enter).
    pub sending: bool,
    /// The pane the editor currently belongs to.
    pub pane: Option<String>,
    /// Unsent text of the panes that are not shown, by pane id. One draft per pane, like the Swift app.
    pub drafts: HashMap<String, String>,
    /// The file dialog behind +. `None`: + does nothing (tests that do not need it).
    pub picker: Option<Arc<dyn FilePicker>>,
}

impl ComposerState {
    pub fn text(&self) -> String {
        self.content.text()
    }
    pub fn set_text(&mut self, s: &str) {
        self.content = Content::with_text(s);
        self.content
            .perform(Action::Move(text_editor::Motion::DocumentEnd));
    }
    /// Switches the editor to `pane`: the text on screen is kept as the old pane's draft, the new pane's draft
    /// (if any) comes back. Does nothing when the pane did not change.
    pub fn switch_pane(&mut self, pane: Option<&str>) {
        if self.pane.as_deref() == pane {
            return;
        }
        self.composing.store(false, Ordering::SeqCst);
        if let Some(old) = self.pane.take() {
            self.image_drafts
                .insert(old.clone(), std::mem::take(&mut self.images));
            let text = self.text();
            if text.is_empty() {
                self.drafts.remove(&old);
            } else {
                self.drafts.insert(old, text);
            }
        }
        self.images = pane
            .and_then(|p| self.image_drafts.remove(p))
            .unwrap_or_default();
        let draft = pane.and_then(|p| self.drafts.remove(p)).unwrap_or_default();
        self.error = None;
        if draft.is_empty() {
            self.content = Content::new();
        } else {
            self.set_text(&draft);
        }
        self.pane = pane.map(str::to_string);
    }

    pub fn can_send(&self) -> bool {
        self.sender.is_some()
            && self.image_jobs == 0
            && !self.composing.load(Ordering::SeqCst)
            && !self.sending
            && !self.pulling
            && (!self.text().trim().is_empty() || !self.images.is_empty())
    }
}

#[derive(Clone)]
pub enum ComposerMsg {
    Edit(Action),
    Submit,
    /// Result of a send. `text` is given back to an empty box on failure.
    Sent {
        pane: String,
        text: String,
        error: Option<String>,
    },
    /// + : open the file dialog.
    Attach,
    /// The dialog's result: these paths go in at the caret.
    Attached(Vec<PathBuf>),
    /// Buttons without a function yet (dictate).
    Noop,
    AttachFolder,
    Picked {
        pane: String,
        paths: Vec<PathBuf>,
    },
    Paste,
    PasteText {
        pane: String,
        text: Option<String>,
    },
    Images {
        pane: String,
        images: Vec<Attachment>,
        error: Option<String>,
    },
    Drop(PathBuf),
    RemoveImage(usize),
    PollQueue,
    QueueRead {
        identity: AgentIdentity,
        queue: Option<QueuedInput>,
        log: ClaudeQueueLog,
    },
    Pull,
    Pulled {
        pane: String,
        outcome: PullOutcome,
    },
    RecoverFailed,
}

impl std::fmt::Debug for ComposerMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComposerMsg::Edit(_) => write!(f, "Edit"),
            ComposerMsg::Submit => write!(f, "Submit"),
            ComposerMsg::Sent { error, .. } => write!(f, "Sent({})", error.is_some()),
            ComposerMsg::Attach => write!(f, "Attach"),
            ComposerMsg::Attached(p) => write!(f, "Attached({})", p.len()),
            ComposerMsg::Noop => write!(f, "Noop"),
            _ => write!(f, "Composer action"),
        }
    }
}

pub fn update(app: &mut App, msg: ComposerMsg) -> Task<Message> {
    if (app.state.settings.open
        || app.state.shortcuts.open
        || app.state.terminal.is_open()
        || crate::workspace_actions::modal(app))
        && matches!(
            &msg,
            ComposerMsg::Edit(_)
                | ComposerMsg::Submit
                | ComposerMsg::Attach
                | ComposerMsg::AttachFolder
                | ComposerMsg::Paste
                | ComposerMsg::Drop(_)
                | ComposerMsg::Pull
                | ComposerMsg::RemoveImage(_)
                | ComposerMsg::RecoverFailed
        )
    {
        return Task::none();
    }
    match msg {
        ComposerMsg::Edit(action) => {
            if app.state.composer.sender.is_none() && action.is_edit() {
                return Task::none();
            }
            if action.is_edit() {
                app.state.composer.error = None;
            }
            app.state.composer.content.perform(action);
            Task::none()
        }
        ComposerMsg::Submit => {
            if !app.state.composer.can_send() || app.state.settings.open {
                return Task::none();
            }
            let Some(sender) = app.state.composer.sender.clone() else {
                return Task::none();
            };
            let Some(pane) = app.state.selected_pane().cloned() else {
                return Task::none();
            };
            let identity = AgentIdentity::for_pane(&pane);
            let c = &mut app.state.composer;
            let text = c.text();
            let images = std::mem::take(&mut c.images);
            let recovery_ids = c
                .recovery
                .iter()
                .filter(|k| k.pane == pane.id && text.contains(&k.text))
                .map(|k| k.id.clone())
                .collect();
            c.pending.insert(
                pane.id.clone(),
                PendingSend {
                    images: images.clone(),
                    recovery_ids,
                },
            );
            c.content = Content::new();
            c.error = None;
            c.sending = true;
            crate::conversation::follow_latest(app);
            let (tx, rx) = iced::futures::channel::oneshot::channel();
            let sent = text.clone();
            let pane_id = pane.id.clone();
            std::thread::spawn(move || {
                let _ = tx.send(
                    sender
                        .send_images(&pane.id, &sent, &images, identity.as_ref())
                        .err(),
                );
            });
            Task::perform(
                async move {
                    rx.await.unwrap_or_else(|_| {
                        Some("Send thread stopped; check the terminal before retrying".into())
                    })
                },
                move |error| {
                    Message::Composer(ComposerMsg::Sent {
                        pane: pane_id.clone(),
                        text: text.clone(),
                        error,
                    })
                },
            )
        }
        ComposerMsg::Sent { pane, text, error } => {
            let c = &mut app.state.composer;
            c.sending = false;
            let pending = c.pending.remove(&pane).unwrap_or_default();
            if let Some(e) = error {
                let current = c.pane.as_deref() == Some(&pane);
                if current && c.text().is_empty() && c.images.is_empty() {
                    c.set_text(&text);
                    c.images = pending.images;
                } else if !current
                    && !c.drafts.contains_key(&pane)
                    && c.image_drafts.get(&pane).is_none_or(Vec::is_empty)
                {
                    c.drafts.insert(pane.clone(), text);
                    c.image_drafts.insert(pane.clone(), pending.images);
                } else {
                    c.failed
                        .entry(pane.clone())
                        .or_default()
                        .push((text, pending.images));
                }
                // The note belongs to the box it happened in (one composer per pane in the Swift app): a failure
                // for a pane that is not shown must not put a note under the shown pane's draft.
                if current {
                    c.error = Some(format!(
                        "Send incomplete: {e}. Check the agent terminal before retrying."
                    ));
                }
            } else if let Some(journal) = c
                .sender
                .as_ref()
                .and_then(|s| s.journal())
                .filter(|_| !pending.recovery_ids.is_empty())
            {
                match journal.sent(&pending.recovery_ids) {
                    Ok(()) => c.recovery.retain(|k| !pending.recovery_ids.contains(&k.id)),
                    Err(e) => c.error = Some(format!("Sent; recovery copy retained: {e}")),
                }
            }
            Task::none()
        }
        ComposerMsg::Attach | ComposerMsg::AttachFolder => {
            let folder = matches!(msg, ComposerMsg::AttachFolder);
            let st = &app.state.composer;
            let (Some(_), Some(picker)) = (&st.sender, st.picker.clone()) else {
                return Task::none();
            };
            let start = app.state.selected_pane().map(|p| expand_home(&p.cwd));
            let Some(pane) = st.pane.clone() else {
                return Task::none();
            };
            let future = if folder {
                picker.pick_folders(start)
            } else {
                picker.pick(start)
            };
            Task::perform(future, move |paths| {
                Message::Composer(ComposerMsg::Picked {
                    pane: pane.clone(),
                    paths,
                })
            })
        }
        ComposerMsg::Picked { pane, paths } => attach_paths(app, pane, paths, false),
        ComposerMsg::Attached(paths) => {
            let Some(pane) = app.state.composer.pane.clone() else {
                return Task::none();
            };
            attach_paths(app, pane, paths, false)
        }
        ComposerMsg::Paste => {
            let Some(pane) = app.state.composer.pane.clone() else {
                return Task::none();
            };
            if app.state.composer.sender.is_none() {
                return Task::none();
            }
            if app.state.composer.image_jobs >= 4 {
                app.state.composer.error =
                    Some("Wait for the current image paste to finish".into());
                return Task::none();
            }
            app.state.composer.image_jobs += 1;
            iced::clipboard::read().map(move |text| {
                Message::Composer(ComposerMsg::PasteText {
                    pane: pane.clone(),
                    text,
                })
            })
        }
        ComposerMsg::PasteText {
            pane,
            text: Some(text),
        } if !text.is_empty() => {
            app.state.composer.image_jobs = app.state.composer.image_jobs.saturating_sub(1);
            if let Some(paths) = clipboard_files(&text) {
                return attach_paths(app, pane, paths, true);
            }
            insert_for(&mut app.state.composer, &pane, &text);
            Task::none()
        }
        ComposerMsg::PasteText { pane, .. } => {
            app.state.composer.image_jobs = app.state.composer.image_jobs.saturating_sub(1);
            image_job(app, pane, vec![], true)
        }
        ComposerMsg::Drop(path) => {
            if app.state.settings.open || app.state.composer.sender.is_none() {
                return Task::none();
            }
            let Some(pane) = app.state.composer.pane.clone() else {
                return Task::none();
            };
            attach_paths(app, pane, vec![path], true)
        }
        ComposerMsg::Images {
            pane,
            images,
            error,
        } => {
            let c = &mut app.state.composer;
            c.image_jobs = c.image_jobs.saturating_sub(1);
            if c.sender.is_none() {
                return Task::none();
            }
            if c.pane.as_deref() == Some(&pane) {
                c.images.extend(images);
                c.error = error;
            } else {
                c.image_drafts.entry(pane).or_default().extend(images);
            }
            Task::none()
        }
        ComposerMsg::RemoveImage(i) => {
            if i < app.state.composer.images.len() {
                app.state.composer.images.remove(i);
            }
            Task::none()
        }
        ComposerMsg::PollQueue => poll_queue(app),
        ComposerMsg::QueueRead {
            identity,
            queue,
            log,
        } => {
            let current = app
                .state
                .workspaces
                .iter()
                .flat_map(|w| &w.tabs)
                .flat_map(|t| &t.panes)
                .find(|p| p.id == identity.pane_id)
                .and_then(AgentIdentity::for_pane);
            let c = &mut app.state.composer;
            c.queue_reading = false;
            if current.as_ref() == Some(&identity) {
                c.queue_logs.insert(identity.pane_id.clone(), log);
                if let Some(q) = queue {
                    c.queues.insert(identity.pane_id, q);
                } else {
                    c.queues.remove(&identity.pane_id);
                }
            }
            Task::none()
        }
        ComposerMsg::Pull => pull_queue(app),
        ComposerMsg::Pulled { pane, outcome } => {
            let c = &mut app.state.composer;
            c.pulling = false;
            c.queues.remove(&pane);
            let (text, note) = match outcome {
                PullOutcome::Pulled(t) => (Some(t), None),
                PullOutcome::Interrupted(t)=>(Some(t),Some("Text recovered; the agent turn stopped. Check the terminal before sending.".into())),
                PullOutcome::Partial(t, e) => (Some(t), Some(e)),
                PullOutcome::Refused(e) | PullOutcome::Failed(e) => (None, Some(e)),
            };
            if let Some(text) = text {
                append_for(c, &pane, &text);
            }
            if let Some(journal) = c.sender.as_ref().and_then(|s| s.journal()) {
                if let Ok(entries) = journal.read() {
                    c.recovery = entries;
                }
            }
            c.error = note;
            Task::none()
        }
        ComposerMsg::RecoverFailed => {
            let c = &mut app.state.composer;
            let Some(pane) = c.pane.clone() else {
                return Task::none();
            };
            if let Some(failed) = c.failed.remove(&pane) {
                for (text, images) in failed {
                    append_for(c, &pane, &text);
                    c.images.extend(images);
                }
            }
            Task::none()
        }
        ComposerMsg::Noop => Task::none(),
    }
}

fn key_binding(kp: text_editor::KeyPress) -> Option<Binding<Message>> {
    if matches!(kp.status, text_editor::Status::Focused { .. })
        && kp.modifiers.command()
        && !kp.modifiers.alt()
        && matches!(kp.key.as_ref(),Key::Character(c) if c.eq_ignore_ascii_case("v"))
    {
        return Some(Binding::Custom(Message::Composer(ComposerMsg::Paste)));
    }
    if matches!(kp.key.as_ref(), Key::Named(Named::Enter))
        && matches!(kp.status, text_editor::Status::Focused { .. })
    {
        return if kp.modifiers.shift() {
            Some(Binding::Enter)
        } else {
            Some(Binding::Custom(Message::Composer(ComposerMsg::Submit)))
        };
    }
    Binding::from_key_press(kp)
}

pub fn view(app: &App) -> Element<'_, Message> {
    let t = app.tokens();
    let st = &app.state.composer;
    let read_only = st.sender.is_none();
    let agent = app
        .state
        .selected_pane()
        .and_then(|p| p.agent.as_ref())
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "agent".into());
    let placeholder = if read_only {
        "Read-only: input is off".to_string()
    } else {
        format!("Reply to {agent}")
    };

    let reading = app.state.settings.reading_size();
    let line_h = reading.line();
    let max_h = line_h * MAX_LINES as f32 + 8.0;
    let lines = st.content.line_count().clamp(1, MAX_LINES);
    let editor = text_editor_fn(&st.content)
        .id(EDITOR_ID)
        .placeholder(placeholder)
        .on_action(|a| Message::Composer(ComposerMsg::Edit(a)))
        .key_binding(key_binding)
        .font(READING.font())
        .size(reading.body())
        .line_height(iced::advanced::text::LineHeight::Absolute(line_h.into()))
        .padding([4, 0])
        .min_height(line_h + 8.0)
        .max_height(max_h)
        .height(Length::Fixed((line_h * lines as f32 + 8.0).min(max_h)))
        .style(move |_, _| text_editor::Style {
            background: Background::Color(iced::Color::TRANSPARENT),
            border: Border::default(),
            placeholder: t.label_3,
            value: t.label,
            selection: iced::Color {
                a: 0.28,
                ..t.accent
            },
        });

    let can_send = st.can_send();
    // The svg renderer drops icon alpha, so translucent icon colors are blended with the surface by hand.
    let card_bg = card_bg(t);
    let send_icon = if can_send {
        t.on_accent
    } else {
        over(t.label_3, over(t.fill_2, card_bg))
    };
    let send_btn =
        button(container(icon(Icon::ArrowUp, SIZE_DEFAULT, send_icon)).center(Length::Fixed(28.0)))
            .padding(0)
            .on_press_maybe(can_send.then_some(Message::Composer(ComposerMsg::Submit)))
            .style(move |_, status| {
                let bg = if can_send {
                    match status {
                        button::Status::Hovered | button::Status::Pressed => iced::Color {
                            a: 0.88,
                            ..t.accent
                        },
                        _ => t.accent,
                    }
                } else {
                    t.fill_2
                };
                button::Style {
                    background: Some(Background::Color(bg)),
                    border: Border {
                        radius: 14.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            });

    let ghost = move |ic: Icon, size: f32, msg: Message| {
        button(
            container(icon(ic, size, attach_icon_color(t, !read_only))).center(Length::Fixed(28.0)),
        )
        .padding(0)
        .on_press_maybe((!read_only).then_some(msg))
        .style(move |_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then(|| Background::Color(t.fill_2)),
            border: Border {
                radius: 14.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
    };

    let plus = ghost(
        Icon::Plus,
        SIZE_DEFAULT + 2.0,
        Message::Composer(ComposerMsg::Attach),
    );
    let tip = if read_only {
        "Attach file (input is off)"
    } else {
        "Attach file (inserts its path)"
    };
    let plus =
        iced::widget::tooltip(plus, tip_box(t, tip), iced::widget::tooltip::Position::Top).gap(6);
    // The Swift app's `+` opens one panel that takes files and folders (`canChooseDirectories`, `Composer.swift:260`).
    // The system dialogs behind rfd pick one kind at a time, so the folder picker is a second quiet icon next to it,
    // and only while input is on: read-only shows `+`, branch, mic and send, like the Swift app's composer.
    // Swift: the row is an `HStack(spacing: 10)` (`Composer.swift:214`).
    let mut controls = row![plus].spacing(10.0).align_y(Alignment::Center);
    if !read_only {
        controls = controls.push(
            iced::widget::tooltip(
                ghost(
                    Icon::Folder,
                    SIZE_DEFAULT,
                    Message::Composer(ComposerMsg::AttachFolder),
                ),
                tip_box(t, "Attach folder (inserts its path)"),
                iced::widget::tooltip::Position::Top,
            )
            .gap(6),
        );
    }
    if let Some(b) = branch_for(app) {
        controls = controls.push(
            row![
                icon(Icon::GitBranch, SIZE_INLINE, over(t.label_2, card_bg)),
                CAPTION
                    .text(b)
                    .color(t.label_2)
                    .wrapping(text::Wrapping::None)
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        );
    }
    controls = controls
        .push(Space::new().width(Length::Fill))
        .push(ghost(
            Icon::Mic,
            SIZE_DEFAULT,
            Message::Composer(ComposerMsg::Noop),
        ))
        .push(send_btn);

    let mut body = column![].spacing(6.0);
    if let Some(queue) = st.pane.as_ref().and_then(|p| st.queues.get(p)) {
        // Faded user bubbles over the composer, right-aligned and ending where the conversation's bubbles end
        // (`QueuedView.swift:147-187`: trailing VStack, 6 between items, 4 inside one, h14/v8 padding,
        // primary 4% fill, 1 pt primary 22% border, at most 80% of the reading column).
        let col = app.state.settings.column_width();
        let mut strip = column![]
            .spacing(6.0)
            .width(Length::Fill)
            .align_x(Alignment::End);
        for item in &queue.items {
            let bubble = column![
                CAPTION.text("Queued").size(10.0).color(t.label_3),
                CAPTION
                    .text(item.as_deref().unwrap_or("1 queued message"))
                    .size(reading.body())
                    .line_height(iced::advanced::text::LineHeight::Absolute(line_h.into()))
                    .color(t.label_2),
            ]
            .spacing(SP_4);
            strip = strip.push(
                container(bubble)
                    .padding([8, 14])
                    .max_width(col * 0.8)
                    .style(move |_| container::Style {
                        background: Some(Background::Color(iced::Color { a: 0.04, ..t.label })),
                        border: Border {
                            color: iced::Color { a: 0.22, ..t.label },
                            width: 1.0,
                            radius: R_BUBBLE.into(),
                        },
                        ..Default::default()
                    }),
            );
        }
        strip = strip.push(
            CAPTION
                .text(if queue.agent == "claude" {
                    "Goes in after the current step or when the turn ends."
                } else {
                    "Goes in when this turn ends."
                })
                .color(t.label_3),
        );
        if queue.agent == "claude" {
            strip = strip.push(quiet(
                t,
                if st.pulling {
                    "Pulling…".into()
                } else {
                    "Pull back to edit".into()
                },
                !read_only && queue.pullable && !st.pulling && !st.sending,
                Message::Composer(ComposerMsg::Pull),
            ));
        } else {
            strip = strip.push(
                CAPTION
                    .text(if queue.agent == "codex" {
                        "Edit in Codex's terminal with Shift+Left."
                    } else {
                        "Edit in Grok's terminal with Ctrl+;."
                    })
                    .color(t.label_3),
            );
        }
        body = body.push(
            container(iced::widget::scrollable(strip).height(Length::Shrink)).max_height(144),
        );
    }
    if !st.images.is_empty() {
        let mut chips = row![].spacing(SP_8);
        for (i, attachment) in st.images.iter().enumerate() {
            chips = chips.push(column![
                iced::widget::image(attachment.preview.clone())
                    .width(48)
                    .height(48),
                quiet(
                    t,
                    "Remove".into(),
                    true,
                    Message::Composer(ComposerMsg::RemoveImage(i))
                )
            ]);
        }
        body = body.push(
            iced::widget::scrollable(chips)
                .direction(iced::widget::scrollable::Direction::Horizontal(
                    Default::default(),
                ))
                .height(88),
        );
    }
    let editor = Element::new(ImeGuard {
        content: editor.into(),
        composing: st.composing.clone(),
    });
    body = body.push(editor);
    if st.pane.as_ref().is_some_and(|p| st.failed.contains_key(p)) {
        body = body.push(quiet(
            t,
            "Restore previous unsent draft".into(),
            true,
            Message::Composer(ComposerMsg::RecoverFailed),
        ));
    }
    if let Some(e) = &st.error {
        body = body.push(CAPTION.text(e.clone()).color(t.blocked));
    }
    body = body.push(controls);

    // The card is the reading column itself, not the column plus its padding: Swift puts the 16 pt inside the card
    // (`.padding(.horizontal, 16)`, `Sources/HerdrDesk/Composer.swift:264`) and the card is the reading column wide
    // (`inputColumn`, `Sources/HerdrDesk/Reading.swift:96-104`), so its edge lines up with the message text.
    let card = container(body.padding([12, 16]))
        .width(Length::Fill)
        .max_width(app.state.settings.column_width())
        .style(style::float_e2(t, R_BUBBLE));
    // `InputColumn`: `.padding(.bottom, 12).padding(.top, 6)` outside the column, and the width setting's 28 pt side
    // padding around it (`Sources/HerdrDesk/Reading.swift:84-85,103`).
    container(card)
        .padding(iced::Padding {
            top: 6.0,
            right: 28.0,
            bottom: 12.0,
            left: 28.0,
        })
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .into()
}

/// A tooltip's label box, the same float the rest of the app uses for tips.
fn tip_box<'a>(t: Tokens, text: &'a str) -> Element<'a, Message> {
    container(CALLOUT.text(text).color(t.label))
        .padding([5, 9])
        .style(style::float_e2(t, R_ROW))
        .into()
}

/// A small secondary button inside the card: hairline border, quiet fill only on hover. This is the Swift app's
/// `.controlSize(.small)` button (`Sources/HerdrDesk/QueuedView.swift:152`), not iced's filled accent default.
fn quiet<'a>(t: Tokens, label: String, enabled: bool, msg: Message) -> Element<'a, Message> {
    button(
        CAPTION
            .text(label)
            .color(if enabled { t.label_2 } else { t.label_3 }),
    )
    .padding([3, 9])
    .on_press_maybe(enabled.then_some(msg))
    .style(move |_, status| button::Style {
        background: matches!(status, button::Status::Hovered | button::Status::Pressed)
            .then(|| Background::Color(t.fill_2)),
        border: Border {
            radius: 6.0.into(),
            width: 1.0,
            color: t.sep,
            ..Default::default()
        },
        text_color: if enabled { t.label_2 } else { t.label_3 },
        ..Default::default()
    })
    .into()
}

/// The card surface the icons sit on (the svg renderer drops icon alpha, so colors are blended by hand).
fn card_bg(t: Tokens) -> iced::Color {
    if t.mode.is_dark() {
        rgba(48, 48, 52, 1.0)
    } else {
        rgba(255, 255, 255, 1.0)
    }
}

/// The + and mic icons: secondary label color, dimmed to the tertiary one when input is off.
pub fn attach_icon_color(t: Tokens, enabled: bool) -> iced::Color {
    over(if enabled { t.label_2 } else { t.label_3 }, card_bg(t))
}

/// `~/x` as an absolute path (pane working folders come with `~`).
fn expand_home(cwd: &str) -> PathBuf {
    match cwd.strip_prefix('~') {
        Some(rest) => dirs::home_dir()
            .map(|h| h.join(rest.trim_start_matches('/')))
            .unwrap_or_else(|| PathBuf::from(cwd)),
        None => PathBuf::from(cwd),
    }
}

// ---------------------------------------------------------------- branch line

fn branch_for(app: &App) -> Option<String> {
    if let Some(b) = &app.state.composer.branch_override {
        return Some(b.clone());
    }
    let cwd = app.state.selected_pane().map(|p| p.cwd.clone())?;
    git_branch(&cwd)
}

type BranchCache = Mutex<HashMap<String, (Instant, Option<String>)>>;
fn cache() -> &'static BranchCache {
    static C: std::sync::OnceLock<BranchCache> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// Current git branch of the repo containing `cwd`. Read-only: reads `.git/HEAD`, never runs `git`.
/// Cached per cwd for 10 s (the view calls this on every frame). `None` outside a repo or on a detached HEAD.
pub fn git_branch(cwd: &str) -> Option<String> {
    if let Some((at, v)) = cache().lock().ok()?.get(cwd) {
        if at.elapsed() < Duration::from_secs(10) {
            return v.clone();
        }
    }
    let v = read_branch(cwd);
    if let Ok(mut c) = cache().lock() {
        c.insert(cwd.to_string(), (Instant::now(), v.clone()));
    }
    v
}

fn read_branch(cwd: &str) -> Option<String> {
    let mut dir = if let Some(rest) = cwd.strip_prefix('~') {
        std::path::PathBuf::from(std::env::var_os("HOME")?).join(rest.trim_start_matches('/'))
    } else {
        std::path::PathBuf::from(cwd)
    };
    loop {
        let git = dir.join(".git");
        if git.is_dir() {
            return parse_head(&std::fs::read_to_string(git.join("HEAD")).ok()?);
        }
        if git.is_file() {
            // Worktree: `gitdir: <path>`.
            let s = std::fs::read_to_string(&git).ok()?;
            let p = s.trim().strip_prefix("gitdir:")?.trim();
            let p = if std::path::Path::new(p).is_absolute() {
                std::path::PathBuf::from(p)
            } else {
                dir.join(p)
            };
            return parse_head(&std::fs::read_to_string(p.join("HEAD")).ok()?);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub fn parse_head(s: &str) -> Option<String> {
    s.trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
        .filter(|b| !b.is_empty())
}

/// Installs the live sender for `session`. Only called from the live path in `main.rs`.
#[cfg(unix)]
pub fn install_live(app: &mut App, session: &herdr_core::HerdrSession) {
    let sender = Arc::new(LiveSender::new(session));
    match sender.journal.read() {
        Ok(entries) => {
            for entry in &entries {
                if !app
                    .state
                    .composer
                    .recovery
                    .iter()
                    .any(|old| old.id == entry.id)
                {
                    append_for(&mut app.state.composer, &entry.pane, &entry.text);
                }
            }
            app.state.composer.recovery = entries;
        }
        Err(e) => app.state.composer.error = Some(format!("Cannot load pulled-text recovery: {e}")),
    }
    app.state.composer.queue_reader = Some(sender.clone());
    app.state.composer.sender = Some(sender);
    app.state.composer.picker = Some(Arc::new(NativePicker));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn head_parsing() {
        assert_eq!(
            parse_head("ref: refs/heads/feature/x\n").as_deref(),
            Some("feature/x")
        );
        assert_eq!(parse_head("3f2a9c0d1e\n"), None);
    }
}

fn insert_for(c: &mut ComposerState, pane: &str, text: &str) {
    if c.sender.is_none() {
        return;
    }
    if c.pane.as_deref() == Some(pane) {
        c.content
            .perform(Action::Edit(text_editor::Edit::Paste(Arc::new(
                text.into(),
            ))));
    } else {
        c.drafts.entry(pane.into()).or_default().push_str(text);
    }
}
fn append_for(c: &mut ComposerState, pane: &str, text: &str) {
    if c.pane.as_deref() == Some(pane) {
        let old = c.text();
        c.set_text(&if old.is_empty() {
            text.into()
        } else {
            format!("{old}\n{text}")
        });
    } else {
        let draft = c.drafts.entry(pane.into()).or_default();
        if !draft.is_empty() {
            draft.push('\n');
        }
        draft.push_str(text);
    }
}
fn image_path(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp"
        )
    })
}
/// `chips`: turn image files into preview chips instead of inserting their paths. Only a drop does that. The + picker
/// inserts every chosen path as text, images and folders included (`attach()`, `Sources/HerdrDesk/Composer.swift:260`),
/// because the agent reads a path as an attachment.
fn attach_paths(app: &mut App, pane: String, paths: Vec<PathBuf>, chips: bool) -> Task<Message> {
    if app.state.composer.sender.is_none() {
        return Task::none();
    }
    let (images, files): (Vec<_>, Vec<_>) = if chips {
        paths
            .into_iter()
            .partition(|p| image_path(p) && !p.is_dir())
    } else {
        (Vec::new(), paths)
    };
    if files.iter().any(|p| p.to_str().is_none()) {
        app.state.composer.error = Some("Attachment path is not valid UTF-8".into());
        return Task::none();
    }
    if !files.is_empty() {
        insert_for(
            &mut app.state.composer,
            &pane,
            &format!(
                "{} ",
                files
                    .iter()
                    .map(|p| quoted(p))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
    }
    if images.is_empty() {
        if app.state.composer.pane.as_deref() != Some(&pane) {
            return Task::none();
        }
        iced::widget::operation::focus(EDITOR_ID)
    } else {
        image_job(app, pane, images, false)
    }
}
fn image_job(app: &mut App, pane: String, paths: Vec<PathBuf>, clipboard: bool) -> Task<Message> {
    if paths.len() > 16 || app.state.composer.image_jobs >= 4 {
        app.state.composer.error =
            Some("Attach up to 16 images at a time; wait for current pastes to finish".into());
        return Task::none();
    }
    app.state.composer.image_jobs += 1;
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<Attachment>, String> {
            let mut images = vec![];
            if clipboard {
                images.push(store_image(&clipboard_image()?)?);
            } else {
                for path in paths {
                    let mut bytes = vec![];
                    use std::io::Read;
                    std::fs::File::open(path)
                        .map_err(|e| e.to_string())?
                        .take((16 << 20) + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| e.to_string())?;
                    images.push(store_image(&bytes)?);
                }
            }
            Ok(images)
        })();
        let _ = tx.send(result);
    });
    Task::perform(
        async move {
            rx.await
                .unwrap_or_else(|_| Err("Image capture stopped".into()))
        },
        move |result| {
            let (images, error) = match result {
                Ok(images) => (images, None),
                Err(e) => (vec![], Some(e)),
            };
            Message::Composer(ComposerMsg::Images {
                pane: pane.clone(),
                images,
                error,
            })
        },
    )
}
fn store_image(bytes: &[u8]) -> Result<Attachment, String> {
    if bytes.len() > 16 << 20 {
        return Err("Image exceeds 16 MB".into());
    }
    let (format, _, _) = herdr_core::log_image::header(bytes).ok_or("Unsupported image")?;
    let preview = crate::pictures::decode(bytes, 96)
        .ok_or("Image cannot be decoded safely")?
        .handle();
    let ext = match format {
        herdr_core::log_image::ImageFormat::Png => "png",
        herdr_core::log_image::ImageFormat::Jpeg => "jpg",
        herdr_core::log_image::ImageFormat::Gif => "gif",
        herdr_core::log_image::ImageFormat::Webp => "webp",
    };
    let dir = dirs::cache_dir()
        .ok_or("No image cache directory")?
        .join("Flockbook/paste");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = dir.join(format!(
        "paste-{stamp}-{}-{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        ext
    ));
    let mut opts = std::fs::OpenOptions::new();
    opts.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    use std::io::Write;
    let mut file = opts.open(&path).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    Ok(Attachment { path, preview })
}
/// No clipboard crate/system package is installed. Use the platform service if present, with bounded output/time.
fn clipboard_command(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut out = child.stdout.take().ok_or("Clipboard output unavailable")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = vec![];
        let result = out
            .by_ref()
            .take((36 << 20) + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
            .map_err(|e| e.to_string());
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|_| "Clipboard service timed out".to_string())
        .and_then(|r| r);
    let _ = child.kill();
    let _ = child.wait();
    let bytes = result?;
    if bytes.len() > 36 << 20 {
        return Err("Clipboard image exceeds the safe limit".into());
    }
    Ok(bytes)
}
#[cfg(target_os = "linux")]
fn clipboard_image() -> Result<Vec<u8>, String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        clipboard_command("wl-paste", &["--no-newline", "--type", "image/png"])
    } else {
        clipboard_command(
            "xclip",
            &["-selection", "clipboard", "-t", "image/png", "-o"],
        )
    }
    .map_err(|_| "Cannot read an image from the clipboard service (wl-paste or xclip)".into())
}
#[cfg(target_os = "macos")]
fn clipboard_image() -> Result<Vec<u8>, String> {
    let raw = clipboard_command("osascript", &["-e", "get the clipboard as «class PNGf»"])?;
    let s = std::str::from_utf8(&raw).map_err(|e| e.to_string())?.trim();
    let hex = s
        .strip_prefix("«data PNGf")
        .and_then(|s| s.strip_suffix('»'))
        .ok_or("Clipboard has no PNG image")?;
    if hex.len() % 2 != 0 || !hex.is_ascii() {
        return Err("Invalid clipboard image".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn clipboard_image() -> Result<Vec<u8>, String> {
    Err("Clipboard images are unavailable on this platform; attach an image file".into())
}

fn poll_queue(app: &mut App) -> Task<Message> {
    let c = &app.state.composer;
    if c.queue_reading || c.pulling {
        return Task::none();
    }
    let Some(sender) = c.queue_reader.clone().or_else(|| c.sender.clone()) else {
        return Task::none();
    };
    let Some(pane) = app.state.selected_pane().cloned() else {
        return Task::none();
    };
    let Some(identity) = AgentIdentity::for_pane(&pane) else {
        return Task::none();
    };
    if !matches!(identity.kind.as_str(), "claude" | "codex" | "grok") {
        return Task::none();
    }
    let path = app.source.log_path(&pane);
    app.state.composer.queue_reading = true;
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    let expect = identity.clone();
    std::thread::spawn(move || {
        let log = if expect.kind == "claude" {
            crate::queued::log_snapshot(path.as_deref()).unwrap_or_default()
        } else {
            ClaudeQueueLog::default()
        };
        let queue = sender.queue(&expect, &log).ok().flatten();
        let _ = tx.send((queue, log));
    });
    Task::perform(
        async move { rx.await.unwrap_or_default() },
        move |(queue, log)| {
            Message::Composer(ComposerMsg::QueueRead {
                identity: identity.clone(),
                queue,
                log,
            })
        },
    )
}
fn pull_queue(app: &mut App) -> Task<Message> {
    if app.state.settings.open || app.state.terminal.is_open() {
        return Task::none();
    }
    let c = &app.state.composer;
    if c.pulling || c.sending {
        return Task::none();
    }
    let Some(sender) = c.sender.clone() else {
        return Task::none();
    };
    let Some(pane) = app.state.selected_pane().cloned() else {
        return Task::none();
    };
    let Some(identity) = AgentIdentity::for_pane(&pane) else {
        return Task::none();
    };
    let path = app.source.log_path(&pane);
    app.state.composer.pulling = true;
    let pane_id = pane.id;
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let outcome = match crate::queued::log_snapshot(path.as_deref()) {
            Ok(log) => sender.pull(&identity, &log),
            Err(e) => PullOutcome::Refused(e),
        };
        let _ = tx.send(outcome);
    });
    Task::perform(
        async move {
            rx.await.unwrap_or_else(|_| {
                PullOutcome::Failed("Pull stopped; recovery copy is retained on disk".into())
            })
        },
        move |outcome| {
            Message::Composer(ComposerMsg::Pulled {
                pane: pane_id.clone(),
                outcome,
            })
        },
    )
}
pub fn subscription(app: &App) -> iced::Subscription<Message> {
    let poll = iced::time::every(Duration::from_millis(1600))
        .map(|_| Message::Composer(ComposerMsg::PollQueue));
    if app.state.settings.open
        || app.state.terminal.is_open()
        || app.state.screen != crate::state::Screen::Conversation
    {
        return poll;
    }
    let drops = iced::event::listen().filter_map(|event| match event {
        iced::Event::Window(iced::window::Event::FileDropped(path)) => {
            Some(Message::Composer(ComposerMsg::Drop(path)))
        }
        _ => None,
    });
    iced::Subscription::batch([poll, drops])
}

#[cfg(unix)]
pub fn install_observer(app: &mut App, session: &herdr_core::HerdrSession) {
    let sender = Arc::new(LiveSender::new(session));
    sender.revoke();
    app.state.composer.queue_reader = Some(sender);
}
/// File URI lists identify file clipboard payloads. Plain copied paths remain ordinary text.
fn clipboard_files(text: &str) -> Option<Vec<PathBuf>> {
    let mut out = vec![];
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let uri = line.strip_prefix("file://")?;
        let path = if uri.starts_with('/') {
            uri
        } else {
            uri.strip_prefix("localhost")?
        };
        let bytes = path.as_bytes();
        let mut decoded = vec![];
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                decoded.push(u8::from_str_radix(path.get(i + 1..i + 3)?, 16).ok()?);
                i += 3;
            } else {
                decoded.push(bytes[i]);
                i += 1;
            }
        }
        let path = PathBuf::from(String::from_utf8(decoded).ok()?);
        if !image_path(&path) {
            return None;
        }
        out.push(path);
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod visual_state_tests {
    use super::*;
    use fb_ui::Mode;

    struct Sender;
    impl MessageSender for Sender {
        fn send(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    /// The queue strip and the failed-send recovery use quiet buttons, not iced's filled accent default. Writes the
    /// shot to the test temp dir, and to `evidence/` with `FB_WRITE_EVIDENCE=1` like the other snapshots.
    #[test]
    fn queue_and_failed_rows_render_quiet_buttons() {
        let mut app = App::demo(Mode::Light, crate::state::Screen::Conversation);
        app.state.composer.sender = Some(Arc::new(Sender));
        let pane = app.state.selected_pane().unwrap().id.clone();
        app.state.composer.pane = Some(pane.clone());
        app.state.composer.queues.insert(
            pane.clone(),
            QueuedInput {
                agent: "claude".into(),
                items: vec![Some("also run the bench on the new decay".into())],
                pullable: true,
            },
        );
        app.state
            .composer
            .failed
            .insert(pane, vec![("an earlier draft".into(), vec![])]);
        let img = crate::snapshot::render_app(&app, (1280, 852));
        let dir = std::env::var_os("CARGO_TARGET_TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        crate::snapshot::write_png(&img, &dir.join("composer-queue-light.png")).unwrap();
        if std::env::var_os("FB_WRITE_EVIDENCE").is_some() {
            crate::snapshot::write_png(
                &img,
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("evidence")
                    .join("composer-queue-light.png"),
            )
            .unwrap();
        }
        assert!(crate::snapshot::distinct_colors(&img) > 40);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    struct Sender;
    impl MessageSender for Sender {
        fn send(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn drafts_preserve_whitespace_and_combining_scalars() {
        let mut c = ComposerState::default();
        c.sender = Some(Arc::new(Sender));
        c.switch_pane(Some("one"));
        c.set_text("  e\u{301}\n\n");
        c.switch_pane(Some("two"));
        c.set_text("second");
        c.switch_pane(Some("one"));
        assert_eq!(c.text(), "  e\u{301}\n\n");
        assert_eq!(c.drafts["two"], "second");
    }
    #[test]
    fn late_paste_is_kept_in_original_pane() {
        let mut c = ComposerState::default();
        c.sender = Some(Arc::new(Sender));
        c.switch_pane(Some("two"));
        c.set_text("new");
        insert_for(&mut c, "one", "late result\n\n");
        assert_eq!(c.text(), "new");
        assert_eq!(c.drafts["one"], "late result\n\n");
    }
    #[test]
    fn pull_appends_without_overwriting_new_draft() {
        let mut c = ComposerState::default();
        c.switch_pane(Some("one"));
        c.set_text("new");
        append_for(&mut c, "one", "old\n\n");
        assert_eq!(c.text(), "new\nold\n\n");
        append_for(&mut c, "two", "other");
        assert_eq!(c.drafts["two"], "other");
    }
    #[test]
    fn file_uri_images_are_distinct_from_copied_path_text() {
        assert_eq!(
            clipboard_files("file:///tmp/a%20b.png\n"),
            Some(vec![PathBuf::from("/tmp/a b.png")])
        );
        assert!(clipboard_files("/tmp/a.png").is_none());
        assert!(clipboard_files("file://remote/tmp/a.png").is_none());
    }
}

/// Observe composition at the widget boundary, before Enter reaches the editor's custom submit binding.
/// A subscription alone can deliver preedit state after the key; this guard shares state synchronously.
struct ImeGuard<'a> {
    content: Element<'a, Message>,
    composing: Arc<AtomicBool>,
}
impl iced::advanced::Widget<Message, iced::Theme, iced::Renderer> for ImeGuard<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        self.content.as_widget().tag()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        self.content.as_widget().state()
    }
    fn children(&self) -> Vec<iced::advanced::widget::Tree> {
        self.content.as_widget().children()
    }
    fn diff(&self, tree: &mut iced::advanced::widget::Tree) {
        self.content.as_widget().diff(tree)
    }
    fn size(&self) -> iced::Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> iced::Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn layout(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        renderer: &iced::Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }
    fn draw(
        &self,
        tree: &iced::advanced::widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::advanced::mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport)
    }
    fn operate(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation)
    }
    fn update(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::advanced::mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &iced::Rectangle,
    ) {
        use iced::advanced::input_method;
        let focused = tree
            .state
            .downcast_ref::<text_editor::State<iced::advanced::text::highlighter::PlainText>>()
            .is_focused();
        if !focused {
            self.composing.store(false, Ordering::SeqCst);
        }
        match event {
            iced::Event::InputMethod(input_method::Event::Preedit(content, _)) if focused => {
                self.composing.store(!content.is_empty(), Ordering::SeqCst)
            }
            iced::Event::InputMethod(
                input_method::Event::Commit(_) | input_method::Event::Closed,
            )
            | iced::Event::Window(iced::window::Event::Unfocused) => {
                self.composing.store(false, Ordering::SeqCst)
            }
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: Key::Named(Named::Enter),
                ..
            }) if focused && self.composing.load(Ordering::SeqCst) => {
                shell.capture_event();
                return;
            }
            _ => {}
        }
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        )
    }
    fn mouse_interaction(
        &self,
        tree: &iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::advanced::mouse::Cursor,
        viewport: &iced::Rectangle,
        renderer: &iced::Renderer,
    ) -> iced::advanced::mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &iced::Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'a, Message, iced::Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod ime_tests {
    use super::*;
    use iced::advanced::input_method;
    use iced::{mouse, Event, Font, Pixels, Point, Size};
    use iced_test::core::{clipboard, renderer::Headless};
    use iced_test::runtime::user_interface::{Cache, UserInterface};
    struct Sender;
    impl MessageSender for Sender {
        fn send(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    #[test]
    fn preedit_enter_does_not_submit_existing_text_or_an_image() {
        for with_image in [false, true] {
            let mut app = App::demo(fb_ui::Mode::Light, crate::state::Screen::Conversation);
            app.state.composer.sender = Some(Arc::new(Sender));
            app.state.composer.set_text("existing");
            if with_image {
                app.state.composer.images.push(Attachment {
                    path: PathBuf::from("/synthetic/image.png"),
                    preview: iced::widget::image::Handle::from_rgba(1, 1, vec![255; 4]),
                });
            }
            let mut renderer = iced_test::futures::futures::executor::block_on(
                <iced::Renderer as Headless>::new(Font::DEFAULT, Pixels(16.0), Some("tiny-skia")),
            )
            .expect("headless renderer");
            let mut cache = Cache::default();
            let cursor =
                mouse::Cursor::Available(Point::new(200.0, if with_image { 125.0 } else { 30.0 }));
            let key = Key::Named(Named::Enter);
            let events = [
                Event::Window(iced::window::Event::Focused),
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                Event::InputMethod(input_method::Event::Opened),
                Event::InputMethod(input_method::Event::Preedit("候選".into(), Some(0..6))),
                Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: key.clone(),
                    modified_key: key,
                    physical_key: iced::keyboard::key::Physical::Unidentified(
                        iced::keyboard::key::NativeCode::Unidentified,
                    ),
                    location: iced::keyboard::Location::Standard,
                    modifiers: iced::keyboard::Modifiers::empty(),
                    text: None,
                    repeat: false,
                }),
            ];
            for event in events {
                let mut messages = vec![];
                let mut ui =
                    UserInterface::build(view(&app), Size::new(640.0, 360.0), cache, &mut renderer);
                ui.update(
                    &[event],
                    cursor,
                    &mut renderer,
                    &mut clipboard::Null,
                    &mut messages,
                );
                cache = ui.into_cache();
                assert!(!messages
                    .iter()
                    .any(|m| matches!(m, Message::Composer(ComposerMsg::Submit))));
                for message in messages {
                    let _ = app.update(message);
                }
            }
            assert!(app.state.composer.composing.load(Ordering::SeqCst));
            assert_eq!(app.state.composer.text(), "existing");
            assert!(app.state.composer.pending.is_empty());
            assert_eq!(app.state.composer.images.len(), usize::from(with_image));
        }
    }
}

/// Pause in-flight composer steps while another sheet owns keyboard input. Reading queues remains available.
pub fn sync_permissions(app: &App) {
    if let Some(sender) = &app.state.composer.sender {
        sender.set_paused(
            app.state.settings.open
                || app.state.terminal.is_open()
                || crate::workspace_actions::modal(app)
                || app.state.shortcuts.open,
        );
    }
}
