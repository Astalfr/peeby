//! Moving a tab along the bar with the mouse, as in a browser (ADR 0028): pressed and moved past
//! [`THRESHOLD`], the tab leaves its slot empty and a copy of it follows the pointer; when the
//! copy's middle passes a neighbor's, the tab takes that neighbor's place. The drag is followed
//! with window listeners rather than GPUI's `on_drag`, which ends at the window's edge: tearing a
//! tab out to a window of its own needs the release wherever it happens (the window captures the
//! mouse while the button is held). The tabs it passes slide to their new places, and so does the
//! tab released (none with reduced motion).

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use gpui::{
  App, Bounds, Context, DispatchPhase, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point,
  WeakEntity, Window, px,
};

use super::Shell;
use super::space::TabId;

/// How far the pointer moves, held, before the tab follows it: less is still a click.
const THRESHOLD: f32 = 4.;
/// Between two tabs: the row's `gap_1`.
const GAP: f32 = 4.;
/// A tab sliding to its new place.
const SLIDE: Duration = Duration::from_millis(150);

/// A tab drawn away from its place by `from`, easing back to it from `started`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Slide {
  from: Pixels,
  started: Instant,
}

impl Slide {
  /// Where the tab is drawn from its place at `now`; `None` once there.
  fn offset(&self, now: Instant) -> Option<Pixels> {
    let t = now.duration_since(self.started).as_secs_f32() / SLIDE.as_secs_f32();
    (t < 1.).then(|| self.from * (1. - t).powi(3))
  }
}

/// Where the bar's tabs were drawn at the last frame, in window coordinates, to place the
/// dragged copy and to find the neighbors it passes.
#[derive(Default)]
pub(super) struct TabLayout {
  /// The row of tabs (with "+").
  pub row: Cell<Bounds<Pixels>>,
  /// Each tab, left to right; refilled at each frame.
  pub tabs: RefCell<Vec<(TabId, Bounds<Pixels>)>>,
}

impl TabLayout {
  fn bounds(&self, id: TabId) -> Option<Bounds<Pixels>> {
    self
      .tabs
      .borrow()
      .iter()
      .find(|(tab, _)| *tab == id)
      .map(|(_, b)| *b)
  }
}

/// A tab held down with the left button.
#[derive(Clone, Copy, Debug)]
pub(super) struct TabDrag {
  pub tab: TabId,
  /// Where the button went down.
  from: Point<Pixels>,
  /// The pointer's x now.
  x: Pixels,
  /// Past the threshold: the tab follows the pointer.
  pub moving: bool,
  /// From the tab's left edge to the pointer, when the button went down.
  grab: Pixels,
  /// The tab's width when the button went down: the copy's.
  pub width: Pixels,
}

impl Shell {
  /// The left button went down on tab `id` at `position`.
  pub(super) fn press_tab(&mut self, id: TabId, position: Point<Pixels>, cx: &mut Context<Self>) {
    let Some(bounds) = self.tab_layout.bounds(id) else {
      return;
    };
    self.tab_drag = Some(TabDrag {
      tab: id,
      from: position,
      x: position.x,
      moving: false,
      grab: position.x - bounds.left(),
      width: bounds.size.width,
    });
    // Drawn again, to listen to the window while the button is held.
    cx.notify();
  }

  /// The pointer moved, the button held.
  fn drag_tab(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
    let Some(drag) = &mut self.tab_drag else {
      return;
    };
    drag.x = position.x;
    if !drag.moving {
      let (dx, dy) = (position.x - drag.from.x, position.y - drag.from.y);
      if f32::from(dx).abs() < THRESHOLD && f32::from(dy).abs() < THRESHOLD {
        return;
      }
      drag.moving = true;
      // A browser shows the tab it drags.
      let tab = drag.tab;
      if self.active.map(|t| t.id) != Some(tab) {
        self.activate(tab, window, cx);
      }
    }
    self.reorder_dragged(cx);
    cx.notify();
  }

  /// Puts the dragged tab after every other tab whose middle is left of the copy's.
  fn reorder_dragged(&mut self, cx: &App) {
    let Some(drag) = self.tab_drag else { return };
    let middle = drag.x - drag.grab + drag.width / 2.;
    let layout = self.tab_layout.tabs.borrow();
    let target = slot(
      self.tabs.iter().filter(|t| t.id != drag.tab).map(|t| {
        layout
          .iter()
          .find(|(id, _)| *id == t.id)
          .map(|(_, b)| b.center().x)
      }),
      middle,
    );
    drop(layout);
    let Some(from) = self.tabs.iter().position(|t| t.id == drag.tab) else {
      return;
    };
    if from == target {
      return;
    }
    let tab = self.tabs.remove(from);
    self.tabs.insert(target, tab);
    // The tabs passed move by the dragged one's width, the other way: drawn where they were,
    // they slide there.
    let shift = drag.width + px(GAP);
    let (passed, by) = if from < target {
      (from..target, shift)
    } else {
      (target + 1..from + 1, -shift)
    };
    let ids: Vec<_> = self.tabs[passed].iter().map(|t| t.id).collect();
    for id in ids {
      self.slide(id, by, cx);
    }
  }

  /// The button was released (or found released): the tab stays where it is, sliding from the
  /// copy into its slot.
  fn drop_tab(&mut self, cx: &mut Context<Self>) {
    let Some(drag) = self.tab_drag.take() else {
      return;
    };
    if drag.moving
      && let Some(slot) = self.tab_layout.bounds(drag.tab)
    {
      let copy = self.tab_layout.row.get().left() + self.dragged_left(&drag);
      self.slide(drag.tab, copy - slot.left(), cx);
    }
    cx.notify();
  }

  /// Draws tab `id` `by` away from its place (added to a slide under way), easing back to it.
  fn slide(&mut self, id: TabId, by: Pixels, cx: &App) {
    if cx.reduce_motion() {
      return;
    }
    let now = Instant::now();
    let current = self.tab_slide_at(id, now).unwrap_or_default();
    self.tab_slides.retain(|_, s| s.offset(now).is_some());
    self.tab_slides.insert(
      id,
      Slide {
        from: current + by,
        started: now,
      },
    );
  }

  fn tab_slide_at(&self, id: TabId, now: Instant) -> Option<Pixels> {
    self.tab_slides.get(&id).and_then(|s| s.offset(now))
  }

  /// Where tab `id` is drawn from its place now, while it slides.
  pub(super) fn tab_slide(&self, id: TabId) -> Option<Pixels> {
    self.tab_slide_at(id, Instant::now())
  }

  /// The dragged copy's left edge from the row's, kept over the tabs.
  pub(super) fn dragged_left(&self, drag: &TabDrag) -> Pixels {
    let row = self.tab_layout.row.get();
    let end = self
      .tab_layout
      .tabs
      .borrow()
      .iter()
      .map(|(_, b)| b.right())
      .fold(row.left(), Pixels::max);
    let left = drag.x - drag.grab;
    let max = (end - drag.width).max(row.left());
    left.clamp(row.left(), max) - row.left()
  }
}

/// The dragged tab's index among all of them: after each other tab (left to right, its middle
/// if it was drawn) whose middle is left of the copy's. A neighbor passed moves by the dragged
/// tab's width, away from the copy's middle, so the order holds until the copy moves back past
/// it.
fn slot(others: impl Iterator<Item = Option<Pixels>>, middle: Pixels) -> usize {
  others.filter(|m| m.is_some_and(|m| m < middle)).count()
}

/// Follows the held tab over the whole screen until the button is released. Registered at each
/// paint while the button is held, as GPUI drops window listeners every frame.
pub(super) fn listen(shell: WeakEntity<Shell>, window: &mut Window) {
  let moved = shell.clone();
  window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
    if phase != DispatchPhase::Capture {
      return;
    }
    moved
      .update(cx, |this, cx| {
        if event.pressed_button == Some(MouseButton::Left) {
          this.drag_tab(event.position, window, cx);
        } else {
          // Released where no mouse up reached the window.
          this.drop_tab(cx);
        }
      })
      .ok();
  });
  window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
      shell.update(cx, |this, cx| this.drop_tab(cx)).ok();
    }
  });
}

#[cfg(test)]
mod tests {
  use gpui::px;

  use super::slot;

  #[test]
  fn the_dragged_tab_goes_after_the_middles_it_passed() {
    let others = || [Some(px(50.)), Some(px(150.)), Some(px(250.))].into_iter();
    assert_eq!(slot(others(), px(10.)), 0);
    assert_eq!(slot(others(), px(149.)), 1);
    assert_eq!(slot(others(), px(151.)), 2);
    assert_eq!(slot(others(), px(900.)), 3);
  }

  #[test]
  fn a_tab_not_yet_drawn_is_not_passed() {
    let others = [Some(px(50.)), None, Some(px(250.))].into_iter();
    assert_eq!(slot(others, px(300.)), 2);
  }
}
