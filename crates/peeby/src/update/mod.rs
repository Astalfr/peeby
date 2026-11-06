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

/// The update's state, and the task that checks. Observed by the title bar and the settings page.
pub struct Updater {
  status: Status,
  /// A check is running (the periodic one or the settings page's).
  checking: bool,
  /// How the last check asked from the settings page ended, until the next one.
  asked: Option<Outcome>,
  http: reqwest::Client,
  dir: PathBuf,
  _checks: Task<()>,
}

impl Global for Updater {}

/// How a check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
  /// No newer release, or the newer one is already downloaded.
  UpToDate,
  /// A newer release was downloaded.
  Found,
  Failed,
}

/// What the title bar and the settings page show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
  /// This copy does not update itself (a debug build, `cargo run`).
  Unavailable,
  /// Nothing to show: not checked from the settings page, nothing newer.
  Idle,
  Checking,
  /// The check asked from the settings page found nothing newer.
  UpToDate,
  /// The check asked from the settings page failed.
  Failed,
  Downloading(Version),
  Ready(Version),
  Installing(Version),
}

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
  // Old installers go at once, before a check from the settings page can download a new one.
  let clear = dir.clone();
  let checks = cx.spawn(async move |cx| {
    cx.background_executor()
      .spawn(async move { github::clear_downloads(&clear) })
      .await;
    cx.background_executor().timer(FIRST_CHECK).await;
    loop {
      run_check(false, cx).await;
      cx.background_executor().timer(CHECK_EVERY).await;
    }
  });
  cx.set_global(Updater {
    status: Status::Idle,
    checking: false,
    asked: None,
    http,
    dir,
    _checks: checks,
  });
}

/// The settings page's button: checks now, unless a check is already running.
pub fn check_now(cx: &mut App) {
  cx.spawn(async move |cx| run_check(true, cx).await).detach();
}

/// Runs one check unless one is already running; `asked` keeps its outcome for the settings page.
async fn run_check(asked: bool, cx: &mut AsyncApp) {
  let start = cx.update(|cx| {
    let updater = cx.try_global::<Updater>()?;
    if updater.checking {
      return None;
    }
    let (http, dir) = (updater.http.clone(), updater.dir.clone());
    let updater = cx.global_mut::<Updater>();
    updater.checking = true;
    if asked {
      updater.asked = None;
    }
    Some((http, dir))
  });
  let Some((http, dir)) = start else {
    return;
  };
  let outcome = check(&http, &dir, cx).await;
  cx.update(|cx| {
    if cx.try_global::<Updater>().is_some() {
      let updater = cx.global_mut::<Updater>();
      updater.checking = false;
      if asked {
        updater.asked = Some(outcome);
      }
    }
  });
}

/// Where installers are downloaded: `%LOCALAPPDATA%\peeby\updates`.
fn downloads_dir() -> Option<PathBuf> {
  Some(dirs::cache_dir()?.join("peeby").join("updates"))
}

/// One check: the latest release, and its installer if it is newer than this build.
async fn check(http: &reqwest::Client, dir: &std::path::Path, cx: &mut AsyncApp) -> Outcome {
  let fetch = {
    let http = http.clone();
    cx.update(|cx| runtime::spawn(cx, async move { github::latest(&http).await }))
  };
  let latest: Release = match fetch.await {
    Some(Ok(Some(release))) => release,
    Some(Ok(None)) => return Outcome::UpToDate,
    Some(Err(e)) => {
      tracing::warn!("updates: cannot check: {e:#}");
      return Outcome::Failed;
    }
    None => return Outcome::Failed,
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
    return Outcome::UpToDate;
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
  let (status, outcome) = match download.await {
    Some(Ok(path)) => {
      tracing::info!("updates: {version} ready");
      (Status::Ready(version, path), Outcome::Found)
    }
    Some(Err(e)) => {
      // The next check tries again.
      tracing::warn!("updates: cannot download {version}: {e:#}");
      (Status::Idle, Outcome::Failed)
    }
    None => (Status::Idle, Outcome::Failed),
  };
  cx.update(|cx| set_status(status, cx));
  outcome
}

/// Changes the state; the title bar redraws (`global_mut` notifies the global's observers).
fn set_status(status: Status, cx: &mut App) {
  match cx.try_global::<Updater>() {
    Some(updater) if updater.status != status => cx.global_mut::<Updater>().status = status,
    _ => {}
  }
}

/// Where the update stands, for the title bar's button and the settings page.
pub fn state(cx: &App) -> State {
  let Some(updater) = cx.try_global::<Updater>() else {
    return State::Unavailable;
  };
  match &updater.status {
    Status::Downloading(v) => State::Downloading(*v),
    Status::Ready(v, _) => State::Ready(*v),
    Status::Installing(v, _) => State::Installing(*v),
    Status::Idle if updater.checking => State::Checking,
    Status::Idle => match updater.asked {
      Some(Outcome::UpToDate) => State::UpToDate,
      Some(Outcome::Failed) => State::Failed,
      // A download that failed afterwards went back to `Idle`.
      Some(Outcome::Found) => State::Failed,
      None => State::Idle,
    },
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
