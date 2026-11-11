//! Peeby on the Windows desktop (ADR 0020): its icon in the notification area, the last window
//! hidden there rather than closed (the others close, ADR 0028), Windows notifications, a single
//! instance that the next launches hand their arguments to, and the start with Windows.

mod autostart;
pub mod sys;
pub mod toast;
mod tray;

use gpui::{App, Global, WeakEntity, Window, WindowId};
use raw_window_handle::RawWindowHandle;

pub use autostart::BACKGROUND;

use crate::settings::Settings;
use crate::shell::{Shell, windows};

/// The app's icon, written next to the caches for the toasts' registration (Windows reads a
/// file).
const ICON: &[u8] = include_bytes!("../../../peeby/assets/peeby.ico");

/// What runs in the window used last once it shows (a toast's click, a launch's arguments).
pub type InMain = Box<dyn FnOnce(&mut Window, &mut App)>;

/// Whether the icon is in the notification area: without it, closing the window quits.
struct TrayShown(bool);

impl Global for TrayShown {}

/// Starts the integration, before the main window opens: the toasts, the icon, and in the
/// background the toasts' registration and the `Run` key.
pub fn init(cx: &mut App) {
  toast::init(cx);
  let shown = tray::init(cx);
  cx.set_global(TrayShown(shown));
  let start_with_windows = Settings::get(cx).start_with_windows;
  cx.background_executor()
    .spawn(async move {
      let icon = dirs::cache_dir().map(|d| d.join("peeby").join("peeby.ico"));
      let icon = icon.filter(|path| {
        path
          .parent()
          .is_some_and(|d| std::fs::create_dir_all(d).is_ok())
          && std::fs::write(path, ICON).is_ok()
      });
      toast::register(icon.as_deref());
      autostart::set(start_with_windows);
    })
    .detach();
  // The menu follows the interface's language.
  cx.observe_global::<Settings>(|cx| tray::relabel(cx))
    .detach();
}

/// The setting changed: the `Run` key follows, in the background.
pub fn set_start_with_windows(on: bool, cx: &mut App) {
  Settings::update(cx, |s| s.start_with_windows = on);
  cx.background_executor()
    .spawn(async move { autostart::set(on) })
    .detach();
}

/// Counts `window` (being opened, with `shell`) among Peeby's windows. Its close button closes
/// it with its tabs while another window is open (it stays as a spare, hidden: ADR 0028); the
/// last one hides in the notification area when the settings say so, else Peeby quits.
pub fn attach(window: &mut Window, shell: WeakEntity<Shell>, cx: &mut App) {
  let hwnd = match raw_window_handle::HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
    Ok(RawWindowHandle::Win32(handle)) => Some(handle.hwnd.get()),
    _ => None,
  };
  windows::register(window, hwnd, shell.clone(), cx);
  let id = Window::window_handle(window).window_id();
  window.on_window_should_close(cx, move |window, cx| {
    if windows::others_in_use(id, cx) {
      shell.update(cx, |shell, cx| shell.close_all(cx)).ok();
      windows::set_spare(id, cx);
      if let Some(hwnd) = hwnd {
        sys::hide(hwnd);
      }
      return false;
    }
    let tray = cx.try_global::<TrayShown>().is_some_and(|t| t.0);
    if !tray || !Settings::get(cx).close_to_tray {
      // Peeby quits once the window closes: its tabs are kept while it still holds them.
      crate::shell::session::save_closing(window, cx);
      return true;
    }
    hide_last(cx);
    false
  });
}

/// Hides the last window in the notification area. Its player pauses: nothing plays where
/// nobody looks (ADR 0019); the mini-player's goes on, on screen.
fn hide_last(cx: &mut App) {
  let Some(last) = windows::last(cx) else {
    return;
  };
  last
    .shell
    .update(cx, |shell, cx| shell.go_background(cx))
    .ok();
  if let Some(hwnd) = last.hwnd {
    sys::hide(hwnd);
  }
}

/// The spaces, for the notification area's list of lives.
fn spaces(cx: &App) -> Vec<crate::shell::space::SpaceHandle> {
  cx.try_global::<windows::Spaces>()
    .map(|spaces| spaces.0.clone())
    .unwrap_or_default()
}

/// Shows window `id` again if it was hidden in the notification area. GPUI's `activate_window`
/// brings a window to the front but never shows a hidden one: call this first.
pub fn reveal(id: WindowId, cx: &App) {
  if let Some(hwnd) = windows::get(id, cx).and_then(|w| w.hwnd) {
    sys::show(hwnd);
  }
}

/// Shows the window used last and brings it to the front, then runs `then` in it (a toast's
/// click, a launch's arguments).
pub fn show_main(cx: &mut App, then: Option<InMain>) {
  let Some(last) = windows::last(cx) else {
    return;
  };
  if let Some(hwnd) = last.hwnd {
    sys::show(hwnd);
  }
  let handle = last.handle;
  handle
    .update(cx, |_, window, cx| {
      window.activate_window();
      if let Some(then) = then {
        then(window, cx);
      }
    })
    .ok();
}

/// Hands what the next launches send (their arguments, one a line) to `on_launch`, in the
/// window used last, shown again. The pipe is read on a thread of its own.
pub fn listen(
  listener: sys::Listener,
  on_launch: impl Fn(Vec<String>, &mut Window, &mut App) + 'static,
  cx: &mut App,
) {
  let (sender, received) = async_channel::unbounded();
  let spawned = std::thread::Builder::new()
    .name("peeby-instance".into())
    .spawn(move || {
      listener.listen(move |message| {
        sender.try_send(message).ok();
      })
    });
  if let Err(e) = spawned {
    tracing::warn!("cannot listen to the next launches: {e}");
    return;
  }
  let on_launch = std::rc::Rc::new(on_launch);
  cx.spawn(async move |cx| {
    while let Ok(message) = received.recv().await {
      let args: Vec<String> = message
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
      let on_launch = on_launch.clone();
      cx.update(|cx| {
        show_main(
          cx,
          Some(Box::new(move |window, cx| on_launch(args, window, cx))),
        )
      });
    }
  })
  .detach();
}
