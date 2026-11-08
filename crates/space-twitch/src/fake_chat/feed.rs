//! The fake chat's lines, written as Twitch's IRC server writes them (tags, prefix, command), so
//! that they go through the real parser (`twitch::chat::irc::parse`) and the space's conversion
//! (`chat::event`) like a live's. Seeded: the same seed gives the same chat.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use ui::chat::fake::Rng;

/// The fake channel's login.
pub const CHANNEL: &str = "peeby_debug";
/// The fake channel's user id (`room-id`).
const ROOM_ID: &str = "4242";
/// Messages kept for the moderation buttons (deleting one, timing out its author).
const RECENT: usize = 100;

/// A chatter.
pub struct User {
  pub id: &'static str,
  pub login: &'static str,
  pub name: &'static str,
  /// `#RRGGBB`, when they chose one.
  pub color: Option<&'static str>,
  /// The `badges` tag: `set/version` pairs.
  pub badges: &'static str,
}

/// The viewer, whose messages the fake writer echoes.
pub const ME: User = User {
  id: "1000",
  login: "toi_peeby",
  name: "Toi_Peeby",
  color: Some("#9146FF"),
  badges: "subscriber/12",
};

/// The other chatters: no color, dark ones (lightened by the panel), a name that is not the
/// login, every badge.
const USERS: &[User] = &[
  User {
    id: "1001",
    login: "viewer1",
    name: "Viewer1",
    color: None,
    badges: "",
  },
  User {
    id: "1002",
    login: "viewer_deux",
    name: "Viewer_Deux",
    color: Some("#0000FF"),
    badges: "subscriber/3",
  },
  User {
    id: "1003",
    login: "modo_trois",
    name: "Modo_Trois",
    color: Some("#9ACD32"),
    badges: "moderator/1,subscriber/24",
  },
  User {
    id: "1004",
    login: "vip4",
    name: "Vip4",
    color: Some("#FF69B4"),
    badges: "vip/1",
  },
  User {
    id: "4242",
    login: "peeby_debug",
    name: "Peeby_Debug",
    color: Some("#FF4500"),
    badges: "broadcaster/1,subscriber/0",
  },
  User {
    id: "1006",
    login: "noir6",
    name: "Noir6",
    color: Some("#000000"),
    badges: "",
  },
  User {
    id: "1007",
    login: "viewer7",
    name: "観客セブン",
    color: Some("#1E90FF"),
    badges: "subscriber/6,premium/1",
  },
  User {
    id: "1008",
    login: "huit_huit",
    name: "Huit_Huit",
    color: None,
    badges: "turbo/1",
  },
];

/// The badges the chatters wear: set, version, title.
pub const BADGES: &[(&str, &str, &str)] = &[
  ("broadcaster", "1", "Broadcaster"),
  ("moderator", "1", "Moderator"),
  ("vip", "1", "VIP"),
  ("subscriber", "0", "Subscriber"),
  ("subscriber", "3", "3-Month Subscriber"),
  ("subscriber", "6", "6-Month Subscriber"),
  ("subscriber", "12", "1-Year Subscriber"),
  ("subscriber", "24", "2-Year Subscriber"),
  ("premium", "1", "Prime Gaming"),
  ("turbo", "1", "Turbo"),
];

/// How a third-party emote looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
  Square,
  /// Twice as wide as high.
  Wide,
  /// Blinks (a GIF).
  Animated,
  /// Drawn over the emote before it (7TV's zero-width).
  Overlay,
}

/// The fake channel's 7TV and BTTV emotes, replaced in the text by the space's conversion.
pub const THIRD_PARTY: &[(&str, Look)] = &[
  ("catJAM", Look::Animated),
  ("peepoHappy", Look::Square),
  ("OMEGALUL", Look::Square),
  ("WIDEPEEPO", Look::Wide),
  ("Clap", Look::Animated),
  ("EZ", Look::Square),
  ("SnowTime", Look::Overlay),
  ("RainTime", Look::Overlay),
];

/// Twitch's own global emotes (id, code): their images come from Twitch's CDN.
const TWITCH_EMOTES: &[(&str, &str)] = &[
  ("25", "Kappa"),
  ("305954156", "PogChamp"),
  ("425618", "LUL"),
  ("86", "BibleThump"),
  ("41", "Kreygasm"),
];

const WORDS: &[&str] = &[
  "salut",
  "le",
  "chat",
  "c'est",
  "incroyable",
  "gg",
  "wp",
  "quel",
  "match",
  "franchement",
  "non",
  "mais",
  "regardez",
  "ça",
  "trop",
  "fort",
  "il",
  "a",
  "raté",
  "encore",
  "une",
  "fois",
  "allez",
  "on",
  "y",
  "croit",
  "été",
  "déjà",
  "où",
  "Ça",
  "l'équipe",
];
/// Real Giphy GIFs (id, the title Twitch writes for them).
const GIFS: &[(&str, &str)] = &[
  ("joSNxeswxuc74Juo8X", "[Y A Y Yes GIF by Djemilah Birnie]"),
  (
    "3o7abKhOpu0NwenH3O",
    "[SpongeBob SquarePants GIF by SpongeBob SquarePants]",
  ),
];
const EMOJI: &[&str] = &["😂", "🔥", "❤️", "👀", "🇫🇷", "👍🏽", "🫠"];
const LINKS: &[&str] = &[
  "https://example.com/un/lien/assez/long/pour/voir/comment/il/passe/a/la/ligne",
  "https://www.twitch.tv/peeby_debug",
  "http://example.org",
];

/// What the generated messages hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
  /// Twitch's emotes (ranges in the `emotes` tag; images from Twitch's CDN).
  pub twitch_emotes: bool,
  /// Twitch's GIFs (ranges in the `gifs` tag; images from Giphy).
  pub gifs: bool,
  /// 7TV and BTTV words, animated and zero-width ones included, BTTV's modifiers too.
  pub third_party: bool,
  pub badges: bool,
  pub colors: bool,
  pub mentions: bool,
  pub links: bool,
  pub emoji: bool,
  /// Now and then a message of 40 words or more.
  pub long: bool,
  /// `/me`.
  pub actions: bool,
  /// Highlighted messages (paid with points).
  pub highlighted: bool,
}

impl Default for Content {
  fn default() -> Self {
    Self {
      // Off by default: the only part that reaches the network.
      twitch_emotes: false,
      gifs: false,
      third_party: true,
      badges: true,
      colors: true,
      mentions: true,
      links: true,
      emoji: true,
      long: true,
      actions: true,
      highlighted: true,
    }
  }
}

/// An event of the channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Notice {
  Sub,
  /// A resubscription with the user's message.
  Resub,
  Gift,
  Raid,
}

/// Real 7TV and BTTV words for the messages, in place of [`THIRD_PARTY`].
pub struct RealWords {
  /// Emotes of their own.
  pub plain: Vec<String>,
  /// Zero-width ones, written after another.
  pub overlays: Vec<String>,
}

/// Writes the chat's lines.
pub struct Feed {
  /// The real global emotes' words, while the fake chat uses them.
  pub real: Option<RealWords>,
  rng: Rng,
  n: u64,
  /// The last messages' ids, with their author's id.
  recent: VecDeque<(String, &'static str)>,
}

impl Feed {
  pub fn new(seed: u64) -> Self {
    Self {
      real: None,
      rng: Rng::new(seed),
      n: 0,
      recent: VecDeque::new(),
    }
  }

  fn next_id(&mut self) -> String {
    self.n += 1;
    format!("fake-{}", self.n)
  }

  /// A random chatter's message.
  pub fn message(&mut self, content: &Content) -> String {
    let user = &USERS[self.rng.below(USERS.len())];
    let count = if content.long && self.rng.one_in(12) {
      40 + self.rng.below(40)
    } else {
      1 + self.rng.below(14)
    };
    let mut words: Vec<String> = Vec::new();
    for _ in 0..count {
      let word = match self.rng.below(14) {
        0 | 1 if content.third_party => {
          let (plain, overlays): (Vec<&str>, Vec<&str>) = match &self.real {
            Some(real) => (
              real.plain.iter().map(String::as_str).collect(),
              real.overlays.iter().map(String::as_str).collect(),
            ),
            None => {
              let named = |overlay: bool| {
                THIRD_PARTY
                  .iter()
                  .filter(move |(_, look)| (*look == Look::Overlay) == overlay)
                  .map(|(name, _)| *name)
                  .collect()
              };
              (named(false), named(true))
            }
          };
          let mut word = self.rng.pick(&plain).copied().unwrap_or("EZ").to_owned();
          // Now and then one of BTTV's modifiers before it (`w!`, `h!`…), more rarely two.
          for _ in 0..2 {
            if self.rng.one_in(5)
              && let Some(modifier) = self.rng.pick(&twitch::bttv::PREFIX_MODIFIERS)
            {
              word = format!("{modifier} {word}");
            } else {
              break;
            }
          }
          // Now and then one overlay, more rarely two: 7TV stacks them all on the emote.
          for _ in 0..2 {
            if self.rng.one_in(3)
              && let Some(overlay) = self.rng.pick(&overlays)
            {
              word = format!("{word} {overlay}");
            } else {
              break;
            }
          }
          word
        }
        2 if content.twitch_emotes => self
          .rng
          .pick(TWITCH_EMOTES)
          .map_or("Kappa", |(_, code)| code)
          .to_owned(),
        3 if content.mentions && self.rng.one_in(2) => {
          // The viewer now and then, as the chat answers them.
          let name = if self.rng.one_in(4) {
            ME.name
          } else {
            USERS[self.rng.below(USERS.len())].name
          };
          format!("@{name}")
        }
        4 if content.links && self.rng.one_in(3) => {
          self.rng.pick(LINKS).copied().unwrap_or_default().to_owned()
        }
        5 | 6 if content.emoji => self.rng.pick(EMOJI).copied().unwrap_or_default().to_owned(),
        _ => self.rng.pick(WORDS).copied().unwrap_or("gg").to_owned(),
      };
      words.push(word);
    }
    // Now and then a GIF, at the end as Twitch's composer puts it, or alone.
    if content.gifs && self.rng.one_in(8) {
      if self.rng.one_in(2) {
        words.clear();
      }
      words.push(
        self
          .rng
          .pick(GIFS)
          .map_or("[GIF]", |(_, title)| title)
          .to_owned(),
      );
    }
    let text = words.join(" ");
    let action = content.actions && self.rng.one_in(25);
    let highlighted = !action && content.highlighted && self.rng.one_in(40);
    self.privmsg(
      user,
      &text,
      action,
      highlighted.then_some("highlighted-message"),
      content,
    )
  }

  /// `user`'s message `text` as a `PRIVMSG`: `/me` when `action`, with `msg_id` when given.
  pub fn privmsg(
    &mut self,
    user: &'static User,
    text: &str,
    action: bool,
    msg_id: Option<&str>,
    content: &Content,
  ) -> String {
    let id = self.next_id();
    self.remember(&id, user.id);
    let mut tags = self.user_tags(user, &id, text, content);
    if let Some(msg_id) = msg_id {
      tags.push(("msg-id", msg_id.to_owned()));
    }
    let body = if action {
      format!("\u{1}ACTION {text}\u{1}")
    } else {
      text.to_owned()
    };
    let login = user.login;
    format!(
      "@{} :{login}!{login}@{login}.tmi.twitch.tv PRIVMSG #{CHANNEL} :{body}",
      tag_string(&tags)
    )
  }

  /// An event of the channel, by a random chatter, as a `USERNOTICE`.
  pub fn notice(&mut self, kind: Notice, content: &Content) -> String {
    let user = &USERS[self.rng.below(USERS.len())];
    let other = &USERS[self.rng.below(USERS.len())];
    let (msg_id, system, text) = match kind {
      Notice::Sub => ("sub", format!("{} subscribed at Tier 1.", user.name), None),
      Notice::Resub => (
        "resub",
        format!(
          "{} subscribed at Tier 1. They've subscribed for 14 months!",
          user.name
        ),
        Some("toujours là depuis le début peepoHappy"),
      ),
      Notice::Gift => (
        "subgift",
        format!("{} gifted a Tier 1 sub to {}!", user.name, other.name),
        None,
      ),
      Notice::Raid => (
        "raid",
        format!(
          "{} raiders from {} have joined!",
          10 + self.rng.below(990),
          user.name
        ),
        None,
      ),
    };
    let id = self.next_id();
    self.remember(&id, user.id);
    let mut tags = self.user_tags(user, &id, text.unwrap_or_default(), content);
    tags.extend([
      ("login", user.login.to_owned()),
      ("msg-id", msg_id.to_owned()),
      ("system-msg", system),
    ]);
    let tail = text.map(|t| format!(" :{t}")).unwrap_or_default();
    format!(
      "@{} :tmi.twitch.tv USERNOTICE #{CHANNEL}{tail}",
      tag_string(&tags)
    )
  }

  /// The tags of `user`'s line `id` saying `text`.
  fn user_tags(
    &self,
    user: &User,
    id: &str,
    text: &str,
    content: &Content,
  ) -> Vec<(&'static str, String)> {
    vec![
      (
        "badges",
        if content.badges { user.badges } else { "" }.to_owned(),
      ),
      (
        "color",
        user
          .color
          .filter(|_| content.colors)
          .unwrap_or_default()
          .to_owned(),
      ),
      ("display-name", user.name.to_owned()),
      (
        "emotes",
        if content.twitch_emotes {
          emotes_tag(text)
        } else {
          String::new()
        },
      ),
      (
        "gifs",
        if content.gifs {
          gifs_tag(text)
        } else {
          String::new()
        },
      ),
      ("id", id.to_owned()),
      ("room-id", ROOM_ID.to_owned()),
      ("tmi-sent-ts", now_millis().to_string()),
      ("user-id", user.id.to_owned()),
    ]
  }

  fn remember(&mut self, id: &str, user: &'static str) {
    self.recent.push_back((id.to_owned(), user));
    while self.recent.len() > RECENT {
      self.recent.pop_front();
    }
  }

  /// The last message removed by a moderator (`CLEARMSG`).
  pub fn delete_last(&mut self) -> Option<String> {
    let (id, _) = self.recent.pop_back()?;
    Some(clear_msg(&id))
  }

  /// A recent message removed by a moderator.
  pub fn delete_recent(&mut self) -> Option<String> {
    let at = self.rng.below(self.recent.len());
    let (id, _) = self.recent.remove(at)?;
    Some(clear_msg(&id))
  }

  /// A recent message's author timed out (`CLEARCHAT` with a target): their messages go.
  pub fn timeout(&mut self) -> Option<String> {
    let (_, user) = self.recent.get(self.rng.below(self.recent.len()))?.clone();
    self.recent.retain(|(_, u)| *u != user);
    let login = USERS
      .iter()
      .chain([&ME])
      .find(|u| u.id == user)
      .map_or("viewer", |u| u.login);
    Some(format!(
      "@ban-duration=600;room-id={ROOM_ID};target-user-id={user};tmi-sent-ts={} :tmi.twitch.tv \
       CLEARCHAT #{CHANNEL} :{login}",
      now_millis()
    ))
  }

  /// The whole chat cleared (`CLEARCHAT` alone).
  pub fn clear(&mut self) -> String {
    self.recent.clear();
    format!(
      "@room-id={ROOM_ID};tmi-sent-ts={} :tmi.twitch.tv CLEARCHAT #{CHANNEL}",
      now_millis()
    )
  }
}

/// The room's modes as `ROOMSTATE` sends them: every one on joining, then only what changed
/// (`tags`: `slow=30`…).
pub fn room_state(tags: &[(&str, String)]) -> String {
  let mut all = vec![("room-id", ROOM_ID.to_owned())];
  all.extend(tags.iter().map(|(k, v)| (*k, v.clone())));
  format!("@{} :tmi.twitch.tv ROOMSTATE #{CHANNEL}", tag_string(&all))
}

/// A `NOTICE` of the server with `msg_id`.
pub fn server_notice(msg_id: &str) -> String {
  format!("@msg-id={msg_id} :tmi.twitch.tv NOTICE #{CHANNEL} :{msg_id}")
}

fn clear_msg(id: &str) -> String {
  format!(
    "@login=viewer;room-id=;target-msg-id={id};tmi-sent-ts={} :tmi.twitch.tv CLEARMSG #{CHANNEL} \
     :supprimé",
    now_millis()
  )
}

/// The `emotes` tag for the Twitch emotes in `text`: `id:start-end,…/…`, `char` indexes, end
/// included, as Twitch counts them.
fn emotes_tag(text: &str) -> String {
  let mut ranges: Vec<(&str, Vec<String>)> = Vec::new();
  let mut at = 0;
  for word in text.split(' ') {
    let len = word.chars().count();
    if let Some((id, _)) = TWITCH_EMOTES.iter().find(|(_, code)| *code == word) {
      let range = format!("{at}-{}", at + len - 1);
      match ranges.iter_mut().find(|(i, _)| i == id) {
        Some((_, list)) => list.push(range),
        None => ranges.push((id, vec![range])),
      }
    }
    // The space after the word.
    at += len + 1;
  }
  ranges
    .into_iter()
    .map(|(id, list)| format!("{id}:{}", list.join(",")))
    .collect::<Vec<_>>()
    .join("/")
}

/// The `gifs` tag for the GIF titles in `text`: `start-end|id|url,…`, `char` indexes, end
/// included.
fn gifs_tag(text: &str) -> String {
  let mut gifs = Vec::new();
  for (id, title) in GIFS {
    for (byte, _) in text.match_indices(title) {
      let start = text[..byte].chars().count();
      let end = start + title.chars().count() - 1;
      gifs.push(format!(
        "{start}-{end}|{id}|https://media4.giphy.com/media/{id}/giphy.gif"
      ));
    }
  }
  gifs.join(",")
}

/// `key=value;…`, values escaped as IRCv3 tags are.
fn tag_string(tags: &[(&str, String)]) -> String {
  tags
    .iter()
    .map(|(key, value)| {
      let escaped: String = value
        .chars()
        .map(|c| match c {
          '\\' => "\\\\".to_owned(),
          ';' => "\\:".to_owned(),
          ' ' => "\\s".to_owned(),
          '\r' => "\\r".to_owned(),
          '\n' => "\\n".to_owned(),
          c => c.to_string(),
        })
        .collect();
      format!("{key}={escaped}")
    })
    .collect::<Vec<_>>()
    .join(";")
}

fn now_millis() -> u128 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| d.as_millis())
}

#[cfg(test)]
mod tests {
  use super::*;
  use twitch::chat::{Event, Piece, RoomState, irc};

  fn all() -> Content {
    Content {
      twitch_emotes: true,
      gifs: true,
      ..Content::default()
    }
  }

  #[test]
  fn generated_messages_parse_with_emotes_where_they_are() {
    let mut feed = Feed::new(7);
    let content = all();
    let (mut emotes, mut gifs) = (0, 0);
    for _ in 0..500 {
      let line = feed.message(&content);
      let Some(Event::Message(message)) = irc::parse(&line) else {
        panic!("not a message: {line}");
      };
      assert!(USERS.iter().any(|u| u.id == message.user_id));
      // Every range covers its code, whatever the emoji and accents before it.
      for piece in message.pieces() {
        if let Piece::Emote { id, name } = &piece {
          emotes += 1;
          assert!(
            TWITCH_EMOTES.contains(&(id.as_str(), name.as_str())),
            "{name}"
          );
        }
        if let Piece::Gif { id, title } = &piece {
          gifs += 1;
          assert!(GIFS.contains(&(id.as_str(), title.as_str())), "{title}");
        }
      }
      assert_eq!(
        message.emotes.len(),
        message
          .text
          .split(' ')
          .filter(|w| TWITCH_EMOTES.iter().any(|(_, c)| c == w))
          .count()
      );
    }
    assert!(emotes > 0 && gifs > 0);
  }

  #[test]
  fn names_and_notices_survive_the_escaping() {
    let mut feed = Feed::new(3);
    let line = feed.notice(Notice::Resub, &Content::default());
    let Some(Event::Notice(notice)) = irc::parse(&line) else {
      panic!("not a notice: {line}");
    };
    assert!(
      notice
        .system
        .as_deref()
        .is_some_and(|s| s.contains(" subscribed at Tier 1."))
    );
    assert_eq!(notice.text, "toujours là depuis le début peepoHappy");
    let me = feed.privmsg(&ME, "salut ; à tous", true, None, &Content::default());
    let Some(Event::Message(message)) = irc::parse(&me) else {
      panic!("not a message: {me}");
    };
    assert!(message.action);
    assert_eq!(message.display_name, "Toi_Peeby");
    assert_eq!(message.text, "salut ; à tous");
  }

  #[test]
  fn moderation_targets_recent_messages() {
    let mut feed = Feed::new(5);
    assert!(feed.delete_last().is_none());
    let line = feed.message(&Content::default());
    let Some(Event::Message(message)) = irc::parse(&line) else {
      panic!("not a message");
    };
    let timeout = feed.timeout().expect("a recent author");
    assert_eq!(
      irc::parse(&timeout),
      Some(Event::ClearUser(message.user_id.clone()))
    );
    feed.message(&Content::default());
    assert!(matches!(
      feed.delete_last().as_deref().and_then(irc::parse),
      Some(Event::Delete(_))
    ));
    assert_eq!(irc::parse(&feed.clear()), Some(Event::Clear));
    let Some(Event::RoomState(state)) = irc::parse(&room_state(&[("slow", "30".into())])) else {
      panic!("not a room state");
    };
    assert_eq!(
      state,
      RoomState {
        slow: Some(30),
        room_id: Some(ROOM_ID.into()),
        ..RoomState::default()
      }
    );
    assert_eq!(
      irc::parse(&server_notice("msg_slowmode")),
      Some(Event::ServerNotice("msg_slowmode".into()))
    );
  }
}
