//! Moving a tab with the mouse, as in a browser (ADR 0028). Pressed and moved past
//! [`THRESHOLD`], the tab leaves its slot empty and a copy of it follows the pointer along the
//! bar; when the copy's middle passes a neighbor's, the tab takes that neighbor's place. Pulled
//! [`DETACH`] above or below the bar (or out of the window), the tab tears out at once into a
//! window of its own, which follows the pointer (without the focus: the window the drag started
//! in keeps the mouse's capture) until the release. Released over the bar of one of Peeby's
//! windows (the one it came from included), it joins that bar at the place under the pointer and
//! its window goes; elsewhere, its window stays there and comes to the front. A window's only
//! tab carries its window instead, as in Chrome: from the first move the window follows the
//! pointer, and released over another window's bar the tab joins it (its window goes). A
//! maximized window is not carried: its only tab detaches as a copy over the window.
//!
//! The drag is followed with window listeners rather than GPUI's `on_drag`, which ends at the
//! window's edge: the window captures the mouse while the button is held, so the release is seen
//! wherever it happens. The tabs it passes slide to their new places, and so does the tab
//! released (none with reduced motion).

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use gpui::{
  AnyWindowHandle, App, Bounds, Context, DispatchPhase, DisplayId, MouseButton, MouseMoveEvent,
  MouseUpEvent, Pixels, Point, Size, WeakEntity, Window, point, px,
};

use super::space::TabId;
use super::{Shell, windows};

/// How far the pointer moves, held, before the tab follows it: less is still a click.
const THRESHOLD: f32 = 4.;
/// How far above or below the bar the pointer goes before the tab detaches; also how far a
/// window's bar reaches for a tab dropped on it.
const DETACH: f32 = 32.;
/// Right of "+", a bar still takes a dropped tab (at its end).
const BAR_END: f32 = 160.;
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
/// dragged copy, to find the neighbors it passes, and (with the window's place on the screen)
/// to take a tab dropped from another window.
#[derive(Default)]
pub(super) struct TabLayout {
  /// The row of tabs (with "+").
  pub row: Cell<Bounds<Pixels>>,
  /// The window's top left corner on the screen.
  pub origin: Cell<Point<Pixels>>,
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
  /// The pointer now.
  at: Point<Pixels>,
  /// Past the threshold: the tab follows the pointer.
  pub moving: bool,
  /// Pulled away from the bar: its slot is closed, the copy follows the pointer anywhere.
  pub detached: bool,
  /// From the tab's top left corner to the pointer, when the button went down.
  grab: Point<Pixels>,
  /// The tab's width when the button went down: the copy's.
  pub width: Pixels,
  /// Torn out: the window it is in now, following the pointer, and that window's size.
  torn: Option<(AnyWindowHandle, Size<Pixels>)>,
  /// The window's only tab, carrying its window: where the pointer was on the screen and the
  /// window's top left corner when the window started following it.
  carry: Option<(Point<Pixels>, Point<Pixels>)>,
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
      at: position,
      moving: false,
      detached: false,
      grab: position - bounds.origin,
      width: bounds.size.width,
      torn: None,
      carry: None,
    });
    // Drawn again, to listen to the window while the button is held.
    cx.notify();
  }

  /// The pointer moved, the button held.
  fn drag_tab(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
    let Some(drag) = &mut self.tab_drag else {
      return;
    };
    drag.at = position;
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
    let Some(drag) = self.tab_drag else { return };
    // Not once a tab tore out: this window may be left with one tab, which is not the one held.
    let only_tab = drag.torn.is_none() && self.tabs.len() == 1 && !window.is_maximized();
    if drag.carry.is_some() || only_tab {
      self.carry_window(position, window, cx);
      return;
    }
    if let Some((handle, size)) = drag.torn {
      let bounds = self.torn_bounds(&drag, window.bounds().origin + position, size);
      windows::move_to(handle, bounds, cx);
      return;
    }
    let viewport = Bounds::new(Point::default(), window.viewport_size());
    let detached = off_bar(self.tab_layout.row.get(), viewport, position);
    if detached != drag.detached {
      self.set_detached(detached, cx);
      if detached && self.tabs.len() > 1 {
        self.tear_out(window, cx);
      }
    }
    if !detached {
      self.reorder_dragged(cx);
    }
    cx.notify();
  }

  /// The window's only tab is dragged: the window follows the pointer, as when dragged by its
  /// title bar (the pointer's screen position is the window's corner plus `position`, which the
  /// mini-player's move relies on too).
  fn carry_window(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
    let Some(drag) = &mut self.tab_drag else {
      return;
    };
    let bounds = window.bounds();
    let screen = bounds.origin + position;
    let (from, corner) = *drag.carry.get_or_insert((screen, bounds.origin));
    ui::set_window_bounds(
      window,
      cx,
      Bounds::new(corner + (screen - from), bounds.size),
    );
  }

  /// The dragged tab leaves this window for one of its own under the pointer, which follows it
  /// until the release. This window keeps the drag (and the mouse).
  fn tear_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(drag) = self.tab_drag else { return };
    let Some(tab) = self.detach_tab(drag.tab, true, cx) else {
      return;
    };
    let size = window.window_bounds().get_bounds().size;
    let screen = window.bounds().origin + drag.at;
    let bounds = self.torn_bounds(&drag, screen, size);
    match windows::open_with(tab, bounds, display_at(screen, cx), false, cx) {
      Some(handle) => {
        self.tab_drag = Some(TabDrag {
          torn: Some((handle, size)),
          ..drag
        });
      }
      None => {
        // No window for it: it stays here, still held.
        self.adopt_tab(tab, None, cx);
        self.tab_drag = Some(drag);
      }
    }
  }

  /// A torn-out window's bounds with the pointer at `screen`: its tab under the pointer, where
  /// it was held (its bar laid out as this one's).
  fn torn_bounds(
    &self,
    drag: &TabDrag,
    screen: Point<Pixels>,
    size: Size<Pixels>,
  ) -> Bounds<Pixels> {
    let row = self.tab_layout.row.get();
    Bounds::new(
      screen - point(row.left() + drag.grab.x, row.top() + drag.grab.y),
      size,
    )
  }

  /// The dragged tab leaves the bar (its slot closes) or comes back to it (its slot opens): the
  /// tabs after it slide over the gap.
  fn set_detached(&mut self, detached: bool, cx: &App) {
    let Some(drag) = &mut self.tab_drag else {
      return;
    };
    drag.detached = detached;
    let (tab, shift) = (drag.tab, drag.width + px(GAP));
    let Some(index) = self.tabs.iter().position(|t| t.id == tab) else {
      return;
    };
    let after: Vec<_> = self.tabs[index + 1..].iter().map(|t| t.id).collect();
    for id in after {
      self.slide(id, if detached { shift } else { -shift }, cx);
    }
  }

  /// Puts the dragged tab after every other tab whose middle is left of the copy's.
  fn reorder_dragged(&mut self, cx: &App) {
    let Some(drag) = self.tab_drag else { return };
    let middle = drag.at.x - drag.grab.x + drag.width / 2.;
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

  /// The button was released (or found released) at `position`. On the bar, the tab stays where
  /// it is, sliding from the copy into its slot. Detached: over another window's bar, it joins
  /// that window; elsewhere, it opens in a window of its own under the pointer (not the window's
  /// only tab, which stays: there would be nothing left to close).
  fn drop_tab(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
    let Some(drag) = self.tab_drag.take() else {
      return;
    };
    cx.notify();
    if !drag.moving {
      return;
    }
    if drag.carry.is_some() {
      // The window stays where it was carried, unless it was over another window's bar.
      let screen = window.bounds().origin + position;
      if let Some((target, index)) = windows::bar_at(screen, &[self.window], cx) {
        self.join(drag.tab, target, index, cx);
      }
      return;
    }
    if !drag.detached {
      if let Some(slot) = self.tab_layout.bounds(drag.tab) {
        let copy = self.tab_layout.row.get().left() + self.dragged_left(&drag);
        self.slide(drag.tab, copy - slot.left(), cx);
      }
      return;
    }
    let origin = window.bounds().origin;
    let screen = origin + position;
    if let Some((handle, _)) = drag.torn {
      // Deferred: other windows are updated, and the torn-out one may be put aside.
      let tab = drag.tab;
      // This window's own bar first: its shell is being updated, the scan cannot read it.
      let own = self
        .bar_slot(screen)
        .and_then(|index| Some((windows::get(self.window, cx)?, index)));
      let target = own.or_else(|| windows::bar_at(screen, &[handle.window_id(), self.window], cx));
      match target {
        Some((target, index)) => {
          cx.defer(move |cx| windows::bring_back(handle, tab, target, index, cx));
        }
        None => cx.defer(move |cx| windows::focus(handle, cx)),
      }
      return;
    }
    if let Some((target, index)) = windows::bar_at(screen, &[self.window], cx) {
      self.join(drag.tab, target, index, cx);
      return;
    }
    if self.tabs.len() <= 1 {
      // Back into its slot: the copy is wherever the pointer was.
      self.set_detached(false, cx);
      return;
    }
    // Detached without a window of its own (none could open): one now, under the pointer.
    let Some(tab) = self.detach_tab(drag.tab, true, cx) else {
      return;
    };
    let bounds = self.torn_bounds(&drag, screen, window.window_bounds().get_bounds().size);
    let display = display_at(screen, cx);
    cx.defer(move |cx| {
      windows::open_with(tab, bounds, display, true, cx);
    });
  }

  /// Tab `id` leaves this window for `target`'s bar, at `index`; emptied, this window is put
  /// aside.
  fn join(&mut self, id: TabId, target: windows::Target, index: usize, cx: &mut Context<Self>) {
    let Some(tab) = self.detach_tab(id, false, cx) else {
      return;
    };
    let (emptied, this) = (self.tabs.is_empty(), self.window);
    // Deferred: other windows are updated, and this one may be put aside.
    cx.defer(move |cx| {
      windows::merge(tab, target, index, cx);
      if emptied {
        windows::retire(this, cx);
      }
    });
  }

  /// Where a tab dropped at `screen` (the screen's coordinates) goes in this window's bar, if
  /// that is over it: its index among the tabs.
  pub(super) fn bar_slot(&self, screen: Point<Pixels>) -> Option<usize> {
    let at = screen - self.tab_layout.origin.get();
    let row = self.tab_layout.row.get();
    let over = at.x >= row.left()
      && at.x <= row.right() + px(BAR_END)
      && at.y >= row.top() - px(DETACH)
      && at.y <= row.bottom() + px(DETACH);
    if !over {
      return None;
    }
    let layout = self.tab_layout.tabs.borrow();
    Some(slot(
      self.tabs.iter().map(|t| {
        layout
          .iter()
          .find(|(id, _)| *id == t.id)
          .map(|(_, b)| b.center().x)
      }),
      at.x,
    ))
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
    let left = drag.at.x - drag.grab.x;
    let max = (end - drag.width).max(row.left());
    left.clamp(row.left(), max) - row.left()
  }

  /// The detached copy's top left corner in the window: where it was held, under the pointer.
  pub(super) fn detached_corner(drag: &TabDrag) -> Point<Pixels> {
    drag.at - drag.grab
  }
}

/// The dragged tab's index among all of them: after each other tab (left to right, its middle
/// if it was drawn) whose middle is left of the copy's. A neighbor passed moves by the dragged
/// tab's width, away from the copy's middle, so the order holds until the copy moves back past
/// it.
fn slot(others: impl Iterator<Item = Option<Pixels>>, middle: Pixels) -> usize {
  others.filter(|m| m.is_some_and(|m| m < middle)).count()
}

/// Whether the pointer at `at` has pulled a tab off the bar `row`: [`DETACH`] above or below
/// it, or out of the window (`viewport`).
fn off_bar(row: Bounds<Pixels>, viewport: Bounds<Pixels>, at: Point<Pixels>) -> bool {
  at.y < row.top() - px(DETACH) || at.y > row.bottom() + px(DETACH) || !viewport.contains(&at)
}

/// The display under `screen`: a window opened there must name it (Windows checks the bounds
/// against it, else against the primary display).
fn display_at(screen: Point<Pixels>, cx: &App) -> Option<DisplayId> {
  cx.displays()
    .into_iter()
    .find(|d| d.bounds().contains(&screen))
    .map(|d| d.id())
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
          this.drop_tab(event.position, window, cx);
        }
      })
      .ok();
  });
  window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
      shell
        .update(cx, |this, cx| this.drop_tab(event.position, window, cx))
        .ok();
    }
  });
}

#[cfg(test)]
mod tests {
  use gpui::{Bounds, point, px, size};

  use super::{off_bar, slot};

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

  #[test]
  fn a_tab_detaches_away_from_the_bar_or_out_of_the_window() {
    let row = Bounds::new(point(px(200.), px(5.)), size(px(600.), px(34.)));
    let viewport = Bounds::new(point(px(0.), px(0.)), size(px(1400.), px(900.)));
    // Along the bar, and a little under it.
    assert!(!off_bar(row, viewport, point(px(400.), px(20.))));
    assert!(!off_bar(row, viewport, point(px(400.), px(60.))));
    // Down into the page.
    assert!(off_bar(row, viewport, point(px(400.), px(80.))));
    // Above the window, or past its side at the bar's height.
    assert!(off_bar(row, viewport, point(px(400.), px(-30.))));
    assert!(off_bar(row, viewport, point(px(1500.), px(20.))));
  }
}
