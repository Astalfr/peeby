//! Client for YouTube's private InnerTube API (the JSON API behind www.youtube.com).
//!
//! There is no official API for the home feed, so this crate speaks InnerTube directly:
//! - [`Client::home`]: the "Home" recommendations (`FEwhat_to_watch`), personalized only when
//!   signed in with browser [`Cookies`] (anonymous Home is empty since 2024);
//! - [`Client::watch`]: watch page metadata, related videos and the comments token (`next`);
//! - [`Client::stream`]: playable HLS streams through the VISIONOS client, which needs no JS
//!   challenge and has no `available_at` delay (see docs/decisions/0003-visionos-hls-streams.md).
//!
//! Responses are parsed leniently from `serde_json::Value`: YouTube changes renderers often,
//! and one malformed item must never fail a whole feed. Every parser is tested against real
//! responses in `tests/fixtures/`.

mod auth;
mod client;
mod error;
pub mod hls;
mod models;
mod parse;

pub use auth::Cookies;
pub use client::{Client, VISIONOS_USER_AGENT, hls_manifest_url};
pub use error::Error;
pub use models::{Feed, Stream, Thumbnail, VideoItem, WatchPage};
pub use parse::{parse_feed, parse_watch};
