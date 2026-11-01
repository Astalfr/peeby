//! Peeby's shared UI: the theme (with the accent of the open space), motion, icons, the image
//! store, and the components every space builds its pages from. Nothing here knows about a
//! service: spaces convert their data into these components' models (docs/refactor-plan.md).

pub mod assets;
pub mod card;
pub mod form;
pub mod images;
pub mod motion;
pub mod picture;
pub mod player;
pub mod runtime;
pub mod theme;
pub mod widgets;
