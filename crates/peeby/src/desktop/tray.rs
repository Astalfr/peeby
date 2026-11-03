//! Peeby's icon in the notification area (ADR 0020): a click shows the window, its menu opens
//! it or quits. The icon's hidden window gets its messages from GPUI's loop (same thread); its
//! events go through a channel to the UI thread.

use gpui::{App, Global};
use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use ui::i18n::t;

/// The icon of peeby.exe's resources (peeby.rc).
const ICON_RESOURCE: u16 = 1;

/// What the icon asks for.
enum Request {
  Show,
  Quit,
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
    .with_tooltip("Peeby")
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
    if let TrayIconEvent::Click {
      button: MouseButton::Left,
      button_state: MouseButtonState::Up,
      ..
    } = event
    {
      requests.try_send(Request::Show).ok();
    }
  }));
  cx.spawn(async move |cx| {
    while let Ok(request) = received.recv().await {
      cx.update(|cx| match request {
        Request::Show => super::show_main(cx, None),
        Request::Quit => cx.quit(),
      });
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
