//! Sending to a channel's chat (ADR 0023): a second IRC connection, signed in with the web
//! `auth-token` (`PASS oauth:<token>`, `NICK <login>`), that only writes. The anonymous reader
//! ([`super::connect`]) keeps showing the room: Twitch sends a message to every connection in the
//! room but the one that wrote it, so the reader shows the viewer's own lines with their tags
//! (badges, color, emotes) and nothing is echoed here.
//!
//! Lines are cleaned before they leave ([`outgoing`]): no line break (it would start another IRC
//! command), at most [`MAX_CHARS`]. At most [`RATE_COUNT`] messages per [`RATE_WINDOW`] leave,
//! Twitch's limit for a viewer, beyond which it locks the account out of chat for 30 minutes.
//! The token never reaches a log or an error message.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::irc;

const URL: &str = "wss://irc-ws.chat.twitch.tv:443";
/// Longest delay before reconnecting.
const MAX_DELAY: Duration = Duration::from_secs(30);
/// Without a line for this long (Twitch pings every ~5 min), the connection is taken as dead.
const SILENCE: Duration = Duration::from_secs(6 * 60);
/// Twitch's longest message, in `char`s.
pub const MAX_CHARS: usize = 500;
/// Messages allowed per [`RATE_WINDOW`] (Twitch's limit for a viewer is 20 per 30 s).
pub const RATE_COUNT: usize = 20;
/// The window of [`RATE_COUNT`].
pub const RATE_WINDOW: Duration = Duration::from_secs(30);
/// Messages waiting to leave.
const QUEUE: usize = 8;

/// What the writer tells its page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriterUpdate {
  /// The connection's state changed.
  Status(WriterStatus),
  /// Twitch refused a message, or told something about the room (`msg-id`: `msg_slowmode`,
  /// `msg_followersonly`, `msg_ratelimit`…).
  Notice(String),
  /// A message was not sent: [`RATE_COUNT`] already left in the last [`RATE_WINDOW`].
  RateLimited,
}

/// The writer's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriterStatus {
  /// Connecting, signing in, or joining the channel.
  Connecting,
  /// Signed in and joined: messages can be sent.
  Ready,
  /// Lost: the next try is in this many seconds.
  Retrying(u64),
  /// Twitch refused the auth-token: it does not try again.
  Refused,
}

/// Sends messages to the writer's channel; dropping it (and the updates' receiver) ends the
/// connection.
#[derive(Clone, Debug)]
pub struct Writer {
  tx: mpsc::Sender<String>,
}

impl Writer {
  /// Queues `text` for the channel; `false` when it is empty once cleaned, or the queue is full.
  pub fn send(&self, text: &str) -> bool {
    let Some(text) = clean(text) else {
      return false;
    };
    self.tx.try_send(text).is_ok()
  }
}

/// `text` as it may be sent: line breaks as spaces, trimmed, at most [`MAX_CHARS`]; `None` when
/// nothing is left.
pub fn clean(text: &str) -> Option<String> {
  let flat: String = text
    .chars()
    .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
    .filter(|c| *c != '\0')
    .collect();
  let trimmed = flat.trim();
  if trimmed.is_empty() {
    return None;
  }
  Some(trimmed.chars().take(MAX_CHARS).collect())
}

/// The IRC line that sends `text` (already [`clean`]) to `channel`: `/me …` as an action.
pub fn outgoing(channel: &str, text: &str) -> String {
  match text.strip_prefix("/me ") {
    Some(action) => format!("PRIVMSG #{channel} :\u{1}ACTION {action}\u{1}"),
    None => format!("PRIVMSG #{channel} :{text}"),
  }
}

/// Starts a writer to `channel` (a login) as `login` with `token` (the web auth-token). Run the
/// future on a tokio runtime; it ends when both the [`Writer`] and the receiver are dropped, or
/// when Twitch refuses the token.
pub fn connect_writer(
  channel: &str,
  login: &str,
  token: &str,
) -> (
  Writer,
  mpsc::Receiver<WriterUpdate>,
  impl Future<Output = ()> + Send + 'static,
) {
  let (tx, rx) = mpsc::channel(QUEUE);
  let (updates, updates_rx) = mpsc::channel(QUEUE);
  let channel = channel.to_ascii_lowercase();
  let login = login.to_ascii_lowercase();
  let token = token.to_owned();
  (
    Writer { tx },
    updates_rx,
    run(channel, login, token, rx, updates),
  )
}

async fn run(
  channel: String,
  login: String,
  token: String,
  mut outgoing: mpsc::Receiver<String>,
  updates: mpsc::Sender<WriterUpdate>,
) {
  let mut delay = Duration::from_secs(1);
  let mut sent = VecDeque::new();
  loop {
    if updates
      .send(WriterUpdate::Status(WriterStatus::Connecting))
      .await
      .is_err()
    {
      return;
    }
    let mut joined = false;
    let session = Session {
      channel: &channel,
      login: &login,
      token: &token,
      outgoing: &mut outgoing,
      updates: &updates,
      sent: &mut sent,
    };
    match session.run(&mut joined).await {
      Ok(Ended::Closed) => return,
      Ok(Ended::Refused) => {
        updates
          .send(WriterUpdate::Status(WriterStatus::Refused))
          .await
          .ok();
        return;
      }
      Ok(Ended::Reconnect) => {
        delay = Duration::from_secs(1);
        continue;
      }
      Err(e) => tracing::info!("Twitch chat writer to {channel}: {e}"),
    }
    if joined {
      delay = Duration::from_secs(1);
    }
    if updates
      .send(WriterUpdate::Status(WriterStatus::Retrying(
        delay.as_secs(),
      )))
      .await
      .is_err()
    {
      return;
    }
    tokio::select! {
      () = tokio::time::sleep(delay) => {}
      () = updates.closed() => return,
    }
    // What was typed while disconnected is not sent late.
    while outgoing.try_recv().is_ok() {}
    delay = (delay * 2).min(MAX_DELAY);
  }
}

enum Ended {
  Closed,
  Reconnect,
  Refused,
}

struct Session<'a> {
  channel: &'a str,
  login: &'a str,
  token: &'a str,
  outgoing: &'a mut mpsc::Receiver<String>,
  updates: &'a mpsc::Sender<WriterUpdate>,
  /// When the last messages left, for the rate limit.
  sent: &'a mut VecDeque<Instant>,
}

impl Session<'_> {
  async fn run(self, joined: &mut bool) -> Result<Ended, String> {
    let Session {
      channel,
      login,
      token,
      outgoing,
      updates,
      sent,
    } = self;
    let (mut socket, _) = tokio_tungstenite::connect_async(URL)
      .await
      .map_err(|e| format!("cannot connect: {e}"))?;
    // The token is in `PASS`: errors here never show the line, only that sending failed.
    for line in [
      "CAP REQ :twitch.tv/tags twitch.tv/commands".to_owned(),
      format!("PASS oauth:{token}"),
      format!("NICK {login}"),
      format!("JOIN #{channel}"),
    ] {
      socket
        .send(Message::text(line))
        .await
        .map_err(|_| "cannot sign in".to_owned())?;
    }
    loop {
      let frame = tokio::select! {
        frame = tokio::time::timeout(SILENCE, socket.next()) => frame,
        text = outgoing.recv() => {
          let Some(text) = text else {
            return Ok(Ended::Closed);
          };
          if !*joined {
            continue;
          }
          let now = Instant::now();
          while sent.front().is_some_and(|at| now.duration_since(*at) >= RATE_WINDOW) {
            sent.pop_front();
          }
          if sent.len() >= RATE_COUNT {
            if updates.send(WriterUpdate::RateLimited).await.is_err() {
              return Ok(Ended::Closed);
            }
            continue;
          }
          sent.push_back(now);
          socket
            .send(Message::text(self::outgoing(channel, &text)))
            .await
            .map_err(|e| format!("cannot send: {e}"))?;
          continue;
        }
        () = updates.closed() => return Ok(Ended::Closed),
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
        let update = match reply(line) {
          Reply::Ping(server) => {
            socket
              .send(Message::text(format!("PONG :{server}")))
              .await
              .map_err(|e| format!("cannot send: {e}"))?;
            continue;
          }
          Reply::Reconnect => return Ok(Ended::Reconnect),
          Reply::Refused => return Ok(Ended::Refused),
          Reply::Joined => {
            *joined = true;
            WriterUpdate::Status(WriterStatus::Ready)
          }
          Reply::Notice(id) => WriterUpdate::Notice(id),
          Reply::Other => continue,
        };
        if updates.send(update).await.is_err() {
          return Ok(Ended::Closed);
        }
      }
    }
  }
}

/// What a line read by the writer means to it.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
  Ping(String),
  Reconnect,
  /// Twitch refused the sign-in (`NOTICE * :Login authentication failed`, or a malformed token).
  Refused,
  /// Its own `JOIN` of the channel.
  Joined,
  /// A `NOTICE` with its `msg-id`.
  Notice(String),
  /// Anything else (the room's messages, which the reader shows).
  Other,
}

fn reply(line: &str) -> Reply {
  let Some(parsed) = irc::parse_line(line) else {
    return Reply::Other;
  };
  match parsed.command.as_str() {
    "PING" => Reply::Ping(parsed.trailing().unwrap_or_default().to_owned()),
    "RECONNECT" => Reply::Reconnect,
    "JOIN" => Reply::Joined,
    "NOTICE" => match parsed.tag("msg-id") {
      Some(id) => Reply::Notice(id.to_owned()),
      // Sign-in failures come before the tags capability is on, without a msg-id.
      None => {
        let text = parsed.trailing().unwrap_or_default();
        if text.contains("authentication failed") || text.contains("Improperly formatted auth") {
          Reply::Refused
        } else {
          Reply::Other
        }
      }
    },
    _ => Reply::Other,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn texts_are_cleaned_before_they_leave() {
    assert_eq!(
      clean("  salut\r\nPRIVMSG #x :spam\n").as_deref(),
      Some("salut  PRIVMSG #x :spam")
    );
    assert_eq!(clean(" \n "), None);
    assert_eq!(clean("a\0b").as_deref(), Some("ab"));
    let long = "é".repeat(MAX_CHARS + 10);
    assert_eq!(clean(&long).map(|t| t.chars().count()), Some(MAX_CHARS));
  }

  #[test]
  fn lines_sent() {
    assert_eq!(outgoing("chan", "hello"), "PRIVMSG #chan :hello");
    assert_eq!(
      outgoing("chan", "/me waves"),
      "PRIVMSG #chan :\u{1}ACTION waves\u{1}"
    );
    assert_eq!(outgoing("chan", "/me"), "PRIVMSG #chan :/me");
  }

  #[test]
  fn replies_read() {
    assert_eq!(
      reply(":tmi.twitch.tv NOTICE * :Login authentication failed"),
      Reply::Refused
    );
    assert_eq!(
      reply(":tmi.twitch.tv NOTICE * :Improperly formatted auth"),
      Reply::Refused
    );
    assert_eq!(
      reply("@msg-id=msg_slowmode :tmi.twitch.tv NOTICE #chan :This room is in slow mode."),
      Reply::Notice("msg_slowmode".into())
    );
    assert_eq!(
      reply(":viewer1!viewer1@viewer1.tmi.twitch.tv JOIN #chan"),
      Reply::Joined
    );
    assert_eq!(
      reply("PING :tmi.twitch.tv"),
      Reply::Ping("tmi.twitch.tv".into())
    );
    assert_eq!(
      reply("@id=1 :a!a@a.tmi.twitch.tv PRIVMSG #chan :hi"),
      Reply::Other
    );
  }
}
