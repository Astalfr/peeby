//! Low-latency catch-up for a live that keeps close to its edge ([`super::Source::catch_up`]):
//! mpv starts a live some segments from its end and then holds whatever it buffered, so the
//! picture stays that far behind the broadcast. While the live plays at its edge, the player
//! runs a little faster while mpv's buffer stays above a floor, and a little slower once it
//! falls under it: the buffer keeps near the floor, the delay with it, and a stall (which
//! refills the buffer) is caught up afterwards. The audio keeps its pitch (mpv's
//! `audio-pitch-correction`, on by default).
//!
//! The buffer is mpv's `demuxer-cache-duration`, read twice a second; the decision uses its
//! smallest value over the last few seconds, since the buffer swings by about a segment.

use std::collections::VecDeque;
use std::time::Duration;

use gpui::{Context, Task};

use super::PlayerView;

/// How often the buffer is read.
const TICK: Duration = Duration::from_millis(500);

/// Readings the decision looks back on: 4 s.
const SAMPLES: usize = 8;

/// Faster while the buffer stays this far above the floor, seconds, until it is at most
/// [`STOP_ABOVE`] above it.
const START_ABOVE: f64 = 0.3;
const STOP_ABOVE: f64 = 0.1;

/// Far above the floor (after a stall, a quality change): [`FASTER`] rather than [`FAST`].
const FAR_ABOVE: f64 = 1.5;

/// Slower under the floor by this much, seconds, until the buffer is back at the floor.
const SLOW_UNDER: f64 = 0.3;

const FAST: f64 = 1.05;
const FASTER: f64 = 1.1;
const SLOW: f64 = 0.97;

/// The controller's state, reset with each mpv (which starts at 1×).
#[derive(Default)]
pub(super) struct CatchUp {
  task: Option<Task<()>>,
  samples: VecDeque<f64>,
  /// The speed it set; 1× when idle.
  speed: Option<f64>,
}

impl PlayerView {
  /// Starts the controller for the mpv just attached, when the source asks for it.
  pub(super) fn start_catch_up(&mut self, cx: &mut Context<Self>) {
    self.catch_up = CatchUp::default();
    if !self.is_live() || self.catch_up_floor().is_none() {
      return;
    }
    self.catch_up.task = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(TICK).await;
        if this.update(cx, |this, _| this.catch_up_tick()).is_err() {
          break;
        }
      }
    }));
  }

  /// The statistics' line: the speed the controller set, for a source that asks for it.
  pub(super) fn catch_up_stat(&self) -> Option<String> {
    let floor = self.catch_up_floor()?;
    let speed = self.catch_up.speed.unwrap_or(1.0);
    Some(format!("{speed}× ({floor} s)"))
  }

  fn catch_up_floor(&self) -> Option<f64> {
    self.source.as_ref()?.catch_up
  }

  /// One reading: the speed follows the buffer while the live plays at its edge; anywhere
  /// else (paused, sought back, its recording, a drag) it is back at 1× and the readings start
  /// over.
  fn catch_up_tick(&mut self) {
    let Some(floor) = self.catch_up_floor() else {
      return;
    };
    let edge =
      self.is_live() && !self.rewound && !self.paused && !self.seek_dragging && self.at_live_edge();
    let cache = self
      .player
      .as_ref()
      .and_then(|p| p.property("demuxer-cache-duration"))
      .and_then(|v| v.parse::<f64>().ok());
    let (true, Some(cache)) = (edge, cache) else {
      self.catch_up.samples.clear();
      self.set_catch_up_speed(1.0);
      return;
    };
    let samples = &mut self.catch_up.samples;
    samples.push_back(cache);
    if samples.len() > SAMPLES {
      samples.pop_front();
    }
    if samples.len() < SAMPLES {
      return;
    }
    let low = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let current = self.catch_up.speed.unwrap_or(1.0);
    self.set_catch_up_speed(next_speed(current, low, floor));
  }

  fn set_catch_up_speed(&mut self, speed: f64) {
    if self.catch_up.speed.unwrap_or(1.0) == speed {
      return;
    }
    tracing::debug!("live catch-up: speed {speed}");
    self.catch_up.speed = Some(speed);
    self.with_player(|p| p.set_speed(speed));
  }
}

/// The speed after `current`, `low` being the buffer's smallest recent value and `floor` the
/// one to keep, seconds.
fn next_speed(current: f64, low: f64, floor: f64) -> f64 {
  if current > 1.0 {
    if low <= floor + STOP_ABOVE {
      1.0
    } else {
      current
    }
  } else if current < 1.0 {
    if low >= floor { 1.0 } else { current }
  } else if low > floor + FAR_ABOVE {
    FASTER
  } else if low > floor + START_ABOVE {
    FAST
  } else if low < floor - SLOW_UNDER {
    SLOW
  } else {
    1.0
  }
}

#[cfg(test)]
mod tests {
  use super::{FAST, FASTER, SLOW, next_speed};

  #[test]
  fn speeds_up_above_the_floor_until_near_it() {
    assert_eq!(next_speed(1.0, 1.0, 0.6), FAST);
    assert_eq!(next_speed(1.0, 2.2, 0.6), FASTER);
    // Keeps going while above, stops near the floor.
    assert_eq!(next_speed(FAST, 0.8, 0.6), FAST);
    assert_eq!(next_speed(FAST, 0.69, 0.6), 1.0);
    assert_eq!(next_speed(FASTER, 0.65, 0.6), 1.0);
  }

  #[test]
  fn slows_down_under_the_floor_until_back_at_it() {
    assert_eq!(next_speed(1.0, 0.2, 0.6), SLOW);
    assert_eq!(next_speed(SLOW, 0.5, 0.6), SLOW);
    assert_eq!(next_speed(SLOW, 0.6, 0.6), 1.0);
  }

  #[test]
  fn holds_near_the_floor() {
    assert_eq!(next_speed(1.0, 0.6, 0.6), 1.0);
    assert_eq!(next_speed(1.0, 0.85, 0.6), 1.0);
    assert_eq!(next_speed(1.0, 0.35, 0.6), 1.0);
  }
}
