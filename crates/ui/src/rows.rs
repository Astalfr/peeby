//! Collections of cards: a horizontal row that scrolls with arrow buttons (home rows), and the
//! layout of a virtualized grid of same-sized cards (a library). From peeby's `ui.rs` and
//! `library.rs`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{AnyElement, App, ElementId, ScrollHandle, SharedString, Window, canvas, div, px};
use gpui_component::{Sizable, h_flex};

use crate::assets::Icon;
use crate::theme;

/// Duration of an arrow's page scroll.
const GLIDE: Duration = Duration::from_millis(280);

/// A horizontal row that scrolls, with arrow buttons on hover; an arrow scrolls by 80 % of the
/// row's width with an animation. `id` must be unique on the page.
pub fn scroll_row(
  id: impl Into<SharedString>,
  children: Vec<AnyElement>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let id: SharedString = id.into();
  let state = window.use_keyed_state(
    ElementId::Name(format!("{id}-scroll").into()),
    cx,
    |_, _| RowScroll {
      handle: ScrollHandle::new(),
      glide: None,
    },
  );
  let handle = state.read(cx).handle.clone();
  state.update(cx, |row, _| row.step(window));
  let offset = -f32::from(handle.offset().x);
  let max = f32::from(handle.max_offset().x);
  let group = SharedString::from(format!("{id}-group"));
  let arrow = |left: bool| {
    let handle = handle.clone();
    let state = state.clone();
    div()
      .id(if left { "row-left" } else { "row-right" })
      .absolute()
      .top_0()
      .bottom(px(44.))
      .when(left, |el| el.left_0())
      .when(!left, |el| el.right_0())
      .w(px(56.))
      .flex()
      .items_center()
      .justify_center()
      .cursor_pointer()
      .opacity(0.)
      .group_hover(group.clone(), |s| s.opacity(1.))
      .bg(gpui::linear_gradient(
        if left { 90. } else { 270. },
        gpui::linear_color_stop(theme::background().opacity(0.95), 0.),
        gpui::linear_color_stop(theme::background().opacity(0.), 1.),
      ))
      .on_click(move |_, window, cx| {
        // The arrow sits over the cards: the click is not theirs.
        cx.stop_propagation();
        let viewport = f32::from(handle.bounds().size.width);
        let step = (viewport * 0.8).max(200.) * if left { -1. } else { 1. };
        state.update(cx, |row, _| row.glide_by(step));
        window.refresh();
      })
      .child(
        div()
          .size(px(36.))
          .rounded_full()
          .flex()
          .items_center()
          .justify_center()
          .bg(theme::surface_hover())
          .border_1()
          .border_color(theme::border())
          .hover(|s| s.bg(theme::accent()).text_color(theme::on_accent()))
          .child(
            gpui_component::Icon::new(if left {
              Icon::ChevronLeft
            } else {
              Icon::ChevronRight
            })
            .small(),
          ),
      )
  };
  div()
    .id(ElementId::Name(id.clone()))
    .relative()
    .group(group.clone())
    .child(
      h_flex()
        .id(ElementId::Name(format!("{id}-items").into()))
        .gap_4()
        .py_2()
        .overflow_x_scroll()
        .track_scroll(&handle)
        .children(children),
    )
    .when(offset > 1., |el| el.child(arrow(true)))
    .when(offset < max - 1., |el| el.child(arrow(false)))
    .into_any_element()
}

/// Scroll state of a [`scroll_row`]: its handle and the page scroll in progress, if any.
struct RowScroll {
  handle: ScrollHandle,
  glide: Option<Glide>,
}

/// An animated horizontal scroll, in pixels from the start of the row.
struct Glide {
  from: f32,
  to: f32,
  started: Instant,
  /// Offset set on the last frame: if the handle moved since, the wheel took over.
  last: f32,
}

impl RowScroll {
  fn offset(&self) -> f32 {
    -f32::from(self.handle.offset().x)
  }

  /// Starts a page scroll by `delta` pixels, from where a scroll in progress is heading.
  fn glide_by(&mut self, delta: f32) {
    let from = self.offset();
    let start = self.glide.as_ref().map_or(from, |g| g.to);
    let to = (start + delta).clamp(0., f32::from(self.handle.max_offset().x));
    self.glide = Some(Glide {
      from,
      to,
      started: Instant::now(),
      last: from,
    });
  }

  /// Advances the page scroll for this frame. It runs from the render, once per frame on the
  /// display's clock, so only the window's normal redraw is needed (no timer).
  fn step(&mut self, window: &mut Window) {
    let current = self.offset();
    let Some(glide) = &mut self.glide else {
      return;
    };
    if (current - glide.last).abs() > 0.5 {
      self.glide = None;
      return;
    }
    let t = (glide.started.elapsed().as_secs_f32() / GLIDE.as_secs_f32()).min(1.);
    let eased = 1. - (1. - t).powi(3);
    let x = glide.from + (glide.to - glide.from) * eased;
    glide.last = x;
    self
      .handle
      .set_offset(gpui::point(px(-x), self.handle.offset().y));
    if t < 1. {
      window.request_animation_frame();
    } else {
      self.glide = None;
    }
  }
}

/// Columns of a grid of cards `card_width` wide in `width` pixels, at least `min_gap` apart; the
/// leftover width is spread between the columns, so the grid fills its width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridLayout {
  /// Cards per row, at least 1.
  pub columns: usize,
  /// Space between two cards of a row.
  pub gap: f32,
}

impl GridLayout {
  /// The layout for `width` pixels.
  pub fn new(width: f32, card_width: f32, min_gap: f32) -> Self {
    let columns = (((width + min_gap) / (card_width + min_gap)).floor() as usize).max(1);
    let gap = if columns > 1 {
      ((width - columns as f32 * card_width) / (columns - 1) as f32).max(min_gap)
    } else {
      min_gap
    };
    Self { columns, gap }
  }

  /// Rows needed for `count` cards.
  pub fn rows(&self, count: usize) -> usize {
    count.div_ceil(self.columns)
  }
}

/// The width its parent had at the last frame, measured by [`Width::probe`]: a page lays out
/// its grid from its own width (the shell's sidebar takes the rest of the window).
#[derive(Clone, Default)]
pub struct Width(Rc<Cell<f32>>);

impl Width {
  /// The last width measured, 0 before the first frame.
  pub fn get(&self) -> f32 {
    self.0.get()
  }

  /// An invisible element filling its (relative) parent that records its width, and redraws
  /// the window when it changed, so the layout follows a resize.
  pub fn probe(&self) -> impl IntoElement + use<> {
    let cell = self.0.clone();
    canvas(
      move |bounds, window, _| {
        let width = f32::from(bounds.size.width);
        if (cell.get() - width).abs() > 0.5 {
          cell.set(width);
          window.refresh();
        }
      },
      |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
  }
}

#[cfg(test)]
mod tests {
  use super::GridLayout;

  #[test]
  fn grid_fills_its_width() {
    let layout = GridLayout::new(1000., 160., 16.);
    assert_eq!(layout.columns, 5);
    assert!((layout.gap - 50.).abs() < 0.01);
    assert_eq!(layout.rows(11), 3);
    assert_eq!(GridLayout::new(100., 160., 16.).columns, 1);
  }
}
