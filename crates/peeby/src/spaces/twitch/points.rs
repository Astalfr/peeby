//! A live's channel points, in the watch page's header, with the web `auth-token` (none: nothing
//! shows): the viewer's balance, refreshed every minute; the channel's rewards in a popover;
//! its predictions with their bets. Spending asks for a second click ("Confirmer", within 8 s; a
//! bet's shows the amount it sends), since it cannot be undone.
//!
//! The bonus chest is not claimed here: Twitch's integrity check refuses its mutation outside the
//! browser (docs/twitch.md, "Channel points"). When one waits, a button opens the channel's chat
//! on twitch.tv, where it is claimed with a click.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui::prelude::*;
use gpui::{
  Anchor, Context, ElementId, Entity, FontWeight, Hsla, SharedString, Task, Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::popover::Popover;
use gpui_component::{Disableable, Sizable, h_flex, v_flex};
use twitch::points::{Points, Prediction, PredictionStatus, Reward};
use ui::assets::Icon;
use ui::{runtime, theme};

use super::cards;

/// How often the balance and the rewards are read again.
const POINTS_EVERY: Duration = Duration::from_secs(60);
/// How often the predictions are read again (a bet window is often 2 to 5 minutes).
const PREDICTIONS_EVERY: Duration = Duration::from_secs(20);
/// How long a first click waits for its confirmation.
const CONFIRM_FOR: Duration = Duration::from_secs(8);
/// Twitch's smallest bet.
const MIN_BET: u64 = 10;
/// The bet amounts offered, as shares of the balance.
const SHARES: [(u64, &str); 4] = [(10, "10 %"), (25, "25 %"), (50, "50 %"), (100, "Tout")];

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
  pub fn new(login: String, client: twitch::Client, cx: &mut Context<Self>) -> Self {
    let mut view = Self {
      login,
      client,
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
          self.outcome = Some(("Écrivez le message de la récompense.".into(), false));
          return cx.notify();
        }
        let done = format!("« {} » échangée.", reward.title).into();
        (Order::Redeem(points.channel_id.clone(), reward, text), done)
      }
      Spend::Bet { event, outcome } => {
        // The amount the button showed, not one made from a balance read since.
        let amount = pending.amount;
        let done = format!("Pari de {} points placé.", cards::grouped(amount)).into();
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
    Popover::new("twitch-points-rewards")
      .anchor(Anchor::TopRight)
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
          .tooltip("Points de chaîne · récompenses"),
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
      list = list.child(
        div()
          .px_2()
          .text_sm()
          .text_color(theme::muted())
          .child("Cette chaîne n'a pas de récompense personnalisée."),
      );
    }
    let now = now_millis();
    for reward in &points.rewards {
      let confirming = self.confirming(&Spend::Reward(reward.id.clone()));
      let cooling = reward.cooldown_until.is_some_and(|until| until > now);
      let affordable = reward.cost <= points.balance;
      let usable = reward.available && !cooling && affordable && !self.busy;
      let label = if confirming {
        "Confirmer".to_owned()
      } else if cooling {
        "En recharge".to_owned()
      } else if !reward.available {
        "Indisponible".to_owned()
      } else {
        "Échanger".to_owned()
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
                        "{} points{}",
                        cards::grouped(reward.cost),
                        if reward.sub_only { " · abonnés" } else { "" }
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
          .child(div().font_weight(FontWeight::SEMIBOLD).child("Récompenses"))
          .child(
            div()
              .text_sm()
              .text_color(theme::muted())
              .child(format!("{} points", cards::grouped(points.balance))),
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
        format!(
          "Votre pari : {} points sur « {title} »",
          cards::grouped(*points)
        )
      }
      (PredictionStatus::Active, None) => match remaining(p) {
        Some(left) if left > 0 => {
          format!("Paris ouverts · {}", ui::player::format_time(left as f64))
        }
        _ => "Paris en cours de fermeture".into(),
      },
      (PredictionStatus::Locked, None) => "Paris clos · en attente du résultat".into(),
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
                "{:.0} %{}",
                share * 100.0,
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
                    format!("Confirmer {}", cards::grouped(pending_amount))
                  } else {
                    format!("Parier {}", cards::grouped(amount))
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
          .child(div().text_xs().text_color(theme::subtle()).child(format!(
            "{} points · {} parieurs",
            cards::grouped(outcome.points),
            cards::grouped(outcome.users)
          ))),
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
            .child("Mise"),
        )
        .children(SHARES.iter().map(|&(share, label)| {
          Button::new(ElementId::Name(format!("twitch-bet-share-{share}").into()))
            .xsmall()
            .when(self.share == share, |b| b.primary())
            .when(self.share != share, |b| b.ghost())
            .label(label)
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
          .child("Prédiction"),
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

impl Render for PointsView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.off || self.points.is_none() {
      return div().into_any_element();
    }
    if self.input.is_none() {
      self.input = Some(cx.new(|cx| InputState::new(window, cx).placeholder("Votre message")));
    }
    let claim = self.points.as_ref().and_then(|p| p.claim.clone());
    let login = self.login.clone();
    let predictions: Vec<_> = self
      .predictions
      .clone()
      .iter()
      .map(|p| self.prediction_card(p, cx).into_any_element())
      .collect();
    v_flex()
      .w(px(320.))
      .flex_none()
      .gap_2()
      .items_end()
      .child(
        h_flex()
          .gap_1()
          .when(claim.is_some(), |row| {
            row.child(
              Button::new("twitch-points-bonus")
                .small()
                .primary()
                .icon(Icon::Gift)
                .label("Bonus")
                .tooltip("Un bonus attend : le réclamer dans le chat de twitch.tv")
                .on_click(move |_, _, cx| {
                  cx.open_url(&format!("https://www.twitch.tv/popout/{login}/chat"))
                }),
            )
          })
          .child(self.balance_chip(cx)),
      )
      .when(self.popover_closed_outcome(), |el| {
        el.children(self.outcome_line())
      })
      .children(predictions)
      .into_any_element()
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
      "NOT_ENOUGH_POINTS" => "Pas assez de points.".into(),
      "COOLDOWN" | "GLOBAL_COOLDOWN" => "Cette récompense est en recharge.".into(),
      "MAX_PER_STREAM" | "MAX_PER_USER_PER_STREAM" => "Limite atteinte pour ce live.".into(),
      "DISABLED" | "PAUSED" | "OUT_OF_STOCK" => "Cette récompense n'est pas disponible.".into(),
      "NOT_FOUND" => "Introuvable : elle vient peut-être de se terminer.".into(),
      code => format!("Twitch a refusé ({code})."),
    },
    twitch::Error::IntegrityRequired => {
      "Twitch demande une vérification que seul son site peut faire.".into()
    }
    twitch::Error::NotLoggedIn => "Twitch n'accepte plus le jeton auth-token.".into(),
    _ => "Twitch n'a pas répondu. Réessayez.".into(),
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
  fn refusals_read_in_french() {
    let refused = |code: &str| refusal(&twitch::Error::Refused(code.into()));
    assert_eq!(refused("NOT_ENOUGH_POINTS"), "Pas assez de points.");
    assert_eq!(refused("WHATEVER"), "Twitch a refusé (WHATEVER).");
  }

  #[test]
  fn colors_are_read() {
    assert_eq!(color("#8B3EF7"), Some(gpui::rgb(0x8b3ef7).into()));
    assert_eq!(color("8B3EF7"), None);
    assert_eq!(first_line("a\nb"), "a");
  }
}
