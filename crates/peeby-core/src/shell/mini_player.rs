//! The mini-player: a window of its own, above every other window and outside the main one, as
//! in Chrome's picture-in-picture (ADR 0008). It shows the video of the tab that was left
//! while it played, without restarting it: the same player and video surface move here. On
//! hover: buttons to go back to the page and to close (which pauses), and play/pause in the
//! middle. Dragging the video moves the window; its edges (12 px bands) and corners (24 px
//! squares) resize it at 16:9, the opposite side staying in place.

use gpui::prelude::*;
use gpui::{
  AnyWindowHandle, App, Bounds, Context, Corners, CursorStyle, DispatchPhase, Entity, EntityId,
  MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, Subscription,
  WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind,
  WindowOptions, canvas, div, linear_color_stop, linear_gradient, point, px, size,
};
use gpui_component::IconName;
use ui::assets::Icon;
use ui::i18n::t;
use ui::player::PlayerView;

use super::Shell;
use super::space::{SpaceHandle, TabId};

/// Width of a new mini-player, logical pixels.
const WIDTH: f32 = 400.;
/// Smallest width the edges resize to.
const MIN_WIDTH: f32 = 240.;
/// Gap between the window and the screen's edges.
const MARGIN: f32 = 24.;
/// Thickness of the resizing edges, and size of the corners, logical pixels (GPUI scales them
/// by the display's factor). Wide enough to grab without aiming: the window has no frame of its
/// own, so these bands reach inward from its sides.
const EDGE: f32 = 12.;
const CORNER: f32 = 24.;
/// Height of the shade at the top, which the control row sits in.
const SHADE: f32 = 48.;
/// Side of the "Back to the page" and "Close" buttons, and their inset from the top right. The
/// inset keeps them clear of the edges and leaves the top right corner an L that thick.
const CONTROL_SIZE: f32 = 28.;
const CONTROLS_INSET: f32 = 14.;
/// Side of the play/pause button, centred below the shade.
const PLAY_SIZE: f32 = 48.;
/// Height of the smallest window the edges resize to, 16:9.
const MIN_HEIGHT: f32 = MIN_WIDTH * 9. / 16.;

// What the layout must hold, checked when the crate compiles: the smallest window
// (`MIN_WIDTH` x `MIN_HEIGHT`) is the worst case, where the bands cost proportionally the most,
// so bumping one of the constants above past what it holds fails the build.
const _: () = {
  // A corner must be easier to hit than an edge.
  assert!(EDGE < CORNER);
  // The top and bottom strips span `CORNER..width - CORNER`, the left and right ones
  // `CORNER..height - CORNER`: a length of zero or less would invert the strip.
  assert!(MIN_WIDTH - 2. * CORNER > 0.);
  assert!(MIN_HEIGHT - 2. * CORNER > 0.);
  // Anything the bands leave moves the window when dragged: over half of the smallest one must
  // still do, or it could no longer be moved comfortably.
  assert!((MIN_WIDTH - 2. * EDGE) * (MIN_HEIGHT - 2. * EDGE) > 0.5 * MIN_WIDTH * MIN_HEIGHT);
  // The play/pause button, centred in the box below the shade as `render` places it, must clear
  // all four bands even at that size.
  let play_left = (MIN_WIDTH - PLAY_SIZE) / 2.;
  let play_top = SHADE + (MIN_HEIGHT - SHADE - PLAY_SIZE) / 2.;
  assert!(play_left > EDGE && play_left + PLAY_SIZE < MIN_WIDTH - EDGE);
  assert!(play_top > EDGE && play_top + PLAY_SIZE < MIN_HEIGHT - EDGE);
  // The control row takes its own clicks where it covers a band (it is painted after them),
  // which leaves the top right corner an L `CONTROLS_INSET` thick: that L must stay at least as
  // wide as a plain edge.
  assert!(CONTROLS_INSET >= EDGE);
  // And the row must stay inside the gradient it sits on.
  assert!(CONTROLS_INSET + CONTROL_SIZE <= SHADE);
};

/// The resizing edges and corners: id, edge (see [`Gesture::edge`]) and cursor.
const EDGES: [(&str, (i8, i8), CursorStyle); 8] = [
  ("mini-edge-left", (-1, 0), CursorStyle::ResizeLeftRight),
  ("mini-edge-right", (1, 0), CursorStyle::ResizeLeftRight),
  ("mini-edge-top", (0, -1), CursorStyle::ResizeUpDown),
  ("mini-edge-bottom", (0, 1), CursorStyle::ResizeUpDown),
  (
    "mini-corner-tl",
    (-1, -1),
    CursorStyle::ResizeUpLeftDownRight,
  ),
  ("mini-corner-br", (1, 1), CursorStyle::ResizeUpLeftDownRight),
  (
    "mini-corner-tr",
    (1, -1),
    CursorStyle::ResizeUpRightDownLeft,
  ),
  (
    "mini-corner-bl",
    (-1, 1),
    CursorStyle::ResizeUpRightDownLeft,
  ),
];

/// A move or a resize in progress: where the mouse went down (screen coordinates: the window
/// moves under it), the window's bounds then, and the edge held, as the direction each side
/// goes: `(-1, 0)` the left edge, `(1, 1)` the bottom right corner, `(0, 0)` a move. Points and
/// bounds are in device pixels (held in `Pixels`): the scale changes when the window goes to a
/// screen with another one, and logical points from before and after would not match.
#[derive(Clone, Copy)]
struct Gesture {
  from: Point<Pixels>,
  start: Bounds<Pixels>,
  edge: (i8, i8),
}

impl Gesture {
  /// The window's bounds with the mouse at `to`, from `min_width` to `max_width` wide.
  fn bounds(&self, to: Point<Pixels>, min_width: f32, max_width: f32) -> Bounds<Pixels> {
    let (dx, dy) = (f32::from(to.x - self.from.x), f32::from(to.y - self.from.y));
    let start = self.start;
    if self.edge == (0, 0) {
      return Bounds::new(start.origin + point(px(dx), px(dy)), start.size);
    }
    let (sx, sy) = (f32::from(self.edge.0), f32::from(self.edge.1));
    let from_x = sx * dx;
    let from_y = sy * dy * 16. / 9.;
    // A corner follows the axis the mouse went further along.
    let grow = if from_x.abs() >= from_y.abs() {
      from_x
    } else {
      from_y
    };
    let width = (f32::from(start.size.width) + grow).clamp(min_width, max_width.max(min_width));
    let new_size = size(px(width), px(width * 9. / 16.));
    // The side opposite the edge stays; along an axis no edge holds, the middle does.
    let place = |edge: i8, low: Pixels, high: Pixels, len: Pixels| match edge {
      -1 => high - len,
      1 => low,
      _ => (low + high - len) / 2.,
    };
    let origin = point(
      place(self.edge.0, start.left(), start.right(), new_size.width),
      place(self.edge.1, start.top(), start.bottom(), new_size.height),
    );
    Bounds::new(origin, new_size)
  }
}

/// The mini-player's view.
pub struct MiniPlayer {
  space: SpaceHandle,
  /// The tab of `space` whose video shows.
  tab: TabId,
  shell: WeakEntity<Shell>,
  /// The main window, where the shell lives.
  main: AnyWindowHandle,
  gesture: Option<Gesture>,
  /// Redraws the buttons when the player shown pauses or plays.
  observed: Option<(EntityId, Subscription)>,
}

/// Opens the mini-player on the screen of `window` (the main one), at its bottom right, on the
/// video of `tab`.
pub fn open(
  space: SpaceHandle,
  tab: TabId,
  shell: WeakEntity<Shell>,
  main: AnyWindowHandle,
  window: &Window,
  cx: &mut App,
) -> anyhow::Result<WindowHandle<MiniPlayer>> {
  let display = window.display(cx);
  let window_size = size(px(WIDTH), px(WIDTH * 9. / 16.));
  let bounds = match &display {
    Some(display) => {
      let screen = display.visible_bounds();
      let origin = point(
        screen.origin.x + screen.size.width - window_size.width - px(MARGIN),
        screen.origin.y + screen.size.height - window_size.height - px(MARGIN),
      );
      Bounds::new(origin, window_size)
    }
    None => Bounds::new(point(px(0.), px(0.)), window_size),
  };
  let options = WindowOptions {
    window_bounds: Some(WindowBounds::Windowed(bounds)),
    // Windows checks the bounds against this display, else the primary one, and falls back to
    // a default place and size when they are outside it.
    display_id: display.map(|d| d.id()),
    kind: WindowKind::PopUp,
    titlebar: None,
    focus: false,
    // Never focused, so GPUI's 30 fps cap for unfocused windows would always apply.
    inactive_frame_interval: None,
    is_resizable: false,
    window_background: WindowBackgroundAppearance::Opaque,
    ..Default::default()
  };
  cx.open_window(options, |_, cx| {
    cx.new(|_| MiniPlayer {
      space,
      tab,
      shell,
      main,
      gesture: None,
      observed: None,
    })
  })
}

impl MiniPlayer {
  fn player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    self.space.player(self.tab, cx)
  }

  /// Runs `f` on the shell, in the main window.
  fn with_shell(
    &self,
    cx: &mut App,
    f: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
  ) {
    let shell = self.shell.clone();
    self
      .main
      .update(cx, |_, window, cx| {
        shell.update(cx, |shell, cx| f(shell, window, cx)).ok();
      })
      .ok();
  }

  /// Starts moving (`(0, 0)`) or resizing from `edge` with the mouse down at `event`.
  fn begin(
    &mut self,
    edge: (i8, i8),
    event: &MouseDownEvent,
    window: &Window,
    cx: &mut Context<Self>,
  ) {
    let (bounds, scale) = (window.bounds(), window.scale_factor());
    self.gesture = Some(Gesture {
      from: (bounds.origin + event.position) * scale,
      start: Bounds::new(bounds.origin * scale, bounds.size.map(|v| v * scale)),
      edge,
    });
    cx.notify();
  }

  /// Follows a move or resize over the whole screen (the window captures the mouse) until the
  /// button is released. Registered at each paint while it lasts.
  fn listen_to_gesture(view: WeakEntity<Self>, window: &mut Window) {
    let moved = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
      if phase != DispatchPhase::Capture {
        return;
      }
      moved
        .update(cx, |this, cx| {
          let Some(gesture) = this.gesture else { return };
          if event.pressed_button != Some(MouseButton::Left) {
            this.gesture = None;
            return cx.notify();
          }
          let scale = window.scale_factor();
          let max_width = window.display(cx).map_or(f32::MAX, |d| {
            f32::from(d.visible_bounds().size.width) * scale
          });
          let to = (window.bounds().origin + event.position) * scale;
          let b = gesture.bounds(to, MIN_WIDTH * scale, max_width);
          let logical = Bounds::new(b.origin / scale, b.size.map(|v| v / scale));
          ui::set_window_bounds(window, cx, logical);
        })
        .ok();
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
      if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
        view
          .update(cx, |this, cx| {
            this.gesture = None;
            cx.notify();
          })
          .ok();
      }
    });
  }
}

/// An invisible strip along an edge (or a square in a corner) that resizes the window.
fn edge(
  id: &'static str,
  edge: (i8, i8),
  cursor: CursorStyle,
  cx: &mut Context<MiniPlayer>,
) -> gpui::Stateful<gpui::Div> {
  let corner = edge.0 != 0 && edge.1 != 0;
  let thickness = px(if corner { CORNER } else { EDGE });
  let mut el = div().id(id).occlude().absolute().cursor(cursor);
  el = match edge.0 {
    -1 => el.left_0().w(thickness),
    1 => el.right_0().w(thickness),
    _ => el.left(px(CORNER)).right(px(CORNER)),
  };
  el = match edge.1 {
    -1 => el.top_0().h(thickness),
    1 => el.bottom_0().h(thickness),
    _ => el.top(px(CORNER)).bottom(px(CORNER)),
  };
  el.on_mouse_down(
    MouseButton::Left,
    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
      cx.stop_propagation();
      this.begin(edge, event, window, cx);
    }),
  )
}

/// A round button over the video.
fn button(
  id: &'static str,
  icon: impl gpui_component::IconNamed,
  size: f32,
) -> gpui::Stateful<gpui::Div> {
  div()
    .id(id)
    .size(px(size))
    .flex_none()
    .rounded_full()
    .flex()
    .items_center()
    .justify_center()
    .cursor_pointer()
    .text_color(gpui::white())
    .bg(gpui::black().opacity(0.45))
    .hover(|s| s.bg(gpui::black().opacity(0.7)))
    .child(gpui_component::Icon::new(icon).size(px(size * 0.5)))
}

impl Render for MiniPlayer {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let player = self.player(cx);
    if let Some(player) = &player
      && self.observed.as_ref().map(|(id, _)| *id) != Some(player.entity_id())
    {
      let subscription = cx.observe(player, |_, _, cx| cx.notify());
      self.observed = Some((player.entity_id(), subscription));
    }
    let video = player.as_ref().and_then(|p| p.read(cx).video());
    if let Some(video) = &video {
      // Square here; the page sets its own corners again when it shows the player.
      video.update(cx, |video, _| video.set_corner_radii(Corners::default()));
    }
    let paused = player.as_ref().is_some_and(|p| p.read(cx).is_paused());
    let show = window.is_window_hovered() || paused || self.gesture.is_some();
    let dragging = self.gesture.is_some();
    let view = cx.weak_entity();
    let edges: Vec<_> = EDGES
      .iter()
      .map(|&(id, side, cursor)| edge(id, side, cursor, cx))
      .collect();

    div()
      .id("mini-player")
      .size_full()
      .relative()
      .bg(gpui::black())
      // Redraws when the mouse comes and goes; whether the controls show is the window's hover
      // (the element's would turn off over the buttons, which occlude it, and flicker).
      .on_hover(cx.listener(|_, _, _, cx| cx.notify()))
      // Dragging the video (anything the buttons and edges leave) moves the window. Done here
      // rather than as a caption area: the window keeps the mouse, and so its hover.
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, event: &MouseDownEvent, window, cx| {
          this.begin((0, 0), event, window, cx)
        }),
      )
      .children(video.map(|video| div().absolute().inset_0().child(video)))
      // Window listeners can only be added while painting: a canvas does it during a gesture.
      .when(dragging, |el| {
        el.child(
          canvas(
            |_, _, _| {},
            move |_, _, window, _| Self::listen_to_gesture(view.clone(), window),
          )
          .absolute()
          .size_0(),
        )
      })
      // Edges and corners, over the video but under the controls: the hit test takes the
      // last-painted occluding element, so a visible button keeps its own clicks where it
      // overlaps a band, and every pixel it does not cover still resizes.
      .children(edges)
      .when(show, |el| {
        el.child(
          // Shade at the top, behind the buttons.
          div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(SHADE))
            .bg(linear_gradient(
              180.,
              linear_color_stop(gpui::black().opacity(0.6), 0.0),
              linear_color_stop(gpui::black().opacity(0.0), 1.0),
            ))
            .child(
              gpui_component::h_flex()
                .occlude()
                .absolute()
                .top(px(CONTROLS_INSET))
                .right(px(CONTROLS_INSET))
                .gap_1()
                .child(
                  button("mini-return", Icon::Maximize, CONTROL_SIZE)
                    .tooltip(|window, cx| {
                      gpui_component::tooltip::Tooltip::new(t(
                        "Back to the page",
                        "Revenir à la page",
                      ))
                      .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                      this.with_shell(cx, |shell, window, cx| shell.return_from_mini(window, cx));
                    })),
                )
                .child(
                  button("mini-close", IconName::Close, CONTROL_SIZE)
                    .tooltip(|window, cx| {
                      gpui_component::tooltip::Tooltip::new(t("Close", "Fermer")).build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                      this.with_shell(cx, |shell, _, cx| shell.dismiss_mini(cx));
                    })),
                ),
            ),
        )
        .child(
          div()
            .absolute()
            .inset_0()
            .top(px(SHADE))
            .flex()
            .items_center()
            .justify_center()
            .child(
              button(
                "mini-play",
                if paused { Icon::Play } else { Icon::Pause },
                PLAY_SIZE,
              )
              .occlude()
              .on_click(cx.listener(|this, _, _, cx| {
                if let Some(player) = this.player(cx) {
                  player.update(cx, |player, cx| player.toggle_pause(cx));
                }
              })),
            ),
        )
      })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn gesture(edge: (i8, i8)) -> Gesture {
    Gesture {
      from: point(px(1000.), px(500.)),
      start: Bounds::new(point(px(900.), px(400.)), size(px(400.), px(225.))),
      edge,
    }
  }

  fn at(g: &Gesture, dx: f32, dy: f32) -> Bounds<Pixels> {
    g.bounds(g.from + point(px(dx), px(dy)), MIN_WIDTH, 1920.)
  }

  // The bands' and controls' geometry is constant arithmetic, so it is asserted beside the
  // constants instead, when the crate compiles.

  #[test]
  fn a_move_keeps_the_size() {
    let b = at(&gesture((0, 0)), -50., 30.);
    assert_eq!(
      b,
      Bounds::new(point(px(850.), px(430.)), size(px(400.), px(225.)))
    );
  }

  #[test]
  fn the_top_left_corner_keeps_the_bottom_right_in_place() {
    let b = at(&gesture((-1, -1)), -160., -10.);
    assert_eq!(b.size, size(px(560.), px(315.)));
    assert_eq!((b.right(), b.bottom()), (px(1300.), px(625.)));
  }

  #[test]
  fn the_left_edge_keeps_the_right_side_and_the_vertical_middle() {
    let b = at(&gesture((-1, 0)), -80., 0.);
    assert_eq!(b.size, size(px(480.), px(270.)));
    assert_eq!(b.right(), px(1300.));
    assert_eq!(b.center().y, px(512.5));
  }

  #[test]
  fn the_bottom_edge_follows_the_height() {
    let b = at(&gesture((0, 1)), 0., 90.);
    assert_eq!(b.size, size(px(560.), px(315.)));
    assert_eq!(b.top(), px(400.));
  }

  #[test]
  fn sizes_are_clamped_and_the_anchor_holds() {
    let small = at(&gesture((-1, 0)), 1000., 0.);
    assert_eq!(small.size.width, px(MIN_WIDTH));
    assert_eq!(small.right(), px(1300.));
    let large = at(&gesture((1, 1)), 5000., 0.);
    assert_eq!(large.size.width, px(1920.));
    assert_eq!(large.origin, point(px(900.), px(400.)));
  }
}
