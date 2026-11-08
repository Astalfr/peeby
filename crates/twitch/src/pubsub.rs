//! One anonymous connection to Twitch's PubSub for the live on screen (ADR 0024, 0032): its raid
//! topic (`raid.<channel id>`, [`crate::raids`]) and its Hype Train topic
//! (`hype-train-events-v1.<channel id>`, [`crate::hype_train`]). No token is ever sent on it.
//!
//! [`listen`] answers PubSub's pings, follows its `RECONNECT`, reconnects after an error with a
//! growing delay (1, 2, 4… 30 s), and stops when its receiver is dropped. Each topic is asked for
//! in its own `LISTEN`: PubSub closes a connection on a large frame (code 1009, seen with 50 Hype
//! Train topics in one).

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::hype_train::{self, HypeEvent};
use crate::raids::{self, RaidEvent};

const URL: &str = "wss://pubsub-edge.twitch.tv/v1";
/// PubSub drops a client that sends no `PING` for 5 minutes.
const PING_EVERY: Duration = Duration::from_secs(4 * 60);
/// No `PONG` this long after a `PING`: the connection is taken as dead.
const PONG_WITHIN: Duration = Duration::from_secs(15);
/// Longest delay before reconnecting.
const MAX_DELAY: Duration = Duration::from_secs(30);
/// Events kept for the page before the connection waits for it to catch up.
const QUEUE: usize = 64;

/// What the channel's topics say.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
  /// The topics are listened to (again): what was said meanwhile was missed.
  Listening,
  /// The raid topic's.
  Raid(RaidEvent),
  /// The Hype Train topic's.
  HypeTrain(HypeEvent),
}

/// The frames asking for `channel_id`'s topics, one `LISTEN` each.
pub fn listen_frames(channel_id: &str) -> Vec<String> {
  [
    ("peeby-raid", format!("raid.{channel_id}")),
    (
      "peeby-hype-train",
      format!("hype-train-events-v1.{channel_id}"),
    ),
  ]
  .into_iter()
  .map(|(nonce, topic)| {
    json!({ "type": "LISTEN", "nonce": nonce, "data": { "topics": [topic] } }).to_string()
  })
  .collect()
}

/// What a frame means; `None` for responses, pongs, other topics and unknown messages.
pub fn parse_message(frame: &str) -> Option<Event> {
  raids::parse_message(frame)
    .map(Event::Raid)
    .or_else(|| hype_train::parse_message(frame).map(Event::HypeTrain))
}

/// Starts listening to `channel_id`'s topics. Run the returned future on a tokio runtime; it ends
/// when the receiver is dropped.
pub fn listen(
  channel_id: &str,
) -> (
  mpsc::Receiver<Event>,
  impl Future<Output = ()> + Send + 'static,
) {
  let (tx, rx) = mpsc::channel(QUEUE);
  (rx, run(channel_id.to_owned(), tx))
}

async fn run(channel_id: String, tx: mpsc::Sender<Event>) {
  let mut delay = Duration::from_secs(1);
  loop {
    match session(&channel_id, &tx).await {
      Ok(Ended::Closed) => return,
      Ok(Ended::Reconnect) => {
        delay = Duration::from_secs(1);
        continue;
      }
      Err(e) => tracing::info!("Twitch PubSub of {channel_id}: {e}"),
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

async fn session(channel_id: &str, tx: &mpsc::Sender<Event>) -> Result<Ended, String> {
  let (mut socket, _) = tokio_tungstenite::connect_async(URL)
    .await
    .map_err(|e| format!("cannot connect: {e}"))?;
  for frame in listen_frames(channel_id) {
    socket
      .send(Message::text(frame))
      .await
      .map_err(|e| format!("cannot send: {e}"))?;
  }
  if tx.send(Event::Listening).await.is_err() {
    return Ok(Ended::Closed);
  }
  let mut ping = tokio::time::interval(PING_EVERY);
  // The first tick is at once: the next ones every PING_EVERY.
  ping.tick().await;
  let mut awaiting_pong: Option<tokio::time::Instant> = None;
  loop {
    let pong_deadline = awaiting_pong.map(|at| at + PONG_WITHIN);
    let frame = tokio::select! {
      frame = socket.next() => frame,
      _ = ping.tick() => {
        socket
          .send(Message::text(r#"{"type":"PING"}"#.to_owned()))
          .await
          .map_err(|e| format!("cannot send: {e}"))?;
        awaiting_pong.get_or_insert_with(tokio::time::Instant::now);
        continue;
      }
      () = async {
        match pong_deadline {
          Some(deadline) => tokio::time::sleep_until(deadline).await,
          None => std::future::pending().await,
        }
      } => return Err("no PONG".into()),
      () = tx.closed() => return Ok(Ended::Closed),
    };
    let text = match frame {
      None => return Err("closed by the server".into()),
      Some(Err(e)) => return Err(format!("read: {e}")),
      Some(Ok(Message::Text(text))) => text,
      Some(Ok(Message::Close(_))) => return Err("closed by the server".into()),
      Some(Ok(_)) => continue,
    };
    let text = text.as_str();
    if text.contains("\"PONG\"") {
      awaiting_pong = None;
    } else if text.contains("\"RECONNECT\"") {
      return Ok(Ended::Reconnect);
    } else if text.contains("\"RESPONSE\"") {
      // An empty `error` is accepted; a refused topic will not change by retrying, and the
      // other one still comes on this connection.
      let refused = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| {
          let error = v["error"].as_str()?.to_owned();
          Some((error, v["nonce"].as_str().unwrap_or_default().to_owned()))
        })
        .filter(|(e, _)| !e.is_empty());
      if let Some((error, nonce)) = refused {
        tracing::warn!("Twitch PubSub of {channel_id}: {nonce} refused ({error})");
      }
    } else if let Some(event) = parse_message(text)
      && tx.send(event).await.is_err()
    {
      return Ok(Ended::Closed);
    }
  }
}
