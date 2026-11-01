//! A live's chat source: `twitch::chat`'s anonymous connection, its events turned into
//! `ui::chat`'s and given to the panel in one batch per wake-up. No UI of its own: the watch page
//! owns the panel and the task, and dropping the task closes the connection.
//!
//! 7TV emotes replace the words they name: the global set and the channel's (its alias winning),
//! loaded once the room's id arrives and kept for the session ([`SevenTvSets`]). Messages that
//! arrived before keep their words. Animated ones are drawn still for now.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Entity, Global, Hsla, SharedString, Task};
use twitch::chat::{ChatLine, Event, Piece, RoomState, Status, Update, emote_url};
use twitch::seventv;
use ui::chat::{
  Author, ChatEvent, ChatMessage, ChatPanel, EMOTE_HEIGHT, Emote, Fragment, MessageKind, split_text,
};
use ui::picture::Picture;
use ui::runtime;

/// Updates given to the panel at most at once, so that a burst does not stall a frame.
const BATCH: usize = 200;

/// 7TV emotes by the word they replace.
type Words = HashMap<String, Rc<seventv::Emote>>;

/// The 7TV sets loaded in this session: the global one under `""`, a channel's under its Twitch
/// user id. Failed loads are not kept, so the next live tries again.
#[derive(Default)]
struct SevenTvSets(HashMap<String, Rc<Vec<seventv::Emote>>>);

impl Global for SevenTvSets {}

/// Connects to `login`'s chat and feeds `panel` until the task is dropped.
pub fn start(
  login: &str,
  client: twitch::Client,
  panel: &Entity<ChatPanel>,
  cx: &mut App,
) -> Task<()> {
  let (mut updates, run) = twitch::chat::connect(login);
  // The connection runs on the network runtime and ends when `updates` is dropped.
  drop(runtime::spawn(cx, run));
  let panel = panel.downgrade();
  let words: Rc<RefCell<Rc<Words>>> = Rc::default();
  cx.spawn(async move |cx| {
    let mut room = RoomState::default();
    let mut seventv_asked = false;
    while let Some(first) = updates.recv().await {
      let mut batch = vec![first];
      while batch.len() < BATCH
        && let Ok(more) = updates.try_recv()
      {
        batch.push(more);
      }
      let known = words.borrow().clone();
      let events: Vec<ChatEvent> = batch
        .into_iter()
        .filter_map(|update| event(update, &mut room, &known))
        .collect();
      if !seventv_asked && let Some(room_id) = room.room_id.clone() {
        seventv_asked = true;
        let (client, words) = (client.clone(), words.clone());
        cx.update(|cx| load_seventv(room_id, client, words, cx));
      }
      if panel.update(cx, |p, cx| p.push(events, cx)).is_err() {
        return;
      }
    }
  })
}

/// Loads (or takes from the session's cache) the global and `room_id`'s 7TV sets into `words`.
fn load_seventv(
  room_id: String,
  client: twitch::Client,
  words: Rc<RefCell<Rc<Words>>>,
  cx: &mut App,
) {
  let cached = |key: &str, cx: &App| {
    cx.try_global::<SevenTvSets>()
      .and_then(|sets| sets.0.get(key).cloned())
  };
  let sets = [String::new(), room_id].map(|key| {
    let fetch = cached(&key, cx).is_none().then(|| {
      let (client, id) = (client.clone(), key.clone());
      runtime::spawn(cx, async move {
        if id.is_empty() {
          client.seventv_global().await
        } else {
          client.seventv_channel(&id).await
        }
      })
    });
    (key, fetch)
  });
  cx.spawn(async move |cx| {
    let mut map = Words::new();
    // The global set first, so that the channel's names win.
    for (key, fetch) in sets {
      let set = match fetch {
        None => cx.update(|cx| cached(&key, cx)),
        Some(fetch) => match fetch.await {
          Some(Ok(emotes)) => {
            let set = Rc::new(emotes);
            cx.update(|cx| {
              cx.default_global::<SevenTvSets>()
                .0
                .insert(key.clone(), set.clone())
            });
            Some(set)
          }
          Some(Err(e)) => {
            tracing::info!("7TV emotes unavailable: {e}");
            None
          }
          None => None,
        },
      };
      for emote in set.iter().flat_map(|set| set.iter()) {
        map.insert(emote.name.clone(), Rc::new(emote.clone()));
      }
    }
    *words.borrow_mut() = Rc::new(map);
  })
  .detach();
}
/// The panel's event for a connection's update, `room` keeping the merged modes.
fn event(update: Update, room: &mut RoomState, words: &Words) -> Option<ChatEvent> {
  Some(match update {
    Update::Status(Status::Connecting) => ChatEvent::Status(Some("Connexion au chat…".into())),
    Update::Status(Status::Connected) => ChatEvent::Status(None),
    Update::Status(Status::Retrying(seconds)) => ChatEvent::Status(Some(
      format!("Chat déconnecté · nouvel essai dans {seconds} s").into(),
    )),
    Update::Event(Event::Message(line)) => ChatEvent::Message(message(&line, None, words)),
    Update::Event(Event::Notice(line)) => {
      let text = line.system.clone().unwrap_or_default();
      ChatEvent::Message(message(&line, Some(text.into()), words))
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
      notice.kind = MessageKind::Notice(format!("Message de Twitch : {id}").into());
      ChatEvent::Message(notice)
    }
    Update::Event(Event::Ping(_) | Event::Reconnect | Event::Joined) => return None,
  })
}

/// A user's message, or a notice with `notice`'s text.
fn message(line: &ChatLine, notice: Option<SharedString>, words: &Words) -> ChatMessage {
  let fragments = line
    .pieces()
    .into_iter()
    .flat_map(|piece| match piece {
      Piece::Text(text) => with_seventv(&text, words),
      Piece::Emote { id, name } => vec![Fragment::Emote(Emote {
        name: name.into(),
        picture: Picture::sized(move |_, height| emote_url(&id, scale(height))),
        // Twitch's own emotes are square.
        aspect: 1.,
      })],
    })
    .collect();
  let author = Author {
    id: line.user_id.clone().into(),
    name: line.display_name.clone().into(),
    color: line.color.as_deref().and_then(color),
    badges: Vec::new(),
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

/// `text` as fragments, its words that name a 7TV emote as that emote.
fn with_seventv(text: &str, words: &Words) -> Vec<Fragment> {
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
        fragments.push(seventv_fragment(emote));
        plain.push_str(&word[bare.len()..]);
      }
      None => plain.push_str(word),
    }
  }
  fragments.extend(split_text(&plain));
  fragments
}

fn seventv_fragment(emote: &Rc<seventv::Emote>) -> Fragment {
  let picture_emote = emote.clone();
  Fragment::Emote(Emote {
    name: emote.name.clone().into(),
    picture: Picture::sized(move |width, _| picture_emote.url(width, true).unwrap_or_default()),
    aspect: emote.width as f32 / emote.height.max(1) as f32,
  })
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
    labels.push("Emotes uniquement".into());
  }
  if room.subs_only == Some(true) {
    labels.push("Abonnés uniquement".into());
  }
  match room.followers_only {
    Some(0) => labels.push("Followers uniquement".into()),
    Some(minutes) if minutes > 0 => {
      let since = if minutes < 60 {
        format!("{minutes} min")
      } else if minutes < 24 * 60 {
        format!("{} h", minutes / 60)
      } else {
        format!("{} j", minutes / (24 * 60))
      };
      labels.push(format!("Followers depuis {since}").into());
    }
    _ => {}
  }
  if let Some(seconds) = room.slow.filter(|s| *s > 0) {
    labels.push(format!("Mode lent : {seconds} s").into());
  }
  if room.unique == Some(true) {
    labels.push("Messages uniques".into());
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
      [
        "Abonnés uniquement",
        "Followers depuis 1 j",
        "Mode lent : 30 s"
      ]
    );
    let off = RoomState {
      followers_only: Some(-1),
      slow: Some(0),
      ..RoomState::default()
    };
    assert!(modes(&off).is_empty());
  }

  #[test]
  fn seventv_words_become_emotes_and_keep_their_spaces() {
    let set = r#"{"emotes":[{"name":"Pog","flags":0,"data":{"id":"a","animated":false,"host":{
      "url":"//cdn.7tv.app/emote/a","files":[{"name":"1x.webp","static_name":"1x.webp",
      "width":64,"height":32,"format":"WEBP"}]}}}]}"#;
    let words: Words = seventv::parse_global(set)
      .expect("a set")
      .into_iter()
      .map(|e| (e.name.clone(), Rc::new(e)))
      .collect();
    let fragments = with_seventv("so Pog @viewer Pogs Pog", &words);
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
        "e:Pog:2"
      ]
    );
    assert!(with_seventv("", &words).is_empty());
  }

  #[test]
  fn colors_and_scales() {
    assert_eq!(color("#FF0000"), Some(gpui::rgb(0xff0000).into()));
    assert_eq!(color("red"), None);
    assert_eq!(color("#FFF"), None);
    assert_eq!((scale(28), scale(42), scale(56), scale(84)), (1, 2, 2, 3));
  }
}
