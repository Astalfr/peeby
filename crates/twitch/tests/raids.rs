//! The raid topic's parser against real PubSub frames (tests/fixtures/pubsub_raid.jsonl, received
//! anonymous on 2026-11-05 with `raid_probe`): the topic's acceptance, one raid's first
//! announcement, its last update, and its departure. Public channels' data only.

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use twitch::pubsub;
use twitch::raids::{self, RaidEvent};

fn frames() -> Vec<String> {
  let path = format!(
    "{}/tests/fixtures/pubsub_raid.jsonl",
    env!("CARGO_MANIFEST_DIR")
  );
  std::fs::read_to_string(path)
    .unwrap()
    .lines()
    .map(str::to_owned)
    .collect()
}

#[test]
fn a_raid_is_announced_grows_and_goes() {
  let frames = frames();
  // The topic's acceptance means nothing for a raid.
  assert_eq!(raids::parse_message(&frames[0]), None);
  let Some(RaidEvent::Update(first)) = raids::parse_message(&frames[1]) else {
    panic!("an announcement");
  };
  assert_eq!(first.target_login, "marzcakez");
  assert_eq!(first.target_name, "MarzCakez");
  assert!(
    first
      .target_avatar
      .as_deref()
      .is_some_and(|url| url.starts_with("https://static-cdn.jtvnw.net/"))
  );
  assert_eq!(first.viewer_count, 1);
  assert_eq!(first.created_at, Some(1_793_889_624_000));
  assert_eq!(first.force_seconds, 90);
  let Some(RaidEvent::Update(last)) = raids::parse_message(&frames[2]) else {
    panic!("an update");
  };
  assert_eq!(last.id, first.id);
  assert_eq!(last.viewer_count, 1_198);
  let Some(RaidEvent::Go(gone)) = raids::parse_message(&frames[3]) else {
    panic!("a departure");
  };
  assert_eq!(gone.id, first.id);
  assert_eq!(gone.target_login, "marzcakez");
}

#[test]
fn cancels_other_topics_and_broken_frames() {
  let frames = frames();
  let mut frame: Value = serde_json::from_str(&frames[1]).unwrap();
  let mut message: Value =
    serde_json::from_str(frame["data"]["message"].as_str().unwrap()).unwrap();
  let id = message["raid"]["id"].as_str().unwrap().to_owned();
  message["type"] = json!("raid_cancel_v2");
  frame["data"]["message"] = json!(message.to_string());
  assert_eq!(
    raids::parse_message(&frame.to_string()),
    Some(RaidEvent::Cancel(id))
  );
  frame["data"]["topic"] = json!("video-playback-by-id.1");
  assert_eq!(raids::parse_message(&frame.to_string()), None);
  assert_eq!(raids::parse_message(r#"{ "type": "PONG" }"#), None);
  assert_eq!(raids::parse_message("not json"), None);
  // The connection asks for the raid topic and the Hype Train's, one `LISTEN` each.
  let frames = pubsub::listen_frames("42");
  assert!(frames[0].contains(r#""topics":["raid.42"]"#));
  assert!(frames[1].contains(r#""topics":["hype-train-events-v1.42"]"#));
}

#[test]
fn the_connection_tells_raids_from_hype_trains() {
  let frames = frames();
  assert!(matches!(
    pubsub::parse_message(&frames[1]),
    Some(pubsub::Event::Raid(RaidEvent::Update(_)))
  ));
  let path = format!(
    "{}/tests/fixtures/pubsub_hype_train.jsonl",
    env!("CARGO_MANIFEST_DIR")
  );
  let hype = std::fs::read_to_string(path).unwrap();
  let mut lines = hype.lines();
  assert_eq!(pubsub::parse_message(lines.next().unwrap()), None);
  assert!(matches!(
    pubsub::parse_message(lines.next().unwrap()),
    Some(pubsub::Event::HypeTrain(_))
  ));
}
