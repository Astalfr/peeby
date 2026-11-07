//! A channel's polls, pinned over its chat beside its predictions (`points::PointsView`): a
//! live's from Twitch (`twitch::polls`, read every 10 s, anonymous), or the fake chat's. Polls
//! are seen by every viewer, with or without points; a free vote takes one click and needs the
//! auth-token. Twitch does not give the viewer's own votes back: Peeby remembers those it sent
//! for the session.

use std::time::{SystemTime, UNIX_EPOCH};

use ui::i18n::t;
use ui::tf;

/// A poll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Poll {
  pub id: String,
  pub title: String,
  pub choices: Vec<Choice>,
  /// When it started, Unix milliseconds.
  pub started_at: i64,
  /// How long it takes votes.
  pub duration_seconds: u64,
  /// Ended: the votes are final.
  pub ended: bool,
  /// When it ended, Unix milliseconds, when known.
  pub ended_at: Option<i64>,
  /// The viewer's choices, by id.
  pub voted: Vec<String>,
  /// A viewer may vote for several choices.
  pub multichoice: bool,
  /// Only subscribers vote.
  pub subscriber_only: bool,
}

/// One answer of a poll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
  pub id: String,
  pub title: String,
  pub votes: u64,
}

impl Poll {
  /// A live's poll from Twitch, with the choices the viewer voted for.
  pub fn from_twitch(poll: twitch::polls::Poll, voted: Vec<String>) -> Self {
    Self {
      id: poll.id,
      title: poll.title,
      choices: poll
        .choices
        .into_iter()
        .map(|c| Choice {
          id: c.id,
          title: c.title,
          votes: c.votes,
        })
        .collect(),
      started_at: poll.started_at.unwrap_or(0),
      duration_seconds: poll.duration_seconds,
      ended: !poll.active,
      ended_at: poll.ended_at,
      voted,
      multichoice: poll.multichoice,
      subscriber_only: poll.subscriber_only,
    }
  }

  /// Whether `choice` can take the viewer's vote: the poll runs, and they have not voted yet (for
  /// it, when several choices are allowed).
  pub fn votable(&self, choice: &str) -> bool {
    !self.ended
      && !self.voted.iter().any(|c| c == choice)
      && (self.voted.is_empty() || self.multichoice)
  }

  /// Seconds left to vote; `None` once ended.
  pub fn remaining(&self, now_millis: i64) -> Option<i64> {
    if self.ended {
      return None;
    }
    let end = self.started_at + i64::try_from(self.duration_seconds).ok()? * 1000;
    Some(((end - now_millis) / 1000).max(0))
  }

  /// The share of its time left, 1 to 0; `None` once ended.
  pub fn time_left(&self, now_millis: i64) -> Option<f32> {
    if self.ended {
      return None;
    }
    let length = i64::try_from(self.duration_seconds).ok()? * 1000;
    share_left(self.started_at, length, now_millis)
  }

  /// Every vote.
  pub fn total(&self) -> u64 {
    self.choices.iter().map(|c| c.votes).sum()
  }

  /// The choice with the most votes, alone; `None` on a tie or without votes.
  pub fn winner(&self) -> Option<&Choice> {
    let best = self
      .choices
      .iter()
      .map(|c| c.votes)
      .max()
      .filter(|&v| v > 0)?;
    let mut top = self.choices.iter().filter(|c| c.votes == best);
    let first = top.next()?;
    top.next().is_none().then_some(first)
  }

  /// Its state in a few words, for its bar.
  pub fn status(&self, now_millis: i64) -> String {
    match self.remaining(now_millis) {
      None => match self.winner() {
        Some(choice) => tf!(
          "Ended · “{}” wins",
          "Terminé · « {} » l'emporte",
          choice.title
        ),
        None => t("Ended · tie", "Terminé · égalité").into(),
      },
      Some(left) => {
        let left = ui::player::format_time(left as f64);
        if !self.voted.is_empty() {
          tf!("Voted · {left}", "A voté · {left}")
        } else {
          tf!("Voting open · {left}", "Vote ouvert · {left}")
        }
      }
    }
  }
}

/// The share left of `length` ms started at `start`, at `now` (Unix milliseconds), 1 to 0.
pub fn share_left(start: i64, length: i64, now: i64) -> Option<f32> {
  (length > 0).then(|| ((start + length - now) as f32 / length as f32).clamp(0., 1.))
}

pub fn now_millis() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn poll(votes: &[u64]) -> Poll {
    Poll {
      id: "p".into(),
      title: "?".into(),
      choices: votes
        .iter()
        .enumerate()
        .map(|(i, &votes)| Choice {
          id: i.to_string(),
          title: format!("c{i}"),
          votes,
        })
        .collect(),
      started_at: 0,
      duration_seconds: 60,
      ended: false,
      ended_at: None,
      voted: Vec::new(),
      multichoice: false,
      subscriber_only: false,
    }
  }

  #[test]
  fn polls_read() {
    let mut p = poll(&[3, 5]);
    assert_eq!(p.remaining(20_000), Some(40));
    assert_eq!(p.remaining(90_000), Some(0));
    assert_eq!(p.status(20_000), "Voting open · 0:40");
    assert_eq!(p.time_left(15_000), Some(0.75));
    assert_eq!(p.time_left(70_000), Some(0.));
    p.voted = vec!["1".into()];
    assert!(!p.votable("0"));
    p.multichoice = true;
    assert!(p.votable("0") && !p.votable("1"));
    assert_eq!(p.status(20_000), "Voted · 0:40");
    p.ended = true;
    assert_eq!(p.time_left(15_000), None);
    assert_eq!(p.total(), 8);
    assert_eq!(p.status(0), "Ended · “c1” wins");
    assert_eq!(poll(&[2, 2]).winner(), None);
    assert_eq!(poll(&[0, 0]).winner(), None);
  }
}
