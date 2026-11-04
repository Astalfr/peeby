//! The subscribe button of the watch page and of a Short: "Subscribe" while the user is not subscribed to the
//! channel; "Subscribed" otherwise, whose menu unsubscribes (a second click, as YouTube's
//! confirmation dialog).
//!
//! The state the page parsed is the starting point; a change shows at once on every page of
//! the channel ([`Subscriptions`]), the bell with it, and goes back with a toast if YouTube
//! refuses.

use std::collections::{HashMap, HashSet};

use gpui::prelude::*;
use gpui::{Anchor, App, ElementId, Global, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::notification::Notification;
use gpui_component::popover::Popover;
use gpui_component::{Sizable, WindowExt, h_flex, v_flex};
use innertube::{Error, Subscription};

use crate::spaces::youtube::convert::error_text;
use crate::spaces::youtube::save::Session;
use ui::assets::Icon;
use ui::i18n::t;
use ui::widgets::loader;
use ui::{runtime, tf, theme};

/// Subscriptions changed in this session, by channel id, over the states the pages parsed, and
/// the channels whose change is waiting for YouTube's answer.
#[derive(Default)]
struct Subscriptions {
  subscribed: HashMap<String, bool>,
  pending: HashSet<String>,
}

impl Global for Subscriptions {}

/// Forgets the changes made so far: the next pages read them from YouTube. Called on every
/// session change.
pub fn reset(cx: &mut App) {
  cx.set_global(Subscriptions::default());
}

/// Whether the user is subscribed to `channel_id`, whose page parsed `subscription`.
pub fn is_subscribed(channel_id: &str, subscription: &Subscription, cx: &App) -> bool {
  cx.try_global::<Subscriptions>()
    .and_then(|s| s.subscribed.get(channel_id).copied())
    .unwrap_or(subscription.subscribed)
}

fn is_pending(channel_id: &str, cx: &App) -> bool {
  cx.try_global::<Subscriptions>()
    .is_some_and(|s| s.pending.contains(channel_id))
}

/// The subscribe button of `channel_id` (named `channel`), whose page parsed `subscription`.
/// `small` under a Short.
pub fn subscribe_button(
  id: &str,
  channel_id: &str,
  channel: &str,
  subscription: &Subscription,
  small: bool,
  cx: &App,
) -> impl IntoElement + use<> {
  let key = format!("subscribe-{id}");
  let subscribed = is_subscribed(channel_id, subscription, cx);
  let busy = is_pending(channel_id, cx);
  let (channel_id, channel, subscription) = (
    channel_id.to_owned(),
    channel.to_owned(),
    subscription.clone(),
  );
  let button = Button::new(ElementId::Name(format!("{key}-button").into()))
    .map(|b| {
      if subscribed {
        b.ghost().label(t("Subscribed", "Abonné"))
      } else {
        b.primary().label(t("Subscribe", "S'abonner"))
      }
    })
    .when(small, |b| b.small())
    .when(busy, |b| b.child(loader(16., theme::muted())));
  if !subscribed {
    return div()
      .flex_none()
      .child(button.on_click(move |_, window, cx| {
        set_subscribed(&channel_id, true, &subscription, window, cx);
      }));
  }
  div().flex_none().child(
    Popover::new(ElementId::Name(key.into()))
      .anchor(Anchor::TopLeft)
      .trigger(button)
      .content(move |_, _, cx| {
        let (channel_id, subscription) = (channel_id.clone(), subscription.clone());
        v_flex().w(px(260.)).gap_1().child(
          h_flex()
            .id("unsubscribe")
            .w_full()
            .px_2()
            .py_1p5()
            .gap_3()
            .items_center()
            .rounded(theme::RADIUS)
            .cursor_pointer()
            .text_sm()
            .hover(|s| s.bg(theme::surface_hover()))
            .on_click(cx.listener(move |state, _, window, cx| {
              state.dismiss(window, cx);
              set_subscribed(&channel_id, false, &subscription, window, cx);
            }))
            .child(
              gpui_component::Icon::new(Icon::BellOff)
                .size(px(16.))
                .flex_none(),
            )
            // A long channel name wraps inside the menu, two lines at most.
            .child(div().flex_1().min_w_0().line_clamp(2).child(tf!(
              "Unsubscribe from {}",
              "Se désabonner de {}",
              channel
            ))),
        )
      }),
  )
}

/// Subscribes to `channel_id` or unsubscribes (`subscribe`). The button changes at once and
/// goes back if YouTube refuses; nothing is sent while a change of this channel is pending.
fn set_subscribed(
  channel_id: &str,
  subscribe: bool,
  subscription: &Subscription,
  window: &mut Window,
  cx: &mut App,
) {
  let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
    return;
  };
  if !client.signed_in() {
    return window.push_notification(
      Notification::error(subscribe_error(&Error::SignInRequired, subscribe)),
      cx,
    );
  }
  let subscriptions = cx.default_global::<Subscriptions>();
  if !subscriptions.pending.insert(channel_id.to_owned()) {
    return;
  }
  subscriptions
    .subscribed
    .insert(channel_id.to_owned(), subscribe);
  cx.refresh_windows();
  let params = subscription.params(subscribe).map(str::to_owned);
  let target = channel_id.to_owned();
  let task = runtime::spawn(cx, async move {
    client
      .set_subscribed(&target, subscribe, params.as_deref())
      .await
  });
  let (channel_id, window) = (channel_id.to_owned(), window.window_handle());
  cx.spawn(async move |cx| {
    let Some(result) = task.await else { return };
    let note = match &result {
      Ok(()) => Notification::success(if subscribe {
        t("Subscription added.", "Abonnement ajouté.")
      } else {
        t("Subscription removed.", "Abonnement supprimé.")
      }),
      Err(e) => {
        tracing::warn!("subscription: {e}");
        Notification::error(subscribe_error(e, subscribe))
      }
    };
    cx.update(|cx| {
      let subscriptions = cx.default_global::<Subscriptions>();
      subscriptions.pending.remove(&channel_id);
      if result.is_err() {
        subscriptions.subscribed.insert(channel_id, !subscribe);
      }
      cx.refresh_windows();
      window
        .update(cx, |_, window, cx| window.push_notification(note, cx))
        .ok();
    });
  })
  .detach();
}

fn subscribe_error(error: &Error, subscribe: bool) -> String {
  match error {
    Error::SignInRequired => t(
      "To subscribe, sign in to YouTube in the settings.",
      "Pour vous abonner, connectez-vous à YouTube dans les réglages.",
    )
    .into(),
    e if subscribe => tf!(
      "Cannot subscribe. {}",
      "Impossible de s'abonner. {}",
      error_text(e)
    ),
    e => tf!(
      "Cannot unsubscribe. {}",
      "Impossible de se désabonner. {}",
      error_text(e)
    ),
  }
}
