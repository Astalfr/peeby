//! Hype Train parsers against real answers, captured anonymous on 2026-11-08: a public channel's
//! train under way from GQL (`gql_hype_train.json`, `hype_probe --save`), another's approach
//! (`gql_hype_train_approaching.json`, `--approaching`), and the first channel's train on its
//! PubSub topic from its approach to its end (`pubsub_hype_train.jsonl`, `raid_probe --topic
//! hype-train-events-v1`). Contributors and conductors renamed `viewerN`, participants' ids
//! replaced, long lists trimmed, request ids removed.

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::{Value, json};
use twitch::Error;
use twitch::hype_train::{self, HypeEvent, Source, State};

fn json(name: &str) -> Value {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn frames() -> Vec<String> {
  let path = format!(
    "{}/tests/fixtures/pubsub_hype_train.jsonl",
    env!("CARGO_MANIFEST_DIR")
  );
  std::fs::read_to_string(path)
    .unwrap()
    .lines()
    .map(str::to_owned)
    .collect()
}

#[test]
fn a_train_under_way_is_read() {
  let state = hype_train::parse_hype_train(&json("gql_hype_train.json")).unwrap();
  assert_eq!(state.approaching, None);
  let train = state.train.unwrap();
  assert_eq!(train.id, "aa6134ce-3203-4592-8fbd-7d7477665733");
  assert_eq!(train.progress.level, 3);
  assert_eq!(train.progress.value, 500);
  assert_eq!(train.progress.goal, 3100);
  assert_eq!(train.progress.total, 5000);
  assert_eq!(train.progress.rewards.len(), 7);
  assert_eq!(train.progress.rewards[0].token, "RawkOut");
  assert!(train.progress.rewards[0].id.starts_with("emotesv2_"));
  assert_eq!(train.color.as_deref(), Some("EB0400"));
  assert_eq!(train.level_seconds, 300);
  assert_eq!(train.record, Some(20));
  assert!(!train.record_beaten() && !train.golden_kappa && !train.treasure);
  assert_eq!(train.ended_at, None);
  assert_eq!(train.levels_done(), 2);
  assert_eq!(train.conductors.len(), 1);
  assert_eq!(train.conductors[0].source, Source::Subs);
  assert_eq!(train.conductors[0].login, "viewer1");
  assert_eq!(train.conductors[0].name, "VIEWER1");
  // The level's time: its end and its length.
  let end = train.expires_at.unwrap();
  assert_eq!(train.remaining(end - 60_000), Some(60));
  assert_eq!(train.time_left(end - 150_000), Some(0.5));
}

#[test]
fn an_approaching_train_is_read() {
  let state = hype_train::parse_hype_train(&json("gql_hype_train_approaching.json")).unwrap();
  assert_eq!(state.train, None);
  let approaching = state.approaching.unwrap();
  assert_eq!(approaching.id, "cf498401-0274-4026-9aac-8436f3c23995");
  assert_eq!((approaching.count, approaching.goal), (2, 3));
  assert_eq!(approaching.left(), 1);
  assert_eq!(approaching.color.as_deref(), Some("E474CF"));
  assert!(approaching.expires_at.is_some());
  assert_eq!(approaching.rewards[0].token, "ConfettiHype");
}

#[test]
fn an_unknown_channel_is_not_found() {
  let answer = json!({ "data": { "user": null } });
  assert!(matches!(
    hype_train::parse_hype_train(&answer),
    Err(Error::NotFound)
  ));
  let none = json!({ "data": { "user": { "channel": { "hypeTrain": {
    "approaching": null, "execution": null } } } } });
  assert_eq!(
    hype_train::parse_hype_train(&none).unwrap(),
    State::default()
  );
}

#[test]
fn a_train_runs_from_its_approach_to_its_end_on_its_topic() {
  let events: Vec<HypeEvent> = frames()
    .iter()
    .filter_map(|f| hype_train::parse_message(f))
    .collect();
  // The acceptance is not an event; the 8 messages are.
  assert_eq!(events.len(), 8);
  let mut state = State::default();
  let mut events = events.into_iter();

  let Some(HypeEvent::Approaching(approaching)) = events.next() else {
    panic!("an approach");
  };
  assert_eq!(approaching.id, "aa6134ce-3203-4592-8fbd-7d7477665733");
  assert_eq!((approaching.count, approaching.goal), (2, 3));
  state.apply(HypeEvent::Approaching(approaching));
  assert!(state.approaching.is_some());

  let Some(HypeEvent::Start(start)) = events.next() else {
    panic!("a start");
  };
  assert_eq!(start.id, "aa6134ce-3203-4592-8fbd-7d7477665733");
  // Its kickoff contributions already passed level 1.
  assert_eq!(start.progress.level, 2);
  assert_eq!(start.level_seconds, 300);
  assert!(start.started_at.is_some() && start.expires_at.is_some());
  assert!(start.record.is_some());
  state.apply(HypeEvent::Start(start));
  // The start replaces the approach.
  assert_eq!(state.approaching, None);

  let Some(HypeEvent::Conductor { train, conductor }) = events.next() else {
    panic!("a conductor");
  };
  assert_eq!(train, "aa6134ce-3203-4592-8fbd-7d7477665733");
  assert_eq!(conductor.source, Source::Subs);
  assert!(conductor.login.starts_with("viewer"));
  state.apply(HypeEvent::Conductor { train, conductor });
  assert_eq!(state.train.as_ref().unwrap().conductors.len(), 1);

  let mut level_ups = 0;
  let mut total = 0;
  for event in events.by_ref() {
    match &event {
      HypeEvent::Progress {
        progress, level_up, ..
      } => {
        assert!(progress.total >= total);
        total = progress.total;
        level_ups += u32::from(*level_up);
        state.apply(event);
        assert_eq!(state.train.as_ref().unwrap().progress.total, total);
      }
      HypeEvent::End {
        id,
        ended_at,
        unlocked,
      } => {
        assert_eq!(id.as_deref(), Some("aa6134ce-3203-4592-8fbd-7d7477665733"));
        assert!(ended_at.is_some());
        assert_eq!(unlocked[0].token, "GeorgieMarch");
        state.apply(event);
        break;
      }
      other => panic!("unexpected {other:?}"),
    }
  }
  assert_eq!(level_ups, 1);
  let train = state.train.unwrap();
  assert!(train.ended_at.is_some());
  assert_eq!(train.unlocked.len(), 8);
  assert_eq!(train.remaining(0), None);
}

#[test]
fn late_and_foreign_messages_move_nothing_back() {
  let mut state = hype_train::parse_hype_train(&json("gql_hype_train.json")).unwrap();
  let train = state.train.clone().unwrap();
  let mut older = train.progress.clone();
  older.total -= 100;
  state.apply(HypeEvent::Progress {
    id: Some(train.id.clone()),
    progress: older,
    expires_at: None,
    level_up: false,
  });
  assert_eq!(state.train.as_ref().unwrap().progress.total, 5000);
  // Another train's end.
  state.apply(HypeEvent::End {
    id: Some("other".into()),
    ended_at: Some(1),
    unlocked: Vec::new(),
  });
  assert_eq!(state.train.as_ref().unwrap().ended_at, None);
  // A GQL reading behind the topic keeps the topic's progress.
  let mut ahead = train.progress.clone();
  ahead.total += 500;
  ahead.value += 500;
  state.apply(HypeEvent::Progress {
    id: None,
    progress: ahead,
    expires_at: None,
    level_up: false,
  });
  state.merge(hype_train::parse_hype_train(&json("gql_hype_train.json")).unwrap());
  assert_eq!(state.train.as_ref().unwrap().progress.total, 5500);
  // Past the level's end and its grace, the train is taken as ended then.
  let end = train.expires_at.unwrap();
  assert!(!state.settle(end + 5_000, 10_000));
  assert!(state.settle(end + 11_000, 10_000));
  assert_eq!(state.train.unwrap().ended_at, Some(end));
}
