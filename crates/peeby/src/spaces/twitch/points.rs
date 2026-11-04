//! A live's channel points, with the web `auth-token` (none: nothing shows): the viewer's
//! balance, refreshed every minute, and the channel's rewards in a popover, under the chat's field
//! ([`PointsFooter`], ADR 0023). Its predictions with their bets, and its polls
//! ([`super::polls`]), are pinned over the chat as Twitch does (`ui::chat::pinned_stack`): folded
//! bars with their state, one card unfolded at a time, new ones folded. Polls show with or without
//! points. Spending asks for a second click ("Confirmer", within 8 s; a bet's shows the amount it
//! sends), since it cannot be undone; a free vote takes one.
//!
//! Every minute the live really plays (mpv neither paused nor buffering), Peeby reports it to
//! Twitch as its web player does (`twitch::spade`): without it, watching earns no points.
//!
//! The bonus chest is not claimed here: Twitch's integrity check refuses its mutation outside the
//! browser (docs/twitch.md, "Channel points"). When one waits, a button opens the channel's chat
//! on twitch.tv, where it is claimed with a click.
//!
//! In debug builds, the fake chat (`super::fake_chat`) drives the same view from a local state
//! ([`Backend::Fake`]): nothing is read from Twitch, spent or reported then.

#[cfg(debug_assertions)]
use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
  Anchor, AnyElement, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Task, WeakEntity,
  Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::popover::Popover;
use gpui_component::{Disableable, Sizable, h_flex, v_flex};
use twitch::points::{Points, Prediction, PredictionStatus, Reward};
use ui::assets::Icon;
use ui::chat::{Pinned, Unfolded, pinned_stack};
use ui::i18n::t;
use ui::images::Images;
use ui::picture::{self, Picture};
use ui::player::PlayerView;
use ui::{runtime, tf, theme};

use super::cards;
use super::polls::{self, Poll};

/// How often the balance and the rewards are read again.
const POINTS_EVERY: Duration = Duration::from_secs(60);
/// How often a minute watched is reported.
const WATCHED_EVERY: Duration = Duration::from_secs(60);
/// How often the predictions are read again (a bet window is often 2 to 5 minutes).
const PREDICTIONS_EVERY: Duration = Duration::from_secs(20);
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
fn percent(n: impl std::fmt::Display) -> String {
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
  /// The channel's polls (only the fake chat's for now, see [`super::polls`]).
  polls: Vec<Poll>,
  /// The pinned item unfolded over the chat.
  unfolded: Unfolded,
  /// A vote is on its way.
  voting: bool,
  /// The last vote failed: why.
  vote_error: Option<SharedString>,
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
      predictions: Vec::new(),
      off: false,
      confirm: None,
      busy: false,
      share: 10,
      outcome: None,
      outcome_of_bet: false,
      polls: Vec::new(),
      unfolded: Unfolded::default(),
      voting: false,
      vote_error: None,
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
              .any(|p| p.status == PredictionStatus::Active)
            || this.polls.iter().any(|p| !p.ended)
          {
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
    self.load_polls(cx);
  }

  /// The channel's polls: the fake chat's; none on Twitch yet (its source is not captured).
  #[cfg(debug_assertions)]
  fn load_polls(&mut self, cx: &mut Context<Self>) {
    if let Backend::Fake(fake) = &self.backend {
      self.polls = fake.borrow().polls();
      self.fold_gone();
      cx.notify();
    }
  }

  /// Folds the pinned item that is gone.
  fn fold_gone(&mut self) {
    let ids: Vec<String> = self
      .predictions
      .iter()
      .map(|p| p.id.clone())
      .chain(self.polls.iter().map(|p| p.id.clone()))
      .collect();
    self.unfolded.keep_only(ids.iter().map(String::as_str));
  }

  /// A free vote for `choice` in `poll`: one click.
  fn vote(&mut self, poll: String, choice: String, cx: &mut Context<Self>) {
    if self.voting {
      return;
    }
    let send: Later<()> = match &self.backend {
      // Unreachable while Twitch's polls are not read: none shows.
      Backend::Twitch(_) => {
        tracing::debug!("Twitch poll vote ({poll}, {choice}): polls are not read from Twitch yet");
        Box::pin(std::future::ready(Some(Err(twitch::Error::Refused(
          "POLLS_NOT_SUPPORTED".into(),
        )))))
      }
      #[cfg(debug_assertions)]
      Backend::Fake(fake) => {
        let (fake, wait) = (
          fake.clone(),
          cx.background_executor().timer(FAKE_SPEND_DELAY),
        );
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
          if let Some(Err(e)) = result {
            tracing::warn!("Twitch poll vote: {e}");
            this.vote_error = Some(refusal(&e).into());
          }
          #[cfg(debug_assertions)]
          this.load_polls(cx);
          cx.notify();
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
            Ok(points) if points.enabled => this.points = Some(points),
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
    // Only the player that started last (the one on screen or in the mini-player): one live at a
    // time.
    let current = ui::player::now_playing(cx).map(|p| p.entity_id());
    let playing = self.player.upgrade().is_some_and(|player| {
      if current != Some(player.entity_id()) {
        return false;
      }
      let player = player.read(cx);
      player.is_live() && player.property("core-idle").as_deref() == Some("no")
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
  fn prediction_body(&self, p: &Prediction, cx: &mut Context<Self>) -> AnyElement {
    let total: u64 = p.outcomes.iter().map(|o| o.points).sum();
    let open = p.status == PredictionStatus::Active && p.bet.is_none();
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
      let spend = Spend::Bet {
        event: p.id.clone(),
        outcome: outcome.id.clone(),
      };
      let confirming = self.confirming(&spend);
      let pending_amount = self.confirm.as_ref().map_or(amount, |p| p.amount);
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
                  .child(outcome.title.clone()),
              )
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
                  .w(gpui::relative(share as f32))
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
  fn poll_body(&self, poll: &Poll, cx: &mut Context<Self>) -> AnyElement {
    let total = poll.total();
    let open = !poll.ended && poll.voted.is_none();
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
      let mine = poll.voted.as_deref() == Some(choice.id.as_str());
      let won = winner.as_deref() == Some(choice.id.as_str());
      let (poll_id, choice_id) = (poll.id.clone(), choice.id.clone());
      // While it takes the viewer's vote, the whole choice is the button.
      let votable = open && !self.voting;
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
                    .w(gpui::relative(share as f32))
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
        el.child(div().text_xs().text_color(theme::muted()).child(t(
          "Click a choice to vote.",
          "Cliquez sur un choix pour voter.",
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
  fn pinned(&self, cx: &mut Context<Self>) -> Vec<Pinned> {
    let mut items = Vec::new();
    let now = polls::now_millis();
    if !self.off && self.points.is_some() {
      for p in &self.predictions {
        let open = self.unfolded.is(&p.id);
        items.push(Pinned {
          id: p.id.clone().into(),
          icon: Icon::Points,
          accent: outcome_color("BLUE"),
          kind: t("Prediction", "Prédiction").into(),
          title: p.title.clone().into(),
          status: prediction_status(p).into(),
          body: open.then(|| self.prediction_body(p, cx)),
          time_left: (p.status == PredictionStatus::Active)
            .then(|| {
              let length = i64::try_from(p.window_seconds).ok()? * 1000;
              polls::share_left(p.created_at?, length, now)
            })
            .flatten(),
        });
      }
    }
    for poll in &self.polls {
      let open = self.unfolded.is(&poll.id);
      items.push(Pinned {
        id: poll.id.clone().into(),
        icon: Icon::Poll,
        accent: theme::accent(),
        kind: t("Poll", "Sondage").into(),
        title: poll.title.clone().into(),
        status: poll.status(now).into(),
        body: open.then(|| self.poll_body(poll, cx)),
        time_left: poll.time_left(now),
      });
    }
    items
  }
}

/// A prediction's state in a few words, for its bar (folded, it is all that shows).
fn prediction_status(p: &Prediction) -> String {
  match (p.status, &p.bet) {
    (_, Some((outcome, points))) => {
      let title = p
        .outcomes
        .iter()
        .find(|o| &o.id == outcome)
        .map_or("", |o| o.title.as_str());
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
    let claim = self.points.as_ref().and_then(|p| p.claim.clone());
    let login = self.login.clone();
    h_flex()
      .gap_1()
      .items_center()
      .child(self.balance_chip(window, cx))
      .when(claim.is_some(), |row| {
        row.child(
          Button::new("twitch-points-bonus")
            .small()
            .primary()
            .icon(Icon::Gift)
            .label("Bonus")
            .tooltip(t(
              "A bonus is waiting: claim it in the chat on twitch.tv",
              "Un bonus attend : le réclamer dans le chat de twitch.tv",
            ))
            .on_click(move |_, _, cx| {
              cx.open_url(&format!("https://www.twitch.tv/popout/{login}/chat"))
            }),
        )
      })
      .into_any_element()
  }
}

/// The predictions and polls pinned over the chat (`ChatPanel::set_pinned`).
impl Render for PointsView {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let items = self.pinned(cx);
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
  use super::*;

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
