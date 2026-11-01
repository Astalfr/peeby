//! Parsers against real InnerTube responses (tests/fixtures/, captured 2026-11-01, fr/FR,
//! public IP and signatures redacted).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use innertube::{
  Error, LikeStatus, hls, hls_manifest_url, parse_feed, parse_playlist, parse_playlist_choices,
  parse_playlist_page, parse_playlists, parse_watch,
};
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
  // Signed out: no rating, the endpoints still carry their params (inside the sign-in modal).
  assert_eq!(page.like_status, LikeStatus::Indifferent);
  for status in [
    LikeStatus::Like,
    LikeStatus::Dislike,
    LikeStatus::Indifferent,
  ] {
    assert!(page.like_params.get(status).is_some(), "{status:?}");
  }

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

#[test]
fn comment_replies() {
  // Replies pages list `commentViewModel`s directly, then "Afficher plus de réponses".
  let page = innertube::parse_comments(&fixture("comment_replies.json"));
  assert_eq!(page.comments.len(), 10);
  assert!(page.continuation.is_some());
  assert_eq!(page.count, None);
  assert_eq!(page.comments[0].author, "@daikerio6830");
  assert!(
    page
      .comments
      .iter()
      .all(|c| c.replies_continuation.is_none())
  );
}

#[test]
fn quality_ladder() {
  let path = format!(
    "{}/tests/fixtures/hls_master.m3u8",
    env!("CARGO_MANIFEST_DIR")
  );
  let master = std::fs::read_to_string(path).unwrap();
  let ladder = hls::ladder(&master);
  let heights: Vec<u32> = ladder.iter().map(|v| v.height).collect();
  assert_eq!(heights, [2160, 1440, 1080, 720, 480, 360, 240, 144]);
  assert_eq!(ladder[2].label(), "1080p60");
  assert!(ladder[2].codecs.starts_with("vp09"));
  assert!(ladder.iter().all(|v| v.audio_url.is_some()));
}

#[test]
fn captions() {
  let captions = innertube::parse_captions(&fixture("player_captions.json"));
  assert_eq!(captions.len(), 6);
  assert_eq!(captions[1].language, "en");
  assert_eq!(captions[1].name, "Anglais");
  assert!(captions[2].auto_generated);
  assert!(captions.iter().all(|c| c.url.ends_with("&fmt=vtt")));
}

#[test]
fn playlists_library() {
  let playlists = parse_playlists(&fixture("playlists_library.json"));
  let ids: Vec<&str> = playlists.iter().map(|p| p.id.as_str()).collect();
  assert_eq!(
    ids,
    [
      "WL",
      "LL",
      "PLREDACTEDplaylist01xxxxxxxxxxxxxx",
      "PLREDACTEDplaylist02xxxxxxxxxxxxxx"
    ]
  );
  let watch_later = &playlists[0];
  assert_eq!(watch_later.title, "À regarder plus tard");
  assert_eq!(watch_later.count.as_deref(), Some("17\u{a0}vidéos"));
  // The "Afficher la playlist complète" link row is left out.
  assert_eq!(
    watch_later.metadata,
    ["Privée", "Playlist", "Mise à jour aujourd'hui"]
  );
  assert!(!watch_later.thumbnails.is_empty());
  assert_eq!(
    playlists[1].count.as_deref(),
    Some("5\u{202f}000\u{a0}vidéos")
  );
}

#[test]
fn watch_later_uses_playlist_video_renderers() {
  let playlist = parse_playlist("WL", &fixture("playlist_watch_later.json"));
  assert!(playlist.title.starts_with("À regarder plus tard"));
  assert_eq!(playlist.count.as_deref(), Some("17\u{a0}vidéos"));
  assert_eq!(playlist.videos.len(), 3);
  let first = &playlist.videos[0];
  assert_eq!(first.id, "tcqah8v8qmo");
  assert_eq!(first.title, "Activision is genuinely desperate now..");
  assert_eq!(first.channel.as_deref(), Some("SmugAlana"));
  assert_eq!(first.duration.as_deref(), Some("23:37"));
  // The " • " separator run is dropped.
  assert_eq!(first.metadata, ["163\u{a0}k\u{a0}vues", "il y a 7 jours"]);
  // "Watch later" can be edited: each entry has its own id for removal.
  assert!(playlist.videos.iter().all(|v| v.set_video_id.is_some()));
  assert_eq!(playlist.continuation, None);
}

#[test]
fn public_playlist_header_and_next_videos_token() {
  let playlist = parse_playlist(
    "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI",
    &fixture("playlist_public.json"),
  );
  assert!(!playlist.title.is_empty());
  assert!(playlist.owner.is_some());
  assert!(
    playlist
      .count
      .as_deref()
      .is_some_and(|c| c.chars().any(|ch| ch.is_ascii_digit()))
  );
  let ids: Vec<&str> = playlist.videos.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["fOT0BUpITw8", "NFvDHYMzj9U", "8c7KOGxeY3w"]);
  // Another channel's playlist: nothing to remove.
  assert!(playlist.videos.iter().all(|v| v.set_video_id.is_none()));
  // Two continuation items on the page: the one after the videos is the next videos.
  let token = playlist.continuation.unwrap();
  assert!(token.starts_with("4qmFsgKBARIkVkxQTEZncXVM"), "{token}");
}

#[test]
fn playlist_continuation_is_the_last_page() {
  let page = parse_playlist_page(&fixture("playlist_public_continuation.json"));
  assert!(page.logged_in);
  let ids: Vec<&str> = page.items.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["1STyaZA7-BY", "Ke9A6XV2H38", "_db-mfu8y1k"]);
  assert_eq!(page.continuation, None);
}

#[test]
fn unplayable_playlist_entries_are_skipped() {
  let response: Value = serde_json::from_str(
    r#"{"contents": [
      {"playlistVideoRenderer": {"videoId": "aaaaaaaaaaa", "isPlayable": false,
        "title": {"runs": [{"text": "[Vidéo supprimée]"}]}}},
      {"playlistVideoRenderer": {"videoId": "bbbbbbbbbbb", "isPlayable": true,
        "title": {"runs": [{"text": "Une vidéo"}]}}}
    ]}"#,
  )
  .unwrap();
  let page = parse_playlist_page(&response);
  let ids: Vec<&str> = page.items.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(ids, ["bbbbbbbbbbb"]);
}

#[test]
fn playlist_choices() {
  // Asked for a video that is in "Watch later" only.
  let choices = parse_playlist_choices(&fixture("add_to_playlist.json"));
  assert_eq!(choices.len(), 7);
  assert_eq!(choices[0].id, "WL");
  assert_eq!(choices[0].title, "À regarder plus tard");
  assert!(choices[0].contains && choices[0].private);
  assert_eq!(choices[2].title, "Ma playlist 1");
  assert!(!choices[2].contains && !choices[2].private);
  assert!(choices[1..].iter().all(|c| !c.contains));
}
