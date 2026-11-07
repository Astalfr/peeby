//! Peeby's windows (ADR 0028): each one a [`Shell`] over the same spaces, holding some of their
//! tabs. The window used last takes what has no window of its own (a launch, a notification,
//! the notification area's icon), and the spaces' events go to the window that holds their
//! tab.
//!
//! A window is never destroyed while Peeby runs: closed while others are open, its tabs close
//! and it hides, kept as a spare for the next new window. The pages of a tab moved to another
//! window keep subscriptions and inputs bound to the window they were built in (GPUI's
//! `subscribe_in`, gpui-component's inputs, a player page's account reports), which would stop
//! with it.

use gpui::{
  AnyWindowHandle, App, AppContext as _, Bounds, DisplayId, Global, Pixels, Point, TitlebarOptions,
  WeakEntity, Window, WindowBounds, WindowId, WindowOptions, px, size,
};
use gpui_component::{Root, TitleBar};
use ui::theme;

use super::space::{SpaceEvent, SpaceHandle, TabId};
use super::tab_drag::TabDrag;
use super::{Request, Shell, TabRef};
use crate::{desktop, settings};

/// The smallest a window may be.
const MIN_SIZE: (f32, f32) = (960., 600.);
/// A new window opens this far right and down of the one its tab came from.
const CASCADE: f32 = 32.;

/// The spaces, once for the whole app.
pub struct Spaces(pub Vec<SpaceHandle>);

impl Global for Spaces {}

/// A shell's window.
struct ShellWindow {
  handle: AnyWindowHandle,
  /// For hiding and showing it (GPUI hides none).
  hwnd: Option<isize>,
  shell: WeakEntity<Shell>,
  /// Hidden without tabs, for the next new window.
  spare: bool,
}

/// Every shell's window, in the order they opened, and the one used last.
#[derive(Default)]
struct Windows {
  list: Vec<ShellWindow>,
  last: Option<WindowId>,
}

impl Global for Windows {}

impl Windows {
  fn in_use(&self) -> impl Iterator<Item = &ShellWindow> {
    self.list.iter().filter(|w| !w.spare)
  }

  /// The window used last, or the first one in use.
  fn last(&self) -> Option<&ShellWindow> {
    self
      .in_use()
      .find(|w| Some(w.handle.window_id()) == self.last)
      .or_else(|| self.in_use().next())
  }
}

/// A window in use, as handed out.
#[derive(Clone)]
pub struct Target {
  /// GPUI's handle of the window.
  pub handle: AnyWindowHandle,
  /// Its Win32 handle, when GPUI gave one.
  pub hwnd: Option<isize>,
  /// The shell it shows.
  pub shell: WeakEntity<Shell>,
}

impl From<&ShellWindow> for Target {
  fn from(w: &ShellWindow) -> Self {
    Self {
      handle: w.handle,
      hwnd: w.hwnd,
      shell: w.shell.clone(),
    }
  }
}

/// Counts `window`, being opened with `shell`, among Peeby's windows; it is the one used last.
pub fn register(window: &Window, hwnd: Option<isize>, shell: WeakEntity<Shell>, cx: &mut App) {
  let handle = window.window_handle();
  let windows = cx.default_global::<Windows>();
  windows.list.push(ShellWindow {
    handle,
    hwnd,
    shell,
    spare: false,
  });
  windows.last = Some(handle.window_id());
}

/// `window` was activated: it takes what has no window of its own from now on.
pub fn used(window: WindowId, cx: &mut App) {
  cx.default_global::<Windows>().last = Some(window);
}

/// The window used last (shown or hidden in the notification area).
pub fn last(cx: &App) -> Option<Target> {
  cx.try_global::<Windows>()?.last().map(Target::from)
}

/// The window `id`, if it is a shell's.
pub fn get(id: WindowId, cx: &App) -> Option<Target> {
  cx.try_global::<Windows>()?
    .list
    .iter()
    .find(|w| w.handle.window_id() == id)
    .map(Target::from)
}

/// Whether a window other than `id` is in use.
pub fn others_in_use(id: WindowId, cx: &App) -> bool {
  cx.try_global::<Windows>()
    .is_some_and(|w| w.in_use().any(|w| w.handle.window_id() != id))
}

/// Puts window `id` aside, hidden without tabs, for the next new window.
pub fn set_spare(id: WindowId, cx: &mut App) {
  let windows = cx.default_global::<Windows>();
  if let Some(w) = windows.list.iter_mut().find(|w| w.handle.window_id() == id) {
    w.spare = true;
  }
  if windows.last == Some(id) {
    windows.last = None;
  }
}

/// Takes a spare window back into use.
fn take_spare(cx: &mut App) -> Option<Target> {
  let windows = cx.default_global::<Windows>();
  let w = windows.list.iter_mut().find(|w| w.spare)?;
  w.spare = false;
  Some(Target::from(&*w))
}

/// The window in use whose shell holds tab `id`.
fn holding(id: TabId, cx: &App) -> Option<Target> {
  let windows = cx.try_global::<Windows>()?;
  windows
    .in_use()
    .find(|w| w.shell.upgrade().is_some_and(|s| s.read(cx).holds(id)))
    .map(Target::from)
}

/// The window in use, other than those of `except`, whose bar is under `screen` (the screen's
/// coordinates), and the index a tab dropped there takes. `except` must name the window whose
/// shell is being updated (the drag's): reading it would panic.
pub(super) fn bar_at(
  screen: Point<Pixels>,
  except: &[WindowId],
  reach: Pixels,
  cx: &App,
) -> Option<(Target, usize)> {
  let windows = cx.try_global::<Windows>()?;
  windows
    .in_use()
    .filter(|w| !except.contains(&w.handle.window_id()))
    .find_map(|w| {
      let index = w.shell.upgrade()?.read(cx).bar_slot(screen, reach)?;
      Some((Target::from(w), index))
    })
}

/// Tab `tab`, dropped on `target`'s bar, joins it at `index` and shows; that window comes to the
/// front.
pub(super) fn merge(tab: TabRef, target: Target, index: usize, cx: &mut App) {
  let merged = target.handle.update(cx, |_, window, cx| {
    if let Some(hwnd) = target.hwnd {
      desktop::sys::show(hwnd);
    }
    window.activate_window();
    target
      .shell
      .update(cx, |shell, cx| shell.adopt_tab(tab, Some(index), cx))
      .ok();
  });
  if let Err(e) = merged {
    tracing::warn!("cannot move a tab to another window: {e:#}");
  }
}

/// Window `id` lost its last tab to another one: it hides, a spare for the next new window.
pub(super) fn retire(id: WindowId, cx: &mut App) {
  let hwnd = get(id, cx).and_then(|w| w.hwnd);
  set_spare(id, cx);
  if let Some(hwnd) = hwnd {
    desktop::sys::hide(hwnd);
  }
}

/// Runs `f` in the window holding tab `tab` now, after the current update: a page that replaces
/// itself (a queue's next video, the next episode) is built where its tab is, which may not be
/// the window it was built in. The page takes the focus, enters fullscreen there. Nothing when
/// the tab is in no window any more.
pub fn in_window_of(tab: TabId, cx: &mut App, f: impl FnOnce(&mut Window, &mut App) + 'static) {
  cx.defer(move |cx| {
    if let Some(target) = holding(tab, cx) {
      target.handle.update(cx, |_, window, cx| f(window, cx)).ok();
    }
  });
}

/// What a window of Peeby's is opened with: GPUI's title bar replaced by gpui-component's, at
/// `bounds` on `display` (Windows checks them against it, else against the primary display).
pub fn options(
  bounds: Bounds<Pixels>,
  display: Option<DisplayId>,
  show: bool,
  cx: &mut App,
) -> WindowOptions {
  WindowOptions {
    window_bounds: Some(WindowBounds::Windowed(bounds)),
    display_id: display,
    window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
    window_background: theme::window_background(cx),
    show,
    focus: show,
    // GPUI caps an unfocused window at 30 fps by default: video plays while you work elsewhere.
    inactive_frame_interval: None,
    titlebar: Some(TitlebarOptions {
      title: Some(settings::APP_NAME.into()),
      ..TitleBar::title_bar_options()
    }),
    ..TitleBar::window_options()
  }
}

/// Where a tab taken from `window` opens: a window of the same size, a little right and down.
pub fn cascade(window: &Window, cx: &App) -> (Bounds<Pixels>, Option<DisplayId>) {
  let bounds = window.window_bounds().get_bounds();
  let display = window.display(cx).map(|d| d.id());
  (
    Bounds::new(
      bounds.origin + gpui::point(px(CASCADE), px(CASCADE)),
      bounds.size,
    ),
    display,
  )
}

/// Opens `tab` in a window of its own at `bounds`: a spare one if there is, else a new one.
///
/// With `focus`, the window comes to the front; without (a tab torn out and still dragged), it
/// shows behind the pointer without the focus, which the window the drag started in keeps, with
/// the mouse's capture. Its handle, to move it.
pub(super) fn open_with(
  tab: TabRef,
  bounds: Bounds<Pixels>,
  display: Option<DisplayId>,
  focus: bool,
  cx: &mut App,
) -> Option<AnyWindowHandle> {
  if let Some(spare) = take_spare(cx) {
    let reused = spare.handle.update(cx, |_, window, cx| {
      place(
        spare.handle.window_id(),
        bounds,
        window.scale_factor(),
        true,
        cx,
      );
      if let Some(hwnd) = spare.hwnd {
        if focus {
          desktop::sys::show(hwnd);
        } else {
          desktop::sys::show_inactive(hwnd);
        }
      }
      if focus {
        window.activate_window();
      }
      spare
        .shell
        .update(cx, |shell, cx| shell.adopt_tab(tab, None, cx))
        .ok();
    });
    if let Err(e) = reused {
      tracing::warn!("cannot reuse a window: {e:#}");
      return None;
    }
    if !focus && let Some(hwnd) = spare.hwnd {
      desktop::sys::raise(hwnd);
    }
    return Some(spare.handle);
  }
  let spaces = cx.try_global::<Spaces>().map(|s| s.0.clone())?;
  let mut options = options(bounds, display, true, cx);
  options.focus = focus;
  let opened = cx.open_window(options, |window, cx| {
    let shell = cx.new(|cx| Shell::with_tabs(spaces, Some(vec![tab]), false, window, cx));
    desktop::attach(window, shell.downgrade(), cx);
    cx.new(|cx| Root::new(shell, window, cx))
  });
  match opened {
    Ok(handle) => {
      let handle: AnyWindowHandle = handle.into();
      if focus {
        // In front with the focus, whatever window the click that opened it was in.
        self::focus(handle, cx);
      } else if let Some(hwnd) = get(handle.window_id(), cx).and_then(|w| w.hwnd) {
        // Shown without the focus, it would stay under the window the drag started in.
        desktop::sys::raise(hwnd);
      }
      Some(handle)
    }
    Err(e) => {
      tracing::error!("cannot open a window: {e:#}");
      None
    }
  }
}

/// Places window `id`'s client area (what GPUI's bounds measure) at `bounds` (logical pixels,
/// the screen's coordinates, at that window's `scale`), or only moves it there without `resize`.
/// As a task: `SetWindowPos` sends its messages at once (see `desktop::sys::set_client_bounds`).
/// GPUI's own move (`ui::set_window_bounds`) places the whole window, its frame included, which
/// would shrink or move a framed window by its borders at each call.
pub(super) fn place(id: WindowId, bounds: Bounds<Pixels>, scale: f32, resize: bool, cx: &App) {
  let Some(hwnd) = get(id, cx).and_then(|w| w.hwnd) else {
    return;
  };
  let device = |v: Pixels| (f32::from(v) * scale).round() as i32;
  let origin = (device(bounds.origin.x), device(bounds.origin.y));
  let size = resize.then(|| (device(bounds.size.width), device(bounds.size.height)));
  cx.foreground_executor()
    .spawn(async move { desktop::sys::set_client_bounds(hwnd, origin, size) })
    .detach();
}

/// A tab torn out and still held: it opens in a window of its own at `bounds` (a spare one if
/// there is), raised without the focus, which takes the drag and the mouse's capture from the
/// window it left. `false` when no window could open.
pub(super) fn tear(
  tab: TabRef,
  bounds: Bounds<Pixels>,
  display: Option<DisplayId>,
  drag: TabDrag,
  screen: Point<Pixels>,
  cx: &mut App,
) -> bool {
  let Some(handle) = open_with(tab, bounds, display, false, cx) else {
    return false;
  };
  if let Some(target) = get(handle.window_id(), cx) {
    target
      .shell
      .update(cx, |shell, cx| {
        shell.take_drag(drag, screen, Some(bounds.origin), cx)
      })
      .ok();
    if let Some(hwnd) = target.hwnd {
      desktop::sys::capture(hwnd);
    }
  }
  true
}

/// A tab still held slides into `target`'s bar at `index` (at the end when `None`): it shows
/// there, that window comes above the others (without the focus, until the release) and takes
/// the drag and the mouse's capture.
pub(super) fn hand_over(
  target: Target,
  tab: TabRef,
  index: Option<usize>,
  drag: TabDrag,
  screen: Point<Pixels>,
  cx: &mut App,
) {
  target
    .shell
    .update(cx, |shell, cx| {
      shell.adopt_tab(tab, index, cx);
      shell.take_drag(drag, screen, None, cx);
    })
    .ok();
  if let Some(hwnd) = target.hwnd {
    desktop::sys::raise(hwnd);
    desktop::sys::capture(hwnd);
  }
}

/// [`hand_over`] to window `id`, at the end of its bar.
pub(super) fn hand_over_to(
  id: WindowId,
  tab: TabRef,
  drag: TabDrag,
  screen: Point<Pixels>,
  cx: &mut App,
) {
  if let Some(target) = get(id, cx) {
    hand_over(target, tab, None, drag, screen, cx);
  }
}

/// A window comes to the front and takes the focus.
pub(super) fn focus(handle: AnyWindowHandle, cx: &mut App) {
  if let Some(hwnd) = get(handle.window_id(), cx).and_then(|w| w.hwnd) {
    desktop::sys::raise(hwnd);
  }
  handle
    .update(cx, |_, window, _| window.activate_window())
    .ok();
}

/// The tab whose window takes `request`: the one to show, the one a tab was opened from.
/// `None`: the window used last.
fn tab_of(request: &Request) -> Option<TabId> {
  match *request {
    Request::Activate(tab) => Some(tab),
    Request::Add { after, .. } => after,
    Request::Settings { .. } => None,
  }
}

/// Hands the spaces' events to the windows, once for the app (each shell subscribing would run
/// them once per window).
pub fn route_events(spaces: &[SpaceHandle], cx: &mut App) {
  for (index, space) in spaces.iter().enumerate() {
    space
      .on_event(
        cx,
        Box::new(move |event, cx| {
          let request = match *event {
            SpaceEvent::Activate(tab) => Request::Activate(tab),
            SpaceEvent::Opened { tab, after, focus } => Request::Add { tab, after, focus },
            SpaceEvent::OpenSettings { reveal } => Request::Settings { reveal },
          };
          // Deferred: the event comes from inside a window (a click), which cannot be updated
          // again from there.
          cx.defer(move |cx| deliver(index, request, cx));
        }),
      )
      .detach();
  }
}

fn deliver(space: usize, request: Request, cx: &mut App) {
  let target = tab_of(&request)
    .and_then(|tab| holding(tab, cx))
    .or_else(|| last(cx));
  let Some(target) = target else {
    return;
  };
  let activate = matches!(request, Request::Activate(_));
  target
    .handle
    .update(cx, |_, window, cx| {
      // A notification of a tab in another window: that window comes to the front.
      if activate && !window.is_window_active() {
        if let Some(hwnd) = target.hwnd {
          desktop::sys::show(hwnd);
        }
        window.activate_window();
      }
      target
        .shell
        .update(cx, |shell, cx| shell.handle(space, request, window, cx))
        .ok();
    })
    .ok();
}

#[cfg(test)]
mod tests {
  use super::super::Request;
  use super::super::space::TabId;
  use super::tab_of;

  #[test]
  fn a_request_goes_to_the_window_of_its_tab_or_to_the_last_one() {
    let (a, b) = (TabId::for_test(1), TabId::for_test(2));
    assert_eq!(tab_of(&Request::Activate(a)), Some(a));
    let opened = |after| Request::Add {
      tab: b,
      after,
      focus: false,
    };
    assert_eq!(tab_of(&opened(Some(a))), Some(a));
    assert_eq!(tab_of(&opened(None)), None);
    assert_eq!(tab_of(&Request::Settings { reveal: true }), None);
  }
}
