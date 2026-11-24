//! Small value animations for hover effects and fades (docs/design.md, "Motion").
//!
//! GPUI animates elements that appear (`with_animation`) but has no transitions between
//! states. [`tween`] keeps a value per element id and eases it toward its target, asking for
//! frames until it gets there.

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{AnyElement, App, ElementId, EntityId, SharedString, Window, div, px};

use crate::theme;

struct Tween {
  from: f32,
  to: f32,
  started: Instant,
  duration: Duration,
}

impl Tween {
  fn value(&self, now: Instant) -> (f32, bool) {
    let t = if self.duration.is_zero() {
      1.0
    } else {
      (now.duration_since(self.started).as_secs_f32() / self.duration.as_secs_f32()).min(1.0)
    };
    (
      self.from + (self.to - self.from) * ease_out_cubic(t),
      t >= 1.0,
    )
  }
}

fn ease_out_cubic(t: f32) -> f32 {
  1.0 - (1.0 - t).powi(3)
}

/// The current value of an animation toward `target`, starting at `initial` the first time
/// the id is seen. With reduced motion (GPUI's `reduce_motion`, Windows' "Animation effects"
/// off), the value is the target at once.
pub fn tween(
  id: impl Into<ElementId>,
  initial: f32,
  target: f32,
  duration: Duration,
  window: &mut Window,
  cx: &mut App,
) -> f32 {
  let reduced = cx.reduce_motion();
  let now = Instant::now();
  let state = window.use_keyed_state(id.into(), cx, |_, _| Tween {
    from: initial,
    to: initial,
    started: now,
    duration,
  });
  let (value, done) = state.update(cx, |tween, _| {
    if reduced {
      *tween = Tween {
        from: target,
        to: target,
        started: now,
        duration: Duration::ZERO,
      };
    } else if tween.to != target {
      let (current, _) = tween.value(now);
      *tween = Tween {
        from: current,
        to: target,
        started: now,
        duration,
      };
    }
    tween.value(now)
  });
  if !done {
    window.request_animation_frame();
  }
  value
}

/// A press's animation: 1 at once while `down`, back to 0 in [`theme::FASTER`] once released
/// (instant feedback on the press, a soft release).
pub fn press(id: impl Into<ElementId>, down: bool, window: &mut Window, cx: &mut App) -> f32 {
  if down {
    tween(id, 1., 1., Duration::ZERO, window, cx)
  } else {
    tween(id, 0., 0., theme::FASTER, window, cx)
  }
}

/// A hover flag per element id, for [`tween`]. Set it from `on_hover`.
pub fn hover_state(
  id: impl Into<ElementId>,
  window: &mut Window,
  cx: &mut App,
) -> gpui::Entity<bool> {
  window.use_keyed_state(id.into(), cx, |_, _| false)
}

/// `skeleton` while `loading`, then `content`, cross-faded: the content fades in while the
/// skeleton, laid over it, fades out. Turning `loading` back on fades the skeleton back in.
/// Content that is ready on the first frame shows directly.
pub fn crossfade(
  id: impl Into<ElementId>,
  loading: bool,
  skeleton: AnyElement,
  content: AnyElement,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let shown = if loading { 0. } else { 1. };
  let t = tween(id, shown, shown, theme::SLOW, window, cx);
  if loading {
    return div().opacity(1. - t).child(skeleton).into_any_element();
  }
  div()
    .relative()
    .child(div().opacity(t).child(content))
    .when(t < 1., |el| {
      el.child(
        div()
          .absolute()
          .top_0()
          .left_0()
          .right_0()
          .opacity(1. - t)
          .child(skeleton),
      )
    })
    .into_any_element()
}

/// The cards of a page's first content come in one after another ([`cascade`]), this far
/// apart…
const CASCADE_STEP: Duration = Duration::from_millis(25);
/// …the first ones only (the last of them brings the rest)…
const CASCADE_STAGGERED: usize = 12;
/// …each fading in as it rises this far, over this long.
const CASCADE_RISE: f32 = 12.;
const CASCADE: Duration = Duration::from_millis(280);
/// Cascades remembered; past this they are forgotten (long over by then).
const CASCADES_KEPT: usize = 1024;

thread_local! {
  /// When each view's cascades started, by the view and the cascade's key.
  static CASCADES: std::cell::RefCell<std::collections::HashMap<(EntityId, SharedString), Instant>> =
    std::cell::RefCell::default();
}

/// When `owner`'s cascade `key` started: the first time it is asked for. Once per view: a view
/// built again (a refresh, a page far back in the history) is a new entity, so it cascades
/// again; the same view shown again does not.
pub fn cascade_start(owner: EntityId, key: &str) -> Instant {
  CASCADES.with(|cascades| {
    let mut cascades = cascades.borrow_mut();
    if cascades.len() >= CASCADES_KEPT {
      cascades.clear();
    }
    *cascades
      .entry((owner, SharedString::from(key.to_owned())))
      .or_insert_with(Instant::now)
  })
}

/// Where card `index` of a cascade that started at `started` is: its opacity and how far below
/// its place it is; `None` once in place, or with reduced motion.
pub fn cascade_at(started: Instant, index: usize, cx: &App) -> Option<(f32, f32)> {
  if cx.reduce_motion() {
    return None;
  }
  let delay = CASCADE_STEP * index.min(CASCADE_STAGGERED - 1) as u32;
  let t = started
    .elapsed()
    .checked_sub(delay)
    .map_or(0., |age| age.as_secs_f32() / CASCADE.as_secs_f32());
  if t >= 1. {
    return None;
  }
  let eased = 1. - (1. - t).powi(3);
  Some((eased, CASCADE_RISE * (1. - eased)))
}

/// `card` at its point of a cascade ([`cascade_at`]).
pub fn cascaded(card: AnyElement, at: Option<(f32, f32)>) -> AnyElement {
  match at {
    Some((opacity, below)) => div()
      .relative()
      .top(px(below))
      .opacity(opacity)
      .child(card)
      .into_any_element(),
    None => card,
  }
}

/// `cards` coming in one after another, from the first time `owner` shows its cascade `key`
/// (docs/design.md, "First content"); `first` is the place of the first of them in the
/// cascade (a later row of it). Asks for frames until they are in place.
pub fn cascade(
  owner: EntityId,
  key: &str,
  first: usize,
  cards: Vec<AnyElement>,
  window: &mut Window,
  cx: &App,
) -> Vec<AnyElement> {
  let started = cascade_start(owner, key);
  let mut moving = false;
  let cards = cards
    .into_iter()
    .enumerate()
    .map(|(i, card)| {
      let at = cascade_at(started, first + i, cx);
      moving |= at.is_some();
      cascaded(card, at)
    })
    .collect();
  if moving {
    window.request_animation_frame();
  }
  cards
}
