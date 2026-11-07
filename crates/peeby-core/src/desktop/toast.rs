//! Windows notifications (toasts) with an image, and what a click on one does (ADR 0020).
//!
//! GPUI has toasts of its own, without an image: these are built with WinRT directly. Windows
//! calls a toast's `Activated` only while the app holds the toast, so the last ones are kept;
//! the click arrives on another thread and goes through a channel to the UI thread, which shows
//! the main window and runs what the toast was given.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use gpui::{App, Global, Window};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager, ToastNotifier};
use windows::core::{HSTRING, IInspectable, h};

use super::InMain as OnClick;

/// Peeby's AppUserModelID, registered under `HKCU\Software\Classes\AppUserModelId` so that an
/// app without a Start menu shortcut can show toasts, under its name and icon.
/// A debug build has its own, so that Windows lists it apart.
const APP_ID: &str = if cfg!(debug_assertions) {
  "Peeby.PeebyDev"
} else {
  "Peeby.Peeby"
};
/// Toasts kept for their click: Windows' Action Center holds about twenty per app.
const KEPT: usize = 32;

/// A toast: its first line, up to two more, an image file shown round on the left and a
/// preview under the text (PNGs: Windows reads neither a URL nor WebP for an unpackaged app).
pub struct Toast {
  /// The first line, in bold.
  pub title: String,
  /// The lines under it (Windows shows two at most).
  pub body: String,
  /// The local PNG shown on the left, if any.
  pub image: Option<PathBuf>,
  /// The local PNG shown under the text, the toast's width (a video's thumbnail), if any.
  pub preview: Option<PathBuf>,
}

struct Shown {
  id: u64,
  _toast: ToastNotification,
  on_click: Option<OnClick>,
}

struct Toasts {
  notifier: ToastNotifier,
  next: u64,
  shown: VecDeque<Shown>,
  clicks: async_channel::Sender<u64>,
}

impl Global for Toasts {}

/// Starts handling the toasts and their clicks. Called once at startup, [`register`] in the
/// background; the toasts are left out when it fails.
pub fn init(cx: &mut App) {
  let notifier = match notifier() {
    Ok(notifier) => notifier,
    Err(e) => {
      tracing::warn!("Windows notifications unavailable: {e}");
      return;
    }
  };
  let (clicks, received) = async_channel::unbounded();
  cx.set_global(Toasts {
    notifier,
    next: 0,
    shown: VecDeque::new(),
    clicks,
  });
  cx.spawn(async move |cx| {
    while let Ok(id) = received.recv().await {
      cx.update(|cx| clicked(id, cx));
    }
  })
  .detach();
}

fn notifier() -> windows::core::Result<ToastNotifier> {
  ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))
}

/// Shows `toast`; a click shows the main window, then runs `on_click` in it.
pub fn show(toast: Toast, on_click: impl FnOnce(&mut Window, &mut App) + 'static, cx: &mut App) {
  let Some(toasts) = cx.try_global::<Toasts>() else {
    return;
  };
  let id = toasts.next;
  let notification = match build(&toast, id, toasts.clicks.clone()) {
    Ok(notification) => notification,
    Err(e) => {
      tracing::warn!("cannot build a Windows notification: {e}");
      return;
    }
  };
  if let Err(e) = toasts.notifier.Show(&notification) {
    // A notifier kept a while gets disconnected from Windows' notification service
    // (0x800401FD, seen after 25 to 70 minutes): a new one shows it.
    tracing::info!("Windows notification failed ({e}), with a new notifier again");
    let shown = notifier().and_then(|notifier| {
      notifier.Show(&notification)?;
      Ok(notifier)
    });
    match shown {
      Ok(notifier) => cx.global_mut::<Toasts>().notifier = notifier,
      Err(e) => {
        tracing::warn!("cannot show a Windows notification: {e}");
        return;
      }
    }
  }
  let toasts = cx.global_mut::<Toasts>();
  toasts.next += 1;
  toasts.shown.push_back(Shown {
    id,
    _toast: notification,
    on_click: Some(Box::new(on_click)),
  });
  while toasts.shown.len() > KEPT {
    toasts.shown.pop_front();
  }
}

fn clicked(id: u64, cx: &mut App) {
  let on_click = cx.try_global::<Toasts>().is_some().then(|| {
    cx.global_mut::<Toasts>()
      .shown
      .iter_mut()
      .find(|s| s.id == id)
      .and_then(|s| s.on_click.take())
  });
  super::show_main(cx, on_click.flatten());
}

/// The toast's XML (`ToastGeneric`) and its click, which sends `id` on `clicks`.
fn build(
  toast: &Toast,
  id: u64,
  clicks: async_channel::Sender<u64>,
) -> windows::core::Result<ToastNotification> {
  let document = XmlDocument::new()?;
  let root = document.CreateElement(h!("toast"))?;
  document.AppendChild(&root)?;
  let visual = document.CreateElement(h!("visual"))?;
  root.AppendChild(&visual)?;
  let binding = document.CreateElement(h!("binding"))?;
  binding.SetAttribute(h!("template"), h!("ToastGeneric"))?;
  visual.AppendChild(&binding)?;
  for line in [&toast.title, &toast.body] {
    if line.is_empty() {
      continue;
    }
    let text = document.CreateElement(h!("text"))?;
    text.SetInnerText(&HSTRING::from(line.as_str()))?;
    binding.AppendChild(&text)?;
  }
  if let Some(image) = &toast.image {
    let element = document.CreateElement(h!("image"))?;
    element.SetAttribute(h!("placement"), h!("appLogoOverride"))?;
    element.SetAttribute(h!("hint-crop"), h!("circle"))?;
    element.SetAttribute(h!("src"), &HSTRING::from(image.as_os_str()))?;
    binding.AppendChild(&element)?;
  }
  if let Some(preview) = &toast.preview {
    // Without a placement, an image is inline: under the text, as wide as the toast.
    let element = document.CreateElement(h!("image"))?;
    element.SetAttribute(h!("src"), &HSTRING::from(preview.as_os_str()))?;
    binding.AppendChild(&element)?;
  }
  let notification = ToastNotification::CreateToastNotification(&document)?;
  notification.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
    move |_, _| {
      clicks.try_send(id).ok();
      Ok(())
    },
  ))?;
  Ok(notification)
}

/// Names the app id after Peeby, with its icon, for the toasts and Windows' notification
/// settings. Blocking (the registry).
pub fn register(icon: Option<&Path>) {
  let key =
    windows_registry::CURRENT_USER.create(format!(r"Software\Classes\AppUserModelId\{APP_ID}"));
  let result = key.and_then(|key| {
    key.set_string("DisplayName", crate::settings::APP_NAME)?;
    match icon {
      Some(icon) => key.set_string("IconUri", icon.to_string_lossy().as_ref()),
      None => Ok(()),
    }
  });
  if let Err(e) = result {
    tracing::warn!("cannot register Peeby's notifications: {e}");
  }
}
