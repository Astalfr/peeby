//! Media cards: a picture, a title and muted lines under it, in the layouts the pages use (grid
//! card, poster, compact card, list row, numbered row), their skeletons, and the buttons they
//! carry.
//! Spaces fill a [`MediaCard`] from their own data. The look and the motion follow
//! docs/design.md.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
  AnyElement, App, ElementId, Entity, FontWeight, Hsla, MouseButton, Pixels, SharedString, Window,
  div, px, relative,
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
/// Narrowest picture whose bottom left badge shows its text after its icon (a duration
/// leaves room for it from there).
pub const BADGE_LABEL_WIDTH: f32 = 200.;
/// Width of a list row's thumbnail.
pub const ROW_WIDTH: f32 = 246.;
/// How far a stacked picture's layers rise above it ([`MediaCard::stacked`]).
pub const STACK: f32 = 8.;
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

/// A frame of the video under the mouse as it moves across a card's picture: an image of its
/// own, or a tile of a sheet of them (a storyboard).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScrubFrame {
  /// The image, or the sheet the tile is on.
  pub url: String,
  /// The tile on the sheet; `None`: the whole image.
  pub tile: Option<ScrubTile>,
}

/// Where a frame sits on its sheet, in the sheet's pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrubTile {
  /// Column of the tile, from 0.
  pub column: u32,
  /// Row of the tile, from 0.
  pub row: u32,
  /// Width of a tile.
  pub width: u32,
  /// Height of a tile.
  pub height: u32,
}

/// How long the mouse rests on a card before its picture follows it ([`MediaCard::scrub`]):
/// sweeping over a grid shows nothing.
const SCRUB_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
/// The scrub line's inset from the picture's edges, clear of the rounded corners and of the
/// hover ring.
const SCRUB_LINE_INSET: f32 = 8.;

/// The frame at a point across the picture (0 at its left edge, 1 at its right), given by the
/// space ([`MediaCard::scrub`]).
pub type Scrub = Rc<dyn Fn(f32, &App) -> Option<ScrubFrame>>;

/// The hover state of a card: whether the mouse is on it, and the hover animation (0 to 1).
/// Read it before building the card when its buttons fade in with the hover.
#[derive(Clone)]
pub struct Hover {
  /// The mouse is on the card.
  pub hovered: bool,
  /// The hover animation, 0 (out) to 1 (in).
  pub t: f32,
  /// The press: 1 at once while the left button is down on the card, back to 0 in
  /// [`theme::FASTER`] once released.
  pub pressed: f32,
  /// Where the mouse is across the picture, 0 to 1, while it moves on it, once the card has
  /// been hovered [`SCRUB_DELAY`].
  pub scrub: Option<f32>,
  state: Entity<bool>,
  down: Entity<bool>,
  scrub_at: Entity<Option<f32>>,
  /// When the mouse came on the card.
  since: Entity<Option<std::time::Instant>>,
  /// The picture's bounds, as last painted.
  picture: Rc<Cell<gpui::Bounds<Pixels>>>,
}

impl Hover {
  /// The hover state of the card `id`. Ids must be unique on the page: two elements with one id
  /// share their state, and their hover flags overwrite each other on every frame.
  pub fn new(id: &str, window: &mut Window, cx: &mut App) -> Self {
    let state: Entity<bool> =
      motion::hover_state(SharedString::from(format!("{id}-hover")), window, cx);
    let down: Entity<bool> =
      motion::hover_state(SharedString::from(format!("{id}-down")), window, cx);
    let scrub_at = window.use_keyed_state(SharedString::from(format!("{id}-scrub")), cx, |_, _| {
      None::<f32>
    });
    let picture = window
      .use_keyed_state(SharedString::from(format!("{id}-picture")), cx, |_, _| {
        Rc::new(Cell::new(gpui::Bounds::<Pixels>::default()))
      })
      .read(cx)
      .clone();
    let since = window.use_keyed_state(SharedString::from(format!("{id}-since")), cx, |_, _| {
      None::<std::time::Instant>
    });
    let hovered = *state.read(cx);
    let rested = (*since.read(cx)).is_some_and(|at| at.elapsed() >= SCRUB_DELAY);
    let scrub = (*scrub_at.read(cx)).filter(|_| hovered && rested);
    let is_down = *down.read(cx) && hovered;
    // A spring: the mouse passing over a card and leaving it halfway turns the rise back
    // without stopping it dead (ADR 0041). Kept within 0 and 1 (opacities follow it).
    let t = motion::spring(
      SharedString::from(format!("{id}-lift")),
      if hovered { 1. } else { 0. },
      theme::FAST,
      motion::CRITICAL,
      window,
      cx,
    )
    .clamp(0., 1.);
    let pressed = motion::press(
      SharedString::from(format!("{id}-press")),
      is_down,
      window,
      cx,
    );
    Self {
      hovered,
      t,
      pressed,
      scrub,
      state,
      down,
      scrub_at,
      since,
      picture,
    }
  }

  /// Follows the mouse on `frame`, the card's clickable box: its hover (and `on_hover`, as
  /// GPUI takes a single hover listener per element) and the left button's press.
  pub fn track(
    &self,
    frame: gpui::Stateful<gpui::Div>,
    on_hover: Option<Box<HoverHandler>>,
  ) -> gpui::Stateful<gpui::Div> {
    let (state, down, scrub_at, since) = (
      self.state.clone(),
      self.down.clone(),
      self.scrub_at.clone(),
      self.since.clone(),
    );
    let set_down = |down: &Entity<bool>, to: bool| {
      let down = down.clone();
      move |window: &mut Window, cx: &mut App| {
        if *down.read(cx) != to {
          down.update(cx, |d, _| *d = to);
          window.refresh();
        }
      }
    };
    let (press, release, release_out) = (
      set_down(&down, true),
      set_down(&down, false),
      set_down(&down, false),
    );
    frame
      .on_hover(move |hover, window, cx| {
        if let Some(on_hover) = &on_hover {
          on_hover(*hover, cx);
        }
        state.update(cx, |h, _| *h = *hover);
        if *hover {
          since.update(cx, |since, _| *since = Some(std::time::Instant::now()));
          // Drawn again once the delay is over, if the mouse rests there.
          window
            .spawn(cx, async move |cx| {
              cx.background_executor().timer(SCRUB_DELAY).await;
              cx.update(|window, _| window.refresh()).ok();
            })
            .detach();
        } else {
          scrub_at.update(cx, |at, _| *at = None);
          since.update(cx, |since, _| *since = None);
        }
        window.refresh();
      })
      .on_mouse_down(MouseButton::Left, move |_, window, cx| press(window, cx))
      .on_mouse_up(MouseButton::Left, move |_, window, cx| release(window, cx))
      .on_mouse_up_out(MouseButton::Left, move |_, window, cx| {
        release_out(window, cx)
      })
  }

  /// How high the card's content rises: 4 px hovered, 2 px while pressed.
  pub fn lift(&self) -> Pixels {
    px(-4. * self.t + 2. * self.pressed)
  }

  /// The hover ring's opacity: full hovered, dimmed while pressed.
  pub fn ring(&self) -> f32 {
    self.t * (1. - 0.4 * self.pressed)
  }
}

/// A hovered surface, one step darker while pressed: `hover` and `press` from [`Hover`].
fn surface_pressed(base: Hsla, hover: f32, press: f32) -> Hsla {
  base
    .opacity(hover)
    .blend(theme::surface_pressed().opacity(press))
}

/// A click's launch is taken by a page built within this long of it, else it is stale (a page
/// opened later, by another way, does not fly from an old click).
const LAUNCH_FRESH: std::time::Duration = std::time::Duration::from_millis(500);

/// Where a clicked card's picture was (docs/responsiveness-plan.md, item 15): the page it opens
/// grows its poster from there (`PlayerView::fly_poster_from`), the card's picture, which it
/// shows until the first frame unless the page has a poster of its own.
#[derive(Clone)]
pub struct Launch {
  /// The picture's bounds in its window, when clicked.
  pub bounds: gpui::Bounds<Pixels>,
  /// Its corner radius.
  pub radius: Pixels,
  /// The picture itself.
  pub picture: Picture,
  window: gpui::WindowId,
  at: std::time::Instant,
}

thread_local! {
  static LAUNCH: Cell<Option<Launch>> = const { Cell::new(None) };
}

fn launch(bounds: gpui::Bounds<Pixels>, radius: Pixels, picture: &Picture, window: &Window) {
  if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) || picture.is_empty() {
    return;
  }
  LAUNCH.set(Some(Launch {
    bounds,
    radius,
    picture: picture.clone(),
    window: window.window_handle().window_id(),
    at: std::time::Instant::now(),
  }));
}

/// The launch of the card just clicked in `window`, once; none past [`LAUNCH_FRESH`] or from
/// another window.
pub fn take_launch(window: &Window) -> Option<Launch> {
  LAUNCH
    .take()
    .filter(|l| l.window == window.window_handle().window_id() && l.at.elapsed() < LAUNCH_FRESH)
}

/// Darkens a card's picture while it is pressed (`press` from [`Hover`]): laid over the
/// picture, under its badges and buttons.
pub fn press_shade(radius: Pixels, press: f32) -> Option<gpui::Div> {
  (press > 0.).then(|| {
    div()
      .absolute()
      .inset_0()
      .rounded(radius)
      .bg(gpui::black().opacity(0.2 * press))
  })
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
  badge_icon: Option<Icon>,
  badge_left: Option<SharedString>,
  badge_left_icon: Option<Icon>,
  badge_dot: Option<Hsla>,
  live: bool,
  progress: Option<f32>,
  corner: Option<Corner>,
  play: bool,
  stacked: bool,
  title_click: Option<Rc<Handler>>,
  title_middle_click: Option<Rc<Handler>>,
  line_click: Option<Rc<Handler>>,
  line_middle_click: Option<Rc<Handler>>,
  overlays: Vec<AnyElement>,
  trailing: Option<AnyElement>,
  on_click: Option<Rc<Handler>>,
  on_middle_click: Option<Rc<Handler>>,
  on_hover: Option<Box<HoverHandler>>,
  scrub: Option<Scrub>,
  /// The picture's corner radius, as last drawn, for a click's launch.
  picture_radius: Rc<Cell<Pixels>>,
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
      badge_icon: None,
      badge_left: None,
      badge_left_icon: None,
      badge_dot: None,
      live: false,
      progress: None,
      corner: None,
      play: true,
      stacked: false,
      title_click: None,
      title_middle_click: None,
      line_click: None,
      line_middle_click: None,
      overlays: Vec::new(),
      trailing: None,
      on_click: None,
      on_middle_click: None,
      on_hover: None,
      scrub: None,
      picture_radius: Rc::default(),
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

  /// An icon before the bottom right badge's text (a playlist's count).
  pub fn badge_icon(mut self, icon: Option<Icon>) -> Self {
    self.badge_icon = icon;
    self
  }

  /// Whether the picture stands on a stack, two thin layers peeking out above it (a
  /// playlist). The picture then has a [`STACK`] margin above it, where the layers show.
  pub fn stacked(mut self, stacked: bool) -> Self {
    self.stacked = stacked;
    self
  }

  /// A dot of this color before the bottom right badge's text (a live's viewers).
  pub fn badge_dot(mut self, color: Option<Hsla>) -> Self {
    self.badge_dot = color;
    self
  }

  /// Text in a dark badge at the picture's bottom left (a live's uptime).
  pub fn badge_left(mut self, badge: Option<impl Into<SharedString>>) -> Self {
    self.badge_left = badge.map(Into::into);
    self
  }

  /// An icon before the bottom left badge's text. On a picture narrower than
  /// [`BADGE_LABEL_WIDTH`], the icon shows alone, so that the badge leaves room for the
  /// bottom right one.
  pub fn badge_left_icon(mut self, icon: Option<Icon>) -> Self {
    self.badge_left_icon = icon;
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

  /// Makes the first line a link of its own (a video's channel), highlighted while hovered.
  pub fn on_line_click(mut self, on_click: impl Fn(&mut Window, &mut App) + 'static) -> Self {
    self.line_click = Some(Rc::new(on_click));
    self
  }

  /// What a middle click on the first line's link does (opens it in a new tab).
  pub fn on_line_middle_click(
    mut self,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
  ) -> Self {
    self.line_middle_click = Some(Rc::new(on_click));
    self
  }

  /// The lines, the first a link when the card has one ([`Self::on_line_click`]).
  fn line_elements(&mut self) -> Vec<AnyElement> {
    let (click, middle) = (self.line_click.take(), self.line_middle_click.take());
    let id = SharedString::from(format!("{}-line", self.id));
    self
      .lines
      .iter()
      .enumerate()
      .map(|(i, line)| match (&click, i) {
        (Some(click), 0) => link(id.clone(), line.clone(), click.clone(), middle.clone()),
        _ => div().truncate().child(line.clone()).into_any_element(),
      })
      .collect()
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

  /// Frames of the video shown as the mouse moves across the picture, with a thin line of
  /// where it is (docs/design.md, "Card hover").
  pub fn scrub(mut self, scrub: Option<Scrub>) -> Self {
    self.scrub = scrub;
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
    let on_hover = self.on_hover.take();
    let on_click = self.on_click.take();
    let on_middle_click = self.on_middle_click.take();
    let frame = div().id(ElementId::Name(self.id.clone())).cursor_pointer();
    let (bounds, radius) = (self.hover.picture.clone(), self.picture_radius.clone());
    let picture = self.picture.clone();
    self
      .hover
      .track(frame, on_hover)
      .when_some(on_click, |el, on_click| {
        el.on_click(move |_, window, cx| {
          // Where the picture was, for the page it opens to grow it from ([`take_launch`]).
          launch(bounds.get(), radius.get(), &picture, window);
          on_click(window, cx)
        })
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
    // The frames under the mouse, while it moves on the picture (the play button gives way).
    let scrub = self.scrub.clone().filter(|_| self.hover.hovered);
    let frame = scrub
      .as_ref()
      .zip(self.hover.scrub)
      .and_then(|(scrub, at)| scrub(at, cx))
      .and_then(|frame| scrub_frame(&frame, width, aspect, radius, window, cx));
    if let Some(scrub) = &scrub {
      // The frames are loaded as soon as the card is hovered, so moving shows them at once.
      for at in [0.1, 0.5, 0.9] {
        if let Some(frame) = scrub(at, cx) {
          scrub_frame(&frame, width, aspect, radius, window, cx);
        }
      }
    }
    let scrubbing = frame.is_some();
    let play = play.filter(|_| self.play && !scrubbing);
    self.picture_radius.set(radius);
    // A stack's layers: narrower, peeking out above the picture, as on the playlists page.
    // They hang out of the box (a margin makes room for them), so that what the layouts put on
    // the box (a hover ring, a shadow) stays on the picture. Painted first, the picture covers
    // their lower part.
    let inset = (width * 0.05).round().max(8.);
    let layer = |inset: f32, top: f32, opacity: f32| {
      div()
        .absolute()
        .top(px(top))
        .left(px(inset))
        .right(px(inset))
        .h(px(12.))
        .rounded_t(theme::RADIUS)
        .bg(theme::muted().opacity(opacity))
    };
    let layers = self.stacked.then(|| {
      [
        layer(inset, -STACK, 0.25),
        layer(inset / 2., -STACK / 2., 0.45),
      ]
    });
    div()
      .relative()
      .w(px(width))
      .h(px(width / aspect))
      .flex_none()
      .when(self.stacked, |el| el.mt(px(STACK)))
      .rounded(radius)
      .children(layers.into_iter().flatten())
      .child(image_fill(&self.picture, width, aspect, radius, window, cx))
      .children(frame)
      .when_some(self.hover.scrub.filter(|_| scrubbing), |el, at| {
        // A thin rounded track inset in the picture, white on a faint white.
        let track = (width - 2. * SCRUB_LINE_INSET).max(0.);
        el.child(
          div()
            .absolute()
            .left(px(SCRUB_LINE_INSET))
            .right(px(SCRUB_LINE_INSET))
            .bottom(px(SCRUB_LINE_INSET))
            .h(px(3.))
            .rounded_full()
            .bg(gpui::white().opacity(0.35))
            .child(
              div()
                .h_full()
                .w(px(track * at))
                .rounded_full()
                .bg(gpui::white()),
            ),
        )
      })
      // The picture's bounds, for the scrub and for a click's launch ([`take_launch`]).
      .child({
        let painted = self.hover.picture.clone();
        gpui::canvas(move |b, _, _| painted.set(b), |_, _, _, _| {})
          .absolute()
          .inset_0()
      })
      .when(scrub.is_some(), |el| {
        let (bounds, at) = (self.hover.picture.clone(), self.hover.scrub_at.clone());
        el.on_mouse_move(move |event, window, cx| {
          let b = bounds.get();
          let width = f32::from(b.size.width);
          // Off the picture (onto the title): the picture comes back.
          if width <= 0. || !b.contains(&event.position) {
            if at.read(cx).is_some() {
              at.update(cx, |at, _| *at = None);
              window.refresh();
            }
            return;
          }
          let x = (f32::from(event.position.x - b.origin.x) / width).clamp(0., 1.);
          at.update(cx, |at, _| *at = Some(x));
          window.refresh();
        })
      })
      .children(press_shade(radius, self.hover.pressed))
      .children(play.map(|size| play_overlay(t, size, radius)))
      .children(self.progress.map(progress_bar))
      .children(self.corner.map(corner))
      .when(self.live, |el| el.child(live_badge()))
      .children(
        self
          .badge
          .take()
          .filter(|_| !self.live)
          .map(|t| badge(Some(t), self.badge_dot, self.badge_icon).right(px(8.))),
      )
      .children({
        let icon = self.badge_left_icon.take();
        let text = self
          .badge_left
          .take()
          .filter(|_| icon.is_none() || width >= BADGE_LABEL_WIDTH);
        (icon.is_some() || text.is_some()).then(|| badge(text, None, icon).left(px(8.)))
      })
      .children(self.overlays.drain(..))
  }

  /// Grid card, `width` wide: the 16:9 picture, then the avatar, title and lines. On hover it
  /// rises a little, a ring and a play button fade in, and the title takes the accent.
  pub fn grid(mut self, width: f32, window: &mut Window, cx: &mut App) -> AnyElement {
    let t = self.hover.t;
    let ring = self.hover.ring();
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
          .border_color(theme::accent().opacity(ring)),
      );
    let lines = v_flex()
      .text_color(theme::muted())
      .children(self.line_elements())
      .text_sm()
      .line_height(px(18.));
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
          .top(self.hover.lift())
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
    let ring = self.hover.ring();
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
          .border_color(theme::accent().opacity(ring)),
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
          .top(self.hover.lift())
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
    let lines = v_flex()
      .text_color(theme::muted())
      .children(self.line_elements())
      .text_xs()
      .line_height(px(16.));
    self
      .frame()
      .w_full()
      .p_1()
      .rounded(theme::RADIUS_LG)
      .bg(surface_pressed(theme::surface(), t, self.hover.pressed))
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
    let mut lines = self.line_elements().into_iter();
    let first = lines.next().map(|line| {
      h_flex()
        .gap_2()
        .items_center()
        .children(avatar)
        .child(div().min_w_0().flex().child(line))
    });
    let trailing = self.trailing.take();
    self
      .frame()
      .w_full()
      .p_2()
      .rounded(theme::RADIUS_LG)
      // The surface switches at once; the rest of the hover keeps its transition.
      .when(hovered, |el| {
        el.bg(surface_pressed(theme::surface(), 1., self.hover.pressed))
      })
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
              .children(lines),
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
      .when(!current, |el| {
        el.bg(surface_pressed(theme::surface(), t, self.hover.pressed))
      })
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
/// A line of a card as a link of its own, only as wide as its text, highlighted while hovered;
/// its clicks stay off the card.
fn link(
  id: SharedString,
  text: SharedString,
  on_click: Rc<Handler>,
  on_middle_click: Option<Rc<Handler>>,
) -> AnyElement {
  // A flex row, so the link is only as wide as its text.
  div()
    .flex()
    .min_w_0()
    .child(
      div()
        .id(ElementId::Name(id))
        .min_w_0()
        .truncate()
        .cursor_pointer()
        .hover(|s| s.text_color(theme::foreground()))
        .on_click(move |_, window, cx| {
          cx.stop_propagation();
          on_click(window, cx);
        })
        .when_some(on_middle_click, |el, on_click| {
          el.on_aux_click(move |event, window, cx| {
            if event.is_middle_click() {
              cx.stop_propagation();
              on_click(window, cx);
            }
          })
        })
        .child(text),
    )
    .into_any_element()
}

/// Dark badge at a picture's bottom (duration), after a `dot` of that color or an `icon` if
/// any, placed on its side by the caller.
fn badge(text: Option<SharedString>, dot: Option<Hsla>, icon: Option<Icon>) -> gpui::Div {
  h_flex()
    .gap_1()
    .items_center()
    .absolute()
    .bottom(px(8.))
    .px(px(6.))
    .py(px(1.))
    .rounded(px(4.))
    .bg(gpui::black().opacity(0.8))
    .text_xs()
    .font_weight(FontWeight::SEMIBOLD)
    .text_color(gpui::white())
    .children(dot.map(|color| div().size(px(6.)).rounded_full().bg(color)))
    .children(icon.map(|icon| gpui_component::Icon::new(icon).size(px(12.))))
    .children(text)
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
        .active(|s| s.bg(theme::surface_pressed()))
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

/// A scrubbed frame over a picture `width` wide at `aspect`, once loaded (nothing until then:
/// the picture shows).
fn scrub_frame(
  frame: &ScrubFrame,
  width: f32,
  aspect: f32,
  radius: Pixels,
  window: &mut Window,
  cx: &mut App,
) -> Option<AnyElement> {
  use gpui::{ImageSource, ObjectFit, img};
  let images = cx.try_global::<crate::images::Images>()?.clone();
  match frame.tile {
    None => {
      let scale = window.scale_factor();
      let (w, h) = (
        (width * scale).round() as u32,
        (width / aspect * scale).round() as u32,
      );
      let image = images.0.update(cx, |store, cx| {
        store.get_fit(
          &frame.url,
          crate::images::Effects::default(),
          crate::images::Fit::Cover,
          w,
          h,
          cx,
        )
      })?;
      Some(
        img(ImageSource::Render(image))
          .absolute()
          .inset_0()
          .size_full()
          .rounded(radius)
          .object_fit(ObjectFit::Cover)
          .into_any_element(),
      )
    }
    Some(tile) => {
      let sheet = images.0.update(cx, |store, cx| store.get(&frame.url, cx))?;
      // Sized from the sheet itself: the last one is cut after its last tile.
      let scale = width / tile.width.max(1) as f32;
      let size = sheet.size(0);
      Some(
        div()
          .absolute()
          .inset_0()
          .rounded(radius)
          .overflow_hidden()
          .child(
            img(ImageSource::Render(sheet))
              .absolute()
              .left(px(-(tile.column as f32) * width))
              .top(px(-(tile.row as f32) * tile.height as f32 * scale))
              .w(px(size.width.0 as f32 * scale))
              .h(px(size.height.0 as f32 * scale))
              .object_fit(ObjectFit::Fill),
          )
          .into_any_element(),
      )
    }
  }
}
