//! Peeby's core: the shell that hosts the spaces (windows, tabs, mini-player, settings page),
//! the settings and the session cookies, the desktop integration (notification area, toasts,
//! single instance), the updates and the sign-in window. It knows no service: each space is a
//! crate of its own (`space-youtube`, `space-twitch`, `space-jellyfin`) that plugs into
//! [`shell::space::Space`], and the `peeby` binary puts them together (ADR 0029).

pub mod desktop;
pub mod logs;
pub mod settings;
pub mod shell;
pub mod update;
pub mod web_sign_in;
