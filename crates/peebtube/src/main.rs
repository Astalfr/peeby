//! PeebTube: native YouTube client for Windows built with GPUI.

// Release builds are GUI apps without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod assets;
mod feed;
mod images;
mod player_view;
mod runtime;
mod settings;
mod theme;
mod tools;
mod ui;
mod watch;

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, TitleBar};
use gpui_platform::application;

fn main() {
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "peebtube=info,innertube=info".into()),
    )
    .init();

  // `peebtube <url-or-id>…` opens those videos, the last one on top ("back" goes through the
  // others).
  let initial_videos: Vec<String> = std::env::args()
    .skip(1)
    .filter_map(|arg| app::video_id(&arg).map(str::to_owned))
    .collect();

  // Small local file, read before the UI starts.
  let cookies = settings::load_cookies();
  tracing::info!(
    "{}",
    if cookies.is_some() {
      "session cookies loaded"
    } else {
      "no session cookies: signed out"
    }
  );
  let (hl, gl) = settings::LANGUAGE;
  let client = match innertube::Client::new(cookies, settings::cookies_path(), hl, gl) {
    Ok(client) => client,
    Err(e) => {
      tracing::error!("cannot create the HTTP client: {e}");
      return;
    }
  };

  application()
    .with_assets(assets::Assets)
    .run(move |cx: &mut App| {
      if let Err(e) = runtime::init(cx) {
        tracing::error!("cannot start the network runtime: {e:#}");
        cx.quit();
        return;
      }
      gpui_component::init(cx);
      theme::apply(cx);
      images::init(client.clone(), cx);

      // Current WEB client version and a visitorData, in the background.
      let warmup = client.clone();
      drop(runtime::spawn(cx, async move {
        if let Err(e) = warmup.refresh_client_version().await {
          tracing::warn!("ytcfg: {e}");
        }
      }));

      let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
      let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(960.), px(600.))),
        window_background: theme::window_background(cx),
        titlebar: Some(TitlebarOptions {
          title: Some("PeebTube".into()),
          ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
      };
      let opened = cx.open_window(options, |window, cx| {
        let app = cx.new(|cx| app::AppView::new(client, initial_videos, window, cx));
        cx.new(|cx| Root::new(app, window, cx))
      });
      if let Err(e) = opened {
        tracing::error!("cannot open the main window: {e:#}");
        cx.quit();
        return;
      }
      cx.on_window_closed(|cx, _| cx.quit()).detach();
      cx.activate(true);
    });
}
