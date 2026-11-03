//! Media cards: a picture, a title and muted lines under it, in the layouts the pages use (grid
//! card, poster, compact card, list row, numbered row), their skeletons, and the buttons they
//! carry.
//! Spaces fill a [`MediaCard`] from their own data. The look and the motion follow
//! docs/design.md.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  AnyElement, App, ElementId, Entity, FontWeight, Pixels, SharedString, Window, div, px, relative,
};
use gpui_component::h_flex;
use gpui_component::skeleton::Skeleton;
use gpui_component::v_flex;

use crate::assets::Icon;
use crate::picture::{Picture, avatar, image_fill};
use crate::widgets::loader;
use crate::{motion, theme};

/// Side of the avatar next to a grid card's title.
const AVATAR: f32 = 36.;
/// Width of a compact card's thumbnail.
pub const COMPACT_WIDTH: f32 = 168.;
/// Width of a list row's thumbnail.
pub const ROW_WIDTH: f32 = 246.;
/// Aspect ratio of video thumbnails.
pub const WIDE: f32 = 16. / 9.;
/// Aspect ratio of posters (movies, series).
pub const TALL: f32 = 2. / 3.;
/// Height of a poster's two text lines, under its picture.
const POSTER_TEXT: f32 = 52.;

/// What a click does.
pub type Handler = dyn Fn(&mut Window, &mut App);
/// Called when the mouse enters (`true`) or leaves a card.
pub type HoverHandler = dyn Fn(bool, &mut App);

/// The hover state of a card: whether the mouse is on it, and the hover animation (0 to 1).
/// Read it before building the card when its buttons fade in with the hover.
#[derive(Clone)]
pub struct Hover {
  /// The mouse is on the card.
  pub hovered: bool,
  /// The hover animation, 0 (out) to 1 (in).
  pub t: f32,
  state: Entity<bool>,
}

impl Hover {
  /// The hover state of the card `id`. Ids must be unique on the page: two elements with one id
  /// share their state, and their hover flags overwrite each other on every frame.
  pub fn new(id: &str, window: &mut Window, cx: &mut App) -> Self {
    let state: Entity<bool> =
      motion::hover_state(SharedString::from(format!("{id}-hover")), window, cx);
    let hovered = *state.read(cx);
    let t = motion::tween(
      SharedString::from(format!("{id}-lift")),
      0.,
      if hovered { 1. } else { 0. },
      theme::FAST,
      window,
      cx,
    );
    Self { hovered, t, state }
  }
}

/// An action of a row, such as removing it from its playlist: what it does, its tooltip, and
/// whether it is running (its button then shows a spinner).
pub struct RowAction {
  /// Runs the action.
  pub run: Box<Handler>,
  /// Tooltip of the button.
  pub label: SharedString,
  /// The action is running.
  pub busy: bool,
}

/// A sign in a picture's top right corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
  /// Watched: a check.
  Watched,
  /// Items left to watch (a series' unwatched episodes).
  Count(u32),
}

/// A media item as a card. Build it, then draw it with one of the layouts.
pub struct MediaCard {
  id: SharedString,
  hover: Hover,
  title: SharedString,
  lines: Vec<SharedString>,
  picture: Picture,
  avatar: Option<Picture>,
  badge: Option<SharedString>,
  live: bool,
  progress: Option<f32>,
  corner: Option<Corner>,
  play: bool,
  title_click: Option<Rc<Handler>>,
  title_middle_click: Option<Rc<Handler>>,
  overlays: Vec<AnyElement>,
  trailing: Option<AnyElement>,
  on_click: Option<Rc<Handler>>,
  on_middle_click: Option<Rc<Handler>>,
  on_hover: Option<Box<HoverHandler>>,
}

impl MediaCard {
  /// A card with its id (see [`Hover::new`]), hover state, title and picture.
  pub fn new(
    id: impl Into<SharedString>,
    hover: Hover,
    title: impl Into<SharedString>,
    picture: Picture,
  ) -> Self {
    Self {
      id: id.into(),
      hover,
      title: title.into(),
      lines: Vec::new(),
      picture,
      avatar: None,
      badge: None,
      live: false,
      progress: None,
      corner: None,
      play: true,
      title_click: None,
      title_middle_click: None,
      overlays: Vec::new(),
      trailing: None,
      on_click: None,
      on_middle_click: None,
      on_hover: None,
    }
  }

  /// Muted lines under the title (channel, views · date…), empty ones left out.
  pub fn lines(mut self, lines: impl IntoIterator<Item = impl Into<SharedString>>) -> Self {
    self.lines = lines
      .into_iter()
      .map(Into::into)
      .filter(|l: &SharedString| !l.is_empty())
      .collect();
    self
  }

  /// A round avatar next to the title (grid) or the first line (row).
  pub fn avatar(mut self, avatar: Option<Picture>) -> Self {
    self.avatar = avatar.filter(|a| !a.is_empty());
    self
  }

  /// Text in a dark badge at the picture's bottom right (duration).
  pub fn badge(mut self, badge: Option<impl Into<SharedString>>) -> Self {
    self.badge = badge.map(Into::into);
    self
  }

  /// Whether the item is live now: a "LIVE" badge on the accent takes the badge's place.
  pub fn live(mut self, live: bool) -> Self {
    self.live = live;
    self
  }

  /// How far the item was watched, 0 to 1: a bar at the bottom of the picture.
  pub fn progress(mut self, progress: Option<f32>) -> Self {
    self.progress = progress;
    self
  }

  /// A sign in the picture's top right corner.
  pub fn corner(mut self, corner: Option<Corner>) -> Self {
    self.corner = corner;
    self
  }

  /// Whether a play button shows over the picture on hover (on by default): off for cards that
  /// open a page rather than play.
  pub fn play(mut self, play: bool) -> Self {
    self.play = play;
    self
  }

  /// Makes the title (a poster's) a link of its own, e.g. an episode's series name.
  pub fn on_title_click(mut self, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Self {
    self.title_click = Some(Rc::new(on_click));
    self
  }

  /// What a middle click on the title link does (opens it in a new tab).
  pub fn on_title_middle_click(
    mut self,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
  ) -> Self {
    self.title_middle_click = Some(Rc::new(on_click));
    self
  }

  /// An element over the picture, such as a button in its corner.
  pub fn overlay(mut self, overlay: impl IntoElement) -> Self {
    self.overlays.push(overlay.into_any_element());
    self
  }

  /// An element at the end of the text, such as a menu button.
  pub fn trailing(mut self, trailing: impl IntoElement) -> Self {
    self.trailing = Some(trailing.into_any_element());
    self
  }

  /// What a click on the card does.
  pub fn on_click(mut self, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Self {
    self.on_click = Some(Rc::new(on_click));
    self
  }

  /// What a middle click on the card does (opens it in a new tab).
  pub fn on_middle_click(mut self, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Self {
    self.on_middle_click = Some(Rc::new(on_click));
    self
  }

  /// Called when the mouse enters (`true`) or leaves the card, e.g. to prefetch what a click
  /// would open.
  pub fn on_hover(mut self, on_hover: impl Fn(bool, &mut App) + 'static) -> Self {
    self.on_hover = Some(Box::new(on_hover));
    self
  }

  /// The clickable box of the card.
  fn frame(&mut self) -> gpui::Stateful<gpui::Div> {
    let state = self.hover.state.clone();
    let on_hover = self.on_hover.take();
    let on_click = self.on_click.take();
    let on_middle_click = self.on_middle_click.take();
    div()
      .id(ElementId::Name(self.id.clone()))
      .cursor_pointer()
      // GPUI takes a single hover listener per element: the hover flag and the caller's share it.
      .on_hover(move |hover, window, cx| {
        if let Some(on_hover) = &on_hover {
          on_hover(*hover, cx);
        }
        state.update(cx, |h, _| *h = *hover);
        window.refresh();
      })
      .when_some(on_click, |el, on_click| {
        el.on_click(move |_, window, cx| on_click(window, cx))
      })
      .when_some(on_middle_click, |el, on_click| {
        el.on_aux_click(move |event, window, cx| {
          if event.is_middle_click() {
            on_click(window, cx);
          }
        })
      })
  }

  fn title_color(&self, highlighted: bool) -> gpui::Hsla {
    if highlighted || self.hover.hovered {
      theme::accent_hover()
    } else {
      theme::foreground()
    }
  }

  /// The picture `width` wide at the 16:9 ratio, with the play button, badges, progress and
  /// overlays.
  fn thumbnail(
    &mut self,
    width: f32,
    radius: Pixels,
    play: Option<f32>,
    window: &mut Window,
    cx: &mut App,
  ) -> gpui::Div {
    self.picture_box(width, WIDE, radius, play, window, cx)
  }

  /// The picture `width` wide at the `aspect` ratio, with what goes over it.
  fn picture_box(
    &mut self,
    width: f32,
    aspect: f32,
    radius: Pixels,
    play: Option<f32>,
    window: &mut Window,
    cx: &mut App,
  ) -> gpui::Div {
    let t = self.hover.t;
    let play = play.filter(|_| self.play);
    div()
      .relative()
      .w(px(width))
      .h(px(width / aspect))
      .flex_none()
      .rounded(radius)
      .child(image_fill(&self.picture, width, aspect, radius, window, cx))
      .children(play.map(|size| play_overlay(t, size, radius)))
      .children(self.progress.map(progress_bar))
      .children(self.corner.map(corner))
      .when(self.live, |el| el.child(live_badge()))
      .children(self.badge.take().filter(|_| !self.live).map(badge))
      .children(self.overlays.drain(..))
  }

  /// Grid card, `width` wide: the 16:9 picture, then the avatar, title and lines. On hover it
  /// rises a little, a ring and a play button fade in, and the title takes the accent.
  pub fn grid(mut self, width: f32, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = self.hover.t;
    let title_color = self.title_color(false);
    let avatar = self.avatar.take().map(|a| avatar(&a, AVATAR, window, cx));
    let thumb = self
      .thumbnail(width, theme::RADIUS_LG, Some(52.), window, cx)
      .w_full()
      .when(t > 0.01, |el| el.shadow_lg())
      .child(
        div()
          .absolute()
          .inset_0()
          .rounded(theme::RADIUS_LG)
          .border_2()
          .border_color(theme::accent().opacity(t)),
      );
    let lines = muted_lines(&self.lines).text_sm().line_height(px(18.));
    let trailing = self.trailing.take();
    // The hovered box stays in place and only its content rises: a box that moved itself could
    // slip out from under the pointer, lose the hover, fall back and loop.
    self
      .frame()
      .w(px(width))
      .flex_none()
      .child(
        v_flex()
          .w_full()
          .relative()
          .top(px(-4. * t))
          .child(thumb)
          .child(
            h_flex()
              .mt_3()
              .gap_3()
              .items_start()
              .children(avatar)
              .child(
                v_flex()
                  .flex_1()
                  .min_w_0()
                  .gap_0p5()
                  .child(
                    div()
                      .line_height(px(20.))
                      .font_weight(FontWeight::SEMIBOLD)
                      .line_clamp(2)
                      .text_color(title_color)
                      .child(self.title.clone()),
                  )
                  .child(lines),
              )
              .children(trailing),
          ),
      )
      .into_any_element()
  }

  /// Poster card, `width` wide, its picture at the `aspect` ratio ([`TALL`] or [`WIDE`]), then
  /// the title and the first line, one line each, so that every poster of a row or a grid has
  /// the same height ([`poster_height`]). On hover it rises a little, a ring and the play button
  /// (if any) fade in, and the title takes the accent.
  pub fn poster(
    mut self,
    width: f32,
    aspect: f32,
    window: &mut Window,
    cx: &mut App,
  ) -> AnyElement {
    let t = self.hover.t;
    let title_color = self.title_color(false);
    let picture = self
      .picture_box(width, aspect, theme::RADIUS, Some(44.), window, cx)
      .when(t > 0.01, |el| el.shadow_lg())
      .child(
        div()
          .absolute()
          .inset_0()
          .rounded(theme::RADIUS)
          .border_2()
          .border_color(theme::accent().opacity(t)),
      );
    let title_id = SharedString::from(format!("{}-title", self.id));
    let title_middle_click = self.title_middle_click.take();
    let title = match self.title_click.take() {
      // A flex row, so the link is only as wide as its text.
      Some(on_click) => div().flex().child(
        div()
          .id(ElementId::Name(title_id))
          .min_w_0()
          .truncate()
          .text_color(theme::foreground())
          .hover(|s| s.text_color(theme::accent_hover()).underline())
          .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
          })
          .when_some(title_middle_click, |el, on_click| {
            el.on_aux_click(move |event, window, cx| {
              if event.is_middle_click() {
                cx.stop_propagation();
                on_click(window, cx);
              }
            })
          })
          .child(self.title.clone()),
      ),
      None => div()
        .truncate()
        .text_color(title_color)
        .child(self.title.clone()),
    };
    let line = self.lines.first().cloned().unwrap_or_default();
    self
      .frame()
      .w(px(width))
      .h(px(poster_height(width, aspect)))
      .flex_none()
      .child(
        v_flex()
          .w_full()
          .relative()
          .top(px(-4. * t))
          .child(picture)
          .child(
            div()
              .pt_2()
              .text_sm()
              .font_weight(FontWeight::MEDIUM)
              .child(title),
          )
          .child(
            div()
              .text_xs()
              .text_color(theme::muted())
              .truncate()
              .child(line),
          ),
      )
      .into_any_element()
  }

  /// Compact card (related videos): small picture on the left, text on the right, on a soft
  /// surface while hovered.
  pub fn compact(mut self, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = self.hover.t;
    let title_color = self.title_color(false);
    let thumb = self.thumbnail(COMPACT_WIDTH, theme::RADIUS, Some(36.), window, cx);
    let lines = muted_lines(&self.lines).text_xs().line_height(px(16.));
    self
      .frame()
      .w_full()
      .p_1()
      .rounded(theme::RADIUS_LG)
      .bg(theme::surface().opacity(t))
      .child(
        h_flex().gap_3().items_start().child(thumb).child(
          v_flex()
            .flex_1()
            .min_w_0()
            .pt_0p5()
            .gap_0p5()
            .child(
              div()
                .text_sm()
                .line_height(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .line_clamp(2)
                .text_color(title_color)
                .child(self.title.clone()),
            )
            .child(lines),
        ),
      )
      .into_any_element()
  }

  /// List row across the page: the picture on the left, then the title, the first line with
  /// the avatar and the other lines; on hover a soft surface (at once), a play button and the
  /// title in the accent.
  pub fn row(mut self, window: &mut Window, cx: &mut App) -> AnyElement {
    let hovered = self.hover.hovered;
    let title_color = self.title_color(false);
    let thumb = self.thumbnail(ROW_WIDTH, theme::RADIUS, Some(44.), window, cx);
    let avatar = self.avatar.take().map(|a| avatar(&a, 20., window, cx));
    let mut lines = self.lines.clone().into_iter();
    let first = lines.next().map(|line| {
      h_flex()
        .gap_2()
        .items_center()
        .children(avatar)
        .child(div().min_w_0().truncate().child(line))
    });
    let trailing = self.trailing.take();
    self
      .frame()
      .w_full()
      .p_2()
      .rounded(theme::RADIUS_LG)
      // The surface switches at once; the rest of the hover keeps its transition.
      .when(hovered, |el| el.bg(theme::surface()))
      .child(
        h_flex()
          .gap_4()
          .items_start()
          .child(thumb)
          .child(
            v_flex()
              .flex_1()
              .min_w_0()
              .pt_1()
              .gap_2()
              .text_sm()
              .text_color(theme::muted())
              .child(
                div()
                  .text_lg()
                  .line_height(px(24.))
                  .font_weight(FontWeight::SEMIBOLD)
                  .line_clamp(2)
                  .text_color(title_color)
                  .child(self.title.clone()),
              )
              .children(first)
              .children(lines.map(|line| div().truncate().child(line))),
          )
          .children(trailing),
      )
      .into_any_element()
  }

  /// A numbered row of a list (a playlist's page, the queue next to the player): the place,
  /// a picture `thumb_width` wide, the title and the lines on one line. `current` marks the
  /// item playing; `on_remove` adds an "x" shown on hover.
  pub fn numbered(
    mut self,
    number: usize,
    thumb_width: f32,
    current: bool,
    on_remove: Option<RowAction>,
    window: &mut Window,
    cx: &mut App,
  ) -> AnyElement {
    let t = self.hover.t;
    let small = thumb_width < 140.;
    let title_color = self.title_color(current);
    let thumb = self.thumbnail(
      thumb_width,
      theme::RADIUS,
      (!small).then_some(32.),
      window,
      cx,
    );
    // Small rows keep the first line only.
    let details = self
      .lines
      .iter()
      .take(if small { 1 } else { usize::MAX })
      .map(|l| l.to_string())
      .collect::<Vec<_>>()
      .join(" · ");
    let id = self.id.clone();
    self
      .frame()
      .w_full()
      .px_2()
      .py_1p5()
      .rounded(theme::RADIUS_LG)
      .when(current, |el| el.bg(theme::accent_soft()))
      .when(!current, |el| el.bg(theme::surface().opacity(t)))
      .child(
        h_flex()
          .gap_3()
          .items_center()
          .child(
            div()
              .w(px(if small { 20. } else { 28. }))
              .flex_none()
              .text_center()
              .text_xs()
              .text_color(if current {
                theme::accent()
              } else {
                theme::muted()
              })
              .map(|el| {
                if current {
                  el.child(
                    gpui_component::Icon::new(Icon::Play)
                      .size(px(12.))
                      .text_color(theme::accent()),
                  )
                } else {
                  el.child(number.to_string())
                }
              }),
          )
          .child(thumb)
          .child(
            v_flex()
              .flex_1()
              .min_w_0()
              .gap_0p5()
              .child(
                div()
                  .when(small, |el| el.text_sm().line_height(px(18.)))
                  .when(!small, |el| el.line_height(px(20.)))
                  .font_weight(FontWeight::SEMIBOLD)
                  .line_clamp(2)
                  .text_color(title_color)
                  .child(self.title.clone()),
              )
              .child(
                div()
                  .truncate()
                  .text_color(theme::muted())
                  .when(small, |el| el.text_xs())
                  .when(!small, |el| el.text_sm())
                  .child(details),
              ),
          )
          .when_some(on_remove, |row, remove| {
            row.child(remove_button(&id, t, remove))
          }),
      )
      .into_any_element()
  }
}

/// The lines under a title, one per line, truncated, in muted text.
fn muted_lines(lines: &[SharedString]) -> gpui::Div {
  v_flex()
    .text_color(theme::muted())
    .children(lines.iter().map(|l| div().truncate().child(l.clone())))
}

/// Dark badge at a picture's bottom right (duration).
fn badge(text: SharedString) -> impl IntoElement {
  div()
    .absolute()
    .bottom(px(8.))
    .right(px(8.))
    .px(px(6.))
    .py(px(1.))
    .rounded(px(4.))
    .bg(gpui::black().opacity(0.8))
    .text_xs()
    .font_weight(FontWeight::SEMIBOLD)
    .text_color(gpui::white())
    .child(text)
}

/// "LIVE" with a white dot on the accent at a picture's bottom right, like the player's.
fn live_badge() -> impl IntoElement {
  h_flex()
    .absolute()
    .bottom(px(8.))
    .right(px(8.))
    .gap_1()
    .items_center()
    .px(px(6.))
    .py(px(1.))
    .rounded(px(4.))
    .bg(theme::accent())
    .text_xs()
    .font_weight(FontWeight::BOLD)
    .text_color(gpui::white())
    .child(div().size(px(6.)).rounded_full().bg(gpui::white()))
    .child(crate::i18n::t("LIVE", "EN DIRECT"))
}

/// Height of a [`MediaCard::poster`] `width` wide with its picture at the `aspect` ratio.
pub fn poster_height(width: f32, aspect: f32) -> f32 {
  width / aspect + POSTER_TEXT
}

/// How far an item was watched (0 to 1): a rounded bar near a picture's bottom edge, under the
/// bottom right badge (which starts 8 px up).
pub fn progress_bar(progress: f32) -> impl IntoElement {
  const HEIGHT: f32 = 4.;
  let radius = px(HEIGHT / 2.);
  div()
    .absolute()
    .left(px(10.))
    .right(px(10.))
    .bottom(px(2.))
    .h(px(HEIGHT))
    .rounded(radius)
    .bg(gpui::black().opacity(0.55))
    .child(
      div()
        .h_full()
        .rounded(radius)
        .w(relative(progress.clamp(0., 1.)))
        // Never narrower than it is tall, so the rounded ends keep their shape.
        .min_w(px(HEIGHT))
        .bg(theme::accent()),
    )
}

/// A picture's top right sign: a check on a dark disc, or a count on the accent.
fn corner(corner: Corner) -> impl IntoElement {
  let sign = div()
    .absolute()
    .top(px(6.))
    .right(px(6.))
    .h(px(20.))
    .min_w(px(20.))
    .px_1()
    .flex()
    .items_center()
    .justify_center()
    .rounded_full()
    .text_xs()
    .font_weight(FontWeight::BOLD);
  match corner {
    Corner::Count(count) => sign
      .bg(theme::accent())
      .text_color(theme::on_accent())
      .child(count.to_string()),
    Corner::Watched => sign
      .bg(gpui::black().opacity(0.6))
      .text_color(theme::accent())
      .child(gpui_component::Icon::new(Icon::Check).size(px(12.))),
  }
}

/// Dimmed layer with a round play button over a picture, faded in by `t`.
pub fn play_overlay(t: f32, size: f32, radius: Pixels) -> impl IntoElement {
  div()
    .absolute()
    .inset_0()
    .flex()
    .items_center()
    .justify_center()
    .opacity(t)
    .rounded(radius)
    .bg(gpui::black().opacity(0.3))
    .child(
      div()
        .size(px(size))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::accent())
        .text_color(theme::on_accent())
        .shadow_lg()
        .child(gpui_component::Icon::new(Icon::Play).size(px(size * 0.45))),
    )
}

/// The "x" of a row, faded in by `t` (the row's hover); a spinner while it runs. Its clicks are
/// its own, not the row's.
fn remove_button(id: &str, t: f32, remove: RowAction) -> impl IntoElement {
  let RowAction { run, label, busy } = remove;
  div()
    .id(ElementId::Name(format!("{id}-remove").into()))
    .size(px(28.))
    .flex_none()
    .rounded_full()
    .flex()
    .items_center()
    .justify_center()
    // Kept in view while busy, even once the mouse has left the row.
    .opacity(if busy { 1. } else { t })
    .text_color(theme::muted())
    .when(!busy, |el| {
      el.hover(|s| s.bg(theme::surface_hover()).text_color(theme::foreground()))
        .cursor_pointer()
    })
    // The row opens its item on click: keep this click from reaching it.
    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .on_click(move |_, window, cx| {
      cx.stop_propagation();
      run(window, cx);
    })
    .tooltip(move |window, cx| {
      gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
    })
    .child(if busy {
      loader(16., theme::foreground()).into_any_element()
    } else {
      gpui_component::Icon::new(gpui_component::IconName::Close)
        .size(px(14.))
        .into_any_element()
    })
}

/// Placeholder shaped like a [`MediaCard::grid`] card.
pub fn skeleton_card(width: f32) -> impl IntoElement {
  v_flex()
    .w(px(width))
    .flex_none()
    .child(
      Skeleton::new()
        .w_full()
        .h(px(width / WIDE))
        .rounded(theme::RADIUS_LG),
    )
    .child(
      h_flex()
        .mt_3()
        .gap_3()
        .items_start()
        .child(Skeleton::new().size(px(AVATAR)).flex_none().rounded_full())
        .child(
          v_flex()
            .flex_1()
            .gap_2()
            .child(Skeleton::new().w_full().h(px(16.)))
            .child(Skeleton::new().w(px(width * 0.6)).h(px(14.)))
            .child(Skeleton::new().w(px(width * 0.4)).h(px(14.))),
        ),
    )
}

/// Placeholder shaped like a [`MediaCard::poster`]'s picture.
pub fn skeleton_poster(width: f32, aspect: f32) -> impl IntoElement {
  Skeleton::new()
    .w(px(width))
    .h(px(width / aspect))
    .flex_none()
    .rounded(theme::RADIUS)
}

/// Placeholder shaped like a [`MediaCard::compact`] card.
pub fn skeleton_compact() -> impl IntoElement {
  h_flex()
    .p_1()
    .gap_3()
    .items_start()
    .child(
      Skeleton::new()
        .w(px(COMPACT_WIDTH))
        .h(px(COMPACT_WIDTH / WIDE))
        .flex_none()
        .rounded(theme::RADIUS),
    )
    .child(
      v_flex()
        .flex_1()
        .pt_0p5()
        .gap_2()
        .child(Skeleton::new().w_full().h(px(14.)))
        .child(Skeleton::new().w(px(120.)).h(px(12.)))
        .child(Skeleton::new().w(px(90.)).h(px(12.))),
    )
}

/// Placeholder shaped like a [`MediaCard::row`].
pub fn skeleton_row() -> impl IntoElement {
  h_flex()
    .p_2()
    .gap_4()
    .items_start()
    .child(
      Skeleton::new()
        .w(px(ROW_WIDTH))
        .h(px(ROW_WIDTH / WIDE))
        .flex_none()
        .rounded(theme::RADIUS),
    )
    .child(
      v_flex()
        .flex_1()
        .pt_1()
        .gap_3()
        .child(Skeleton::new().w(relative(0.6)).h(px(20.)))
        .child(Skeleton::new().w(px(160.)).h(px(14.)))
        .child(Skeleton::new().w(px(200.)).h(px(14.))),
    )
}
