//! Herdr link core: a Rust port of HerdrKit (models, snapshot parsing, line-socket client, conversation parsers).
//!
//! Read-only by design in this gate: the client exposes `session.snapshot` and `events.subscribe`. `raw_request` is
//! generic, but nothing here sends input, keys, resize, takeover or any other mutating method.

pub mod client;
pub mod conversation;
pub mod error;
pub mod fake;
pub mod hc;
pub mod keygate;
pub mod log_image;
pub mod model;
pub mod question;
pub mod screen;
pub mod session;
pub mod snapshot;
pub mod transport;
pub mod util;

pub use client::{EventStream, HerdrClient};
#[cfg(unix)]
pub use client::UnixConnector;
pub use conversation::{parse_lines, ConversationParser};
pub use error::{HerdrError, QuestionFailure};
pub use screen::{all_panes, DeliveryOutcome, PaneScreen};
pub use model::*;
pub use session::HerdrSession;
pub use transport::{Connector, LineSocket, Transport};

/// Swift `PermissionGate`: the prompt marker and the whole-prompt check (only the screen tells a prompt from a
/// running tool). One implementation, in `approval`.
pub use approval::{screen_shows_prompt, PROMPT_MARKER};

pub mod approval;
pub mod scrollback;
pub mod store;
pub mod terminal;
pub mod resolver;
pub mod session_log;
pub mod statusline;
pub use terminal::TerminalFrame;
