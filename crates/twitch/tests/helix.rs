//! Helix parsers against real answers (tests/fixtures/helix_*.json, captured 2026-11-01 with the
//! user's session by `cargo run -p peeby --example twitch_capture`: trimmed, and everything about
//! the account replaced).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use twitch::helix;

fn json(name: &str) -> Value {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn me() {
  let users = helix::parse_users(&json("helix_users_me.json"));
  assert_eq!(users.len(), 1);
  assert_eq!(users[0].login, "peeby_test");
  assert!(users[0].profile_image_url.starts_with("https://"));
}

#[test]
fn a_broadcaster() {
  let users = helix::parse_users(&json("helix_users.json"));
  assert_eq!(users.len(), 1);
  assert!(!users[0].login.is_empty());
  assert!(!users[0].display_name.is_empty());
  assert!(users[0].profile_image_url.contains("profile_image"));
}

#[test]
fn followed_streams() {
  let page = helix::parse_streams(&json("helix_streams_followed.json"));
  assert_eq!(page.items.len(), 4);
  let first = &page.items[0];
  assert_eq!(first.user_login, "followed_1");
  assert_eq!(first.game_name, "Just Chatting");
  assert_eq!(first.viewer_count, 1000);
  assert!(first.started_at.is_some());
  assert!(first.thumbnail_url.contains("{width}x{height}"));
  assert_eq!(first.tags, ["Français"]);
  assert_eq!(page.cursor.as_deref(), Some("REDACTED_CURSOR"));
}

#[test]
fn followed_channels() {
  let page = helix::parse_followed(&json("helix_channels_followed.json"));
  let logins: Vec<&str> = page
    .items
    .iter()
    .map(|f| f.broadcaster_login.as_str())
    .collect();
  assert_eq!(
    logins,
    ["followed_1", "followed_2", "followed_3", "followed_4"]
  );
  assert!(page.cursor.is_some());
}

#[test]
fn top_categories() {
  let page = helix::parse_categories(&json("helix_games_top.json"));
  assert_eq!(page.items.len(), 4);
  assert!(page.items.iter().all(|c| !c.name.is_empty()));
  assert!(page.items[0].box_art_url.contains("{width}x{height}"));
  assert!(page.cursor.is_some());
}

#[test]
fn category_streams() {
  let page = helix::parse_streams(&json("helix_streams.json"));
  assert_eq!(page.items.len(), 3);
  // Most viewers first.
  assert!(page.items[0].viewer_count >= page.items[1].viewer_count);
  assert!(page.items.iter().all(|s| !s.user_login.is_empty()));
}

#[test]
fn search() {
  let channels = helix::parse_search_channels(&json("helix_search_channels.json"));
  assert_eq!(channels.items.len(), 3);
  assert!(channels.items.iter().any(|c| c.login.contains("squeezie")));
  let categories = helix::parse_categories(&json("helix_search_categories.json"));
  assert!(
    categories
      .items
      .iter()
      .any(|c| c.name.to_lowercase().contains("valorant"))
  );
}

#[test]
fn malformed_items_are_skipped() {
  let answer =
    json!({ "data": [{ "id": "" }, { "title": "no id" }, { "id": "1", "user_login": "a" }] });
  let page = helix::parse_streams(&answer);
  assert_eq!(page.items.len(), 1);
  assert_eq!(page.cursor, None);
}

#[test]
fn templates() {
  assert_eq!(
    helix::sized("https://x/a-{width}x{height}.jpg", 440, 248),
    "https://x/a-440x248.jpg"
  );
  assert_eq!(
    helix::sized("https://x/a-%{width}x%{height}.jpg", 320, 180),
    "https://x/a-320x180.jpg"
  );
}

#[test]
fn videos() {
  let page = helix::parse_videos(&json("helix_videos.json"));
  assert_eq!(page.items.len(), 3);
  assert!(page.items.iter().all(|v| v.user_login == "ow_esports"));
  assert!(page.items.iter().all(|v| v.duration > 0));
  // The first one is the broadcast still going: its thumbnail is not ready.
  assert!(page.items[0].thumbnail_url.contains("404_processing"));
  assert!(page.items[1].created_at.is_some());
  // `null` without any; otherwise in the order of the VOD.
  assert!(page.items[0].muted_segments.is_empty());
  let muted = &page.items[1].muted_segments;
  assert_eq!(muted.len(), 10);
  assert_eq!(
    muted[1],
    helix::MutedSegment {
      offset: 721,
      duration: 180
    }
  );
}

#[test]
fn clips() {
  let page = helix::parse_clips(&json("helix_clips.json"));
  assert_eq!(page.items.len(), 3);
  assert!(page.items.iter().all(|c| c.duration > 0.0));
  assert!(page.items[0].thumbnail_url.starts_with("https://"));
  assert!(page.items[0].view_count >= page.items[1].view_count);
}

#[test]
fn durations() {
  assert_eq!(helix::parse_duration("3h8m33s"), 3 * 3600 + 8 * 60 + 33);
  assert_eq!(helix::parse_duration("45m2s"), 45 * 60 + 2);
  assert_eq!(helix::parse_duration("59s"), 59);
  assert_eq!(helix::parse_duration("bad"), 0);
}

#[test]
fn global_badges() {
  let badges = helix::parse_badges(&json("helix_badges_global.json"));
  assert_eq!(badges.len(), 6);
  let moderator = badges.iter().find(|b| b.set == "moderator").unwrap();
  assert_eq!(moderator.version, "1");
  assert_eq!(moderator.title, "Moderator");
  assert!(moderator.url_1x.ends_with("/1"));
  assert!(moderator.url_4x.ends_with("/3"));
}

#[test]
fn channel_badges() {
  let badges = helix::parse_badges(&json("helix_badges_channel.json"));
  let versions: Vec<&str> = badges.iter().map(|b| b.version.as_str()).collect();
  assert_eq!(versions, ["0", "102", "108"]);
  assert!(badges.iter().all(|b| b.set == "subscriber"));
  assert_eq!(badges[1].title, "8.5-Year Subscriber");
}
