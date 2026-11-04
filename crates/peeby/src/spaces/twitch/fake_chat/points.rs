//! The fake channel's points and predictions, read and spent by `points::PointsView` through
//! `Backend::Fake`: what the debug page's controls set, spends that take the points (or are
//! refused with a chosen code), and predictions with fake bettors.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use twitch::points::{Outcome, Points, Prediction, PredictionStatus, Reward};
use ui::chat::fake::Rng;

use super::super::points::Order;
use super::SCHEME;

/// How long the "Shoutout" reward cools down once redeemed.
const COOLDOWN_MS: i64 = 60_000;

pub struct FakePoints {
  /// The channel has points (else the view shows nothing).
  pub enabled: bool,
  pub balance: u64,
  /// A bonus chest waits.
  pub bonus: bool,
  /// The channel has custom rewards.
  pub rewards: bool,
  /// The code every spend is refused with; `None`: spends go through.
  pub refuse: Option<&'static str>,
  /// Other viewers bet on the predictions taking bets.
  pub bettors: bool,
  predictions: Vec<Prediction>,
  /// Reward id → end of its cooldown, Unix milliseconds.
  cooldowns: HashMap<String, i64>,
  /// Predictions made so far, for their ids.
  made: u64,
  rng: Rng,
}

impl Default for FakePoints {
  fn default() -> Self {
    Self {
      enabled: true,
      balance: 12_500,
      bonus: false,
      rewards: true,
      refuse: None,
      bettors: true,
      predictions: Vec::new(),
      cooldowns: HashMap::new(),
      made: 0,
      rng: Rng::new(0x5eed),
    }
  }
}

impl FakePoints {
  pub fn points(&self) -> twitch::Result<Points> {
    Ok(Points {
      channel_id: "fake-channel".into(),
      // No viewer nor broadcast: nothing could be reported even by mistake.
      viewer_id: None,
      broadcast_id: None,
      enabled: self.enabled,
      balance: self.balance,
      claim: self.bonus.then(|| "fake-claim".into()),
      multiplier: 1.,
      name: Some("Peebs".into()),
      icon: Some(format!("{SCHEME}points/icon")),
      rewards: if self.rewards {
        self.reward_list()
      } else {
        Vec::new()
      },
    })
  }

  pub fn predictions(&self) -> twitch::Result<Vec<Prediction>> {
    Ok(self.predictions.clone())
  }

  /// Every case of the rewards' popover: a plain one, one with a text to write, subscribers
  /// only, unavailable, one that cools down once redeemed, one nobody can afford.
  fn reward_list(&self) -> Vec<Reward> {
    let now = now_millis();
    let reward = |id: &str, title: &str, cost: u64, color: &str| Reward {
      id: id.into(),
      title: title.into(),
      cost,
      color: Some(color.into()),
      available: true,
      cooldown_until: self.cooldowns.get(id).copied().filter(|&until| until > now),
      ..Reward::default()
    };
    vec![
      reward("hydrate", "Hydrate-toi !", 100, "#00C7AC"),
      Reward {
        prompt: Some("Titre et artiste\nUne seule ligne est montrée".into()),
        input_required: true,
        ..reward("song", "Choisir la prochaine musique", 500, "#9147FF")
      },
      Reward {
        sub_only: true,
        ..reward("emote", "Débloquer une emote", 1_000, "#FA1ED2")
      },
      Reward {
        available: false,
        ..reward("bw", "Stream en noir et blanc", 5_000, "#5C5C5C")
      },
      reward("shoutout", "Shoutout (recharge 60 s)", 250, "#FF6905"),
      reward("castle", "Acheter le château", 1_000_000, "#1F69FF"),
    ]
  }

  /// A spend sent by the view: refused with [`Self::refuse`], else as Twitch would take it.
  pub(in crate::spaces::twitch) fn spend(&mut self, order: &Order) -> twitch::Result<()> {
    let refused = |code: &str| Err(twitch::Error::Refused(code.into()));
    if let Some(code) = self.refuse {
      return refused(code);
    }
    match order {
      Order::Redeem(_, reward, _) => {
        if reward.cost > self.balance {
          return refused("NOT_ENOUGH_POINTS");
        }
        if self
          .cooldowns
          .get(&reward.id)
          .is_some_and(|&until| until > now_millis())
        {
          return refused("COOLDOWN");
        }
        self.balance -= reward.cost;
        if reward.id == "shoutout" {
          self
            .cooldowns
            .insert(reward.id.clone(), now_millis() + COOLDOWN_MS);
        }
        Ok(())
      }
      Order::Bet(event, outcome, amount) => {
        if *amount > self.balance {
          return refused("NOT_ENOUGH_POINTS");
        }
        let Some(prediction) = self
          .predictions
          .iter_mut()
          .find(|p| &p.id == event && p.status == PredictionStatus::Active)
        else {
          return refused("NOT_FOUND");
        };
        if prediction.bet.is_some() {
          return refused("ALREADY_BET");
        }
        let Some(chosen) = prediction.outcomes.iter_mut().find(|o| &o.id == outcome) else {
          return refused("NOT_FOUND");
        };
        chosen.points += amount;
        chosen.users += 1;
        prediction.bet = Some((outcome.clone(), *amount));
        self.balance -= amount;
        Ok(())
      }
    }
  }

  /// A new prediction taking bets for `window` seconds, with `outcomes` answers (2 to 10).
  pub fn start_prediction(&mut self, outcomes: usize, window: u64) {
    self.made += 1;
    let outcomes = outcomes.clamp(2, 10);
    let (title, answers): (&str, Vec<String>) = if outcomes == 2 {
      (
        "Il gagne la partie ?",
        vec!["Oui, facile".into(), "Non, il va rager".into()],
      )
    } else {
      (
        "Combien de morts avant le boss ? (une question assez longue pour passer à la ligne)",
        (0..outcomes)
          .map(|i| {
            if i + 1 == outcomes {
              format!("{i} ou plus")
            } else {
              i.to_string()
            }
          })
          .collect(),
      )
    };
    let id = format!("fake-prediction-{}", self.made);
    self.predictions.push(Prediction {
      outcomes: answers
        .into_iter()
        .enumerate()
        .map(|(i, title)| Outcome {
          id: format!("{id}-{i}"),
          title,
          // Twitch's: blue then pink with two answers, all blue but the second beyond.
          color: if i == 1 { "PINK" } else { "BLUE" }.into(),
          points: 0,
          users: 0,
        })
        .collect(),
      id,
      title: title.into(),
      status: PredictionStatus::Active,
      created_at: Some(now_millis()),
      window_seconds: window,
      bet: None,
    });
  }

  /// Closes the bets of the first prediction taking them.
  pub fn lock(&mut self) {
    if let Some(p) = self
      .predictions
      .iter_mut()
      .find(|p| p.status == PredictionStatus::Active)
    {
      p.status = PredictionStatus::Locked;
    }
  }

  /// Ends the first prediction with its answer `winner`: a bet on it pays its share of every
  /// point bet.
  pub fn resolve(&mut self, winner: usize) {
    if self.predictions.is_empty() {
      return;
    }
    let p = self.predictions.remove(0);
    let total: u64 = p.outcomes.iter().map(|o| o.points).sum();
    if let (Some((outcome, amount)), Some(won)) = (&p.bet, p.outcomes.get(winner))
      && &won.id == outcome
      && won.points > 0
    {
      self.balance += amount * total / won.points;
    }
  }

  /// Cancels the first prediction: its bet comes back.
  pub fn cancel(&mut self) {
    if self.predictions.is_empty() {
      return;
    }
    let p = self.predictions.remove(0);
    if let Some((_, amount)) = p.bet {
      self.balance += amount;
    }
  }

  /// A second goes by: the bets of an elapsed window close, and fake bettors join the others.
  pub fn tick(&mut self) {
    let now = now_millis();
    for p in &mut self.predictions {
      if p.status != PredictionStatus::Active {
        continue;
      }
      let end = p
        .created_at
        .and_then(|at| u64::try_from(at).ok())
        .map_or(0, |at| at + p.window_seconds * 1000);
      if u64::try_from(now).unwrap_or(u64::MAX) >= end {
        p.status = PredictionStatus::Locked;
        continue;
      }
      if !self.bettors {
        continue;
      }
      for outcome in &mut p.outcomes {
        let joined = self.rng.below(4) as u64;
        outcome.users += joined;
        outcome.points += joined * (10 + self.rng.below(5_000) as u64);
      }
    }
  }
}

fn now_millis() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn bet(p: &FakePoints, outcome: usize, amount: u64) -> Order {
    let prediction = &p.predictions[0];
    Order::Bet(
      prediction.id.clone(),
      prediction.outcomes[outcome].id.clone(),
      amount,
    )
  }

  #[test]
  fn a_won_bet_pays_its_share() {
    let mut points = FakePoints {
      bettors: false,
      balance: 1_000,
      ..FakePoints::default()
    };
    points.start_prediction(2, 120);
    points.predictions[0].outcomes[1].points = 300;
    let order = bet(&points, 0, 100);
    assert!(points.spend(&order).is_ok());
    assert_eq!(points.balance, 900);
    // Twice: refused.
    assert!(points.spend(&order).is_err());
    points.lock();
    points.resolve(0);
    // 100 of 400 points: the whole 400.
    assert_eq!(points.balance, 1_300);
    assert!(points.predictions().is_ok_and(|p| p.is_empty()));
  }

  #[test]
  fn cancelled_bets_come_back_and_refusals_win() {
    let mut points = FakePoints {
      bettors: false,
      balance: 500,
      ..FakePoints::default()
    };
    points.start_prediction(5, 30);
    assert_eq!(points.predictions[0].outcomes.len(), 5);
    let order = bet(&points, 3, 200);
    assert!(points.spend(&order).is_ok());
    points.cancel();
    assert_eq!(points.balance, 500);
    points.refuse = Some("COOLDOWN");
    let reward = points.reward_list().remove(0);
    assert!(matches!(
      points.spend(&Order::Redeem(String::new(), reward, None)),
      Err(twitch::Error::Refused(code)) if code == "COOLDOWN"
    ));
  }

  #[test]
  fn the_shoutout_cools_down() {
    let mut points = FakePoints::default();
    let shoutout = points
      .reward_list()
      .into_iter()
      .find(|r| r.id == "shoutout")
      .expect("the shoutout");
    let order = Order::Redeem(String::new(), shoutout, None);
    assert!(points.spend(&order).is_ok());
    assert!(points.spend(&order).is_err());
    assert!(
      points
        .reward_list()
        .iter()
        .any(|r| r.id == "shoutout" && r.cooldown_until.is_some())
    );
  }
}
