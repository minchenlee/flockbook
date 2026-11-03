//! HerdrKit in Rust: the parts of Flockbook's Swift `HerdrKit` that the herdr-app mod's sidecar
//! (`flockbook-modfeed`) reaches. No UI, no TUI (`herdr` 0.9) backend: herdr-core (`hc`) only.

pub mod fake;
pub mod hc;
pub mod keygate;
pub mod log_image;
pub mod models;
pub mod parser;
pub mod question_adapter;
pub mod session_log;
pub mod session_resolver;
pub mod statusline;

pub use models::*;
