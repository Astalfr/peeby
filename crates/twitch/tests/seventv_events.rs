//! 7TV EventAPI dispatches: a real `emote_set.update` (`seventv_event_pushed.json`, saved with
//! `examples/seventv_events.rs`, the editor and the emote's creator renamed), and what a set does
//! with the changes.

// Tests fail loudly on a broken fixture.
#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use twitch::seventv::events::{Change, Update, dispatch};
use twitch::seventv::{Set, parse_user};

fn data(text: &str) -> Value {
  let message: Value = serde_json::from_str(text).expect("JSON");
  message["d"].clone()
}

#[test]
fn a_pushed_emote_is_read_as_the_set_entry_it_is() {
  let update = dispatch(&data(include_str!("fixtures/seventv_event_pushed.json")));
  let Some(Update::Set { id, changes }) = update else {
    panic!("a set update: {update:?}");
  };
  assert_eq!(id, "01FHBRYNJ00008T926GQR9T23X");
  let [Change::Added(emote)] = changes.as_slice() else {
    panic!("one added emote: {changes:?}");
  };
  assert_eq!(emote.name, "CLEAN");
  assert_eq!(emote.id, "01F6N2WHKG000AR0YATR3ZV5X4");
  assert!(emote.animated && !emote.zero_width);
  assert_eq!(
    emote.url(32, false).as_deref(),
    Some("https://cdn.7tv.app/emote/01F6N2WHKG000AR0YATR3ZV5X4/1x.webp")
  );
}

#[test]
fn a_user_update_and_an_unreadable_push_ask_for_the_sets_again() {
  let user = json!({"type": "user.update", "body": {"id": "01G8964EZ0000DKQYXCZGWX037"}});
  assert_eq!(dispatch(&user), Some(Update::Resync));
  let pushed = json!({"type": "emote_set.update", "body": {"id": "s", "pushed": [
    {"key": "emotes", "index": 0, "value": {"id": "e", "name": "NoData"}}
  ]}});
  assert_eq!(dispatch(&pushed), Some(Update::Resync));
  // A change to another field than the emotes (the set's name) is nothing to Peeby.
  let renamed_set = json!({"type": "emote_set.update", "body": {"id": "s", "updated": [
    {"key": "name", "old_value": "Old", "value": "New"}
  ]}});
  assert_eq!(dispatch(&renamed_set), None);
}

#[test]
fn pulled_and_renamed_emotes_are_read_from_their_old_and_new_entries() {
  // Hand-written from 7TV's ChangeMap (docs/twitch.md): no removal or rename was seen while probing.
  let body = json!({"type": "emote_set.update", "body": {"id": "s",
    "pulled": [{"key": "emotes", "index": 3, "old_value": {"id": "a", "name": "Gone"}, "value": null}],
    "updated": [{"key": "emotes", "index": 4,
      "old_value": {"id": "b", "name": "Before"}, "value": {"id": "b", "name": "After"}}]
  }});
  assert_eq!(
    dispatch(&body),
    Some(Update::Set {
      id: "s".into(),
      changes: vec![
        Change::Removed {
          id: "a".into(),
          name: "Gone".into()
        },
        Change::Renamed {
          id: "b".into(),
          from: "Before".into(),
          to: "After".into()
        },
      ],
    })
  );
}

#[test]
fn a_set_applies_additions_removals_and_renames() {
  let mut set: Set = parse_user(include_str!("fixtures/seventv_user.json")).expect("a set");
  let count = set.emotes.len();
  let Some(Update::Set { changes, .. }) =
    dispatch(&data(include_str!("fixtures/seventv_event_pushed.json")))
  else {
    panic!("a set update");
  };
  for change in &changes {
    set.apply(change);
    // Twice: another tab on the same channel applies it to the same copy.
    set.apply(change);
  }
  assert_eq!(set.emotes.len(), count + 1);
  let bob = set
    .emotes
    .iter()
    .find(|e| e.name == "Bob")
    .expect("Bob")
    .id
    .clone();
  set.apply(&Change::Renamed {
    id: bob.clone(),
    from: "Bob".into(),
    to: "Robert".into(),
  });
  assert!(set.emotes.iter().any(|e| e.name == "Robert" && e.id == bob));
  set.apply(&Change::Removed {
    id: bob,
    name: "Robert".into(),
  });
  assert!(!set.emotes.iter().any(|e| e.name == "Robert"));
  assert!(set.emotes.iter().any(|e| e.name == "CLEAN"));
  assert_eq!(set.emotes.len(), count);
}
