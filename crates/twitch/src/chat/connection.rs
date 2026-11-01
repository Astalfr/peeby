//! The chat's connection: Twitch's IRC over WebSocket, anonymous and read-only (`justinfan<n>`,
//! which needs no token). It joins one channel, answers pings, follows `RECONNECT`, and reconnects
//! after an error with a growing delay (1, 2, 4… 30 s). It stops when its receiver is dropped.

use std::hash::{BuildHasher, RandomState};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::irc::{self, Event};

const URL: &str = "wss://irc-ws.chat.twitch.tv:443";
/// Longest delay before reconnecting.
const MAX_DELAY: Duration = Duration::from_secs(30);
/// Without a line for this long (Twitch pings every ~5 min), the connection is taken as dead.
const SILENCE: Duration = Duration::from_secs(6 * 60);
/// Events kept for the page before the connection waits for it to catch up.
const QUEUE: usize = 1024;

/// What the connection sends its page.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
  /// An event of the chat; never [`Event::Ping`] nor [`Event::Reconnect`], handled here.
  Event(Event),
  /// The connection's state changed.
  Status(Status),
}

/// The connection's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
  /// Connecting, or joining the channel.
  Connecting,
  /// The channel is joined.
  Connected,
  /// Lost: the next try is in this many seconds.
  Retrying(u64),
}

/// Starts reading `channel`'s chat (a login). Run the returned future on a tokio runtime; it ends
/// when the receiver is dropped.
pub fn connect(
  channel: &str,
) -> (
  mpsc::Receiver<Update>,
  impl Future<Output = ()> + Send + 'static,
) {
  let (tx, rx) = mpsc::channel(QUEUE);
  let channel = channel.to_ascii_lowercase();
  (rx, run(channel, tx))
}

async fn run(channel: String, tx: mpsc::Sender<Update>) {
  let mut delay = Duration::from_secs(1);
  loop {
    if tx.send(Update::Status(Status::Connecting)).await.is_err() {
      return;
    }
    let mut joined = false;
    match session(&channel, &tx, &mut joined).await {
      // The page is gone.
      Ok(Ended::Closed) => return,
      // Asked to by the server: at once.
      Ok(Ended::Reconnect) => {
        delay = Duration::from_secs(1);
        continue;
      }
      Err(e) => tracing::info!("Twitch chat of {channel}: {e}"),
    }
    // A connection that worked starts the delays over.
    if joined {
      delay = Duration::from_secs(1);
    }
    if tx
      .send(Update::Status(Status::Retrying(delay.as_secs())))
      .await
      .is_err()
    {
      return;
    }
    tokio::select! {
      () = tokio::time::sleep(delay) => {}
      () = tx.closed() => return,
    }
    delay = (delay * 2).min(MAX_DELAY);
  }
}

enum Ended {
  Closed,
  Reconnect,
}

async fn session(
  channel: &str,
  tx: &mpsc::Sender<Update>,
  joined: &mut bool,
) -> Result<Ended, String> {
  let (mut socket, _) = tokio_tungstenite::connect_async(URL)
    .await
    .map_err(|e| format!("cannot connect: {e}"))?;
  let nick = format!(
    "justinfan{}",
    RandomState::new().hash_one(channel) % 90_000 + 10_000
  );
  for line in [
    "CAP REQ :twitch.tv/tags twitch.tv/commands".to_owned(),
    "PASS SCHMOOPIIE".to_owned(),
    format!("NICK {nick}"),
    format!("JOIN #{channel}"),
  ] {
    socket
      .send(Message::text(line))
      .await
      .map_err(|e| format!("cannot send: {e}"))?;
  }
  loop {
    let frame = tokio::select! {
      frame = tokio::time::timeout(SILENCE, socket.next()) => frame,
      () = tx.closed() => return Ok(Ended::Closed),
    };
    let text = match frame {
      Err(_) => return Err("no line for 6 minutes".into()),
      Ok(None) => return Err("closed by the server".into()),
      Ok(Some(Err(e))) => return Err(format!("read: {e}")),
      Ok(Some(Ok(Message::Text(text)))) => text,
      Ok(Some(Ok(Message::Close(_)))) => return Err("closed by the server".into()),
      Ok(Some(Ok(_))) => continue,
    };
    for line in text.as_str().split("\r\n").filter(|l| !l.is_empty()) {
      let Some(event) = irc::parse(line) else {
        continue;
      };
      let update = match event {
        Event::Ping(server) => {
          socket
            .send(Message::text(format!("PONG :{server}")))
            .await
            .map_err(|e| format!("cannot send: {e}"))?;
          continue;
        }
        Event::Reconnect => return Ok(Ended::Reconnect),
        Event::Joined => {
          *joined = true;
          Update::Status(Status::Connected)
        }
        other => Update::Event(other),
      };
      if tx.send(update).await.is_err() {
        return Ok(Ended::Closed);
      }
    }
  }
}
