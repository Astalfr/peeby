//! The chat's IRC parser on captured lines (`fixtures/chat_lines.txt`: busy public channels,
//! anonymous connection, chatters renamed `viewerN` with names of the same length, so that emote
//! ranges still fit).

use twitch::chat::irc::parse;
use twitch::chat::{ChatLine, Event, Piece};

fn lines() -> Vec<Event> {
  include_str!("fixtures/chat_lines.txt")
    .lines()
    .map(|line| parse(line).unwrap_or_else(|| panic!("no event for {line}")))
    .collect()
}

fn message(events: &[Event], index: usize) -> &ChatLine {
  match &events[index] {
    Event::Message(line) | Event::Notice(line) => line,
    other => panic!("line {index} is {other:?}"),
  }
}

fn emote_names(line: &ChatLine) -> Vec<String> {
  line
    .pieces()
    .into_iter()
    .filter_map(|piece| match piece {
      Piece::Emote { name, .. } => Some(name),
      Piece::Text(_) => None,
    })
    .collect()
}

#[test]
fn the_room_state_comes_whole_on_joining() {
  let Event::RoomState(state) = &lines()[0] else {
    panic!("not a room state");
  };
  assert_eq!(state.followers_only, Some(5));
  assert_eq!(state.slow, Some(0));
  assert_eq!(state.subs_only, Some(false));
  assert_eq!(state.emote_only, Some(false));
}

#[test]
fn messages_carry_their_author_and_flags() {
  let events = lines();
  let highlighted = message(&events, 1);
  assert_eq!(highlighted.kind.as_deref(), Some("highlighted-message"));
  assert_eq!(highlighted.display_name, "viewer3___");
  assert_eq!(highlighted.login, "viewer3___");
  assert!(highlighted.user_id.starts_with("100"));
  assert_eq!(highlighted.color, None);
  let first = message(&events, 2);
  assert!(first.first);
  assert_eq!(first.color.as_deref(), Some("#1E90FF"));
  assert_eq!(first.badges, [("zevent-2026".to_owned(), "1".to_owned())]);
  assert_eq!(emote_names(first), ["TwitchUnity"]);
  assert!(message(&events, 4).text.starts_with("ça peut"));
}

#[test]
fn replies_keep_the_mention_and_their_emotes_count_it() {
  let events = lines();
  let reply = message(&events, 5);
  assert!(reply.text.starts_with("@viewer"));
  assert_eq!(emote_names(reply), ["hasHi", "hasL"]);
  assert_eq!(emote_names(message(&events, 6)), ["LUL", "LUL"]);
}

#[test]
fn emoji_before_emotes_do_not_shift_them() {
  let events = lines();
  let laughing = message(&events, 7);
  assert!(laughing.text.starts_with("🤣🤣🤣"));
  assert_eq!(emote_names(laughing), ["hasL", "hasL", "hasL"]);
  let pointing = message(&events, 8);
  assert_eq!(emote_names(pointing), ["kamelMegaphone"]);
  assert!(
    pointing
      .emotes
      .iter()
      .all(|e| e.id.starts_with("emotesv2_"))
  );
}

#[test]
fn me_messages_lose_their_wrapping_and_count_from_the_text() {
  let events = lines();
  let action = message(&events, 9);
  assert!(action.action);
  assert!(!action.text.contains('\u{1}'));
  assert_eq!(
    emote_names(action),
    [
      "nukkBagel",
      "yoteHappi",
      "HahaDoge",
      "dviperUp",
      "buffpupChonk"
    ]
  );
  let second = message(&events, 10);
  assert!(second.action);
  assert_eq!(emote_names(second), ["PogChamp"]);
}

#[test]
fn cheers_carry_their_bits() {
  assert_eq!(message(&lines(), 11).bits, 1);
}

#[test]
fn moderation_removes_messages_and_users() {
  let events = lines();
  assert!(matches!(&events[12], Event::Delete(id) if id.len() == 36));
  for index in [13, 14] {
    assert!(
      matches!(&events[index], Event::ClearUser(user) if user.starts_with("100")),
      "{:?}",
      events[index]
    );
  }
}

#[test]
fn notices_carry_their_text_and_the_user_message() {
  let events = lines();
  let sub = message(&events, 15);
  assert!(matches!(events[15], Event::Notice(_)));
  assert_eq!(sub.kind.as_deref(), Some("sub"));
  assert_eq!(
    sub.system.as_deref(),
    Some("v24____ subscribed with Prime.")
  );
  assert!(sub.text.is_empty());
  let resub = message(&events, 16);
  assert_eq!(resub.kind.as_deref(), Some("resub"));
  assert_eq!(resub.text, "Ok");
  assert_eq!(message(&events, 17).kind.as_deref(), Some("subgift"));
  assert_eq!(message(&events, 18).kind.as_deref(), Some("submysterygift"));
  assert_eq!(message(&events, 19).text, "Based");
  let announcement = message(&events, 20);
  assert_eq!(announcement.kind.as_deref(), Some("announcement"));
  assert!(announcement.text.starts_with("🤖"));
}

#[test]
fn pings_and_reconnections_are_events() {
  assert_eq!(
    parse("PING :tmi.twitch.tv"),
    Some(Event::Ping("tmi.twitch.tv".into()))
  );
  assert_eq!(parse(":tmi.twitch.tv RECONNECT"), Some(Event::Reconnect));
  assert_eq!(
    parse("@room-id=1;tmi-sent-ts=1 :tmi.twitch.tv CLEARCHAT #chan"),
    Some(Event::Clear)
  );
  assert_eq!(parse(":tmi.twitch.tv 001 justinfan1 :Welcome, GLHF!"), None);
}
