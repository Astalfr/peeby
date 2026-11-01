//! Typed Jellyfin 12 REST client: authentication, browsing, images, playback reporting and the direct-play device profile.
//!
//! The client is async and built on `reqwest`, so it must run inside a tokio runtime.
//! Reference spec: `docs/reference/openapi-12.4.0.json` (only endpoints that also exist in 12.0 are used).

mod auth;
mod client;
mod error;
mod ids;
pub mod media;
pub mod models;
pub mod playback;

pub use auth::ClientInfo;
pub use client::{Client, ImageOptions, ItemsQuery, NextUpOptions, Session};
pub use error::{Error, Result};
pub use ids::{ItemId, UserId};
