//! A live's chat source: `twitch::chat`'s anonymous connection, its events turned into
//! `ui::chat`'s and given to the panel in one batch per wake-up. No UI of its own: the watch page
//! owns the panel and the task, and dropping the task closes the connection.

use gpui::{App, Entity, Hsla, SharedString, Task};
use twitch::chat::{ChatLine, Event, Piece, RoomState, Status, Update, emote_url};
use ui::chat::{
  Author, ChatEvent, ChatMessage, ChatPanel, EMOTE_HEIGHT, Emote, Fragment, MessageKind, split_text,
};
use ui::picture::Picture;
use ui::runtime;

/// Updates given to the panel at most at once, so that a burst does not stall a frame.
const BATCH: usize = 200;

/// Connects to `login`'s chat and feeds `panel` until the task is dropped.
pub fn start(login: &str, panel: &Entity<ChatPanel>, cx: &mut App) -> Task<()> {
  let (mut updates, run) = twitch::chat::connect(login);
  // The connection runs on the network runtime and ends when `updates` is dropped.
  drop(runtime::spawn(cx, run));
  let panel = panel.downgrade();
  cx.spawn(async move |cx| {
    let mut room = RoomState::default();
    while let Some(first) = updates.recv().await {
      let mut batch = vec![first];
      while batch.len() < BATCH
        && let Ok(more) = updates.try_recv()
      {
        batch.push(more);
      }
      let events: Vec<ChatEvent> = batch
        .into_iter()
        .filter_map(|update| event(update, &mut room))
        .collect();
      if panel.update(cx, |p, cx| p.push(events, cx)).is_err() {
        return;
      }
    }
  })
}

/// The panel's event for a connection's update, `room` keeping the merged modes.
fn event(update: Update, room: &mut RoomState) -> Option<ChatEvent> {
  Some(match update {
    Update::Status(Status::Connecting) => ChatEvent::Status(Some("Connexion au chat…".into())),
    Update::Status(Status::Connected) => ChatEvent::Status(None),
    Update::Status(Status::Retrying(seconds)) => ChatEvent::Status(Some(
      format!("Chat déconnecté · nouvel essai dans {seconds} s").into(),
    )),
    Update::Event(Event::Message(line)) => ChatEvent::Message(message(&line, None)),
    Update::Event(Event::Notice(line)) => {
      let text = line.system.clone().unwrap_or_default();
      ChatEvent::Message(message(&line, Some(text.into())))
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
fn message(line: &ChatLine, notice: Option<SharedString>) -> ChatMessage {
  let fragments = line
    .pieces()
    .into_iter()
    .flat_map(|piece| match piece {
      Piece::Text(text) => split_text(&text),
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
  fn colors_and_scales() {
    assert_eq!(color("#FF0000"), Some(gpui::rgb(0xff0000).into()));
    assert_eq!(color("red"), None);
    assert_eq!(color("#FFF"), None);
    assert_eq!((scale(28), scale(42), scale(56), scale(84)), (1, 2, 2, 3));
  }
}
