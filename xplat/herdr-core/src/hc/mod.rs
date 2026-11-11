//! herdr-core (`hc`): where it listens, its plain-mode JSON-RPC client, and its terminals mapped to the model
//! (Swift `HCSession`, `HCClient`, `HCKeys`, `HCMapper`). The line framing is `crate::transport::LineSocket`.

pub mod client;
pub mod keys;
pub mod mapper;
pub mod observe;
pub mod session;

pub use client::HcClient;
pub use session::HcSession;
