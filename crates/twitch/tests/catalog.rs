//! The GQL catalog's parsers against real answers (tests/fixtures/gql_catalog_*.json, captured
//! 2026-11-03 anonymously by `cargo run -p peeby --example twitch_capture -- catalog`, trimmed).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use twitch::{Error, catalog, helix};

fn json(name: &str) -> Value {
  let path = format!(
    "{}/tests/fixtures/gql_catalog_{name}.json",
    env!("CARGO_MANIFEST_DIR")
  );
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn top_categories() {
  let categories = catalog::parse_top_categories(&json("top_categories")).unwrap();
  assert_eq!(categories.len(), 3);
  assert_eq!(categories[0].id, "509658");
  assert_eq!(categories[0].name, "Just Chatting");
  assert_eq!(
    helix::sized(&categories[0].box_art_url, 150, 200),
    "https://static-cdn.jtvnw.net/ttv-boxart/509658-150x200.jpg"
  );
}

#[test]
fn category_streams() {
  let streams = catalog::parse_category_streams(&json("category_streams")).unwrap();
  assert_eq!(streams.len(), 3);
  let first = &streams[0];
  assert_eq!(first.user_login, "therealknossi");
  assert_eq!(first.user_name, "TheRealKnossi");
  assert_eq!(first.user_id, "71588578");
  assert_eq!(first.game_name, "Just Chatting");
  assert_eq!(first.viewer_count, 6185);
  assert!(first.started_at.is_some());
  assert_eq!(first.tags, ["Deutsch", "DropsAktiviert"]);
  assert!(first.thumbnail_url.ends_with("-{width}x{height}.jpg"));
}

#[test]
fn users() {
  let users = catalog::parse_users(&json("users")).unwrap();
  let logins: Vec<&str> = users.iter().map(|u| u.login.as_str()).collect();
  assert_eq!(logins, ["ow_esports", "squeezie"]);
  assert!(users.iter().all(|u| u.partner));
  assert!(users[0].profile_image_url.contains("profile_image"));
}

#[test]
fn user() {
  let user = catalog::parse_user(&json("user")).unwrap();
  assert_eq!(user.id, "137512364");
  assert_eq!(user.display_name, "ow_esports");
  assert!(!user.description.is_empty());
  assert!(user.offline_image_url.contains("offline_image"));
  assert!(user.partner);
  assert!(matches!(
    catalog::parse_user(&json("user_not_found")),
    Err(Error::NotFound)
  ));
}

#[test]
fn videos() {
  let videos = catalog::parse_videos(&json("videos")).unwrap();
  assert_eq!(videos.len(), 3);
  let first = &videos[0];
  assert_eq!(first.id, "2916849417");
  assert_eq!(first.user_login, "ow_esports");
  assert_eq!(first.duration, 25265);
  assert_eq!(first.view_count, 312900);
  assert!(first.created_at.is_some());
  assert!(first.thumbnail_url.ends_with("thumb0-{width}x{height}.jpg"));
  assert_eq!(first.muted_segments.len(), 3);
}

#[test]
fn video_with_muted_parts() {
  let video = catalog::parse_video(&json("video")).unwrap();
  assert_eq!(video.id, "2915878235");
  assert_eq!(video.duration, 29349);
  assert_eq!(
    video.muted_segments,
    [
      helix::MutedSegment {
        offset: 0,
        duration: 180
      },
      helix::MutedSegment {
        offset: 721,
        duration: 180
      },
      helix::MutedSegment {
        offset: 5047,
        duration: 180
      },
    ]
  );
}

#[test]
fn clips() {
  let clips = catalog::parse_clips(&json("clips")).unwrap();
  assert_eq!(clips.len(), 3);
  // The slug, as Helix's id.
  assert_eq!(clips[0].id, "OpenRenownedNewtPMSTwin");
  let clip = catalog::parse_clip(&json("clip")).unwrap();
  assert_eq!(clip, clips[0]);
  assert_eq!(clip.broadcaster_id, "137512364");
  assert_eq!(clip.creator_name, "chegg_ow");
  assert_eq!(clip.view_count, 345913);
  assert_eq!(clip.duration, 30.0);
  assert!(clip.thumbnail_url.ends_with(".jpg"));
}

#[test]
fn search() {
  let channels = catalog::parse_search_channels(&json("search_channels")).unwrap();
  assert_eq!(channels.len(), 3);
  let first = &channels[0];
  assert_eq!(first.login, "squeezie");
  assert_eq!(first.display_name, "Squeezie");
  assert!(!first.is_live);
  // Offline: the last category.
  assert_eq!(first.game_name, "Fortnite");
  assert!(!first.title.is_empty());
  assert!(first.thumbnail_url.contains("profile_image"));
  let categories = catalog::parse_search_categories(&json("search_categories")).unwrap();
  assert_eq!(categories[0].name, "Minecraft");
  assert!(categories[0].box_art_url.contains("{width}x{height}"));
}

#[test]
fn badges() {
  let global = catalog::parse_badges(&json("badges_global")).unwrap();
  assert_eq!(global.len(), 8);
  let moderator = global.iter().find(|b| b.set == "moderator").unwrap();
  assert_eq!(moderator.version, "1");
  assert_eq!(moderator.title, "Moderator");
  // Helix's sizes: 18, 36 and 72 px.
  assert!(moderator.url_1x.ends_with("/1"));
  assert!(moderator.url_2x.ends_with("/2"));
  assert!(moderator.url_4x.ends_with("/3"));
  let channel = catalog::parse_badges(&json("badges_channel")).unwrap();
  assert!(!channel.is_empty());
  assert!(channel.iter().all(|b| b.set == "subscriber"));
  assert_eq!(channel[0].version, "2072");
}

/// The answer to a second page, as seen on 2026-11-03.
#[test]
fn integrity_failure() {
  let answer = json!({
    "errors": [{ "message": "failed integrity check", "path": ["games"] }],
    "data": { "games": null },
  });
  assert!(matches!(
    catalog::parse_top_categories(&answer),
    Err(Error::IntegrityRequired)
  ));
}

/// A list that comes back null without an error is a failure (Helix is asked), not an empty page.
#[test]
fn null_list() {
  let answer = json!({ "data": { "games": null } });
  assert!(matches!(
    catalog::parse_top_categories(&answer),
    Err(Error::Parse { .. })
  ));
  let unknown = json!({ "data": { "game": null } });
  assert!(matches!(
    catalog::parse_category_streams(&unknown),
    Err(Error::NotFound)
  ));
}

#[test]
fn schedule() {
  // Ponce's schedule (captured 2026-11-11, anonymous): seven segments of the week, titled, each
  // with its category; and a channel without a schedule.
  let answer: serde_json::Value =
    serde_json::from_str(include_str!("fixtures/gql_schedule.json")).unwrap();
  let schedule = twitch::catalog::parse_schedule(&answer)
    .unwrap()
    .expect("a schedule");
  assert_eq!(schedule.segments.len(), 7);
  let first = &schedule.segments[0];
  assert_eq!(first.title, "Le lundi c'est culture !");
  assert_eq!(first.category.as_deref(), Some("Maitre des Fleurs"));
  assert!(
    first
      .box_art
      .as_deref()
      .is_some_and(|art| art.contains("{width}x{height}"))
  );
  assert!(first.end.is_some_and(|end| end > first.start));
  assert!(!first.cancelled);
  assert_eq!(schedule.vacation, None);
  // Those not over on 2026-11-11 at noon UTC, soonest first.
  let noon = twitch::time::epoch_millis("2026-11-11T12:00:00Z").unwrap();
  let upcoming = schedule.upcoming(noon);
  assert_eq!(
    upcoming.first().map(|s| s.title.as_str()),
    Some("On continue le REMAKE")
  );
  assert!(upcoming.windows(2).all(|w| w[0].start <= w[1].start));
  let none: serde_json::Value =
    serde_json::from_str(include_str!("fixtures/gql_schedule_none.json")).unwrap();
  assert_eq!(twitch::catalog::parse_schedule(&none).unwrap(), None);
}
