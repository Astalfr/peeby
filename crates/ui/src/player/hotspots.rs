//! Links a source places over the picture (docs/player/hotspots.md): teasers that slide in at a
//! moment of the video ([`Teaser`], YouTube's cards) and boxes over the picture between two
//! moments ([`Hotspot`], YouTube's end screens). The space gives them with what a click does;
//! the player only places and shows them.

use std::rc::Rc;

use gpui::AnimationExt as _;
use gpui::prelude::*;
use gpui::{AnyElement, App, Context, FontWeight, SharedString, Window, div, px};
use gpui_component::{ActiveTheme, h_flex, v_flex};

use super::PlayerView;
use crate::picture::{self, Picture};

/// What a click on a teaser or a hotspot does (open a page, a video, a site).
pub type OnPick = Rc<dyn Fn(&mut Window, &mut App)>;

/// A link announced at a moment of the video: from [`Teaser::at`], it slides in at the top right
/// for [`TEASER_FOR`] seconds; a click follows it.
#[derive(Clone)]
pub struct Teaser {
  /// When it shows, seconds.
  pub at: f64,
  /// Its text.
  pub title: SharedString,
  /// Under it (a video's channel, a channel's subscribers).
  pub subtitle: Option<SharedString>,
  /// Its picture, left of the text.
  pub picture: Option<Picture>,
  /// The picture is an avatar: round and square, else 16:9.
  pub round: bool,
  /// What a click does.
  pub on_click: OnPick,
}

/// A box over the picture from [`Hotspot::start`] to [`Hotspot::end`], placed and sized as
/// shares of the picture (not of the player: its letterbox bars are left out).
#[derive(Clone)]
pub struct Hotspot {
  /// When it shows, seconds.
  pub start: f64,
  /// When it leaves, seconds.
  pub end: f64,
  /// Its left edge, as a share of the picture's width.
  pub left: f32,
  /// Its top edge, as a share of the picture's height.
  pub top: f32,
  /// Its width, as a share of the picture's width.
  pub width: f32,
  /// Its width over its height.
  pub aspect_ratio: f32,
  /// Its title, shown while the pointer is over it.
  pub title: SharedString,
  /// Under the title.
  pub subtitle: Option<SharedString>,
  /// In its bottom right corner (a video's length).
  pub badge: Option<SharedString>,
  /// What fills it.
  pub picture: Option<Picture>,
  /// Round (an avatar, a site's icon), else a rounded rectangle.
  pub round: bool,
  /// What a click does.
  pub on_click: OnPick,
}

/// How long a teaser stays, seconds.
pub const TEASER_FOR: f64 = 6.0;
const TEASER_WIDTH: f32 = 300.;
const TEASER_THUMB: (f32, f32) = (80., 45.);
/// The teaser's distance from the player's top edge: with the controls hidden, then shown
/// (under the title bar or the top bar).
const TEASER_TOP: (f32, f32) = (12., 68.);
const TEASER_RIGHT: f32 = 12.;
/// How far right the teaser comes from and goes back to: wholly out of the player.
const TEASER_AWAY: f32 = TEASER_RIGHT + TEASER_WIDTH;

impl PlayerView {
  /// The teasers of this video (none: `Vec::new()`), shown as playback reaches them.
  pub fn set_teasers(&mut self, teasers: Vec<Teaser>, cx: &mut Context<Self>) {
    self.teasers = teasers;
    cx.notify();
  }

  /// The hotspots of this video (none: `Vec::new()`), shown over the picture in their time.
  pub fn set_hotspots(&mut self, hotspots: Vec<Hotspot>, cx: &mut Context<Self>) {
    self.hotspots = hotspots;
    self.video_aspect = None;
    cx.notify();
  }

  /// The teaser whose time it is, if any: the latest one reached less than [`TEASER_FOR`] ago.
  /// The latest one reached, while it shows or leaves: whether it is leaving (its time is up,
  /// for as long as its way out takes).
  fn current_teaser(&self) -> Option<(usize, &Teaser, bool)> {
    if self.is_live() || self.in_pip {
      return None;
    }
    let position = self.position;
    let leave = crate::theme::SLOW.as_secs_f64();
    let (index, teaser) = self
      .teasers
      .iter()
      .enumerate()
      .filter(|(_, t)| t.at <= position && position < t.at + TEASER_FOR + leave)
      .max_by(|a, b| a.1.at.total_cmp(&b.1.at))?;
    Some((index, teaser, position >= teaser.at + TEASER_FOR))
  }

  /// The current teaser at the top right, as the "now playing" card at the top left: it slides
  /// in from beyond the player's right edge and back there when its time is up (at once with
  /// reduced motion), clipped at that edge; moved under the title bar with the controls' fade
  /// while they show.
  pub(super) fn render_teaser(
    &self,
    show_ui: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let (index, teaser, leaving) = self.current_teaser()?;
    let theme = cx.theme();
    let (popover, border, muted) = (theme.popover, theme.border, theme.muted_foreground);
    let thumbnail = teaser.picture.as_ref().map(|p| {
      if teaser.round {
        picture::avatar(p, TEASER_THUMB.1, window, cx)
      } else {
        div()
          .w(px(TEASER_THUMB.0))
          .h(px(TEASER_THUMB.1))
          .flex_none()
          .child(picture::image_fill(
            p,
            TEASER_THUMB.0,
            TEASER_THUMB.0 / TEASER_THUMB.1,
            px(4.),
            window,
            cx,
          ))
          .into_any_element()
      }
    });
    let on_click = teaser.on_click.clone();
    let card = h_flex()
      .id(SharedString::from(format!("teaser-{index}")))
      .occlude()
      .w(px(TEASER_WIDTH))
      .p_2()
      .gap_2p5()
      .items_center()
      .rounded_lg()
      .bg(popover)
      .border_1()
      .border_color(border)
      .cursor_pointer()
      .hover(|s| s.border_color(gpui::white().opacity(0.4)))
      .on_click(move |_, window, cx| {
        cx.stop_propagation();
        on_click(window, cx);
      })
      .children(thumbnail)
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .child(
            div()
              .text_sm()
              .line_height(px(18.))
              .font_weight(FontWeight::SEMIBOLD)
              .line_clamp(2)
              .child(teaser.title.clone()),
          )
          .children(
            teaser
              .subtitle
              .clone()
              .map(|s| div().text_xs().text_color(muted).truncate().child(s)),
          ),
      );
    let card = if cx.reduce_motion() {
      card.when(leaving, |el| el.invisible()).into_any_element()
    } else {
      card
        .with_animation(
          SharedString::from(format!("teaser-{index}-{leaving}")),
          gpui::Animation::new(crate::theme::SLOW).with_easing(move |t| {
            // Leaving speeds up as it goes (ease in), coming slows down (ease out).
            if leaving {
              t * t * t
            } else {
              gpui::ease_out_quint()(t)
            }
          }),
          // From beyond the player's right edge, and back there.
          move |el, t| {
            let out = if leaving { t } else { 1. - t };
            el.left(px(TEASER_AWAY * out))
          },
        )
        .into_any_element()
    };
    let shown = self.under_top_bar(show_ui, window, cx);
    let top = TEASER_TOP.0 + (TEASER_TOP.1 - TEASER_TOP.0) * shown;
    Some(
      // Clipped at the player's right edge: the page beside it never shows the card.
      div()
        .absolute()
        .right_0()
        .top(px(top))
        .w(px(TEASER_RIGHT + TEASER_WIDTH))
        .overflow_hidden()
        .pr(px(TEASER_RIGHT))
        .child(card)
        .into_any_element(),
    )
  }

  /// The hotspots whose time it is, placed over the picture: the player's box less mpv's
  /// letterbox bars, from the picture's aspect (read from mpv once they first show).
  pub(super) fn render_hotspots(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    if self.is_live() || self.in_pip || self.player.is_none() {
      return None;
    }
    let position = self.position;
    let shown: Vec<(usize, Hotspot)> = self
      .hotspots
      .iter()
      .enumerate()
      .filter(|(_, h)| h.start <= position && position < h.end)
      .map(|(i, h)| (i, h.clone()))
      .collect();
    if shown.is_empty() {
      return None;
    }
    if self.video_aspect.is_none() {
      self.video_aspect = self
        .property("video-params/aspect")
        .and_then(|a| a.parse::<f32>().ok())
        .filter(|a| a.is_finite() && *a > 0.);
    }
    let size = self.bounds.get().size;
    let (width, height) = (f32::from(size.width), f32::from(size.height));
    let (left, top, picture_width, picture_height) =
      fit(width, height, self.video_aspect.unwrap_or(16. / 9.));
    let theme = cx.theme();
    let (popover, muted) = (theme.popover, theme.muted_foreground);
    let reduced = cx.reduce_motion();
    let boxes = shown.into_iter().map(|(index, hotspot)| {
      let w = hotspot.width * picture_width;
      let h = w / hotspot.aspect_ratio;
      let radius = if hotspot.round { w / 2. } else { 6. };
      let group = SharedString::from(format!("hotspot-{index}"));
      let fill = hotspot
        .picture
        .as_ref()
        .map(|p| picture::image_fill(p, w, hotspot.aspect_ratio, px(radius), window, cx));
      let on_click = hotspot.on_click.clone();
      // The title over a video's top edge, beside a round one: shown while hovered.
      let label = v_flex()
        .absolute()
        .when(hotspot.round, |el| {
          el.top(px(h + 6.)).left(px((w - 220.) / 2.)).w(px(220.))
        })
        .when(!hotspot.round, |el| {
          el.top(px(4.)).left(px(4.)).right(px(4.))
        })
        .px_2()
        .py_1()
        .rounded_md()
        .bg(popover.opacity(0.92))
        .opacity(0.)
        .group_hover(group.clone(), |s| s.opacity(1.))
        .child(
          div()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .line_clamp(2)
            .child(hotspot.title.clone()),
        )
        .children(
          hotspot
            .subtitle
            .clone()
            .map(|s| div().text_xs().text_color(muted).truncate().child(s)),
        );
      let badge = hotspot.badge.clone().map(|b| {
        div()
          .absolute()
          .bottom(px(4.))
          .right(px(4.))
          .px_1()
          .rounded_sm()
          .bg(gpui::black().opacity(0.8))
          .text_xs()
          .text_color(gpui::white())
          .child(b)
      });
      let item = div()
        .id(group.clone())
        .group(group.clone())
        .occlude()
        .absolute()
        .left(px(left + hotspot.left * picture_width))
        .top(px(top + hotspot.top * picture_height))
        .w(px(w))
        .h(px(h))
        .cursor_pointer()
        .rounded(px(radius))
        .border_1()
        .border_color(gpui::white().opacity(0.15))
        .hover(|s| s.border_color(gpui::white().opacity(0.8)))
        .on_click(move |_, window, cx| {
          cx.stop_propagation();
          on_click(window, cx);
        })
        .children(fill)
        .children(badge)
        .child(label);
      if reduced {
        item.into_any_element()
      } else {
        item
          .with_animation(
            SharedString::from(format!("hotspot-in-{index}")),
            gpui::Animation::new(crate::theme::NORMAL).with_easing(gpui::ease_out_quint()),
            |el, t| el.opacity(t),
          )
          .into_any_element()
      }
    });
    Some(
      div()
        .absolute()
        .inset_0()
        .children(boxes)
        .into_any_element(),
    )
  }
}

/// The picture of `aspect` (width over height) fitted in a `width`×`height` box, as mpv draws it
/// letterboxed: its left, top, width and height.
fn fit(width: f32, height: f32, aspect: f32) -> (f32, f32, f32, f32) {
  if width <= 0. || height <= 0. {
    return (0., 0., width.max(0.), height.max(0.));
  }
  if width / height > aspect {
    let w = height * aspect;
    ((width - w) / 2., 0., w, height)
  } else {
    let h = width / aspect;
    (0., (height - h) / 2., width, h)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_picture_fits_between_its_letterbox_bars() {
    // 16:9 in a 16:9 box fills it.
    assert_eq!(fit(1600., 900., 16. / 9.), (0., 0., 1600., 900.));
    // 4:3 in a wider box: bars left and right.
    assert_eq!(fit(1600., 900., 4. / 3.), (200., 0., 1200., 900.));
    // 21:9-ish in a 16:9 box: bars above and below.
    assert_eq!(fit(1600., 900., 2.), (0., 50., 1600., 800.));
  }
}
