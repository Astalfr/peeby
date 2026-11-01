//! Small shared pieces: page title, centered notices, the loader and list-end helpers.

use gpui::prelude::*;
use gpui::{FontWeight, SharedString, Window, div, px};
use gpui_component::{IconNamed, h_flex, v_flex};

use crate::theme;

/// Page title.
pub fn page_title(text: impl Into<SharedString>) -> impl IntoElement {
  div()
    .text_2xl()
    .font_weight(FontWeight::BOLD)
    .child(text.into())
}

/// Centered notice for an empty or failed page: an icon in a soft disc, a title and a body.
/// Callers add their buttons as children.
pub fn notice(
  icon: impl IconNamed,
  title: impl Into<SharedString>,
  body: impl Into<SharedString>,
) -> gpui::Div {
  let body: SharedString = body.into();
  v_flex()
    .max_w(px(520.))
    .mx_auto()
    .mt(px(72.))
    .gap_3()
    .items_center()
    .text_center()
    .child(
      div()
        .size(px(64.))
        .mb_2()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::accent_soft())
        .text_color(theme::accent())
        .child(gpui_component::Icon::new(icon).size(px(28.))),
    )
    .child(
      div()
        .text_xl()
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.into()),
    )
    .when(!body.is_empty(), |el| {
      el.child(div().text_color(theme::muted()).child(body))
    })
}

/// A small muted "loading" line with a spinner, for the end of a list.
pub fn loading_more() -> impl IntoElement {
  h_flex()
    .gap_2()
    .items_center()
    .text_sm()
    .text_color(theme::muted())
    .child(loader(16., theme::accent()))
    .child("Chargement…")
}

/// Length of one grow-and-shrink cycle of the [`loader`]'s arc.
const LOADER_CYCLE: f32 = 1.4;
/// Seconds per turn of the [`loader`]'s steady rotation.
const LOADER_TURN: f32 = 2.0;
/// Shortest and longest arc of the [`loader`], in degrees.
const LOADER_MIN_ARC: f32 = 12.;
const LOADER_MAX_ARC: f32 = 270.;

/// A spinner of `size` pixels: a faint ring with a round-capped arc turning inside it, which
/// stretches (its head runs ahead) and shrinks (its tail catches up) as it turns, as Material
/// spinners do. Drawn on a canvas from a shared clock, so every loader turns in step and asks
/// for the next frame itself.
pub fn loader(size: f32, color: gpui::Hsla) -> impl IntoElement {
  div().size(px(size)).flex_none().child(
    gpui::canvas(
      |_, _, _| {},
      move |bounds, _, window, _| {
        paint_loader(bounds, color, window);
        window.request_animation_frame();
      },
    )
    .size_full(),
  )
}

fn paint_loader(bounds: gpui::Bounds<gpui::Pixels>, color: gpui::Hsla, window: &mut Window) {
  static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
  let time = EPOCH
    .get_or_init(std::time::Instant::now)
    .elapsed()
    .as_secs_f32();

  let side = f32::from(bounds.size.width.min(bounds.size.height));
  let width = (side * 0.14).max(2.);
  let radius = (side - width) / 2.;
  let center = bounds.center();
  // Clockwise from the top, in degrees.
  let point = |degrees: f32| {
    let a = degrees.to_radians();
    gpui::point(
      center.x + px(radius * a.sin()),
      center.y - px(radius * a.cos()),
    )
  };

  // The track: two half circles (an arc cannot end where it starts).
  let mut ring = gpui::PathBuilder::stroke(px(width));
  ring.move_to(point(0.));
  ring.arc_to(
    gpui::point(px(radius), px(radius)),
    px(0.),
    false,
    true,
    point(180.),
  );
  ring.arc_to(
    gpui::point(px(radius), px(radius)),
    px(0.),
    false,
    true,
    point(360.),
  );
  if let Ok(path) = ring.build() {
    window.paint_path(path, color.opacity(0.18));
  }

  // The arc: in the first half of a cycle its head runs ahead, in the second its tail catches
  // up; each cycle starts where the last one's tail stopped, on top of a steady rotation.
  let cycle = (time / LOADER_CYCLE).floor();
  let t = time / LOADER_CYCLE - cycle;
  let ease = |x: f32| {
    if x < 0.5 {
      4. * x * x * x
    } else {
      1. - (-2. * x + 2.).powi(3) / 2.
    }
  };
  let stretch = LOADER_MAX_ARC - LOADER_MIN_ARC;
  let (head, tail) = if t < 0.5 {
    (LOADER_MIN_ARC + stretch * ease(t * 2.), 0.)
  } else {
    (LOADER_MAX_ARC, stretch * ease(t * 2. - 1.))
  };
  let offset = (cycle * stretch + time / LOADER_TURN * 360.) % 360.;
  let (start, end) = (offset + tail, offset + head);
  let mut arc = gpui::PathBuilder::stroke(px(width));
  arc.move_to(point(start));
  arc.arc_to(
    gpui::point(px(radius), px(radius)),
    px(0.),
    end - start > 180.,
    true,
    point(end),
  );
  if let Ok(path) = arc.build() {
    window.paint_path(path, color);
  }
  // Round caps: lyon's caps are not reachable through GPUI, so a disc ends each side.
  for degrees in [start, end] {
    let cap = gpui::Bounds::centered_at(point(degrees), gpui::size(px(width), px(width)));
    window.paint_quad(gpui::fill(cap, color).corner_radii(px(width / 2.)));
  }
}

/// Whether a scroll area is within about a screen of its end, to load the next page early.
pub fn near_scroll_end(handle: &gpui::ScrollHandle) -> bool {
  let max = f32::from(handle.max_offset().y);
  let scrolled = -f32::from(handle.offset().y);
  max - scrolled < 1200.
}
