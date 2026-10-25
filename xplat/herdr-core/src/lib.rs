//! Herdr link core: a Rust port of HerdrKit (models, snapshot parsing, line-socket client, conversation parsers).
//!
//! Read-only by design in this gate: the client exposes `session.snapshot` and `events.subscribe`. `raw_request` is
//! generic, but nothing here sends input, keys, resize, takeover or any other mutating method.

pub mod client;
pub mod conversation;
pub mod error;
pub mod model;
pub mod question;
pub mod session;
pub mod snapshot;
pub mod transport;
pub mod util;

pub use client::{EventStream, HerdrClient};
#[cfg(unix)]
pub use client::UnixConnector;
pub use conversation::{parse_lines, ConversationParser};
pub use error::HerdrError;
pub use model::*;
pub use session::HerdrSession;
pub use transport::{Connector, LineSocket, Transport};

/// Text Claude Code draws on its permission prompt (port of `PermissionGate`).
pub const PROMPT_MARKER: &str = "Do you want";

/// Does the pane's visible screen show a permission prompt? Only the screen tells a prompt from a running tool.
pub fn screen_shows_prompt(screen_text: &str) -> bool {
    screen_text.contains(PROMPT_MARKER) || approval::grok_box(screen_text).is_some()
}

pub mod approval;
pub mod scrollback;
pub mod store;
pub mod terminal;
pub mod resolver;
pub mod session_log;
pub use terminal::TerminalFrame;

pub mod log_image;
pub use log_image::{ImageFormat, ImageRef};

pub mod status_line;
pub mod grok_status_line;

pub mod shell_quote;
pub mod image_paste;
pub mod message_send;
pub mod queued_input;
