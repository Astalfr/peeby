//! Peeby: native media client for Windows (YouTube, Jellyfin, Twitch) built with GPUI.

// Release builds are GUI apps without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, AppContext, Bounds, DisplayId, Entity, WeakEntity, Window, px, size};
use gpui_component::Root;
use gpui_platform::application;
use peeby_core::shell::Shell;
use peeby_core::shell::space::SpaceHandle;
use peeby_core::{desktop, logs, settings, shell, update, web_sign_in};
use space_jellyfin::{JellyfinSpace, session};
use space_twitch::TwitchSpace;
use space_youtube::{YouTubeSpace, convert, prefetch, tools};
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
  // `peeby --twitch-chat <login>`: a channel's chat on twitch.tv for a few seconds (ADR 0012).
  if let Some(code) = web_sign_in::chat_from_args() {
    std::process::exit(code);
  }
  // `peeby --export-cookies`: the jar in clear on stdout, for the innertube examples (ADR 0022).
  if std::env::args().nth(1).as_deref() == Some(EXPORT_COOKIES) {
    std::process::exit(export_cookies());
  }
  std::thread::spawn(tools::remove_cookie_copy);
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
  let restore_tabs = prefs.restore_tabs;
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
  // Its WireGuard configuration (Credential Manager, blocking), when one was imported (ADR 0037).
  let jellyfin_vpn = session::load_vpn();
  // Twitch's secrets (Credential Manager, blocking), and its sign-in client.
  let twitch_secrets = space_twitch::session::restore();
  let twitch_auth = match twitch::auth::Auth::new(prefs.twitch.client_id.clone()) {
    Ok(auth) => Some(auth),
    Err(e) => {
      tracing::error!("cannot create the Twitch HTTP client: {e}");
      None
    }
  };
  let client = match innertube::Client::new(cookies, Some(settings::cookie_saver()), &hl, &gl) {
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
      // An installed copy installs a newer release before it opens, and opens as usual when
      // GitHub cannot say (ADR 0027).
      let startup = update::startup(background, cx);
      let open = move |cx: &mut App| {
        // Then it checks GitHub's releases every few hours.
        update::init(cx);
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

        // The last session's windows and tabs (ADR 0035), unless the command line asks for a page.
        let session = (restore_tabs && initial_videos.is_empty() && twitch_page.is_none())
          .then(shell::session::load)
          .flatten()
          .filter(|session| !session.windows.is_empty());
        let placement = session
          .as_ref()
          .and_then(|session| session.windows.first())
          .and_then(|window| window.placement(cx));
        let options = match placement {
          Some((bounds, display)) => shell::windows::options_for(bounds, display, !background, cx),
          None => {
            let display = chosen_display(cx);
            let bounds = Bounds::centered(display, size(px(1440.), px(900.)), cx);
            shell::windows::options(bounds, display, !background, cx)
          }
        };
        // The opening animation, unless something opens right away (a video, a live).
        let splash = initial_videos.is_empty() && twitch_page.is_none() && !background;
        let mut launches = None;
        let opened = cx.open_window(options, |window, cx| {
          let youtube = cx.new(|cx| YouTubeSpace::new(client, initial_videos, cx));
          let jellyfin = cx.new(|cx| JellyfinSpace::new(jellyfin_client, jellyfin_vpn, cx));
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
          // Before any space emits (a live opened at startup, once the shell is up).
          shell::windows::route_events(&spaces, cx);
          cx.set_global(shell::windows::Spaces(spaces.clone()));
          // The spaces' startup tabs give way to the session's, once one of these came back.
          let startup: Vec<_> = spaces
            .iter()
            .flat_map(|space| {
              space
                .tabs(cx)
                .into_iter()
                .map(move |tab| (space.clone(), tab))
            })
            .collect();
          let restored = session
            .as_ref()
            .and_then(|session| session.windows.first())
            .and_then(|saved| saved.restore(&spaces, cx));
          let shell = match restored {
            Some(restored) => {
              for (space, tab) in startup {
                space.close_tab(tab, cx);
              }
              cx.new(|cx| Shell::restored(spaces, restored, window, cx))
            }
            None => cx.new(|cx| Shell::new(spaces, splash, window, cx)),
          };
          desktop::attach(window, shell.downgrade(), cx);
          cx.new(|cx| Root::new(shell, window, cx))
        });
        if let Err(e) = opened {
          tracing::error!("cannot open the main window: {e:#}");
          cx.quit();
          return;
        }
        // The session's other windows.
        if let Some(session) = &session
          && let Some(spaces) = cx
            .try_global::<shell::windows::Spaces>()
            .map(|s| s.0.clone())
        {
          for saved in session.windows.iter().skip(1) {
            let Some(restored) = saved.restore(&spaces, cx) else {
              continue;
            };
            let (bounds, display) = saved.placement(cx).unwrap_or_else(|| {
              let display = chosen_display(cx);
              let bounds = Bounds::centered(display, size(px(1440.), px(900.)), cx);
              (gpui::WindowBounds::Windowed(bounds), display)
            });
            shell::windows::open_restored(restored, bounds, display, !background, cx);
          }
        }
        shell::session::keep_saving(cx);
        // The app ends when one of its windows closes: they only hide, unless it is the last one
        // and Peeby does not stay in the notification area (ADR 0028). Not with the mini-player's.
        cx.on_window_closed(move |cx, closed| {
          if shell::windows::get(closed, cx).is_some() {
            tracing::info!("a window closed: quitting");
            cx.quit();
          }
        })
        .detach();
        // A next launch's arguments, and the links dropped on a window, open the same way.
        if let Some(launches) = launches.map(Rc::new) {
          let opener = launches.clone();
          shell::drop::set_opener(
            Rc::new(
              move |args: Vec<String>, window: &mut Window, cx: &mut App| {
                opener.open(&args, window, cx)
              },
            ),
            cx,
          );
          if let Some(listener) = listener {
            desktop::listen(
              listener,
              move |args, window, cx| launches.open(&args, window, cx),
              cx,
            );
          }
        }
        if !background {
          cx.activate(true);
        }
      };
      cx.spawn(async move |cx| {
        if startup.await {
          tracing::info!("updates: the installer started, quitting");
          cx.update(|cx| cx.quit());
        } else {
          cx.update(open);
        }
      })
      .detach();
    });
}

/// The argument that writes the cookie jar in clear on stdout and ends.
const EXPORT_COOKIES: &str = "--export-cookies";

/// Writes the session's cookie jar (Netscape format, in clear) on stdout: the jar is encrypted
/// for this Windows user (ADR 0022) and the innertube examples read a plain one. Exit code 0,
/// or 1 without a session.
fn export_cookies() -> i32 {
  use std::io::Write as _;
  let Some(cookies) = settings::load_cookies() else {
    tracing::error!("no session cookies to export");
    return 1;
  };
  let mut stdout = std::io::stdout();
  match stdout
    .write_all(cookies.to_netscape().as_bytes())
    .and_then(|()| stdout.flush())
  {
    Ok(()) => 0,
    Err(e) => {
      tracing::error!("cannot write the cookies: {e}");
      1
    }
  }
}

/// The YouTube videos among a launch's arguments (`peeby <url-or-id>…`), the last one on top
/// ("back" goes through the others).
fn youtube_videos(args: &[String]) -> Vec<String> {
  args
    .iter()
    .filter(|arg| !arg.contains("twitch.tv/"))
    // Eleven characters a video id could have.
    .filter(|arg| *arg != FAKE_CHAT)
    .filter_map(|arg| space_youtube::video_id(arg).map(str::to_owned))
    .collect()
}

/// `peeby --fake-chat` (debug builds): opens the Twitch space's fake chat.
const FAKE_CHAT: &str = "--fake-chat";

/// The Twitch page among a launch's arguments: a live, a VOD or a clip (or the fake chat in a
/// debug build).
fn twitch_route(args: &[String]) -> Option<space_twitch::Route> {
  #[cfg(debug_assertions)]
  if args.iter().any(|arg| arg == FAKE_CHAT) {
    return Some(space_twitch::Route::FakeChat);
  }
  args
    .iter()
    .find_map(|arg| space_twitch::Route::from_url(arg))
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
          space.open_from_notification(space_youtube::Route::Watch(id), window, cx)
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
