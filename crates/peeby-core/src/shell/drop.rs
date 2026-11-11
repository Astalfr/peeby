//! Opening what is dropped on a window: a link dragged from a browser (a video, a live, a VOD,
//! a clip), or an Internet shortcut (`.url`) dragged from the desktop or the Explorer. Both go
//! where a next launch's arguments go (`peeby <url>`, ADR 0020): the opener `main` sets, which
//! knows the spaces' links.
//!
//! GPUI's Windows drop target takes files only; the vendored patch hands a dragged text over as
//! the drop's only "path" (vendor/README.md).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, ExternalPaths, Global, Window};

/// Opens a launch's arguments in a window (`main`'s spaces).
pub type OpenFn = dyn Fn(Vec<String>, &mut Window, &mut App);

struct Opener(Rc<OpenFn>);

impl Global for Opener {}

/// Sets what opens the links dropped on a window.
pub fn set_opener(open: Rc<OpenFn>, cx: &mut App) {
  cx.set_global(Opener(open));
}

/// The longest Internet shortcut read: they hold a few lines.
const MAX_SHORTCUT: u64 = 16 << 10;

/// Opens what `paths` hold, in `window`. Shortcuts are read in the background.
pub fn open(paths: &ExternalPaths, window: &mut Window, cx: &mut App) {
  let Some(opener) = cx.try_global::<Opener>().map(|o| o.0.clone()) else {
    return;
  };
  let mut links: Vec<String> = Vec::new();
  let mut shortcuts: Vec<PathBuf> = Vec::new();
  for path in paths.paths() {
    let text = path.to_string_lossy();
    if is_link(&text) {
      links.extend(
        text
          .split_whitespace()
          .filter(|w| is_link(w))
          .map(str::to_owned),
      );
    } else if path
      .extension()
      .is_some_and(|e| e.eq_ignore_ascii_case("url"))
    {
      shortcuts.push(path.clone());
    }
  }
  if shortcuts.is_empty() {
    if !links.is_empty() {
      opener(links, window, cx);
    }
    return;
  }
  let handle = window.window_handle();
  let read = cx.background_executor().spawn(async move {
    shortcuts
      .iter()
      .filter_map(|path| shortcut_url(path))
      .collect::<Vec<_>>()
  });
  cx.spawn(async move |cx| {
    links.extend(read.await);
    if links.is_empty() {
      return;
    }
    handle
      .update(cx, |_, window, cx| opener(links, window, cx))
      .ok();
  })
  .detach();
}

/// Whether `text` is a web link.
fn is_link(text: &str) -> bool {
  let lower = text.get(..8).unwrap_or(text).to_ascii_lowercase();
  lower.starts_with("https://") || lower.starts_with("http://")
}

/// The `URL=` of an Internet shortcut (`[InternetShortcut]`). Blocking.
fn shortcut_url(path: &Path) -> Option<String> {
  if std::fs::metadata(path).ok()?.len() > MAX_SHORTCUT {
    return None;
  }
  let text = std::fs::read_to_string(path).ok()?;
  parse_shortcut(&text)
}

fn parse_shortcut(text: &str) -> Option<String> {
  text
    .lines()
    .find_map(|line| line.trim().strip_prefix("URL="))
    .map(str::trim)
    .filter(|url| is_link(url))
    .map(str::to_owned)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn links_and_shortcuts_are_read() {
    assert!(is_link("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
    assert!(is_link("HTTP://twitch.tv/x"));
    assert!(!is_link(r"C:\Users\me\video.mkv"));
    assert!(!is_link("youtube"));
    let shortcut = "[InternetShortcut]\r\nURL=https://www.twitch.tv/videos/123\r\nIDList=\r\n";
    assert_eq!(
      parse_shortcut(shortcut).as_deref(),
      Some("https://www.twitch.tv/videos/123")
    );
    assert_eq!(
      parse_shortcut("[InternetShortcut]\nURL=file:///C:/x\n"),
      None
    );
  }
}
