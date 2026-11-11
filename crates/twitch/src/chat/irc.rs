//! Twitch's IRC lines, with the `tags` and `commands` capabilities: a pure parser from one line to
//! an [`Event`]. Tested on captured lines (`tests/fixtures/chat_*.txt`).
//!
//! Emote ranges (`emotes=<id>:<start>-<end>,…/…`) count Unicode scalar values (Rust `char`s), not
//! bytes nor UTF-16 units: they are mapped on `char` indexes, never on byte slices, so an emoji
//! before an emote does not shift it, and a range that does not fit is dropped. GIFs
//! (`gifs=<start>-<end>|<giphy id>|<url>,…`, 2026-07) cover the text Twitch writes for them
//! (`[Yes GIF by …]`) and count the same way.

use std::collections::HashMap;

/// One IRC line, split: tags, prefix, command, parameters (the trailing one last).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
  /// Tag values, unescaped.
  pub tags: HashMap<String, String>,
  /// `nick!user@host` or `tmi.twitch.tv`.
  pub prefix: Option<String>,
  /// `PRIVMSG`, `PING`, `001`…
  pub command: String,
  /// The parameters, the trailing one (after `:`) last.
  pub params: Vec<String>,
}

impl Line {
  /// The tag's value, unescaped; `None` when absent or empty.
  pub fn tag(&self, key: &str) -> Option<&str> {
    self
      .tags
      .get(key)
      .map(String::as_str)
      .filter(|v| !v.is_empty())
  }

  /// The prefix's nick (the sender's login).
  pub fn nick(&self) -> Option<&str> {
    let prefix = self.prefix.as_deref()?;
    prefix.split_once('!').map(|(nick, _)| nick)
  }

  /// The last parameter (a message's text).
  pub fn trailing(&self) -> Option<&str> {
    self.params.last().map(String::as_str)
  }
}

/// Splits a line (without its `\r\n`); `None` when it has no command.
pub fn parse_line(line: &str) -> Option<Line> {
  let mut rest = line.trim_end_matches(['\r', '\n']);
  let mut tags = HashMap::new();
  if let Some(tagged) = rest.strip_prefix('@') {
    let (raw, after) = tagged.split_once(' ')?;
    for pair in raw.split(';') {
      let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
      tags.insert(key.to_owned(), unescape(value));
    }
    rest = after.trim_start_matches(' ');
  }
  let mut prefix = None;
  if let Some(prefixed) = rest.strip_prefix(':') {
    let (p, after) = prefixed.split_once(' ')?;
    prefix = Some(p.to_owned());
    rest = after.trim_start_matches(' ');
  }
  let (command, mut rest) = match rest.split_once(' ') {
    Some((command, after)) => (command, after),
    None => (rest, ""),
  };
  if command.is_empty() {
    return None;
  }
  let mut params = Vec::new();
  while !rest.is_empty() {
    if let Some(trailing) = rest.strip_prefix(':') {
      params.push(trailing.to_owned());
      break;
    }
    match rest.split_once(' ') {
      Some((param, after)) => {
        params.push(param.to_owned());
        rest = after.trim_start_matches(' ');
      }
      None => {
        params.push(rest.to_owned());
        break;
      }
    }
  }
  Some(Line {
    tags,
    prefix,
    command: command.to_owned(),
    params,
  })
}

/// IRCv3 tag value escapes: `\s` space, `\:` semicolon, `\\`, `\r`, `\n`.
fn unescape(value: &str) -> String {
  let mut out = String::with_capacity(value.len());
  let mut chars = value.chars();
  while let Some(c) = chars.next() {
    if c != '\\' {
      out.push(c);
      continue;
    }
    match chars.next() {
      Some('s') => out.push(' '),
      Some(':') => out.push(';'),
      Some('\\') => out.push('\\'),
      Some('r') => out.push('\r'),
      Some('n') => out.push('\n'),
      Some(other) => out.push(other),
      None => {}
    }
  }
  out
}

/// What a line means for the chat.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
  /// A user's message (`PRIVMSG`).
  Message(Box<ChatLine>),
  /// An event of the channel (`USERNOTICE`: a subscription, a raid…), its text in
  /// [`ChatLine::system`], with the user's message if any.
  Notice(Box<ChatLine>),
  /// A message removed by a moderator (`CLEARMSG`), by id.
  Delete(String),
  /// A user's messages removed, by user id (`CLEARCHAT` with a target: a ban or a timeout).
  ClearUser(String),
  /// The whole chat cleared (`CLEARCHAT` alone).
  Clear,
  /// The room's modes (`ROOMSTATE`): the full state on joining, then only what changed.
  RoomState(RoomState),
  /// The server pings: answer `PONG` with this.
  Ping(String),
  /// The server is about to restart: reconnect (`RECONNECT`).
  Reconnect,
  /// The channel is joined (`ROOMSTATE`'s first one comes right after).
  Joined,
  /// A `NOTICE` from the server (e.g. the channel does not exist), its `msg-id`.
  ServerNotice(String),
}

/// A message or a notice of a user.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChatLine {
  /// The message's id.
  pub id: String,
  /// The author's user id.
  pub user_id: String,
  /// The author's login.
  pub login: String,
  /// The name shown; the login when the tag is empty.
  pub display_name: String,
  /// `#RRGGBB`, when the user chose one.
  pub color: Option<String>,
  /// `(set, version)` pairs, in display order (`subscriber`, `12`).
  pub badges: Vec<(String, String)>,
  /// The text, without the `/me` wrapping.
  pub text: String,
  /// Sent with `/me`.
  pub action: bool,
  /// Twitch emotes in `text`, by `char` range, sorted.
  pub emotes: Vec<EmoteRange>,
  /// GIFs in `text` (Giphy's), by `char` range, sorted.
  pub gifs: Vec<EmoteRange>,
  /// The notice's text (`system-msg`), for a [`Event::Notice`].
  pub system: Option<String>,
  /// `msg-id` of a notice (`sub`, `raid`…) or of a special message (`highlighted-message`).
  pub kind: Option<String>,
  /// The user's first message in this channel.
  pub first: bool,
  /// Bits cheered with it.
  pub bits: u32,
  /// When the server got it, Unix milliseconds.
  pub sent: Option<i64>,
  /// In a Shared Chat session, the room the line was sent in when it is another one's
  /// (`source-room-id`, a channel's user id); `None` for the room's own lines.
  pub source_room: Option<String>,
  /// The author's badges in that room (`source-badges`), for a line of another room.
  pub source_badges: Option<Vec<(String, String)>>,
}

/// A Twitch emote in a message's text: `chars` indexes, end excluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmoteRange {
  /// The emote's id ([`emote_url`]).
  pub id: String,
  /// First `char`.
  pub start: usize,
  /// The `char` after the last.
  pub end: usize,
}

/// A piece of a message's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
  /// Words.
  Text(String),
  /// An emote.
  Emote {
    /// Its id ([`emote_url`]).
    id: String,
    /// Its code (the text it replaces).
    name: String,
  },
  /// A GIF, on its own.
  Gif {
    /// Its Giphy id ([`gif_url`]).
    id: String,
    /// The text it replaces: its title in brackets.
    title: String,
  },
}

/// A GIF's file on Giphy, by its id: animated WebP, 100 or 200 px high (Giphy's `fixed_height`
/// renditions, the width follows), whichever is the first at least `height` device pixels.
pub fn gif_url(id: &str, height: u32) -> String {
  let size = if height <= 100 { "100" } else { "200" };
  format!("https://media4.giphy.com/media/{id}/{size}.webp")
}

/// A GIF's page on Giphy.
pub fn gif_link(id: &str) -> String {
  format!("https://giphy.com/gifs/{id}")
}

impl ChatLine {
  /// The text cut at its emotes and GIFs; of two that overlap, the first is kept.
  pub fn pieces(&self) -> Vec<Piece> {
    let chars: Vec<char> = self.text.chars().collect();
    let mut ranges: Vec<(&EmoteRange, bool)> = self
      .emotes
      .iter()
      .map(|r| (r, false))
      .chain(self.gifs.iter().map(|r| (r, true)))
      .collect();
    ranges.sort_by_key(|(r, _)| r.start);
    let mut pieces = Vec::new();
    let mut at = 0;
    for (range, gif) in ranges {
      if range.start < at || range.end > chars.len() || range.start >= range.end {
        continue;
      }
      if range.start > at {
        pieces.push(Piece::Text(chars[at..range.start].iter().collect()));
      }
      let name: String = chars[range.start..range.end].iter().collect();
      pieces.push(if gif {
        Piece::Gif {
          id: range.id.clone(),
          title: name,
        }
      } else {
        Piece::Emote {
          id: range.id.clone(),
          name,
        }
      });
      at = range.end;
    }
    if at < chars.len() {
      pieces.push(Piece::Text(chars[at..].iter().collect()));
    }
    pieces
  }
}

/// A room's modes; `None`: not in this update.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomState {
  /// Emotes only.
  pub emote_only: Option<bool>,
  /// Minutes a user must have followed to chat; `-1`: off, `0`: any follower.
  pub followers_only: Option<i64>,
  /// Seconds between two messages of a user; `0`: off.
  pub slow: Option<u32>,
  /// Subscribers only.
  pub subs_only: Option<bool>,
  /// Unique messages only (`r9k`).
  pub unique: Option<bool>,
  /// The channel's Twitch user id (`room-id`), for its third-party emotes.
  pub room_id: Option<String>,
}

impl RoomState {
  /// `self` updated with what `update` holds.
  pub fn merge(&mut self, update: &RoomState) {
    let RoomState {
      emote_only,
      followers_only,
      slow,
      subs_only,
      unique,
      room_id,
    } = update;
    self.emote_only = emote_only.or(self.emote_only);
    self.followers_only = followers_only.or(self.followers_only);
    self.slow = slow.or(self.slow);
    self.subs_only = subs_only.or(self.subs_only);
    self.unique = unique.or(self.unique);
    if room_id.is_some() {
      self.room_id.clone_from(room_id);
    }
  }
}

/// The URL of a Twitch emote's image, at `scale` 1, 2 or 3 (28, 56, 112 px high), animated
/// when the emote is.
pub fn emote_url(id: &str, scale: u32) -> String {
  format!(
    "https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/{}.0",
    scale.clamp(1, 3)
  )
}

/// What `line` means for the chat; `None` for lines without meaning here (`CAP`, welcome numerics,
/// `USERSTATE`…).
pub fn parse(line: &str) -> Option<Event> {
  let line = parse_line(line)?;
  match line.command.as_str() {
    "PRIVMSG" => Some(Event::Message(Box::new(chat_line(&line)))),
    "USERNOTICE" => Some(Event::Notice(Box::new(chat_line(&line)))),
    "CLEARMSG" => line
      .tag("target-msg-id")
      .map(|id| Event::Delete(id.to_owned())),
    "CLEARCHAT" => Some(match line.tag("target-user-id") {
      Some(user) => Event::ClearUser(user.to_owned()),
      None => Event::Clear,
    }),
    "ROOMSTATE" => Some(Event::RoomState(room_state(&line))),
    "PING" => Some(Event::Ping(line.trailing().unwrap_or_default().to_owned())),
    "RECONNECT" => Some(Event::Reconnect),
    "JOIN" => Some(Event::Joined),
    "NOTICE" => Some(Event::ServerNotice(
      line
        .tag("msg-id")
        .or(line.trailing())
        .unwrap_or_default()
        .to_owned(),
    )),
    _ => None,
  }
}

fn chat_line(line: &Line) -> ChatLine {
  let tag = |key: &str| line.tag(key).map(str::to_owned);
  let login = tag("login")
    .or_else(|| line.nick().map(str::to_owned))
    .unwrap_or_default();
  // `PRIVMSG #channel :text`: the text is the second parameter; a notice may have none.
  let mut text = if line.params.len() >= 2 {
    line.trailing().unwrap_or_default().to_owned()
  } else {
    String::new()
  };
  let mut action = false;
  if let Some(inner) = text
    .strip_prefix("\u{1}ACTION ")
    .map(|t| t.strip_suffix('\u{1}').unwrap_or(t))
  {
    text = inner.to_owned();
    action = true;
  }
  ChatLine {
    id: tag("id").unwrap_or_default(),
    user_id: tag("user-id").unwrap_or_default(),
    display_name: tag("display-name").unwrap_or_else(|| login.clone()),
    login,
    color: tag("color"),
    badges: line
      .tag("badges")
      .map(|b| {
        b.split(',')
          .filter_map(|badge| {
            let (set, version) = badge.split_once('/')?;
            Some((set.to_owned(), version.to_owned()))
          })
          .collect()
      })
      .unwrap_or_default(),
    emotes: emotes(line.tag("emotes").unwrap_or_default()),
    gifs: gifs(line.tag("gifs").unwrap_or_default()),
    text,
    action,
    system: tag("system-msg"),
    kind: tag("msg-id"),
    first: line.tag("first-msg") == Some("1"),
    bits: line.tag("bits").and_then(|b| b.parse().ok()).unwrap_or(0),
    sent: line.tag("tmi-sent-ts").and_then(|t| t.parse().ok()),
    source_room: line
      .tag("source-room-id")
      .filter(|source| Some(*source) != line.tag("room-id"))
      .map(str::to_owned),
    source_badges: line.tag("source-badges").map(badge_list),
  }
}

/// `set/version,set/version` as pairs.
fn badge_list(tag: &str) -> Vec<(String, String)> {
  tag
    .split(',')
    .filter_map(|badge| {
      let (set, version) = badge.split_once('/')?;
      Some((set.to_owned(), version.to_owned()))
    })
    .collect()
}

/// `id:0-4,6-10/id2:12-15` as ranges sorted by start, end excluded.
fn emotes(tag: &str) -> Vec<EmoteRange> {
  let mut ranges: Vec<EmoteRange> = tag
    .split('/')
    .filter_map(|emote| emote.split_once(':'))
    .flat_map(|(id, ranges)| {
      ranges.split(',').filter_map(move |range| {
        let (start, end) = range.split_once('-')?;
        let (start, end) = (start.parse::<usize>().ok()?, end.parse::<usize>().ok()?);
        (end >= start).then(|| EmoteRange {
          id: id.to_owned(),
          start,
          end: end + 1,
        })
      })
    })
    .collect();
  ranges.sort_by_key(|r| r.start);
  ranges
}

/// `0-33|<id>|<url>,…` as ranges sorted by start, end excluded. Only the id is kept (the URL
/// carries Giphy's tracking parameters); one that is not a plain Giphy id is dropped, since it
/// goes into URLs.
fn gifs(tag: &str) -> Vec<EmoteRange> {
  let mut ranges: Vec<EmoteRange> = tag
    .split(',')
    .filter_map(|gif| {
      let mut parts = gif.split('|');
      let (start, end) = parts.next()?.split_once('-')?;
      let (start, end) = (start.parse::<usize>().ok()?, end.parse::<usize>().ok()?);
      let id = parts.next()?;
      let plain = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric());
      (plain && end >= start).then(|| EmoteRange {
        id: id.to_owned(),
        start,
        end: end + 1,
      })
    })
    .collect();
  ranges.sort_by_key(|r| r.start);
  ranges
}

fn room_state(line: &Line) -> RoomState {
  let flag = |key: &str| line.tag(key).map(|v| v == "1");
  RoomState {
    emote_only: flag("emote-only"),
    followers_only: line.tag("followers-only").and_then(|v| v.parse().ok()),
    slow: line.tag("slow").and_then(|v| v.parse().ok()),
    subs_only: flag("subs-only"),
    unique: flag("r9k"),
    room_id: line.tag("room-id").map(str::to_owned),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tags_are_unescaped() {
    assert_eq!(unescape(r"a\sb\:c\\d\re\nf\"), "a b;c\\d\re\nf");
  }

  #[test]
  fn lines_split_into_their_parts() {
    let line = parse_line("@a=1;b= :nick!nick@nick.tmi.twitch.tv PRIVMSG #chan :hello : world")
      .expect("a line");
    assert_eq!(line.tag("a"), Some("1"));
    assert_eq!(line.tag("b"), None);
    assert_eq!(line.nick(), Some("nick"));
    assert_eq!(line.command, "PRIVMSG");
    assert_eq!(line.params, ["#chan", "hello : world"]);
    let ping = parse_line("PING :tmi.twitch.tv").expect("a ping");
    assert_eq!(ping.prefix, None);
    assert_eq!(ping.params, ["tmi.twitch.tv"]);
    assert_eq!(parse_line(""), None);
  }

  #[test]
  fn emotes_count_chars_not_bytes() {
    // "é" is two bytes, "😀" four bytes and two UTF-16 units: each is one char.
    let line = ChatLine {
      text: "é😀 Kappa x".into(),
      emotes: emotes("25:3-7"),
      ..ChatLine::default()
    };
    assert_eq!(
      line.pieces(),
      [
        Piece::Text("é😀 ".into()),
        Piece::Emote {
          id: "25".into(),
          name: "Kappa".into()
        },
        Piece::Text(" x".into()),
      ]
    );
  }

  #[test]
  fn ranges_out_of_the_text_are_dropped() {
    let line = ChatLine {
      text: "Kappa".into(),
      emotes: emotes("25:0-4,3-9/1:10-12"),
      ..ChatLine::default()
    };
    assert_eq!(
      line.pieces(),
      [Piece::Emote {
        id: "25".into(),
        name: "Kappa".into()
      }]
    );
  }

  #[test]
  fn gifs_cut_their_title_out() {
    // Twitch's own sample (Chatterino's `SampleData.cpp`), its user renamed.
    let raw = concat!(
      "@badge-info=;badges=;color=#033700;display-name=Viewer1;emotes=;first-msg=0;flags=;",
      "gifs=7-40|joSNxeswxuc74Juo8X|https://media4.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif?",
      "cid=095d7a5d&ep=v1_gifs_trending&rid=giphy.gif&ct=g;id=401abf17-7e99-45d6-9bdf-43934e839327;",
      "mod=0;room-id=11148817;subscriber=0;tmi-sent-ts=1783632907018;turbo=0;user-id=1;user-type= ",
      ":viewer1!viewer1@viewer1.tmi.twitch.tv PRIVMSG #channel1 ",
      ":before [Y A Y Yes GIF by Djemilah Birnie] after"
    );
    let Some(Event::Message(line)) = parse(raw) else {
      panic!("a message");
    };
    assert_eq!(
      line.pieces(),
      [
        Piece::Text("before ".into()),
        Piece::Gif {
          id: "joSNxeswxuc74Juo8X".into(),
          title: "[Y A Y Yes GIF by Djemilah Birnie]".into()
        },
        Piece::Text(" after".into()),
      ]
    );
    assert_eq!(
      gif_url("joSNxeswxuc74Juo8X", 150),
      "https://media4.giphy.com/media/joSNxeswxuc74Juo8X/200.webp"
    );
    // An id that is not plain, or a range without one, is dropped.
    assert_eq!(gifs("0-3|a/b|x,0-3"), []);
  }

  #[test]
  fn room_states_merge() {
    let mut state = RoomState {
      slow: Some(0),
      subs_only: Some(false),
      ..RoomState::default()
    };
    state.merge(&RoomState {
      slow: Some(30),
      ..RoomState::default()
    });
    assert_eq!(state.slow, Some(30));
    assert_eq!(state.subs_only, Some(false));
  }
}
