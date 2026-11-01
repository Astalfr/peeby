//! Parsers against real InnerTube responses (tests/fixtures/, captured 2026-11-01, fr/FR,
//! public IP and signatures redacted).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use innertube::{Error, hls, hls_manifest_url, parse_feed, parse_watch};
use serde_json::Value;

fn fixture(name: &str) -> Value {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn anonymous_home_is_empty_with_a_message() {
  let feed = parse_feed(&fixture("home_anonymous.json"));
  assert!(!feed.logged_in);
  assert!(feed.items.is_empty());
  assert_eq!(
    feed.message.as_deref(),
    Some("Votre historique YouTube est désactivé")
  );
}

#[test]
fn channel_videos_grid() {
  let feed = parse_feed(&fixture("channel_videos.json"));
  assert_eq!(feed.items.len(), 30);
  assert!(feed.continuation.is_some());
  let first = &feed.items[0];
  assert_eq!(first.id, "PXC_PYeB6F8");
  assert_eq!(
    first.title,
    "Rick Astley - Angels On My Side (Live at The O2, London, April 2026)"
  );
  assert_eq!(first.duration.as_deref(), Some("3:51"));
  // A channel tab has a single views · date row and no channel row.
  assert_eq!(first.channel, None);
  assert_eq!(first.metadata.len(), 2);
  assert!(first.metadata[1].starts_with("il y a"));
  assert!(first.thumbnails.iter().any(|t| t.width >= 720));
  assert!(feed.items.iter().all(|v| v.id.len() == 11));
}

#[test]
fn watch_page() {
  let page = parse_watch("dQw4w9WgXcQ", &fixture("next.json"));
  assert_eq!(
    page.title,
    "Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)"
  );
  assert_eq!(page.channel.as_deref(), Some("Rick Astley"));
  assert_eq!(page.channel_id.as_deref(), Some("UCuAXFkgsw1L7xaCfnd5JJOw"));
  assert!(page.subscribers.is_some());
  assert!(page.views.as_deref().is_some_and(|v| v.ends_with("vues")));
  assert!(page.description.starts_with("The official video"));
  assert!(page.comments_continuation.is_some());
  assert!(page.related_continuation.is_some());

  assert_eq!(page.related.len(), 20);
  let related = &page.related[0];
  // Related videos carry the channel row, then views · date.
  assert_eq!(related.channel.as_deref(), Some("Retro Pulse"));
  assert!(
    related
      .channel_id
      .as_deref()
      .is_some_and(|id| id.starts_with("UC"))
  );
  assert_eq!(related.metadata.len(), 2);
  assert!(!related.avatar.is_empty());
}

#[test]
fn visionos_player_and_master_playlist() {
  let player = fixture("player_visionos.json");
  let url = hls_manifest_url(&player).unwrap();
  assert!(url.starts_with("https://manifest.googlevideo.com/api/manifest/hls_variant/"));

  let path = format!(
    "{}/tests/fixtures/hls_master.m3u8",
    env!("CARGO_MANIFEST_DIR")
  );
  let master = std::fs::read_to_string(path).unwrap();
  let best = hls::best_variant(&master, 1080).unwrap();
  assert_eq!(best.resolution, "1920x1080");
  assert!(best.codecs.starts_with("vp09"));
  let audio = best.audio_url.unwrap();
  assert!(audio.contains("/itag/234/"));
  let small = hls::best_variant(&master, 360).unwrap();
  assert_eq!(small.resolution, "640x360");
}

#[test]
fn login_required_is_unplayable() {
  let err = hls_manifest_url(&fixture("player_login_required.json")).unwrap_err();
  assert!(matches!(err, Error::Unplayable { ref status, .. } if status == "LOGIN_REQUIRED"));
}

#[test]
fn feed_continuation() {
  // Continuations carry `logged_in` too, so the signed-in check applies to every page.
  let feed = parse_feed(&fixture("channel_videos_continuation.json"));
  assert!(!feed.logged_in);
  assert_eq!(feed.items.len(), 30);
  assert!(feed.continuation.is_some());
  assert!(
    feed
      .items
      .iter()
      .all(|v| v.id.len() == 11 && !v.title.is_empty())
  );
}

#[test]
fn subscriptions_feed() {
  // Signed-in response, trimmed to 4 videos, account data removed.
  let feed = parse_feed(&fixture("subscriptions_feed.json"));
  assert!(feed.logged_in);
  assert_eq!(feed.items.len(), 4);
  assert!(feed.continuation.is_some());
  assert!(
    feed
      .items
      .iter()
      .all(|v| v.channel.is_some() && v.channel_id.is_some())
  );
}

#[test]
fn subscribed_channels() {
  let channels = innertube::parse_channels(&fixture("subscriptions_channels.json"));
  assert_eq!(channels.len(), 3);
  let first = &channels[0];
  assert!(first.id.starts_with("UC"));
  assert!(!first.name.is_empty() && first.name == first.name.trim());
  assert!(first.handle.as_deref().is_some_and(|h| h.starts_with('@')));
  assert!(
    first
      .subscribers
      .as_deref()
      .is_some_and(|s| s.contains("abonnés"))
  );
  assert!(first.avatar.iter().all(|a| a.url.starts_with("https://")));
}

#[test]
fn comments_page() {
  let page = innertube::parse_comments(&fixture("comments.json"));
  assert_eq!(page.comments.len(), 20);
  assert!(page.continuation.is_some());
  let first = &page.comments[0];
  assert_eq!(first.author, "@bdr8823");
  assert_eq!(
    first.text,
    "My algebra teacher said that this was a link to the test answers"
  );
  assert_eq!(first.published, "il y a 7 ans");
  assert!(!first.likes.is_empty());
  assert!(first.replies_continuation.is_some());
}
