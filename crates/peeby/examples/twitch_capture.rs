//! Saves Helix answers as test fixtures (`crates/twitch/tests/fixtures/helix_*.json`), with the
//! signed-in user's session from the Credential Manager. Each list is trimmed to a few items, and
//! everything about the account is replaced: who the user is, which channels they follow (names,
//! ids, titles, previews), when they followed them, how many. Public lists (top categories, a
//! category's streams, searches) are only trimmed.
//!
//! `cargo run -p peeby --example twitch_capture` · `-- badges` saves only the chat badges (global, and a
//! public channel's), trimmed to a few sets and versions.
//!
//! The saved refresh token is spent and its successor saved back, as the app does: do not run it
//! while Peeby runs, or the app's session ends at its next refresh. Prints counts only.

// A command-line tool reports on the console.
#![allow(clippy::print_stdout)]

use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use twitch::auth::{Auth, Session, Store};
use twitch::helix::Helix;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../twitch/tests/fixtures");

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let entry = keyring::Entry::new("peeby", "twitch|refresh")?;
  let refresh = entry.get_password()?;
  let store: Store = Arc::new(move |token: &str| {
    keyring::Entry::new("peeby", "twitch|refresh")
      .and_then(|e| e.set_password(token))
      .map_err(|e| e.to_string())
  });
  twitch::block_on(async move {
    let session = Session::restore(Auth::new(None)?, &refresh, store).await?;
    let helix = Helix::new(session)?;
    if std::env::args().any(|a| a == "badges") {
      capture_badges(&helix).await
    } else {
      capture(&helix).await
    }
  })??;
  Ok(())
}

async fn capture(helix: &Helix) -> Result<(), Box<dyn std::error::Error>> {
  let me = helix.get("users", &[]).await?;
  let user_id = me["data"][0]["id"].as_str().unwrap_or_default().to_owned();
  save("helix_users_me.json", &redact_me(me))?;

  let followed = helix
    .get("streams/followed", &[("user_id", &user_id), ("first", "4")])
    .await?;
  save(
    "helix_streams_followed.json",
    &redact_followed_streams(followed),
  )?;

  let channels = helix
    .get(
      "channels/followed",
      &[("user_id", &user_id), ("first", "4")],
    )
    .await?;
  save(
    "helix_channels_followed.json",
    &redact_followed_channels(channels),
  )?;

  let top = helix.get("games/top", &[("first", "4")]).await?;
  let game = top["data"][0]["id"].as_str().unwrap_or_default().to_owned();
  save("helix_games_top.json", &top)?;

  let streams = helix
    .get("streams", &[("game_id", &game), ("first", "3")])
    .await?;
  let broadcaster = streams["data"][0]["user_id"]
    .as_str()
    .unwrap_or_default()
    .to_owned();
  save("helix_streams.json", &streams)?;

  let users = helix.get("users", &[("id", &broadcaster)]).await?;
  save("helix_users.json", &users)?;

  let channels = helix
    .get("search/channels", &[("query", "squeezie"), ("first", "3")])
    .await?;
  save("helix_search_channels.json", &channels)?;

  let categories = helix
    .get(
      "search/categories",
      &[("query", "valorant"), ("first", "3")],
    )
    .await?;
  save("helix_search_categories.json", &categories)?;

  // A public channel's VODs and clips (a channel known to keep its VODs public).
  let channel = helix.get("users", &[("login", "ow_esports")]).await?;
  let channel_id = channel["data"][0]["id"]
    .as_str()
    .unwrap_or_default()
    .to_owned();
  let videos = helix
    .get(
      "videos",
      &[
        ("user_id", &channel_id),
        ("type", "archive"),
        ("first", "3"),
      ],
    )
    .await?;
  save("helix_videos.json", &videos)?;
  let clips = helix
    .get("clips", &[("broadcaster_id", &channel_id), ("first", "3")])
    .await?;
  save("helix_clips.json", &clips)?;
  Ok(())
}

/// The chat badges: the global sets most chats show, and a busy public channel's (its
/// subscriber and bits versions, a few of each).
async fn capture_badges(helix: &Helix) -> Result<(), Box<dyn std::error::Error>> {
  const GLOBAL: &[&str] = &[
    "broadcaster",
    "moderator",
    "vip",
    "partner",
    "premium",
    "turbo",
  ];
  let mut global = helix.get("chat/badges/global", &[]).await?;
  trim_badges(&mut global, |set| GLOBAL.contains(&set));
  save("helix_badges_global.json", &global)?;
  // A busy public channel with its own subscriber and bits badges.
  let user = helix.get("users", &[("login", "hasanabi")]).await?;
  let id = user["data"][0]["id"]
    .as_str()
    .unwrap_or_default()
    .to_owned();
  let mut channel = helix.get("chat/badges", &[("broadcaster_id", &id)]).await?;
  trim_badges(&mut channel, |_| true);
  save("helix_badges_channel.json", &channel)?;
  Ok(())
}

/// Keeps the sets `keep` accepts, with at most 3 versions each.
fn trim_badges(answer: &mut Value, keep: impl Fn(&str) -> bool) {
  if let Some(sets) = answer["data"].as_array_mut() {
    sets.retain(|set| keep(set["set_id"].as_str().unwrap_or_default()));
    for set in sets {
      if let Some(versions) = set["versions"].as_array_mut() {
        versions.truncate(3);
      }
    }
  }
}

/// The signed-in user, replaced by a made-up one (same shape).
fn redact_me(mut answer: Value) -> Value {
  if let Some(user) = answer["data"].get_mut(0) {
    user["id"] = json!("100000001");
    user["login"] = json!("peeby_test");
    user["display_name"] = json!("peeby_test");
    user["description"] = json!("");
    user["profile_image_url"] = json!(
      "https://static-cdn.jtvnw.net/user-default-pictures-uv/example-profile_image-300x300.png"
    );
    user["offline_image_url"] = json!("");
    user["created_at"] = json!("2020-01-01T00:00:00Z");
    user["view_count"] = json!(0);
  }
  answer
}

/// Followed live channels: who they are is the user's data, and so is anything that would find
/// them again (category, viewers, start time, language): all made up, the shape kept.
fn redact_followed_streams(mut answer: Value) -> Value {
  if let Some(list) = answer["data"].as_array_mut() {
    for (i, stream) in list.iter_mut().enumerate() {
      let n = i + 1;
      stream["id"] = json!(format!("30000000{n}"));
      stream["user_id"] = json!(format!("20000000{n}"));
      stream["user_login"] = json!(format!("followed_{n}"));
      stream["user_name"] = json!(format!("Followed{n}"));
      stream["title"] = json!(format!("Live title {n}"));
      stream["thumbnail_url"] = json!(format!(
        "https://static-cdn.jtvnw.net/previews-ttv/live_user_followed_{n}-{{width}}x{{height}}.jpg"
      ));
      stream["tags"] = json!(["Français"]);
      stream["game_id"] = json!("509658");
      stream["game_name"] = json!("Just Chatting");
      stream["viewer_count"] = json!(1000 * n);
      stream["started_at"] = json!(format!("2026-11-01T1{n}:00:00Z"));
      stream["language"] = json!("fr");
    }
  }
  redact_cursor(&mut answer);
  answer
}

fn redact_followed_channels(mut answer: Value) -> Value {
  if let Some(list) = answer["data"].as_array_mut() {
    for (i, channel) in list.iter_mut().enumerate() {
      let n = i + 1;
      channel["broadcaster_id"] = json!(format!("20000000{n}"));
      channel["broadcaster_login"] = json!(format!("followed_{n}"));
      channel["broadcaster_name"] = json!(format!("Followed{n}"));
      channel["followed_at"] = json!(format!("2024-01-0{n}T00:00:00Z"));
    }
  }
  if answer.get("total").is_some() {
    answer["total"] = json!(120);
  }
  redact_cursor(&mut answer);
  answer
}

fn redact_cursor(answer: &mut Value) {
  if answer["pagination"]["cursor"].is_string() {
    answer["pagination"]["cursor"] = json!("REDACTED_CURSOR");
  }
}

fn save(name: &str, answer: &Value) -> Result<(), Box<dyn std::error::Error>> {
  let path = Path::new(FIXTURES).join(name);
  std::fs::write(&path, serde_json::to_string_pretty(answer)? + "\n")?;
  let count = answer["data"].as_array().map_or(0, Vec::len);
  println!("{name}: {count} items");
  Ok(())
}
