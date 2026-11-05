//! A channel's outgoing raid, from Twitch's PubSub (ADR 0024): the topic `raid.<channel id>`,
//! which takes an anonymous listener (no token), checked on 2026-11-05 on 500 lives for 30 minutes
//! (`cargo run -p twitch --example raid_probe`): 13 raids, each a `raid_update_v2` when announced
//! and again as viewers join it, then `raid_go_v2` (several times) when it leaves. Both carry the
//! target (`target_login`, `target_display_name`, `target_profile_image`), `viewer_count`, and
//! `raid_created_at` (Unix seconds) with `force_raid_now_seconds` (90): it leaves by then at the
//! latest. No cancel was seen; Twitch's web player names it `raid_cancel_v2` **[?]**.
//!
//! GQL's `user(login) { raid }` exists but stayed null for every viewer, anonymous or signed in
//! (2026-11-04, about 2,600 lives scanned 50 times): it is the broadcaster's own.
//!
//! [`listen`] holds one connection for one channel: it answers PubSub's pings, follows its
//! `RECONNECT`, reconnects after an error with a growing delay (1, 2, 4… 30 s), and stops when
//! its receiver is dropped.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const URL: &str = "wss://pubsub-edge.twitch.tv/v1";
/// PubSub drops a client that sends no `PING` for 5 minutes.
const PING_EVERY: Duration = Duration::from_secs(4 * 60);
/// No `PONG` this long after a `PING`: the connection is taken as dead.
const PONG_WITHIN: Duration = Duration::from_secs(15);
/// Longest delay before reconnecting.
const MAX_DELAY: Duration = Duration::from_secs(30);
/// Events kept for the page before the connection waits for it to catch up.
const QUEUE: usize = 64;

/// A raid about to leave a channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Raid {
  /// Id: the same in every message of a raid.
  pub id: String,
  /// The raided channel's login (its live opens when the raid goes).
  pub target_login: String,
  /// The raided channel's shown name.
  pub target_name: String,
  /// The raided channel's avatar, 70 px.
  pub target_avatar: Option<String>,
  /// The viewers it brings, so far.
  pub viewer_count: u64,
  /// When it was announced, milliseconds since the epoch.
  pub created_at: Option<i64>,
  /// It leaves at the latest this long after its announcement.
  pub force_seconds: u64,
}

/// What a raid topic says.
#[derive(Clone, Debug, PartialEq)]
pub enum RaidEvent {
  /// Announced, or its viewers changed.
  Update(Raid),
  /// It leaves now: the viewers who follow it go to the target.
  Go(Raid),
  /// Canceled, by id.
  Cancel(String),
}

/// What a PubSub frame means for a raid topic; `None` for every other frame (responses, pongs,
/// other topics, unknown types).
pub fn parse_message(frame: &str) -> Option<RaidEvent> {
  let frame: Value = serde_json::from_str(frame).ok()?;
  if frame["type"] != "MESSAGE" || !frame["data"]["topic"].as_str()?.starts_with("raid.") {
    return None;
  }
  // The message is JSON in a string.
  let message: Value = serde_json::from_str(frame["data"]["message"].as_str()?).ok()?;
  let raid = &message["raid"];
  let id = raid["id"].as_str()?.to_owned();
  let kind = message["type"].as_str()?;
  if kind.contains("cancel") {
    return Some(RaidEvent::Cancel(id));
  }
  let login = raid["target_login"].as_str()?.to_owned();
  let parsed = Raid {
    id,
    target_name: raid["target_display_name"]
      .as_str()
      .filter(|n| !n.is_empty())
      .unwrap_or(&login)
      .to_owned(),
    target_login: login,
    target_avatar: raid["target_profile_image"].as_str().map(str::to_owned),
    viewer_count: raid["viewer_count"].as_u64().unwrap_or(0),
    created_at: raid["raid_created_at"]
      .as_i64()
      .and_then(|s| s.checked_mul(1000)),
    force_seconds: raid["force_raid_now_seconds"].as_u64().unwrap_or(0),
  };
  match kind {
    "raid_update_v2" | "raid_update" => Some(RaidEvent::Update(parsed)),
    "raid_go_v2" | "raid_go" => Some(RaidEvent::Go(parsed)),
    _ => None,
  }
}

/// The frame asking for `channel_id`'s raid topic.
pub fn listen_frame(channel_id: &str) -> String {
  json!({
    "type": "LISTEN",
    "nonce": "peeby-raid",
    "data": { "topics": [format!("raid.{channel_id}")] },
  })
  .to_string()
}

/// Starts listening to `channel_id`'s raids. Run the returned future on a tokio runtime; it ends
/// when the receiver is dropped.
pub fn listen(
  channel_id: &str,
) -> (
  mpsc::Receiver<RaidEvent>,
  impl Future<Output = ()> + Send + 'static,
) {
  let (tx, rx) = mpsc::channel(QUEUE);
  (rx, run(channel_id.to_owned(), tx))
}

async fn run(channel_id: String, tx: mpsc::Sender<RaidEvent>) {
  let mut delay = Duration::from_secs(1);
  loop {
    match session(&channel_id, &tx).await {
      Ok(Ended::Closed) => return,
      Ok(Ended::Reconnect) => {
        delay = Duration::from_secs(1);
        continue;
      }
      Err(e) => tracing::info!("Twitch raids of {channel_id}: {e}"),
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

async fn session(channel_id: &str, tx: &mpsc::Sender<RaidEvent>) -> Result<Ended, String> {
  let (mut socket, _) = tokio_tungstenite::connect_async(URL)
    .await
    .map_err(|e| format!("cannot connect: {e}"))?;
  let send = |text: String| Message::text(text);
  socket
    .send(send(listen_frame(channel_id)))
    .await
    .map_err(|e| format!("cannot send: {e}"))?;
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
          .send(send(r#"{"type":"PING"}"#.to_owned()))
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
      // An empty `error` is accepted; anything else will not change by retrying.
      let refused = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_owned))
        .filter(|e| !e.is_empty());
      if let Some(error) = refused {
        tracing::warn!("Twitch raids of {channel_id}: refused ({error})");
        tx.closed().await;
        return Ok(Ended::Closed);
      }
    } else if let Some(event) = parse_message(text)
      && tx.send(event).await.is_err()
    {
      return Ok(Ended::Closed);
    }
  }
}
