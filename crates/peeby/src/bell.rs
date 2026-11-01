//! The notification bell of a subscribed channel, on the watch page and the channels page: a
//! menu with "Toutes", "Personnalisées" and "Aucune" (those the channel offers), as YouTube's
//! bell next to "Abonné".
//!
//! The level the pages parsed is the starting point; a change shows at once on every page of
//! the channel ([`Bells`]) and goes back with a toast if YouTube refuses.

use std::collections::{HashMap, HashSet};

use gpui::prelude::*;
use gpui::{Anchor, App, ElementId, FontWeight, Global, MouseButton, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::notification::Notification;
use gpui_component::popover::Popover;
use gpui_component::{Sizable, WindowExt, h_flex, v_flex};
use innertube::{ChannelNotifications, Error, NotificationLevel};

use crate::assets::Icon;
use crate::save::Session;
use crate::ui::{error_text, loader};
use crate::{runtime, theme};

/// Levels set in this session, by channel id, over the ones the pages parsed (the watch page
/// and the channels page agree after a change), and the channels whose change is waiting for
/// YouTube's answer.
#[derive(Default)]
struct Bells {
  levels: HashMap<String, NotificationLevel>,
  pending: HashSet<String>,
}

impl Global for Bells {}

/// Forgets the levels set so far: the next pages read them from YouTube. Called on every
/// session change.
pub fn reset(cx: &mut App) {
  cx.set_global(Bells::default());
}

fn level(channel_id: &str, bell: &ChannelNotifications, cx: &App) -> NotificationLevel {
  cx.try_global::<Bells>()
    .and_then(|b| b.levels.get(channel_id).copied())
    .unwrap_or(bell.level)
}

fn is_pending(channel_id: &str, cx: &App) -> bool {
  cx.try_global::<Bells>()
    .is_some_and(|b| b.pending.contains(channel_id))
}

fn icon(level: NotificationLevel) -> Icon {
  match level {
    NotificationLevel::All => Icon::BellRing,
    NotificationLevel::Personalized => Icon::Bell,
    NotificationLevel::None => Icon::BellOff,
  }
}

fn label(level: NotificationLevel) -> &'static str {
  match level {
    NotificationLevel::All => "Toutes",
    NotificationLevel::Personalized => "Personnalisées",
    NotificationLevel::None => "Aucune",
  }
}

/// The bell of `channel_id`, whose page parsed `bell`. `small` on the channels page's tiles.
pub fn bell_button(
  id: &str,
  channel_id: &str,
  bell: &ChannelNotifications,
  small: bool,
  cx: &App,
) -> impl IntoElement + use<> {
  let key = format!("bell-{id}");
  let current = level(channel_id, bell, cx);
  let busy = is_pending(channel_id, cx);
  let button = Button::new(ElementId::Name(format!("{key}-button").into()))
    .ghost()
    .tooltip(format!("Notifications : {}", label(current).to_lowercase()))
    .map(|b| if small { b.small() } else { b })
    .map(|b| {
      if busy {
        b.child(loader(16., theme::muted()))
      } else {
        b.icon(icon(current))
      }
    });
  let (channel_id, bell) = (channel_id.to_owned(), bell.clone());
  div()
    .flex_none()
    // The channels page's tile opens the channel on click: keep this click from reaching it.
    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
    .child(
      Popover::new(ElementId::Name(key.into()))
        .anchor(Anchor::TopRight)
        .trigger(button)
        .content(move |_, _, cx| {
          let current = level(&channel_id, &bell, cx);
          v_flex()
            .w(px(220.))
            .gap_0p5()
            .child(
              div()
                .px_2()
                .pb_1()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Notifications"),
            )
            .children(bell.options.iter().map(|(level, params)| {
              let (level, params) = (*level, params.clone());
              let channel_id = channel_id.clone();
              h_flex()
                .id(label(level))
                .w_full()
                .px_2()
                .py_1p5()
                .gap_3()
                .items_center()
                .rounded(theme::RADIUS)
                .cursor_pointer()
                .text_sm()
                .hover(|s| s.bg(theme::surface_hover()))
                .when(level == current, |el| el.text_color(theme::accent()))
                .on_click(cx.listener(move |state, _, window, cx| {
                  state.dismiss(window, cx);
                  if level != current {
                    set_level(&channel_id, current, level, &params, window, cx);
                  }
                }))
                .child(gpui_component::Icon::new(icon(level)).size(px(16.)))
                .child(div().flex_1().child(label(level)))
                .when(level == current, |el| {
                  el.child(gpui_component::Icon::new(Icon::Check).size(px(14.)))
                })
            }))
        }),
    )
}

/// Sets the bell of `channel_id` to `level` (`params` from its menu). The bell changes at once
/// and goes back to `previous` if YouTube refuses; nothing is sent while a change of this
/// channel is pending.
fn set_level(
  channel_id: &str,
  previous: NotificationLevel,
  level: NotificationLevel,
  params: &str,
  window: &mut Window,
  cx: &mut App,
) {
  let Some(client) = cx.try_global::<Session>().map(|s| s.0.clone()) else {
    return;
  };
  if !client.signed_in() {
    return window.push_notification(Notification::error(bell_error(&Error::SignInRequired)), cx);
  }
  let bells = cx.default_global::<Bells>();
  if !bells.pending.insert(channel_id.to_owned()) {
    return;
  }
  bells.levels.insert(channel_id.to_owned(), level);
  cx.refresh_windows();
  let params = params.to_owned();
  let task = runtime::spawn(cx, async move {
    client.set_channel_notifications(&params).await
  });
  let (channel_id, window) = (channel_id.to_owned(), window.window_handle());
  cx.spawn(async move |cx| {
    let Some(result) = task.await else { return };
    let note = match &result {
      Ok(()) => Notification::success(match level {
        NotificationLevel::All => "Vous recevrez toutes les notifications de cette chaîne.",
        NotificationLevel::Personalized => {
          "Vous recevrez des notifications personnalisées de cette chaîne."
        }
        NotificationLevel::None => "Vous ne recevrez plus de notifications de cette chaîne.",
      }),
      Err(e) => {
        tracing::warn!("channel notifications: {e}");
        Notification::error(bell_error(e))
      }
    };
    cx.update(|cx| {
      let bells = cx.default_global::<Bells>();
      bells.pending.remove(&channel_id);
      if result.is_err() {
        bells.levels.insert(channel_id, previous);
      }
      cx.refresh_windows();
      window
        .update(cx, |_, window, cx| window.push_notification(note, cx))
        .ok();
    });
  })
  .detach();
}

fn bell_error(error: &Error) -> String {
  match error {
    Error::SignInRequired => {
      "Connectez-vous pour régler les notifications : importez vos cookies depuis la page Compte."
        .into()
    }
    e => format!(
      "Impossible de modifier les notifications. {}",
      error_text(e)
    ),
  }
}
