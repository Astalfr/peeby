//! The mini-player: a window of its own, above every other window and outside the main one, as
//! in Chrome's picture-in-picture (ADR 0008). It shows the video of the space that was left
//! while it played, without restarting it: the same player and video surface move here. On
//! hover: a strip at the top to move the window, buttons to go back to the page and to close
//! (which pauses), play/pause in the middle, and a grip in the corner to resize it at 16:9.

use gpui::prelude::*;
use gpui::{
  AnyWindowHandle, App, Bounds, Context, Corners, DispatchPhase, Entity, EntityId, MouseButton,
  MouseMoveEvent, MouseUpEvent, Pixels, Point, Subscription, WeakEntity, Window,
  WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowHandle, WindowKind,
  WindowOptions, canvas, div, linear_color_stop, linear_gradient, point, px, size,
};
use gpui_component::IconName;
use ui::assets::Icon;
use ui::player::PlayerView;

use super::Shell;
use super::space::SpaceHandle;

/// Width of a new mini-player, logical pixels.
const WIDTH: f32 = 400.;
/// Smallest width the grip resizes to.
const MIN_WIDTH: f32 = 240.;
/// Gap between the window and the screen's edges.
const MARGIN: f32 = 24.;

/// A resize in progress: where the mouse went down and the window's width then.
#[derive(Clone, Copy)]
struct Resize {
  from: Point<Pixels>,
  width: f32,
}

/// The mini-player's view.
pub struct MiniPlayer {
  space: SpaceHandle,
  shell: WeakEntity<Shell>,
  /// The main window, where the shell lives.
  main: AnyWindowHandle,
  resize: Option<Resize>,
  /// Redraws the buttons when the player shown pauses or plays.
  observed: Option<(EntityId, Subscription)>,
}

/// Opens the mini-player on the screen of `window` (the main one), at its bottom right.
pub fn open(
  space: SpaceHandle,
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
    is_resizable: false,
    window_background: WindowBackgroundAppearance::Opaque,
    ..Default::default()
  };
  cx.open_window(options, |_, cx| {
    cx.new(|_| MiniPlayer {
      space,
      shell,
      main,
      resize: None,
      observed: None,
    })
  })
}

impl MiniPlayer {
  fn player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    self.space.player(cx)
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

  /// Follows the grip's drag over the whole screen (the window captures the mouse) until the
  /// button is released. Registered at each paint while the drag lasts.
  fn listen_to_resize(view: WeakEntity<Self>, window: &mut Window) {
    let moved = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
      if phase != DispatchPhase::Capture {
        return;
      }
      moved
        .update(cx, |this, cx| {
          let Some(resize) = this.resize else { return };
          if event.pressed_button != Some(MouseButton::Left) {
            this.resize = None;
            return cx.notify();
          }
          let width = (resize.width + f32::from(event.position.x - resize.from.x)).max(MIN_WIDTH);
          window.resize(size(px(width), px(width * 9. / 16.)));
        })
        .ok();
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
      if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
        view
          .update(cx, |this, cx| {
            this.resize = None;
            cx.notify();
          })
          .ok();
      }
    });
  }
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
    let show = window.is_window_hovered() || paused || self.resize.is_some();
    let resizing = self.resize.is_some();
    let view = cx.weak_entity();

    div()
      .id("mini-player")
      .size_full()
      .relative()
      .bg(gpui::black())
      // Redraws when the mouse comes and goes; whether the controls show is the window's hover
      // (the element's would turn off over the buttons, which occlude it, and flicker).
      .on_hover(cx.listener(|_, _, _, cx| cx.notify()))
      .children(video.map(|video| div().absolute().inset_0().child(video)))
      // Window listeners can only be added while painting: a canvas does it during a resize.
      .when(resizing, |el| {
        el.child(
          canvas(
            |_, _, _| {},
            move |_, _, window, _| Self::listen_to_resize(view.clone(), window),
          )
          .absolute()
          .size_0(),
        )
      })
      .when(show, |el| {
        el.child(
          // Strip at the top: moves the window (a double click does not maximize it, the
          // window has no maximize box), with the buttons on its right.
          div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(40.))
            .bg(linear_gradient(
              180.,
              linear_color_stop(gpui::black().opacity(0.6), 0.0),
              linear_color_stop(gpui::black().opacity(0.0), 1.0),
            ))
            .child(
              div()
                .id("mini-drag")
                .absolute()
                .inset_0()
                .window_control_area(WindowControlArea::Drag),
            )
            .child(
              gpui_component::h_flex()
                .occlude()
                .absolute()
                .top(px(6.))
                .right(px(6.))
                .gap_1()
                .child(
                  button("mini-return", Icon::Maximize, 28.)
                    .tooltip(|window, cx| {
                      gpui_component::tooltip::Tooltip::new("Revenir à la page").build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                      this.with_shell(cx, |shell, window, cx| shell.return_from_mini(window, cx));
                    })),
                )
                .child(
                  button("mini-close", IconName::Close, 28.)
                    .tooltip(|window, cx| {
                      gpui_component::tooltip::Tooltip::new("Fermer").build(window, cx)
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
            .top(px(40.))
            .flex()
            .items_center()
            .justify_center()
            .child(
              button(
                "mini-play",
                if paused { Icon::Play } else { Icon::Pause },
                48.,
              )
              .occlude()
              .on_click(cx.listener(|this, _, _, cx| {
                if let Some(player) = this.player(cx) {
                  player.read(cx).toggle_pause();
                }
              })),
            ),
        )
        .child(
          // Grip in the bottom right corner: resizes, keeping 16:9.
          div()
            .id("mini-grip")
            .occlude()
            .absolute()
            .right_0()
            .bottom_0()
            .size(px(18.))
            .cursor_nwse_resize()
            .child(
              div()
                .absolute()
                .right(px(4.))
                .bottom(px(4.))
                .size(px(8.))
                .border_r_2()
                .border_b_2()
                .border_color(gpui::white().opacity(0.8)),
            )
            .on_mouse_down(
              MouseButton::Left,
              cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                this.resize = Some(Resize {
                  from: event.position,
                  width: f32::from(window.bounds().size.width),
                });
                cx.notify();
              }),
            ),
        )
      })
  }
}
