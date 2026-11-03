//! herdr-core (`hc`): where it listens, its plain-mode JSON-RPC client, and its terminals mapped to the model.

pub mod client;
pub mod keys;
pub mod mapper;
pub mod session;
pub mod socket;

pub use client::HcClient;
pub use session::HcSession;
