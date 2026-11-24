//! Small value animations for hover effects and fades (docs/design.md, "Motion").
//!
//! GPUI animates elements that appear (`with_animation`) but has no transitions between
//! states. [`tween`] keeps a value per element id and eases it toward its target, asking for
//! frames until it gets there; a new target starts a new ease from rest. [`spring`] does the
//! same with a damped spring ([`Spring`], ADR 0041), which keeps its speed when its target
//! changes: for values redirected while they move (a hover left halfway, a fold reversed).

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

/// A spring's damping ratio without bounce: the fastest return that does not overshoot from
/// rest.
pub const CRITICAL: f32 = 1.;
/// A spring is at rest once both its distance to its target and the distance its speed would
/// cover in a radian of its swing are under this, in the value's units; it then snaps to its
/// target.
const REST: f32 = 1e-3;

/// A value pulled toward its target by a damped spring (ADR 0041), as SwiftUI's and Apple's
/// "fluid interfaces" springs: `response` is the period the spring would swing at without
/// damping (about the time it takes to get there, critically damped), `damping` its damping
/// ratio (1 for [`CRITICAL`], under 1 bounces, over 1 creeps in). Its motion is solved in
/// closed form from its last change, so it is the same at any frame rate, and a change of
/// target (or a shift of its value) keeps both where it is and how fast it moves.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
  /// Value and speed (units per second) at `since`.
  value: f32,
  velocity: f32,
  target: f32,
  since: Instant,
  /// Undamped angular frequency, from the response (rad/s).
  omega: f32,
  damping: f32,
}

impl Spring {
  /// At rest at `value`.
  pub fn new(value: f32, response: Duration, damping: f32, now: Instant) -> Self {
    let secs = response.as_secs_f32().max(1e-3);
    Self {
      value,
      velocity: 0.,
      target: value,
      since: now,
      omega: std::f32::consts::TAU / secs,
      damping: damping.max(0.),
    }
  }

  /// Where it heads.
  pub fn target(&self) -> f32 {
    self.target
  }

  /// Its value and speed at `now`: the target, still, once at rest.
  pub fn at(&self, now: Instant) -> (f32, f32) {
    let t = now.saturating_duration_since(self.since).as_secs_f32();
    let (y, v) = self.offset(self.value - self.target, self.velocity, t);
    if self.rests(y, v) {
      (self.target, 0.)
    } else {
      (self.target + y, v)
    }
  }

  /// Its value at `now`.
  pub fn value(&self, now: Instant) -> f32 {
    self.at(now).0
  }

  /// At rest on its target at `now`.
  pub fn settled(&self, now: Instant) -> bool {
    let t = now.saturating_duration_since(self.since).as_secs_f32();
    let (y, v) = self.offset(self.value - self.target, self.velocity, t);
    self.rests(y, v)
  }

  /// Heads for `target` from `now`, from where it is and as fast as it moves.
  pub fn retarget(&mut self, target: f32, now: Instant) {
    if target == self.target {
      return;
    }
    self.rebase(now);
    self.target = target;
  }

  /// Its value moved `by` at `now` (the target stays), its speed kept: a slide added to one
  /// under way.
  pub fn shift(&mut self, by: f32, now: Instant) {
    self.rebase(now);
    self.value += by;
  }

  /// At rest on `target` at once (reduced motion).
  pub fn snap(&mut self, target: f32, now: Instant) {
    self.value = target;
    self.velocity = 0.;
    self.target = target;
    self.since = now;
  }

  fn rebase(&mut self, now: Instant) {
    (self.value, self.velocity) = self.at(now);
    self.since = now;
  }

  fn rests(&self, y: f32, v: f32) -> bool {
    y.abs() < REST && (v / self.omega).abs() < REST
  }

  /// The distance to the target and the speed `t` seconds after being `y0` away at `v0`: the
  /// damped oscillator `y'' + 2ζω y' + ω² y = 0`, in its three cases.
  fn offset(&self, y0: f32, v0: f32, t: f32) -> (f32, f32) {
    let (w, z) = (self.omega, self.damping);
    if (z - 1.).abs() < 1e-3 {
      // Critically damped: y = e^(-ωt) (y0 + (v0 + ω y0) t).
      let b = v0 + w * y0;
      let e = (-w * t).exp();
      (e * (y0 + b * t), e * (v0 - w * b * t))
    } else if z < 1. {
      // Underdamped: it swings at ωd, its swings shrinking at ζω.
      let a = z * w;
      let wd = w * (1. - z * z).sqrt();
      let b = (v0 + a * y0) / wd;
      let e = (-a * t).exp();
      let (sin, cos) = (wd * t).sin_cos();
      (
        e * (y0 * cos + b * sin),
        e * (v0 * cos - (a * b + y0 * wd) * sin),
      )
    } else {
      // Overdamped: two decays, at r1 and r2.
      let root = (z * z - 1.).sqrt();
      let (r1, r2) = (-w * (z - root), -w * (z + root));
      let c2 = (v0 - r1 * y0) / (r2 - r1);
      let c1 = y0 - c2;
      let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
      (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    }
  }
}

/// The current value of a spring toward `target` ([`Spring`]), at rest on it the first time
/// the id is seen; `response` and `damping` are read then. Asks for frames until it rests.
/// With reduced motion, the value is the target at once.
pub fn spring(
  id: impl Into<ElementId>,
  target: f32,
  response: Duration,
  damping: f32,
  window: &mut Window,
  cx: &mut App,
) -> f32 {
  let reduced = cx.reduce_motion();
  let now = Instant::now();
  let state = window.use_keyed_state(id.into(), cx, |_, _| {
    Spring::new(target, response, damping, now)
  });
  let (value, settled) = state.update(cx, |spring, _| {
    if reduced {
      spring.snap(target, now);
    } else {
      spring.retarget(target, now);
    }
    (spring.value(now), spring.settled(now))
  });
  if !settled {
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

#[cfg(test)]
mod tests {
  use super::*;

  const RESPONSE: Duration = Duration::from_millis(150);

  fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
  }

  #[test]
  fn a_critical_spring_gets_there_without_overshooting() {
    let start = Instant::now();
    let mut spring = Spring::new(0., RESPONSE, CRITICAL, start);
    spring.retarget(1., start);
    let mut last = 0.;
    for n in 1..=100 {
      let value = spring.value(start + ms(5 * n));
      assert!(value >= last && value <= 1., "{value} after {last}");
      last = value;
    }
    assert!(spring.value(start + RESPONSE) > 0.95);
    assert!(spring.settled(start + ms(500)));
    assert_eq!(spring.value(start + ms(500)), 1.);
  }

  #[test]
  fn a_new_target_keeps_position_and_velocity() {
    for damping in [0.6, CRITICAL, 1.5] {
      let start = Instant::now();
      let mut spring = Spring::new(0., RESPONSE, damping, start);
      spring.retarget(1., start);
      let turn = start + ms(40);
      let (before, speed) = spring.at(turn);
      assert!(speed > 0., "{damping}: moving toward 1");
      spring.retarget(0., turn);
      let (after, kept) = spring.at(turn);
      assert!(
        (after - before).abs() < 1e-6,
        "{damping}: {before} → {after}"
      );
      assert!((kept - speed).abs() < 1e-3, "{damping}: {speed} → {kept}");
      // Just after, it still goes on toward 1 before it turns back: it does not stop dead.
      assert!(spring.value(turn + ms(2)) > before, "{damping}");
      assert!(spring.value(turn + ms(800)).abs() < 0.01, "{damping}");
    }
  }

  #[test]
  fn the_velocity_is_the_derivative_of_the_value() {
    for damping in [0.5, CRITICAL, 2.] {
      let start = Instant::now();
      let mut spring = Spring::new(3., RESPONSE, damping, start);
      spring.retarget(-2., start);
      spring.shift(1., start + ms(30));
      for n in [40, 70, 120] {
        let at = start + ms(n);
        let (a, b) = (spring.value(at), spring.value(at + ms(1)));
        let (_, v) = spring.at(at);
        let slope = (b - a) / 1e-3;
        assert!(
          (slope - v).abs() < 0.05 * v.abs().max(1.),
          "{damping} at {n}: {slope} vs {v}"
        );
      }
    }
  }

  #[test]
  fn a_shift_moves_the_value_and_keeps_the_speed() {
    let start = Instant::now();
    let mut spring = Spring::new(40., RESPONSE, CRITICAL, start);
    spring.retarget(0., start);
    let at = start + ms(50);
    let (value, speed) = spring.at(at);
    spring.shift(-25., at);
    let (shifted, kept) = spring.at(at);
    assert!((shifted - (value - 25.)).abs() < 1e-4);
    assert!((kept - speed).abs() < 1e-3);
    assert_eq!(spring.target(), 0.);
  }

  #[test]
  fn an_underdamped_spring_bounces_then_rests() {
    let start = Instant::now();
    let mut spring = Spring::new(0., RESPONSE, 0.4, start);
    spring.retarget(1., start);
    let peak = (1..200)
      .map(|n| spring.value(start + ms(n)))
      .fold(0., f32::max);
    assert!(peak > 1.05, "{peak}");
    assert!(spring.settled(start + Duration::from_secs(2)));
  }

  #[test]
  fn snapping_rests_on_the_target() {
    let start = Instant::now();
    let mut spring = Spring::new(0., RESPONSE, CRITICAL, start);
    spring.retarget(1., start);
    spring.snap(1., start + ms(10));
    assert!(spring.settled(start + ms(10)));
    assert_eq!(spring.value(start + ms(10)), 1.);
  }
}
