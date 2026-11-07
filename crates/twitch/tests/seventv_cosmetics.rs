//! 7TV cosmetics from the EventAPI: a real stream of a busy channel's `cosmetic.create` and
//! `entitlement.create` dispatches (`seventv_cosmetics.jsonl`, saved with
//! `examples/seventv_events.rs --cosmetics`, the users renamed `viewerN` with made-up Twitch ids,
//! their avatars and other platforms taken out), and an emote set update for a set no chat shows.

// Tests fail loudly on a broken fixture.
#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use twitch::seventv::cosmetics::{Cosmetic, Entitlement, Fill, Kind};
use twitch::seventv::events::{Update, dispatch};

fn updates() -> Vec<Update> {
  include_str!("fixtures/seventv_cosmetics.jsonl")
    .lines()
    .filter_map(|line| {
      let message: Value = serde_json::from_str(line).expect("JSON");
      dispatch(&message["d"])
    })
    .collect()
}

fn cosmetics() -> Vec<Cosmetic> {
  updates()
    .into_iter()
    .filter_map(|u| match u {
      Update::Cosmetic(c) => Some(c),
      _ => None,
    })
    .collect()
}

#[test]
fn paints_read_as_gradients_and_images() {
  let paints: Vec<_> = cosmetics()
    .into_iter()
    .filter_map(|c| match c {
      Cosmetic::Paint(p) => Some(p),
      Cosmetic::Badge(_) => None,
    })
    .collect();
  assert_eq!(paints.len(), 3);
  let ghoulish = &paints[0];
  assert_eq!(ghoulish.name, "Ghoulish");
  assert_eq!(ghoulish.fill, Fill::Linear(90.));
  assert!(!ghoulish.repeat);
  assert_eq!(
    ghoulish.stops,
    [
      (0.15, 0x8940_C4FF),
      (0.45, 0x5922_BFFF),
      (0.65, 0x00FF_04FF)
    ]
  );
  let radial = &paints[1];
  assert_eq!(radial.fill, Fill::Radial { circle: true });
  assert!(radial.repeat && radial.stops.len() == 4);
  assert_eq!(radial.stops[0], (0., 0xFFFF_FFFF));
  let Fill::Image(url) = &paints[2].fill else {
    panic!("an image paint: {:?}", paints[2]);
  };
  assert!(url.starts_with("https://cdn.7tv.app/paint/01KA84S7GDS77JGK2RRG7ZZ946/"));
  assert!(paints[2].stops.is_empty() && paints[2].color.is_none());
}

#[test]
fn a_badge_reads_with_its_tooltip_and_files() {
  let badges: Vec<_> = cosmetics()
    .into_iter()
    .filter_map(|c| match c {
      Cosmetic::Badge(b) => Some(b),
      Cosmetic::Paint(_) => None,
    })
    .collect();
  let [badge] = badges.as_slice() else {
    panic!("one badge: {badges:?}");
  };
  assert_eq!(badge.id, "01GAFGTXQ0000E8VNG1S1RMTC4");
  assert_eq!(badge.tooltip, "7TV Translator");
  assert_eq!(
    badge.url(18).as_deref(),
    Some("https://cdn.7tv.app/badge/01GAFGTXQ0000E8VNG1S1RMTC4/1x.webp")
  );
  assert_eq!(
    badge.url(40).as_deref(),
    Some("https://cdn.7tv.app/badge/01GAFGTXQ0000E8VNG1S1RMTC4/3x.webp")
  );
  assert!(badge.url(500).is_some_and(|u| u.ends_with("/4x.webp")));
}

#[test]
fn entitlements_name_the_twitch_user_and_come_after_their_cosmetic() {
  let mut defined = Vec::new();
  let mut entitled = Vec::new();
  for update in updates() {
    match update {
      Update::Cosmetic(Cosmetic::Paint(p)) => defined.push(p.id),
      Update::Cosmetic(Cosmetic::Badge(b)) => defined.push(b.id),
      Update::Entitled(e) => {
        assert!(defined.contains(&e.id), "{e:?} before its cosmetic");
        entitled.push(e);
      }
      // The set no chat shows is a plain set update, ignored by the chat.
      Update::Set { .. } => {}
      other => panic!("unexpected {other:?}"),
    }
  }
  assert_eq!(
    entitled[0],
    Entitlement {
      kind: Kind::Badge,
      id: "01GAFGTXQ0000E8VNG1S1RMTC4".into(),
      user: "1000001".into(),
    }
  );
  assert_eq!(entitled.len(), 4);
  assert_eq!(entitled.iter().filter(|e| e.kind == Kind::Paint).count(), 3);
}

#[test]
fn deletions_and_what_is_not_worn_read_as_such() {
  let user = |style: Value| {
    json!({"id": "0", "kind": 10, "object": {"kind": "PAINT", "ref_id": "P1", "user": {
      "style": style,
      "connections": [{"platform": "KICK", "id": "9"}, {"platform": "TWITCH", "id": "42"}]
    }}})
  };
  let delete = json!({"type": "entitlement.delete", "body": user(json!({}))});
  assert_eq!(
    dispatch(&delete),
    Some(Update::Unentitled(Entitlement {
      kind: Kind::Paint,
      id: "P1".into(),
      user: "42".into(),
    }))
  );
  // Owned but not on: the style names another paint.
  let other = json!({"type": "entitlement.create", "body": user(json!({"paint_id": "P2"}))});
  assert_eq!(dispatch(&other), None);
  // Personal emote sets are not drawn yet.
  let set = json!({"type": "entitlement.create", "body": {"object": {"kind": "EMOTE_SET",
    "ref_id": "S", "user": {"connections": [{"platform": "TWITCH", "id": "42"}]}}}});
  assert_eq!(dispatch(&set), None);
}
