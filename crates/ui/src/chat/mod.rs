//! A live's chat, whatever the service (docs/twitch-plan.md, "Live and chat are generic"): the
//! model a space converts its protocol into ([`ChatEvent`]s of [`ChatMessage`]s), the
//! [`ChatPanel`] that shows them, what a channel pins over it ([`pinned_stack`]: predictions,
//! polls), and the watch layout with a chat column. Nothing here knows
//! IRC or InnerTube: a space owns the connection and feeds the panel.

mod emoji;
pub mod fake;
mod layout;
mod paint;
mod panel;
mod pinned;
mod user;

use std::time::SystemTime;

use gpui::{Hsla, SharedString, hsla};

use crate::picture::Picture;

pub use layout::{CHAT_WIDTH, watch_layout};
pub use paint::{Paint, PaintFill, image_colors};
pub use panel::{
  ChatOption, ChatPanel, ChatPanelEvent, Composing, EmoteGroup, MAX_MESSAGES, Viewer,
};
pub use pinned::{LEAD, OnToggle, Pinned, Unfolded, pinned_stack};
pub use user::{UserLookup, UserProfile, UserQuery, same_user};

/// One line of the chat.
#[derive(Clone)]
pub struct ChatMessage {
  /// The service's message id, for [`ChatEvent::Delete`].
  pub id: SharedString,
  /// Who wrote it; `None` for a line of the service itself.
  pub author: Option<Author>,
  /// What it says.
  pub fragments: Vec<Fragment>,
  /// When it arrived.
  pub time: SystemTime,
  /// A plain message, a `/me`, a highlighted one or a notice.
  pub kind: MessageKind,
  /// Removed by a moderator: shown greyed.
  pub deleted: bool,
}

/// The author of a message.
#[derive(Clone)]
pub struct Author {
  /// The service's user id, for [`ChatEvent::ClearUser`].
  pub id: SharedString,
  /// The name shown.
  pub name: SharedString,
  /// The color the user chose; one from [`name_color`] when `None`.
  pub color: Option<Hsla>,
  /// Shown before the name.
  pub badges: Vec<Badge>,
  /// Their picture, for their card; the space's [`UserLookup`] finds it when `None`.
  pub avatar: Option<Picture>,
  /// What opens their channel ([`ChatPanelEvent::OpenChannel`]): the service's own key (a
  /// Twitch login, a YouTube channel id); the space's [`UserLookup`] finds it when `None`.
  pub channel: Option<SharedString>,
}

/// What a third party adds to an author's look, apart from the message (7TV's cosmetics): it
/// applies to every line of theirs, the ones already shown included.
#[derive(Clone, Default)]
pub struct AuthorStyle {
  /// Colors the name, over [`Author::color`].
  pub paint: Option<Paint>,
  /// Shown after the author's own badges.
  pub badge: Option<Badge>,
}

/// A badge before an author's name (moderator, subscriber…).
#[derive(Clone)]
pub struct Badge {
  /// What it means ("Modérateur"…).
  pub title: SharedString,
  /// Its image, square.
  pub picture: Picture,
}

/// A piece of a message.
#[derive(Clone)]
pub enum Fragment {
  /// Words.
  Text(SharedString),
  /// An image in the line.
  Emote(Emote),
  /// `@name`.
  Mention(SharedString),
  /// An `http(s)` URL, shown as such and opened in the browser by a click.
  Link(SharedString),
}

/// An emote drawn inline, [`EMOTE_HEIGHT`] high; an animated image plays.
#[derive(Clone)]
pub struct Emote {
  /// Its code, shown while it loads.
  pub name: SharedString,
  /// Its image.
  pub picture: Picture,
  /// Width / height, known before it loads so that the line does not move when it arrives.
  pub aspect: f32,
  /// Drawn over the emote before it (7TV's zero-width emotes), else as an ordinary one.
  pub overlay: bool,
  /// What changes its look in this message (BTTV's modifiers, say).
  pub modifiers: Modifiers,
}

/// What a message changes in an emote's look; none by default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Modifiers {
  /// Done to its pixels: turned, tinted.
  pub effects: crate::images::Effects,
  /// Stretched to four times its height.
  pub wide: bool,
  /// Shakes, a few pixels at a time (not with reduced motion).
  pub shake: bool,
  /// The words that changed it, shown in its preview.
  pub codes: Vec<SharedString>,
}

/// Height of an emote, logical pixels (Twitch's 1.0 size).
pub const EMOTE_HEIGHT: f32 = 28.;

/// What kind of line a message is.
#[derive(Clone, Debug, PartialEq)]
pub enum MessageKind {
  /// A plain message.
  Normal,
  /// `/me`: the text in the author's color.
  Action,
  /// Picked out by the service (a highlighted or paid message).
  Highlighted,
  /// An event of the channel (a subscription, a raid…) with its text, and the author's message
  /// when there is one.
  Notice(SharedString),
}

/// What a chat source sends the panel.
#[derive(Clone)]
pub enum ChatEvent {
  /// A new message, at the end.
  Message(ChatMessage),
  /// A message removed by a moderator, by id.
  Delete(SharedString),
  /// Every message of a user removed (a ban or a timeout), by user id.
  ClearUser(SharedString),
  /// The whole chat cleared.
  Clear,
  /// The room's modes, as the labels to show ("Mode lent : 30 s"…), replacing the previous ones.
  State(Vec<SharedString>),
  /// The connection's state when it is not simply connected ("Connexion…", "Reconnexion…").
  Status(Option<SharedString>),
  /// A [`ChatEvent::Status`] that waits for something ("Connecting to chat…", "Loading
  /// emotes…"): while the chat is empty it is drawn in its middle, under the app's loader.
  Loading(SharedString),
  /// The look of the author with this user id (`None`: their own again), on all their lines.
  AuthorStyle(SharedString, Option<AuthorStyle>),
}

impl ChatMessage {
  /// A message of `author` made of `fragments`, now.
  pub fn new(
    id: impl Into<SharedString>,
    author: Option<Author>,
    fragments: Vec<Fragment>,
  ) -> Self {
    Self {
      id: id.into(),
      author,
      fragments,
      time: SystemTime::now(),
      kind: MessageKind::Normal,
      deleted: false,
    }
  }
}

/// `word` (an `@name` or a bare word, punctuation after it aside: `@name,`) names `user`, case
/// aside.
pub fn names(word: &str, user: &str) -> bool {
  let word = word.trim_end_matches(|c: char| !(c.is_alphanumeric() || c == '_'));
  same_user(word, user)
}

/// `message` mentions `user`, as Twitch highlights it: an `@` mention, or their name as a word.
pub fn mentions(message: &ChatMessage, user: &str) -> bool {
  message.fragments.iter().any(|fragment| match fragment {
    Fragment::Mention(name) => names(name, user),
    Fragment::Text(text) => text.split_whitespace().any(|word| names(word, user)),
    Fragment::Emote(_) | Fragment::Link(_) => false,
  })
}

/// `text` as fragments: `@name` words become mentions and `http(s)://` words links.
pub fn split_text(text: &str) -> Vec<Fragment> {
  let mut fragments = Vec::new();
  let mut plain = String::new();
  for word in text.split_inclusive(' ') {
    let bare = word.trim_end_matches(' ');
    let special = if bare.len() > 1 && bare.starts_with('@') {
      Some(Fragment::Mention(bare.to_owned().into()))
    } else if bare.starts_with("https://") || bare.starts_with("http://") {
      Some(Fragment::Link(bare.to_owned().into()))
    } else {
      None
    };
    match special {
      Some(fragment) => {
        if !plain.is_empty() {
          fragments.push(Fragment::Text(std::mem::take(&mut plain).into()));
        }
        fragments.push(fragment);
        if word.len() > bare.len() {
          plain.push(' ');
        }
      }
      None => plain.push_str(word),
    }
  }
  if !plain.is_empty() {
    fragments.push(Fragment::Text(plain.into()));
  }
  fragments
}

/// Colors given to authors without one of their own, picked by name.
const NAME_COLORS: [u32; 12] = [
  0xff6b63, 0xff9f43, 0xfeca57, 0x1dd1a1, 0x48dbfb, 0x54a0ff, 0x5f9ea0, 0xa29bfe, 0xff9ff3,
  0xf368e0, 0x00d2d3, 0x10ac84,
];

/// The color of an author's name: theirs made readable on the dark theme, else one picked from
/// their name (always the same one for a name).
pub fn name_color(author: &Author) -> Hsla {
  match author.color {
    Some(color) => readable(color),
    None => {
      let hash = author
        .name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
      gpui::rgb(NAME_COLORS[hash as usize % NAME_COLORS.len()]).into()
    }
  }
}

/// `color` lightened until it reads on the dark background (pure blue, black…).
pub fn readable(color: Hsla) -> Hsla {
  const LEAST: f32 = 0.35;
  if luminance(color) >= LEAST {
    return color;
  }
  let mut light = color;
  // Lightness up in steps: luminance grows with it, and 1.0 is white.
  while light.l < 1. && luminance(light) < LEAST {
    light.l = (light.l + 0.05).min(1.);
  }
  hsla(light.h, light.s, light.l, color.a)
}

/// Relative luminance (WCAG) of a color.
fn luminance(color: Hsla) -> f32 {
  let rgba = color.to_rgb();
  let channel = |c: f32| {
    if c <= 0.039_28 {
      c / 12.92
    } else {
      ((c + 0.055) / 1.055).powf(2.4)
    }
  };
  0.2126 * channel(rgba.r) + 0.7152 * channel(rgba.g) + 0.0722 * channel(rgba.b)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mentions_name_the_user_whatever_the_punctuation() {
    assert!(names("@Streamer,", "streamer"));
    assert!(names("streamer!", "Streamer"));
    assert!(names("@viewer_1", "viewer_1"));
    assert!(!names("@streamers", "streamer"));
    assert!(!names("@", "streamer"));
    let message = ChatMessage::new("1", None, split_text("hi viewer1, look"));
    assert!(mentions(&message, "Viewer1"));
    assert!(!mentions(&message, "viewer2"));
    let message = ChatMessage::new("2", None, split_text("gg @viewer2"));
    assert!(mentions(&message, "viewer2"));
  }

  fn texts(fragments: &[Fragment]) -> Vec<String> {
    fragments
      .iter()
      .map(|f| match f {
        Fragment::Text(t) => format!("t:{t}"),
        Fragment::Mention(m) => format!("m:{m}"),
        Fragment::Link(l) => format!("l:{l}"),
        Fragment::Emote(e) => format!("e:{}", e.name),
      })
      .collect()
  }

  #[test]
  fn split_text_finds_mentions_and_links() {
    assert_eq!(
      texts(&split_text("@peeby regarde https://example.com/a ici")),
      [
        "m:@peeby",
        "t: regarde ",
        "l:https://example.com/a",
        "t: ici"
      ]
    );
    assert_eq!(texts(&split_text("salut @ tout")), ["t:salut @ tout"]);
    assert!(split_text("").is_empty());
  }

  #[test]
  fn dark_names_are_lightened() {
    let blue: Hsla = gpui::rgb(0x0000ff).into();
    let readable_blue = readable(blue);
    assert!(luminance(readable_blue) >= 0.35);
    assert!((readable_blue.h - blue.h).abs() < 0.01);
    let light: Hsla = gpui::rgb(0x9acd32).into();
    assert_eq!(readable(light), light);
  }

  #[test]
  fn names_without_a_color_get_a_stable_one() {
    let author = |name: &str| Author {
      id: name.to_owned().into(),
      name: name.to_owned().into(),
      color: None,
      badges: Vec::new(),
      avatar: None,
      channel: None,
    };
    assert_eq!(
      name_color(&author("viewer1")),
      name_color(&author("viewer1"))
    );
  }
}
