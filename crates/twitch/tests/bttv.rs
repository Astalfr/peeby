//! BTTV sets on saved responses: a channel's set (trimmed to a few emotes, without the uploaders'
//! accounts) and the global set.

use twitch::bttv::{Emote, parse_global, parse_user};

fn by_name<'a>(emotes: &'a [Emote], name: &str) -> &'a Emote {
  emotes
    .iter()
    .find(|e| e.name == name)
    .unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn the_global_set_skips_modifiers_and_marks_overlays() {
  let emotes = parse_global(include_str!("fixtures/bttv_global.json")).expect("a set");
  let names: Vec<&str> = emotes.iter().map(|e| e.name.as_str()).collect();
  assert_eq!(names, ["AngelThump", "D:", "SourPls", "cvHazmat"]);
  let frown = by_name(&emotes, "D:");
  assert_eq!((frown.width, frown.height), (28, 28));
  assert_eq!(
    frown.url(2),
    "https://cdn.betterttv.net/emote/55028cd2135896936880fdd7/2x.webp"
  );
  assert_eq!((by_name(&emotes, "AngelThump").width), 84);
  assert!(by_name(&emotes, "SourPls").animated);
  assert!(by_name(&emotes, "cvHazmat").zero_width);
  assert!(!frown.zero_width);
}

#[test]
fn the_global_set_modifiers_are_all_known() {
  use twitch::bttv::{Modifier, PREFIX_MODIFIERS};
  // The fixture's modifier entry, and BTTV's whole list.
  assert_eq!(Modifier::prefix("w!"), Some(Modifier::Wide));
  assert!(
    PREFIX_MODIFIERS
      .iter()
      .all(|m| Modifier::prefix(m).is_some())
  );
  assert_eq!(Modifier::suffix("ffzX"), Some(Modifier::FlipX));
  assert_eq!(Modifier::prefix("ffzX"), None);
  assert_eq!(Modifier::prefix("x!"), None);
}

#[test]
fn a_channel_set_has_its_own_and_its_shared_emotes() {
  let emotes = parse_user(include_str!("fixtures/bttv_user.json")).expect("a set");
  let names: Vec<&str> = emotes.iter().map(|e| e.name.as_str()).collect();
  assert_eq!(
    names,
    ["forsenBlunder", "forsenBucket", "EZ", "Clap", "AYAYA"]
  );
  let ayaya = by_name(&emotes, "AYAYA");
  assert_eq!((ayaya.width, ayaya.height), (28, 27));
  assert!(by_name(&emotes, "Clap").animated);
}

#[test]
fn a_user_without_bttv_has_no_emotes() {
  // BTTV's 404 body: the client maps the status to no emotes; the body alone has no set either.
  assert!(
    parse_user(include_str!("fixtures/bttv_user_not_found.json"))
      .expect("JSON")
      .is_empty()
  );
}
