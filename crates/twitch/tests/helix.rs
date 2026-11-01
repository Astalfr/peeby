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
