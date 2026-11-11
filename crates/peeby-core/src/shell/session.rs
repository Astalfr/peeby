//! The windows and their tabs kept from one start to the next (ADR 0035): each window's place
//! (its bounds, maximized or not) and its tabs (their pages up to the one shown, as
//! [`SavedTab`]s), written to `session.json` beside the settings every 30 seconds and when
//! Peeby quits. At startup, with the setting "Restore tabs" on (and nothing asked on the command
//! line), they come back, each page built when its tab shows; a tab whose page plays is not the
//! one shown first, so that nothing plays before a click.

use std::time::Duration;

use gpui::{App, Bounds, DisplayId, Window, WindowBounds, point, px, size};
use serde::{Deserialize, Serialize};

use super::space::{SavedTab, SpaceHandle};
use super::{TabRef, windows};
use crate::settings;

/// How often the session is written while Peeby runs (a crash loses at most this much).
const SAVE_EVERY: Duration = Duration::from_secs(30);

/// Every window in use, in the order they opened.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
  /// The windows; the first one is the main window.
  pub windows: Vec<SavedWindow>,
}

/// A window kept for the next start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedWindow {
  /// Its client area when not maximized (logical pixels, the screen's coordinates): x, y,
  /// width, height.
  pub bounds: [f32; 4],
  /// Maximized (`bounds` are where it goes back to).
  #[serde(default)]
  pub maximized: bool,
  /// The tab shown, among `tabs`.
  #[serde(default)]
  pub active: usize,
  /// Its tabs, in the bar's order.
  pub tabs: Vec<SavedTab>,
}

impl SavedWindow {
  /// Where the window opens: its bounds, maximized or not, on the display they are on (`None`
  /// when that display is gone: the caller opens it at its default place).
  pub fn placement(&self, cx: &App) -> Option<(WindowBounds, Option<DisplayId>)> {
    let [x, y, width, height] = self.bounds;
    let bounds = Bounds::new(point(px(x), px(y)), size(px(width), px(height)));
    let display = cx
      .displays()
      .into_iter()
      .find(|d| d.bounds().contains(&bounds.center()))?
      .id();
    let bounds = if self.maximized {
      WindowBounds::Maximized(bounds)
    } else {
      WindowBounds::Windowed(bounds)
    };
    Some((bounds, Some(display)))
  }

  /// The window's tabs made again in their spaces, and the index of the one to show: the one
  /// that was, unless its page plays and another's does not (the nearest one before it, else
  /// after). `None` when no tab could be made again.
  pub fn restore(&self, spaces: &[SpaceHandle], cx: &mut App) -> Option<RestoredTabs> {
    let mut tabs = Vec::new();
    let mut plays = Vec::new();
    let mut active = 0;
    for (i, saved) in self.tabs.iter().enumerate() {
      let Some(space) = spaces.iter().position(|s| s.info(cx).id == saved.space) else {
        continue;
      };
      let Some(id) = spaces[space].restore_tab(saved, cx) else {
        continue;
      };
      if i == self.active {
        active = tabs.len();
      }
      tabs.push(TabRef { space, id });
      plays.push(saved.plays);
    }
    if tabs.is_empty() {
      return None;
    }
    let active = shown_first(&plays, active);
    Some(RestoredTabs { tabs, active })
  }
}

/// A window's tabs made again ([`SavedWindow::restore`]), for [`super::Shell::restored`] and
/// [`windows::open_restored`].
pub struct RestoredTabs {
  pub(super) tabs: Vec<TabRef>,
  /// The index of the tab shown.
  pub(super) active: usize,
}

/// The tab shown first among tabs whose pages play or not (`plays`), `active` the one that was
/// shown: it, unless it plays and another does not (the nearest before it, else after).
fn shown_first(plays: &[bool], active: usize) -> usize {
  let active = active.min(plays.len().saturating_sub(1));
  if !plays.get(active).copied().unwrap_or(false) {
    return active;
  }
  (0..active)
    .rev()
    .chain(active + 1..plays.len())
    .find(|&i| !plays[i])
    .unwrap_or(active)
}

/// The session as kept, if any (none on the first start, or unreadable).
pub fn load() -> Option<Session> {
  let path = settings::session_path()?;
  let text = std::fs::read_to_string(&path).ok()?;
  match serde_json::from_str::<Session>(&text) {
    Ok(session) => Some(session),
    Err(e) => {
      tracing::warn!("session: {e}");
      None
    }
  }
}

/// Writes the windows in use and their tabs (a window with nothing to keep is left out);
/// nothing when no window is left to keep.
pub fn save(cx: &mut App) {
  save_with(None, cx);
}

/// [`save`] from inside the update of `closing` (the last window, about to close and quit
/// Peeby): its own place is read from it, which the app cannot hand out meanwhile.
pub fn save_closing(closing: &Window, cx: &mut App) {
  save_with(Some(closing), cx);
}

fn save_with(closing: Option<&Window>, cx: &mut App) {
  let mut session = Session::default();
  for target in windows::in_use(cx) {
    let Some(shell) = target.shell.upgrade() else {
      continue;
    };
    let (tabs, active) = {
      let shell = shell.read(cx);
      let tabs: Vec<_> = shell
        .tabs
        .iter()
        .filter_map(|tab| {
          let saved = shell.spaces[tab.space].save_tab(tab.id, cx)?;
          Some((saved, shell.active == Some(*tab)))
        })
        .collect();
      let active = tabs.iter().position(|(_, shown)| *shown).unwrap_or(0);
      (tabs, active)
    };
    if tabs.is_empty() {
      continue;
    }
    let placement =
      match closing.filter(|w| w.window_handle().window_id() == target.handle.window_id()) {
        Some(window) => Some((window.window_bounds().get_bounds(), window.is_maximized())),
        None => target
          .handle
          .update(cx, |_, window, _| {
            (window.window_bounds().get_bounds(), window.is_maximized())
          })
          .ok(),
      };
    let Some((bounds, maximized)) = placement else {
      continue;
    };
    session.windows.push(SavedWindow {
      bounds: [
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
      ],
      maximized,
      active,
      tabs: tabs.into_iter().map(|(saved, _)| saved).collect(),
    });
  }
  // The windows are gone (the last one closing): the session written before them stays.
  if session.windows.is_empty() {
    return;
  }
  let Some(path) = settings::session_path() else {
    return;
  };
  let text = match serde_json::to_string_pretty(&session) {
    Ok(text) => text,
    Err(e) => return tracing::warn!("session: {e}"),
  };
  if let Err(e) = settings::write_atomically(&path, text) {
    tracing::warn!("session: {e}");
  }
}

/// Saves the session every [`SAVE_EVERY`] and when Peeby quits.
pub fn keep_saving(cx: &mut App) {
  cx.on_app_quit(|cx| {
    save(cx);
    async {}
  })
  .detach();
  cx.spawn(async move |cx| {
    loop {
      cx.background_executor().timer(SAVE_EVERY).await;
      cx.update(save);
    }
  })
  .detach();
}

#[cfg(test)]
mod tests {
  use super::shown_first;

  #[test]
  fn a_tab_that_plays_is_not_shown_first_when_another_does_not() {
    // The tab shown did not play: it stays.
    assert_eq!(shown_first(&[true, false, true], 1), 1);
    // It played: the nearest one before it that does not.
    assert_eq!(shown_first(&[false, true, true], 2), 0);
    // None before: the nearest after.
    assert_eq!(shown_first(&[true, true, false], 0), 2);
    // Every tab plays: the one that was shown.
    assert_eq!(shown_first(&[true, true], 1), 1);
    // An index past the end: the last tab.
    assert_eq!(shown_first(&[false, false], 5), 1);
  }

  #[test]
  fn a_session_reads_back_what_it_wrote() {
    use super::super::space::SavedTab;
    use super::{SavedWindow, Session};
    let session = Session {
      windows: vec![SavedWindow {
        bounds: [10., 20., 1440., 900.],
        maximized: true,
        active: 1,
        tabs: vec![SavedTab {
          space: "youtube".into(),
          routes: vec!["home".into(), "watch\u{1f}dQw4w9WgXcQ".into()],
          title: Some("A video".into()),
          plays: true,
        }],
      }],
    };
    let text = serde_json::to_string(&session).unwrap();
    assert_eq!(serde_json::from_str::<Session>(&text).unwrap(), session);
  }
}
