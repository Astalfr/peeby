//! 7TV sets on saved responses: a channel's set (trimmed to a few emotes, without the creators'
//! accounts) and the global set.

use twitch::seventv::{Emote, parse_global, parse_user};

fn by_name<'a>(emotes: &'a [Emote], name: &str) -> &'a Emote {
  emotes
    .iter()
    .find(|e| e.name == name)
    .unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn a_channel_set_uses_its_aliases_and_keeps_case() {
  let emotes = parse_user(include_str!("fixtures/seventv_user.json")).expect("a set");
  assert_eq!(emotes.len(), 8);
  // "Bob" is the channel's name for the emote "ReallyNow".
  let bob = by_name(&emotes, "Bob");
  assert_eq!(bob.id, "01FHMGJRDR000FFDX954T14FX0");
  assert_eq!((bob.width, bob.height), (32, 32));
  assert!(!bob.animated && !bob.zero_width);
  assert_ne!(by_name(&emotes, "HUH").id, by_name(&emotes, "huh").id);
}

#[test]
fn files_are_picked_by_width_and_animated_ones_have_a_still_frame() {
  let emotes = parse_user(include_str!("fixtures/seventv_user.json")).expect("a set");
  let what = by_name(&emotes, "WHAT");
  assert!(what.animated);
  assert_eq!((what.width, what.height), (47, 32));
  let base = format!("https://cdn.7tv.app/emote/{}", what.id);
  assert_eq!(what.url(47, true), Some(format!("{base}/1x_static.webp")));
  assert_eq!(what.url(48, false), Some(format!("{base}/2x.webp")));
  // Wider than the largest kept file: the largest.
  assert_eq!(what.url(500, true), Some(format!("{base}/2x_static.webp")));
}

#[test]
fn the_global_set_marks_zero_width_emotes() {
  let emotes = parse_global(include_str!("fixtures/seventv_global.json")).expect("a set");
  assert_eq!(emotes.len(), 5);
  assert!(by_name(&emotes, "RainTime").zero_width);
  assert!(by_name(&emotes, "PETPET").zero_width);
  assert!(!by_name(&emotes, "EZ").zero_width);
}

#[test]
fn a_user_without_7tv_has_no_emotes() {
  // 7TV's 404 body: the client maps the status to no emotes; the body alone has no set either.
  assert!(
    parse_user(include_str!("fixtures/seventv_user_not_found.json"))
      .expect("JSON")
      .is_empty()
  );
}
