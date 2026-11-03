//! The list of lives the notification area's icon opens (ADR 0020): a small window above the
//! icon, a tab per space that has lives (`Space::lives`), the followed channels on air in each.
//! A click on one shows the main window on its live; the list closes when it loses the focus,
//! with Escape, or with another click on the icon. It rises from the taskbar as it opens, a
//! tab's list slides in from the side of its tab and its rows come in from the right one after
//! the other (none of it with reduced motion).

use std::time::{Duration, Instant, SystemTime};

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, App, Bounds, Context, DisplayId, ElementId, FocusHandle,
  FontWeight, Global, KeyDownEvent, SharedString, Subscription, Window, WindowBackgroundAppearance,
  WindowBounds, WindowHandle, WindowKind, WindowOptions, div, ease_out_quint, point, px, relative,
  size,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use ui::i18n::t;
use ui::images::Images;
use ui::{assets, motion, picture, tf, theme, widgets};

use super::space::{LiveEntry, Lives, SpaceHandle};
use crate::desktop;

const WIDTH: f32 = 380.;
const MAX_HEIGHT: f32 = 560.;
/// Between the list and the screen's edges or the icon.
const MARGIN: f32 = 12.;
const AVATAR: f32 = 40.;
/// How far the list rises from the taskbar as it opens.
const RISE: f32 = 16.;
/// How far a tab's list slides in from the side of its tab.
const SLIDE: f32 = 24.;
/// How far a row comes in from the right.
const ROW_SLIDE: f32 = 32.;
/// Between two rows coming in, and how many rows wait for the ones above at most.
const STAGGER: Duration = Duration::from_millis(30);
const STAGGERED: usize = 10;

/// The icon the list opens from: its rectangle, physical pixels of the virtual screen.
#[derive(Clone, Copy, Debug)]
pub struct Anchor {
  pub x: i32,
  pub y: i32,
  pub width: i32,
  pub height: i32,
}

/// The list's window while it is open, when it last closed, and the tab last shown.
#[derive(Default)]
struct State {
  window: Option<WindowHandle<LivesView>>,
  closed: Option<Instant>,
  tab: Option<SharedString>,
}

impl Global for State {}

/// When the list last closed: a click on the icon takes the focus from it first, and that click
/// must not open it again.
pub fn closed_at(cx: &App) -> Option<Instant> {
  cx.try_global::<State>().and_then(|s| s.closed)
}

/// Whether the list is open.
pub fn is_open(cx: &App) -> bool {
  cx.try_global::<State>().is_some_and(|s| s.window.is_some())
}

/// Opens the list over `anchor` on the lives of `spaces` (those that have some).
pub fn open(spaces: &[SpaceHandle], anchor: Anchor, cx: &mut App) {
  if is_open(cx) {
    return;
  }
  let spaces: Vec<SpaceHandle> = spaces
    .iter()
    .filter(|s| s.lives(cx).is_some())
    .cloned()
    .collect();
  if spaces.is_empty() {
    return;
  }
  for space in &spaces {
    space.refresh_lives(cx);
  }
  let (display, bounds, from_below) = place(anchor, cx);
  let options = WindowOptions {
    window_bounds: Some(WindowBounds::Windowed(bounds)),
    // Windows checks the bounds against this display, else the primary one (CLAUDE.md).
    display_id: display,
    kind: WindowKind::PopUp,
    titlebar: None,
    focus: true,
    is_resizable: false,
    is_movable: false,
    is_minimizable: false,
    // The rounded panel rises inside it, and its corners show what is behind.
    window_background: WindowBackgroundAppearance::Transparent,
    ..Default::default()
  };
  // Without gpui-component's `Root`, which paints the window's background.
  let opened = cx.open_window(options, |window, cx| {
    cx.new(|cx| LivesView::new(spaces, from_below, window, cx))
  });
  match opened {
    Ok(handle) => {
      handle
        .update(cx, |_, window, _| window.activate_window())
        .ok();
      cx.default_global::<State>().window = Some(handle);
    }
    Err(e) => tracing::warn!("cannot open the list of lives: {e:#}"),
  }
}

/// Closes the list if it is open.
pub fn close(cx: &mut App) {
  let state = cx.default_global::<State>();
  let Some(handle) = state.window.take() else {
    return;
  };
  state.closed = Some(Instant::now());
  handle
    .update(cx, |_, window, _| window.remove_window())
    .ok();
}

/// Where the list goes: on the screen of the icon, against the taskbar, centered on the icon
/// as far as the work area allows; and whether the icon is below it (it rises from there).
fn place(anchor: Anchor, cx: &App) -> (Option<DisplayId>, Bounds<gpui::Pixels>, bool) {
  let (cx_phys, cy_phys) = (anchor.x + anchor.width / 2, anchor.y + anchor.height / 2);
  let (monitor, scale) = desktop::sys::monitor_at(cx_phys, cy_phys);
  let display = cx
    .displays()
    .into_iter()
    .find(|d| u64::from(d.id()) == monitor)
    .or_else(|| cx.primary_display());
  let Some(display) = display else {
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(WIDTH), px(MAX_HEIGHT)));
    return (None, bounds, true);
  };
  let area = display.visible_bounds();
  let (left, top) = (f32::from(area.origin.x), f32::from(area.origin.y));
  let (right, bottom) = (
    left + f32::from(area.size.width),
    top + f32::from(area.size.height),
  );
  let height = MAX_HEIGHT.min(bottom - top - 2. * MARGIN);
  let (icon_x, icon_y) = (cx_phys as f32 / scale, cy_phys as f32 / scale);
  let icon_top = anchor.y as f32 / scale;
  let clamp = |v: f32, low: f32, high: f32| v.max(low).min(high.max(low));
  let x = clamp(icon_x - WIDTH / 2., left + MARGIN, right - WIDTH - MARGIN);
  // In the work area (the taskbar's overflow): above the icon. Else (the taskbar) against it.
  let y = if (top..bottom).contains(&icon_y) {
    clamp(
      icon_top - height - MARGIN,
      top + MARGIN,
      bottom - height - MARGIN,
    )
  } else {
    clamp(icon_y - height / 2., top + MARGIN, bottom - height - MARGIN)
  };
  (
    Some(display.id()),
    Bounds::new(point(px(x), px(y)), size(px(WIDTH), px(height))),
    icon_y > y + height / 2.,
  )
}

struct LivesView {
  spaces: Vec<SpaceHandle>,
  /// The space whose tab shows, by name.
  tab: SharedString,
  /// Where the tab shown came from: -1 from a tab on its left, 1 on its right, 0 at the start.
  came_from: f32,
  /// The icon is below the list.
  from_below: bool,
  focus: FocusHandle,
  _subscriptions: Vec<Subscription>,
}

impl LivesView {
  fn new(
    spaces: Vec<SpaceHandle>,
    from_below: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut subscriptions = Vec::new();
    for space in &spaces {
      let view = cx.weak_entity();
      subscriptions.push(space.observe(
        cx,
        Box::new(move |cx| {
          view.update(cx, |_, cx| cx.notify()).ok();
        }),
      ));
    }
    if let Some(images) = cx.try_global::<Images>().cloned() {
      subscriptions.push(cx.observe(&images.0, |_, _, cx| cx.notify()));
    }
    // Clicking elsewhere takes the focus: the list closes, after the event.
    subscriptions.push(cx.observe_window_activation(window, |_, window, cx| {
      if !window.is_window_active() {
        cx.defer(close);
      }
    }));
    // The tab last shown, else the first with someone on air, else the first.
    let names: Vec<SharedString> = spaces.iter().map(|s| s.info(cx).name).collect();
    let remembered = cx
      .try_global::<State>()
      .and_then(|s| s.tab.clone())
      .filter(|tab| names.contains(tab));
    let on_air = spaces
      .iter()
      .find(|s| matches!(s.lives(cx), Some(Lives::On(l)) if !l.is_empty()))
      .map(|s| s.info(cx).name);
    let tab = remembered
      .or(on_air)
      .or_else(|| names.first().cloned())
      .unwrap_or_default();
    let focus = cx.focus_handle();
    window.focus(&focus, cx);
    Self {
      spaces,
      tab,
      came_from: 0.,
      from_below,
      focus,
      _subscriptions: subscriptions,
    }
  }

  fn select(&mut self, tab: SharedString, cx: &mut Context<Self>) {
    if tab == self.tab {
      return;
    }
    let index = |name: &SharedString| {
      self
        .spaces
        .iter()
        .position(|s| s.info(cx).name == *name)
        .unwrap_or(0)
    };
    self.came_from = if index(&tab) > index(&self.tab) {
      1.
    } else {
      -1.
    };
    cx.default_global::<State>().tab = Some(tab.clone());
    self.tab = tab;
    cx.notify();
  }

  /// Shows the main window on the live `id` of `space`.
  fn open_live(&self, space: SpaceHandle, id: SharedString, cx: &mut Context<Self>) {
    cx.defer(move |cx| {
      close(cx);
      desktop::show_main(
        cx,
        Some(Box::new(move |window, cx| space.open_live(&id, window, cx))),
      );
    });
  }

  fn tabs(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let mut bar = h_flex().px_2().gap_1();
    for space in &self.spaces {
      let info = space.info(cx);
      let count = match space.lives(cx) {
        Some(Lives::On(lives)) => Some(lives.len()),
        _ => None,
      };
      let selected = info.name == self.tab;
      let name = info.name.clone();
      // The underline grows from the middle of the tab chosen and shrinks under the other.
      let shown = if selected { 1. } else { 0. };
      let line = if cx.reduce_motion() {
        shown
      } else {
        motion::tween(
          ElementId::Name(format!("lives-line-{}", info.name).into()),
          shown,
          shown,
          theme::NORMAL,
          window,
          cx,
        )
      };
      bar = bar.child(
        h_flex()
          .id(ElementId::Name(format!("lives-tab-{}", info.name).into()))
          .relative()
          .px_3()
          .py_2()
          .gap_2()
          .cursor_pointer()
          .text_sm()
          .text_color(if selected {
            theme::foreground()
          } else {
            theme::muted()
          })
          .hover(|s| s.text_color(theme::foreground()))
          .on_click(cx.listener(move |this, _, _, cx| this.select(name.clone(), cx)))
          .child(assets::color_svg(info.icon, 16., window, cx))
          .child(info.name.clone())
          .children(count.map(|n| {
            div()
              .px_1p5()
              .rounded_full()
              .text_xs()
              .bg(if selected {
                info.accent.base
              } else {
                theme::surface_hover()
              })
              .text_color(if selected {
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

  /// The tab's list, sliding in from the side of the tab it came from.
  fn list(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let content = self.content(window, cx);
    let slide = SLIDE * self.came_from;
    let list = div()
      .flex_1()
      .min_h_0()
      .flex()
      .flex_col()
      .relative()
      .child(content);
    animated(
      list,
      ElementId::Name(format!("lives-tab-list-{}", self.tab).into()),
      Animation::new(theme::NORMAL).with_easing(ease_out_quint()),
      move |el, t| el.opacity(t).left(px(slide * (1. - t))),
      cx,
    )
  }

  fn content(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
    let Some(space) = self.spaces.iter().find(|s| s.info(cx).name == self.tab) else {
      return div().into_any_element();
    };
    let lives = match space.lives(cx) {
      Some(Lives::On(lives)) if !lives.is_empty() => lives,
      Some(Lives::On(_)) => {
        return message(t(
          "No followed channel is on air.",
          "Aucune chaîne suivie n'est en direct.",
        ));
      }
      Some(Lives::Unavailable(why)) => return message(why),
      Some(Lives::Loading) | None => {
        return div()
          .flex_1()
          .flex()
          .items_center()
          .justify_center()
          .child(widgets::loader(24., theme::muted()))
          .into_any_element();
      }
    };
    let now = SystemTime::now();
    let mut list = v_flex()
      .id("lives-list")
      .flex_1()
      .min_h_0()
      .overflow_y_scroll()
      .p_1();
    for (i, live) in lives.into_iter().enumerate() {
      // A row waits for the ones above it (the first ones only), then comes in from the right.
      let delay = STAGGER * i.min(STAGGERED) as u32;
      let total = theme::NORMAL + delay;
      let wait = delay.as_secs_f32() / total.as_secs_f32();
      let ease = ease_out_quint();
      let key = format!("lives-row-{}-{}", self.tab, live.id);
      let row = self.row(space.clone(), live, now, window, cx);
      list = list.child(animated(
        div().relative().child(row),
        ElementId::Name(key.into()),
        Animation::new(total).with_easing(move |t| ease(((t - wait) / (1. - wait)).max(0.))),
        |el, t| el.opacity(t).left(px(ROW_SLIDE * (1. - t))),
        cx,
      ));
    }
    list.into_any_element()
  }

  fn row(
    &self,
    space: SpaceHandle,
    live: LiveEntry,
    now: SystemTime,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let red = theme::Accent::red().base;
    // The tab's space's accent, as its pages have it: a soft tint and the name in the accent.
    let accent = space.info(cx).accent;
    let group = SharedString::from(format!("live-row-{}", live.id));
    let avatar = live
      .avatar
      .picture
      .as_ref()
      .map(|p| picture::avatar(p, AVATAR - 4., window, cx));
    let uptime = live.since.and_then(|since| now.duration_since(since).ok());
    let id = live.id.clone();
    h_flex()
      .id(ElementId::Name(format!("live-{}", live.id).into()))
      .w_full()
      .px_2()
      .py_2()
      .gap_3()
      .group(group.clone())
      .rounded(theme::RADIUS)
      .cursor_pointer()
      .hover(move |s| s.bg(accent.base.opacity(0.14)))
      .on_click(cx.listener(move |this, _, _, cx| this.open_live(space.clone(), id.clone(), cx)))
      // The avatar in a red ring, its initial while it loads.
      .child(
        div()
          .size(px(AVATAR))
          .flex_none()
          .rounded_full()
          .border_2()
          .border_color(red)
          .flex()
          .items_center()
          .justify_center()
          .child(
            div()
              .size(px(AVATAR - 4.))
              .rounded_full()
              .bg(theme::surface_hover())
              .flex()
              .items_center()
              .justify_center()
              .text_sm()
              .text_color(theme::muted())
              .when(avatar.is_none(), |el| el.child(live.avatar.initial.clone()))
              .children(avatar),
          ),
      )
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .gap_0p5()
          .child(
            h_flex()
              .gap_2()
              .child(
                div()
                  .flex_1()
                  .min_w_0()
                  .truncate()
                  .text_sm()
                  .font_weight(FontWeight::SEMIBOLD)
                  .group_hover(group, move |s| s.text_color(accent.hover))
                  .child(live.channel.clone()),
              )
              .children(live.viewers.clone().map(|viewers| {
                h_flex()
                  .flex_none()
                  .gap_1()
                  .text_xs()
                  .text_color(theme::muted())
                  .child(div().size(px(6.)).rounded_full().bg(red))
                  .child(viewers)
              })),
          )
          .when(live.detail.is_some() || uptime.is_some(), |el| {
            el.child(
              h_flex()
                .gap_2()
                .text_xs()
                .text_color(theme::muted())
                .child(
                  div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .children(live.detail.clone()),
                )
                .children(uptime.map(|d| div().flex_none().child(duration(d.as_secs())))),
            )
          })
          .child(
            div()
              .truncate()
              .text_xs()
              .text_color(theme::subtle())
              .child(live.title.clone()),
          ),
      )
      .into_any_element()
  }
}

impl Render for LivesView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // What gpui-component's `Root` would set: the theme's font and its rem size.
    window.set_rem_size(cx.theme().font_size);
    let font = cx.theme().font_family.clone();
    let tabs = self.tabs(window, cx);
    let list = self.list(window, cx);
    let rise = if self.from_below { RISE } else { -RISE };
    let panel = v_flex()
      .id("lives")
      .track_focus(&self.focus)
      .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
        if event.keystroke.key == "escape" {
          cx.defer(close);
        }
      }))
      .relative()
      .size_full()
      .overflow_hidden()
      .rounded(theme::RADIUS_LG)
      .bg(theme::base())
      .border_1()
      .border_color(theme::border())
      .font_family(font)
      .text_color(theme::foreground())
      .child(
        h_flex()
          .pt_1()
          .border_b_1()
          .border_color(theme::border())
          .child(tabs),
      )
      .child(list)
      .child(
        h_flex()
          .id("lives-open")
          .px_3()
          .py_2()
          .border_t_1()
          .border_color(theme::border())
          .justify_center()
          .text_xs()
          .text_color(theme::muted())
          .cursor_pointer()
          .hover(|s| s.text_color(theme::foreground()).bg(theme::surface_hover()))
          .on_click(|_, _, cx| {
            cx.defer(|cx| {
              close(cx);
              desktop::show_main(cx, None);
            })
          })
          .child(t("Open Peeby", "Ouvrir Peeby")),
      );
    animated(
      panel,
      "lives-open",
      Animation::new(theme::SLOW).with_easing(ease_out_quint()),
      move |el, t| el.opacity(t).top(px(rise * (1. - t))),
      cx,
    )
  }
}

/// `element` animated by `animator`, or as it is with reduced motion.
fn animated<E: IntoElement + 'static>(
  element: E,
  id: impl Into<ElementId>,
  animation: Animation,
  animator: impl Fn(E, f32) -> E + 'static,
  cx: &App,
) -> AnyElement {
  if cx.reduce_motion() {
    return element.into_any_element();
  }
  element
    .with_animation(id, animation, animator)
    .into_any_element()
}

/// The tab's text when nobody shows.
fn message(text: impl Into<SharedString>) -> AnyElement {
  div()
    .flex_1()
    .p_6()
    .flex()
    .items_center()
    .justify_center()
    .text_sm()
    .text_center()
    .text_color(theme::muted())
    .child(text.into())
    .into_any_element()
}

/// A live's uptime: `45 min`, `2 h 05`.
fn duration(seconds: u64) -> String {
  let minutes = seconds / 60;
  match minutes {
    0..60 => tf!("{minutes} min", "{minutes} min"),
    _ => {
      let (hours, minutes) = (minutes / 60, minutes % 60);
      tf!("{hours} h {minutes:02}", "{hours} h {minutes:02}")
    }
  }
}

#[cfg(test)]
mod tests {
  use super::duration;

  #[test]
  fn uptimes() {
    assert_eq!(duration(59), "0 min");
    assert_eq!(duration(45 * 60), "45 min");
    assert_eq!(duration(2 * 3600 + 5 * 60 + 30), "2 h 05");
  }
}
