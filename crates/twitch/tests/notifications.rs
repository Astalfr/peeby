//! The account's notification center (tests/fixtures/gql_notifications.json, read signed in on
//! 2026-11-09 with `twitch_capture -- notifications`, trimmed to six notifications, the followed
//! channels and their avatars renamed, the account's id made up).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use twitch::Error;
use twitch::notifications::{self, Span};

fn json(name: &str) -> Value {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json")
}

#[test]
fn reads_the_list() {
  let page = notifications::parse(&json("gql_notifications.json")).unwrap();
  assert_eq!(page.unseen, 1);
  assert!(page.last_seen_at.is_some());
  assert_eq!(page.notifications.len(), 6);
  let first = &page.notifications[0];
  assert_eq!(
    first.text(),
    "Your 3-stream streak is maintained with channelone!"
  );
  assert_eq!(first.category, "recommended_content");
  assert!(!first.read);
  assert_eq!(
    first.url.as_deref(),
    Some("https://www.twitch.tv/channelone")
  );
  assert!(
    first
      .thumbnail
      .as_deref()
      .is_some_and(|t| t.contains("profile_image"))
  );
  // Newer than twitch.tv's last look: its one unseen notification.
  assert!(first.created_at > page.last_seen_at);
  assert!(
    page.notifications[1..]
      .iter()
      .all(|n| n.created_at < page.last_seen_at)
  );
  // Newest first.
  assert!(
    page
      .notifications
      .windows(2)
      .all(|w| w[0].created_at >= w[1].created_at)
  );
  // A drop's reward in bold.
  let reward = page
    .notifications
    .iter()
    .find(|n| n.category == "transactional")
    .unwrap();
  assert!(reward.spans.contains(&Span {
    text: " Cyber Glasses".to_owned(),
    bold: true
  }));
}

#[test]
fn signed_out_is_not_logged_in() {
  let answer = json!({ "data": { "currentUser": null } });
  assert!(matches!(
    notifications::parse(&answer),
    Err(Error::NotLoggedIn)
  ));
}

#[test]
fn a_node_without_a_text_is_skipped() {
  let answer = json!({ "data": { "currentUser": { "id": "1", "notifications": {
    "summary": { "unseenCount": 0 },
    "edges": [
      { "node": { "id": "a", "body": "" } },
      { "node": { "id": "b" } },
      { "node": { "body": "no id" } },
      { "node": { "id": "c", "body": "kept", "createdAt": "2026-11-09T19:08:52.084876728Z" } },
    ],
  } } } });
  let page = notifications::parse(&answer).unwrap();
  assert_eq!(page.notifications.len(), 1);
  assert_eq!(page.notifications[0].text(), "kept");
}
