//! The opening animation alone, over and over, to tune its timings and springs (constants at the
//! top of crates/ui/src/splash.rs). A click skips to the fade out, as in the app.
//!
//! `cargo run -p ui --example splash --release` · `PEEBY_SPLASH_AT=1.2` holds that moment (seconds) ·
//! `PEEBY_DISPLAY=secondary` opens it on another screen.

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  App, AppContext, Bounds, Context, MouseButton, TitlebarOptions, Window, WindowBounds,
  WindowOptions, div, px, size,
};
use gpui_component::{Root, TitleBar};
use gpui_platform::application;
use ui::splash::Splash;
use ui::{assets, theme};

/// Pause between two runs.
const PAUSE: Duration = Duration::from_millis(700);

struct Demo {
  splash: Splash,
  frozen: Option<f32>,
  /// When the last run ended.
  ended: Option<Instant>,
}

impl Render for Demo {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.ended.is_some_and(|at| at.elapsed() >= PAUSE) {
      self.ended = None;
      self.splash = Splash::new();
    }
    let frame = if self.ended.is_some() {
      window.request_animation_frame();
      None
    } else {
      let frame = self.splash.render(window);
      if frame.is_none() {
        self.ended = self.frozen.is_none().then(Instant::now);
        window.request_animation_frame();
      }
      frame
    };
    div()
      .size_full()
      .relative()
      .bg(theme::background())
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _, _, cx| {
          this.splash.skip();
          cx.notify();
        }),
      )
      .children(frame)
  }
}

fn main() {
  let frozen = std::env::var("PEEBY_SPLASH_AT")
    .ok()
    .and_then(|s| s.parse::<f32>().ok());
  application()
    .with_assets(assets::Assets)
    .run(move |cx: &mut App| {
      gpui_component::init(cx);
      theme::apply(cx);
      let display = (std::env::var("PEEBY_DISPLAY").as_deref() == Ok("secondary"))
        .then(|| {
          let primary = cx.primary_display().map(|d| d.id());
          cx.displays()
            .into_iter()
            .find(|d| Some(d.id()) != primary)
            .map(|d| d.id())
        })
        .flatten();
      let bounds = Bounds::centered(display, size(px(900.), px(620.)), cx);
      let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display,
        window_background: gpui::WindowBackgroundAppearance::Opaque,
        titlebar: Some(TitlebarOptions {
          title: Some("Peeby splash".into()),
          ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
      };
      let opened = cx.open_window(options, |window, cx| {
        let demo = cx.new(|_| Demo {
          splash: frozen.map_or_else(Splash::new, Splash::frozen),
          frozen,
          ended: None,
        });
        cx.new(|cx| Root::new(demo, window, cx))
      });
      if opened.is_err() {
        cx.quit();
      }
    });
}
