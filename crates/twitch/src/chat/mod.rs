//! Twitch chat (docs/twitch-plan.md, step 9): the IRC parser ([`irc`]), the anonymous WebSocket
//! connection that reads a room ([`connect`]), and the signed-in one that sends to it
//! ([`connect_writer`], ADR 0023). The app turns their events into `ui::chat`'s.

mod connection;
pub mod irc;
mod writer;

pub use connection::{Status, Update, connect};
pub use irc::{ChatLine, EmoteRange, Event, Piece, RoomState, emote_url, gif_link, gif_url};
pub use writer::{
  MAX_CHARS, RATE_COUNT, RATE_WINDOW, Writer, WriterStatus, WriterUpdate, clean, connect_writer,
  outgoing,
};
