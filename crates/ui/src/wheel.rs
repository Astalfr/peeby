//! Smooth wheel scrolling (docs/design.md, "Wheel scrolling"; docs/responsiveness-plan.md,
//! item 16). GPUI scrolls by a whole step at each notch of a mouse wheel; WinUI's ScrollViewer
//! eases each one over about 150 ms. [`listen`], on a window's root, takes each notch (a
//! `ScrollDelta::Lines` event) before anything else sees it and hands it out again, frame by
//! frame, as smaller `Lines` events where the pointer is: every scrolling element (pages,
//! lists, the chat) eases without knowing it, and notches turned in a row add up. A notch also
//! goes as far as in a browser ([`LINE`]): GPUI's is a third shorter than other apps'. With
//! reduced motion or the setting off it goes at once. Touchpads and precise wheels (`ScrollDelta::Pixels`) are
//! left as they are, and so is the wheel with Ctrl held.
//!
//! What counts notches rather than distance (a volume's step) asks [`notches`]: a slice carries
//! the notches that came since the last one, so one notch is one step.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui::{App, DispatchPhase, PlatformInput, Point, ScrollDelta, ScrollEvent, Window, WindowId};

/// The ease's time constant: a notch is 95 % through after three of them (about 150 ms).
const TAU: Duration = Duration::from_millis(50);
/// Left to scroll under this (in lines, about half a pixel), the rest goes at once.
const DONE: f32 = 0.03;
/// A wheel line, as browsers on Windows scroll it (100 px a notch at 3 lines a notch, the default):
/// GPUI takes a line of text (about 24 px here), a third less than other apps.
const LINE: f32 = 100. / 3.;

/// A window's notches still being handed out.
#[derive(Default)]
struct Glide {
  /// Lines left to scroll, on each axis.
  left: Point<f32>,
  /// Notches that came since the last slice.
  notches: u32,
  /// The last slice; `None` while idle.
  last: Option<Instant>,
  /// The notch's event, for the slices' modifiers and source.
  event: Option<ScrollEvent>,
  /// Smooth scrolling is off: each notch goes at once.
  instant: bool,
}

impl Glide {
  /// A notch of `lines`: added to what is left, or in place of it when it turns back.
  fn add(&mut self, lines: Point<f32>) {
    let against = |left: f32, new: f32| left * new < 0.;
    if against(self.left.x, lines.x) {
      self.left.x = 0.;
    }
    if against(self.left.y, lines.y) {
      self.left.y = 0.;
    }
    self.left.x += lines.x;
    self.left.y += lines.y;
    self.notches += 1;
  }

  /// The slice to scroll `dt` after the last one: what is left eases out exponentially, and the
  /// end goes at once.
  fn slice(&mut self, dt: Duration) -> Point<f32> {
    let part = 1. - (-dt.as_secs_f32() / TAU.as_secs_f32()).exp();
    let take = |left: &mut f32| {
      let slice = if (*left * (1. - part)).abs() < DONE {
        *left
      } else {
        *left * part
      };
      *left -= slice;
      slice
    };
    Point {
      x: take(&mut self.left.x),
      y: take(&mut self.left.y),
    }
  }

  fn done(&self) -> bool {
    self.left.x == 0. && self.left.y == 0.
  }
}

thread_local! {
  static GLIDES: RefCell<HashMap<WindowId, Glide>> = RefCell::default();
  /// While a slice is dispatched: the notches it carries.
  static SLICE: Cell<Option<u32>> = const { Cell::new(None) };
}

/// Takes the window's wheel notches and eases them; call it from a paint callback of the
/// window's root (a `canvas`), at every frame, before the window's content registers its
/// listeners. Without `smooth` (the setting off), each notch goes at once, as far.
pub fn listen(window: &mut Window, smooth: bool) {
  window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
    if phase != DispatchPhase::Capture || SLICE.get().is_some() {
      return;
    }
    let ScrollDelta::Lines(lines) = event.delta else {
      return;
    };
    if event.modifiers.control {
      return;
    }
    cx.stop_propagation();
    // Windows' lines (its "lines per notch" setting) as long as a browser's, not GPUI's line of
    // text.
    let scale = LINE / f32::from(window.line_height()).max(1.);
    let lines = Point {
      x: lines.x * scale,
      y: lines.y * scale,
    };
    let id = window.window_handle().window_id();
    let start = GLIDES.with_borrow_mut(|glides| {
      let glide = glides.entry(id).or_default();
      glide.add(lines);
      glide.event = Some(event.clone());
      glide.instant = !smooth;
      let idle = glide.last.is_none();
      if idle {
        glide.last = Some(Instant::now());
      }
      idle
    });
    if start {
      // The first slice goes as soon as this event's dispatch is over (at the next frame would
      // add one of delay to every notch).
      window.defer(cx, step);
    }
  });
}

/// Hands out the window's next slice, and asks for the one after it.
fn step(window: &mut Window, cx: &mut App) {
  let id = window.window_handle().window_id();
  let now = Instant::now();
  let slice = GLIDES.with_borrow_mut(|glides| {
    let glide = glides.get_mut(&id)?;
    let last = glide.last?;
    // The first slice of a notch after a pause is a frame's worth; all of it at once with
    // reduced motion or smooth scrolling off.
    let dt = if cx.reduce_motion() || glide.instant {
      Duration::from_secs(1)
    } else {
      now.duration_since(last).max(Duration::from_millis(16))
    };
    glide.last = Some(now);
    let lines = glide.slice(dt);
    let notches = std::mem::take(&mut glide.notches);
    let event = glide.event.clone()?;
    if glide.done() {
      glide.last = None;
    }
    Some((lines, notches, event, glide.last.is_some()))
  });
  let Some((lines, notches, event, more)) = slice else {
    return;
  };
  let slice = ScrollEvent {
    // Where the pointer is now: dispatching moves GPUI's idea of it there.
    position: window.mouse_position(),
    delta: ScrollDelta::Lines(lines),
    ..event
  };
  SLICE.set(Some(notches));
  window.dispatch_event(PlatformInput::Scroll(slice), cx);
  SLICE.set(None);
  if more {
    window.on_next_frame(step);
  }
}

/// How many wheel notches `event` stands for, for what steps once per notch (a volume): 1 for
/// an event from the device (a notch, or a touchpad's move), the notches a smoothed slice
/// carries (0 for most of them).
pub fn notches(_event: &ScrollEvent) -> u32 {
  SLICE.get().unwrap_or(1)
}

#[cfg(test)]
mod tests {
  use super::*;

  const FRAME: Duration = Duration::from_millis(16);

  fn run(glide: &mut Glide) -> (Point<f32>, usize) {
    let mut total = Point::default();
    let mut frames = 0;
    while !glide.done() {
      let slice = glide.slice(FRAME);
      total.x += slice.x;
      total.y += slice.y;
      frames += 1;
      assert!(frames < 100, "never ends");
    }
    (total, frames)
  }

  #[test]
  fn a_notch_scrolls_its_whole_distance_over_a_few_frames() {
    let mut glide = Glide::default();
    glide.add(Point { x: 0., y: 3. });
    let first = glide.slice(FRAME).y;
    assert!(first > 0.5 && first < 1.5, "{first}");
    let (rest, frames) = run(&mut glide);
    assert!((first + rest.y - 3.).abs() < 1e-5);
    assert!((5..40).contains(&frames), "{frames}");
  }

  #[test]
  fn notches_in_a_row_add_up() {
    let mut glide = Glide::default();
    glide.add(Point { x: 0., y: 3. });
    glide.slice(FRAME);
    glide.add(Point { x: 0., y: 3. });
    assert_eq!(glide.notches, 2);
    let (total, _) = run(&mut glide);
    assert!(total.y > 3. && total.y < 6.);
  }

  #[test]
  fn turning_back_drops_what_was_left() {
    let mut glide = Glide::default();
    glide.add(Point { x: 0., y: 3. });
    glide.slice(FRAME);
    glide.add(Point { x: 0., y: -3. });
    let (total, _) = run(&mut glide);
    assert!((total.y + 3.).abs() < 1e-5);
  }
}
