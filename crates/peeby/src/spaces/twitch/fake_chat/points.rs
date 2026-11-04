//! The fake channel's points, predictions and polls, read and spent by `points::PointsView`
//! through `Backend::Fake`: what the debug page's controls set, spends that take the points (or
//! are refused with a chosen code), predictions with fake bettors, polls with fake voters.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use twitch::points::{Outcome, Points, Prediction, PredictionStatus, Reward};
use ui::chat::fake::Rng;

use super::super::points::{ENDED_SHOWN, Order};
use super::super::polls::{Choice, Poll};
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
  /// Other viewers vote in the polls taking votes.
  pub voters: bool,
  polls: Vec<Poll>,
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
      voters: true,
      polls: Vec::new(),
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

  pub fn polls(&self) -> Vec<Poll> {
    self.polls.clone()
  }

  /// The viewer's free vote: one per poll, while it takes votes.
  pub fn vote(&mut self, poll: &str, choice: &str) -> twitch::Result<()> {
    let refused = |code: &str| Err(twitch::Error::Refused(code.into()));
    if let Some(code) = self.refuse {
      return refused(code);
    }
    let Some(poll) = self.polls.iter_mut().find(|p| p.id == poll && !p.ended) else {
      return refused("NOT_FOUND");
    };
    if poll.voted.is_some() {
      return refused("ALREADY_VOTED");
    }
    let Some(chosen) = poll.choices.iter_mut().find(|c| c.id == choice) else {
      return refused("NOT_FOUND");
    };
    chosen.votes += 1;
    poll.voted = Some(choice.to_owned());
    Ok(())
  }

  /// A new poll taking votes for `duration` seconds, with `choices` answers (2 to 5).
  pub fn start_poll(&mut self, choices: usize, duration: u64) {
    self.made += 1;
    let choices = choices.clamp(2, 5);
    let (title, answers): (&str, &[&str]) = if choices == 2 {
      ("On continue la partie ?", &["Oui", "Non"])
    } else {
      (
        "Quel jeu après ? (un titre de sondage assez long pour passer sur plusieurs lignes)",
        &[
          "Celeste",
          "Hades II",
          "Outer Wilds",
          "Un jeu au nom vraiment très long pour voir la troncature",
          "Surprise",
        ],
      )
    };
    let id = format!("fake-poll-{}", self.made);
    self.polls.push(Poll {
      choices: answers
        .iter()
        .take(choices)
        .enumerate()
        .map(|(i, title)| Choice {
          id: format!("{id}-{i}"),
          title: (*title).to_owned(),
          votes: 0,
        })
        .collect(),
      id,
      title: title.into(),
      started_at: now_millis(),
      duration_seconds: duration,
      ended: false,
      ended_at: None,
      voted: None,
    });
  }

  /// Ends the first poll taking votes: its result stays shown for a while ([`ENDED_SHOWN`]).
  pub fn end_poll(&mut self) {
    if let Some(poll) = self.polls.iter_mut().find(|p| !p.ended) {
      poll.ended = true;
      poll.ended_at = Some(now_millis());
    }
  }

  /// Removes the first poll, as Twitch does a while after its end.
  pub fn remove_poll(&mut self) {
    if !self.polls.is_empty() {
      self.polls.remove(0);
    }
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
      winner: None,
      ended_at: None,
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

  /// The first prediction not ended yet.
  fn running(&mut self) -> Option<&mut Prediction> {
    self.predictions.iter_mut().find(|p| !p.status.ended())
  }

  /// Ends the first running prediction with its answer `winner`: a bet on it pays its share of
  /// every point bet. It stays shown with its result for a while ([`ENDED_SHOWN`]).
  pub fn resolve(&mut self, winner: usize) {
    let Some(p) = self.running() else {
      return;
    };
    let Some(won) = p.outcomes.get(winner).map(|o| o.id.clone()) else {
      return;
    };
    p.status = PredictionStatus::Resolved;
    p.winner = Some(won);
    p.ended_at = Some(now_millis());
    let payout = p
      .bet
      .as_ref()
      .and_then(|(outcome, amount)| p.payout(outcome, *amount));
    self.balance += payout.unwrap_or(0);
  }

  /// Cancels the first running prediction: its bet comes back.
  pub fn cancel(&mut self) {
    let Some(p) = self.running() else {
      return;
    };
    p.status = PredictionStatus::Canceled;
    p.ended_at = Some(now_millis());
    let refund = p.bet.as_ref().map_or(0, |(_, amount)| *amount);
    self.balance += refund;
  }

  /// A second goes by: the bets and votes of an elapsed window close, and fake bettors and voters
  /// join the others.
  pub fn tick(&mut self) {
    let now = now_millis();
    // Ended ones go once shown long enough, as Twitch stops returning them.
    let shown_for = i64::try_from(ENDED_SHOWN.as_millis()).unwrap_or(i64::MAX);
    self
      .predictions
      .retain(|p| !p.status.ended() || p.ended_at.is_some_and(|end| now - end < shown_for));
    self
      .polls
      .retain(|p| !p.ended || p.ended_at.is_some_and(|end| now - end < shown_for));
    for poll in &mut self.polls {
      if poll.ended {
        continue;
      }
      if poll.remaining(now) == Some(0) {
        poll.ended = true;
        poll.ended_at = Some(now);
        continue;
      }
      if self.voters {
        for choice in &mut poll.choices {
          choice.votes += self.rng.below(4) as u64;
        }
      }
    }
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
    // Still shown, with its winner.
    let ended = points.predictions().expect("the predictions");
    assert_eq!(ended[0].status, PredictionStatus::Resolved);
    assert_eq!(
      ended[0].winner.as_deref(),
      Some(ended[0].outcomes[0].id.as_str())
    );
    // Resolving again does nothing: it has ended.
    points.resolve(1);
    assert_eq!(points.balance, 1_300);
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
    assert!(
      points
        .predictions()
        .is_ok_and(|p| p[0].status == PredictionStatus::Canceled)
    );
    points.refuse = Some("COOLDOWN");
    let reward = points.reward_list().remove(0);
    assert!(matches!(
      points.spend(&Order::Redeem(String::new(), reward, None)),
      Err(twitch::Error::Refused(code)) if code == "COOLDOWN"
    ));
  }

  #[test]
  fn one_vote_per_poll_and_ended_polls_stay() {
    let mut points = FakePoints {
      voters: false,
      ..FakePoints::default()
    };
    points.start_poll(4, 60);
    let poll = points.polls()[0].clone();
    assert_eq!(poll.choices.len(), 4);
    assert!(points.vote(&poll.id, &poll.choices[2].id).is_ok());
    assert!(points.vote(&poll.id, &poll.choices[1].id).is_err());
    assert_eq!(
      points.polls()[0].voted.as_deref(),
      Some(poll.choices[2].id.as_str())
    );
    points.end_poll();
    assert!(points.vote(&poll.id, &poll.choices[0].id).is_err());
    assert!(points.polls()[0].ended);
    points.remove_poll();
    assert!(points.polls().is_empty());
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
