//! A live's channel points, with the web `auth-token` (none: nothing shows): the viewer's
//! balance, refreshed every minute, and the channel's rewards in a popover, under the chat's field
//! ([`PointsFooter`], ADR 0023); what a reading finds earned since the last one rises over it and
//! fades out. Its predictions with their bets, and its polls
//! ([`super::polls`]), are pinned over the chat as Twitch does (`ui::chat::pinned_stack`): folded
//! bars with their state, one card unfolded at a time, new ones folded. Polls show with or without
//! points. A raid the channel is about to send (Twitch's PubSub, ADR 0024) is pinned first, its
//! countdown along its bar: when it goes, the page follows it to the raided channel
//! ([`PointsEvent::Raid`]), unless the viewer chose to stay. Spending asks for a second click ("Confirmer", within 8 s; a bet's shows the amount it
//! sends), since it cannot be undone; a free vote takes one.
//!
//! Every minute the live really plays (mpv neither paused nor buffering), Peeby reports it to
//! Twitch as its web player does (`twitch::spade`): without it, watching earns no points.
//!
//! The bonus chest is not claimed by Peeby's own calls: Twitch's integrity check refuses its
//! mutation outside the browser (docs/twitch.md, "Channel points"). It is clicked in twitch.tv's
//! chat, in a hidden window signed in with the auth-token (`web_sign_in::twitch_chat`): on its
//! own a second or two after a reading shows it (the "auto-claim" setting), or from the "Bonus"
//! button, greyed out with a spinner meanwhile. The button then hides until the points are read
//! again, which shows it again if the bonus still waits.
//!
//! In debug builds, the fake chat (`super::fake_chat`) drives the same view from a local state
//! ([`Backend::Fake`]): nothing is read from Twitch, spent or reported then.

#[cfg(debug_assertions)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
  Anchor, AnyElement, App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Task,
  WeakEntity, Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::popover::Popover;
use gpui_component::{ActiveTheme as _, Disableable, Sizable, h_flex, v_flex};
use twitch::points::{Points, Prediction, PredictionStatus, Reward};

use peeby_core::settings::Settings;
use twitch::hype_train::{self, HypeEvent};
use twitch::pubsub;
use twitch::raids::{Raid, RaidEvent};
use ui::assets::Icon;
use ui::chat::{LEAD, Pinned, Unfolded, pinned_stack};
use ui::i18n::t;
use ui::images::Images;
use ui::picture::{self, Picture};
use ui::player::PlayerView;
use ui::{runtime, tf, theme};

use super::cards;
use super::hype_train as hype_train_view;
use super::polls::{self, Poll};

/// How often the balance and the rewards are read again.
const POINTS_EVERY: Duration = Duration::from_secs(60);
/// How often a minute watched is reported.
const WATCHED_EVERY: Duration = Duration::from_secs(60);
/// How often the predictions are read again (a bet window is often 2 to 5 minutes).
const PREDICTIONS_EVERY: Duration = Duration::from_secs(20);
/// How often the poll is read again (a poll often lasts 1 to 5 minutes).
const POLLS_EVERY: Duration = Duration::from_secs(10);

/// What the view asks its page.
pub enum PointsEvent {
  /// Open this channel's live: the raid the viewer follows has gone.
  Raid(String),
}
/// How long an ended prediction stays pinned with its result, as on Twitch, from when Peeby
/// first reads it ended (the predictions are read every [`PREDICTIONS_EVERY`]).
pub const ENDED_SHOWN: Duration = Duration::from_secs(20);
/// An ended prediction is shown only when it ended this recently: Twitch always gives the last
/// one, hours old or not.
const ENDED_RECENTLY: Duration = Duration::from_secs(120);
/// The bonus button's height at rest, logical pixels (gpui-component's small button).
const BONUS_HEIGHT: f32 = 24.;
/// How long the bonus button takes to come in, with its bump.
const BONUS_BUMP: Duration = Duration::from_millis(420);
/// How long it takes to fade and shrink away.
const BONUS_LEAVE: Duration = Duration::from_millis(220);
/// How long a poll's or a prediction's bar takes to reach a new share of the votes or points.
const SHARE_MOTION: Duration = Duration::from_millis(600);

/// The bonus button's scale and opacity at a moment of its animation.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BonusLook {
  scale: f32,
  opacity: f32,
}

impl BonusLook {
  const REST: Self = Self {
    scale: 1.,
    opacity: 1.,
  };

  /// Coming in at `t` (0 to 1): grows from nothing past its size (about 1.15×), then settles
  /// (an "ease out back" curve), fading in over the first third.
  fn appearing(t: f32) -> Self {
    let t = t.clamp(0., 1.);
    const OVERSHOOT: f32 = 2.2;
    let u = t - 1.;
    Self {
      scale: 1. + (OVERSHOOT + 1.) * u * u * u + OVERSHOOT * u * u,
      opacity: (t * 3.).min(1.),
    }
  }

  /// Leaving at `t` (0 to 1): fades out while it shrinks to 60 %.
  fn leaving(t: f32) -> Self {
    let t = t.clamp(0., 1.);
    let eased = 1. - (1. - t) * (1. - t);
    Self {
      scale: 1. - 0.4 * eased,
      opacity: 1. - eased,
    }
  }
}

/// How long a gain ("+50") rises over the balance and fades away.
const GAIN_FOR: Duration = Duration::from_millis(1600);
/// How far it rises meanwhile, logical pixels.
const GAIN_RISE: f32 = 18.;
/// Its line's height, logical pixels: it starts just over the balance.
const GAIN_HEIGHT: f32 = 16.;

/// A gain's rise (pixels above its start) and opacity at `t` (0 to 1): it rises fast then
/// slows down ("ease out"), and stays visible a while before fading out. With `still` (reduced
/// motion), it only fades.
fn gain_look(t: f32, still: bool) -> (f32, f32) {
  let t = t.clamp(0., 1.);
  let rise = if still {
    0.
  } else {
    GAIN_RISE * (1. - (1. - t).powi(3))
  };
  (rise, 1. - t * t)
}

/// How many times a bonus is claimed automatically before it waits for a click.
const AUTO_CLAIM_TRIES: u8 = 2;
/// The points are read again this long after a claim, to see whether it went through.
const AFTER_CLAIM: Duration = Duration::from_secs(2);

/// A random wait of 1 to 2 s before an automatic claim, so that it does not follow the reading
/// like clockwork.
fn auto_claim_delay() -> Duration {
  // `uuid`'s v4 is random: no other source of randomness in `peeby`.
  let jitter = (uuid::Uuid::new_v4().as_u128() % 1000) as u64;
  Duration::from_millis(1000 + jitter)
}

/// How long a first click waits for its confirmation.
const CONFIRM_FOR: Duration = Duration::from_secs(8);
/// Side of the channel's points icon in the balance, logical pixels (as the default icon).
const POINTS_ICON: f32 = 14.;
/// Twitch's smallest bet.
const MIN_BET: u64 = 10;
/// The bet amounts offered, as shares of the balance in percent.
const SHARES: [u64; 4] = [10, 25, 50, 100];

/// A share's button: `25%`, or "All" for the whole balance.
fn share_label(share: u64) -> String {
  if share == 100 {
    t("All", "Tout").into()
  } else {
    percent(share)
  }
}

/// `25%` (French: `25 %`, a no-break space).
/// `share` (0 to 1) eased from the value `id` last showed, for a bar and its percentage: a new
/// count slides there instead of jumping. Shown as is the first time (an unfolded card does
/// not grow from nothing) and with reduced motion.
pub(crate) fn eased_share(id: String, share: f64, window: &mut Window, cx: &mut App) -> f32 {
  let share = share as f32;
  let duration = if cx.reduce_motion() {
    Duration::ZERO
  } else {
    SHARE_MOTION
  };
  ui::motion::tween(
    ElementId::Name(id.into()),
    share,
    share,
    duration,
    window,
    cx,
  )
}

pub(crate) fn percent(n: impl std::fmt::Display) -> String {
  tf!("{n}%", "{n}\u{a0}%")
}

/// `1 point`, `1,200 points`.
fn points_text(n: u64) -> String {
  let count = cards::grouped(n);
  // The same word in both languages, but not the same plural rule.
  if cards::plural(n) {
    format!("{count} points")
  } else {
    format!("{count} point")
  }
}

/// Where the points come from.
#[derive(Clone)]
pub enum Backend {
  /// Twitch, as the auth-token's account.
  Twitch(twitch::Client),
  /// The debug page's fake channel: read and spent locally, never reported.
  #[cfg(debug_assertions)]
  Fake(Rc<RefCell<super::fake_chat::FakePoints>>),
}

impl Backend {
  /// Twitch's client; `None` for the fake channel.
  fn twitch(&self) -> Option<&twitch::Client> {
    match self {
      Self::Twitch(client) => Some(client),
      #[cfg(debug_assertions)]
      Self::Fake(_) => None,
    }
  }
}

/// An answer on its way (from the network runtime, or ready at once from the fake state).
type Later<T> = Pin<Box<dyn Future<Output = Option<twitch::Result<T>>>>>;

/// How long the fake channel takes to answer a spend, so that the button's wait shows.
#[cfg(debug_assertions)]
const FAKE_SPEND_DELAY: Duration = Duration::from_millis(600);

/// A spend as it is sent: a reward on a channel with the viewer's text, or a bet of points on an
/// outcome of a prediction.
pub(super) enum Order {
  Redeem(String, Reward, Option<String>),
  Bet(String, String, u64),
}

/// A first click: what it asked for, the bet's amount as it was shown then, and when.
struct Pending {
  spend: Spend,
  amount: u64,
  at: Instant,
}

/// What a spend button asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Spend {
  /// A reward, by id.
  Reward(String),
  /// A bet on an outcome of a prediction.
  Bet { event: String, outcome: String },
}

pub struct PointsView {
  login: String,
  backend: Backend,
  /// The page's player: minutes are reported only while it plays.
  player: WeakEntity<PlayerView>,
  /// The last watch-time report failed (the next failures log at debug).
  report_failing: bool,
  points: Option<Points>,
  /// A claim went through the chat window: the button hides until the next reading, which
  /// shows it again if the bonus still waits.
  bonus_hidden: bool,
  /// The chat window is claiming the bonus: the button is greyed out, a spinner beside it.
  claiming: bool,
  /// The bonus button shows (or is coming in): its presence at the last render.
  bonus_shown: bool,
  /// When the button started coming in or leaving, while it animates.
  bonus_since: Option<Instant>,
  /// The last rise of the balance: by how much, and when it was read (shown over the balance
  /// for [`GAIN_FOR`]).
  gain: Option<(u64, Instant)>,
  /// The automatic claims tried, by claim id: at most [`AUTO_CLAIM_TRIES`] each.
  auto_claims: HashMap<String, u8>,
  /// The automatic claim waiting for its delay, or the claim on its way.
  claim_task: Option<Task<()>>,
  predictions: Vec<Prediction>,
  /// No auth-token, or the channel has no points: nothing shows and nothing is read again.
  off: bool,
  /// The spend the next click confirms.
  confirm: Option<Pending>,
  /// A spend is on its way.
  busy: bool,
  /// The share of the balance a bet takes, in percent.
  share: u64,
  /// The last spend's outcome: text, and whether it went through.
  outcome: Option<(SharedString, bool)>,
  /// That outcome is a bet's (shown in the prediction's card; a reward's in its popover).
  outcome_of_bet: bool,
  /// The channel's polls ([`super::polls`]).
  polls: Vec<Poll>,
  /// The raid the channel is about to send.
  raid: Option<Raid>,
  /// The viewer stays on this channel instead of following this raid (by id).
  raid_stay: Option<String>,
  /// The raid already followed (by id): its next departures do nothing.
  raid_followed: Option<String>,
  /// Hears the channel's PubSub topics ([`Self::watch_channel`]); dropping it ends the
  /// connection.
  pubsub_task: Option<Task<()>>,
  /// The channel's Hype Train ([`super::hype_train`]).
  hype: hype_train::State,
  /// The last Hype Train reading failed (the next failures log at debug).
  hype_failing: bool,
  /// The choices the viewer voted for, by poll id: Twitch does not give them back.
  voted: HashMap<String, Vec<String>>,
  /// The last poll reading failed (the next failures log at debug).
  polls_failing: bool,
  /// The pinned item unfolded over the chat.
  unfolded: Unfolded,
  /// A vote is on its way.
  voting: bool,
  /// The last vote failed: why.
  vote_error: Option<SharedString>,
  /// When each recently ended prediction was first read ended, by id: it shows for
  /// [`ENDED_SHOWN`] from then.
  ended_seen: HashMap<String, Instant>,
  /// The text of a reward that asks for one (made at the first render: it needs the window).
  input: Option<Entity<InputState>>,
  _tasks: Vec<Task<()>>,
}

impl PointsView {
  pub fn new(
    login: String,
    backend: Backend,
    player: WeakEntity<PlayerView>,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut view = Self {
      login,
      backend,
      player,
      report_failing: false,
      points: None,
      bonus_hidden: false,
      claiming: false,
      bonus_shown: false,
      bonus_since: None,
      gain: None,
      auto_claims: HashMap::new(),
      claim_task: None,
      predictions: Vec::new(),
      off: false,
      confirm: None,
      busy: false,
      share: 10,
      outcome: None,
      outcome_of_bet: false,
      polls: Vec::new(),
      voted: HashMap::new(),
      polls_failing: false,
      raid: None,
      raid_stay: None,
      raid_followed: None,
      pubsub_task: None,
      hype: hype_train::State::default(),
      hype_failing: false,
      unfolded: Unfolded::default(),
      voting: false,
      vote_error: None,
      ended_seen: HashMap::new(),
      input: None,
      _tasks: Vec::new(),
    };
    view._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        let Ok(alive) = this.update(cx, |this, cx| (!this.off).then(|| this.load_points(cx)))
        else {
          return;
        };
        match alive {
          Some(load) => load.await,
          None => return,
        }
        cx.background_executor().timer(POINTS_EVERY).await;
      }
    }));
    view._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        let Ok(alive) = this.update(cx, |this, cx| {
          (!this.off).then(|| this.load_predictions(cx))
        }) else {
          return;
        };
        match alive {
          Some(load) => load.await,
          None => return,
        }
        cx.background_executor().timer(PREDICTIONS_EVERY).await;
      }
    }));
    // The poll, with or without points: every viewer sees it.
    view._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        let Ok(load) = this.update(cx, |this, cx| this.load_polls(cx)) else {
          return;
        };
        load.await;
        cx.background_executor().timer(POLLS_EVERY).await;
      }
    }));
    // A minute watched, after each minute played.
    view._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(WATCHED_EVERY).await;
        let Ok(report) = this.update(cx, |this, cx| this.minute_watched(cx)) else {
          return;
        };
        if let Some(report) = report {
          let result = report.await;
          if this.update(cx, |this, _| this.reported(result)).is_err() {
            return;
          }
        }
      }
    }));
    // The countdown of a prediction taking bets or a poll taking votes, and the end of a
    // confirmation's wait.
    view._tasks.push(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        let counting = this.update(cx, |this, cx| {
          let expired = this
            .confirm
            .take_if(|p| p.at.elapsed() >= CONFIRM_FOR)
            .is_some();
          if expired
            || this
              .predictions
              .iter()
              .any(|p| p.status == PredictionStatus::Active || this.shown(p))
            || this
              .polls
              .iter()
              .any(|p| !p.ended || this.ended_left(&p.id).is_some())
          {
            cx.notify();
          }
          // The train's countdown, and its end when its message was missed.
          let now = polls::now_millis();
          if this.hype.settle(now, hype_train_view::GRACE_MS) {
            this.fold_gone();
            cx.notify();
          } else if hype_train_view::shown_id(&this.hype, now).is_some() {
            cx.notify();
          }
        });
        if counting.is_err() {
          return;
        }
      }
    }));
    view
  }

  /// Reads the balance and the predictions again now (the fake chat, after a change), even when
  /// the channel had no points.
  #[cfg(debug_assertions)]
  pub fn reload(&mut self, cx: &mut Context<Self>) {
    self.off = false;
    self.load_points(cx).detach();
    self.load_predictions(cx).detach();
    self.load_polls(cx).detach();
  }

  /// The channel's poll on screen: Twitch's (anonymous; a failed reading keeps the last one), or
  /// the fake chat's.
  fn load_polls(&mut self, cx: &mut Context<Self>) -> Task<()> {
    let client = match &self.backend {
      Backend::Twitch(client) => client.clone(),
      #[cfg(debug_assertions)]
      Backend::Fake(fake) => {
        self.polls = fake.borrow().polls();
        self.polls_read(cx);
        return Task::ready(());
      }
    };
    let login = self.login.clone();
    let fetch = runtime::spawn(cx, async move { client.poll(&login).await });
    cx.spawn(async move |this, cx| {
      let Some(result) = fetch.await else {
        return;
      };
      this
        .update(cx, |this, cx| {
          match result {
            Ok(poll) => {
              this.polls = poll
                .into_iter()
                .map(|poll| {
                  let voted = this.voted.get(&poll.id).cloned().unwrap_or_default();
                  Poll::from_twitch(poll, voted)
                })
                .collect();
              this.polls_failing = false;
            }
            Err(e) if this.polls_failing => tracing::debug!("Twitch poll: {e}"),
            Err(e) => {
              tracing::warn!("Twitch poll: {e}");
              this.polls_failing = true;
            }
          }
          this.polls_read(cx);
        })
        .ok();
    })
  }

  /// Listens to `channel_id`'s raids and Hype Trains on Twitch's PubSub (anonymous, ADR 0024
  /// and 0032), once; the connection ends with the view. Each (re)connection reads the train
  /// from GQL: PubSub says only what changes.
  pub fn watch_channel(&mut self, channel_id: String, cx: &mut Context<Self>) {
    if self.pubsub_task.is_some() || self.backend.twitch().is_none() {
      return;
    }
    let (mut events, run) = twitch::pubsub::listen(&channel_id);
    // The connection runs on the network runtime and ends when `events` is dropped.
    drop(runtime::spawn(cx, run));
    self.pubsub_task = Some(cx.spawn(async move |this, cx| {
      while let Some(event) = events.recv().await {
        let alive = this.update(cx, |this, cx| match event {
          pubsub::Event::Listening => this.load_hype(cx),
          pubsub::Event::Raid(event) => this.raid_event(event, cx),
          pubsub::Event::HypeTrain(event) => this.hype_event(event, cx),
        });
        if alive.is_err() {
          return;
        }
      }
    }));
  }

  /// The channel's Hype Train from GQL (anonymous; a failed reading keeps the state).
  fn load_hype(&mut self, cx: &mut Context<Self>) {
    let Some(client) = self.backend.twitch().cloned() else {
      return;
    };
    let login = self.login.clone();
    let fetch = runtime::spawn(cx, async move { client.hype_train(&login).await });
    cx.spawn(async move |this, cx| {
      let Some(result) = fetch.await else {
        return;
      };
      this
        .update(cx, |this, cx| match result {
          Ok(read) => {
            this.hype.merge(read);
            this.hype_failing = false;
            this.fold_gone();
            cx.notify();
          }
          Err(e) if this.hype_failing => tracing::debug!("Twitch Hype Train: {e}"),
          Err(e) => {
            tracing::warn!("Twitch Hype Train: {e}");
            this.hype_failing = true;
          }
        })
        .ok();
    })
    .detach();
  }

  /// The channel's Hype Train as shown.
  #[cfg(debug_assertions)]
  pub fn hype_state(&self) -> &hype_train::State {
    &self.hype
  }

  /// What the Hype Train topic says (or the fake chat).
  pub fn hype_event(&mut self, event: HypeEvent, cx: &mut Context<Self>) {
    self.hype.apply(event);
    self.fold_gone();
    cx.notify();
  }

  /// What the raid topic says: an announced raid is pinned; when it goes, the page follows it
  /// unless the viewer stays (Twitch sends the departure several times: followed once).
  pub fn raid_event(&mut self, event: RaidEvent, cx: &mut Context<Self>) {
    match event {
      RaidEvent::Update(raid) => {
        if self.raid_followed.as_deref() != Some(raid.id.as_str()) {
          self.raid = Some(raid);
        }
      }
      RaidEvent::Go(raid) => {
        self.raid = None;
        let staying = self.raid_stay.as_deref() == Some(raid.id.as_str());
        if !staying && self.raid_followed.as_deref() != Some(raid.id.as_str()) {
          self.raid_followed = Some(raid.id.clone());
          cx.emit(PointsEvent::Raid(raid.target_login));
        }
      }
      RaidEvent::Cancel(id) => {
        if self.raid.as_ref().is_some_and(|r| r.id == id) {
          self.raid = None;
        }
      }
    }
    self.fold_gone();
    cx.notify();
  }

  /// The raid's card: where it goes and what the viewer does.
  fn raid_body(&self, raid: &Raid, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let staying = self.raid_stay.as_deref() == Some(raid.id.as_str());
    let id = raid.id.clone();
    let avatar = raid_avatar(raid, RAID_AVATAR, window, cx);
    v_flex()
      .gap_2()
      .child(
        h_flex().gap_3().items_center().children(avatar).child(
          v_flex()
            .min_w_0()
            .child(
              div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .truncate()
                .child(raid.target_name.clone()),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme::muted())
                .child(format!("twitch.tv/{}", raid.target_login)),
            ),
        ),
      )
      .child(
        div()
          .text_xs()
          .text_color(theme::muted())
          .child(if staying {
            t(
              "You left the raid: you stay here when it goes.",
              "Vous avez quitté le raid : vous restez ici quand il part.",
            )
          } else {
            t(
              "You're in the raid: when it goes, this page opens the raided channel's live.",
              "Vous êtes dans le raid : quand il part, cette page ouvre le live de la chaîne raidée.",
            )
          }),
      )
      // In the raid by default, as on Twitch: the one choice is to leave it (and come back).
      .child(
        h_flex().child(
          Button::new("twitch-raid-leave")
            .small()
            .when(staying, |b| b.primary())
            .when(!staying, |b| b.ghost())
            .label(if staying {
              t("Join the raid", "Rejoindre le raid")
            } else {
              t("Leave the raid", "Quitter le raid")
            })
            .on_click(cx.listener(move |this, _, _, cx| {
              this.raid_stay = if staying { None } else { Some(id.clone()) };
              cx.notify();
            })),
        ),
      )
      .into_any_element()
  }

  /// The polls were read: the ended ones' time is noted, a gone one folds.
  fn polls_read(&mut self, cx: &mut Context<Self>) {
    self.note_ended();
    self.fold_gone();
    cx.notify();
  }

  /// Whether the viewer can vote: with the auth-token's account on Twitch.
  fn can_vote(&self) -> bool {
    match &self.backend {
      Backend::Twitch(_) => self.points.as_ref().is_some_and(|p| p.viewer_id.is_some()),
      #[cfg(debug_assertions)]
      Backend::Fake(_) => true,
    }
  }

  /// Notes when the predictions and polls that ended lately were first read ended; forgets the
  /// others.
  fn note_ended(&mut self) {
    let now = now_millis();
    let recently = i64::try_from(ENDED_RECENTLY.as_millis()).unwrap_or(i64::MAX);
    let ended: Vec<&str> = self
      .predictions
      .iter()
      .filter(|p| p.status.ended() && p.ended_at.is_some_and(|end| now - end < recently))
      .map(|p| p.id.as_str())
      // A poll without its end's time ended when first read so.
      .chain(
        self
          .polls
          .iter()
          .filter(|p| p.ended && p.ended_at.is_none_or(|end| now - end < recently))
          .map(|p| p.id.as_str()),
      )
      .collect();
    self.ended_seen.retain(|id, _| ended.contains(&id.as_str()));
    for id in ended {
      self
        .ended_seen
        .entry(id.to_owned())
        .or_insert_with(Instant::now);
    }
  }

  /// Whether `p` is pinned now: a running one always, an ended one for [`ENDED_SHOWN`] after it
  /// was first read ended, if it ended lately.
  fn shown(&self, p: &Prediction) -> bool {
    !p.status.ended() || self.ended_left(&p.id).is_some()
  }

  /// The share left of an ended item's [`ENDED_SHOWN`], 1 to 0; `None` once over (or not ended).
  fn ended_left(&self, id: &str) -> Option<f32> {
    let elapsed = self.ended_seen.get(id)?.elapsed();
    (elapsed < ENDED_SHOWN).then(|| 1. - elapsed.as_secs_f32() / ENDED_SHOWN.as_secs_f32())
  }

  /// Folds the pinned item that is gone.
  fn fold_gone(&mut self) {
    let ids: Vec<String> = self
      .predictions
      .iter()
      .map(|p| p.id.clone())
      .chain(self.polls.iter().map(|p| p.id.clone()))
      .chain(self.raid.iter().map(|r| r.id.clone()))
      .chain(hype_train_view::shown_id(&self.hype, polls::now_millis()).map(str::to_owned))
      .collect();
    self.unfolded.keep_only(ids.iter().map(String::as_str));
  }

  /// A free vote for `choice` in `poll`: one click.
  fn vote(&mut self, poll: String, choice: String, cx: &mut Context<Self>) {
    if self.voting {
      return;
    }
    let send: Later<()> = match &self.backend {
      Backend::Twitch(client) => {
        // The choices only show as votable with it.
        let Some(user) = self.points.as_ref().and_then(|p| p.viewer_id.clone()) else {
          return;
        };
        let (client, poll, choice) = (client.clone(), poll.clone(), choice.clone());
        // 32 hex digits, new for each vote, as a spend's transaction id.
        let vote_id = uuid::Uuid::new_v4().simple().to_string();
        Box::pin(runtime::spawn(cx, async move {
          client.vote(&poll, &choice, &user, &vote_id).await
        }))
      }
      #[cfg(debug_assertions)]
      Backend::Fake(fake) => {
        let (fake, wait) = (
          fake.clone(),
          cx.background_executor().timer(FAKE_SPEND_DELAY),
        );
        let (poll, choice) = (poll.clone(), choice.clone());
        Box::pin(async move {
          wait.await;
          Some(fake.borrow_mut().vote(&poll, &choice))
        })
      }
    };
    self.voting = true;
    self.vote_error = None;
    cx.notify();
    self._tasks.push(cx.spawn(async move |this, cx| {
      let result = send.await;
      this
        .update(cx, |this, cx| {
          this.voting = false;
          match result {
            Some(Ok(())) => {
              // Twitch does not give it back: kept for the session.
              this
                .voted
                .entry(poll.clone())
                .or_default()
                .push(choice.clone());
              if let Some(shown) = this.polls.iter_mut().find(|p| p.id == poll) {
                shown.voted.push(choice);
              }
            }
            Some(Err(e)) => {
              tracing::warn!("Twitch poll vote: {e}");
              this.vote_error = Some(refusal(&e).into());
            }
            None => {}
          }
          this.load_polls(cx).detach();
          cx.notify();
        })
        .ok();
    }));
  }

  /// The auth-token, for the chat window; `None` for the fake channel.
  fn auth_token(&self) -> Option<String> {
    self
      .backend
      .twitch()
      .and_then(|c| c.auth_token())
      .map(str::to_owned)
  }

  /// With the "auto-claim" setting, claims a waiting bonus after a short random wait, at most
  /// [`AUTO_CLAIM_TRIES`] times per bonus. Never for the fake channel (it would open the browser).
  fn auto_claim(&mut self, cx: &mut Context<Self>) {
    if !Settings::get(cx).twitch.auto_claim_bonus.unwrap_or(true)
      || self.claiming
      || self.claim_task.is_some()
      || self.auth_token().is_none()
    {
      return;
    }
    let Some(id) = self.points.as_ref().and_then(|p| p.claim.clone()) else {
      return;
    };
    let tries = self.auto_claims.entry(id.clone()).or_default();
    if *tries >= AUTO_CLAIM_TRIES {
      return;
    }
    *tries += 1;
    self.claim_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(auto_claim_delay()).await;
      this
        .update(cx, |this, cx| {
          this.claim_task = None;
          // Still the same bonus, still waiting.
          if this.points.as_ref().and_then(|p| p.claim.as_deref()) == Some(id.as_str()) {
            this.claim_bonus(cx);
          }
        })
        .ok();
    }));
  }

  /// Claims the bonus in twitch.tv's chat, in its hidden window (`web_sign_in::twitch_chat`), then
  /// reads the points again. Without the token, opens the chat in the browser. The fake channel
  /// claims it in its local state, after a second with the spinner, as the chat window takes.
  fn claim_bonus(&mut self, cx: &mut Context<Self>) {
    if self.claiming {
      return;
    }
    #[cfg(debug_assertions)]
    if let Backend::Fake(fake) = &self.backend {
      let fake = fake.clone();
      self.claiming = true;
      cx.notify();
      self.claim_task = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        fake.borrow_mut().claim_bonus();
        this
          .update(cx, |this, cx| {
            this.claiming = false;
            this.claim_task = None;
            this.load_points(cx).detach();
          })
          .ok();
      }));
      return;
    }
    let login = self.login.clone();
    let Some(token) = self.auth_token() else {
      cx.open_url(&format!("https://www.twitch.tv/popout/{login}/chat"));
      self.bonus_hidden = true;
      cx.notify();
      return;
    };
    self.claiming = true;
    cx.notify();
    let claim = cx
      .background_executor()
      .spawn(async move { peeby_core::web_sign_in::twitch_chat(&login, &token) });
    self.claim_task = Some(cx.spawn(async move |this, cx| {
      let claim = claim.await;
      tracing::info!("Twitch bonus claim: {claim:?}");
      let Ok(()) = this.update(cx, |this, cx| {
        this.claiming = false;
        this.bonus_hidden = true;
        cx.notify();
      }) else {
        return;
      };
      cx.background_executor().timer(AFTER_CLAIM).await;
      // Detached: clearing `claim_task` drops this task.
      this
        .update(cx, |this, cx| {
          this.claim_task = None;
          this.load_points(cx).detach();
        })
        .ok();
    }));
  }

  fn load_points(&mut self, cx: &mut Context<Self>) -> Task<()> {
    let fetch: Later<Points> = match &self.backend {
      Backend::Twitch(client) => {
        let (client, login) = (client.clone(), self.login.clone());
        Box::pin(runtime::spawn(
          cx,
          async move { client.points(&login).await },
        ))
      }
      #[cfg(debug_assertions)]
      Backend::Fake(fake) => Box::pin(std::future::ready(Some(fake.borrow().points()))),
    };
    cx.spawn(async move |this, cx| {
      let Some(result) = fetch.await else {
        return;
      };
      this
        .update(cx, |this, cx| {
          match result {
            Ok(points) if points.enabled => {
              // Not at the first reading: only what was earned since one.
              if let Some(before) = &this.points
                && points.balance > before.balance
              {
                this.gain = Some((points.balance - before.balance, Instant::now()));
              }
              this.points = Some(points);
              this.bonus_hidden = false;
              this.auto_claim(cx);
            }
            Ok(_) | Err(twitch::Error::NotLoggedIn | twitch::Error::NotFound) => {
              this.off = true;
              this.points = None;
            }
            // A failed reading keeps the last one.
            Err(e) => tracing::warn!("Twitch channel points: {e}"),
          }
          cx.notify();
        })
        .ok();
    })
  }

  /// Reports a minute watched when the live plays now, with the channel's points read (the ids
  /// come with them). Nothing without the auth-token or when the channel has no points.
  fn minute_watched(
    &mut self,
    cx: &mut Context<Self>,
  ) -> Option<impl Future<Output = Option<twitch::Result<()>>> + use<>> {
    // Never for the fake channel: nobody watches a live there.
    let client = self.backend.twitch()?;
    let points = self.points.as_ref().filter(|_| !self.off)?;
    // Only the player that started last in its window (the one on screen there or in its
    // mini-player): one live at a time per window, as one per browser tab on twitch.tv.
    let playing = self.player.upgrade().is_some_and(|player| {
      if !ui::player::is_now_playing(&player, cx) {
        return false;
      }
      let player = player.read(cx);
      // Not while the live's recording plays: that is not the live (ADR 0026).
      player.is_live() && !player.rewound() && player.property("core-idle").as_deref() == Some("no")
    });
    if !playing {
      return None;
    }
    let event = twitch::spade::MinuteWatched {
      channel_id: points.channel_id.clone(),
      channel: self.login.clone(),
      broadcast_id: points.broadcast_id.clone()?,
      user_id: points.viewer_id.clone()?,
    };
    let client = client.clone();
    Some(runtime::spawn(cx, async move {
      client.minute_watched(&event).await
    }))
  }

  /// A report's outcome: the first failure in a row is a warning with its causes (a DNS filter
  /// blocking `spade.twitch.tv` shows as a connection to 0.0.0.0), the next ones are debug.
  fn reported(&mut self, result: Option<twitch::Result<()>>) {
    match result {
      Some(Err(e)) => {
        let mut causes = e.to_string();
        let mut source = std::error::Error::source(&e);
        while let Some(cause) = source {
          causes.push_str(&format!(": {cause}"));
          source = cause.source();
        }
        if self.report_failing {
          tracing::debug!("Twitch watch-time report: {causes}");
        } else {
          tracing::warn!("Twitch watch-time report (no points earned while it fails): {causes}");
        }
        self.report_failing = true;
      }
      Some(Ok(())) => {
        if self.report_failing {
          tracing::info!("Twitch watch-time report: sent again");
        }
        self.report_failing = false;
      }
      None => {}
    }
  }

  fn load_predictions(&mut self, cx: &mut Context<Self>) -> Task<()> {
    let fetch: Later<Vec<Prediction>> = match &self.backend {
      Backend::Twitch(client) => {
        let (client, login) = (client.clone(), self.login.clone());
        Box::pin(runtime::spawn(cx, async move {
          client.predictions(&login).await
        }))
      }
      #[cfg(debug_assertions)]
      Backend::Fake(fake) => Box::pin(std::future::ready(Some(fake.borrow().predictions()))),
    };
    cx.spawn(async move |this, cx| {
      let Some(result) = fetch.await else {
        return;
      };
      this
        .update(cx, |this, cx| {
          match result {
            Ok(predictions) => {
              this.predictions = predictions;
              this.note_ended();
              this.fold_gone();
            }
            Err(e) => tracing::warn!("Twitch predictions: {e}"),
          }
          cx.notify();
        })
        .ok();
    })
  }

  /// The first click on a spend asks for its confirmation, the second sends it.
  fn spend(&mut self, spend: Spend, cx: &mut Context<Self>) {
    if self.busy {
      return;
    }
    let Some(pending) = self
      .confirm
      .take_if(|p| p.spend == spend && p.at.elapsed() < CONFIRM_FOR)
    else {
      if matches!(&spend, Spend::Reward(_)) && !self.confirming(&spend) {
        // Text typed for another reward does not carry over.
        self.input = None;
      }
      self.outcome_of_bet = matches!(spend, Spend::Bet { .. });
      self.confirm = Some(Pending {
        amount: self.bet_amount(),
        spend,
        at: Instant::now(),
      });
      self.outcome = None;
      return cx.notify();
    };
    let Some(points) = self.points.clone() else {
      return;
    };
    // 32 hex digits, new for each spend, as the web player sends.
    let transaction = uuid::Uuid::new_v4().simple().to_string();
    let (order, done): (_, SharedString) = match &spend {
      Spend::Reward(id) => {
        let Some(reward) = points.rewards.iter().find(|r| &r.id == id).cloned() else {
          return;
        };
        let text = self
          .input
          .as_ref()
          .map(|input| input.read(cx).value().trim().to_owned())
          .filter(|t| !t.is_empty());
        if reward.input_required && text.is_none() {
          self.outcome = Some((
            t(
              "Write the reward's message.",
              "Écrivez le message de la récompense.",
            )
            .into(),
            false,
          ));
          return cx.notify();
        }
        let done = tf!("Redeemed “{}”.", "« {} » échangée.", reward.title).into();
        (Order::Redeem(points.channel_id.clone(), reward, text), done)
      }
      Spend::Bet { event, outcome } => {
        // The amount the button showed, not one made from a balance read since.
        let amount = pending.amount;
        let points = points_text(amount);
        let done = tf!("Bet of {points} placed.", "Pari de {points} placé.").into();
        (Order::Bet(event.clone(), outcome.clone(), amount), done)
      }
    };
    let send: Later<()> = match &self.backend {
      Backend::Twitch(client) => {
        let client = client.clone();
        Box::pin(runtime::spawn(cx, async move {
          match order {
            Order::Redeem(channel, reward, text) => {
              client
                .redeem(&channel, &reward, text.as_deref(), &transaction)
                .await
            }
            Order::Bet(event, outcome, amount) => {
              client.predict(&event, &outcome, amount, &transaction).await
            }
          }
        }))
      }
      #[cfg(debug_assertions)]
      Backend::Fake(fake) => {
        let (fake, wait) = (
          fake.clone(),
          cx.background_executor().timer(FAKE_SPEND_DELAY),
        );
        Box::pin(async move {
          wait.await;
          Some(fake.borrow_mut().spend(&order))
        })
      }
    };
    self.busy = true;
    cx.notify();
    self._tasks.push(cx.spawn(async move |this, cx| {
      let result = send.await;
      this
        .update(cx, |this, cx| {
          this.busy = false;
          this.confirm = None;
          this.outcome = match result {
            Some(Ok(())) => {
              // A new, empty field at the next render.
              this.input = None;
              Some((done, true))
            }
            Some(Err(e)) => {
              tracing::warn!("Twitch channel points spend: {e}");
              Some((refusal(&e).into(), false))
            }
            None => None,
          };
          this.load_points(cx).detach();
          this.load_predictions(cx).detach();
          cx.notify();
        })
        .ok();
    }));
  }

  /// Whether `spend`'s button waits for its confirmation.
  fn confirming(&self, spend: &Spend) -> bool {
    self
      .confirm
      .as_ref()
      .is_some_and(|p| &p.spend == spend && p.at.elapsed() < CONFIRM_FOR)
  }

  /// The bet the chosen share makes: at least Twitch's smallest one.
  fn bet_amount(&self) -> u64 {
    let balance = self.points.as_ref().map_or(0, |p| p.balance);
    (balance * self.share / 100).max(MIN_BET).min(balance)
  }

  fn balance_chip(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let points = self.points.clone().unwrap_or_default();
    let name = points.name.clone().unwrap_or_else(|| "points".into());
    let label = format!("{} {name}", cards::grouped(points.balance));
    // The channel's own icon for its points once loaded, else the default one.
    let icon = points.icon.clone().and_then(|url| {
      picture::logo(
        &Picture::sized(move |_, _| url.clone()),
        POINTS_ICON,
        POINTS_ICON,
        window,
        cx,
      )
    });
    let this = cx.entity().downgrade();
    let closed = this.clone();
    // Under the chat's field: it opens upward.
    Popover::new("twitch-points-rewards")
      .anchor(Anchor::BottomLeft)
      .on_open_change(move |open, _, cx| {
        if !*open {
          closed
            .update(cx, |this, cx| {
              if this
                .confirm
                .as_ref()
                .is_some_and(|p| matches!(p.spend, Spend::Reward(_)))
              {
                this.confirm = None;
                cx.notify();
              }
            })
            .ok();
        }
      })
      .trigger(
        Button::new("twitch-points-balance")
          .small()
          .ghost()
          .map(|button| match icon {
            Some(icon) => button.child(
              h_flex()
                .gap_1()
                .items_center()
                .child(icon)
                .child(label.clone()),
            ),
            None => button.icon(Icon::Points).label(label.clone()),
          })
          .tooltip(t(
            "Channel Points · rewards",
            "Points de chaîne · récompenses",
          )),
      )
      .content(move |_, _, cx| {
        this
          .update(cx, |this, cx| this.rewards_panel(cx).into_any_element())
          .unwrap_or_else(|_| div().into_any_element())
      })
  }

  fn rewards_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let points = self.points.clone().unwrap_or_default();
    let input = self.input.clone();
    let mut list = v_flex()
      .id("twitch-rewards-list")
      .max_h(px(480.))
      .overflow_y_scroll()
      .gap_1();
    if points.rewards.is_empty() {
      list = list.child(div().px_2().text_sm().text_color(theme::muted()).child(t(
        "This channel has no custom rewards.",
        "Cette chaîne n'a pas de récompense personnalisée.",
      )));
    }
    let now = now_millis();
    for reward in &points.rewards {
      let confirming = self.confirming(&Spend::Reward(reward.id.clone()));
      let cooling = reward.cooldown_until.is_some_and(|until| until > now);
      let affordable = reward.cost <= points.balance;
      let usable = reward.available && !cooling && affordable && !self.busy;
      let label = if confirming {
        t("Confirm", "Confirmer")
      } else if cooling {
        t("On cooldown", "En recharge")
      } else if !reward.available {
        t("Unavailable", "Indisponible")
      } else {
        t("Redeem", "Échanger")
      };
      let spend = Spend::Reward(reward.id.clone());
      list = list.child(
        v_flex()
          .px_2()
          .py_2()
          .gap_1()
          .rounded(theme::RADIUS)
          .when(confirming, |el| el.bg(theme::surface_hover()))
          .child(
            h_flex()
              .gap_3()
              .items_center()
              .child(
                div().size(px(10.)).flex_none().rounded_full().bg(
                  reward
                    .color
                    .as_deref()
                    .and_then(color)
                    .unwrap_or_else(theme::accent),
                ),
              )
              .child(
                v_flex()
                  .flex_1()
                  .min_w_0()
                  .child(div().text_sm().child(reward.title.clone()))
                  .child(
                    div()
                      .text_xs()
                      .text_color(if affordable {
                        theme::muted()
                      } else {
                        theme::subtle()
                      })
                      .child(format!(
                        "{}{}",
                        points_text(reward.cost),
                        if reward.sub_only {
                          t(" · subscribers", " · abonnés")
                        } else {
                          ""
                        }
                      )),
                  ),
              )
              .child(
                Button::new(ElementId::Name(
                  format!("twitch-reward-{}", reward.id).into(),
                ))
                .small()
                .when(confirming, |b| b.primary())
                .label(label)
                .loading(confirming && self.busy)
                .disabled(!usable)
                .on_click(cx.listener(move |this, _, _, cx| this.spend(spend.clone(), cx))),
              ),
          )
          .children(reward.prompt.clone().map(|prompt| {
            div()
              .text_xs()
              .text_color(theme::muted())
              .line_clamp(2)
              .child(first_line(&prompt))
          }))
          .when(confirming && reward.input_required, |el| {
            el.children(input.clone().map(|input| Input::new(&input).small()))
          }),
      );
    }
    v_flex()
      .w(px(360.))
      .gap_2()
      .child(
        h_flex()
          .px_2()
          .pb_1()
          .justify_between()
          .child(
            div()
              .font_weight(FontWeight::SEMIBOLD)
              .child(t("Rewards", "Récompenses")),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme::muted())
              .child(points_text(points.balance)),
          ),
      )
      .children(self.outcome_line())
      .child(list)
  }

  fn outcome_line(&self) -> Option<impl IntoElement + use<>> {
    self.outcome.clone().map(|(text, ok)| {
      div()
        .px_2()
        .text_sm()
        .text_color(if ok { theme::accent() } else { theme::muted() })
        .child(text)
    })
  }

  /// A prediction's card, unfolded over the chat: its outcomes, the bet amounts, a bet's outcome.
  fn prediction_body(
    &self,
    p: &Prediction,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let total: u64 = p.outcomes.iter().map(|o| o.points).sum();
    let open = p.status == PredictionStatus::Active && p.bet.is_none();
    let canceled = p.status == PredictionStatus::Canceled;
    let amount = self.bet_amount();
    let balance = self.points.as_ref().map_or(0, |p| p.balance);
    let mut outcomes = v_flex().gap_1();
    for outcome in &p.outcomes {
      let share = if total > 0 {
        outcome.points as f64 / total as f64
      } else {
        0.0
      };
      // What a point bet on it returns, if it wins.
      let ratio = (outcome.points > 0).then(|| total as f64 / outcome.points as f64);
      let share = eased_share(
        format!("twitch-bet-bar-{}-{}", p.id, outcome.id),
        share,
        window,
        cx,
      );
      let spend = Spend::Bet {
        event: p.id.clone(),
        outcome: outcome.id.clone(),
      };
      let confirming = self.confirming(&spend);
      let pending_amount = self.confirm.as_ref().map_or(amount, |p| p.amount);
      let won = p.winner.as_deref() == Some(outcome.id.as_str());
      outcomes = outcomes.child(
        v_flex()
          .gap_0p5()
          .child(
            h_flex()
              .gap_2()
              .items_center()
              .child(
                div()
                  .size(px(8.))
                  .flex_none()
                  .rounded_full()
                  .bg(outcome_color(&outcome.color)),
              )
              .child(
                div()
                  .flex_1()
                  .min_w_0()
                  .text_sm()
                  .truncate()
                  .when(won, |el| el.font_weight(FontWeight::SEMIBOLD))
                  .when(canceled, |el| el.text_color(theme::muted()))
                  .child(outcome.title.clone()),
              )
              .when(won, |row| {
                row.child(
                  gpui_component::Icon::new(Icon::Check)
                    .size(px(14.))
                    .text_color(outcome_color(&outcome.color)),
                )
              })
              .child(div().text_xs().text_color(theme::muted()).child(format!(
                "{}{}",
                percent(format!("{:.0}", share * 100.0)),
                ratio.map(|r| format!(" · 1:{r:.2}")).unwrap_or_default()
              )))
              .when(open, |row| {
                row.child(
                  Button::new(ElementId::Name(
                    format!("twitch-bet-{}-{}", p.id, outcome.id).into(),
                  ))
                  .small()
                  .icon(Icon::Points)
                  // Filled in the accent, and amber while it waits for its confirmation.
                  .when(!confirming, |b| b.primary())
                  .when(confirming, |b| b.warning())
                  .label(if confirming {
                    let amount = cards::grouped(pending_amount);
                    tf!("Confirm {amount}", "Confirmer {amount}")
                  } else {
                    let amount = cards::grouped(amount);
                    tf!("Bet {amount}", "Parier {amount}")
                  })
                  .loading(confirming && self.busy)
                  .disabled(self.busy || balance < MIN_BET)
                  .on_click(cx.listener(move |this, _, _, cx| this.spend(spend.clone(), cx))),
                )
              }),
          )
          .child(
            div()
              .h(px(3.))
              .w_full()
              .rounded_full()
              .bg(theme::surface_hover())
              .child(
                div()
                  .h_full()
                  .rounded_full()
                  .w(gpui::relative(share))
                  .bg(outcome_color(&outcome.color)),
              ),
          )
          .child(div().text_xs().text_color(theme::subtle()).child({
            let points = points_text(outcome.points);
            let users = cards::grouped(outcome.users);
            if cards::plural(outcome.users) {
              tf!("{points} · {users} bettors", "{points} · {users} parieurs")
            } else {
              tf!("{points} · {users} bettor", "{points} · {users} parieur")
            }
          })),
      );
    }
    let shares = open.then(|| {
      h_flex()
        .gap_1()
        .child(
          div()
            .text_xs()
            .text_color(theme::muted())
            .mr_1()
            .child(t("Amount", "Mise")),
        )
        .children(SHARES.iter().map(|&share| {
          Button::new(ElementId::Name(format!("twitch-bet-share-{share}").into()))
            .xsmall()
            .when(self.share == share, |b| b.primary())
            .when(self.share != share, |b| b.ghost())
            .label(share_label(share))
            .on_click(cx.listener(move |this, _, _, cx| {
              this.share = share;
              this.confirm = None;
              cx.notify();
            }))
        }))
    });
    v_flex()
      .gap_2()
      .child(outcomes)
      .children(shares)
      .when(self.outcome_of_bet, |el| el.children(self.outcome_line()))
      .into_any_element()
  }

  /// A poll's card: its choices with their share of the votes; while it takes votes and the
  /// viewer has not voted, a click on a choice (lit up under the pointer) votes for it.
  fn poll_body(&self, poll: &Poll, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let total = poll.total();
    let can_vote = self.can_vote();
    let open = can_vote && poll.choices.iter().any(|c| poll.votable(&c.id));
    let winner = poll
      .ended
      .then(|| poll.winner().map(|c| c.id.clone()))
      .flatten();
    let mut choices = v_flex().gap_1();
    for choice in &poll.choices {
      let share = if total > 0 {
        choice.votes as f64 / total as f64
      } else {
        0.0
      };
      let share = eased_share(
        format!("twitch-vote-bar-{}-{}", poll.id, choice.id),
        share,
        window,
        cx,
      );
      let mine = poll.voted.iter().any(|c| c == &choice.id);
      let won = winner.as_deref() == Some(choice.id.as_str());
      let (poll_id, choice_id) = (poll.id.clone(), choice.id.clone());
      // While it takes the viewer's vote, the whole choice is the button.
      let votable = can_vote && poll.votable(&choice.id) && !self.voting;
      choices =
        choices.child(
          v_flex()
            .id(ElementId::Name(
              format!("twitch-vote-{}-{}", poll.id, choice.id).into(),
            ))
            .gap_0p5()
            .px_2()
            .py_1p5()
            .rounded(theme::RADIUS)
            .when(mine, |el| el.bg(theme::surface_hover()))
            .when(votable, |el| {
              el.cursor_pointer()
                .hover(|s| s.bg(theme::surface_hover()))
                .on_click(cx.listener(move |this, _, _, cx| {
                  this.vote(poll_id.clone(), choice_id.clone(), cx)
                }))
            })
            .child(
              h_flex()
                .gap_2()
                .items_center()
                .child(
                  div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    .when(won, |el| el.font_weight(FontWeight::SEMIBOLD))
                    .child(choice.title.clone()),
                )
                .when(mine, |row| {
                  row.child(
                    gpui_component::Icon::new(Icon::Check)
                      .size(px(14.))
                      .text_color(theme::accent()),
                  )
                })
                .child(
                  div()
                    .text_xs()
                    .text_color(theme::muted())
                    .child(percent(format!("{:.0}", share * 100.0))),
                ),
            )
            .child(
              div()
                .h(px(3.))
                .w_full()
                .rounded_full()
                // Not the hover's color: the track stays visible under the pointer.
                .bg(theme::border())
                .child(
                  div()
                    .h_full()
                    .rounded_full()
                    .w(gpui::relative(share))
                    .bg(if won || mine {
                      theme::accent()
                    } else {
                      theme::muted()
                    }),
                ),
            )
            .child(div().text_xs().text_color(theme::subtle()).child({
              let votes = cards::grouped(choice.votes);
              if cards::plural(choice.votes) {
                format!("{votes} votes")
              } else {
                format!("{votes} vote")
              }
            })),
        );
    }
    v_flex()
      .gap_2()
      .when(open, |el| {
        el.child(div().text_xs().text_color(theme::muted()).child(
          match (poll.multichoice, poll.subscriber_only) {
            (true, _) => t(
              "Click the choices to vote for: several are allowed.",
              "Cliquez sur les choix pour lesquels voter : plusieurs sont permis.",
            ),
            (false, true) => t(
              "Click a choice to vote (subscribers only).",
              "Cliquez sur un choix pour voter (abonnés uniquement).",
            ),
            (false, false) => t(
              "Click a choice to vote.",
              "Cliquez sur un choix pour voter.",
            ),
          },
        ))
      })
      .when(!can_vote && !poll.ended, |el| {
        el.child(div().text_xs().text_color(theme::muted()).child(t(
          "Add the Twitch site token in the settings to vote.",
          "Ajoutez le jeton du site Twitch dans les réglages pour voter.",
        )))
      })
      .child(choices)
      .children(
        self
          .vote_error
          .clone()
          .map(|e| div().text_sm().text_color(theme::muted()).child(e)),
      )
      .into_any_element()
  }

  /// What is pinned over the chat: the predictions (with points), then the polls.
  fn pinned(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<Pinned> {
    let mut items = Vec::new();
    let now = polls::now_millis();
    // The raid first: it may take the viewer away.
    if let Some(raid) = &self.raid {
      let viewers = cards::grouped(raid.viewer_count);
      let staying = self.raid_stay.as_deref() == Some(raid.id.as_str());
      let next = if staying {
        t("raid left", "raid quitté")
      } else {
        t("you're in it", "vous en êtes")
      };
      // It leaves at the latest `force_seconds` after its announcement.
      let length = i64::try_from(raid.force_seconds).unwrap_or(0) * 1000;
      let left = raid
        .created_at
        .map(|start| ((start + length - now) / 1000).max(0))
        .map(|s| format!(" · {}", ui::player::format_time(s as f64)))
        .unwrap_or_default();
      items.push(Pinned {
        id: raid.id.clone().into(),
        icon: Icon::Tv,
        accent: theme::accent(),
        kind: "Raid".into(),
        title: tf!("Raid to {}", "Raid vers {}", raid.target_name).into(),
        status: if cards::plural(raid.viewer_count) {
          tf!(
            "{viewers} viewers · {next}{left}",
            "{viewers} spectateurs · {next}{left}"
          )
        } else {
          tf!(
            "{viewers} viewer · {next}{left}",
            "{viewers} spectateur · {next}{left}"
          )
        }
        .into(),
        body: self
          .unfolded
          .is(&raid.id)
          .then(|| self.raid_body(raid, window, cx)),
        time_left: raid
          .created_at
          .and_then(|start| polls::share_left(start, length, now)),
        lead: raid_avatar(raid, LEAD, window, cx),
        fill: None,
      });
    }
    // Then the Hype Train, at the top as on Twitch.
    items.extend(hype_train_view::pinned(
      &self.hype,
      &self.unfolded,
      now,
      window,
      cx,
    ));
    if !self.off && self.points.is_some() {
      for p in self.predictions.iter().filter(|p| self.shown(p)) {
        let open = self.unfolded.is(&p.id);
        items.push(Pinned {
          id: p.id.clone().into(),
          icon: Icon::Points,
          accent: outcome_color("BLUE"),
          kind: t("Prediction", "Prédiction").into(),
          title: p.title.clone().into(),
          status: prediction_status(p).into(),
          body: open.then(|| self.prediction_body(p, window, cx)),
          // The bets' time, then the result's.
          lead: None,
          time_left: match p.status {
            PredictionStatus::Active => i64::try_from(p.window_seconds)
              .ok()
              .zip(p.created_at)
              .and_then(|(window, start)| polls::share_left(start, window * 1000, now)),
            PredictionStatus::Locked => None,
            PredictionStatus::Resolved | PredictionStatus::Canceled => self.ended_left(&p.id),
          },
          fill: None,
        });
      }
    }
    for poll in self
      .polls
      .iter()
      .filter(|p| !p.ended || self.ended_left(&p.id).is_some())
    {
      let open = self.unfolded.is(&poll.id);
      items.push(Pinned {
        id: poll.id.clone().into(),
        icon: Icon::Poll,
        accent: theme::accent(),
        kind: t("Poll", "Sondage").into(),
        title: poll.title.clone().into(),
        status: poll.status(now).into(),
        body: open.then(|| self.poll_body(poll, window, cx)),
        // The votes' time, then the result's.
        time_left: poll.time_left(now).or_else(|| self.ended_left(&poll.id)),
        lead: None,
        fill: None,
      });
    }
    items
  }
}

/// A prediction's state in a few words, for its bar (folded, it is all that shows).
fn prediction_status(p: &Prediction) -> String {
  let title_of = |id: &str| {
    p.outcomes
      .iter()
      .find(|o| o.id == id)
      .map_or(String::new(), |o| o.title.clone())
  };
  match (p.status, &p.bet) {
    (PredictionStatus::Resolved, bet) => {
      let winner = p.winner.as_deref().map(title_of).unwrap_or_default();
      let result = match bet {
        Some((outcome, points)) => match p.payout(outcome, *points) {
          Some(payout) => {
            let payout = points_text(payout);
            tf!(" · you win {payout}", " · vous gagnez {payout}")
          }
          None => {
            let points = points_text(*points);
            tf!(" · you lose {points}", " · vous perdez {points}")
          }
        },
        None => String::new(),
      };
      tf!(
        "Ended · “{winner}” wins{result}",
        "Terminée · « {winner} » l'emporte{result}"
      )
    }
    (PredictionStatus::Canceled, Some((_, points))) => {
      let points = points_text(*points);
      tf!(
        "Canceled · your {points} came back",
        "Annulée · vos {points} sont rendus"
      )
    }
    (PredictionStatus::Canceled, None) => {
      t("Canceled · bets refunded", "Annulée · paris remboursés").into()
    }
    (_, Some((outcome, points))) => {
      let title = title_of(outcome);
      let points = points_text(*points);
      let state = if p.status == PredictionStatus::Locked {
        t(" · closed", " · clos")
      } else {
        ""
      };
      tf!(
        "Your bet: {points} on “{title}”{state}",
        "Votre pari : {points} sur « {title} »{state}"
      )
    }
    (PredictionStatus::Active, None) => match remaining(p) {
      Some(left) if left > 0 => {
        let left = ui::player::format_time(left as f64);
        tf!("Betting open · {left}", "Paris ouverts · {left}")
      }
      _ => t("Betting closing", "Paris en cours de fermeture").into(),
    },
    (PredictionStatus::Locked, None) => t(
      "Betting closed · waiting for the result",
      "Paris clos · en attente du résultat",
    )
    .into(),
  }
}

impl PointsView {
  /// The balance and its rewards, with the bonus button when one waits: under the chat's field.
  fn footer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    if self.off || self.points.is_none() {
      return div().into_any_element();
    }
    if self.input.is_none() {
      self.input = Some(
        cx.new(|cx| InputState::new(window, cx).placeholder(t("Your message", "Votre message"))),
      );
    }
    let shown = self.points.as_ref().is_some_and(|p| p.claim.is_some())
      && (!self.bonus_hidden || self.claiming);
    // Each change of the button's presence starts its animation (none with reduced motion).
    if shown != self.bonus_shown {
      self.bonus_shown = shown;
      self.bonus_since = (!cx.reduce_motion()).then(Instant::now);
    }
    let mut look = BonusLook::REST;
    if let Some(since) = self.bonus_since {
      let length = if shown { BONUS_BUMP } else { BONUS_LEAVE };
      let t = since.elapsed().as_secs_f32() / length.as_secs_f32();
      if t < 1. {
        look = if shown {
          BonusLook::appearing(t)
        } else {
          BonusLook::leaving(t)
        };
        window.request_animation_frame();
      } else {
        self.bonus_since = None;
      }
    }
    let leaving = !shown && self.bonus_since.is_some();
    let claiming = self.claiming;
    let gain = self.gain.and_then(|(amount, since)| {
      let t = since.elapsed().as_secs_f32() / GAIN_FOR.as_secs_f32();
      (t < 1.).then(|| (amount, gain_look(t, cx.reduce_motion())))
    });
    if gain.is_some() {
      window.request_animation_frame();
    } else {
      self.gain = None;
    }
    h_flex()
      .gap_1()
      .items_center()
      .child(
        // The gain floats over the balance (over the chat's field), out of the row's layout.
        div()
          .relative()
          .child(self.balance_chip(window, cx))
          .when_some(gain, |chip, (amount, (rise, opacity))| {
            chip.child(
              div()
                .absolute()
                .left(px(8.))
                .top(px(-GAIN_HEIGHT - rise))
                .h(px(GAIN_HEIGHT))
                .flex()
                .items_center()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(theme::accent())
                .opacity(opacity)
                .child(format!("+{}", cards::grouped(amount))),
            )
          }),
      )
      .when(shown || leaving, |row| {
        row
          // The button's full height, so that the row does not move while it scales.
          .child(
            div()
              .h(px(BONUS_HEIGHT))
              .flex()
              .items_center()
              .child(self.bonus_button(look, claiming || leaving, cx)),
          )
          .when(claiming, |row| {
            row.child(ui::widgets::loader(16., theme::muted()))
          })
      })
      .into_any_element()
  }

  /// The "Bonus" button drawn at `look`'s scale and opacity: gpui-component's small primary
  /// button, redrawn, since GPUI scales no element. `inert` greys it out (a claim on its way) or
  /// only ignores clicks (leaving).
  fn bonus_button(&self, look: BonusLook, inert: bool, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme();
    let (background, hover, foreground, radius) = (
      theme.button_primary,
      theme.button_primary_hover,
      theme.button_primary_foreground,
      theme.radius,
    );
    let s = look.scale;
    let this = cx.entity().downgrade();
    let tooltip = if self.claiming {
      t("Claiming the bonus…", "Réclamation du bonus…")
    } else {
      t(
        "A bonus is waiting: claim it",
        "Un bonus attend : le réclamer",
      )
    };
    h_flex()
      .id("twitch-points-bonus")
      .h(px(BONUS_HEIGHT * s))
      .px(px(8. * s))
      .gap(px(4. * s))
      .items_center()
      .rounded(radius * s)
      .bg(background)
      .text_color(foreground)
      .text_size(px(14. * s))
      .font_weight(FontWeight::MEDIUM)
      .opacity(look.opacity * if self.claiming { 0.5 } else { 1. })
      .child(gpui_component::Icon::new(Icon::Gift).with_size(px(14. * s)))
      .child("Bonus")
      .tooltip(move |window, cx| gpui_component::tooltip::Tooltip::new(tooltip).build(window, cx))
      .when(!inert, |button| {
        button
          .cursor_pointer()
          .hover(move |style| style.bg(hover))
          .on_click(move |_, _, cx| {
            this.update(cx, |this, cx| this.claim_bonus(cx)).ok();
          })
      })
      .into_any_element()
  }
}

impl gpui::EventEmitter<PointsEvent> for PointsView {}

/// The raid, predictions and polls pinned over the chat (`ChatPanel::set_pinned`).
impl Render for PointsView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let items = self.pinned(window, cx);
    let this = cx.entity().downgrade();
    pinned_stack(
      items,
      Rc::new(move |id, _, cx| {
        this
          .update(cx, |this, cx| {
            this.unfolded.toggle(id);
            cx.notify();
          })
          .ok();
      }),
    )
  }
}

/// The chat's footer element: [`PointsView`]'s balance, rewards and bonus, drawn from the same
/// view so that it is read and reported once.
pub struct PointsFooter {
  points: Entity<PointsView>,
}

impl PointsFooter {
  pub fn new(points: Entity<PointsView>, cx: &mut Context<Self>) -> Self {
    cx.observe(&points, |_, _, cx| cx.notify()).detach();
    // The channel's points icon, once loaded.
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    Self { points }
  }
}

impl Render for PointsFooter {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self
      .points
      .update(cx, |points, cx| points.footer(window, cx))
  }
}

/// Side of the raided channel's avatar in the raid's card, logical pixels.
const RAID_AVATAR: f32 = 40.;

/// The raided channel's avatar, `size` px round; `None` without one.
fn raid_avatar(
  raid: &Raid,
  size: f32,
  window: &mut Window,
  cx: &mut gpui::App,
) -> Option<AnyElement> {
  let url = raid.target_avatar.clone()?;
  Some(picture::avatar(
    &Picture::sized(move |_, _| url.clone()),
    size,
    window,
    cx,
  ))
}

/// Seconds left to bet on `p`.
fn remaining(p: &Prediction) -> Option<i64> {
  let end = p.created_at? + i64::try_from(p.window_seconds).ok()? * 1000;
  Some((end - now_millis()) / 1000)
}

fn now_millis() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// What to tell the viewer when a spend fails. Only `NOT_FOUND` was seen; the other codes are
/// guesses at Twitch's names, and an unknown one shows as is.
fn refusal(error: &twitch::Error) -> String {
  match error {
    twitch::Error::Refused(code) => match code.as_str() {
      "NOT_ENOUGH_POINTS" => t("Not enough points.", "Pas assez de points.").into(),
      "COOLDOWN" | "GLOBAL_COOLDOWN" => t(
        "This reward is on cooldown.",
        "Cette récompense est en recharge.",
      )
      .into(),
      "MAX_PER_STREAM" | "MAX_PER_USER_PER_STREAM" => t(
        "Limit reached for this live.",
        "Limite atteinte pour ce live.",
      )
      .into(),
      "DISABLED" | "PAUSED" | "OUT_OF_STOCK" => t(
        "This reward isn't available.",
        "Cette récompense n'est pas disponible.",
      )
      .into(),
      "NOT_FOUND" => t(
        "Not found: it may have just ended.",
        "Introuvable : elle vient peut-être de se terminer.",
      )
      .into(),
      code => tf!("Twitch refused ({code}).", "Twitch a refusé ({code})."),
    },
    twitch::Error::IntegrityRequired => t(
      "Twitch asks for a check only its site can do.",
      "Twitch demande une vérification que seul son site peut faire.",
    )
    .into(),
    twitch::Error::NotLoggedIn => t(
      "Twitch no longer accepts the auth-token.",
      "Twitch n'accepte plus le jeton auth-token.",
    )
    .into(),
    _ => t(
      "Twitch didn't answer. Try again.",
      "Twitch n'a pas répondu. Réessayez.",
    )
    .into(),
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

/// Twitch's colors for an outcome.
fn outcome_color(name: &str) -> Hsla {
  match name {
    "PINK" => gpui::rgb(0xf5009b).into(),
    _ => gpui::rgb(0x387aff).into(),
  }
}

/// The first line of a text (`line_clamp` limits wrapped lines only).
fn first_line(text: &str) -> String {
  text.lines().next().unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests {
  #[test]
  fn bonus_bumps_in_and_shrinks_away() {
    use super::BonusLook;
    let start = BonusLook::appearing(0.);
    assert!(start.scale.abs() < 1e-6 && start.opacity == 0.);
    assert_eq!(BonusLook::appearing(1.), BonusLook::REST);
    let peak = (1..100)
      .map(|i| BonusLook::appearing(i as f32 / 100.).scale)
      .fold(0., f32::max);
    assert!((1.1..1.2).contains(&peak), "bump peak {peak}");
    assert_eq!(BonusLook::leaving(0.), BonusLook::REST);
    let end = BonusLook::leaving(1.);
    assert!((end.scale - 0.6).abs() < 1e-6 && end.opacity == 0.);
  }

  use super::*;

  #[test]
  fn gain_rises_and_fades() {
    assert_eq!(gain_look(0., false), (0., 1.));
    let (rise, opacity) = gain_look(1., false);
    assert!((rise - GAIN_RISE).abs() < 1e-6 && opacity == 0.);
    let (rise, opacity) = gain_look(0.5, false);
    assert!(rise > GAIN_RISE / 2. && opacity > 0.5);
    assert_eq!(gain_look(0.5, true).0, 0.);
  }

  #[test]
  fn refusals_read() {
    let refused = |code: &str| refusal(&twitch::Error::Refused(code.into()));
    assert_eq!(refused("NOT_ENOUGH_POINTS"), "Not enough points.");
    assert_eq!(refused("WHATEVER"), "Twitch refused (WHATEVER).");
  }

  #[test]
  fn amounts_read() {
    assert_eq!(points_text(1), "1 point");
    assert_eq!(points_text(1_200), "1,200 points");
    assert_eq!(share_label(25), "25%");
    assert_eq!(share_label(100), "All");
  }

  #[test]
  fn colors_are_read() {
    assert_eq!(color("#8B3EF7"), Some(gpui::rgb(0x8b3ef7).into()));
    assert_eq!(color("8B3EF7"), None);
    assert_eq!(first_line("a\nb"), "a");
  }
}
