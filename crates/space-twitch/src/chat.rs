//! A live's chat source: `twitch::chat`'s anonymous connection, its events turned into
//! `ui::chat`'s and given to the panel in one batch per wake-up. No UI of its own: the watch page
//! owns the panel and the task, and dropping the task closes the connection.
//!
//! Once the room's id arrives, the channel's extras load ([`Extras`]): 7TV and BTTV emotes, which
//! replace the words they name (BTTV's global set, 7TV's, then the channel's BTTV and 7TV sets,
//! each winning over the ones before; animated ones play, zero-width ones are drawn over the emote
//! before them), and the chat badges (global, then the channel's over them). Both are
//! kept for the session ([`ChatSets`]). The chat's messages wait for them ("Loading emotes…"), so
//! that it opens with its emotes; past [`EXTRAS_WAIT`] they show without, and only the next ones get
//! them. The VOD chat (`super::replay`) does not wait.
//! The 7TV and BTTV sets also fill the panel's picker, and 7TV's EventAPI keeps its sets current
//! while the chat is open ([`watch_seventv`], ADR 0030). The same connection brings the paints and
//! badges of the chat's 7TV users, which the panel puts on all their lines (ADR 0031).
//!
//! With the auth-token, [`start_writer`] signs in a second connection that sends what the viewer
//! writes (ADR 0023); the reader shows those messages when Twitch relays them to it.
//!
//! The debug builds' fake chat (`super::fake_chat`) feeds its generated lines through the same
//! conversions ([`event`], [`writer_event`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::Poll;
use std::time::Duration;

use gpui::{App, Entity, Global, Hsla, SharedString, Subscription, Task, WeakEntity};
use twitch::catalog::Catalog;
use twitch::chat::{
  ChatLine, Event, Piece, RoomState, Status, Update, Writer, WriterStatus, WriterUpdate, emote_url,
  gif_link, gif_url,
};
use twitch::helix;
use twitch::seventv::cosmetics::{self, Cosmetic, Entitlement, Fill};
use twitch::seventv::events::{self, Topic};
use twitch::{bttv, seventv};
use ui::chat::{
  Author, AuthorStyle, Badge, ChatEvent, ChatMessage, ChatOption, ChatPanel, ChatPanelEvent,
  Composing, EMOTE_HEIGHT, Emote, EmoteGroup, Fragment, Gif, MessageKind, UserLookup, UserProfile,
  UserQuery, split_text,
};
use ui::i18n::t;
use ui::picture::Picture;
use ui::{runtime, tf};

/// Updates given to the panel at most at once, so that a burst does not stall a frame.
pub(super) const BATCH: usize = 200;

/// How long a live's first messages wait for the channel's emotes and badges ([`load_extras`])
/// before they show without them: 7TV or BTTV may hang, the chat must not.
const EXTRAS_WAIT: Duration = Duration::from_secs(5);

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
  seventv: HashMap<String, Rc<seventv::Set>>,
  bttv: HashMap<String, Rc<Vec<bttv::Emote>>>,
  badges: HashMap<String, Rc<Vec<helix::Badge>>>,
  /// 7TV's paints and badges by id, as the EventAPI defined them ([`watch_seventv`]).
  cosmetics: HashMap<String, Rc<Cosmetic>>,
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
    // What the connection said before the channel's extras were loaded, `None` once they are:
    // the chat opens with its 7TV and BTTV emotes and badges rather than their plain words.
    let mut held: Option<Vec<Update>> = Some(Vec::new());
    // The extras' load and its deadline, while the messages wait for it.
    let mut loading: Option<(Task<()>, Task<()>)> = None;
    let mut _seventv_live: Option<Task<()>> = None;
    loop {
      let next = match loading.as_mut() {
        Some((load, deadline)) => race(|ctx| updates.poll_recv(ctx), load, deadline).await,
        None => Next::Update(updates.recv().await),
      };
      let first = match next {
        Next::Update(Some(update)) => update,
        Next::Update(None) => return,
        Next::Loaded | Next::Late => {
          let Some((load, _)) = loading.take() else {
            continue;
          };
          // A finished task is not polled again; a late one goes on loading for the next lines.
          let load = if matches!(next, Next::Loaded) {
            Task::ready(())
          } else {
            tracing::info!("chat: the emotes are late, the messages show without them");
            load
          };
          let (client, catalog, panel_, handle) = (
            client.clone(),
            catalog.clone(),
            panel.clone(),
            shared.clone(),
          );
          // Once the sets are known, 7TV's pushes keep them current; the task ends with this one.
          _seventv_live = Some(cx.update(|cx| {
            cx.spawn(async move |cx| {
              load.await;
              let groups = handle.extras.borrow().picker.clone();
              if panel_.update(cx, |p, cx| p.set_emotes(groups, cx)).is_err() {
                return;
              }
              let live = cx.update(|cx| watch_seventv(handle, client, catalog, panel_, cx));
              live.await;
            })
          }));
          let known = extras.borrow().clone();
          let mut events: Vec<ChatEvent> = held
            .take()
            .into_iter()
            .flatten()
            .filter_map(|update| event(update, &mut room, &known))
            .collect();
          // The status said the emotes were loading: the connection is up.
          events.push(ChatEvent::Status(None));
          while !events.is_empty() {
            let rest = events.split_off(events.len().min(BATCH));
            if panel.update(cx, |p, cx| p.push(events, cx)).is_err() {
              return;
            }
            events = rest;
          }
          continue;
        }
      };
      let mut batch = vec![first];
      while batch.len() < BATCH
        && let Ok(more) = updates.try_recv()
      {
        batch.push(more);
      }
      let known = extras.borrow().clone();
      let mut events = Vec::new();
      for update in batch {
        match (&mut held, update) {
          // The room's state carries its id, which the extras' load needs.
          (_, update @ (Update::Status(_) | Update::Event(Event::RoomState(_))))
          | (None, update) => {
            let connected = matches!(update, Update::Status(Status::Connected));
            events.extend(event(update, &mut room, &known));
            if connected && held.is_some() {
              events.push(ChatEvent::Loading(
                t("Loading emotes…", "Chargement des emotes…").into(),
              ));
            }
          }
          (Some(held), update) => held.push(update),
        }
      }
      if !extras_asked && let Some(room_id) = room.room_id.clone() {
        extras_asked = true;
        *shared.room_id.borrow_mut() = Some(room_id.clone());
        loading = Some(cx.update(|cx| {
          let load = load_extras(room_id, client.clone(), catalog.clone(), extras.clone(), cx);
          (load, cx.background_executor().timer(EXTRAS_WAIT))
        }));
      }
      if !events.is_empty() && panel.update(cx, |p, cx| p.push(events, cx)).is_err() {
        return;
      }
    }
  });
  (task, handle)
}

/// What a live's chat task wakes up for while its messages wait for the channel's extras.
enum Next {
  /// The connection said something (`None`: it ended).
  Update(Option<Update>),
  /// The extras are loaded.
  Loaded,
  /// [`EXTRAS_WAIT`] went by first.
  Late,
}

/// Whichever comes first: the extras' `load`, its `deadline`, or the connection's next update.
fn race<'a>(
  mut poll_update: impl FnMut(&mut std::task::Context) -> Poll<Option<Update>> + 'a,
  load: &'a mut Task<()>,
  deadline: &'a mut Task<()>,
) -> impl Future<Output = Next> + 'a {
  std::future::poll_fn(move |ctx| {
    if Pin::new(&mut *load).poll(ctx).is_ready() {
      return Poll::Ready(Next::Loaded);
    }
    if Pin::new(&mut *deadline).poll(ctx).is_ready() {
      return Poll::Ready(Next::Late);
    }
    poll_update(ctx).map(Next::Update)
  })
}

/// The chat's options under the gear, left of "Send": the panel offers them and the subscription
/// does them until it is dropped. For now, reloading the channel's 7TV emotes: 7TV's pushes
/// ([`watch_seventv`]) usually keep them current, the reload is for when that connection is down.
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
      global.emotes.len(),
      channel.emotes.len()
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

/// The EventAPI topics of the sets `room_id`'s chat shows, from the session's copies: the global
/// set, the channel's, and the channel's 7TV user (a switch to another set).
fn seventv_topics(room_id: &str, cx: &mut App) -> Vec<Topic> {
  let sets = &cx.default_global::<ChatSets>().seventv;
  // The chatters' paints and badges, whether or not the channel has 7TV emotes.
  let mut topics = vec![Topic::Channel(room_id.to_owned())];
  for set in [sets.get(""), sets.get(room_id)].into_iter().flatten() {
    topics.extend(set.id.clone().map(Topic::EmoteSet));
    topics.extend(set.owner.clone().map(Topic::User));
  }
  topics
}

/// The paint and the badge a chatter has on, by 7TV id.
#[derive(Clone, Default, PartialEq, Eq)]
struct Worn {
  paint: Option<String>,
  badge: Option<String>,
}

impl Worn {
  /// Puts on (`on`) or takes off what `entitlement` names; `true` when that changed something.
  fn wear(&mut self, entitlement: &Entitlement, on: bool) -> bool {
    let slot = match entitlement.kind {
      cosmetics::Kind::Paint => &mut self.paint,
      cosmetics::Kind::Badge => &mut self.badge,
    };
    let before = slot.clone();
    if on {
      *slot = Some(entitlement.id.clone());
    } else if slot.as_ref() == Some(&entitlement.id) {
      *slot = None;
    }
    *slot != before
  }

  /// How the panel draws it, from the session's definitions; `None` when nothing shows.
  fn style(&self, cx: &mut App) -> Option<AuthorStyle> {
    let defined = &cx.default_global::<ChatSets>().cosmetics;
    let paint = self
      .paint
      .as_ref()
      .and_then(|id| defined.get(id))
      .and_then(|c| match c.as_ref() {
        Cosmetic::Paint(paint) => name_paint(paint),
        Cosmetic::Badge(_) => None,
      });
    let badge = self
      .badge
      .as_ref()
      .and_then(|id| defined.get(id))
      .and_then(|c| match c.as_ref() {
        Cosmetic::Badge(badge) => Some(seventv_badge(badge)),
        Cosmetic::Paint(_) => None,
      });
    (paint.is_some() || badge.is_some()).then_some(AuthorStyle { paint, badge })
  }
}

/// Puts on (`on`) or takes off what `entitlement` names for its user, and redraws them when that
/// changed something; `false` once the panel is gone.
fn wear(
  worn: &mut HashMap<String, Worn>,
  entitlement: Entitlement,
  on: bool,
  panel: &WeakEntity<ChatPanel>,
  cx: &mut gpui::AsyncApp,
) -> bool {
  let user = worn.entry(entitlement.user.clone()).or_default();
  if !user.wear(&entitlement, on) {
    return true;
  }
  let style = cx.update(|cx| user.style(cx));
  push_styles(panel, vec![(entitlement.user, style)], cx).is_ok()
}

/// Gives the panel these users' looks.
fn push_styles(
  panel: &WeakEntity<ChatPanel>,
  styles: Vec<(String, Option<AuthorStyle>)>,
  cx: &mut gpui::AsyncApp,
) -> gpui::Result<()> {
  let events = styles
    .into_iter()
    .map(|(user, style)| ChatEvent::AuthorStyle(user.into(), style));
  panel.update(cx, |p, cx| p.push(events, cx))
}

/// A 7TV paint as the panel draws it: its gradient, or its image (whose colors the panel takes
/// letter by letter).
fn name_paint(paint: &cosmetics::Paint) -> Option<ui::chat::Paint> {
  let fill = match &paint.fill {
    Fill::Linear(angle) => ui::chat::PaintFill::Linear(*angle),
    Fill::Radial { circle } => ui::chat::PaintFill::Radial { circle: *circle },
    Fill::Image(url) => ui::chat::PaintFill::Image(url.clone().into()),
  };
  Some(ui::chat::Paint {
    fill,
    stops: paint
      .stops
      .iter()
      .map(|&(at, color)| (at, gpui::rgba(color)))
      .collect(),
    repeat: paint.repeat,
  })
}

fn seventv_badge(badge: &cosmetics::Badge) -> Badge {
  let urls = badge.clone();
  Badge {
    title: if badge.tooltip.is_empty() {
      "7TV".into()
    } else {
      badge.tooltip.clone().into()
    },
    picture: Picture::sized(move |width, _| urls.url(width).unwrap_or_default()),
  }
}

/// The 7TV id of `room_id`'s set in the session's copies.
fn channel_set_id(room_id: &str, cx: &mut App) -> Option<String> {
  cx.default_global::<ChatSets>()
    .seventv
    .get(room_id)
    .and_then(|set| set.id.clone())
}

/// Keeps the chat's 7TV emotes current with 7TV's EventAPI (docs/twitch.md, "7TV live updates")
/// until the task is dropped: a set change is applied to the session's copy and the words and the
/// picker are built again from it (no request), with a notice line (the channel's set or the global
/// one); a
/// [`events::Update::Resync`] asks 7TV for the sets again ([`reload_seventv`]), and a channel that
/// switched to another set is watched under its new id and says so in the chat.
fn watch_seventv(
  handle: ChatExtras,
  client: twitch::Client,
  catalog: Catalog,
  panel: WeakEntity<ChatPanel>,
  cx: &mut App,
) -> Task<()> {
  let Some(room_id) = handle.room_id.borrow().clone() else {
    return Task::ready(());
  };
  cx.spawn(async move |cx| {
    let mut topics = cx.update(|cx| seventv_topics(&room_id, cx));
    // What this chat's 7TV users have on, by Twitch user id: entitlements come per channel.
    let mut worn: HashMap<String, Worn> = HashMap::new();
    while !topics.is_empty() {
      let (mut updates, run) = events::connect(topics.clone());
      // The connection runs on the network runtime and ends when `updates` is dropped.
      cx.update(|cx| drop(runtime::spawn(cx, run)));
      let mut switched = false;
      while let Some(update) = updates.recv().await {
        match update {
          events::Update::Cosmetic(cosmetic) => {
            let id = match &cosmetic {
              Cosmetic::Paint(paint) => paint.id.clone(),
              Cosmetic::Badge(badge) => badge.id.clone(),
            };
            cx.update(|cx| {
              let defined = &mut cx.default_global::<ChatSets>().cosmetics;
              defined.insert(id.clone(), Rc::new(cosmetic));
            });
            // A redefinition (`cosmetic.update`) redraws those who have it on.
            let wearing: Vec<(String, Option<AuthorStyle>)> = cx.update(|cx| {
              worn
                .iter()
                .filter(|(_, w)| w.paint.as_ref() == Some(&id) || w.badge.as_ref() == Some(&id))
                .map(|(user, w)| (user.clone(), w.style(cx)))
                .collect()
            });
            if !wearing.is_empty() && push_styles(&panel, wearing, cx).is_err() {
              return;
            }
          }
          events::Update::Entitled(entitlement) => {
            if !wear(&mut worn, entitlement, true, &panel, cx) {
              return;
            }
          }
          events::Update::Unentitled(entitlement) => {
            if !wear(&mut worn, entitlement, false, &panel, cx) {
              return;
            }
          }
          events::Update::Set { id, changes } => {
            let scope = cx.update(|cx| apply_seventv(&room_id, &id, &changes, cx));
            // A set no chat of the session shows (a chatter's personal emotes, which the
            // channel's cosmetics bring along): nothing to redraw.
            if scope.is_none() {
              continue;
            }
            let load = cx.update(|cx| {
              load_extras(
                room_id.clone(),
                client.clone(),
                catalog.clone(),
                handle.extras.clone(),
                cx,
              )
            });
            load.await;
            let groups = handle.extras.borrow().picker.clone();
            let lines: Vec<ChatEvent> = scope
              .map(|scope| changes.iter().map(|c| seventv_changed(c, scope)).collect())
              .unwrap_or_default();
            let shown = panel.update(cx, |p, cx| {
              p.set_emotes(groups, cx);
              p.push(lines, cx);
            });
            if shown.is_err() {
              return;
            }
          }
          events::Update::Resync => {
            let before = cx.update(|cx| channel_set_id(&room_id, cx));
            let Some(reload) = cx.update(|cx| {
              let panel = panel.upgrade()?;
              Some(reload_seventv(
                &handle,
                client.clone(),
                catalog.clone(),
                &panel,
                cx,
              ))
            }) else {
              return;
            };
            reload.await;
            let after = cx.update(|cx| channel_set_id(&room_id, cx));
            if before.is_some()
              && after.is_some()
              && before != after
              && panel
                .update(cx, |p, cx| p.push([seventv_switched()], cx))
                .is_err()
            {
              return;
            }
            let now = cx.update(|cx| seventv_topics(&room_id, cx));
            if now != topics {
              topics = now;
              switched = true;
              break;
            }
          }
        }
      }
      // The connection stopped on its own (a close that trying again would repeat).
      if !switched {
        return;
      }
    }
  })
}

/// Which of a chat's 7TV sets a pushed change was for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SetScope {
  /// The channel's own set.
  Channel,
  /// 7TV's global set.
  Global,
}

/// Applies a set's changes to the session's copies that hold it; which of `room_id`'s chat sets it
/// was (the channel's when it is both), `None` when the chat shows neither.
fn apply_seventv(
  room_id: &str,
  set_id: &str,
  changes: &[events::Change],
  cx: &mut App,
) -> Option<SetScope> {
  let mut scope = None;
  for (key, set) in &mut cx.default_global::<ChatSets>().seventv {
    if set.id.as_deref() != Some(set_id) {
      continue;
    }
    let set = Rc::make_mut(set);
    for change in changes {
      set.apply(change);
    }
    if key == room_id {
      scope = Some(SetScope::Channel);
    } else if key.is_empty() && scope.is_none() {
      scope = Some(SetScope::Global);
    }
  }
  scope
}

/// A new id for each 7TV notice line: the same emote may be added, removed and added again.
fn seventv_line(kind: &str, text: String) -> ChatEvent {
  use std::sync::atomic::{AtomicU64, Ordering};
  static NEXT: AtomicU64 = AtomicU64::new(0);
  let n = NEXT.fetch_add(1, Ordering::Relaxed);
  notice_line(&format!("7tv-{kind}-{n}"), text)
}

/// The line that tells the chat a 7TV emote was added, removed or renamed, in the channel's set
/// or in 7TV's global one.
fn seventv_changed(change: &events::Change, scope: SetScope) -> ChatEvent {
  let global = scope == SetScope::Global;
  match change {
    events::Change::Added(emote) => {
      let name = &emote.name;
      let text = if global {
        tf!(
          "7TV: new global emote {name}",
          "7TV : nouvelle emote globale {name}"
        )
      } else {
        tf!(
          "7TV: new emote {name} added",
          "7TV : nouvelle emote {name} ajoutée"
        )
      };
      seventv_line("added", text)
    }
    events::Change::Removed { name, .. } => {
      let text = if global {
        tf!(
          "7TV: global emote {name} removed",
          "7TV : emote globale {name} retirée"
        )
      } else {
        tf!("7TV: emote {name} removed", "7TV : emote {name} retirée")
      };
      seventv_line("removed", text)
    }
    events::Change::Renamed { from, to, .. } => {
      let text = if global {
        tf!(
          "7TV: global emote {from} renamed {to}",
          "7TV : emote globale {from} renommée {to}"
        )
      } else {
        tf!(
          "7TV: emote {from} renamed {to}",
          "7TV : emote {from} renommée {to}"
        )
      };
      seventv_line("renamed", text)
    }
  }
}

/// The line that tells the chat the channel now uses another 7TV set (its emotes replaced).
fn seventv_switched() -> ChatEvent {
  seventv_line(
    "switched",
    t(
      "7TV: the channel switched to another emote set",
      "7TV : la chaîne utilise un autre jeu d'emotes",
    )
    .into(),
  )
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
    let as_seventv = |set: &Option<Rc<seventv::Set>>| -> Vec<Emote> {
      set
        .iter()
        .flat_map(|s| s.emotes.iter())
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
      .flat_map(|s| s.emotes.iter())
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
  cache: fn(&mut ChatSets) -> &mut HashMap<String, Rc<T>>,
  fetch: impl FnOnce() -> F,
  cx: &mut App,
) -> Task<Option<Rc<T>>>
where
  T: Send + 'static,
  F: Future<Output = twitch::Result<T>> + Send + 'static,
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
      ChatEvent::Loading(t("Connecting to chat…", "Connexion au chat…").into())
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

/// How the chat's user cards find a user's picture (IRC has none) and someone mentioned who has
/// not written: the catalog's user (anonymous GQL), by login, or by the name mentioned
/// (`@name`, a login or a display name that differs from it only by case). Answers are kept
/// for the panel's life.
pub fn user_lookup(catalog: Catalog) -> UserLookup {
  let found: Rc<RefCell<HashMap<String, Option<UserProfile>>>> = Rc::default();
  Rc::new(move |query: UserQuery, cx: &mut App| {
    let login = query
      .channel
      .as_deref()
      .unwrap_or(&query.name)
      .trim_start_matches('@')
      .to_lowercase();
    let found = found.clone();
    if let Some(known) = found.borrow().get(&login).cloned() {
      return cx.spawn(async move |_| known);
    }
    let catalog = catalog.clone();
    let wanted = login.clone();
    let fetch = runtime::spawn(cx, async move { catalog.user(&wanted).await });
    cx.spawn(async move |_| {
      let profile = match fetch.await? {
        Ok(user) => Some(UserProfile {
          name: user.display_name.into(),
          avatar: (!user.profile_image_url.is_empty())
            .then(|| crate::cards::template(&user.profile_image_url)),
          channel: Some(user.login.into()),
        }),
        // Nobody by that name: kept, so that it is not asked again.
        Err(twitch::Error::NotFound) => None,
        Err(e) => {
          tracing::debug!("chat user lookup: {e}");
          return None;
        }
      };
      found.borrow_mut().insert(login, profile.clone());
      profile
    })
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
        modifiers: Default::default(),
      })],
      Piece::Gif { id, title } => vec![Fragment::Gif(Gif {
        title: title.into(),
        link: gif_link(&id).into(),
        picture: Picture::sized(move |_, height| gif_url(&id, height)),
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
    // IRC has no picture: the card asks the catalog for it ([`user_lookup`]).
    avatar: None,
    channel: (!line.login.is_empty()).then(|| line.login.clone().into()),
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
    for emote in seventv::parse_global(set).expect("a set").emotes {
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
        Fragment::Gif(g) => format!("g:{}", g.title),
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
  fn seventv_pushes_read_as_chat_lines() {
    let text = |line: ChatEvent| match line {
      ChatEvent::Message(ChatMessage {
        kind: MessageKind::Notice(text),
        id,
        ..
      }) => (id.to_string(), text.to_string()),
      _ => panic!("a notice line"),
    };
    let set = include_str!("../../twitch/tests/fixtures/seventv_global.json");
    let ez = seventv::parse_global(set)
      .expect("a set")
      .emotes
      .into_iter()
      .find(|e| e.name == "EZ")
      .expect("EZ");
    let added = events::Change::Added(ez);
    let (first, line) = text(seventv_changed(&added, SetScope::Channel));
    assert_eq!(line, "7TV: new emote EZ added");
    // The same emote added again gets a line of its own.
    let (second, _) = text(seventv_changed(&added, SetScope::Channel));
    assert_ne!(first, second);
    assert_eq!(
      text(seventv_changed(&added, SetScope::Global)).1,
      "7TV: new global emote EZ"
    );
    let removed = events::Change::Removed {
      id: "e".into(),
      name: "EZ".into(),
    };
    assert_eq!(
      text(seventv_changed(&removed, SetScope::Channel)).1,
      "7TV: emote EZ removed"
    );
    let renamed = events::Change::Renamed {
      id: "e".into(),
      from: "EZ".into(),
      to: "EZY".into(),
    };
    assert_eq!(
      text(seventv_changed(&renamed, SetScope::Global)).1,
      "7TV: global emote EZ renamed EZY"
    );
    assert_eq!(
      text(seventv_switched()).1,
      "7TV: the channel switched to another emote set"
    );
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
