//! The account's notification center (twitch.tv's bell: watch streaks, drops, Twitch's news),
//! read with the auth-token every 3 minutes ([`twitch::notifications`]). It shows in the Twitch
//! tab of Peeby's bell with the followed lives' events ([`super::bell`]), newest first, and as
//! Windows notifications (`[notifications] twitch_inbox`, ADR 0020). Reading the list writes
//! nothing on Twitch: Peeby's "seen" is its own, after twitch.tv's (`lastSeenAt`), so twitch.tv's
//! count stays as it is. The first reading only learns, as the followed lives' bell does.

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, WeakEntity};
use twitch::notifications::{Notification, Page};
use ui::i18n::t;
use ui::images;
use ui::picture::{Picture, Size};
use ui::tf;

use super::{Route, TwitchSpace};
use peeby_core::desktop::toast::{self, Toast};
use peeby_core::settings::{Delivery, Settings};

/// How often the list is read.
pub const EVERY: Duration = Duration::from_secs(180);
/// A notification made this long before Peeby started is not announced (a late one in the list).
const GRACE_MS: i64 = 60_000;
/// Beyond this many Windows notifications from one reading, a single one sums them up.
const MAX_TOASTS: usize = 3;
/// The picture's size in a Windows notification, pixels.
const TOAST_IMAGE: u32 = 96;

/// Where the notification center's notifications show.
pub fn delivery(cx: &App) -> Delivery {
  Settings::get(cx).notifications.twitch_inbox
}

/// The list as last read, and what Peeby has seen of it.
#[derive(Default)]
pub struct Inbox {
  /// The notifications, newest first.
  notifications: Vec<Notification>,
  /// The ids read so far; `None` before the first reading.
  known: Option<HashSet<String>>,
  /// twitch.tv's last look at its bell, milliseconds since the epoch.
  twitch_seen_at: Option<i64>,
  /// Peeby's last look (its panel opened), milliseconds since the epoch.
  seen_at: Option<i64>,
}

impl Inbox {
  /// A new reading at `now` (milliseconds since the epoch): the notifications not read before
  /// and made since Peeby started (`started`), none on the first reading.
  pub fn update(&mut self, page: Page, started: i64) -> Vec<Notification> {
    let fresh = match &self.known {
      None => Vec::new(),
      Some(known) => page
        .notifications
        .iter()
        .filter(|n| !known.contains(&n.id))
        .filter(|n| n.created_at.is_some_and(|at| at >= started - GRACE_MS))
        .cloned()
        .collect(),
    };
    let known = self.known.get_or_insert_with(HashSet::new);
    known.extend(page.notifications.iter().map(|n| n.id.clone()));
    self.notifications = page.notifications;
    self.twitch_seen_at = page.last_seen_at;
    fresh
  }

  /// When the list was last looked at, on twitch.tv or in Peeby.
  fn seen_at(&self) -> Option<i64> {
    self.twitch_seen_at.max(self.seen_at)
  }

  /// Whether `notification` came after the last look.
  pub fn is_new(&self, notification: &Notification) -> bool {
    match (notification.created_at, self.seen_at()) {
      (Some(at), Some(seen)) => at > seen,
      // Never looked at: twitch.tv's count would have them all.
      (Some(_), None) => !notification.read,
      (None, _) => false,
    }
  }

  /// The notifications made after the last look.
  pub fn unseen(&self) -> usize {
    self.notifications.iter().filter(|n| self.is_new(n)).count()
  }

  /// The panel showed them, at `now` (milliseconds since the epoch).
  pub fn see(&mut self, now: i64) {
    let newest = self.notifications.iter().filter_map(|n| n.created_at).max();
    self.seen_at = Some(newest.map_or(now, |n| n.max(now)));
  }

  /// The notifications, newest first.
  pub fn notifications(&self) -> &[Notification] {
    &self.notifications
  }

  /// The newest one, for the settings' test.
  pub fn newest(&self) -> Option<&Notification> {
    self.notifications.first()
  }

  /// The list is forgotten (signed out, another account).
  pub fn clear(&mut self) {
    *self = Self::default();
  }
}

/// Now, milliseconds since the epoch.
pub fn now_ms() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// A notification's time as a `SystemTime`, `UNIX_EPOCH` when it has none.
pub fn created(notification: &Notification) -> SystemTime {
  let millis = notification
    .created_at
    .and_then(|ms| u64::try_from(ms).ok())
    .unwrap_or(0);
  UNIX_EPOCH + Duration::from_millis(millis)
}

/// The notification's picture: a channel's avatar (asked at 150 px, Twitch keeps 70 and 150),
/// a drop's box, a reward.
pub fn picture(notification: &Notification) -> Option<Picture> {
  let url = notification.thumbnail.as_deref()?;
  Some(Picture::Sizes(vec![Size {
    url: url.replace("-70x70.", "-150x150."),
    width: 0,
  }]))
}

/// Whether the picture is a channel's avatar (drawn round), else Twitch's art (drawn square).
pub fn is_avatar(notification: &Notification) -> bool {
  notification
    .thumbnail
    .as_deref()
    .is_some_and(|u| u.contains("profile_image"))
}

/// twitch.tv paths that are pages of the site, not channels.
const SITE_PAGES: &[&str] = &[
  "directory",
  "downloads",
  "drops",
  "friends",
  "inventory",
  "jobs",
  "messages",
  "p",
  "payments",
  "prime",
  "search",
  "settings",
  "store",
  "subscriptions",
  "turbo",
  "wallet",
];

/// Where a click on `url` goes in Peeby: a channel's live, a VOD, a clip. Anything else (a drop
/// campaign, a streak to save, Twitch's blog) opens in the browser.
pub fn route(url: &str) -> Option<Route> {
  let (host, rest) = url.split_once("://")?.1.split_once('/')?;
  if !(host == "twitch.tv" || host.ends_with(".twitch.tv")) {
    return None;
  }
  let path: Vec<&str> = rest
    .split(['?', '#'])
    .next()?
    .split('/')
    .filter(|p| !p.is_empty())
    .collect();
  match path.as_slice() {
    ["videos", _] | [_, "clip", _] => Route::from_url(url),
    [_] if host == "clips.twitch.tv" => Route::from_url(url),
    [login] if !SITE_PAGES.contains(&login.to_ascii_lowercase().as_str()) => Route::from_url(url),
    _ => None,
  }
}

/// Opens `notification`'s page: in Peeby when it is a live, a VOD or a clip, else in the browser.
pub fn open(nav: &WeakEntity<TwitchSpace>, notification: &Notification, cx: &mut App) {
  let Some(url) = notification.url.clone() else {
    return;
  };
  match route(&url) {
    Some(route) => {
      nav
        .update(cx, |space, cx| space.open_from_notification(route, cx))
        .ok();
    }
    None => cx.open_url(&url),
  }
}

/// Shows `fresh` (those of a reading) as Windows notifications when their kind asks for them;
/// beyond [`MAX_TOASTS`], one for them all, a click showing Peeby.
pub fn toasts(fresh: Vec<Notification>, nav: WeakEntity<TwitchSpace>, cx: &mut App) {
  if fresh.is_empty() || !delivery(cx).windows {
    return;
  }
  if fresh.len() > MAX_TOASTS {
    let toast = Toast {
      title: tf!(
        "{} new Twitch notifications",
        "{} nouvelles notifications Twitch",
        fresh.len()
      ),
      body: fresh.first().map(Notification::text).unwrap_or_default(),
      image: None,
      preview: None,
    };
    toast::show(toast, |_, _| {}, cx);
    return;
  }
  for notification in fresh {
    let nav = nav.clone();
    let clicked = notification.clone();
    show_toast(&notification, move |_, cx| open(&nav, &clicked, cx), cx);
  }
}

/// Shows `notification` as a Windows notification, with its picture once on disk.
fn show_toast(
  notification: &Notification,
  on_click: impl FnOnce(&mut gpui::Window, &mut App) + 'static,
  cx: &mut App,
) {
  let image = picture(notification)
    .and_then(|p| p.url(TOAST_IMAGE, TOAST_IMAGE))
    .map(|url| images::local_png(&url, (TOAST_IMAGE, TOAST_IMAGE), cx));
  let body = notification.text();
  cx.spawn(async move |cx| {
    let image = match image {
      Some(image) => image.await,
      None => None,
    };
    cx.update(|cx| {
      toast::show(
        Toast {
          title: t("Twitch", "Twitch").to_owned(),
          body,
          image,
          preview: None,
        },
        on_click,
        cx,
      )
    });
  })
  .detach();
}

/// The settings' test: `sample` shown as a real notification, whatever the settings say; a
/// click only shows Peeby.
pub fn test_toast(sample: &Notification, cx: &mut App) {
  show_toast(sample, |_, _| {}, cx);
}

#[cfg(test)]
mod tests {
  use twitch::notifications::Span;

  use super::*;

  fn notification(id: &str, at: i64) -> Notification {
    Notification {
      id: id.into(),
      spans: vec![Span {
        text: format!("text {id}"),
        bold: false,
      }],
      kind: String::new(),
      category: String::new(),
      read: false,
      created_at: Some(at),
      thumbnail: None,
      url: None,
    }
  }

  fn page(notifications: Vec<Notification>, last_seen_at: Option<i64>) -> Page {
    Page {
      notifications,
      unseen: 0,
      last_seen_at,
    }
  }

  #[test]
  fn the_first_reading_only_learns() {
    let mut inbox = Inbox::default();
    let fresh = inbox.update(page(vec![notification("a", 1_000)], Some(500)), 0);
    assert!(fresh.is_empty());
    // Made after twitch.tv's last look: unseen.
    assert_eq!(inbox.unseen(), 1);
  }

  #[test]
  fn new_ones_are_announced_once_and_seen_by_the_panel() {
    let mut inbox = Inbox::default();
    let started = 10_000_000;
    inbox.update(page(vec![notification("a", 1_000)], Some(2_000)), started);
    assert_eq!(inbox.unseen(), 0);
    let list = vec![notification("b", started + 5), notification("a", 1_000)];
    let fresh = inbox.update(page(list.clone(), Some(2_000)), started);
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].id, "b");
    assert_eq!(inbox.unseen(), 1);
    // Read again: nothing new.
    assert!(inbox.update(page(list, Some(2_000)), started).is_empty());
    inbox.see(started + 10);
    assert_eq!(inbox.unseen(), 0);
  }

  #[test]
  fn an_old_one_showing_up_late_is_not_announced() {
    let mut inbox = Inbox::default();
    let started = 10_000_000;
    inbox.update(page(Vec::new(), None), started);
    let fresh = inbox.update(page(vec![notification("old", 1_000)], None), started);
    assert!(fresh.is_empty());
  }

  #[test]
  fn clicks_open_lives_videos_and_clips_in_peeby_and_the_rest_in_the_browser() {
    assert_eq!(
      route("https://www.twitch.tv/channelone"),
      Some(Route::Watch("channelone".into()))
    );
    assert!(matches!(
      route("https://www.twitch.tv/videos/2915878235"),
      Some(Route::Vod { .. })
    ));
    assert!(matches!(
      route("https://clips.twitch.tv/SomeSlug"),
      Some(Route::Clip(_))
    ));
    assert_eq!(route("https://www.twitch.tv/save-streak/channelone"), None);
    assert_eq!(
      route("https://www.twitch.tv/drops/campaigns?dropID=x"),
      None
    );
    assert_eq!(route("https://www.twitch.tv/inventory"), None);
    assert_eq!(route("https://blog.twitch.tv/2026/10/20/sub/"), None);
    assert_eq!(route("https://thestreamerawards.com"), None);
  }
}
