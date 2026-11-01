//! Twitch chat, read-only (docs/twitch-plan.md, step 9): the IRC parser ([`irc`]) and the
//! anonymous WebSocket connection ([`connect`]). The app turns its events into `ui::chat`'s.

mod connection;
pub mod irc;

pub use connection::{Status, Update, connect};
pub use irc::{ChatLine, EmoteRange, Event, Piece, RoomState, emote_url};
