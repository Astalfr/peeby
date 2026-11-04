//! Saves Helix answers as test fixtures (`crates/twitch/tests/fixtures/helix_*.json`), with the
//! signed-in user's session from the Credential Manager. Each list is trimmed to a few items, and
//! everything about the account is replaced: who the user is, which channels they follow (names,
//! ids, titles, previews), when they followed them, how many. Public lists (top categories, a
//! category's streams, searches) are only trimmed.
//!
//! `cargo run -p peeby --example twitch_capture` · `-- badges` saves only the chat badges (global, and a
//! public channel's), trimmed to a few sets and versions. `-- master <login>` prints the variants (name,
//! codec, height) of a live's master asked for with every codec (AV1, HEVC, H.264) and the saved
//! `auth-token`, and saves it redacted as `usher_master_codecs.m3u8` when it has HEVC or AV1.
//! `-- master-scan` lists which of the top 100 lives offer HEVC or AV1 (logins and codecs only).
//! `-- positions <vod-id>…` prints how many of those VODs have a resume position on the account
//! (saved `auth-token`) and saves the answer as `gql_vod_positions.json`; it reads, never writes.
//! `-- chat-login [login]` signs in to a channel's chat (the user's own by default) with the saved
//! `auth-token` and prints whether Twitch accepted it (ADR 0023); it never sends a message.
//! `-- catalog` saves the GQL catalog's answers (`gql_catalog_*.json`, [`twitch::catalog`]),
//! anonymous: no secret is read, the lists are trimmed to a few items.
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
  let args: Vec<String> = std::env::args().collect();
  if let Some(i) = args.iter().position(|a| a == "positions") {
    let ids = args[i + 1..].to_vec();
    return twitch::block_on(capture_positions(ids))?;
  }
  if let Some(i) = args.iter().position(|a| a == "master") {
    let login = args.get(i + 1).ok_or("master <login>")?.clone();
    return twitch::block_on(capture_master(login))?;
  }
  if let Some(i) = args.iter().position(|a| a == "chat-login") {
    let channel = args.get(i + 1).cloned();
    return twitch::block_on(check_chat_login(channel))?;
  }
  if args.iter().any(|a| a == "catalog") {
    return twitch::block_on(capture_catalog())?;
  }
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
    if std::env::args().any(|a| a == "master-scan") {
      scan_masters(&helix).await
    } else if std::env::args().any(|a| a == "badges") {
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

/// A live's master with every codec, as the user's account (auth-token): variants printed, the
/// master saved redacted when it offers HEVC or AV1. Its URLs and session keys never print.
async fn capture_master(login: String) -> Result<(), Box<dyn std::error::Error>> {
  use twitch::Target;
  use twitch::usher::Codec;
  let auth_token = keyring::Entry::new("peeby", "twitch|auth-token")
    .and_then(|e| e.get_password())
    .ok();
  println!(
    "auth-token: {}",
    if auth_token.is_some() { "yes" } else { "no" }
  );
  let client = twitch::Client::new(None, auth_token)?;
  let target = Target::Live(login);
  let token = client
    .playback_token(&target, twitch::gql::PLAYER_TYPE)
    .await?;
  let master = client
    .master(&target, &token, &[Codec::Av1, Codec::H265, Codec::H264])
    .await?;
  let mut other = false;
  for v in &master.variants {
    println!(
      "{:<20} {:<8} {:?} {:?} portrait={}",
      v.id, v.name, v.codec, v.height, v.portrait
    );
    other |= !v.portrait && matches!(v.codec, Some(Codec::Av1 | Codec::H265));
  }
  if other {
    let path = Path::new(FIXTURES).join("usher_master_codecs.m3u8");
    std::fs::write(&path, redact_master(&master.text))?;
    println!("saved {}", path.display());
  }
  Ok(())
}

/// Signs in to `channel`'s chat (the account's own when `None`) and prints whether Twitch accepts
/// the auth-token. Nothing is sent to the room.
async fn check_chat_login(channel: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
  let auth_token = keyring::Entry::new("peeby", "twitch|auth-token")
    .and_then(|e| e.get_password())
    .ok();
  let client = twitch::Client::new(None, auth_token)?;
  let status = client.check_chat_sign_in(channel.as_deref()).await?;
  println!("chat sign-in: {status:?}");
  Ok(())
}

/// The account's resume positions in VODs `ids` (auth-token), saved as they come: the VODs are
/// public, the answer holds no user id.
async fn capture_positions(ids: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
  let auth_token = keyring::Entry::new("peeby", "twitch|auth-token")
    .and_then(|e| e.get_password())
    .ok();
  println!(
    "auth-token: {}",
    if auth_token.is_some() { "yes" } else { "no" }
  );
  let client = twitch::Client::new(None, auth_token)?;
  let answer = client.gql(&twitch::gql::positions_request(&ids)).await?;
  let positions = twitch::gql::parse_positions(&answer)?;
  println!("{} VODs, {} with a position", ids.len(), positions.len());
  let path = Path::new(FIXTURES).join("gql_vod_positions.json");
  std::fs::write(&path, serde_json::to_string(&answer)?)?;
  println!("saved {}", path.display());
  Ok(())
}

/// The top 100 lives whose master offers HEVC or AV1 in landscape, with the user's auth-token.
async fn scan_masters(helix: &Helix) -> Result<(), Box<dyn std::error::Error>> {
  use twitch::Target;
  use twitch::usher::Codec;
  let auth_token = keyring::Entry::new("peeby", "twitch|auth-token")
    .and_then(|e| e.get_password())
    .ok();
  let client = twitch::Client::new(None, auth_token)?;
  let streams = helix.get("streams", &[("first", "100")]).await?;
  let logins: Vec<String> = streams["data"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|s| s["user_login"].as_str().map(str::to_owned))
    .collect();
  println!("{} lives", logins.len());
  for login in logins {
    let target = Target::Live(login.clone());
    let Ok(token) = client
      .playback_token(&target, twitch::gql::PLAYER_TYPE)
      .await
    else {
      continue;
    };
    let Ok(master) = client
      .master(&target, &token, &[Codec::Av1, Codec::H265, Codec::H264])
      .await
    else {
      continue;
    };
    let found: Vec<String> = master
      .variants
      .iter()
      .filter(|v| !v.portrait && matches!(v.codec, Some(Codec::Av1 | Codec::H265)))
      .map(|v| format!("{} {:?}", v.id, v.codec))
      .collect();
    if !found.is_empty() {
      println!("{login}: {}", found.join(", "));
    }
  }
  Ok(())
}

/// Session keys kept as they are; every other `SESSION-DATA` value is redacted.
const MASTER_KEPT: &[&str] = &[
  "SUPPRESS",
  "SERVER-TIME",
  "TRANSCODESTACK",
  "TRANSCODEMODE",
  "ABS",
  "STREAM-TIME",
  "FUTURE",
  "B",
  "D",
  "CHANNEL-METADATA",
];

/// A master with its session values redacted (but [`MASTER_KEPT`]) and its variant URLs
/// replaced by placeholders.
fn redact_master(text: &str) -> String {
  let mut out = String::new();
  let mut variant = 0;
  for line in text.lines() {
    if let Some(rest) = line.strip_prefix("#EXT-X-SESSION-DATA:") {
      let id = rest
        .split_once("DATA-ID=\"")
        .and_then(|(_, r)| r.split_once('"'))
        .map_or("", |(id, _)| id);
      if MASTER_KEPT.contains(&id) {
        out.push_str(line);
      } else {
        out.push_str(&format!(
          "#EXT-X-SESSION-DATA:DATA-ID=\"{id}\",VALUE=\"REDACTED\""
        ));
      }
    } else if line.starts_with("#EXT-X-TWITCH-INFO:") {
      out.push_str("#EXT-X-TWITCH-INFO:REDACTED");
    } else if !line.starts_with('#') && !line.trim().is_empty() {
      variant += 1;
      out.push_str(&format!(
        "https://video-edge.example.invalid/variant/{variant}.m3u8"
      ));
    } else {
      out.push_str(line);
    }
    out.push('\n');
  }
  out
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

/// The GQL catalog's answers, anonymous, trimmed: public channels (`ow_esports`, its VOD with
/// muted parts and a clip, as the Helix fixtures), Just Chatting, the searches of `smoke`.
async fn capture_catalog() -> Result<(), Box<dyn std::error::Error>> {
  use twitch::catalog;
  const CHANNEL: &str = "137512364";
  let client = twitch::Client::new(None, None)?;
  let answers = [
    ("top_categories", catalog::top_categories_request()),
    (
      "category_streams",
      catalog::category_streams_request("509658"),
    ),
    (
      "users",
      catalog::users_request(&[CHANNEL.to_owned(), "52130765".to_owned()]),
    ),
    ("user", catalog::user_request("ow_esports")),
    (
      "user_not_found",
      catalog::user_request("peeby_no_such_user_0"),
    ),
    ("videos", catalog::videos_request(CHANNEL, 4)),
    ("video", catalog::video_request("2915878235")),
    ("clips", catalog::clips_request(CHANNEL, 3)),
    ("clip", catalog::clip_request("OpenRenownedNewtPMSTwin")),
    (
      "search_channels",
      catalog::search_channels_request("squeezie"),
    ),
    (
      "search_categories",
      catalog::search_categories_request("minecraft"),
    ),
    ("badges_global", catalog::global_badges_request()),
    (
      "badges_channel",
      catalog::channel_badges_request("207813352"),
    ),
  ];
  for (name, request) in answers {
    let mut answer = client.gql(&request).await?;
    trim(&mut answer["data"]);
    if let Some(extensions) = answer.as_object_mut() {
      extensions.remove("extensions");
    }
    let path = Path::new(FIXTURES).join(format!("gql_catalog_{name}.json"));
    std::fs::write(
      &path,
      serde_json::to_string_pretty(&answer)?
        + "
",
    )?;
    let errors = answer["errors"].as_array().map_or(0, Vec::len);
    println!("gql_catalog_{name}.json: {errors} errors");
  }
  Ok(())
}

/// Every list cut to 3 items, but the global badges: the sets of [`capture_badges`], 3 versions
/// each.
fn trim(value: &mut Value) {
  const GLOBAL: &[&str] = &[
    "broadcaster",
    "moderator",
    "vip",
    "partner",
    "subscriber",
    "turbo",
  ];
  match value {
    Value::Object(map) => {
      if let Some(Value::Array(badges)) = map.get_mut("badges") {
        let mut kept = std::collections::HashMap::<String, usize>::new();
        badges.retain(|b| {
          let set = b["setID"].as_str().unwrap_or_default();
          let n = kept.entry(set.to_owned()).or_default();
          *n += 1;
          GLOBAL.contains(&set) && *n <= 3
        });
        return;
      }
      map.values_mut().for_each(trim);
    }
    Value::Array(list) => {
      list.truncate(3);
      list.iter_mut().for_each(trim);
    }
    _ => {}
  }
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
