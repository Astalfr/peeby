//! Peeby: native media client for Windows (YouTube, Jellyfin) built with GPUI.

// Release builds are GUI apps without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod settings;
mod shell;
mod spaces;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
  App, AppContext, Bounds, DisplayId, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_component::{Root, TitleBar};
use gpui_platform::application;
use shell::Shell;
use shell::space::SpaceHandle;
use spaces::jellyfin::JellyfinSpace;
use spaces::youtube::{YouTubeSpace, convert, prefetch, tools};
use ui::{assets, images, runtime, theme};

fn main() {
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "peeby=info,innertube=info".into()),
    )
    .init();

  // `peeby <url-or-id>…` opens those videos, the last one on top ("back" goes through the
  // others).
  let initial_videos: Vec<String> = std::env::args()
    .skip(1)
    .filter_map(|arg| spaces::youtube::video_id(&arg).map(str::to_owned))
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
  let prefs = settings::Settings::load();
  let (hl, gl) = (prefs.language.clone(), prefs.region.clone());
  let client = match innertube::Client::new(cookies, settings::cookies_path(), &hl, &gl) {
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
      // Images and stream resolution need no session: an anonymous client of their own, so
      // a cookie import only replaces the app's client.
      cx.set_global(prefs);
      match innertube::Client::new(None, None, &hl, &gl) {
        Ok(anonymous) => {
          images::init(Arc::new(convert::ImageFetch(anonymous.clone())), cx);
          prefetch::init(anonymous, cx);
        }
        Err(e) => tracing::error!("cannot create the HTTP client: {e}"),
      }

      // Current WEB client version and a visitorData, in the background.
      // yt-dlp (the fallback) keeps up with YouTube's changes.
      cx.background_executor()
        .spawn(async { tools::update_ytdlp() })
        .detach();

      let warmup = client.clone();
      drop(runtime::spawn(cx, async move {
        if let Err(e) = warmup.refresh_client_version().await {
          tracing::warn!("ytcfg: {e}");
        }
      }));

      let display = chosen_display(cx);
      let bounds = Bounds::centered(display, size(px(1440.), px(900.)), cx);
      let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display,
        window_min_size: Some(size(px(960.), px(600.))),
        window_background: theme::window_background(cx),
        titlebar: Some(TitlebarOptions {
          title: Some("Peeby".into()),
          ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
      };
      let opened = cx.open_window(options, |window, cx| {
        let youtube = cx.new(|cx| YouTubeSpace::new(client, initial_videos, window, cx));
        let jellyfin = cx.new(|_| JellyfinSpace);
        let spaces: Vec<SpaceHandle> = vec![Rc::new(youtube.clone()), Rc::new(jellyfin)];
        let shell = cx.new(|cx| Shell::new(spaces, youtube.downgrade(), window, cx));
        cx.new(|cx| Root::new(shell, window, cx))
      });
      let main = match opened {
        Ok(main) => main.window_id(),
        Err(e) => {
          tracing::error!("cannot open the main window: {e:#}");
          cx.quit();
          return;
        }
      };
      // The app ends with its main window, not with the mini-player's.
      cx.on_window_closed(move |cx, closed| {
        if closed == main {
          cx.quit();
        }
      })
      .detach();
      cx.activate(true);
    });
}

/// The display the window opens on: `PEEBY_DISPLAY=2` for the second one (in Windows' order),
/// `PEEBY_DISPLAY=secondary` for the first that is not the primary. Default: the system's choice.
fn chosen_display(cx: &App) -> Option<DisplayId> {
  let wanted = std::env::var("PEEBY_DISPLAY").ok()?;
  let displays = cx.displays();
  let display = if wanted.eq_ignore_ascii_case("secondary") {
    let primary = cx.primary_display().map(|d| d.id());
    displays.into_iter().find(|d| Some(d.id()) != primary)
  } else {
    let index = wanted.parse::<usize>().ok()?.checked_sub(1)?;
    displays.into_iter().nth(index)
  };
  if display.is_none() {
    tracing::warn!("PEEBY_DISPLAY={wanted}: no such display, using the default one");
  }
  display.map(|d| d.id())
}
