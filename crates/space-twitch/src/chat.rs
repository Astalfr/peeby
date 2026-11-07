//! A live's chat source: `twitch::chat`'s anonymous connection, its events turned into
//! `ui::chat`'s and given to the panel in one batch per wake-up. No UI of its own: the watch page
//! owns the panel and the task, and dropping the task closes the connection.
//!
//! Once the room's id arrives, the channel's extras load ([`Extras`]): 7TV and BTTV emotes, which
//! replace the words they name (BTTV's global set, 7TV's, then the channel's BTTV and 7TV sets,
//! each winning over the ones before; animated ones play, zero-width ones are drawn over the emote
//! before them), and the chat badges (global, then the channel's over them). Both are
//! kept for the session ([`ChatSets`]). Messages that arrived before keep their words and no badges.
//! The 7TV and BTTV sets also fill the panel's picker.
//!
//! With the auth-token, [`start_writer`] signs in a second connection that sends what the viewer
//! writes (ADR 0023); the reader shows those messages when Twitch relays them to it.
//!
//! The debug builds' fake chat (`super::fake_chat`) feeds its generated lines through the same
//! conversions ([`event`], [`writer_event`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;

use gpui::{App, Entity, Global, Hsla, SharedString, Subscription, Task};
use twitch::catalog::Catalog;
use twitch::chat::{
  ChatLine, Event, Piece, RoomState, Status, Update, Writer, WriterStatus, WriterUpdate, emote_url,
};
use twitch::helix;
use twitch::{bttv, seventv};
use ui::chat::{
  Author, Badge, ChatEvent, ChatMessage, ChatOption, ChatPanel, ChatPanelEvent, Composing,
  EMOTE_HEIGHT, Emote, EmoteGroup, Fragment, MessageKind, split_text,
};
use ui::i18n::t;
use ui::picture::Picture;
use ui::{runtime, tf};

/// Updates given to the panel at most at once, so that a burst does not stall a frame.
pub(super) const BATCH: usize = 200;

/// The id of the gear's "Reload 7TV emotes" option.
pub(super) const RELOAD_SEVENTV: &str = "reload-7tv";

/// 7TV and BTTV emotes by the word they replace.
pub(super) type Words = HashMap<String, Emote>;
/// Badges by set and version.
pub(super) type Badges = HashMap<(String, String), Rc<helix::Badge>>;

/// What a channel adds to its messages.
#[derive(Default)]
pub(super) struct Extras {
  pub(super) words: Words,
  pub(super) badges: Badges,
  /// The 7TV and BTTV sets as the picker shows them: the channel's first.
  pub(super) picker: Vec<EmoteGroup>,
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

/// What a running chat loaded, so that the page can load it again from the chat's options
/// ([`start_options`], [`reload_seventv`]). It is handed out by [`start`] and shares the task's
/// own state: the task keeps writing to it while it runs, and a clone kept after it ends only
/// reads stale data.
#[derive(Clone, Default)]
pub struct ChatExtras {
  /// The room's Twitch user id, once the connection has said it.
  room_id: Rc<RefCell<Option<String>>>,
  /// What the messages are built with.
  extras: Rc<RefCell<Rc<Extras>>>,
}

/// Connects to `login`'s chat and feeds `panel` until the task is dropped; the handle gives the
/// chat's options what they need to load the channel's extras again.
pub fn start(
  login: &str,
  client: twitch::Client,
  catalog: Catalog,
  panel: &Entity<ChatPanel>,
  cx: &mut App,
) -> (Task<()>, ChatExtras) {
  let (mut updates, run) = twitch::chat::connect(login);
  // The connection runs on the network runtime and ends when `updates` is dropped.
  drop(runtime::spawn(cx, run));
  let panel = panel.downgrade();
  let handle = ChatExtras::default();
  let shared = handle.clone();
  let task = cx.spawn(async move |cx| {
    let extras = shared.extras.clone();
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
        *shared.room_id.borrow_mut() = Some(room_id.clone());
        let (client, catalog, extras) = (client.clone(), catalog.clone(), extras.clone());
        let panel = panel.clone();
        cx.update(|cx| {
          let load = load_extras(room_id, client, catalog, extras.clone(), cx);
          cx.spawn(async move |cx| {
            load.await;
            let groups = extras.borrow().picker.clone();
            panel.update(cx, |p, cx| p.set_emotes(groups, cx)).ok();
          })
          .detach();
        });
      }
      if panel.update(cx, |p, cx| p.push(events, cx)).is_err() {
        return;
      }
    }
  });
  (task, handle)
}

/// The chat's options under the gear, left of "Send": the panel offers them and the subscription
/// does them until it is dropped. For now, reloading the channel's 7TV emotes (7TV pushes nothing
/// to us: a set changed while watching is only seen by asking again, docs/twitch.md).
pub fn start_options(
  handle: ChatExtras,
  client: twitch::Client,
  catalog: Catalog,
  panel: &Entity<ChatPanel>,
  cx: &mut App,
) -> Subscription {
  panel.update(cx, |p, cx| p.set_options(options(false), cx));
  // A second click while the first reload runs would ask 7TV twice for the same sets.
  let running = Rc::new(std::cell::Cell::new(false));
  cx.subscribe(panel, move |panel, event: &ChatPanelEvent, cx| {
    let ChatPanelEvent::Option(id) = event else {
      return;
    };
    if id.as_ref() != RELOAD_SEVENTV || running.replace(true) {
      return;
    }
    panel.update(cx, |p, cx| p.set_options(options(true), cx));
    let reload = reload_seventv(&handle, client.clone(), catalog.clone(), &panel, cx);
    let (running, panel) = (running.clone(), panel.downgrade());
    cx.spawn(async move |cx| {
      let done = reload.await;
      running.set(false);
      panel
        .update(cx, |p, cx| {
          p.set_options(options(false), cx);
          if !done {
            p.push([seventv_unchanged()], cx);
          }
        })
        .ok();
    })
    .detach();
  })
}

/// The gear's menu: the 7TV reload, greyed while it `runs`.
pub(super) fn options(runs: bool) -> Vec<ChatOption> {
  vec![
    ChatOption::new(
      RELOAD_SEVENTV,
      if runs {
        t("Reloading 7TV emotes…", "Rechargement des emotes 7TV…")
      } else {
        t("Reload 7TV emotes", "Recharger les emotes 7TV")
      },
    )
    .disabled(runs),
  ]
}

/// Asks 7TV for the global and the channel's sets again and, once both have answered, replaces
/// the ones kept for the session and builds the channel's extras from them: the next messages
/// take the new words and the picker shows them. The messages already on screen keep the emotes
/// they were drawn with.
///
/// `false` when nothing changed: the room's id is not known yet (the connection has not said it),
/// or 7TV did not answer. A failed half is not kept, so the previous emotes stay in place instead
/// of the chat losing them.
pub fn reload_seventv(
  handle: &ChatExtras,
  client: twitch::Client,
  catalog: Catalog,
  panel: &Entity<ChatPanel>,
  cx: &mut App,
) -> Task<bool> {
  let Some(room_id) = handle.room_id.borrow().clone() else {
    return Task::ready(false);
  };
  let global = runtime::spawn(cx, {
    let client = client.clone();
    async move { client.seventv_global().await }
  });
  let channel = runtime::spawn(cx, {
    let (client, id) = (client.clone(), room_id.clone());
    async move { client.seventv_channel(&id).await }
  });
  let (extras, panel) = (handle.extras.clone(), panel.downgrade());
  cx.spawn(async move |cx| {
    let (global, channel) = (global.await, channel.await);
    let (Some(Ok(global)), Some(Ok(channel))) = (global, channel) else {
      tracing::info!("chat: the 7TV emotes could not be reloaded");
      return false;
    };
    tracing::info!(
      "chat: 7TV reloaded, {} global and {} channel emotes",
      global.len(),
      channel.len()
    );
    cx.update(|cx| {
      let sets = &mut cx.default_global::<ChatSets>().seventv;
      sets.insert(String::new(), Rc::new(global));
      sets.insert(room_id.clone(), Rc::new(channel));
    });
    // Everything else comes from the session's copies: only 7TV was asked again.
    let load = cx.update(|cx| load_extras(room_id, client, catalog, extras.clone(), cx));
    load.await;
    let groups = extras.borrow().picker.clone();
    panel.update(cx, |p, cx| p.set_emotes(groups, cx)).is_ok()
  })
}

/// The line shown when the 7TV reload brought nothing (7TV unreachable, or the room's id is not
/// known yet).
fn seventv_unchanged() -> ChatEvent {
  notice_line(
    "reload-7tv",
    t(
      "The 7TV emotes could not be reloaded.",
      "Les emotes 7TV n'ont pas pu être rechargées.",
    )
    .into(),
  )
}

/// Loads (or takes from the session's cache) the global and `room_id`'s 7TV and BTTV emotes and
/// badges into `target`; the task ends once they are there.
pub(super) fn load_extras(
  room_id: String,
  client: twitch::Client,
  catalog: Catalog,
  target: Rc<RefCell<Rc<Extras>>>,
  cx: &mut App,
) -> Task<()> {
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
    let (bttv_global, seventv_global) = (bttv_global.await, seventv_global.await);
    let (bttv_channel, seventv_channel) = (bttv_channel.await, seventv_channel.await);
    let as_bttv = |set: &Option<Rc<Vec<bttv::Emote>>>| -> Vec<Emote> {
      set.iter().flat_map(|s| s.iter()).map(bttv_emote).collect()
    };
    let as_seventv = |set: &Option<Rc<Vec<seventv::Emote>>>| -> Vec<Emote> {
      set
        .iter()
        .flat_map(|s| s.iter())
        .map(seventv_emote)
        .collect()
    };
    // In the picker's order: the channel's first.
    let sets = [
      (
        t("Channel · 7TV", "Chaîne · 7TV"),
        as_seventv(&seventv_channel),
      ),
      (t("Channel · BTTV", "Chaîne · BTTV"), as_bttv(&bttv_channel)),
      ("7TV", as_seventv(&seventv_global)),
      ("BTTV", as_bttv(&bttv_global)),
    ];
    // The global sets first, so that the channel's win, and 7TV after BTTV in each.
    for (_, emotes) in sets.iter().rev() {
      for emote in emotes {
        extras.words.insert(emote.name.to_string(), emote.clone());
      }
    }
    extras.picker = sets
      .into_iter()
      .filter(|(_, emotes)| !emotes.is_empty())
      .map(|(title, emotes)| EmoteGroup {
        title: title.into(),
        emotes,
      })
      .collect();
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
}

/// Forgets the 7TV sets kept for the session, so that the next load asks 7TV again. For the fake
/// chat's own reload; a live's goes through [`reload_seventv`], which replaces them only once
/// 7TV has answered.
#[cfg(debug_assertions)]
pub(super) fn forget_seventv(cx: &mut App) {
  cx.default_global::<ChatSets>().seventv.clear();
}

/// The global 7TV and BTTV emotes (from the session's cache, else fetched), as the picker's
/// groups, 7TV first; a set that fails to load is left out. For the fake chat.
#[cfg(debug_assertions)]
pub(super) fn global_emotes(client: twitch::Client, cx: &mut App) -> Task<Vec<EmoteGroup>> {
  let seventv = set(
    String::new(),
    |s| &mut s.seventv,
    {
      let client = client.clone();
      move || async move { client.seventv_global().await }
    },
    cx,
  );
  let bttv = set(
    String::new(),
    |s| &mut s.bttv,
    move || async move { client.bttv_global().await },
    cx,
  );
  cx.spawn(async move |_| {
    let seventv: Vec<Emote> = seventv
      .await
      .iter()
      .flat_map(|s| s.iter())
      .map(seventv_emote)
      .collect();
    let bttv: Vec<Emote> = bttv
      .await
      .iter()
      .flat_map(|s| s.iter())
      .map(bttv_emote)
      .collect();
    [("7TV", seventv), ("BTTV", bttv)]
      .into_iter()
      .filter(|(_, emotes)| !emotes.is_empty())
      .map(|(title, emotes)| EmoteGroup {
        title: title.into(),
        emotes,
      })
      .collect()
  })
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
pub(super) fn event(update: Update, room: &mut RoomState, extras: &Extras) -> Option<ChatEvent> {
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
    Update::Event(Event::ServerNotice(id)) => notice_line(&id, notice_text(&id)),
    Update::Event(Event::Ping(_) | Event::Reconnect | Event::Joined) => return None,
  })
}

/// A line of Twitch itself in the chat, with `text`.
pub(super) fn notice_line(id: &str, text: String) -> ChatEvent {
  let mut notice = ChatMessage::new(format!("notice-{id}"), None, Vec::new());
  notice.kind = MessageKind::Notice(text.into());
  ChatEvent::Message(notice)
}

/// What a `NOTICE`'s `msg-id` tells the viewer; an unknown one shows as is.
pub(super) fn notice_text(id: &str) -> String {
  let text = match id {
    "msg_slowmode" => t(
      "Slow mode: wait before sending another message.",
      "Mode lent : attendez avant d'envoyer un autre message.",
    ),
    "msg_followersonly" | "msg_followersonly_zero" | "msg_followersonly_followed" => t(
      "This chat is for followers only.",
      "Ce chat est réservé aux followers.",
    ),
    "msg_subsonly" => t(
      "This chat is for subscribers only.",
      "Ce chat est réservé aux abonnés.",
    ),
    "msg_emoteonly" => t(
      "This chat accepts emotes only.",
      "Ce chat n'accepte que des emotes.",
    ),
    "msg_r9k" => t(
      "Unique mode: this message was already sent.",
      "Mode messages uniques : ce message a déjà été envoyé.",
    ),
    "msg_duplicate" => t(
      "You already sent this message less than 30 seconds ago.",
      "Vous avez déjà envoyé ce message il y a moins de 30 secondes.",
    ),
    "msg_ratelimit" => t(
      "You're sending messages too quickly.",
      "Vous envoyez des messages trop vite.",
    ),
    "msg_banned" => t(
      "You're banned from this chat.",
      "Vous êtes banni de ce chat.",
    ),
    "msg_timedout" => t(
      "You're timed out in this chat for now.",
      "Vous êtes exclu temporairement de ce chat.",
    ),
    "msg_verified_email" | "msg_requires_verified_phone_number" => t(
      "This chat asks for a verified account: see twitch.tv.",
      "Ce chat demande un compte vérifié : voir twitch.tv.",
    ),
    "msg_rejected" | "msg_rejected_mandatory" => t(
      "Your message was held by the channel's moderation.",
      "Votre message a été retenu par la modération de la chaîne.",
    ),
    "msg_channel_suspended" => t("This channel is suspended.", "Cette chaîne est suspendue."),
    "msg_suspended" => t("Your account is suspended.", "Votre compte est suspendu."),
    id => return tf!("Message from Twitch: {id}", "Message de Twitch : {id}"),
  };
  text.to_owned()
}

/// Sends what the viewer writes in `panel` to `channel`'s chat as the auth-token's account
/// (`login` gives it, called out of any update; read from Twitch when it gives none), until the
/// task and the subscription are dropped. Without the auth-token, the panel's field is off with
/// how to get one.
pub fn start_writer(
  channel: &str,
  client: twitch::Client,
  login: impl FnOnce(&mut App) -> Option<String> + 'static,
  panel: &Entity<ChatPanel>,
  cx: &mut App,
) -> Option<(Task<()>, Subscription)> {
  if !client.signed_in() {
    panel.update(cx, |p, cx| p.set_composing(no_token(), cx));
    return None;
  }
  let writer: Rc<RefCell<Option<Writer>>> = Rc::default();
  let subscription = cx.subscribe(panel, {
    let writer = writer.clone();
    move |panel, event: &ChatPanelEvent, cx| {
      let ChatPanelEvent::Send(text) = event else {
        return;
      };
      if !writer.borrow().as_ref().is_some_and(|w| w.send(text)) {
        panel.update(cx, |p, cx| p.push([unsent()], cx));
      }
    }
  });
  let channel = channel.to_owned();
  let panel = panel.downgrade();
  let task = cx.spawn(async move |cx| {
    let login = match cx.update(login) {
      Some(login) => login,
      None => {
        let client = client.clone();
        let fetch = cx.update(|cx| runtime::spawn(cx, async move { client.current_user().await }));
        match fetch.await {
          Some(Ok(login)) => login,
          Some(Err(e)) => {
            tracing::warn!("Twitch chat writer: no account: {e}");
            let off = t(
              "Twitch didn't give the account: the chat can't be written to.",
              "Twitch n'a pas donné le compte : impossible d'écrire dans le chat.",
            );
            panel
              .update(cx, |p, cx| p.set_composing(Composing::Off(off.into()), cx))
              .ok();
            return;
          }
          None => return,
        }
      }
    };
    let Some((sender, mut updates, run)) = client.chat_writer(&channel, &login) else {
      return;
    };
    // The connection runs on the network runtime and ends when `updates` and the writer go.
    drop(cx.update(|cx| runtime::spawn(cx, run)));
    *writer.borrow_mut() = Some(sender);
    while let Some(update) = updates.recv().await {
      let (state, line) = writer_event(update);
      let alive = panel.update(cx, |p, cx| {
        if let Some(state) = state {
          p.set_composing(state, cx);
        }
        if let Some(line) = line {
          p.push([line], cx);
        }
      });
      if alive.is_err() {
        return;
      }
    }
  });
  Some((task, subscription))
}

/// The field's state without the auth-token.
pub(super) fn no_token() -> Composing {
  Composing::Off(
    t(
      "Add the Twitch site token in the settings to chat.",
      "Ajoutez le jeton du site Twitch dans les réglages pour discuter.",
    )
    .into(),
  )
}

/// The line shown when a message could not even be queued (empty once cleaned, queue full).
pub(super) fn unsent() -> ChatEvent {
  notice_line(
    "unsent",
    t("The message wasn't sent.", "Le message n'est pas parti.").into(),
  )
}

/// The panel's state and line for a writer's update.
pub(super) fn writer_event(update: WriterUpdate) -> (Option<Composing>, Option<ChatEvent>) {
  let waiting = |text: String| Some(Composing::Waiting(text.into()));
  match update {
    WriterUpdate::Status(WriterStatus::Connecting) => (
      waiting(t("Signing in to the chat…", "Connexion au chat en cours…").into()),
      None,
    ),
    WriterUpdate::Status(WriterStatus::Ready) => (Some(Composing::Ready), None),
    WriterUpdate::Status(WriterStatus::Retrying(seconds)) => (
      waiting(tf!(
        "Can't send for now · retrying in {seconds} s",
        "Envoi impossible pour l'instant · nouvel essai dans {seconds} s"
      )),
      None,
    ),
    WriterUpdate::Status(WriterStatus::Refused) => (
      Some(Composing::Off(
        t(
          "Twitch refused the site token for the chat: add it again in the settings.",
          "Twitch a refusé le jeton du site pour le chat : ajoutez-le de nouveau dans les réglages.",
        )
        .into(),
      )),
      None,
    ),
    WriterUpdate::Notice(id) => (None, Some(notice_line(&id, notice_text(&id)))),
    WriterUpdate::RateLimited => (
      None,
      Some(notice_line("local-ratelimit", notice_text("msg_ratelimit"))),
    ),
  }
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
        modifiers: Default::default(),
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
  // BTTV's modifiers written just before the next word, and where their text starts in `plain`:
  // an emote takes them and their text goes, another word leaves them as text.
  let mut prefixes: Vec<(&str, bttv::Modifier)> = Vec::new();
  let mut prefixes_at = 0;
  for word in text.split_inclusive(' ') {
    let bare = word.trim_end_matches(' ');
    let space = &word[bare.len()..];
    if let Some(modifier) = bttv::Modifier::prefix(bare) {
      if prefixes.is_empty() {
        prefixes_at = plain.len();
      }
      prefixes.push((bare, modifier));
      plain.push_str(word);
      continue;
    }
    if let Some(emote) = words.get(bare) {
      let mut emote = emote.clone();
      if !prefixes.is_empty() {
        plain.truncate(prefixes_at);
        for (code, modifier) in prefixes.drain(..) {
          if modifier == bttv::Modifier::ZeroSpace {
            // Against what comes before it.
            if plain.ends_with(' ') {
              plain.pop();
            }
          }
          modify(&mut emote, code, modifier);
        }
      }
      fragments.extend(split_text(&std::mem::take(&mut plain)));
      fragments.push(Fragment::Emote(emote));
      plain.push_str(space);
      continue;
    }
    prefixes.clear();
    // FFZ's modifiers go with the emote just before them (only blanks between): their word goes.
    if let Some(modifier) = bttv::Modifier::suffix(bare)
      && plain.trim().is_empty()
      && let Some(Fragment::Emote(emote)) = fragments.last_mut()
    {
      modify(emote, bare, modifier);
      plain.clear();
      plain.push_str(space);
      continue;
    }
    plain.push_str(word);
  }
  fragments.extend(split_text(&plain));
  fragments
}

/// `modifier` (written `code`) done to `emote`. Of the turns, the strongest wins, as in BTTV's
/// style sheet where the last rule sets `transform`: a quarter turn, then a vertical flip, then
/// a horizontal one; the party's colors win over the cursed ones the same way.
fn modify(emote: &mut Emote, code: &str, modifier: bttv::Modifier) {
  use bttv::Modifier as M;
  use ui::images::{Tint, Turn};
  let rank = |turn: Turn| match turn {
    Turn::None => 0,
    Turn::FlipX => 1,
    Turn::FlipY => 2,
    Turn::Left => 3,
    Turn::Right => 4,
  };
  let effects = &mut emote.modifiers.effects;
  let mut turn = |to: Turn| {
    if rank(to) > rank(effects.turn) {
      effects.turn = to;
    }
  };
  match modifier {
    M::Wide => emote.modifiers.wide = true,
    M::FlipX => turn(Turn::FlipX),
    M::FlipY => turn(Turn::FlipY),
    M::RotateLeft => turn(Turn::Left),
    M::RotateRight => turn(Turn::Right),
    M::ZeroSpace => {}
    M::Cursed => {
      if effects.tint != Tint::Party {
        effects.tint = Tint::Cursed;
      }
    }
    M::Party => effects.tint = Tint::Party,
    M::Shake => emote.modifiers.shake = true,
  }
  emote.modifiers.codes.push(code.to_owned().into());
}

fn seventv_emote(emote: &seventv::Emote) -> Emote {
  let urls = emote.clone();
  Emote {
    name: emote.name.clone().into(),
    picture: Picture::sized(move |width, _| urls.url(width, false).unwrap_or_default()),
    aspect: emote.width as f32 / emote.height.max(1) as f32,
    overlay: emote.zero_width,
    modifiers: Default::default(),
  }
}

fn bttv_emote(emote: &bttv::Emote) -> Emote {
  let urls = emote.clone();
  Emote {
    name: emote.name.clone().into(),
    picture: Picture::sized(move |_, height| urls.url(scale(height))),
    aspect: emote.width as f32 / emote.height.max(1) as f32,
    overlay: emote.zero_width,
    modifiers: Default::default(),
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
  fn the_gear_offers_the_seventv_reload_and_greys_it_while_it_runs() {
    let ready = options(false);
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id.as_ref(), RELOAD_SEVENTV);
    assert!(!ready[0].disabled);
    let running = options(true);
    // The same id, so the panel's menu keeps its entry; a label that says it is under way.
    assert_eq!(running[0].id, ready[0].id);
    assert!(running[0].disabled);
    assert_ne!(running[0].label, ready[0].label);
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
  fn bttv_modifiers_change_the_emote_beside_them_and_disappear() {
    use ui::images::{Tint, Turn};
    let bttv = r#"[{"id":"b","code":"D:"},{"id":"c","code":"Pog"}]"#;
    let mut words: Words = Words::new();
    for emote in bttv::parse_global(bttv).expect("a set") {
      words.insert(emote.name.clone(), bttv_emote(&emote));
    }
    let shown = |text: &str| -> Vec<String> {
      with_words(text, &words)
        .iter()
        .map(|f| match f {
          Fragment::Text(t) => format!("t:{t}"),
          Fragment::Emote(e) => format!("e:{}:{}", e.name, e.modifiers.codes.join("+")),
          _ => "other".into(),
        })
        .collect()
    };
    // Prefixes before an emote, a suffix after it; the modifiers' words go with their space.
    assert_eq!(
      shown("a w! h! Pog ffzY b"),
      ["t:a ", "e:Pog:w!+h!+ffzY", "t: b"]
    );
    // Not before an emote: plain text, as BTTV leaves them.
    assert_eq!(shown("w! hello ffzX"), ["t:w! hello ffzX"]);
    assert_eq!(shown("h! w!"), ["t:h! w!"]);
    // `z!` takes the space before it away.
    assert_eq!(shown("Pog z! D:"), ["e:Pog:", "e:D::z!"]);
    let fragments = with_words("v! h! r! c! p! s! Pog", &words);
    let Some(Fragment::Emote(pog)) = fragments.first() else {
      panic!("an emote");
    };
    // The quarter turn wins over the flips, the party over the cursed colors.
    assert_eq!(pog.modifiers.effects.turn, Turn::Right);
    assert_eq!(pog.modifiers.effects.tint, Tint::Party);
    assert!(pog.modifiers.shake && !pog.modifiers.wide);
  }

  #[test]
  fn notices_read() {
    assert_eq!(
      notice_text("msg_slowmode"),
      "Slow mode: wait before sending another message."
    );
    assert_eq!(notice_text("whatever"), "Message from Twitch: whatever");
    let (state, line) = writer_event(WriterUpdate::RateLimited);
    assert!(state.is_none());
    assert!(matches!(
      line,
      Some(ChatEvent::Message(ChatMessage {
        kind: MessageKind::Notice(_),
        ..
      }))
    ));
    assert_eq!(
      writer_event(WriterUpdate::Status(WriterStatus::Ready)).0,
      Some(Composing::Ready)
    );
  }

  #[test]
  fn colors_and_scales() {
    assert_eq!(color("#FF0000"), Some(gpui::rgb(0xff0000).into()));
    assert_eq!(color("red"), None);
    assert_eq!(color("#FFF"), None);
    assert_eq!((scale(28), scale(42), scale(56), scale(84)), (1, 2, 2, 3));
  }
}
