//! A live's chat source: `twitch::chat`'s anonymous connection, its events turned into
//! `ui::chat`'s and given to the panel in one batch per wake-up. No UI of its own: the watch page
//! owns the panel and the task, and dropping the task closes the connection.
//!
//! Once the room's id arrives, the channel's extras load ([`Extras`]): 7TV and BTTV emotes, which
//! replace the words they name (BTTV's global set, 7TV's, then the channel's BTTV and 7TV sets,
//! each winning over the ones before; animated ones play, zero-width ones are drawn over the emote
//! before them), and the chat badges (global, then the channel's over them). Both are
//! kept for the session ([`ChatSets`]). Messages that arrived before keep their words and no badges.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use gpui::{App, Entity, Global, Hsla, SharedString, Task};
use twitch::catalog::Catalog;
use twitch::chat::{ChatLine, Event, Piece, RoomState, Status, Update, emote_url};
use twitch::helix;
use twitch::{bttv, seventv};
use ui::chat::{
  Author, Badge, ChatEvent, ChatMessage, ChatPanel, EMOTE_HEIGHT, Emote, Fragment, MessageKind,
  split_text,
};
use ui::i18n::t;
use ui::picture::Picture;
use ui::{runtime, tf};

/// Updates given to the panel at most at once, so that a burst does not stall a frame.
const BATCH: usize = 200;

/// 7TV and BTTV emotes by the word they replace.
type Words = HashMap<String, Emote>;
/// Badges by set and version.
type Badges = HashMap<(String, String), Rc<helix::Badge>>;

/// What a channel adds to its messages.
#[derive(Default)]
pub(super) struct Extras {
  words: Words,
  badges: Badges,
}

/// The sets loaded in this session: the global ones under `""`, a channel's under its Twitch user
/// id. Failed loads are not kept, so the next live tries again.
#[derive(Default)]
struct ChatSets {
  seventv: HashMap<String, Rc<Vec<seventv::Emote>>>,
  bttv: HashMap<String, Rc<Vec<bttv::Emote>>>,
  badges: HashMap<String, Rc<Vec<helix::Badge>>>,
}

impl Global for ChatSets {}

/// Connects to `login`'s chat and feeds `panel` until the task is dropped.
pub fn start(
  login: &str,
  client: twitch::Client,
  catalog: Catalog,
  panel: &Entity<ChatPanel>,
  cx: &mut App,
) -> Task<()> {
  let (mut updates, run) = twitch::chat::connect(login);
  // The connection runs on the network runtime and ends when `updates` is dropped.
  drop(runtime::spawn(cx, run));
  let panel = panel.downgrade();
  let extras: Rc<RefCell<Rc<Extras>>> = Rc::default();
  cx.spawn(async move |cx| {
    let mut room = RoomState::default();
    let mut extras_asked = false;
    while let Some(first) = updates.recv().await {
      let mut batch = vec![first];
      while batch.len() < BATCH
        && let Ok(more) = updates.try_recv()
      {
        batch.push(more);
      }
      let known = extras.borrow().clone();
      let events: Vec<ChatEvent> = batch
        .into_iter()
        .filter_map(|update| event(update, &mut room, &known))
        .collect();
      if !extras_asked && let Some(room_id) = room.room_id.clone() {
        extras_asked = true;
        let (client, catalog, extras) = (client.clone(), catalog.clone(), extras.clone());
        cx.update(|cx| load_extras(room_id, client, catalog, extras, cx));
      }
      if panel.update(cx, |p, cx| p.push(events, cx)).is_err() {
        return;
      }
    }
  })
}

/// Loads (or takes from the session's cache) the global and `room_id`'s 7TV and BTTV emotes and
/// badges into `target`.
pub(super) fn load_extras(
  room_id: String,
  client: twitch::Client,
  catalog: Catalog,
  target: Rc<RefCell<Rc<Extras>>>,
  cx: &mut App,
) {
  let bttv_global = set(
    String::new(),
    |s| &mut s.bttv,
    {
      let client = client.clone();
      move || async move { client.bttv_global().await }
    },
    cx,
  );
  let seventv_global = set(
    String::new(),
    |s| &mut s.seventv,
    {
      let client = client.clone();
      move || async move { client.seventv_global().await }
    },
    cx,
  );
  let bttv_channel = set(
    room_id.clone(),
    |s| &mut s.bttv,
    {
      let (client, id) = (client.clone(), room_id.clone());
      move || async move { client.bttv_channel(&id).await }
    },
    cx,
  );
  let seventv_channel = set(
    room_id.clone(),
    |s| &mut s.seventv,
    {
      let id = room_id.clone();
      move || async move { client.seventv_channel(&id).await }
    },
    cx,
  );
  let badges = [
    set(
      String::new(),
      |s| &mut s.badges,
      {
        let catalog = catalog.clone();
        move || async move { catalog.global_badges().await }
      },
      cx,
    ),
    set(
      room_id.clone(),
      |s| &mut s.badges,
      move || async move { catalog.channel_badges(&room_id).await },
      cx,
    ),
  ];
  cx.spawn(async move |_| {
    let mut extras = Extras::default();
    // The global sets first, so that the channel's win, and 7TV after BTTV in each.
    let words = &mut extras.words;
    for emote in bttv_global.await.iter().flat_map(|set| set.iter()) {
      words.insert(emote.name.clone(), bttv_emote(emote));
    }
    for emote in seventv_global.await.iter().flat_map(|set| set.iter()) {
      words.insert(emote.name.clone(), seventv_emote(emote));
    }
    for emote in bttv_channel.await.iter().flat_map(|set| set.iter()) {
      words.insert(emote.name.clone(), bttv_emote(emote));
    }
    for emote in seventv_channel.await.iter().flat_map(|set| set.iter()) {
      words.insert(emote.name.clone(), seventv_emote(emote));
    }
    for set in badges {
      for badge in set.await.iter().flat_map(|set| set.iter()) {
        extras.badges.insert(
          (badge.set.clone(), badge.version.clone()),
          Rc::new(badge.clone()),
        );
      }
    }
    *target.borrow_mut() = Rc::new(extras);
  })
  .detach();
}

/// The set under `key` in the session's `cache`, else fetched with `fetch` (on the network
/// runtime) and kept; `None` when it fails.
fn set<T, F>(
  key: String,
  cache: fn(&mut ChatSets) -> &mut HashMap<String, Rc<Vec<T>>>,
  fetch: impl FnOnce() -> F,
  cx: &mut App,
) -> Task<Option<Rc<Vec<T>>>>
where
  T: Send + 'static,
  F: Future<Output = twitch::Result<Vec<T>>> + Send + 'static,
{
  if let Some(set) = cache(cx.default_global::<ChatSets>()).get(&key).cloned() {
    return Task::ready(Some(set));
  }
  let fetch = runtime::spawn(cx, fetch());
  cx.spawn(async move |cx| match fetch.await {
    Some(Ok(items)) => {
      let set = Rc::new(items);
      cx.update(|cx| cache(cx.default_global::<ChatSets>()).insert(key, set.clone()));
      Some(set)
    }
    Some(Err(e)) => {
      tracing::info!("chat emotes or badges unavailable: {e}");
      None
    }
    None => None,
  })
}
/// The panel's event for a connection's update, `room` keeping the merged modes.
fn event(update: Update, room: &mut RoomState, extras: &Extras) -> Option<ChatEvent> {
  Some(match update {
    Update::Status(Status::Connecting) => {
      ChatEvent::Status(Some(t("Connecting to chat…", "Connexion au chat…").into()))
    }
    Update::Status(Status::Connected) => ChatEvent::Status(None),
    Update::Status(Status::Retrying(seconds)) => ChatEvent::Status(Some(
      tf!(
        "Chat disconnected · retrying in {seconds} s",
        "Chat déconnecté · nouvel essai dans {seconds} s"
      )
      .into(),
    )),
    Update::Event(Event::Message(line)) => ChatEvent::Message(message(&line, None, extras)),
    Update::Event(Event::Notice(line)) => {
      let text = line.system.clone().unwrap_or_default();
      ChatEvent::Message(message(&line, Some(text.into()), extras))
    }
    Update::Event(Event::Delete(id)) => ChatEvent::Delete(id.into()),
    Update::Event(Event::ClearUser(user)) => ChatEvent::ClearUser(user.into()),
    Update::Event(Event::Clear) => ChatEvent::Clear,
    Update::Event(Event::RoomState(update)) => {
      room.merge(&update);
      ChatEvent::State(modes(room))
    }
    Update::Event(Event::ServerNotice(id)) => {
      let mut notice = ChatMessage::new(format!("notice-{id}"), None, Vec::new());
      notice.kind =
        MessageKind::Notice(tf!("Message from Twitch: {id}", "Message de Twitch : {id}").into());
      ChatEvent::Message(notice)
    }
    Update::Event(Event::Ping(_) | Event::Reconnect | Event::Joined) => return None,
  })
}

/// A user's message, or a notice with `notice`'s text.
pub(super) fn message(
  line: &ChatLine,
  notice: Option<SharedString>,
  extras: &Extras,
) -> ChatMessage {
  let fragments = line
    .pieces()
    .into_iter()
    .flat_map(|piece| match piece {
      Piece::Text(text) => with_words(&text, &extras.words),
      Piece::Emote { id, name } => vec![Fragment::Emote(Emote {
        name: name.into(),
        picture: Picture::sized(move |_, height| emote_url(&id, scale(height))),
        // Twitch's own emotes are square.
        aspect: 1.,
        overlay: false,
      })],
    })
    .collect();
  let author = Author {
    id: line.user_id.clone().into(),
    name: line.display_name.clone().into(),
    color: line.color.as_deref().and_then(color),
    badges: line
      .badges
      .iter()
      .filter_map(|key| extras.badges.get(key))
      .map(badge_of)
      .collect(),
  };
  let mut message = ChatMessage::new(line.id.clone(), Some(author), fragments);
  message.kind = match notice {
    Some(text) => MessageKind::Notice(text),
    None if line.action => MessageKind::Action,
    None if line.kind.as_deref() == Some("highlighted-message") => MessageKind::Highlighted,
    None => MessageKind::Normal,
  };
  message
}

/// A badge as the panel's, its image picked for the drawn width (18, 36 or 72 px).
fn badge_of(badge: &Rc<helix::Badge>) -> Badge {
  let urls = badge.clone();
  Badge {
    title: badge.title.clone().into(),
    picture: Picture::sized(move |width, _| match width {
      0..=18 => urls.url_1x.clone(),
      19..=36 => urls.url_2x.clone(),
      _ => urls.url_4x.clone(),
    }),
  }
}

/// `text` as fragments, its words that name a 7TV or BTTV emote as that emote.
fn with_words(text: &str, words: &Words) -> Vec<Fragment> {
  if words.is_empty() {
    return split_text(text);
  }
  let mut fragments = Vec::new();
  let mut plain = String::new();
  for word in text.split_inclusive(' ') {
    let bare = word.trim_end_matches(' ');
    match words.get(bare) {
      Some(emote) => {
        fragments.extend(split_text(&std::mem::take(&mut plain)));
        fragments.push(Fragment::Emote(emote.clone()));
        plain.push_str(&word[bare.len()..]);
      }
      None => plain.push_str(word),
    }
  }
  fragments.extend(split_text(&plain));
  fragments
}

fn seventv_emote(emote: &seventv::Emote) -> Emote {
  let urls = emote.clone();
  Emote {
    name: emote.name.clone().into(),
    picture: Picture::sized(move |width, _| urls.url(width, false).unwrap_or_default()),
    aspect: emote.width as f32 / emote.height.max(1) as f32,
    overlay: emote.zero_width,
  }
}

fn bttv_emote(emote: &bttv::Emote) -> Emote {
  let urls = emote.clone();
  Emote {
    name: emote.name.clone().into(),
    picture: Picture::sized(move |_, height| urls.url(scale(height))),
    aspect: emote.width as f32 / emote.height.max(1) as f32,
    overlay: emote.zero_width,
  }
}

/// The emote scale (1, 2 or 3) for `height` device pixels.
fn scale(height: u32) -> u32 {
  let base = EMOTE_HEIGHT as u32;
  if height <= base {
    1
  } else if height <= base * 2 {
    2
  } else {
    3
  }
}

/// `#RRGGBB` as a color.
fn color(hex: &str) -> Option<Hsla> {
  let digits = hex.strip_prefix('#')?;
  (digits.len() == 6)
    .then(|| u32::from_str_radix(digits, 16).ok())
    .flatten()
    .map(|rgb| gpui::rgb(rgb).into())
}

/// The room's modes as the panel's labels.
fn modes(room: &RoomState) -> Vec<SharedString> {
  let mut labels: Vec<SharedString> = Vec::new();
  if room.emote_only == Some(true) {
    labels.push(t("Emote-only", "Emotes uniquement").into());
  }
  if room.subs_only == Some(true) {
    labels.push(t("Subscribers-only", "Abonnés uniquement").into());
  }
  match room.followers_only {
    Some(0) => labels.push(t("Followers-only", "Followers uniquement").into()),
    Some(minutes) if minutes > 0 => {
      let since = if minutes < 60 {
        format!("{minutes} min")
      } else if minutes < 24 * 60 {
        format!("{} h", minutes / 60)
      } else {
        let days = minutes / (24 * 60);
        tf!("{days} d", "{days} j")
      };
      labels.push(tf!("Followers for {since}", "Followers depuis {since}").into());
    }
    _ => {}
  }
  if let Some(seconds) = room.slow.filter(|s| *s > 0) {
    labels.push(tf!("Slow mode: {seconds} s", "Mode lent : {seconds} s").into());
  }
  if room.unique == Some(true) {
    labels.push(t("Unique messages", "Messages uniques").into());
  }
  labels
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn modes_read_as_labels() {
    let room = RoomState {
      emote_only: Some(false),
      followers_only: Some(1440),
      slow: Some(30),
      subs_only: Some(true),
      unique: Some(false),
      room_id: None,
    };
    assert_eq!(
      modes(&room),
      ["Subscribers-only", "Followers for 1 d", "Slow mode: 30 s"]
    );
    let off = RoomState {
      followers_only: Some(-1),
      slow: Some(0),
      ..RoomState::default()
    };
    assert!(modes(&off).is_empty());
  }

  #[test]
  fn seventv_and_bttv_words_become_emotes_and_keep_their_spaces() {
    let set = r#"{"emotes":[{"name":"Pog","flags":0,"data":{"id":"a","animated":false,"host":{
      "url":"//cdn.7tv.app/emote/a","files":[{"name":"1x.webp","static_name":"1x.webp",
      "width":64,"height":32,"format":"WEBP"}]}}}]}"#;
    let bttv = r#"[{"id":"b","code":"D:","animated":false},{"id":"c","code":"Pog"}]"#;
    let mut words: Words = Words::new();
    for emote in bttv::parse_global(bttv).expect("a set") {
      words.insert(emote.name.clone(), bttv_emote(&emote));
    }
    // 7TV after BTTV: its Pog wins.
    for emote in seventv::parse_global(set).expect("a set") {
      words.insert(emote.name.clone(), seventv_emote(&emote));
    }
    let fragments = with_words("so Pog @viewer Pogs Pog D:", &words);
    let shown: Vec<String> = fragments
      .iter()
      .map(|f| match f {
        Fragment::Text(t) => format!("t:{t}"),
        Fragment::Emote(e) => format!("e:{}:{}", e.name, e.aspect),
        Fragment::Mention(m) => format!("m:{m}"),
        Fragment::Link(l) => format!("l:{l}"),
      })
      .collect();
    assert_eq!(
      shown,
      [
        "t:so ",
        "e:Pog:2",
        "t: ",
        "m:@viewer",
        "t: Pogs ",
        "e:Pog:2",
        "t: ",
        "e:D::1"
      ]
    );
    assert!(with_words("", &words).is_empty());
  }

  #[test]
  fn colors_and_scales() {
    assert_eq!(color("#FF0000"), Some(gpui::rgb(0xff0000).into()));
    assert_eq!(color("red"), None);
    assert_eq!(color("#FFF"), None);
    assert_eq!((scale(28), scale(42), scale(56), scale(84)), (1, 2, 2, 3));
  }
}
