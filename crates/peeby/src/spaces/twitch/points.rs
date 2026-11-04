//! A live's channel points, with the web `auth-token` (none: nothing shows): the viewer's
//! balance, refreshed every minute, and the channel's rewards in a popover, under the chat's field
//! ([`PointsFooter`], ADR 0023); its predictions with their bets at the watch page header's right. Spending asks for a second click ("Confirmer", within 8 s; a
//! bet's shows the amount it sends), since it cannot be undone.
//!
//! Every minute the live really plays (mpv neither paused nor buffering), Peeby reports it to
//! Twitch as its web player does (`twitch::spade`): without it, watching earns no points.
//!
//! The bonus chest is not claimed here: Twitch's integrity check refuses its mutation outside the
//! browser (docs/twitch.md, "Channel points"). When one waits, a button opens the channel's chat
//! on twitch.tv, where it is claimed with a click.

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
use ui::i18n::t;
use ui::player::PlayerView;
use ui::{runtime, tf, theme};

use super::cards;

/// How often the balance and the rewards are read again.
const POINTS_EVERY: Duration = Duration::from_secs(60);
/// How often a minute watched is reported.
const WATCHED_EVERY: Duration = Duration::from_secs(60);
/// How often the predictions are read again (a bet window is often 2 to 5 minutes).
const PREDICTIONS_EVERY: Duration = Duration::from_secs(20);
/// How long a first click waits for its confirmation.
const CONFIRM_FOR: Duration = Duration::from_secs(8);
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

/// A spend as it is sent: a reward on a channel with the viewer's text, or a bet of points on an
/// outcome of a prediction.
enum Order {
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
  client: twitch::Client,
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
  /// That outcome is a bet's (shown under the balance; a reward's shows in its popover).
  outcome_of_bet: bool,
  /// The text of a reward that asks for one (made at the first render: it needs the window).
  input: Option<Entity<InputState>>,
  _tasks: Vec<Task<()>>,
}

impl PointsView {
  pub fn new(
    login: String,
    client: twitch::Client,
    player: WeakEntity<PlayerView>,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut view = Self {
      login,
      client,
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
    // The countdown of a prediction taking bets, and the end of a confirmation's wait.
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

  fn load_points(&mut self, cx: &mut Context<Self>) -> Task<()> {
    let (client, login) = (self.client.clone(), self.login.clone());
    let fetch = runtime::spawn(cx, async move { client.points(&login).await });
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
    let client = self.client.clone();
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
    let (client, login) = (self.client.clone(), self.login.clone());
    let fetch = runtime::spawn(cx, async move { client.predictions(&login).await });
    cx.spawn(async move |this, cx| {
      let Some(result) = fetch.await else {
        return;
      };
      this
        .update(cx, |this, cx| {
          match result {
            Ok(predictions) => this.predictions = predictions,
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
    let client = self.client.clone();
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
    let send = runtime::spawn(cx, async move {
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
    });
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

  fn balance_chip(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let points = self.points.clone().unwrap_or_default();
    let name = points.name.clone().unwrap_or_else(|| "points".into());
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
          .icon(Icon::Points)
          .label(format!("{} {name}", cards::grouped(points.balance)))
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

  fn prediction_card(&self, p: &Prediction, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let total: u64 = p.outcomes.iter().map(|o| o.points).sum();
    let open = p.status == PredictionStatus::Active && p.bet.is_none();
    let status = match (p.status, &p.bet) {
      (_, Some((outcome, points))) => {
        let title = p
          .outcomes
          .iter()
          .find(|o| &o.id == outcome)
          .map_or("", |o| o.title.as_str());
        let points = points_text(*points);
        tf!(
          "Your bet: {points} on “{title}”",
          "Votre pari : {points} sur « {title} »"
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
    };
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
                  .xsmall()
                  .when(confirming, |b| b.primary())
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
      .p_3()
      .gap_2()
      .rounded(theme::RADIUS)
      .bg(theme::surface())
      .border_1()
      .border_color(theme::border())
      .child(
        div()
          .text_xs()
          .text_color(theme::muted())
          .font_weight(FontWeight::MEDIUM)
          .child(t("Prediction", "Prédiction")),
      )
      .child(
        div()
          .text_sm()
          .font_weight(FontWeight::SEMIBOLD)
          .child(p.title.clone()),
      )
      .child(div().text_xs().text_color(theme::muted()).child(status))
      .child(outcomes)
      .children(shares)
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
      .child(self.balance_chip(cx))
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

/// The predictions (and a bet's outcome), at the header's right.
impl Render for PointsView {
  fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.off || self.points.is_none() {
      return div().into_any_element();
    }
    let predictions: Vec<_> = self
      .predictions
      .clone()
      .iter()
      .map(|p| self.prediction_card(p, cx).into_any_element())
      .collect();
    let outcome = self.popover_closed_outcome();
    if predictions.is_empty() && !outcome {
      return div().into_any_element();
    }
    v_flex()
      .w(px(320.))
      .flex_none()
      .gap_2()
      .items_end()
      .when(outcome, |el| el.children(self.outcome_line()))
      .children(predictions)
      .into_any_element()
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

impl PointsView {
  /// A bet's outcome shows under the balance (a reward's shows in its popover).
  fn popover_closed_outcome(&self) -> bool {
    self.outcome.is_some() && self.outcome_of_bet
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
