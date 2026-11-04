//! A channel's polls, pinned over its chat beside its predictions (`points::PointsView`). Only
//! the debug builds' fake chat fills them for now: Twitch's source for a live's polls is not
//! captured yet (CLAUDE.md: a saved response before its parser), so a live shows none.
//! Polls are seen by every viewer, with or without points; a free vote takes one click.

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
  /// The viewer's choice, by id.
  pub voted: Option<String>,
}

/// One answer of a poll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
  pub id: String,
  pub title: String,
  pub votes: u64,
}

impl Poll {
  /// Seconds left to vote; `None` once ended.
  pub fn remaining(&self, now_millis: i64) -> Option<i64> {
    if self.ended {
      return None;
    }
    let end = self.started_at + i64::try_from(self.duration_seconds).ok()? * 1000;
    Some(((end - now_millis) / 1000).max(0))
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
        if self.voted.is_some() {
          tf!("Voted · {left}", "A voté · {left}")
        } else {
          tf!("Voting open · {left}", "Vote ouvert · {left}")
        }
      }
    }
  }
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
      voted: None,
    }
  }

  #[test]
  fn polls_read() {
    let mut p = poll(&[3, 5]);
    assert_eq!(p.remaining(20_000), Some(40));
    assert_eq!(p.remaining(90_000), Some(0));
    assert_eq!(p.status(20_000), "Voting open · 0:40");
    p.voted = Some("1".into());
    assert_eq!(p.status(20_000), "Voted · 0:40");
    p.ended = true;
    assert_eq!(p.total(), 8);
    assert_eq!(p.status(0), "Ended · “c1” wins");
    assert_eq!(poll(&[2, 2]).winner(), None);
    assert_eq!(poll(&[0, 0]).winner(), None);
  }
}
