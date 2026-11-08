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
//! The connection is [`crate::pubsub::listen`]'s, shared with the channel's Hype Train topic.

use serde_json::Value;

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
