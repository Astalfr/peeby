//! Small value animations for hover effects and fades (docs/design.md, "Motion").
//!
//! GPUI animates elements that appear (`with_animation`) but has no transitions between
//! states. [`tween`] keeps a value per element id and eases it toward its target, asking for
//! frames until it gets there.

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{AnyElement, App, ElementId, Window, div};

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
/// the id is seen.
pub fn tween(
  id: impl Into<ElementId>,
  initial: f32,
  target: f32,
  duration: Duration,
  window: &mut Window,
  cx: &mut App,
) -> f32 {
  let now = Instant::now();
  let state = window.use_keyed_state(id.into(), cx, |_, _| Tween {
    from: initial,
    to: initial,
    started: now,
    duration,
  });
  let (value, done) = state.update(cx, |tween, _| {
    if tween.to != target {
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
