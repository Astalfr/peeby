//! The notifications inbox: a bell in the title bar with the unseen count, opening the list
//! of notifications in one list, newest first (YouTube's sections, "Important" then the
//! others, are merged), each opening its video.
//!
//! Peeby's bell counts the notifications not seen in Peeby: YouTube's unseen count at startup
//! and on every session change, then the new ones. The count is read every [`POLL`]; when it
//! grows, the list's first page tells what arrived since the session started (a notification's
//! id is its time, in microseconds), shown as Windows notifications for the kinds that ask for
//! them (`[notifications]`, ADR 0020). Reading the list sees the notifications on youtube.com
//! too (its badge clears a few seconds later, checked 2026-11-03): it is read only when the
//! count grows.
//!
//! The list is loaded each time the panel opens, without the kinds left out of the bell, its
//! next page when the end of the list comes near; each page is merged in by date (a
//! notification's id). Opening the panel marks the unread notifications it loads read on
//! YouTube (one request each, as clicks on youtube.com); their dot still shows until it closes,
//! telling what was new.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use gpui::prelude::*;
use gpui::{
  AnyElement, App, Context, ElementId, Entity, Global, ScrollHandle, SharedString, WeakEntity,
  Window, div, px,
};
use gpui_component::popover::PopoverState;
use gpui_component::skeleton::Skeleton;
use gpui_component::{h_flex, v_flex};
use innertube::{Error, Notification, NotificationKind, NotificationPage, Thumbnail};

use crate::desktop::toast::{self, Toast};
use crate::settings::{Delivery, Settings};
use crate::spaces::youtube::convert::{self, error_text};
use crate::spaces::youtube::save::Session;
use crate::spaces::youtube::{Route, YouTubeSpace};
use ui::card::WIDE;
use ui::i18n::t;
use ui::images::{self, Images};
use ui::picture::image_fill;
use ui::widgets::{loading_more, near_scroll_end};
use ui::{runtime, tf, theme};

/// How often the unseen count is read again.
const POLL: Duration = Duration::from_secs(3 * 60);
/// Notifications a little older than the session still count as new: YouTube dates them from
/// the upload, a moment before they reach the inbox.
const SLACK: Duration = Duration::from_secs(60);
/// Beyond this many Windows notifications from one reading, a single one sums them up.
const MAX_TOASTS: usize = 3;
/// The avatar's size in a Windows notification, pixels.
const TOAST_AVATAR: u32 = 96;
const AVATAR: f32 = 40.;
const THUMB_WIDTH: f32 = 86.;

/// The unseen count. `generation` tells apart the sessions, so the polling of an old one
/// stops and its late answers are dropped.
#[derive(Default)]
pub struct Inbox {
  unseen: u32,
  generation: u64,
  /// The ids of the session's notifications already counted or shown in the panel: never
  /// announced (again).
  seen: HashSet<String>,
}

/// Where notifications of `kind` show; `None` for the kinds Peeby has no setting for (they
/// show in the bell, never in Windows').
fn delivery(kind: NotificationKind, cx: &App) -> Option<Delivery> {
  let notifications = &Settings::get(cx).notifications;
  match kind {
    NotificationKind::Upload => Some(notifications.youtube_video),
    NotificationKind::Live => Some(notifications.youtube_live),
    NotificationKind::Other => None,
  }
}

fn in_bell(kind: NotificationKind, cx: &App) -> bool {
  delivery(kind, cx).is_none_or(|d| d.in_app)
}

/// What the polling of a session remembers.
struct Poll {
  /// The last unseen count; `None` before the first.
  count: Option<u32>,
  /// Older notifications were there before the session: not new.
  since_micros: u64,
}

impl Poll {
  fn new(start: SystemTime) -> Self {
    let since = start
      .checked_sub(SLACK)
      .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
      .unwrap_or_default();
    Self {
      count: None,
      since_micros: u64::try_from(since.as_micros()).unwrap_or(u64::MAX),
    }
  }

  /// The notifications of `page` that arrived since the session started and are not in
  /// `seen` yet (they are now), newest first.
  fn fresh(&self, page: &NotificationPage, seen: &mut HashSet<String>) -> Vec<Notification> {
    let mut fresh: Vec<Notification> = page
      .sections
      .iter()
      .flat_map(|s| &s.items)
      // An id that is not a time is never new: nothing tells when it came.
      .filter(|n| n.id.parse::<u64>().is_ok_and(|at| at >= self.since_micros))
      .filter(|n| seen.insert(n.id.clone()))
      .cloned()
      .collect();
    fresh.sort_by_key(|n| std::cmp::Reverse(sent_micros(n)));
    fresh
  }
}

/// When `n` was sent, microseconds since the Unix epoch (its id); 0 when the id is no time.
fn sent_micros(n: &Notification) -> u64 {
  n.id.parse().unwrap_or_default()
}

/// Merges the sections of `page` into `items`, one list newest first: the notifications that
/// `keep` keeps and `items` does not have yet.
fn merge(
  items: &mut Vec<Notification>,
  page: NotificationPage,
  keep: impl Fn(&Notification) -> bool,
) {
  for n in page.sections.into_iter().flat_map(|s| s.items) {
    if keep(&n) && !items.iter().any(|m| m.id == n.id) {
      items.push(n);
    }
  }
  // Stable: the notifications without a time keep YouTube's order, at the end.
  items.sort_by_key(|n| std::cmp::Reverse(sent_micros(n)));
}

/// Marks notifications read on YouTube, as a click on each on youtube.com (it has no request
/// for several), one after the other; `tokens` are their [`Notification::read_token`]s.
fn mark_read(tokens: Vec<String>, cx: &mut App) {
  if tokens.is_empty() {
    return;
  }
  let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
    return;
  };
  // Runs on the network runtime to the end even though nobody awaits it.
  drop(runtime::spawn(cx, async move {
    for token in tokens {
      if let Err(e) = client.mark_notification_read(&token).await {
        tracing::warn!("mark notification read: {e}");
      }
    }
  }));
}

/// Shows the uploads and lives of `fresh` as Windows notifications, for the kinds that ask
/// for them: each with the channel's avatar, a click opening the video and marking it read;
/// beyond [`MAX_TOASTS`], one for them all.
fn toasts(fresh: &[Notification], nav: &WeakEntity<YouTubeSpace>, cx: &mut App) {
  let fresh: Vec<&Notification> = fresh
    .iter()
    .filter(|n| n.video_id.is_some() && delivery(n.kind, cx).is_some_and(|d| d.windows))
    .collect();
  if fresh.len() > MAX_TOASTS {
    let toast = Toast {
      title: tf!(
        "{} new YouTube notifications",
        "{} nouvelles notifications YouTube",
        fresh.len()
      ),
      body: fresh
        .iter()
        .take(MAX_TOASTS)
        .map(|n| n.message.as_str())
        .collect::<Vec<_>>()
        .join("\n"),
      image: None,
    };
    // The window shows: the bell has them.
    toast::show(toast, |_, _| {}, cx);
    return;
  }
  for n in fresh {
    let image = largest(&n.avatar).map(|url| images::local_png(url, TOAST_AVATAR, cx));
    let title = match n.kind {
      NotificationKind::Live => t("Live now", "En direct"),
      _ => t("New video", "Nouvelle vidéo"),
    };
    let (body, video_id, token) = (n.message.clone(), n.video_id.clone(), n.read_token.clone());
    let nav = nav.clone();
    cx.spawn(async move |cx| {
      let image = match image {
        Some(image) => image.await,
        None => None,
      };
      cx.update(|cx| {
        let toast = Toast {
          title: title.to_owned(),
          body,
          image,
        };
        toast::show(
          toast,
          move |window, cx| {
            mark_read(token.into_iter().collect(), cx);
            if let Some(id) = video_id {
              nav
                .update(cx, |space, cx| {
                  space.open_from_notification(Route::Watch(id), window, cx)
                })
                .ok();
            }
          },
          cx,
        )
      });
    })
    .detach();
  }
}

/// The URL of the largest of `sizes`.
fn largest(sizes: &[Thumbnail]) -> Option<&str> {
  sizes.iter().max_by_key(|t| t.width).map(|t| t.url.as_str())
}

#[derive(Clone)]
struct InboxHandle(Entity<Inbox>);

impl Global for InboxHandle {}

/// The inbox, created the first time it is asked for.
pub fn inbox(cx: &mut App) -> Entity<Inbox> {
  if let Some(InboxHandle(inbox)) = cx.try_global::<InboxHandle>() {
    return inbox.clone();
  }
  let inbox = cx.new(|_| Inbox::default());
  cx.set_global(InboxHandle(inbox.clone()));
  inbox
}

/// Starts reading the notifications of the current session (`save::Session`), every
/// [`POLL`]; `nav` opens a Windows notification's video. Called at startup and on every
/// session change.
pub fn reset(nav: WeakEntity<YouTubeSpace>, cx: &mut App) {
  let inbox = inbox(cx);
  let generation = inbox.update(cx, |inbox, cx| {
    inbox.unseen = 0;
    inbox.generation += 1;
    inbox.seen.clear();
    cx.notify();
    inbox.generation
  });
  let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
    return;
  };
  if !client.signed_in() {
    return;
  }
  let current = move |cx: &App| inbox_generation(cx) == Some(generation);
  cx.spawn(async move |cx| {
    let mut poll = Poll::new(SystemTime::now());
    loop {
      let fetch = cx.update(|cx| {
        let client = client.clone();
        runtime::spawn(cx, async move { client.unseen_notifications().await })
      });
      let count = match fetch.await {
        Some(Ok(count)) => Some(count),
        Some(Err(e)) => {
          tracing::warn!("unseen notifications: {e}");
          None
        }
        None => return,
      };
      if !cx.update(|cx| current(cx)) {
        return;
      }
      match (poll.count, count) {
        // YouTube's count of before the session.
        (None, Some(count)) => {
          poll.count = Some(count);
          cx.update(|cx| add_unseen(count, cx));
        }
        (Some(last), Some(count)) if count > last => {
          let fetch = cx.update(|cx| {
            let client = client.clone();
            runtime::spawn(cx, async move { client.notifications().await })
          });
          match fetch.await {
            Some(Ok(page)) => {
              // Reading the list sees them on YouTube: its count starts over.
              poll.count = Some(0);
              cx.update(|cx| {
                if current(cx) {
                  let fresh = seen(cx, |seen| poll.fresh(&page, seen));
                  let shown = fresh.iter().filter(|n| in_bell(n.kind, cx)).count();
                  add_unseen(u32::try_from(shown).unwrap_or(u32::MAX), cx);
                  toasts(&fresh, &nav, cx);
                }
              });
            }
            Some(Err(e)) => tracing::warn!("notifications: {e}"),
            None => return,
          }
        }
        (_, Some(count)) => poll.count = Some(count),
        (_, None) => {}
      }
      cx.background_executor().timer(POLL).await;
    }
  })
  .detach();
}

/// Runs `f` on the session's [`Inbox::seen`].
fn seen<T>(cx: &mut App, f: impl FnOnce(&mut HashSet<String>) -> T) -> T {
  let inbox = inbox(cx);
  inbox.update(cx, |inbox, _| f(&mut inbox.seen))
}

fn inbox_generation(cx: &App) -> Option<u64> {
  cx.try_global::<InboxHandle>()
    .map(|h| h.0.read(cx).generation)
}

fn add_unseen(count: u32, cx: &mut App) {
  if count == 0 {
    return;
  }
  if let Some(InboxHandle(inbox)) = cx.try_global::<InboxHandle>().cloned() {
    inbox.update(cx, |inbox, cx| {
      inbox.unseen = inbox.unseen.saturating_add(count);
      cx.notify();
    });
  }
}

/// Unread notifications; `None` signed out, when YouTube has no inbox to show.
pub fn unseen(cx: &App) -> Option<usize> {
  let signed_in = cx.try_global::<Session>().is_some_and(|s| s.0.signed_in());
  if !signed_in {
    return None;
  }
  Some(
    cx.try_global::<InboxHandle>()
      .map_or(0, |h| h.0.read(cx).unseen as usize),
  )
}

/// YouTube's section of the notifications panel. It lives in the popover's element state, so
/// it starts over (and reloads) each time the panel opens; `popover` closes the panel.
pub fn panel(
  nav: WeakEntity<YouTubeSpace>,
  popover: WeakEntity<PopoverState>,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  window
    .use_keyed_state(ElementId::Name("inbox-panel".into()), cx, |_, cx| {
      InboxPanel::new(nav, popover, cx)
    })
    .into_any_element()
}

enum State {
  Loading,
  Loaded,
  Failed(SharedString),
}

/// The open inbox. It lives in the popover's element state, so it starts over (and reloads)
/// each time the popover opens.
struct InboxPanel {
  nav: WeakEntity<YouTubeSpace>,
  popover: WeakEntity<PopoverState>,
  state: State,
  /// The notifications of every page loaded, newest first.
  items: Vec<Notification>,
  continuation: Option<String>,
  loading_more: bool,
  /// The last next page failed: no automatic retry.
  failed_more: bool,
  scroll: ScrollHandle,
}

impl InboxPanel {
  fn new(
    nav: WeakEntity<YouTubeSpace>,
    popover: WeakEntity<PopoverState>,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    // Opening the inbox sees its notifications, as on youtube.com.
    if let Some(InboxHandle(inbox)) = cx.try_global::<InboxHandle>().cloned() {
      inbox.update(cx, |inbox, cx| {
        inbox.unseen = 0;
        cx.notify();
      });
    }
    let mut panel = Self {
      nav,
      popover,
      state: State::Loading,
      items: Vec::new(),
      continuation: None,
      loading_more: false,
      failed_more: false,
      scroll: ScrollHandle::new(),
    };
    panel.load(None, cx);
    panel
  }

  /// The first page, or the one of `token`.
  fn load(&mut self, token: Option<String>, cx: &mut Context<Self>) {
    let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
      return;
    };
    let more = token.is_some();
    if more {
      self.loading_more = true;
    }
    let fetch = runtime::spawn(cx, async move {
      match token {
        Some(token) => client.notifications_more(&token).await,
        None => client.notifications().await,
      }
    });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          this.loading_more = false;
          match result {
            Some(Ok(page)) => {
              // Seen here: not to be announced later.
              let ids = page
                .sections
                .iter()
                .flat_map(|s| &s.items)
                .map(|n| n.id.clone());
              seen(cx, |seen| seen.extend(ids));
              // Opening the inbox reads what it shows: marked read on YouTube, while their dot
              // still tells here what was new.
              let tokens = page
                .sections
                .iter()
                .flat_map(|s| &s.items)
                .filter(|n| !n.read && in_bell(n.kind, cx))
                .filter_map(|n| n.read_token.clone())
                .collect();
              mark_read(tokens, cx);
              this.append(page, |n| in_bell(n.kind, cx))
            }
            Some(Err(e)) => {
              tracing::warn!("notifications: {e}");
              if more {
                this.failed_more = true;
              } else {
                this.state = State::Failed(inbox_error(&e).into());
              }
            }
            None => return,
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// Adds the notifications of a page that `keep` keeps (the kinds the settings leave out of
  /// the bell are not).
  fn append(&mut self, page: NotificationPage, keep: impl Fn(&Notification) -> bool) {
    self.state = State::Loaded;
    self.continuation = page.continuation.clone();
    merge(&mut self.items, page, keep);
  }

  fn load_more_if_near_end(&mut self, cx: &mut Context<Self>) {
    if matches!(self.state, State::Loaded)
      && !self.loading_more
      && !self.failed_more
      && self.continuation.is_some()
      && near_scroll_end(&self.scroll)
    {
      let token = self.continuation.clone();
      self.load(token, cx);
    }
  }

  /// Opens the video of a notification (already marked read on YouTube by the opening).
  fn open(&mut self, item: usize, window: &mut Window, cx: &mut Context<Self>) {
    let Some(notification) = self.items.get_mut(item) else {
      return;
    };
    let Some(video_id) = notification.video_id.clone() else {
      return;
    };
    notification.read = true;
    self
      .popover
      .update(cx, |popover, cx| popover.dismiss(window, cx))
      .ok();
    self
      .nav
      .update(cx, |space, cx| {
        space.open_from_notification(Route::Watch(video_id), window, cx)
      })
      .ok();
  }

  fn row(
    &self,
    item: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let n = &self.items[item];
    h_flex()
      .id(ElementId::Name(format!("notification-{}", n.id).into()))
      .w_full()
      .px_2()
      .py_2()
      .gap_3()
      .items_start()
      .rounded(theme::RADIUS)
      .cursor_pointer()
      .hover(|s| s.bg(theme::surface_hover()))
      .on_click(cx.listener(move |this, _, window, cx| this.open(item, window, cx)))
      .child(
        div()
          .mt(px(16.))
          .size(px(6.))
          .flex_none()
          .rounded_full()
          .when(!n.read, |el| el.bg(theme::accent())),
      )
      .child(div().size(px(AVATAR)).flex_none().child(image_fill(
        &convert::picture(&n.avatar),
        AVATAR,
        1.,
        px(AVATAR / 2.),
        window,
        cx,
      )))
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .gap_1()
          .child(div().text_sm().line_clamp(3).child(n.message.clone()))
          .child(
            div()
              .text_xs()
              .text_color(theme::muted())
              .child(n.sent.clone()),
          ),
      )
      .when(!n.thumbnails.is_empty(), |el| {
        el.child(
          div()
            .w(px(THUMB_WIDTH))
            .h(px(THUMB_WIDTH * 9. / 16.))
            .flex_none()
            .child(image_fill(
              &convert::picture(&n.thumbnails),
              THUMB_WIDTH,
              WIDE,
              theme::RADIUS,
              window,
              cx,
            )),
        )
      })
  }
}

fn inbox_error(error: &Error) -> String {
  match error {
    Error::SignInRequired | Error::NotLoggedIn => t(
      "Sign in to see your notifications: import your cookies from the Account page.",
      "Connectez-vous pour voir vos notifications : importez vos cookies depuis la page Compte.",
    )
    .into(),
    e => tf!(
      "Cannot load the notifications. {}",
      "Impossible de charger les notifications. {}",
      error_text(e)
    ),
  }
}

impl Render for InboxPanel {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let body = match &self.state {
      State::Loading => v_flex()
        .gap_3()
        .children((0..5).map(|_| {
          h_flex()
            .px_2()
            .gap_3()
            .child(Skeleton::new().size(px(AVATAR)).rounded_full())
            .child(
              v_flex()
                .flex_1()
                .gap_2()
                .child(Skeleton::new().w_full().h(px(14.)))
                .child(Skeleton::new().w(px(80.)).h(px(12.))),
            )
            .child(
              Skeleton::new()
                .w(px(THUMB_WIDTH))
                .h(px(THUMB_WIDTH * 9. / 16.)),
            )
        }))
        .into_any_element(),
      State::Failed(message) => div()
        .px_2()
        .text_sm()
        .text_color(theme::muted())
        .child(message.clone())
        .into_any_element(),
      State::Loaded if self.items.is_empty() => div()
        .px_2()
        .text_sm()
        .text_color(theme::muted())
        .child(t("No notifications.", "Aucune notification."))
        .into_any_element(),
      State::Loaded => {
        let mut list = v_flex()
          .id("inbox-list")
          .max_h(px(560.))
          .overflow_y_scroll()
          .track_scroll(&self.scroll)
          .on_scroll(cx.listener(|this, _, _, cx| {
            this.load_more_if_near_end(cx);
            cx.notify();
          }))
          .gap_0p5();
        for i in 0..self.items.len() {
          list = list.child(self.row(i, window, cx));
        }
        if self.loading_more {
          list = list.child(div().p_2().child(loading_more()));
        }
        list.into_any_element()
      }
    };
    // The shell's panel draws the title and the space's name above.
    body
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use innertube::NotificationSection;

  fn notification(id: &str) -> Notification {
    Notification {
      id: id.into(),
      message: String::new(),
      sent: String::new(),
      read: false,
      video_id: Some(format!("v{id}")),
      short: false,
      kind: NotificationKind::Upload,
      avatar: Vec::new(),
      thumbnails: Vec::new(),
      read_token: None,
    }
  }

  fn page(ids: &[&str]) -> NotificationPage {
    NotificationPage {
      sections: vec![NotificationSection {
        title: String::new(),
        items: ids.iter().map(|id| notification(id)).collect(),
      }],
      continuation: None,
    }
  }

  #[test]
  fn fresh_notifications_arrived_since_the_session_once() {
    // The session started at 1 000 s; ids are microseconds.
    let poll = Poll::new(SystemTime::UNIX_EPOCH + Duration::from_secs(1000));
    let mut seen = HashSet::new();
    let ids = |fresh: Vec<Notification>| fresh.into_iter().map(|n| n.id).collect::<Vec<_>>();
    // Before the session (beyond the slack), and an id that is no time: not new.
    let first = page(&["1000500000", "900000000", "950000000", "notatime"]);
    assert_eq!(
      ids(poll.fresh(&first, &mut seen)),
      ["1000500000", "950000000"]
    );
    // Counted once (or seen in the panel); the newest first.
    seen.insert("1300000000".to_owned());
    let second = page(&["1000500000", "1200000000", "1100000000", "1300000000"]);
    assert_eq!(
      ids(poll.fresh(&second, &mut seen)),
      ["1200000000", "1100000000"]
    );
    assert!(poll.fresh(&second, &mut seen).is_empty());
  }

  #[test]
  fn panel_merges_sections_newest_first() {
    let section = |title: &str, ids: &[&str]| NotificationSection {
      title: title.into(),
      items: ids.iter().map(|id| notification(id)).collect(),
    };
    let page = |sections| NotificationPage {
      sections,
      continuation: None,
    };
    let mut items = Vec::new();
    let first = page(vec![
      section("Important", &["100", "300"]),
      section("Others", &["400", "notatime", "200", "250"]),
    ]);
    merge(&mut items, first, |n| n.id != "250");
    // A next page: older ones, and one already there.
    merge(&mut items, page(vec![section("", &["150", "300"])]), |_| {
      true
    });
    let ids: Vec<&str> = items.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["400", "300", "200", "150", "100", "notatime"]);
  }
}
