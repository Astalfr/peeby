//! A user's card over the messages, as Twitch's viewer card: their picture, their name, a button
//! opening their channel in a new tab ([`ChatPanelEvent::OpenChannel`]) and their last messages
//! in this chat. A click on an author's name or on a mention (`@name`) opens it; what the
//! messages do not tell (a picture, a channel, someone who has not written) the space finds
//! ([`crate::chat::UserLookup`]). Clicking the same user again or the cross closes it.

use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDate};
use gpui::prelude::*;
use gpui::{
  AnyElement, Context, FontWeight, ScrollHandle, SharedString, Task, WeakEntity, Window, div, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{IconName, Sizable, h_flex, v_flex};

use super::{ChatPanel, ChatPanelEvent, push_fragments, store_image};
use crate::chat::{Author, ChatMessage, MessageKind, UserQuery, name_color, same_user};
use crate::i18n::{Lang, lang, t};
use crate::images::{Effects, Images};
use crate::picture::{self, Picture};
use crate::{tf, theme};

/// Most of a user's messages the card lists, newest last.
const CARD_MESSAGES: usize = 50;
/// Side of the picture, logical pixels.
const CARD_AVATAR: f32 = 48.;
/// The card's height at most, logical pixels: the messages scroll under its head.
const CARD_MAX_HEIGHT: f32 = 420.;

/// When a message was sent, in local time: `14:32` today, its day before it otherwise (`07/11
/// 14:32` in French, `11/07 14:32` in English).
fn sent_at(time: SystemTime, today: NaiveDate) -> SharedString {
  let local: DateTime<Local> = time.into();
  let format = if local.date_naive() == today {
    "%H:%M"
  } else if lang() == Lang::Fr {
    "%d/%m %H:%M"
  } else {
    "%m/%d %H:%M"
  };
  local.format(format).to_string().into()
}

/// The card open over the panel.
pub(super) struct UserCard {
  /// Who it is about, as asked (a click on a name or a mention).
  asked: UserQuery,
  /// Their user id, once known: their messages are picked by it, else by name.
  id: Option<SharedString>,
  name: SharedString,
  avatar: Option<Picture>,
  channel: Option<SharedString>,
  /// One of their messages' authors, for the name's color.
  author: Option<Author>,
  /// The space is looking for what the messages do not tell.
  looking: bool,
  scroll: ScrollHandle,
  _lookup: Option<Task<()>>,
}

impl ChatPanel {
  /// Opens the card of `query`, or closes it when it is already the one open.
  pub(super) fn toggle_card(&mut self, query: UserQuery, cx: &mut Context<Self>) {
    if self.card.as_ref().is_some_and(|card| card.asked == query) {
      self.close_card(cx);
      return;
    }
    let author = self.find_author(&query).cloned();
    let id = author.as_ref().map(|a| a.id.clone()).or(query.id.clone());
    let name = author
      .as_ref()
      .map(|a| a.name.clone())
      .unwrap_or(query.name.clone());
    let avatar = author.as_ref().and_then(|a| a.avatar.clone());
    let channel = author
      .as_ref()
      .and_then(|a| a.channel.clone())
      .or(query.channel.clone());
    // What the space is asked for: everything the chat knows about them.
    let known = UserQuery {
      id: id.clone(),
      name: name.clone(),
      channel: channel.clone(),
    };
    let lookup = (avatar.is_none() || channel.is_none())
      .then(|| self.lookup.clone())
      .flatten()
      .map(|lookup| {
        let task = lookup(known, cx);
        let asked = query.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
          let profile = task.await;
          this
            .update(cx, |this, cx| {
              let Some(card) = this.card.as_mut().filter(|card| card.asked == asked) else {
                return;
              };
              card.looking = false;
              if let Some(profile) = profile {
                card.avatar = card.avatar.take().or(profile.avatar);
                card.channel = card.channel.take().or(profile.channel);
                // Someone who has not written: the name their service shows.
                if card.author.is_none() {
                  card.name = profile.name;
                }
              }
              cx.notify();
            })
            .ok();
        })
      });
    self.card = Some(UserCard {
      asked: query,
      id,
      name,
      avatar,
      channel,
      author,
      looking: lookup.is_some(),
      scroll: ScrollHandle::new(),
      _lookup: lookup,
    });
    // Their last messages show first, at the bottom as in the chat.
    if let Some(card) = &self.card {
      card.scroll.scroll_to_bottom();
    }
    cx.notify();
  }

  pub(super) fn close_card(&mut self, cx: &mut Context<Self>) {
    if self.card.take().is_some() {
      cx.notify();
    }
  }

  /// The newest author of the panel `query` is about: by user id when known, else by name or
  /// channel (a Twitch login), case aside.
  fn find_author(&self, query: &UserQuery) -> Option<&Author> {
    self
      .messages
      .iter()
      .chain(&self.held)
      .rev()
      .filter_map(|m| m.author.as_ref())
      .find(|a| match &query.id {
        Some(id) => a.id == *id,
        None => {
          same_user(&a.name, &query.name)
            || a
              .channel
              .as_ref()
              .is_some_and(|c| same_user(c, &query.name))
        }
      })
  }

  /// The messages of the open card's user, oldest first, at most [`CARD_MESSAGES`].
  fn card_messages(&self, card: &UserCard) -> Vec<ChatMessage> {
    let of_user = |m: &&ChatMessage| {
      m.author.as_ref().is_some_and(|a| match &card.id {
        Some(id) => a.id == *id,
        None => same_user(&a.name, &card.name),
      })
    };
    let mut messages: Vec<ChatMessage> = self
      .messages
      .iter()
      .chain(&self.held)
      .rev()
      .filter(of_user)
      .take(CARD_MESSAGES)
      .cloned()
      .collect();
    messages.reverse();
    messages
  }

  /// The open card, over the messages; `None` when none is open.
  pub(super) fn render_card(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let card = self.card.as_ref()?;
    let messages = self.card_messages(card);
    let panel = cx.entity().downgrade();
    let scale = window.scale_factor();
    let images = cx.try_global::<Images>().cloned();
    let color = card
      .author
      .as_ref()
      .map(name_color)
      .unwrap_or_else(theme::foreground);
    let initial: SharedString = card
      .name
      .trim_start_matches('@')
      .chars()
      .next()
      .unwrap_or('?')
      .to_uppercase()
      .to_string()
      .into();
    let avatar = match &card.avatar {
      Some(avatar) => picture::avatar(avatar, CARD_AVATAR, window, cx),
      None => div()
        .size(px(CARD_AVATAR))
        .flex_none()
        .rounded_full()
        .bg(theme::surface_hover())
        .flex()
        .items_center()
        .justify_center()
        .text_lg()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::muted())
        .child(initial)
        .into_any_element(),
    };
    let open = card.channel.clone().map(|channel| {
      let name = card.name.clone();
      Button::new("chat-card-channel")
        .small()
        .primary()
        .label(t("Open channel", "Ouvrir la chaîne"))
        .on_click(cx.listener(move |_, _, _, cx| {
          cx.emit(ChatPanelEvent::OpenChannel {
            channel: channel.clone(),
            name: name.clone(),
          })
        }))
    });
    let looking = card.looking;
    let head = h_flex()
      .gap_3()
      .items_center()
      .child(avatar)
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .gap_1()
          .child(
            div()
              .text_base()
              .font_weight(FontWeight::BOLD)
              .text_color(color)
              .truncate()
              .child(card.name.clone()),
          )
          .child(
            div()
              .text_xs()
              .text_color(theme::muted())
              .child(match messages.len() {
                0 => t("No message in this chat", "Aucun message dans ce chat").to_owned(),
                1 => t("1 message in this chat", "1 message dans ce chat").to_owned(),
                n => tf!("{n} messages in this chat", "{n} messages dans ce chat"),
              }),
          ),
      )
      .child(
        Button::new("chat-card-close")
          .ghost()
          .small()
          .icon(IconName::Close)
          .on_click(cx.listener(|this, _, _, cx| this.close_card(cx))),
      );
    let mut image = |picture: &Picture, effects: Effects, width: f32, height: f32| {
      store_image(
        images.as_ref(),
        picture,
        effects,
        (width, height),
        scale,
        cx,
      )
    };
    let today = Local::now().date_naive();
    let lines = messages.iter().enumerate().map(|(i, message)| {
      let action = message.kind == MessageKind::Action;
      let text_color = if message.deleted {
        theme::subtle()
      } else if action {
        color
      } else {
        theme::foreground()
      };
      let line = h_flex()
        .flex_1()
        .min_w_0()
        .flex_wrap()
        .items_center()
        .text_sm();
      // Each message apart, its time on its left.
      h_flex()
        .w_full()
        .gap_2()
        .items_start()
        .py_1p5()
        .when(i > 0, |el| el.border_t_1().border_color(theme::border()))
        .child(
          div()
            .flex_none()
            // On the text's first line (14 px text, 20 px lines).
            .pt(px(3.))
            .text_xs()
            .text_color(theme::subtle())
            .child(sent_at(message.time, today)),
        )
        .child(push_fragments(
          line, message, text_color, action, &panel, &mut image,
        ))
    });
    let scroll = card.scroll.clone();
    Some(
      v_flex()
        .id("chat-card")
        .absolute()
        .top_2()
        .left_2()
        .right_2()
        .max_h(px(CARD_MAX_HEIGHT))
        .p_3()
        .gap_3()
        .rounded(theme::RADIUS_LG)
        .border_1()
        .border_color(theme::border())
        .bg(theme::surface())
        .shadow_lg()
        .occlude()
        .child(head)
        .child(
          h_flex()
            .gap_2()
            .items_center()
            .children(open)
            .when(looking, |el| {
              el.child(crate::widgets::loader(16., theme::muted()))
            }),
        )
        .when(!messages.is_empty(), |el| {
          el.child(
            v_flex()
              .id("chat-card-messages")
              .flex_1()
              .min_h_0()
              .pt_2()
              .border_t_1()
              .border_color(theme::border())
              .overflow_y_scroll()
              .track_scroll(&scroll)
              .children(lines),
          )
        })
        .into_any_element(),
    )
  }
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::*;

  #[test]
  fn times_show_their_day_unless_today() {
    let now = SystemTime::now();
    let today = Local::now().date_naive();
    let shown = sent_at(now, today);
    assert_eq!(shown.len(), 5, "{shown}");
    assert_eq!(&shown[2..3], ":");
    // Two days ago: its day (month first, in English) before it.
    let before = sent_at(now - Duration::from_secs(2 * 24 * 3600), today);
    assert_eq!(before.len(), 11, "{before}");
    assert_eq!(&before[2..3], "/");
  }
}
