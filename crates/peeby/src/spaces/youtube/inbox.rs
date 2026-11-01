//! The notifications inbox: a bell in the title bar with the unseen count, opening the list
//! of notifications as youtube.com's bell does ("Important", then the others), each opening
//! its video.
//!
//! The unseen count is read at startup, on every session change and then every 30 minutes
//! (the delay YouTube's answer gives). The list is loaded each time the panel opens, its next
//! page when the end of the list comes near. A click marks the notification read, at once here
//! and on YouTube.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  AnyElement, App, Context, ElementId, Entity, FontWeight, Global, ScrollHandle, SharedString,
  WeakEntity, Window, div, px,
};
use gpui_component::popover::PopoverState;
use gpui_component::skeleton::Skeleton;
use gpui_component::{h_flex, v_flex};
use innertube::{Error, NotificationPage, NotificationSection};

use crate::spaces::youtube::convert::{self, error_text};
use crate::spaces::youtube::save::Session;
use crate::spaces::youtube::{Route, YouTubeSpace};
use ui::card::WIDE;
use ui::images::Images;
use ui::picture::image_fill;
use ui::widgets::{loading_more, near_scroll_end};
use ui::{runtime, theme};

/// How often the unseen count is read again.
const POLL: Duration = Duration::from_secs(30 * 60);
const AVATAR: f32 = 40.;
const THUMB_WIDTH: f32 = 86.;

/// The unseen count. `generation` tells apart the sessions, so the polling of an old one
/// stops and its late answers are dropped.
#[derive(Default)]
pub struct Inbox {
  unseen: u32,
  generation: u64,
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

/// Starts reading the unseen count of the current session (`save::Session`), every
/// [`POLL`]. Called at startup and on every session change.
pub fn reset(cx: &mut App) {
  let inbox = inbox(cx);
  let generation = inbox.update(cx, |inbox, cx| {
    inbox.unseen = 0;
    inbox.generation += 1;
    cx.notify();
    inbox.generation
  });
  let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
    return;
  };
  if !client.signed_in() {
    return;
  }
  cx.spawn(async move |cx| {
    loop {
      let fetch = cx.update(|cx| {
        let client = client.clone();
        runtime::spawn(cx, async move { client.unseen_notifications().await })
      });
      let result = fetch.await;
      let current = cx.update(|cx| {
        inbox.update(cx, |inbox, cx| {
          if inbox.generation != generation {
            return false;
          }
          match result {
            Some(Ok(unseen)) => {
              inbox.unseen = unseen;
              cx.notify();
            }
            Some(Err(e)) => tracing::warn!("unseen notifications: {e}"),
            None => {}
          }
          true
        })
      });
      if !current {
        return;
      }
      cx.background_executor().timer(POLL).await;
    }
  })
  .detach();
}

/// The bell of the title bar: the unseen count on a red badge, the inbox on click. Signed
/// out, nothing.
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
  sections: Vec<NotificationSection>,
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
      sections: Vec::new(),
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
            Some(Ok(page)) => this.append(page),
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

  /// Adds a page: a next page's untitled section continues the last one.
  fn append(&mut self, page: NotificationPage) {
    self.state = State::Loaded;
    self.continuation = page.continuation;
    for section in page.sections {
      match self.sections.last_mut() {
        Some(last) if section.title.is_empty() => last.items.extend(section.items),
        _ => self.sections.push(section),
      }
    }
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

  /// Opens the video of a notification and marks it read.
  fn open(&mut self, section: usize, item: usize, window: &mut Window, cx: &mut Context<Self>) {
    let Some(notification) = self
      .sections
      .get_mut(section)
      .and_then(|s| s.items.get_mut(item))
    else {
      return;
    };
    let Some(video_id) = notification.video_id.clone() else {
      return;
    };
    if !notification.read {
      notification.read = true;
      if let (Some(token), Some(client)) = (
        notification.read_token.clone(),
        cx.try_global::<Session>().map(|s| s.0.clone()),
      ) {
        let task = runtime::spawn(
          cx,
          async move { client.mark_notification_read(&token).await },
        );
        cx.background_executor()
          .spawn(async move {
            if let Some(Err(e)) = task.await {
              tracing::warn!("mark notification read: {e}");
            }
          })
          .detach();
      }
    }
    self
      .popover
      .update(cx, |popover, cx| popover.dismiss(window, cx))
      .ok();
    self
      .nav
      .update(cx, |space, cx| {
        space.request_focus(cx);
        space.open(Route::Watch(video_id), window, cx);
      })
      .ok();
  }

  fn row(
    &self,
    section: usize,
    item: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let n = &self.sections[section].items[item];
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
      .on_click(cx.listener(move |this, _, window, cx| this.open(section, item, window, cx)))
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
    Error::SignInRequired | Error::NotLoggedIn => {
      "Connectez-vous pour voir vos notifications : importez vos cookies depuis la page Compte."
        .into()
    }
    e => format!("Impossible de charger les notifications. {}", error_text(e)),
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
      State::Loaded if self.sections.is_empty() => div()
        .px_2()
        .text_sm()
        .text_color(theme::muted())
        .child("Aucune notification.")
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
        for (s, section) in self.sections.iter().enumerate() {
          if !section.title.is_empty() {
            list = list.child(
              div()
                .px_2()
                .pt_2()
                .pb_1()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::muted())
                .child(section.title.clone()),
            );
          }
          for i in 0..section.items.len() {
            list = list.child(self.row(s, i, window, cx));
          }
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
