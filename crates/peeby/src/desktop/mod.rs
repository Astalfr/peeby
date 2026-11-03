//! Peeby on the Windows desktop (ADR 0020): its icon in the notification area, the main window
//! hidden there rather than closed, Windows notifications, a single instance that the next
//! launches hand their arguments to, and the start with Windows.

mod autostart;
pub mod sys;
pub mod toast;
mod tray;

use gpui::{AnyWindowHandle, App, Global, WeakEntity, Window};
use raw_window_handle::RawWindowHandle;

pub use autostart::BACKGROUND;

use crate::settings::Settings;
use crate::shell::Shell;

/// The app's icon, written next to the caches for the toasts' registration (Windows reads a
/// file).
const ICON: &[u8] = include_bytes!("../../assets/peeby.ico");

/// What runs in the main window once it shows (a toast's click, a launch's arguments).
pub type InMain = Box<dyn FnOnce(&mut Window, &mut App)>;

/// The main window, hidden in the notification area or shown.
struct MainWindow {
  handle: AnyWindowHandle,
  hwnd: Option<isize>,
  shell: WeakEntity<Shell>,
}

impl Global for MainWindow {}

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

/// Makes `window` (being opened, with `shell`) the main window: its close button hides it in
/// the notification area when the settings say so.
pub fn attach(window: &mut Window, shell: WeakEntity<Shell>, cx: &mut App) {
  let hwnd = match raw_window_handle::HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
    Ok(RawWindowHandle::Win32(handle)) => Some(handle.hwnd.get()),
    _ => None,
  };
  cx.set_global(MainWindow {
    handle: Window::window_handle(window),
    hwnd,
    shell,
  });
  window.on_window_should_close(cx, |_, cx| {
    let tray = cx.try_global::<TrayShown>().is_some_and(|t| t.0);
    if !tray || !Settings::get(cx).close_to_tray {
      return true;
    }
    hide_main(cx);
    false
  });
}

/// Hides the main window in the notification area. Its player pauses: nothing plays where
/// nobody looks (ADR 0019); the mini-player's goes on, on screen.
fn hide_main(cx: &mut App) {
  let Some(main) = cx.try_global::<MainWindow>() else {
    return;
  };
  let (shell, hwnd) = (main.shell.clone(), main.hwnd);
  shell.update(cx, |shell, cx| shell.go_background(cx)).ok();
  if let Some(hwnd) = hwnd {
    sys::hide(hwnd);
  }
}

/// The spaces of the main window's shell.
fn spaces(cx: &App) -> Vec<crate::shell::space::SpaceHandle> {
  cx.try_global::<MainWindow>()
    .and_then(|main| main.shell.upgrade())
    .map(|shell| shell.read(cx).spaces().to_vec())
    .unwrap_or_default()
}

/// Shows the main window again if it was hidden in the notification area. GPUI's
/// `activate_window` brings a window to the front but never shows a hidden one: call this first.
pub fn reveal_main(cx: &App) {
  if let Some(hwnd) = cx.try_global::<MainWindow>().and_then(|main| main.hwnd) {
    sys::show(hwnd);
  }
}

/// Shows the main window and brings it to the front, then runs `then` in it (a toast's click,
/// a launch's arguments).
pub fn show_main(cx: &mut App, then: Option<InMain>) {
  let Some(handle) = cx.try_global::<MainWindow>().map(|main| main.handle) else {
    return;
  };
  reveal_main(cx);
  handle
    .update(cx, |_, window, cx| {
      window.activate_window();
      if let Some(then) = then {
        then(window, cx);
      }
    })
    .ok();
}

/// Hands what the next launches send (their arguments, one a line) to `on_launch`, in the main
/// window shown again. The pipe is read on a thread of its own.
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
