//! Peeby updates itself from the project's GitHub Releases (ADR 0027). An installed copy checks
//! the latest release shortly after it starts and then every few hours; a newer one has its
//! installer downloaded and checked (SHA-256) in the background, and only then does the title
//! bar show the green update button. A click starts the installer silently and quits; the
//! installer replaces the files and starts Peeby again.
//!
//! Only a copy installed by the installer checks (not `cargo run`, not a debug build).

mod github;
mod release;

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, AsyncApp, Global, Task, Window};
use gpui_component::WindowExt as _;
use gpui_component::notification::Notification;
use ui::i18n::t;
use ui::runtime;

use crate::settings::Settings;
pub use release::Version;
use release::{Release, Status};

/// The first check, once the startup's own requests are done.
const FIRST_CHECK: Duration = Duration::from_secs(20);
/// Then one check every few hours: Peeby stays in the notification area for days. GitHub allows
/// 60 anonymous requests an hour.
const CHECK_EVERY: Duration = Duration::from_secs(4 * 3600);

/// The update's state, and the task that checks. Observed by the title bar.
pub struct Updater {
  status: Status,
  _checks: Task<()>,
}

impl Global for Updater {}

/// Starts the checks, if this copy can update itself.
pub fn init(cx: &mut App) {
  if cfg!(debug_assertions) {
    return;
  }
  let installed = std::env::current_exe().is_ok_and(|exe| github::installed_by_setup(&exe));
  if !installed {
    tracing::info!("updates: not an installed copy, no checks");
    return;
  }
  let (Some(dir), Ok(http)) = (downloads_dir(), github::http()) else {
    tracing::warn!("updates: no download folder or HTTP client");
    return;
  };
  let checks = cx.spawn(async move |cx| {
    cx.background_executor().timer(FIRST_CHECK).await;
    let clear = dir.clone();
    cx.background_executor()
      .spawn(async move { github::clear_downloads(&clear) })
      .await;
    loop {
      check(&http, &dir, cx).await;
      cx.background_executor().timer(CHECK_EVERY).await;
    }
  });
  cx.set_global(Updater {
    status: Status::Idle,
    _checks: checks,
  });
}

/// Where installers are downloaded: `%LOCALAPPDATA%\peeby\updates`.
fn downloads_dir() -> Option<PathBuf> {
  Some(dirs::cache_dir()?.join("peeby").join("updates"))
}

/// One check: the latest release, and its installer if it is newer than this build.
async fn check(http: &reqwest::Client, dir: &std::path::Path, cx: &mut AsyncApp) {
  let fetch = {
    let http = http.clone();
    cx.update(|cx| runtime::spawn(cx, async move { github::latest(&http).await }))
  };
  let latest: Release = match fetch.await {
    Some(Ok(Some(release))) => release,
    Some(Ok(None)) => return,
    Some(Err(e)) => {
      tracing::warn!("updates: cannot check: {e:#}");
      return;
    }
    None => return,
  };
  let version = latest.version;
  let wanted = cx.update(|cx| {
    let wanted = cx
      .try_global::<Updater>()
      .is_some_and(|u| u.status.wants(Version::current(), version));
    if wanted {
      set_status(Status::Downloading(version), cx);
    }
    wanted
  });
  if !wanted {
    return;
  }
  tracing::info!("updates: downloading {version}");
  let download = {
    let (http, dir) = (http.clone(), dir.to_path_buf());
    cx.update(|cx| {
      runtime::spawn(
        cx,
        async move { github::download(&http, &latest, &dir).await },
      )
    })
  };
  let status = match download.await {
    Some(Ok(path)) => {
      tracing::info!("updates: {version} ready");
      Status::Ready(version, path)
    }
    Some(Err(e)) => {
      // The next check tries again.
      tracing::warn!("updates: cannot download {version}: {e:#}");
      Status::Idle
    }
    None => Status::Idle,
  };
  cx.update(|cx| set_status(status, cx));
}

/// Changes the state; the title bar redraws (`global_mut` notifies the global's observers).
fn set_status(status: Status, cx: &mut App) {
  match cx.try_global::<Updater>() {
    Some(updater) if updater.status != status => cx.global_mut::<Updater>().status = status,
    _ => {}
  }
}

/// The version whose installer is ready, while the title bar's button should show.
pub fn ready(cx: &App) -> Option<Version> {
  match cx.try_global::<Updater>()?.status {
    Status::Ready(version, _) => Some(version),
    _ => None,
  }
}

/// The button's click: the installer starts, and Peeby quits once the settings are written. The
/// installer starts Peeby again when it is done.
pub fn install(window: &mut Window, cx: &mut App) {
  let Some(Status::Ready(version, path)) = cx.try_global::<Updater>().map(|u| u.status.clone())
  else {
    return;
  };
  set_status(Status::Installing(version, path.clone()), cx);
  tracing::info!("updates: installing {version}");
  let settings = Settings::get(cx).clone();
  let handle = window.window_handle();
  let installer = path.clone();
  cx.spawn(async move |cx| {
    let launched = cx
      .background_executor()
      .spawn(async move {
        // A change of the last half second is still waiting to be written (`Settings::update`).
        if let Err(e) = settings.save_now() {
          tracing::warn!("cannot save the settings: {e}");
        }
        github::launch_installer(&installer)
      })
      .await;
    match launched {
      Ok(()) => cx.update(|cx| cx.quit()),
      Err(e) => {
        tracing::warn!("updates: {e:#}");
        cx.update(|cx| {
          set_status(Status::Ready(version, path), cx);
          handle
            .update(cx, |_, window, cx| {
              let note = Notification::error(t(
                "The update could not start.",
                "La mise à jour n'a pas pu démarrer.",
              ));
              window.push_notification(note, cx);
            })
            .ok();
        });
      }
    }
  })
  .detach();
}
