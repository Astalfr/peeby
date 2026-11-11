//! The update at startup (ADR 0027): before Peeby opens, an installed copy asks GitHub for the
//! latest release, and a newer one is installed at once: downloaded (a small window shows how
//! far, none at a start with Windows), checked, then its installer starts and Peeby quits; the
//! installer starts it again. If anything fails (no network, GitHub slow or down, a corrupt
//! download), Peeby opens as usual and the periodic checks try again later.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use gpui::prelude::*;
use gpui::{
  AnyWindowHandle, App, AsyncApp, Bounds, Context, Task, TitlebarOptions, Window, WindowBounds,
  WindowOptions, div, px, relative, size,
};
use ui::i18n::t;
use ui::{runtime, tf, theme};

use super::github::{self, Progress};
use super::release::Version;
use crate::settings::{self, Settings};

/// How long the startup's check may take: past it, Peeby opens without waiting for GitHub.
const CHECK_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the window redraws its progress.
const REDRAW: Duration = Duration::from_millis(100);

/// Installs a newer release before Peeby opens. Resolves to `true` once its installer started
/// (Peeby should quit), `false` to open Peeby: nothing newer, a copy that does not update itself,
/// or a failure. `background`: started with Windows, so no window.
pub fn startup(background: bool, cx: &mut App) -> Task<bool> {
  let Some((http, dir)) = super::available() else {
    return Task::ready(false);
  };
  cx.spawn(async move |cx| {
    let mut window = None;
    let result = run(&http, &dir, background, &mut window, cx).await;
    let started = result.unwrap_or_else(|e| {
      tracing::warn!("updates: no update at startup: {e:#}");
      false
    });
    if !started && let Some(window) = window {
      cx.update(|cx| {
        window
          .update(cx, |_, window, _| window.remove_window())
          .ok();
      });
    }
    started
  })
}

/// The check, the download and the installer's start. `window`: the progress window, once open.
async fn run(
  http: &reqwest::Client,
  dir: &std::path::Path,
  background: bool,
  window: &mut Option<AnyWindowHandle>,
  cx: &mut AsyncApp,
) -> anyhow::Result<bool> {
  let fetch = {
    let http = http.clone();
    cx.update(|cx| {
      runtime::spawn(
        cx,
        async move { github::latest(&http, CHECK_TIMEOUT).await },
      )
    })
  };
  let Some(latest) = fetch.await.context("the network runtime stopped")?? else {
    return Ok(false);
  };
  let version = latest.version;
  if version <= Version::current() {
    return Ok(false);
  }
  tracing::info!("updates: installing {version} at startup");
  let progress = Arc::new(Progress::default());
  if !background {
    let progress = progress.clone();
    *window = cx.update(|cx| open_window(version, progress, cx));
  }
  // An earlier session may have downloaded it already.
  let found = {
    let (latest, dir) = (latest.clone(), dir.to_path_buf());
    cx.background_executor()
      .spawn(async move { github::downloaded(&latest, &dir) })
      .await
  };
  let path = match found {
    Some(path) => path,
    None => {
      let (http, dir) = (http.clone(), dir.to_path_buf());
      let download = cx.update(|cx| {
        runtime::spawn(cx, async move {
          github::download(&http, &latest, &dir, &progress).await
        })
      });
      download.await.context("the network runtime stopped")??
    }
  };
  let settings = cx.update(|cx| Settings::get(cx).clone());
  cx.background_executor()
    .spawn(async move {
      if let Err(e) = settings.save_now() {
        tracing::warn!("cannot save the settings: {e}");
      }
      github::launch_installer(&path, background)
    })
    .await?;
  Ok(true)
}

/// The small window that shows the download.
fn open_window(version: Version, progress: Arc<Progress>, cx: &mut App) -> Option<AnyWindowHandle> {
  let bounds = Bounds::centered(None, size(px(440.), px(150.)), cx);
  let options = WindowOptions {
    window_bounds: Some(WindowBounds::Windowed(bounds)),
    titlebar: Some(TitlebarOptions {
      title: Some(settings::APP_NAME.into()),
      ..Default::default()
    }),
    is_resizable: false,
    is_minimizable: false,
    window_background: theme::window_background(cx),
    ..Default::default()
  };
  let opened = cx.open_window(options, |_, cx| {
    cx.new(|cx| UpdateWindow::new(version, progress, cx))
  });
  match opened {
    Ok(handle) => {
      cx.activate(true);
      Some(handle.into())
    }
    // The update goes on without it.
    Err(e) => {
      tracing::warn!("updates: cannot open the progress window: {e:#}");
      None
    }
  }
}

struct UpdateWindow {
  version: Version,
  progress: Arc<Progress>,
  _redraw: Task<()>,
}

impl UpdateWindow {
  fn new(version: Version, progress: Arc<Progress>, cx: &mut Context<Self>) -> Self {
    let redraw = cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(REDRAW).await;
        if this.update(cx, |_, cx| cx.notify()).is_err() {
          break;
        }
      }
    });
    Self {
      version,
      progress,
      _redraw: redraw,
    }
  }
}

impl Render for UpdateWindow {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    let version = self.version;
    let fraction = self.progress.fraction();
    let detail = match fraction {
      Some(f) => format!("{:.0} %", f * 100.),
      None => t("Downloading…", "Téléchargement…").to_owned(),
    };
    div()
      .size_full()
      .bg(theme::background())
      .text_color(theme::foreground())
      .flex()
      .flex_col()
      .justify_center()
      .gap_3()
      .px_8()
      .child(div().text_base().child(tf!(
        "Updating Peeby to {version}…",
        "Mise à jour de Peeby vers la {version}…"
      )))
      .child(
        div()
          .h(px(6.))
          .w_full()
          .rounded(px(3.))
          .bg(theme::surface())
          .child(
            div()
              .h_full()
              .rounded(px(3.))
              .bg(theme::accent())
              .w(relative(fraction.unwrap_or(0.))),
          ),
      )
      .child(div().text_sm().text_color(theme::muted()).child(detail))
  }
}
