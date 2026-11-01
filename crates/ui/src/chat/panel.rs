//! The chat panel: a virtual list of variable-height messages (`gpui::list`) that follows the
//! newest, with emotes drawn inline. Scrolled up, it stops following and holds the new messages
//! until "Reprendre le défilement", so that what is read does not move.

use std::collections::VecDeque;

use gpui::prelude::*;
use gpui::{
  AnyElement, Context, FollowMode, FontWeight, ImageSource, ListAlignment, ListState, ObjectFit,
  SharedString, Window, div, img, list, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{Sizable, h_flex, v_flex};

use super::{
  Badge, ChatEvent, ChatMessage, EMOTE_HEIGHT, Emote, Fragment, MessageKind, name_color,
};
use crate::images::Images;
use crate::picture::Picture;
use crate::theme;

/// Messages kept; older ones are dropped (and held ones beyond it, while scrolled up).
pub const MAX_MESSAGES: usize = 500;
/// Size of a badge, logical pixels.
const BADGE_SIZE: f32 = 18.;
/// Height kept above and below the visible messages, laid out ahead of a scroll.
const OVERDRAW: f32 = 400.;

/// The chat panel: see the module documentation. Its page feeds it with [`ChatPanel::push`].
pub struct ChatPanel {
  messages: VecDeque<ChatMessage>,
  /// Arrived while scrolled up, shown on resuming.
  held: VecDeque<ChatMessage>,
  list: ListState,
  modes: Vec<SharedString>,
  status: Option<SharedString>,
}

impl ChatPanel {
  /// An empty panel, connecting.
  pub fn new(cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let list = ListState::new(0, ListAlignment::Top, px(OVERDRAW));
    list.set_follow_mode(FollowMode::Tail);
    Self {
      messages: VecDeque::new(),
      held: VecDeque::new(),
      list,
      modes: Vec::new(),
      status: Some("Connexion au chat…".into()),
    }
  }

  /// Applies a batch of events: give it everything that arrived since the last frame, so that
  /// the list is spliced once.
  pub fn push(&mut self, events: impl IntoIterator<Item = ChatEvent>, cx: &mut Context<Self>) {
    let following = self.list.is_following_tail();
    if following {
      self.resume_held();
    }
    let mut changed = false;
    let mut added = 0;
    for event in events {
      changed = true;
      match event {
        ChatEvent::Message(message) if following => {
          self.messages.push_back(message);
          added += 1;
        }
        ChatEvent::Message(message) => {
          self.held.push_back(message);
          if self.held.len() > MAX_MESSAGES {
            self.held.pop_front();
          }
        }
        ChatEvent::Delete(id) => self.mark_deleted(|m| m.id == id),
        ChatEvent::ClearUser(user) => {
          self.mark_deleted(|m| m.author.as_ref().is_some_and(|a| a.id == user))
        }
        ChatEvent::Clear => {
          self.messages.clear();
          self.held.clear();
          self.list.reset(0);
          added = 0;
        }
        ChatEvent::State(modes) => self.modes = modes,
        ChatEvent::Status(status) => self.status = status,
      }
    }
    if added > 0 {
      let end = self.messages.len();
      self.list.splice(end - added..end - added, added);
    }
    if following {
      self.trim();
    }
    if changed {
      cx.notify();
    }
  }

  /// Moves the held messages into the list.
  fn resume_held(&mut self) {
    if self.held.is_empty() {
      return;
    }
    let count = self.held.len();
    let start = self.messages.len();
    self.messages.append(&mut self.held);
    self.list.splice(start..start, count);
  }

  /// Drops the oldest messages beyond [`MAX_MESSAGES`]: only while following, so that the
  /// messages being read stay put.
  fn trim(&mut self) {
    let extra = self.messages.len().saturating_sub(MAX_MESSAGES);
    if extra > 0 {
      self.messages.drain(..extra);
      self.list.splice(0..extra, 0);
    }
  }

  fn mark_deleted(&mut self, matches: impl Fn(&ChatMessage) -> bool) {
    for message in self.messages.iter_mut().chain(self.held.iter_mut()) {
      if matches(message) {
        message.deleted = true;
      }
    }
  }

  fn resume(&mut self, cx: &mut Context<Self>) {
    self.resume_held();
    self.trim();
    self.list.set_follow_mode(FollowMode::Tail);
    cx.notify();
  }

  fn render_message(
    &mut self,
    ix: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let Some(message) = self.messages.get(ix) else {
      return div().into_any_element();
    };
    let scale = window.scale_factor();
    let images = cx.try_global::<Images>().cloned();
    let mut image = |picture: &Picture, width: f32, height: f32| {
      let images = images.as_ref()?;
      let url = picture.url(
        (width * scale).round() as u32,
        (height * scale).round() as u32,
      )?;
      images.0.update(cx, |store, cx| store.get(&url, cx))
    };

    let mut line = h_flex().flex_wrap().items_center().text_sm().min_w_0();
    let action = message.kind == MessageKind::Action;
    let mut text_color = theme::foreground();
    if let Some(author) = &message.author {
      for Badge { picture, .. } in &author.badges {
        line = line.child(
          div().size(px(BADGE_SIZE)).flex_none().mr_1().children(
            image(picture, BADGE_SIZE, BADGE_SIZE)
              .map(|render| img(ImageSource::Render(render)).size(px(BADGE_SIZE))),
          ),
        );
      }
      let color = name_color(author);
      if action {
        text_color = color;
      }
      line = line.child(
        div()
          .font_weight(FontWeight::BOLD)
          .text_color(color)
          .child(if action {
            format!("{} ", author.name)
          } else {
            format!("{} : ", author.name)
          }),
      );
    }
    if message.deleted {
      text_color = theme::subtle();
    }
    for fragment in &message.fragments {
      match fragment {
        Fragment::Text(text) => {
          for word in text.split_inclusive(' ') {
            line = line.child(
              div()
                .max_w_full()
                .text_color(text_color)
                .when(action, |el| el.italic())
                .child(SharedString::from(word.to_owned())),
            );
          }
        }
        Fragment::Mention(name) => {
          line = line.child(
            div()
              .font_weight(FontWeight::SEMIBOLD)
              .text_color(if message.deleted {
                theme::subtle()
              } else {
                theme::foreground()
              })
              .child(name.clone()),
          );
        }
        Fragment::Link(url) => {
          line = line.child(
            div()
              .max_w_full()
              .text_color(if message.deleted {
                theme::subtle()
              } else {
                theme::accent_hover()
              })
              .underline()
              .child(url.clone()),
          );
        }
        Fragment::Emote(Emote {
          picture, aspect, ..
        }) => {
          let width = EMOTE_HEIGHT * aspect.max(0.1);
          line = line.child(
            div()
              .w(px(width))
              .h(px(EMOTE_HEIGHT))
              .flex_none()
              .when(message.deleted, |el| el.opacity(0.35))
              .children(image(picture, width, EMOTE_HEIGHT).map(|render| {
                img(ImageSource::Render(render))
                  .w(px(width))
                  .h(px(EMOTE_HEIGHT))
                  // Else the image's own ratio wins over the height.
                  .aspect_ratio(width / EMOTE_HEIGHT)
                  .object_fit(ObjectFit::Contain)
              })),
          );
        }
      }
    }

    let item = div().w_full().px_3().py(px(3.));
    match &message.kind {
      MessageKind::Notice(text) => item
        .child(
          v_flex()
            .gap_1()
            .px_2()
            .py_1p5()
            .rounded(theme::RADIUS)
            .border_l_2()
            .border_color(theme::accent())
            .bg(theme::surface_hover())
            .child(
              div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::muted())
                .child(text.clone()),
            )
            .when(!message.fragments.is_empty(), |el| el.child(line)),
        )
        .into_any_element(),
      MessageKind::Highlighted => item
        .child(
          div()
            .px_2()
            .py_1()
            .rounded(theme::RADIUS)
            .bg(theme::accent_soft())
            .child(line),
        )
        .into_any_element(),
      MessageKind::Normal | MessageKind::Action => item.child(line).into_any_element(),
    }
  }
}

impl Render for ChatPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let following = self.list.is_following_tail();
    let held = self.held.len();
    let header = h_flex()
      .flex_none()
      .gap_2()
      .px_3()
      .py_2()
      .items_center()
      .border_b_1()
      .border_color(theme::border())
      .child(
        div()
          .text_sm()
          .font_weight(FontWeight::SEMIBOLD)
          .child("Chat du live"),
      )
      .child(div().flex_1())
      .children(self.modes.iter().map(|mode| {
        div()
          .px_1p5()
          .rounded(px(4.))
          .bg(theme::surface_hover())
          .text_xs()
          .text_color(theme::muted())
          .child(mode.clone())
      }));
    let resume = (!following).then(|| {
      let label = match held {
        0 => "Reprendre le défilement".to_owned(),
        1 => "Reprendre le défilement · 1 nouveau message".to_owned(),
        n => format!("Reprendre le défilement · {n} nouveaux messages"),
      };
      h_flex()
        .absolute()
        .bottom_3()
        .left_0()
        .right_0()
        .justify_center()
        .child(
          Button::new("chat-resume")
            .primary()
            .small()
            .label(label)
            .on_click(cx.listener(|this, _, _, cx| this.resume(cx))),
        )
    });
    let empty = self.messages.is_empty() && held == 0;
    v_flex()
      .size_full()
      .rounded(theme::RADIUS_LG)
      .bg(theme::surface())
      .border_1()
      .border_color(theme::border())
      .overflow_hidden()
      .child(header)
      .children(self.status.clone().map(|status| {
        div()
          .flex_none()
          .px_3()
          .py_1()
          .text_xs()
          .text_color(theme::muted())
          .child(status)
      }))
      .child(
        div()
          .relative()
          .flex_1()
          .min_h_0()
          .child(
            list(
              self.list.clone(),
              cx.processor(|this, ix, window, cx| this.render_message(ix, window, cx)),
            )
            .size_full()
            .py_1(),
          )
          .when(empty && self.status.is_none(), |el| {
            el.child(
              div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme::subtle())
                .child("Bienvenue dans le chat !"),
            )
          })
          .children(resume),
      )
  }
}
