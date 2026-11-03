//! Peeby's icon in the notification area (ADR 0020): a click opens the list of lives
//! (`shell::lives`), a double click shows the window, its menu opens it or quits. The icon's
//! hidden window gets its messages from GPUI's loop (same thread); its events go through a
//! channel to the UI thread.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, Global};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{
  Icon, MouseButton, MouseButtonState, Rect, TrayIcon, TrayIconBuilder, TrayIconEvent,
};
use ui::i18n::t;

use super::sys;
use crate::shell::lives::{self, Anchor};

/// The icon of peeby.exe's resources (peeby.rc).
const ICON_RESOURCE: u16 = 1;

/// A click this soon after the list closed is the one that closed it (it took the focus from
/// the list): it does not open it again.
const CLOSED_BY_CLICK: Duration = Duration::from_millis(800);

/// A click waits at most this long for a second one (Windows' double click time is 500 ms by
/// default, which makes the list slow to open): a slower double click opens the list for a
/// moment before the window shows.
const CLICK_WAIT: Duration = Duration::from_millis(100);

/// What the icon asks for.
enum Request {
  Show,
  Quit,
  /// A left click, released at that instant, on the icon there.
  Click(Instant, Anchor),
  DoubleClick,
}

/// The icon and its menu items, held for as long as the app runs (dropping it removes it).
struct Tray {
  _icon: TrayIcon,
  open: MenuItem,
  quit: MenuItem,
}

impl Global for Tray {}

/// Puts the icon in the notification area. Without it, closing the window cannot keep Peeby
/// running: `false` then.
pub fn init(cx: &mut App) -> bool {
  let open = MenuItem::new(open_label(), true, None);
  let quit = MenuItem::new(quit_label(), true, None);
  let menu = Menu::new();
  if let Err(e) = menu.append_items(&[&open, &quit]) {
    tracing::warn!("cannot build the notification area menu: {e}");
    return false;
  }
  let icon = match Icon::from_resource(ICON_RESOURCE, None) {
    Ok(icon) => icon,
    Err(e) => {
      tracing::warn!("cannot load the notification area icon: {e}");
      return false;
    }
  };
  let built = TrayIconBuilder::new()
    .with_tooltip(crate::settings::APP_NAME)
    .with_icon(icon)
    .with_menu(Box::new(menu))
    .with_menu_on_left_click(false)
    .build();
  let tray = match built {
    Ok(tray) => tray,
    Err(e) => {
      tracing::warn!("cannot add the notification area icon: {e}");
      return false;
    }
  };

  let (requests, received) = async_channel::unbounded();
  let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
  let sender = requests.clone();
  MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
    let request = if event.id == open_id {
      Request::Show
    } else if event.id == quit_id {
      Request::Quit
    } else {
      return;
    };
    sender.try_send(request).ok();
  }));
  TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
    let request = match event {
      TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        rect,
        ..
      } => Request::Click(Instant::now(), anchor(rect)),
      TrayIconEvent::DoubleClick {
        button: MouseButton::Left,
        ..
      } => Request::DoubleClick,
      _ => return,
    };
    requests.try_send(request).ok();
  }));
  cx.spawn(async move |cx| {
    // Windows sends a double click as a click, the double click, then a release: a click opens
    // the list only once the double click's time has passed without another. Every click and
    // double click bumps the count, which cancels the click waiting.
    let clicks = Rc::new(Cell::new(0_u64));
    let wait = sys::double_click_time().min(CLICK_WAIT);
    // The release that ends a double click is no click of its own.
    let mut after_double = false;
    while let Ok(request) = received.recv().await {
      match request {
        Request::Click(at, anchor) => {
          if std::mem::take(&mut after_double) {
            continue;
          }
          let click = clicks.get() + 1;
          clicks.set(click);
          let clicks = clicks.clone();
          cx.spawn(async move |cx| {
            cx.background_executor().timer(wait).await;
            if clicks.get() == click {
              cx.update(|cx| toggle(at, anchor, cx));
            }
          })
          .detach();
        }
        Request::DoubleClick => {
          after_double = true;
          clicks.set(clicks.get() + 1);
          cx.update(|cx| {
            lives::close(cx);
            super::show_main(cx, None);
          });
        }
        Request::Show => cx.update(|cx| super::show_main(cx, None)),
        Request::Quit => cx.update(|cx| cx.quit()),
      }
    }
  })
  .detach();

  cx.set_global(Tray {
    _icon: tray,
    open,
    quit,
  });
  true
}

/// Opens the list of lives, or closes it, after a click on the icon at `clicked`.
fn toggle(clicked: Instant, anchor: Anchor, cx: &mut App) {
  if lives::is_open(cx) {
    return lives::close(cx);
  }
  let closed_by_click = lives::closed_at(cx)
    .is_some_and(|closed| clicked.saturating_duration_since(closed) < CLOSED_BY_CLICK);
  if !closed_by_click {
    lives::open(&super::spaces(cx), anchor, cx);
  }
}

/// The icon's rectangle, physical pixels.
fn anchor(rect: Rect) -> Anchor {
  Anchor {
    x: rect.position.x as i32,
    y: rect.position.y as i32,
    width: i32::try_from(rect.size.width).unwrap_or(0),
    height: i32::try_from(rect.size.height).unwrap_or(0),
  }
}

/// The menu in the interface's language, after it changed.
pub fn relabel(cx: &App) {
  if let Some(tray) = cx.try_global::<Tray>() {
    tray.open.set_text(open_label());
    tray.quit.set_text(quit_label());
  }
}

fn open_label() -> &'static str {
  t("Open Peeby", "Ouvrir Peeby")
}

fn quit_label() -> &'static str {
  t("Quit", "Fermer")
}
