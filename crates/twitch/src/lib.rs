//! Twitch client for Peeby (docs/twitch-plan.md, ADR 0010). No GPUI.
//!
//! - [`Client::playback_token`]: the GQL playback token of a live or a VOD, with the raw query and
//!   the web Client-ID ([`Client::refresh_client_id`] keeps it current), `playerType=embed`;
//! - [`Client::master`]: the usher master playlist (v2, then v1) and its [`usher::Variant`]s.
//!
//! Responses are parsed leniently from `serde_json::Value` and HLS text, and every parser is
//! tested against real, redacted responses in `tests/fixtures/`. Tokens, signatures and usher or
//! CDN URLs carry the user's IP: they never appear in logs or error messages.

pub mod ads;
pub mod auth;
pub mod chat;
mod client;
mod error;
pub mod gql;
pub mod helix;
mod proxy;
pub mod seventv;
pub mod splice;
pub mod time;
pub mod usher;

pub use client::{Client, Master};
pub use error::{Error, Result};
pub use gql::{PlaybackToken, Target};
pub use proxy::Proxy;

/// Runs `future` to its end on a new single-thread runtime, for command-line tools and examples
/// (the app runs this crate on its own network runtime).
pub fn block_on<F: std::future::Future>(future: F) -> std::io::Result<F::Output> {
  Ok(
    tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()?
      .block_on(future),
  )
}
