//! Moving a tab along the bar with the mouse, as in a browser (ADR 0028): pressed and moved past
//! [`THRESHOLD`], the tab leaves its slot empty and a copy of it follows the pointer; when the
//! copy's middle passes a neighbor's, the tab takes that neighbor's place. The drag is followed
//! with window listeners rather than GPUI's `on_drag`, which ends at the window's edge: tearing a
//! tab out to a window of its own needs the release wherever it happens (the window captures the
//! mouse while the button is held).

use std::cell::{Cell, RefCell};

use gpui::{
  Bounds, Context, DispatchPhase, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point,
  WeakEntity, Window,
};

use super::Shell;
use super::space::TabId;

/// How far the pointer moves, held, before the tab follows it: less is still a click.
const THRESHOLD: f32 = 4.;

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
    self.reorder_dragged();
    cx.notify();
  }

  /// Puts the dragged tab after every other tab whose middle is left of the copy's.
  fn reorder_dragged(&mut self) {
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
    if from != target {
      let tab = self.tabs.remove(from);
      self.tabs.insert(target, tab);
    }
  }

  /// The button was released (or found released): the tab stays where it is.
  fn drop_tab(&mut self, cx: &mut Context<Self>) {
    if self.tab_drag.take().is_some() {
      cx.notify();
    }
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
