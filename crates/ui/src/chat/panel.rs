//! The chat panel: a virtual list of variable-height messages (`gpui::list`) that follows the
//! newest, with emotes drawn inline. Scrolled up, it stops following and holds the new messages
//! until "Resume scrolling", so that what is read does not move. With a composer, the viewer
//! writes under it ([`composer`]).

mod composer;

use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  Anchor, AnyElement, App, Bounds, Context, Corners, ElementId, FollowMode, FontWeight,
  ImageSource, ListAlignment, ListState, ObjectFit, Pixels, RenderImage, SharedString,
  StyleRefinement, Window, canvas, div, img, list, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::hover_card::HoverCard;
use gpui_component::{Sizable, h_flex, v_flex};

pub use composer::{ChatPanelEvent, Composing, EmoteGroup};

use super::{
  Badge, ChatEvent, ChatMessage, EMOTE_HEIGHT, Emote, Fragment, MessageKind, name_color,
};
use crate::assets::Icon;
use crate::i18n::t;
use crate::images::Images;
use crate::picture::Picture;
use crate::{tf, theme};

/// Messages kept; older ones are dropped (and held ones beyond it, while scrolled up).
pub const MAX_MESSAGES: usize = 500;
/// Hover time before the room's modes show.
const MODES_DELAY: Duration = Duration::from_millis(200);
/// Size of a badge, logical pixels.
const BADGE_SIZE: f32 = 18.;
/// Hover time before an emote's preview shows.
const PREVIEW_DELAY: Duration = Duration::from_millis(300);
/// Height of an emote in its preview, logical pixels (Twitch's 3.0 size, 7TV's 4x).
const PREVIEW_HEIGHT: f32 = 112.;
/// Width an emote is drawn at, [`EMOTE_HEIGHT`] high.
fn emote_width(emote: &Emote) -> f32 {
  EMOTE_HEIGHT * emote.aspect.max(0.1)
}

/// An emote's image, its frame picked from the time when it is animated: GPUI's `img` plays
/// frames only while the window is active, and the chat goes on beside a video watched from
/// another window. Not animated with reduced motion.
fn emote_image(render: Arc<RenderImage>) -> gpui::Canvas<()> {
  canvas(
    |_, _, _| {},
    move |bounds: Bounds<Pixels>, (), window: &mut Window, cx: &mut App| {
      let animated = render.frame_count() > 1 && !cx.reduce_motion();
      let frame = if animated {
        window.request_animation_frame();
        frame_at(&render, clock())
      } else {
        0
      };
      let fitted = ObjectFit::Contain.get_bounds(bounds, render.size(frame));
      // A frame that cannot be drawn leaves the emote's place empty.
      window
        .paint_image(
          bounds,
          fitted,
          Corners::default(),
          render.clone(),
          frame,
          false,
        )
        .ok();
    },
  )
}

/// An emote and the overlays drawn on it, `height` high and centered on the widest; `image`
/// gives a layer's image for a width and a height in logical pixels.
fn emote_stack<'a>(
  layers: impl IntoIterator<Item = &'a Emote>,
  height: f32,
  mut image: impl FnMut(&Picture, f32, f32) -> Option<Arc<RenderImage>>,
) -> gpui::Div {
  let layers: Vec<&Emote> = layers.into_iter().collect();
  let scale = height / EMOTE_HEIGHT;
  let width = layers.first().map_or(0., |e| emote_width(e) * scale);
  let mut drawn = div().relative().w(px(width)).h(px(height)).flex_none();
  for layer in layers {
    let layer_width = emote_width(layer) * scale;
    if let Some(render) = image(&layer.picture, layer_width, height) {
      drawn = drawn.child(
        emote_image(render)
          .absolute()
          .bottom_0()
          .left(px((width - layer_width) / 2.))
          .w(px(layer_width))
          .h(px(height)),
      );
    }
  }
  drawn
}

/// An emote's preview on hover: it at [`PREVIEW_HEIGHT`] with its code, and its overlays' codes.
fn emote_preview(layers: &[Emote], window: &mut Window, cx: &mut App) -> gpui::Div {
  let scale = window.scale_factor();
  let images = cx.try_global::<Images>().cloned();
  let stack = emote_stack(layers, PREVIEW_HEIGHT, |picture, width, height| {
    let images = images.as_ref()?;
    let url = picture.url(
      (width * scale).round() as u32,
      (height * scale).round() as u32,
    )?;
    images.0.update(cx, |store, cx| store.get(&url, cx))
  });
  v_flex()
    .items_center()
    .gap_2()
    .p_1()
    .child(stack)
    .children(layers.first().map(|emote| {
      div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(emote.name.clone())
    }))
    .when(layers.len() > 1, |el| {
      el.child(
        div().text_xs().text_color(theme::muted()).child(
          layers[1..]
            .iter()
            .map(|e| e.name.as_ref())
            .collect::<Vec<_>>()
            .join(" · "),
        ),
      )
    })
}

/// Time since the first animated emote: copies of an emote play in step, as on Twitch.
fn clock() -> Duration {
  static START: OnceLock<Instant> = OnceLock::new();
  START.get_or_init(Instant::now).elapsed()
}

/// The frame of `image` shown `elapsed` into its loop. Delays of 10 ms or less count as 100 ms,
/// as browsers do (GIFs that say 0 mean "as fast as possible").
fn frame_at(image: &RenderImage, elapsed: Duration) -> usize {
  let delays: Vec<u128> = (0..image.frame_count())
    .map(|i| {
      let delay = Duration::from(image.delay(i)).as_millis();
      if delay <= 10 { 100 } else { delay }
    })
    .collect();
  let total: u128 = delays.iter().sum();
  if total == 0 {
    return 0;
  }
  let mut at = elapsed.as_millis() % total;
  for (index, delay) in delays.iter().enumerate() {
    if at < *delay {
      return index;
    }
    at -= delay;
  }
  0
}

/// Height kept above and below the visible messages, laid out ahead of a scroll.
const OVERDRAW: f32 = 400.;
/// How far a new line slides in from the right, logical pixels.
const ENTER_SLIDE: f32 = 10.;
/// How long a new line takes to slide and fade in.
const ENTER: Duration = theme::NORMAL;
/// Lines arriving together beyond this many (the history on joining) show at once.
const ENTER_MAX_BATCH: usize = 30;
/// Time constant of the glide toward the newest line: the distance left shrinks by e every
/// this long, so a line rises in about 3 of them, and a fast chat scrolls steadily.
const GLIDE_TAU: f32 = 0.06;
/// The glide never lags further behind the newest line than this, logical pixels: the rest is
/// jumped at once.
const GLIDE_MAX_LAG: f32 = 160.;

/// A smooth scroll toward the end of the list, see [`ChatPanel::glide`].
struct Glide {
  /// The previous frame, for the step's length.
  last: Instant,
  /// Frames in a row that found nothing left to scroll.
  idle: u8,
}

/// The chat panel: see the module documentation. Its page feeds it with [`ChatPanel::push`]. It
/// draws no background nor frame: its container does (`watch_layout`).
pub struct ChatPanel {
  messages: VecDeque<ChatMessage>,
  /// When each of `messages` arrived, for its entrance; `None` shows it at once.
  arrived: VecDeque<Option<Instant>>,
  /// Arrived while scrolled up, shown on resuming.
  held: VecDeque<ChatMessage>,
  list: ListState,
  /// Following, but scrolling smoothly to the newest lines instead of jumping there: the
  /// list's own following is paused meanwhile, and comes back on reaching the end.
  glide: Option<Glide>,
  modes: Vec<SharedString>,
  status: Option<SharedString>,
  title: SharedString,
  /// The writing area, for a space that sends to its chat.
  composer: Option<composer::Composer>,
}

impl gpui::EventEmitter<ChatPanelEvent> for ChatPanel {}

impl ChatPanel {
  /// An empty panel, connecting.
  pub fn new(cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let list = ListState::new(0, ListAlignment::Top, px(OVERDRAW));
    list.set_follow_mode(FollowMode::Tail);
    // The wheel takes over from a glide (the list's own scrolls do not call this).
    let panel = cx.entity().downgrade();
    list.set_scroll_handler(move |_, _, cx| {
      if let Some(panel) = panel.upgrade() {
        panel.update(cx, |panel, _| panel.glide = None);
      }
    });
    Self {
      messages: VecDeque::new(),
      arrived: VecDeque::new(),
      held: VecDeque::new(),
      list,
      glide: None,
      modes: Vec::new(),
      status: Some(t("Connecting to chat…", "Connexion au chat…").into()),
      title: t("Live chat", "Chat du live").into(),
      composer: None,
    }
  }

  /// The panel with another title than "Live chat" (a replay).
  pub fn titled(mut self, title: impl Into<SharedString>) -> Self {
    self.title = title.into();
    self
  }

  /// Applies a batch of events: give it everything that arrived since the last frame, so that
  /// the list is spliced once.
  pub fn push(&mut self, events: impl IntoIterator<Item = ChatEvent>, cx: &mut Context<Self>) {
    let following = self.following();
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
          self.arrived.clear();
          self.held.clear();
          self.list.reset(0);
          self.list.set_follow_mode(FollowMode::Tail);
          self.glide = None;
          added = 0;
        }
        ChatEvent::State(modes) => self.modes = modes,
        ChatEvent::Status(status) => self.status = status,
      }
    }
    if added > 0 {
      let enter = (added <= ENTER_MAX_BATCH).then(Instant::now);
      // Glide to them (the history on joining is jumped to): pausing before the splice keeps
      // the lines on screen where they are, the glide then brings the new ones up.
      if enter.is_some() && !cx.reduce_motion() && self.glide.is_none() {
        self.list.pause_following_tail();
        self.glide = Some(Glide {
          last: Instant::now(),
          idle: 0,
        });
      }
      self.arrived.extend(std::iter::repeat_n(enter, added));
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
    // Shown at once: they come in together, where the list jumps to.
    self.arrived.extend(std::iter::repeat_n(None, count));
    self.list.splice(start..start, count);
  }

  /// Drops the oldest messages beyond [`MAX_MESSAGES`]: only while following, so that the
  /// messages being read stay put.
  fn trim(&mut self) {
    let extra = self.messages.len().saturating_sub(MAX_MESSAGES);
    if extra > 0 {
      self.messages.drain(..extra);
      self.arrived.drain(..extra);
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

  /// Whether new lines join the list: following its end, or gliding to it.
  fn following(&self) -> bool {
    self.glide.is_some() || self.list.is_following_tail()
  }

  /// One frame of the glide: a step toward the end, easing out. It ends when the list
  /// follows its end again, which it does on its own on reaching it.
  fn step_glide(&mut self, window: &mut Window) {
    let Some(glide) = self.glide.as_mut() else {
      return;
    };
    if self.list.is_following_tail() {
      self.glide = None;
      return;
    }
    let now = Instant::now();
    let dt = now.duration_since(glide.last).as_secs_f32().min(0.05);
    glide.last = now;
    let end = self.list.max_offset_for_scrollbar().y;
    let at = -self.list.scroll_px_offset_for_scrollbar().y;
    let left = f32::from(end - at);
    if left <= 0.5 {
      // Lines just spliced are measured by the next layout: nothing is left before it. Two
      // frames in a row with nothing left end the glide.
      glide.idle += 1;
      if glide.idle >= 2 {
        self.glide = None;
        self.list.set_follow_mode(FollowMode::Tail);
        return;
      }
      if left > 0. {
        self.list.scroll_by(px(left));
      }
    } else {
      glide.idle = 0;
      let lag = (left - GLIDE_MAX_LAG).max(0.);
      let step = (left - lag) * (1. - (-dt / GLIDE_TAU).exp());
      self.list.scroll_by(px(lag + step.max(0.5).min(left - lag)));
    }
    window.request_animation_frame();
  }

  fn resume(&mut self, cx: &mut Context<Self>) {
    self.resume_held();
    self.trim();
    self.glide = None;
    self.list.set_follow_mode(FollowMode::Tail);
    cx.notify();
  }

  fn render_message(
    &mut self,
    ix: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    // A new line fades in sliding a few pixels from the right (none with reduced motion).
    let enter = match self.arrived.get(ix).copied().flatten() {
      Some(arrived) if !cx.reduce_motion() => {
        let t = (arrived.elapsed().as_secs_f32() / ENTER.as_secs_f32()).min(1.);
        if t < 1. {
          window.request_animation_frame();
        } else if let Some(slot) = self.arrived.get_mut(ix) {
          *slot = None;
        }
        1. - (1. - t).powi(3)
      }
      _ => 1.,
    };
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
            tf!("{}: ", "{} : ", author.name)
          }),
      );
    }
    if message.deleted {
      text_color = theme::subtle();
    }
    let fragments = &message.fragments;
    let mut index = 0;
    while index < fragments.len() {
      match &fragments[index] {
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
        Fragment::Emote(emote) => {
          // The overlays that follow it (blank text between) are drawn on it.
          let mut overlays = Vec::new();
          let mut next = index + 1;
          if !emote.overlay {
            let mut look = next;
            loop {
              while matches!(fragments.get(look), Some(Fragment::Text(t)) if t.trim().is_empty()) {
                look += 1;
              }
              match fragments.get(look) {
                Some(Fragment::Emote(over)) if over.overlay => {
                  overlays.push(over);
                  look += 1;
                  next = look;
                }
                _ => break,
              }
            }
          }
          let layers: Vec<Emote> = std::iter::once(emote).chain(overlays).cloned().collect();
          let drawn = emote_stack(&layers, EMOTE_HEIGHT, &mut image)
            .when(message.deleted, |el| el.opacity(0.35));
          line = line.child(
            HoverCard::new(ElementId::NamedInteger(message.id.clone(), index as u64))
              .anchor(Anchor::BottomCenter)
              .bg(theme::surface().opacity(0.8))
              .open_delay(PREVIEW_DELAY)
              .close_delay(Duration::ZERO)
              .trigger_style(StyleRefinement::default().flex_none())
              .trigger(drawn)
              .content(move |_, window, cx| emote_preview(&layers, window, cx)),
          );
          index = next;
          continue;
        }
      }
      index += 1;
    }

    let item = div()
      .w_full()
      .px_3()
      .py(px(3.))
      .hover(|s| s.bg(theme::foreground().opacity(0.04)))
      .when(enter < 1., |el| {
        el.relative()
          .left(px(ENTER_SLIDE * (1. - enter)))
          .opacity(enter)
      });
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
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    self.step_glide(window);
    let following = self.following();
    let held = self.held.len();
    // The room's modes behind an "i" on the right, shown on hover: several do not fit beside the
    // title.
    let modes = self.modes.clone();
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
          .flex_1()
          .text_sm()
          .font_weight(FontWeight::SEMIBOLD)
          .child(self.title.clone()),
      )
      .when(!modes.is_empty(), |el| {
        el.child(
          HoverCard::new("chat-modes")
            .anchor(Anchor::TopRight)
            .open_delay(MODES_DELAY)
            .trigger(
              div()
                .text_color(theme::muted())
                .hover(|s| s.text_color(theme::foreground()))
                .child(gpui_component::Icon::new(Icon::Info).size(px(16.))),
            )
            .content(move |_, _, _| {
              v_flex()
                .gap_1()
                .child(
                  div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::muted())
                    .child(t("Chat modes", "Modes du chat")),
                )
                .children(modes.iter().map(|mode| div().text_sm().child(mode.clone())))
            }),
        )
      });
    let resume = (!following).then(|| {
      let label = match held {
        0 => t("Resume scrolling", "Reprendre le défilement").to_owned(),
        1 => t(
          "Resume scrolling · 1 new message",
          "Reprendre le défilement · 1 nouveau message",
        )
        .to_owned(),
        n => tf!(
          "Resume scrolling · {n} new messages",
          "Reprendre le défilement · {n} nouveaux messages"
        ),
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
    let composer = self.render_composer(window, cx);
    v_flex()
      .size_full()
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
          // Around the list rather than on it: the glide aims at the end from its items' height,
          // which leaves out the list's own padding.
          .py_1()
          .child(
            list(
              self.list.clone(),
              cx.processor(|this, ix, window, cx| this.render_message(ix, window, cx)),
            )
            .size_full(),
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
                .child(t("Welcome to the chat!", "Bienvenue dans le chat !")),
            )
          })
          .children(resume),
      )
      .children(composer)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn animation(delays_ms: &[u32]) -> RenderImage {
    let frames: Vec<image::Frame> = delays_ms
      .iter()
      .map(|&ms| {
        image::Frame::from_parts(
          image::RgbaImage::new(1, 1),
          0,
          0,
          image::Delay::from_numer_denom_ms(ms, 1),
        )
      })
      .collect();
    RenderImage::new(frames)
  }

  #[test]
  fn frames_follow_the_clock_and_loop() {
    let image = animation(&[50, 100, 50]);
    let at = |ms| frame_at(&image, Duration::from_millis(ms));
    assert_eq!((at(0), at(49), at(50), at(149), at(150)), (0, 0, 1, 1, 2));
    // 200 ms a loop.
    assert_eq!((at(200), at(260)), (0, 1));
    // A delay of 0 counts as 100 ms.
    let fast = animation(&[0, 0]);
    assert_eq!(frame_at(&fast, Duration::from_millis(150)), 1);
  }
}
