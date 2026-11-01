//! Parsers against real Twitch responses (tests/fixtures/, captured 2026-11-01, anonymous; tokens,
//! signatures, URLs and session ids redacted).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::Value;
use twitch::usher::{self, Codec, Version};
use twitch::{Error, PlaybackToken, Target, gql};

fn text(name: &str) -> String {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  std::fs::read_to_string(path).unwrap()
}

fn json(name: &str) -> Value {
  serde_json::from_str(&text(name)).unwrap()
}

fn live() -> Target {
  Target::Live("squeezie".into())
}

fn vod() -> Target {
  Target::Vod("2916854926".into())
}

#[test]
fn live_token_is_read() {
  let token = gql::parse_token(&json("gql_playback_token_live.json"), &live()).unwrap();
  assert_eq!(token.signature, "REDACTED");
  assert!(token.value.contains("user_ip"));
  // The fixture's values are redacted, chansub included: nothing restricted can be read.
  assert!(!token.subscriber_only());
}

#[test]
fn subscriber_only_vod_is_known_from_its_token() {
  let token = gql::parse_token(&json("gql_playback_token_vod.json"), &vod()).unwrap();
  assert_eq!(token.restricted.len(), 12);
  assert!(token.restricted.iter().any(|q| q == "chunked"));
  assert!(token.subscriber_only());
}

#[test]
fn unknown_channel_has_no_token() {
  let answer = json("gql_playback_token_not_found.json");
  let target = Target::Live("peeby_no_such_channel_0000".into());
  assert!(matches!(
    gql::parse_token(&answer, &target),
    Err(Error::NotFound)
  ));
}

#[test]
fn refused_auth_token_is_not_logged_in() {
  let body = text("gql_invalid_auth_token.json");
  assert!(matches!(gql::parse_status(401, &body), Error::NotLoggedIn));
}

#[test]
fn integrity_errors_have_their_own_variant() {
  // Shape reported by Streamlink; no real answer saved yet (anonymous tokens never asked).
  let answer: Value =
    serde_json::from_str(r#"{"errors":[{"message":"failed integrity check"}],"data":null}"#)
      .unwrap();
  assert!(matches!(
    gql::parse_token(&answer, &live()),
    Err(Error::IntegrityRequired)
  ));
}

#[test]
fn token_request_asks_for_the_right_kind() {
  let body = gql::token_request(&vod(), gql::PLAYER_TYPE);
  assert_eq!(body["variables"]["isVod"], true);
  assert_eq!(body["variables"]["vodID"], "2916854926");
  assert_eq!(body["variables"]["playerType"], "embed");
  assert!(
    body["query"]
      .as_str()
      .unwrap()
      .contains("videoPlaybackAccessToken")
  );
}

#[test]
fn client_id_is_read_from_the_page() {
  let page = text("twitch_page_client_id.html");
  assert_eq!(
    gql::client_id_from_page(&page),
    Some("kimne78kx3ncx6brgo4mv6wki5h1ko")
  );
  assert_eq!(gql::client_id_from_page("no script here"), None);
}

#[test]
fn v2_master_variants() {
  let variants = usher::variants(&text("usher_master_v2.m3u8"));
  let ids: Vec<&str> = variants.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(
    ids,
    [
      "1080p60",
      "720p60",
      "480p30",
      "360p30",
      "160p30",
      "audio_only",
      "1080p60-portrait",
      "720p60-portrait",
      "480p30-portrait",
      "360p30-portrait",
      "160p30-portrait",
    ]
  );
  let best = &variants[0];
  assert_eq!(best.name, "1080p60");
  assert_eq!(
    (best.height, best.frame_rate, best.codec),
    (Some(1080), Some(60), Some(Codec::H264))
  );
  assert!(!best.portrait);
  assert_eq!(
    best.url,
    "https://video-edge.example.invalid/variant/1.m3u8"
  );

  let audio = &variants[5];
  assert!(audio.audio_only() && !audio.portrait);
  assert_eq!(audio.codec, None);

  let portrait = &variants[6];
  assert!(portrait.portrait);
  assert_eq!(portrait.name, "1080p60");
  assert_eq!(portrait.codec, Some(Codec::H265));
  assert_eq!(portrait.height, Some(1920));
}

#[test]
fn v1_master_variants() {
  let variants = usher::variants(&text("usher_master_v1.m3u8"));
  let ids: Vec<&str> = variants.iter().map(|v| v.id.as_str()).collect();
  assert_eq!(
    ids,
    [
      "1080p60",
      "720p60",
      "480p30",
      "360p30",
      "160p30",
      "audio_only"
    ]
  );
  assert!(variants.iter().all(|v| !v.portrait));
  assert_eq!(variants[0].name, "1080p60");
  assert!(variants[5].audio_only());
}

#[test]
fn subscriber_only_vod_error() {
  let body = text("usher_vod_restricted.json");
  assert!(matches!(
    usher::parse_error(403, &body, &vod()),
    Error::SubscriberOnly
  ));
}

#[test]
fn offline_channel_error() {
  let body = text("usher_live_offline.json");
  assert!(matches!(
    usher::parse_error(404, &body, &live()),
    Error::Offline
  ));
}

#[test]
fn other_usher_errors_keep_only_the_code() {
  let body = r#"[{"url":"/api/v2/channel/hls/x.m3u8?sig=SECRET&token=SECRET","error":"Bad","error_code":"bad_thing"}]"#;
  let error = usher::parse_error(400, body, &live());
  let message = error.to_string();
  assert!(message.contains("bad_thing"));
  assert!(!message.contains("SECRET"));
}

#[test]
fn master_url_carries_the_token_in_its_query() {
  let token = PlaybackToken {
    value: "{\"v\":1}".into(),
    signature: "abc".into(),
    restricted: Vec::new(),
  };
  let url = usher::master_url(&vod(), &token, &[Codec::Av1, Codec::H264], Version::V2, 42).unwrap();
  assert_eq!(url.path(), "/vod/v2/2916854926.m3u8");
  let query: Vec<(String, String)> = url.query_pairs().into_owned().collect();
  let get = |k: &str| {
    query
      .iter()
      .find(|(key, _)| key == k)
      .map(|(_, v)| v.as_str())
  };
  assert_eq!(get("nauthsig"), Some("abc"));
  assert_eq!(get("nauth"), Some("{\"v\":1}"));
  assert_eq!(get("supported_codecs"), Some("av1,h264"));
  assert_eq!(get("p"), Some("42"));
  // Debug never shows the token.
  assert!(!format!("{token:?}").contains("abc"));
}

#[test]
fn live_channel() {
  let channel = gql::parse_channel(&json("gql_channel_live.json")).unwrap();
  assert_eq!(channel.login, "squeezie");
  assert_eq!(channel.display_name, "Squeezie");
  assert!(channel.title.starts_with("GENTLE MATES"));
  assert!(channel.avatar_url.unwrap().ends_with("70x70.png"));
  let live = channel.live.unwrap();
  assert_eq!(live.viewers, 80308);
  assert_eq!(live.category.as_deref(), Some("VALORANT"));
  assert_eq!(
    live.started_at,
    twitch::time::epoch_millis("2026-11-01T17:39:06Z")
  );
  assert!(live.started_at.is_some());
}

#[test]
fn offline_channel() {
  let channel = gql::parse_channel(&json("gql_channel_offline.json")).unwrap();
  assert_eq!(channel.login, "twitch");
  assert!(channel.live.is_none());
  assert!(!channel.title.is_empty());
}

#[test]
fn clip_playback() {
  let clip = gql::parse_clip_playback(&json("gql_clip_playback.json")).unwrap();
  let heights: Vec<u32> = clip.qualities.iter().map(|q| q.height).collect();
  assert_eq!(heights, [1080, 720, 480, 360]);
  assert_eq!(clip.qualities[0].frame_rate, 60);
  let url = &clip.qualities[0].url;
  assert!(url.contains("/landscape/h264/1080/index.mp4?sig=REDACTED&token="));
  // Debug shows the qualities, never the signed URLs.
  assert!(!format!("{clip:?}").contains("sig="));
}

#[test]
fn vod_storyboard() {
  let url = gql::parse_seek_previews(&json("gql_vod_seek_previews.json")).unwrap();
  assert!(url.ends_with("/storyboards/2915878235-info.json"));
  let levels = gql::parse_storyboard(&text("vod_storyboard.json"), &url);
  assert_eq!(levels.len(), 2);
  let high = &levels[0];
  assert_eq!(
    (high.width, high.height, high.cols, high.rows),
    (220, 124, 5, 10)
  );
  assert_eq!(high.images.len(), 4);
  assert!(high.images[0].ends_with("/storyboards/2915878235-high-0.jpg"));
  // Frame 0, then frame 51 (second sheet, row 0, column 1), then the last one.
  assert_eq!(high.frame_at(0.0).map(|f| (f.1, f.2)), Some((0, 0)));
  let (sheet, col, row) = high.frame_at(51.0 * 147.0).unwrap();
  assert!(sheet.ends_with("-high-1.jpg"));
  assert_eq!((col, row), (1, 0));
  let (sheet, col, row) = high.frame_at(1e9).unwrap();
  assert!(sheet.ends_with("-high-3.jpg"));
  assert_eq!((col, row), (4, 9));
}
