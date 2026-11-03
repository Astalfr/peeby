//! Peeby: native media client for Windows (YouTube, Jellyfin, Twitch) built with GPUI.

// Release builds are GUI apps without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod logs;
mod settings;
mod shell;
mod spaces;
mod web_sign_in;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
  App, AppContext, Bounds, DisplayId, Entity, TitlebarOptions, WeakEntity, Window, WindowBounds,
  WindowOptions, px, size,
};
use gpui_component::{Root, TitleBar};
use gpui_platform::application;
use shell::Shell;
use shell::space::SpaceHandle;
use spaces::jellyfin::{JellyfinSpace, session};
use spaces::twitch::TwitchSpace;
use spaces::youtube::{YouTubeSpace, convert, prefetch, tools};
use ui::{assets, images, runtime, theme};

fn main() {
  logs::init();

  let prefs = settings::Settings::load();
  std::thread::spawn(settings::prune_logs);
  ui::i18n::set(prefs.interface_lang());
  // `peeby --login <service>`: the sign-in window only (ADR 0017), started by Peeby itself.
  if let Some(code) = web_sign_in::child_from_args() {
    std::process::exit(code);
  }
  // One Peeby at a time (ADR 0020): two would rewrite the same cookie jar and spend the same
  // Twitch refresh token. A next launch hands its arguments to the first one and ends.
  let args: Vec<String> = std::env::args().skip(1).collect();
  let listener = match desktop::sys::claim(&args.join(
    "
",
  )) {
    Some(desktop::sys::Instance::Running) => return,
    Some(desktop::sys::Instance::First(listener)) => Some(listener),
    None => None,
  };
  // Started with Windows: straight to the notification area.
  let background = args.iter().any(|arg| arg == desktop::BACKGROUND);

  // `peeby <url-or-id>…` opens those videos, the last one on top ("back" goes through the
  // others).
  let initial_videos = youtube_videos(&args);
  // `peeby twitch.tv/<login>` (a live), `twitch.tv/videos/<id>` or a clip's link opens it in the
  // Twitch tab.
  let twitch_page = twitch_route(&args);

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
  let (hl, gl) = (prefs.language.clone(), prefs.region.clone());
  // The Jellyfin session, with its token from the Credential Manager (a blocking call). A first
  // run makes this installation's id, saved once the UI is up.
  let device_id = match prefs.jellyfin.device_id.as_str() {
    "" => session::new_device_id(),
    id => id.to_owned(),
  };
  let jellyfin_client = session::restore(&settings::Jellyfin {
    device_id: device_id.clone(),
    ..prefs.jellyfin.clone()
  });
  // Twitch's secrets (Credential Manager, blocking), and its sign-in client.
  let twitch_secrets = spaces::twitch::session::restore();
  let twitch_auth = match twitch::auth::Auth::new(prefs.twitch.client_id.clone()) {
    Ok(auth) => Some(auth),
    Err(e) => {
      tracing::error!("cannot create the Twitch HTTP client: {e}");
      None
    }
  };
  let client = match innertube::Client::new(cookies, settings::cookies_path(), &hl, &gl) {
    Ok(client) => client.with_original_titles(prefs.original_titles),
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
      cx.set_global(ui::player::AudioOutput(prefs.player.audio_device.clone()));
      cx.set_global(prefs);
      settings::Settings::update(cx, |s| s.jellyfin.device_id = device_id);
      desktop::init(cx);
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
        show: !background,
        focus: !background,
        // GPUI caps an unfocused window at 30 fps by default: video plays while you work elsewhere.
        inactive_frame_interval: None,
        titlebar: Some(TitlebarOptions {
          title: Some(settings::APP_NAME.into()),
          ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
      };
      // The opening animation, unless something opens right away (a video, a live).
      let splash = initial_videos.is_empty() && twitch_page.is_none() && !background;
      let mut launches = None;
      let opened = cx.open_window(options, |window, cx| {
        let youtube = cx.new(|cx| YouTubeSpace::new(client, initial_videos, cx));
        let jellyfin = cx.new(|cx| JellyfinSpace::new(jellyfin_client, cx));
        let mut spaces: Vec<SpaceHandle> = vec![Rc::new(youtube.clone()), Rc::new(jellyfin)];
        let twitch = twitch_auth
          .map(|auth| cx.new(|cx| TwitchSpace::new(auth, twitch_secrets, twitch_page, cx)));
        if let Some(twitch) = &twitch {
          spaces.push(Rc::new(twitch.clone()));
        }
        launches = Some(Launches {
          youtube: youtube.downgrade(),
          twitch: twitch.as_ref().map(Entity::downgrade),
        });
        let shell = cx.new(|cx| Shell::new(spaces, splash, window, cx));
        desktop::attach(window, shell.downgrade(), cx);
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
      if let (Some(listener), Some(launches)) = (listener, launches) {
        desktop::listen(
          listener,
          move |args, window, cx| launches.open(&args, window, cx),
          cx,
        );
      }
      if !background {
        cx.activate(true);
      }
    });
}

/// The YouTube videos among a launch's arguments (`peeby <url-or-id>…`), the last one on top
/// ("back" goes through the others).
fn youtube_videos(args: &[String]) -> Vec<String> {
  args
    .iter()
    .filter(|arg| !arg.contains("twitch.tv/"))
    .filter_map(|arg| spaces::youtube::video_id(arg).map(str::to_owned))
    .collect()
}

/// The Twitch page among a launch's arguments: a live, a VOD or a clip.
fn twitch_route(args: &[String]) -> Option<spaces::twitch::Route> {
  args
    .iter()
    .find_map(|arg| spaces::twitch::Route::from_url(arg))
}

/// The spaces that open what a next launch was given (ADR 0020).
struct Launches {
  youtube: WeakEntity<YouTubeSpace>,
  twitch: Option<WeakEntity<TwitchSpace>>,
}

impl Launches {
  fn open(&self, args: &[String], window: &mut Window, cx: &mut App) {
    for id in youtube_videos(args) {
      self
        .youtube
        .update(cx, |space, cx| {
          space.open_from_notification(spaces::youtube::Route::Watch(id), window, cx)
        })
        .ok();
    }
    if let (Some(route), Some(twitch)) = (twitch_route(args), &self.twitch) {
      twitch
        .update(cx, |space, cx| space.open_from_notification(route, cx))
        .ok();
    }
  }
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
