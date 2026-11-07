//! A fake live chat, in debug builds only ("Faux chat" in the Twitch space's sidebar): every
//! feature of the chat column without a channel, an account or the network. The generated lines
//! are IRC as Twitch writes it ([`feed`]) and go through the real parser and the space's
//! conversion (`chat::event`), with fake 7TV/BTTV emotes and badges drawn in memory
//! (`ui::chat::fake::Squares`, under [`SCHEME`]). The panel is the live's, with its field:
//! what is sent comes back as Twitch relays it, or is refused as the controls choose
//! (`chat::writer_event`). The channel points, predictions and polls are `points::PointsView` on
//! a local state ([`points::FakePoints`]), the predictions and polls pinned over the messages as
//! on a live: nothing is read, spent or reported on Twitch.
//!
//! The controls on the left set the rate of messages (and bursts), what they hold, the
//! moderation, the channel's events, the room's modes, the connection's state, the field's state,
//! the points, the rewards, the predictions and the polls. Only Twitch's own emotes and the real
//! global 7TV and BTTV sets (both off by default) reach the network: Twitch's CDN, 7TV and BTTV.

mod feed;
mod points;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  AnyElement, Context, ElementId, Entity, FontWeight, SharedString, Subscription, Task, WeakEntity,
  Window, div, px,
};
use gpui_component::Sizable;
use gpui_component::button::Button;
use gpui_component::{h_flex, v_flex};
use twitch::chat::{
  RATE_COUNT, RATE_WINDOW, RoomState, Status, Update, WriterStatus, WriterUpdate,
};
use twitch::helix;
use ui::chat::fake::Squares;
use ui::chat::{CHAT_WIDTH, ChatEvent, ChatPanel, ChatPanelEvent, Emote, EmoteGroup};
use ui::i18n::t;
use ui::picture::Picture;
use ui::{form, images, tf, theme};

use super::chat::{self, BATCH, Extras};
use super::points::{Backend, PointsEvent, PointsFooter, PointsView};
use super::polls;
use feed::{CHANNEL, Content, Feed, Look, ME, Notice, RealWords, THIRD_PARTY};
pub use points::FakePoints;
use twitch::raids::RaidEvent;

/// The images' URLs: drawn by [`Squares`], never downloaded.
pub const SCHEME: &str = "fake-chat://";
/// How often the feed wakes up, as a live's connection drains its queue.
const TICK: Duration = Duration::from_millis(50);
/// How long Twitch takes to relay a sent message back.
const ECHO_DELAY: Duration = Duration::from_millis(300);
/// Emotes of the picker's second set, to fill it.
const FILLER_EMOTES: usize = 60;

/// The connection's state shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Connection {
  Connected,
  Connecting,
  Retrying,
}

/// The writer's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Writer {
  Ready,
  Connecting,
  Retrying,
  Refused,
  /// No auth-token.
  NoToken,
}

/// What happens to a sent message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reply {
  /// Twitch relays it back.
  Echo,
  /// Twitch refuses it with this `msg-id`.
  Notice(&'static str),
  /// It cannot be queued.
  Unsent,
}

pub struct FakeChatView {
  panel: Entity<ChatPanel>,
  points_view: Entity<PointsView>,
  points: Rc<RefCell<FakePoints>>,
  feed: Feed,
  extras: Extras,
  /// The real global 7TV and BTTV sets used in place of the fake ones.
  real_emotes: bool,
  /// Those sets, once loaded.
  real: Option<Vec<EmoteGroup>>,
  /// They are loading.
  loading_real: bool,
  room: RoomState,
  content: Content,
  /// Messages a minute (6: one every 10 s).
  rate: u32,
  /// Messages due but not sent yet (a fraction of one between ticks).
  owed: f64,
  /// Messages of a burst still to send, [`BATCH`] at a time.
  backlog: usize,
  last_tick: Instant,
  last_second: Instant,
  connection: Connection,
  writer: Writer,
  reply: Reply,
  /// When the last messages were sent, for the writer's own limit.
  sent: VecDeque<Instant>,
  emote_only: bool,
  subs_only: bool,
  unique: bool,
  /// Minutes; -1: off.
  followers: i64,
  /// Seconds; 0: off.
  slow: u32,
  /// The fake outgoing raid, while it counts down.
  raid: Option<twitch::raids::Raid>,
  /// Fake raids announced so far, for their ids.
  made: u64,
  _send: Subscription,
  _tasks: Vec<Task<()>>,
}

impl FakeChatView {
  pub fn new(cx: &mut Context<Self>) -> Self {
    tracing::info!("fake chat: page built");
    images::route(SCHEME, Some(Arc::new(Squares)), cx);
    let extras = extras();
    let panel = cx.new(|cx| {
      let mut panel = ChatPanel::new(cx).with_composer(twitch::chat::MAX_CHARS);
      panel.set_emotes(extras.picker.clone(), cx);
      panel
    });
    let points = Rc::new(RefCell::new(FakePoints::default()));
    let points_view = cx.new(|cx| {
      PointsView::new(
        CHANNEL.into(),
        Backend::Fake(points.clone()),
        WeakEntity::new_invalid(),
        cx,
      )
    });
    // A followed raid would open the raided channel: here it only says so.
    cx.subscribe(&points_view, |this, _, event: &PointsEvent, cx| {
      let PointsEvent::Raid(login) = event;
      let line = chat::notice_line(
        "fake-raid",
        tf!(
          "Fake chat: the raid is followed, {login}'s live would open now.",
          "Faux chat : le raid est suivi, le live de {login} s'ouvrirait maintenant."
        ),
      );
      this.push(line, cx);
    })
    .detach();
    let footer = cx.new(|cx| PointsFooter::new(points_view.clone(), cx));
    panel.update(cx, |p, cx| {
      p.set_footer(footer.into(), cx);
      p.set_pinned(points_view.clone().into(), cx);
    });
    let send = cx.subscribe(&panel, |this, _, event: &ChatPanelEvent, cx| {
      let ChatPanelEvent::Send(text) = event;
      this.send(text, cx);
    });
    let now = Instant::now();
    let mut view = Self {
      panel,
      points_view,
      points,
      feed: Feed::new(0x9e37_79b9),
      extras,
      real_emotes: false,
      real: None,
      loading_real: false,
      room: RoomState::default(),
      content: Content::default(),
      rate: 300,
      owed: 0.,
      backlog: 0,
      last_tick: now,
      last_second: now,
      connection: Connection::Connecting,
      writer: Writer::Ready,
      reply: Reply::Echo,
      sent: VecDeque::new(),
      emote_only: false,
      subs_only: false,
      unique: false,
      followers: -1,
      slow: 0,
      raid: None,
      made: 0,
      _send: send,
      _tasks: Vec::new(),
    };
    // Joining as a live does: connecting, then the room's whole state.
    view.deliver(vec![Update::Status(Status::Connecting)], cx);
    view.set_writer(Writer::Ready, cx);
    view._tasks.push(cx.spawn(async move |this, cx| {
      cx.background_executor()
        .timer(Duration::from_millis(800))
        .await;
      let joined = this.update(cx, |this, cx| {
        this.set_connection(Connection::Connected, cx);
        let state = feed::room_state(&this.room_tags());
        this.deliver_lines(&[state], cx);
      });
      if joined.is_err() {
        return;
      }
      loop {
        cx.background_executor().timer(TICK).await;
        if this.update(cx, |this, cx| this.tick(cx)).is_err() {
          return;
        }
      }
    }));
    view
  }

  /// Sends what is due since the last tick; every second, the predictions move.
  fn tick(&mut self, cx: &mut Context<Self>) {
    let now = Instant::now();
    let elapsed = now - self.last_tick;
    self.last_tick = now;
    if self.connection == Connection::Connected {
      self.owed += f64::from(self.rate) / 60. * elapsed.as_secs_f64();
    }
    // Whole messages only; a burst fills the batch up to its size.
    let due = (self.owed as usize).min(BATCH);
    self.owed -= due as f64;
    let burst = self.backlog.min(BATCH - due);
    self.backlog -= burst;
    let lines: Vec<String> = (0..due + burst)
      .map(|_| self.feed.message(&self.content))
      .collect();
    if !lines.is_empty() {
      self.deliver_lines(&lines, cx);
    }
    if now - self.last_second >= Duration::from_secs(1) {
      self.last_second = now;
      self.points.borrow_mut().tick();
      self.reload_points(cx);
      self.raid_tick(cx);
    }
  }

  /// A second of the fake raid: viewers join it, and it goes at the end of its countdown.
  fn raid_tick(&mut self, cx: &mut Context<Self>) {
    let Some(raid) = self.raid.as_mut() else {
      return;
    };
    let end = raid.created_at.unwrap_or(0) + i64::try_from(raid.force_seconds).unwrap_or(0) * 1000;
    if polls::now_millis() >= end {
      if let Some(raid) = self.raid.take() {
        self.raid_event(RaidEvent::Go(raid), cx);
      }
      return;
    }
    raid.viewer_count += 7;
    let raid = raid.clone();
    self.raid_event(RaidEvent::Update(raid), cx);
  }

  /// IRC lines through the parser, then as the connection's updates.
  fn deliver_lines(&mut self, lines: &[String], cx: &mut Context<Self>) {
    let updates = lines
      .iter()
      .filter_map(|line| twitch::chat::irc::parse(line))
      .map(Update::Event)
      .collect();
    self.deliver(updates, cx);
  }

  /// The connection's updates through the space's conversion, to the panel.
  fn deliver(&mut self, updates: Vec<Update>, cx: &mut Context<Self>) {
    let events: Vec<ChatEvent> = updates
      .into_iter()
      .filter_map(|update| chat::event(update, &mut self.room, &self.extras))
      .collect();
    self.panel.update(cx, |p, cx| p.push(events, cx));
  }

  fn push(&self, event: ChatEvent, cx: &mut Context<Self>) {
    self.panel.update(cx, |p, cx| p.push([event], cx));
  }

  fn reload_points(&self, cx: &mut Context<Self>) {
    self.points_view.update(cx, |p, cx| p.reload(cx));
  }

  fn set_connection(&mut self, connection: Connection, cx: &mut Context<Self>) {
    self.connection = connection;
    let status = match connection {
      Connection::Connected => Status::Connected,
      Connection::Connecting => Status::Connecting,
      Connection::Retrying => Status::Retrying(8),
    };
    self.deliver(vec![Update::Status(status)], cx);
  }

  fn set_writer(&mut self, writer: Writer, cx: &mut Context<Self>) {
    self.writer = writer;
    let state = match writer {
      Writer::NoToken => Some(chat::no_token()),
      Writer::Ready => chat::writer_event(WriterUpdate::Status(WriterStatus::Ready)).0,
      Writer::Connecting => chat::writer_event(WriterUpdate::Status(WriterStatus::Connecting)).0,
      Writer::Retrying => chat::writer_event(WriterUpdate::Status(WriterStatus::Retrying(8))).0,
      Writer::Refused => chat::writer_event(WriterUpdate::Status(WriterStatus::Refused)).0,
    };
    if let Some(state) = state {
      self.panel.update(cx, |p, cx| p.set_composing(state, cx));
    }
  }

  /// What the viewer sent: refused as chosen, held by the writer's limit, else relayed back.
  fn send(&mut self, text: &str, cx: &mut Context<Self>) {
    let Some(clean) = twitch::chat::clean(text).filter(|_| self.writer == Writer::Ready) else {
      return self.push(chat::unsent(), cx);
    };
    // The real writer's limit: beyond it, Twitch locks the account out of chat.
    let now = Instant::now();
    while self.sent.front().is_some_and(|&at| now - at >= RATE_WINDOW) {
      self.sent.pop_front();
    }
    if self.sent.len() >= RATE_COUNT {
      if let (_, Some(line)) = chat::writer_event(WriterUpdate::RateLimited) {
        self.push(line, cx);
      }
      return;
    }
    match self.reply {
      // Never left: it does not count.
      Reply::Unsent => return self.push(chat::unsent(), cx),
      Reply::Notice(_) | Reply::Echo => self.sent.push_back(now),
    }
    match self.reply {
      Reply::Unsent => {}
      Reply::Notice(id) => {
        if let (_, Some(line)) = chat::writer_event(WriterUpdate::Notice(id.into())) {
          self.push(line, cx);
        }
      }
      Reply::Echo => {
        let (action, text) = match clean.strip_prefix("/me ") {
          Some(rest) => (true, rest.to_owned()),
          None => (false, clean),
        };
        let line = self.feed.privmsg(&ME, &text, action, None, &self.content);
        cx.spawn(async move |this, cx| {
          cx.background_executor().timer(ECHO_DELAY).await;
          this
            .update(cx, |this, cx| this.deliver_lines(&[line], cx))
            .ok();
        })
        .detach();
      }
    }
  }

  /// The room's modes as `ROOMSTATE`'s tags.
  fn room_tags(&self) -> Vec<(&'static str, String)> {
    let flag = |on: bool| if on { "1" } else { "0" }.to_owned();
    vec![
      ("emote-only", flag(self.emote_only)),
      ("followers-only", self.followers.to_string()),
      ("r9k", flag(self.unique)),
      ("slow", self.slow.to_string()),
      ("subs-only", flag(self.subs_only)),
    ]
  }

  /// One mode changed: Twitch sends only it.
  fn room_changed(&mut self, tag: &str, cx: &mut Context<Self>) {
    let tags: Vec<(&str, String)> = self
      .room_tags()
      .into_iter()
      .filter(|(key, _)| *key == tag)
      .collect();
    self.deliver_lines(&[feed::room_state(&tags)], cx);
  }

  /// Uses the real global 7TV and BTTV sets (`on`) or the fake ones; loads them the first time.
  fn set_real_emotes(&mut self, on: bool, cx: &mut Context<Self>) {
    self.real_emotes = on;
    if !on || self.real.is_some() {
      self.apply_emotes(cx);
      return;
    }
    if self.loading_real {
      return;
    }
    let client = match twitch::Client::new(None, None) {
      Ok(client) => client,
      Err(e) => {
        tracing::info!("fake chat: no client for the real emotes: {e}");
        self.real_emotes = false;
        return;
      }
    };
    self.loading_real = true;
    let load = chat::global_emotes(client, cx);
    self._tasks.push(cx.spawn(async move |this, cx| {
      let groups = load.await;
      this
        .update(cx, |this, cx| {
          this.loading_real = false;
          if groups.is_empty() {
            this.real_emotes = false;
            let line = chat::notice_line(
              "fake-emotes",
              t(
                "Fake chat: the 7TV and BTTV emotes could not be loaded.",
                "Faux chat : les emotes 7TV et BTTV n'ont pas pu être chargées.",
              )
              .into(),
            );
            this.push(line, cx);
          } else {
            this.real = Some(groups);
          }
          this.apply_emotes(cx);
          cx.notify();
        })
        .ok();
    }));
  }

  /// The words, the picker and the feed's words for the emotes in use: the fake ones, with the
  /// real global sets over them (and first in the picker) while they are on.
  fn apply_emotes(&mut self, cx: &mut Context<Self>) {
    let mut extras = extras();
    self.feed.real = None;
    if self.real_emotes
      && let Some(groups) = &self.real
    {
      // BTTV first, so that 7TV wins, as on a live.
      for group in groups.iter().rev() {
        for emote in &group.emotes {
          extras.words.insert(emote.name.to_string(), emote.clone());
        }
      }
      let (mut plain, mut overlays) = (Vec::new(), Vec::new());
      for emote in extras.words.values() {
        if groups
          .iter()
          .any(|g| g.emotes.iter().any(|e| e.name == emote.name))
        {
          let list = if emote.overlay {
            &mut overlays
          } else {
            &mut plain
          };
          list.push(emote.name.to_string());
        }
      }
      // The words' map has no order: sorted, the feed stays the same for a seed.
      plain.sort();
      overlays.sort();
      self.feed.real = Some(RealWords { plain, overlays });
      extras.picker.splice(0..0, groups.iter().cloned());
    }
    let picker = extras.picker.clone();
    self.panel.update(cx, |p, cx| p.set_emotes(picker, cx));
    self.extras = extras;
  }

  fn points_changed(&mut self, change: impl FnOnce(&mut FakePoints), cx: &mut Context<Self>) {
    change(&mut self.points.borrow_mut());
    self.reload_points(cx);
  }
}

/// The fake channel's 7TV and BTTV emotes, a picker set big enough to scroll, and the badges.
fn extras() -> Extras {
  let mut extras = Extras::default();
  let emote = |name: &str, look: Look| {
    let kind = match look {
      Look::Animated => "anim",
      Look::Overlay => "over",
      Look::Square | Look::Wide => "emote",
    };
    let path = format!("{kind}/{name}");
    Emote {
      name: name.to_owned().into(),
      picture: Picture::sized(move |w, h| format!("{SCHEME}{path}?w={w}&h={h}")),
      aspect: if look == Look::Wide { 2. } else { 1. },
      overlay: look == Look::Overlay,
      modifiers: Default::default(),
    }
  };
  let channel: Vec<Emote> = THIRD_PARTY
    .iter()
    .map(|&(name, look)| emote(name, look))
    .collect();
  let filler: Vec<Emote> = (0..FILLER_EMOTES)
    .map(|i| {
      let look = match i % 9 {
        0 => Look::Wide,
        4 => Look::Animated,
        _ => Look::Square,
      };
      emote(&format!("fake{i}"), look)
    })
    .collect();
  for e in channel.iter().chain(&filler) {
    extras.words.insert(e.name.to_string(), e.clone());
  }
  extras.picker = vec![
    EmoteGroup {
      title: t("Fake channel · 7TV", "Fausse chaîne · 7TV").into(),
      emotes: channel,
    },
    EmoteGroup {
      title: t("Many emotes", "Beaucoup d'emotes").into(),
      emotes: filler,
    },
  ];
  for &(set, version, title) in feed::BADGES {
    let url = |size: u32| format!("{SCHEME}badge/{set}/{version}?s={size}");
    extras.badges.insert(
      (set.to_owned(), version.to_owned()),
      Rc::new(helix::Badge {
        set: set.into(),
        version: version.into(),
        title: title.into(),
        url_1x: url(18),
        url_2x: url(36),
        url_4x: url(72),
      }),
    );
  }
  extras
}

/// What a control does to the page.
type Action = fn(&mut FakeChatView, &mut Context<FakeChatView>);

impl FakeChatView {
  fn switch(
    &self,
    id: &str,
    label: &str,
    checked: bool,
    set: fn(&mut Self, bool, &mut Context<Self>),
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let this = cx.entity().downgrade();
    h_flex()
      .w(px(250.))
      .gap_2()
      .items_center()
      .child(form::toggle(id, checked, move |on, cx| {
        this
          .update(cx, |this, cx| {
            set(this, on, cx);
            cx.notify();
          })
          .ok();
      }))
      .child(div().text_sm().child(label.to_owned()))
      .into_any_element()
  }

  fn button(&self, id: &str, label: String, action: Action, cx: &mut Context<Self>) -> AnyElement {
    Button::new(ElementId::Name(format!("fake-chat-{id}").into()))
      .small()
      .label(label)
      .on_click(cx.listener(move |this, _, _, cx| {
        action(this, cx);
        cx.notify();
      }))
      .into_any_element()
  }

  fn chips<T: Clone + PartialEq + 'static>(
    &self,
    id: &str,
    options: Vec<(T, String)>,
    selected: &T,
    set: fn(&mut Self, T, &mut Context<Self>),
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let this = cx.entity().downgrade();
    div()
      .child(form::chips(id, options, selected, move |value, cx| {
        this
          .update(cx, |this, cx| {
            set(this, value, cx);
            cx.notify();
          })
          .ok();
      }))
      .into_any_element()
  }

  /// A labelled line of controls.
  fn line(label: &str, controls: Vec<AnyElement>) -> AnyElement {
    h_flex()
      .gap_3()
      .items_center()
      .flex_wrap()
      .child(
        div()
          .w(px(140.))
          .flex_none()
          .text_sm()
          .text_color(theme::muted())
          .child(label.to_owned()),
      )
      .children(controls)
      .into_any_element()
  }

  /// A titled card of lines.
  fn card(title: &str, lines: Vec<AnyElement>) -> AnyElement {
    v_flex()
      .p_4()
      .gap_3()
      .rounded(theme::RADIUS)
      .bg(theme::surface())
      .child(
        div()
          .text_base()
          .font_weight(FontWeight::SEMIBOLD)
          .child(title.to_owned()),
      )
      .children(lines)
      .into_any_element()
  }

  fn feed_card(&self, cx: &mut Context<Self>) -> AnyElement {
    // A minute's messages: one every few seconds for a quiet chat, up to a busy channel's.
    let rates = [0u32, 6, 12, 30, 60, 300, 1_200, 3_000, 6_000, 18_000]
      .into_iter()
      .map(|r| {
        let label = match r {
          0 => t("Paused", "Pause").to_owned(),
          r if r < 60 => {
            let every = 60 / r;
            tf!("every {every} s", "toutes les {every} s")
          }
          r => {
            let rate = r / 60;
            tf!("{rate}/s", "{rate}/s")
          }
        };
        (r, label)
      })
      .collect();
    let connections = vec![
      (Connection::Connected, t("Connected", "Connecté").to_owned()),
      (
        Connection::Connecting,
        t("Connecting", "Connexion").to_owned(),
      ),
      (
        Connection::Retrying,
        t("Retrying", "Nouvel essai").to_owned(),
      ),
    ];
    Self::card(
      t("Feed", "Flux"),
      vec![
        Self::line(
          t("Messages", "Messages"),
          vec![self.chips("rate", rates, &self.rate, |this, r, _| this.rate = r, cx)],
        ),
        Self::line(
          t("Burst", "Rafale"),
          vec![
            self.button(
              "burst-100",
              "+100".into(),
              |this, _| this.backlog += 100,
              cx,
            ),
            self.button(
              "burst-1000",
              "+1 000".into(),
              |this, _| this.backlog += 1_000,
              cx,
            ),
            self.button(
              "burst-max",
              tf!(
                "+{} (the panel's maximum)",
                "+{} (le maximum du panneau)",
                ui::chat::MAX_MESSAGES
              ),
              |this, _| this.backlog += ui::chat::MAX_MESSAGES,
              cx,
            ),
          ],
        ),
        Self::line(
          t("Connection", "Connexion"),
          vec![self.chips(
            "connection",
            connections,
            &self.connection,
            |this, c, cx| this.set_connection(c, cx),
            cx,
          )],
        ),
      ],
    )
  }

  fn content_card(&self, cx: &mut Context<Self>) -> AnyElement {
    let c = &self.content;
    let switches = vec![
      self.switch(
        "third-party",
        t("7TV / BTTV emotes", "Emotes 7TV / BTTV"),
        c.third_party,
        |this, on, _| this.content.third_party = on,
        cx,
      ),
      self.switch(
        "real-emotes",
        if self.loading_real {
          t(
            "Real 7TV / BTTV (loading…)",
            "Vraies 7TV / BTTV (chargement…)",
          )
        } else {
          t(
            "Real 7TV / BTTV (global sets)",
            "Vraies 7TV / BTTV (sets globaux)",
          )
        },
        self.real_emotes,
        Self::set_real_emotes,
        cx,
      ),
      self.switch(
        "twitch-emotes",
        t(
          "Twitch emotes (Twitch's CDN)",
          "Emotes Twitch (CDN de Twitch)",
        ),
        c.twitch_emotes,
        |this, on, _| this.content.twitch_emotes = on,
        cx,
      ),
      self.switch(
        "badges",
        t("Badges", "Badges"),
        c.badges,
        |this, on, _| this.content.badges = on,
        cx,
      ),
      self.switch(
        "colors",
        t("Name colors", "Couleurs des noms"),
        c.colors,
        |this, on, _| this.content.colors = on,
        cx,
      ),
      self.switch(
        "mentions",
        t("Mentions", "Mentions"),
        c.mentions,
        |this, on, _| this.content.mentions = on,
        cx,
      ),
      self.switch(
        "links",
        t("Links", "Liens"),
        c.links,
        |this, on, _| this.content.links = on,
        cx,
      ),
      self.switch(
        "emoji",
        t("Emoji", "Emoji"),
        c.emoji,
        |this, on, _| this.content.emoji = on,
        cx,
      ),
      self.switch(
        "long",
        t("Long messages", "Messages longs"),
        c.long,
        |this, on, _| this.content.long = on,
        cx,
      ),
      self.switch(
        "actions",
        "/me",
        c.actions,
        |this, on, _| this.content.actions = on,
        cx,
      ),
      self.switch(
        "highlighted",
        t("Highlighted", "Mis en avant"),
        c.highlighted,
        |this, on, _| this.content.highlighted = on,
        cx,
      ),
    ];
    Self::card(
      t("Content", "Contenu"),
      vec![
        h_flex()
          .flex_wrap()
          .gap_3()
          .children(switches)
          .into_any_element(),
      ],
    )
  }

  fn events_card(&self, cx: &mut Context<Self>) -> AnyElement {
    Self::card(
      t("Events", "Événements"),
      vec![
        Self::line(
          t("Channel", "Chaîne"),
          vec![
            self.button(
              "sub",
              t("Sub", "Abonnement").into(),
              |this, cx| this.channel_event(Notice::Sub, cx),
              cx,
            ),
            self.button(
              "resub",
              t("Resub with a message", "Réabonnement avec message").into(),
              |this, cx| this.channel_event(Notice::Resub, cx),
              cx,
            ),
            self.button(
              "gift",
              t("Gifted sub", "Abonnement offert").into(),
              |this, cx| this.channel_event(Notice::Gift, cx),
              cx,
            ),
            self.button(
              "raid",
              "Raid".into(),
              |this, cx| this.channel_event(Notice::Raid, cx),
              cx,
            ),
          ],
        ),
        Self::line(
          t("Moderation", "Modération"),
          vec![
            self.button(
              "delete-last",
              t("Delete the last one", "Supprimer le dernier").into(),
              |this, cx| {
                if let Some(line) = this.feed.delete_last() {
                  this.deliver_lines(&[line], cx);
                }
              },
              cx,
            ),
            self.button(
              "delete-recent",
              t("Delete a recent one", "Supprimer un récent").into(),
              |this, cx| {
                if let Some(line) = this.feed.delete_recent() {
                  this.deliver_lines(&[line], cx);
                }
              },
              cx,
            ),
            self.button(
              "timeout",
              t("Time out an author", "Exclure un auteur").into(),
              |this, cx| {
                if let Some(line) = this.feed.timeout() {
                  this.deliver_lines(&[line], cx);
                }
              },
              cx,
            ),
            self.button(
              "clear",
              t("Clear the chat", "Vider le chat").into(),
              |this, cx| {
                let line = this.feed.clear();
                this.deliver_lines(&[line], cx);
              },
              cx,
            ),
          ],
        ),
        Self::line(
          t("Server", "Serveur"),
          vec![
            self.button(
              "suspended",
              t("Channel suspended", "Chaîne suspendue").into(),
              |this, cx| this.deliver_lines(&[feed::server_notice("msg_channel_suspended")], cx),
              cx,
            ),
            self.button(
              "unknown-notice",
              t("Unknown notice", "Notice inconnue").into(),
              |this, cx| this.deliver_lines(&[feed::server_notice("msg_peeby_unknown")], cx),
              cx,
            ),
          ],
        ),
      ],
    )
  }

  fn channel_event(&mut self, kind: Notice, cx: &mut Context<Self>) {
    let line = self.feed.notice(kind, &self.content);
    self.deliver_lines(&[line], cx);
  }

  fn room_card(&self, cx: &mut Context<Self>) -> AnyElement {
    let followers = [
      (-1i64, t("Off", "Non").to_owned()),
      (0, t("Any follower", "Tout follower").to_owned()),
      (10, "10 min".to_owned()),
      (60, "1 h".to_owned()),
      (1_440, tf!("{} d", "{} j", 1)),
    ]
    .into();
    let slow = [0u32, 3, 30, 120]
      .into_iter()
      .map(|s| {
        let label = if s == 0 {
          t("Off", "Non").to_owned()
        } else {
          format!("{s} s")
        };
        (s, label)
      })
      .collect();
    Self::card(
      t("Room modes", "Modes du salon"),
      vec![
        h_flex()
          .flex_wrap()
          .gap_3()
          .child(self.switch(
            "emote-only",
            t("Emote-only", "Emotes uniquement"),
            self.emote_only,
            |this, on, cx| {
              this.emote_only = on;
              this.room_changed("emote-only", cx);
            },
            cx,
          ))
          .child(self.switch(
            "subs-only",
            t("Subscribers-only", "Abonnés uniquement"),
            self.subs_only,
            |this, on, cx| {
              this.subs_only = on;
              this.room_changed("subs-only", cx);
            },
            cx,
          ))
          .child(self.switch(
            "unique",
            t("Unique messages", "Messages uniques"),
            self.unique,
            |this, on, cx| {
              this.unique = on;
              this.room_changed("r9k", cx);
            },
            cx,
          ))
          .into_any_element(),
        Self::line(
          t("Followers-only", "Followers uniquement"),
          vec![self.chips(
            "followers",
            followers,
            &self.followers,
            |this, minutes, cx| {
              this.followers = minutes;
              this.room_changed("followers-only", cx);
            },
            cx,
          )],
        ),
        Self::line(
          t("Slow mode", "Mode lent"),
          vec![self.chips(
            "slow",
            slow,
            &self.slow,
            |this, seconds, cx| {
              this.slow = seconds;
              this.room_changed("slow", cx);
            },
            cx,
          )],
        ),
      ],
    )
  }

  fn writer_card(&self, cx: &mut Context<Self>) -> AnyElement {
    let writers = vec![
      (Writer::Ready, t("Ready", "Prêt").to_owned()),
      (Writer::Connecting, t("Connecting", "Connexion").to_owned()),
      (Writer::Retrying, t("Retrying", "Nouvel essai").to_owned()),
      (
        Writer::Refused,
        t("Token refused", "Jeton refusé").to_owned(),
      ),
      (Writer::NoToken, t("No token", "Sans jeton").to_owned()),
    ];
    let replies = vec![
      (Reply::Echo, t("Shown", "Affiché").to_owned()),
      (
        Reply::Notice("msg_slowmode"),
        t("Slow mode", "Mode lent").to_owned(),
      ),
      (
        Reply::Notice("msg_duplicate"),
        t("Duplicate", "Doublon").to_owned(),
      ),
      (Reply::Notice("msg_followersonly"), "Followers".to_owned()),
      (
        Reply::Notice("msg_subsonly"),
        t("Subscribers", "Abonnés").to_owned(),
      ),
      (Reply::Notice("msg_emoteonly"), "Emotes".to_owned()),
      (Reply::Notice("msg_banned"), t("Banned", "Banni").to_owned()),
      (
        Reply::Notice("msg_timedout"),
        t("Timed out", "Exclu").to_owned(),
      ),
      (
        Reply::Notice("msg_rejected"),
        t("Held", "Retenu").to_owned(),
      ),
      (Reply::Unsent, t("Not sent", "Pas parti").to_owned()),
    ];
    Self::card(
      t("Writing", "Écriture"),
      vec![
        Self::line(
          t("Field", "Champ"),
          vec![self.chips(
            "writer",
            writers,
            &self.writer,
            |this, w, cx| this.set_writer(w, cx),
            cx,
          )],
        ),
        Self::line(
          t("A sent message", "Un message envoyé"),
          vec![self.chips("reply", replies, &self.reply, |this, r, _| this.reply = r, cx)],
        ),
        div()
          .text_xs()
          .text_color(theme::muted())
          .child(tf!(
            "Beyond {RATE_COUNT} messages in {} s, the writer holds them back, as the real one does.",
            "Au-delà de {RATE_COUNT} messages en {} s, l'écriture les retient, comme la vraie.",
            RATE_WINDOW.as_secs()
          ))
          .into_any_element(),
      ],
    )
  }

  fn points_card(&self, cx: &mut Context<Self>) -> AnyElement {
    let points = self.points.borrow();
    let balances = [0u64, 50, 1_000, 25_000, 1_000_000]
      .into_iter()
      .map(|b| (b, super::cards::grouped(b)))
      .collect();
    let refusals = vec![
      (None, t("Accepted", "Acceptées").to_owned()),
      (Some("NOT_ENOUGH_POINTS"), "NOT_ENOUGH_POINTS".to_owned()),
      (Some("COOLDOWN"), "COOLDOWN".to_owned()),
      (Some("NOT_FOUND"), "NOT_FOUND".to_owned()),
      (
        Some("SOMETHING_NEW"),
        t("Unknown code", "Code inconnu").to_owned(),
      ),
    ];
    let balance = points.balance;
    let lines = vec![
      h_flex()
        .flex_wrap()
        .gap_3()
        .child(self.switch(
          "points-enabled",
          t("Channel points", "Points de chaîne"),
          points.enabled,
          |this, on, cx| this.points_changed(|p| p.enabled = on, cx),
          cx,
        ))
        .child(self.switch(
          "points-rewards",
          t("Custom rewards", "Récompenses"),
          points.rewards,
          |this, on, cx| this.points_changed(|p| p.rewards = on, cx),
          cx,
        ))
        .child(self.switch(
          "points-bonus",
          t("Bonus waiting", "Bonus en attente"),
          points.bonus,
          |this, on, cx| this.points_changed(|p| p.bonus = on, cx),
          cx,
        ))
        .into_any_element(),
      Self::line(
        t("Balance", "Solde"),
        vec![self.chips(
          "balance",
          balances,
          &balance,
          |this, b, cx| this.points_changed(|p| p.balance = b, cx),
          cx,
        )],
      ),
      Self::line(
        t("Spends", "Dépenses"),
        vec![self.chips(
          "refuse",
          refusals,
          &points.refuse,
          |this, r, cx| this.points_changed(|p| p.refuse = r, cx),
          cx,
        )],
      ),
    ];
    drop(points);
    Self::card(t("Channel points", "Points de chaîne"), lines)
  }

  fn predictions_card(&self, cx: &mut Context<Self>) -> AnyElement {
    let bettors = self.points.borrow().bettors;
    Self::card(
      t("Predictions", "Prédictions"),
      vec![
        Self::line(
          t("New", "Nouvelle"),
          vec![
            self.button(
              "prediction-2",
              t("2 outcomes · 2 min", "2 issues · 2 min").into(),
              |this, cx| this.points_changed(|p| p.start_prediction(2, 120), cx),
              cx,
            ),
            self.button(
              "prediction-5",
              t("5 outcomes · 30 s", "5 issues · 30 s").into(),
              |this, cx| this.points_changed(|p| p.start_prediction(5, 30), cx),
              cx,
            ),
          ],
        ),
        Self::line(
          t("The running one", "Celle en cours"),
          vec![
            self.button(
              "prediction-lock",
              t("Lock", "Verrouiller").into(),
              |this, cx| this.points_changed(FakePoints::lock, cx),
              cx,
            ),
            self.button(
              "prediction-win-1",
              t("Outcome 1 wins", "L'issue 1 gagne").into(),
              |this, cx| this.points_changed(|p| p.resolve(0), cx),
              cx,
            ),
            self.button(
              "prediction-win-2",
              t("Outcome 2 wins", "L'issue 2 gagne").into(),
              |this, cx| this.points_changed(|p| p.resolve(1), cx),
              cx,
            ),
            self.button(
              "prediction-cancel",
              t("Cancel", "Annuler").into(),
              |this, cx| this.points_changed(FakePoints::cancel, cx),
              cx,
            ),
          ],
        ),
        self.switch(
          "bettors",
          t("Other bettors", "Autres parieurs"),
          bettors,
          |this, on, cx| this.points_changed(|p| p.bettors = on, cx),
          cx,
        ),
      ],
    )
  }
}

impl FakeChatView {
  fn raid_card(&self, cx: &mut Context<Self>) -> AnyElement {
    Self::card(
      t("Outgoing raid", "Raid sortant"),
      vec![
        Self::line(
          "Raid",
          vec![
            self.button(
              "raid-start",
              t("Announce one (90 s)", "En annoncer un (90 s)").into(),
              |this, cx| {
                this.made += 1;
                let raid = twitch::raids::Raid {
                  id: format!("fake-raid-{}", this.made),
                  target_login: "peeby_ami".into(),
                  target_name: "Peeby_Ami".into(),
                  // Drawn in memory, as the fake emotes.
                  target_avatar: Some(format!("{SCHEME}avatar/peeby_ami")),
                  viewer_count: 1,
                  created_at: Some(polls::now_millis()),
                  force_seconds: 90,
                };
                this.raid = Some(raid.clone());
                this.raid_event(RaidEvent::Update(raid), cx);
              },
              cx,
            ),
            self.button(
              "raid-cancel",
              t("Cancel it", "L'annuler").into(),
              |this, cx| {
                if let Some(raid) = this.raid.take() {
                  this.raid_event(RaidEvent::Cancel(raid.id), cx);
                }
              },
              cx,
            ),
            self.button(
              "raid-go",
              t("It goes now", "Il part maintenant").into(),
              |this, cx| {
                if let Some(raid) = this.raid.take() {
                  this.raid_event(RaidEvent::Go(raid), cx);
                }
              },
              cx,
            ),
          ],
        ),
        div()
          .text_xs()
          .text_color(theme::muted())
          .child(t(
            "As on Twitch: viewers join while it counts down, and it goes by itself after 90 s. On a live, a followed raid opens the raided channel. Incoming raids are chat notices (Events).",
            "Comme sur Twitch : des spectateurs le rejoignent pendant le compte à rebours, et il part tout seul après 90 s. Sur un live, un raid suivi ouvre la chaîne raidée. Les raids reçus sont des notices du chat (Événements).",
          ))
          .into_any_element(),
      ],
    )
  }

  /// A fake raid event, as the raid topic would send it.
  fn raid_event(&mut self, event: RaidEvent, cx: &mut Context<Self>) {
    self.points_view.update(cx, |p, cx| p.raid_event(event, cx));
  }

  fn polls_card(&self, cx: &mut Context<Self>) -> AnyElement {
    let voters = self.points.borrow().voters;
    Self::card(
      t("Polls", "Sondages"),
      vec![
        Self::line(
          t("New", "Nouveau"),
          vec![
            self.button(
              "poll-2",
              t("2 choices · 1 min", "2 choix · 1 min").into(),
              |this, cx| this.points_changed(|p| p.start_poll(2, 60), cx),
              cx,
            ),
            self.button(
              "poll-5",
              t("5 choices · 20 s", "5 choix · 20 s").into(),
              |this, cx| this.points_changed(|p| p.start_poll(5, 20), cx),
              cx,
            ),
          ],
        ),
        Self::line(
          t("The first one", "Le premier"),
          vec![
            self.button(
              "poll-end",
              t("End", "Terminer").into(),
              |this, cx| this.points_changed(FakePoints::end_poll, cx),
              cx,
            ),
            self.button(
              "poll-remove",
              t("Remove", "Retirer").into(),
              |this, cx| this.points_changed(FakePoints::remove_poll, cx),
              cx,
            ),
          ],
        ),
        self.switch(
          "voters",
          t("Other voters", "Autres votants"),
          voters,
          |this, on, cx| this.points_changed(|p| p.voters = on, cx),
          cx,
        ),
        div()
          .text_xs()
          .text_color(theme::muted())
          .child(t(
            "Polls show even with the channel points off, as on Twitch. Refused spends refuse votes too.",
            "Les sondages s'affichent même sans points de chaîne, comme sur Twitch. Les refus de dépenses refusent aussi les votes.",
          ))
          .into_any_element(),
      ],
    )
  }
}

impl Render for FakeChatView {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let title: SharedString = t("Fake chat", "Faux chat").into();
    h_flex()
      .size_full()
      .items_start()
      .child(
        v_flex()
          .id("fake-chat-controls")
          .flex_1()
          .min_w_0()
          .h_full()
          .overflow_y_scroll()
          .p_6()
          .gap_4()
          .child(
            h_flex().gap_4().items_start().child(
              v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                  div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
                )
                .child(div().text_sm().text_color(theme::muted()).child(t(
                  "Debug builds only: generated messages through the real parser, points and \
                     predictions kept here. Nothing goes to Twitch.",
                  "Builds debug uniquement : des messages générés passent par le vrai parseur, \
                     les points et les prédictions restent ici. Rien ne part chez Twitch.",
                ))),
            ),
          )
          .child(self.feed_card(cx))
          .child(self.content_card(cx))
          .child(self.events_card(cx))
          .child(self.room_card(cx))
          .child(self.writer_card(cx))
          .child(self.points_card(cx))
          .child(self.predictions_card(cx))
          .child(self.polls_card(cx))
          .child(self.raid_card(cx)),
      )
      .child(
        div()
          .w(px(CHAT_WIDTH))
          .flex_none()
          .h_full()
          .py_6()
          .pr_6()
          .child(
            div()
              .size_full()
              .rounded(theme::RADIUS_LG)
              .bg(theme::surface())
              .border_1()
              .border_color(theme::border())
              .overflow_hidden()
              .child(self.panel.clone()),
          ),
      )
  }
}
