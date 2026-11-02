//! Peeby's shared UI: the theme (with the accent of the open space), motion, icons, the image
//! store, and the components every space builds its pages from. Nothing here knows about a
//! service: spaces convert their data into these components' models (docs/refactor-plan.md).

pub mod assets;
pub mod card;
pub mod chat;
pub mod form;
pub mod i18n;
pub mod images;
pub mod motion;
pub mod picture;
pub mod player;
pub mod rows;
pub mod runtime;
pub mod splash;
pub mod text;
pub mod theme;
pub mod widgets;

/// Moves and resizes a window without activating it (patched GPUI), for the mini-player.
pub use gpui_video::set_window_bounds;
