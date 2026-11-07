//! 7TV's EventAPI (`wss://events.7tv.io/v3`, docs/twitch.md "7TV live updates"), anonymous: the
//! changes to the emote sets a chat shows, pushed while it is open, so that an emote the streamer
//! adds or removes on 7TV shows without asking 7TV again.
//!
//! The connection subscribes to its [`Topic`]s after the server's Hello (op 1), takes the
//! heartbeats (op 2) as the sign it is alive (three missed: dead), and turns the dispatches (op 0)
//! into [`Update`]s. It reconnects after an error with a growing delay (1, 2, 4… 60 s; at least 5
//! minutes when 7TV says it is in maintenance, close 4007) and stops on a close that names a
//! client bug (4001–4004, 4009, 4010: reconnecting would repeat it). It does not resume (op 34):
//! a new connection subscribes again and sends [`Update::Resync`], so that the page asks 7TV for
//! the sets once instead of trusting the replay of what it missed. It stops when its receiver is
//! dropped.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

use super::cosmetics::{self, Cosmetic, Entitlement};
use super::{Emote, emote};

const URL: &str = "wss://events.7tv.io/v3";
/// Longest delay before reconnecting.
const MAX_DELAY: Duration = Duration::from_secs(60);
/// The wait after a maintenance close (4007), as 7TV asks: at least 5 minutes, plus jitter.
const MAINTENANCE: Duration = Duration::from_secs(5 * 60);
/// The heartbeat interval until the Hello gives it (7TV's is 45 s).
const HEARTBEAT: Duration = Duration::from_secs(45);
/// Updates kept for the page before the connection waits for it to catch up.
const QUEUE: usize = 64;

/// What a connection watches.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Topic {
  /// An emote set's changes (`emote_set.update`), by its 7TV id.
  EmoteSet(String),
  /// A 7TV user's changes (`user.update`), by their 7TV id: a channel switching to another set.
  User(String),
  /// The cosmetics of the 7TV users in a chat (`cosmetic.*`, `entitlement.*` in the `channel`
  /// context), by the channel's Twitch id: two subscriptions.
  Channel(String),
}

impl Topic {
  /// The subscribe messages (op 35).
  fn subscriptions(&self) -> Vec<String> {
    let one = |kind: &str, condition: Value| {
      json!({"op": 35, "d": {"type": kind, "condition": condition}}).to_string()
    };
    match self {
      Topic::EmoteSet(id) => vec![one("emote_set.update", json!({"object_id": id}))],
      Topic::User(id) => vec![one("user.update", json!({"object_id": id}))],
      Topic::Channel(id) => ["cosmetic.*", "entitlement.*"]
        .into_iter()
        .map(|kind| {
          one(
            kind,
            json!({"ctx": "channel", "platform": "TWITCH", "id": id}),
          )
        })
        .collect(),
    }
  }
}

/// What the connection sends its page.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
  /// The set with this 7TV id changed.
  Set {
    /// The set's 7TV id.
    id: String,
    /// Its changes, in order.
    changes: Vec<Change>,
  },
  /// What is loaded may be stale: a new connection (what changed while there was none is not
  /// known), a change that could not be read, or a user whose active set may have switched. Ask
  /// 7TV for the sets again.
  Resync,
  /// A paint or a badge's definition, before anyone wears it ([`Topic::Channel`]).
  Cosmetic(Cosmetic),
  /// A user of the chat wears this paint or badge (they announced their presence in it).
  Entitled(Entitlement),
  /// A user no longer wears this paint or badge.
  Unentitled(Entitlement),
}

/// A change to a set's emotes.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
  /// An emote added to the set.
  Added(Emote),
  /// The emote with this 7TV id taken out of the set; `name` is the word it replaced.
  Removed {
    /// 7TV's emote id.
    id: String,
    /// The word it replaced in the set.
    name: String,
  },
  /// The emote with this 7TV id given another word in the set (an alias).
  Renamed {
    /// 7TV's emote id.
    id: String,
    /// The word it replaced.
    from: String,
    /// The word it replaces now.
    to: String,
  },
}

/// Starts watching `topics`. Run the returned future on a tokio runtime; it ends when the receiver
/// is dropped, or after a close that reconnecting would repeat.
pub fn connect(
  topics: Vec<Topic>,
) -> (
  mpsc::Receiver<Update>,
  impl Future<Output = ()> + Send + 'static,
) {
  let (tx, rx) = mpsc::channel(QUEUE);
  (rx, run(topics, tx))
}

async fn run(topics: Vec<Topic>, tx: mpsc::Sender<Update>) {
  let mut delay = Duration::from_secs(1);
  let mut first = true;
  loop {
    let (mut hello, mut alive) = (false, false);
    let ended = session(&topics, &tx, &mut hello, &mut alive, first).await;
    let wait = match ended {
      Ended::Closed => return,
      Ended::Stop(why) => {
        tracing::info!("7TV events: stopped, {why}");
        return;
      }
      Ended::Reconnect => Duration::ZERO,
      Ended::Maintenance => {
        tracing::info!("7TV events: maintenance, waiting 5 minutes");
        MAINTENANCE + Duration::from_secs(rand::random_range(0..60))
      }
      Ended::Retry(why) => {
        tracing::info!("7TV events: {why}");
        delay
      }
    };
    if hello {
      first = false;
    }
    // Only a heartbeat proves the connection worked (the Hello comes first, before anything can
    // fail): a session that ended before one waits the delay, even when 7TV asked to reconnect, so
    // that a server closing right after the Hello is not asked again every second.
    let wait = if alive {
      delay = Duration::from_secs(1);
      wait
    } else {
      let wait = wait.max(delay);
      delay = (delay * 2).min(MAX_DELAY);
      wait
    };
    tokio::select! {
      () = tokio::time::sleep(wait) => {}
      () = tx.closed() => return,
    }
  }
}

#[derive(Debug)]
enum Ended {
  /// The page is gone.
  Closed,
  /// 7TV asked for a new connection (op 4, close 4006): at once.
  Reconnect,
  /// 7TV is in maintenance (close 4007).
  Maintenance,
  /// Lost: try again after the delay.
  Retry(String),
  /// A close that names a client bug: trying again would repeat it.
  Stop(String),
}

async fn session(
  topics: &[Topic],
  tx: &mpsc::Sender<Update>,
  hello: &mut bool,
  alive: &mut bool,
  first: bool,
) -> Ended {
  let mut socket = match tokio_tungstenite::connect_async(URL).await {
    Ok((socket, _)) => socket,
    Err(e) => return Ended::Retry(format!("cannot connect: {e}")),
  };
  let mut heartbeat = HEARTBEAT;
  loop {
    // Three missed heartbeats: the connection is dead.
    let frame = tokio::select! {
      frame = tokio::time::timeout(heartbeat * 3, socket.next()) => frame,
      () = tx.closed() => return Ended::Closed,
    };
    let text = match frame {
      Err(_) => return Ended::Retry("no heartbeat".into()),
      Ok(None) => return Ended::Retry("closed by the server".into()),
      Ok(Some(Err(e))) => return Ended::Retry(format!("read: {e}")),
      Ok(Some(Ok(Message::Text(text)))) => text,
      Ok(Some(Ok(Message::Close(close)))) => {
        return closed(close.map(|c| u16::from(c.code)), "close frame");
      }
      Ok(Some(Ok(_))) => continue,
    };
    let Ok(message) = serde_json::from_str::<Value>(text.as_str()) else {
      continue;
    };
    let data = message.get("d").unwrap_or(&Value::Null);
    match message.get("op").and_then(Value::as_u64) {
      // Hello: subscribe.
      Some(1) => {
        if let Some(ms) = data.get("heartbeat_interval").and_then(Value::as_u64) {
          heartbeat = Duration::from_millis(ms.max(1000));
        }
        let limit = data
          .get("subscription_limit")
          .and_then(Value::as_i64)
          .and_then(|l| usize::try_from(l).ok())
          .unwrap_or(usize::MAX);
        let subscriptions: Vec<String> = topics.iter().flat_map(Topic::subscriptions).collect();
        if subscriptions.len() > limit {
          tracing::info!(
            "7TV events: {} subscriptions, 7TV takes {limit}",
            subscriptions.len()
          );
        }
        for subscription in subscriptions.into_iter().take(limit) {
          if let Err(e) = socket.send(Message::text(subscription)).await {
            return Ended::Retry(format!("cannot send: {e}"));
          }
        }
        *hello = true;
        if !first && tx.send(Update::Resync).await.is_err() {
          return Ended::Closed;
        }
      }
      // Dispatch.
      Some(0) => {
        if let Some(update) = dispatch(data)
          && tx.send(update).await.is_err()
        {
          return Ended::Closed;
        }
      }
      // Reconnect.
      Some(4) => return Ended::Reconnect,
      // Error: 7TV says to log it.
      Some(6) => tracing::info!("7TV events: error {}", data),
      // End of stream: the close code says what to do.
      Some(7) => {
        let code = data
          .get("code")
          .and_then(Value::as_u64)
          .and_then(|c| u16::try_from(c).ok());
        return closed(code, "end of stream");
      }
      // Heartbeat.
      Some(2) => *alive = true,
      // Ack (5).
      _ => {}
    }
  }
}

/// What a close with `code` asks for.
fn closed(code: Option<u16>, how: &str) -> Ended {
  match code {
    Some(4006) => Ended::Reconnect,
    Some(4007) => Ended::Maintenance,
    Some(code @ (4001..=4004 | 4009 | 4010)) => Ended::Stop(format!("{how} {code}")),
    Some(code) if code == u16::from(CloseCode::Normal) => {
      Ended::Retry(format!("{how} {code}, normal"))
    }
    Some(code) => Ended::Retry(format!("{how} {code}")),
    None => Ended::Retry(how.to_owned()),
  }
}

/// A dispatch's update: an emote set's changes, or [`Update::Resync`] for a user's (their active
/// set may have switched) and for a set change that cannot be read; a cosmetic, or who wears one.
/// `None` for what Peeby does not use (a cosmetic's deletion: it stays until nobody wears it).
pub fn dispatch(data: &Value) -> Option<Update> {
  let kind = data.get("type").and_then(Value::as_str)?;
  let body = data.get("body")?;
  match kind {
    "user.update" => Some(Update::Resync),
    "cosmetic.create" | "cosmetic.update" => cosmetics::cosmetic(body).map(Update::Cosmetic),
    "entitlement.create" => cosmetics::entitlement(body).map(Update::Entitled),
    "entitlement.delete" => cosmetics::entitlement(body).map(Update::Unentitled),
    "emote_set.update" => {
      let id = body.get("id").and_then(Value::as_str)?.to_owned();
      let field = |list: &str| -> Vec<&Value> {
        body
          .get(list)
          .and_then(Value::as_array)
          .into_iter()
          .flatten()
          .filter(|f| f.get("key").and_then(Value::as_str) == Some("emotes"))
          .collect()
      };
      let mut changes = Vec::new();
      for f in field("pulled") {
        let Some(old) = f.get("old_value") else {
          return Some(Update::Resync);
        };
        let (Some(id), Some(name)) = (text(old, "id"), text(old, "name")) else {
          return Some(Update::Resync);
        };
        changes.push(Change::Removed { id, name });
      }
      // After the removals: an emote taken out and put back in one change stays.
      for f in field("pushed") {
        let Some(emote) = f.get("value").and_then(emote) else {
          return Some(Update::Resync);
        };
        changes.push(Change::Added(emote));
      }
      for f in field("updated") {
        let (Some(old), Some(new)) = (f.get("old_value"), f.get("value")) else {
          return Some(Update::Resync);
        };
        let (Some(id), Some(from), Some(to)) =
          (text(new, "id"), text(old, "name"), text(new, "name"))
        else {
          return Some(Update::Resync);
        };
        if from != to {
          changes.push(Change::Renamed { id, from, to });
        }
      }
      (!changes.is_empty()).then_some(Update::Set { id, changes })
    }
    _ => None,
  }
}

fn text(value: &Value, key: &str) -> Option<String> {
  value.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn close_codes_say_whether_to_reconnect() {
    assert!(matches!(closed(Some(4006), "close"), Ended::Reconnect));
    assert!(matches!(closed(Some(4007), "close"), Ended::Maintenance));
    for bug in [4001, 4002, 4003, 4004, 4009, 4010] {
      assert!(
        matches!(closed(Some(bug), "close"), Ended::Stop(_)),
        "{bug}"
      );
    }
    for retry in [Some(1000), Some(4000), Some(4005), Some(4008), None] {
      assert!(
        matches!(closed(retry, "close"), Ended::Retry(_)),
        "{retry:?}"
      );
    }
  }

  #[test]
  fn a_topic_subscribes_by_object_id() {
    let messages = Topic::EmoteSet("01SET".into()).subscriptions();
    let message: Value = serde_json::from_str(&messages[0]).expect("JSON");
    assert_eq!(
      message,
      json!({"op": 35, "d": {"type": "emote_set.update", "condition": {"object_id": "01SET"}}})
    );
  }

  #[test]
  fn a_channel_subscribes_to_its_cosmetics_and_entitlements() {
    let messages: Vec<Value> = Topic::Channel("123".into())
      .subscriptions()
      .iter()
      .map(|m| serde_json::from_str(m).expect("JSON"))
      .collect();
    let condition = json!({"ctx": "channel", "platform": "TWITCH", "id": "123"});
    assert_eq!(
      messages,
      [
        json!({"op": 35, "d": {"type": "cosmetic.*", "condition": condition}}),
        json!({"op": 35, "d": {"type": "entitlement.*", "condition": condition}}),
      ]
    );
  }
}
