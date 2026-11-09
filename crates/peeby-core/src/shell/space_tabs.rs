//! A tab per space over a panel, as the notification area's list of lives and the bell's panel
//! have it: the space's logo and name, an optional count (in the accent on the tab chosen), an
//! underline in the accent that grows from the middle of the tab chosen and shrinks under the
//! other, and the tab's content sliding in from the side of the tab it came from (none of it
//! with reduced motion).

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, App, ElementId, SharedString, Window, div, ease_out_quint,
  px, relative,
};
use gpui_component::h_flex;
use ui::{assets, motion, theme};

use super::space::SpaceInfo;

/// How far a tab's content slides in from the side of its tab.
const SLIDE: f32 = 24.;

/// One tab: its space and the count it shows, if any.
pub struct SpaceTab {
  /// The space.
  pub info: SpaceInfo,
  /// Shown next to the name (lives on air, unread notifications).
  pub count: Option<usize>,
}

/// What a click on a tab does, with the name of its space.
pub type OnSelect = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

/// The bar of `tabs`, `selected` (a space's name) chosen; `id` keeps it apart from another bar.
pub fn bar(
  id: &str,
  tabs: &[SpaceTab],
  selected: &SharedString,
  on_select: OnSelect,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let mut bar = h_flex().px_2().gap_1();
  for tab in tabs {
    let info = &tab.info;
    let chosen = info.name == *selected;
    let shown = if chosen { 1. } else { 0. };
    let line = if cx.reduce_motion() {
      shown
    } else {
      motion::tween(
        ElementId::Name(format!("{id}-line-{}", info.name).into()),
        shown,
        shown,
        theme::NORMAL,
        window,
        cx,
      )
    };
    let name = info.name.clone();
    let on_select = on_select.clone();
    bar = bar.child(
      h_flex()
        .id(ElementId::Name(format!("{id}-tab-{}", info.name).into()))
        .relative()
        .px_3()
        .py_2()
        .gap_2()
        .cursor_pointer()
        .text_sm()
        .text_color(if chosen {
          theme::foreground()
        } else {
          theme::muted()
        })
        .hover(|s| s.text_color(theme::foreground()))
        .on_click(move |_, window, cx| on_select(name.clone(), window, cx))
        .child(assets::color_svg(info.icon, 16., window, cx))
        .child(info.name.clone())
        .children(tab.count.map(|n| {
          div()
            .px_1p5()
            .rounded_full()
            .text_xs()
            .bg(if chosen {
              info.accent.base
            } else {
              theme::surface_hover()
            })
            .text_color(if chosen {
              theme::on_accent()
            } else {
              theme::muted()
            })
            .child(n.to_string())
        }))
        .when(line > 0., |el| {
          el.child(
            h_flex()
              .absolute()
              .bottom_0()
              .left_0()
              .right_0()
              .justify_center()
              .child(
                div()
                  .h(px(2.))
                  .w(relative(line))
                  .rounded_full()
                  .bg(info.accent.base)
                  .opacity(line),
              ),
          )
        }),
    );
  }
  bar.into_any_element()
}

/// Where the tab `to` comes from, seen from `from`: 1 from the right, -1 from the left (a slide's
/// direction).
pub fn direction(tabs: &[SpaceTab], from: &SharedString, to: &SharedString) -> f32 {
  let index = |name: &SharedString| tabs.iter().position(|t| t.info.name == *name);
  if index(to) > index(from) { 1. } else { -1. }
}

/// `content`, the tab `selected`'s, sliding in from `came_from` (see [`direction`]; 0 at the
/// start: it only fades in).
pub fn sliding(
  id: &str,
  selected: &SharedString,
  came_from: f32,
  content: impl IntoElement,
  cx: &App,
) -> AnyElement {
  let element = div()
    .flex_1()
    .min_h_0()
    .flex()
    .flex_col()
    .relative()
    .child(content);
  if cx.reduce_motion() {
    return element.into_any_element();
  }
  let slide = SLIDE * came_from;
  element
    .with_animation(
      ElementId::Name(format!("{id}-tab-content-{selected}").into()),
      Animation::new(theme::NORMAL).with_easing(ease_out_quint()),
      move |el, t| el.opacity(t).left(px(slide * (1. - t))),
    )
    .into_any_element()
}
