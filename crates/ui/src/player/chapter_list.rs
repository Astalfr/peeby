//! A list of chapters, one row each: its picture (a game's box art) when the list has some, its
//! start time in a column as wide as the longest one, and its title, wrapped on two lines at
//! most. The player's chapters menu draws it, and so can a space's cards (a Twitch VOD's
//! chapters on the channel page).

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, Div, Pixels, SharedString, Window, div, px, rems};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{h_flex, v_flex};

use super::format_time;
use crate::picture::{Picture, image_fill};

/// Width of a chapter's picture.
const PICTURE_WIDTH: f32 = 24.;

/// Widest a chapter's title gets before it wraps.
const TITLE_MAX_WIDTH: f32 = 220.;

/// Width / height of a chapter's picture: Twitch's box art (52×72).
pub const PICTURE_ASPECT: f32 = 52. / 72.;

/// A chapter as a list shows it.
#[derive(Clone)]
pub struct ChapterItem {
  /// Start, seconds.
  pub start: f64,
  /// Title.
  pub title: String,
  /// Picture, if the source gave one.
  pub picture: Option<Picture>,
}

/// The rows of `items`, ids from `id`; `current` is highlighted, and a click calls `on_select`
/// with the row's index.
pub fn chapter_list(
  id: &str,
  items: &[ChapterItem],
  current: Option<usize>,
  on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
  window: &mut Window,
  cx: &mut App,
) -> Div {
  let on_select = Rc::new(on_select);
  let pictures = items.iter().any(|item| item.picture.is_some());
  // The times as wide as the longest one (`0:00` and `1:00:04`), so that the titles line up.
  let time_width = {
    let style = window.text_style();
    // A button's text size.
    let size = rems(0.875).to_pixels(window.rem_size());
    items
      .iter()
      .map(|item| {
        let time = format_time(item.start);
        let run = style.to_run(time.len());
        window
          .text_system()
          .shape_line(time.into(), size, &[run], None)
          .width
      })
      .fold(px(0.), Pixels::max)
  };
  let mut list = v_flex().gap_1();
  for (i, item) in items.iter().enumerate() {
    // `line_clamp` only counts wrapped lines: a line break would add more.
    let title = item.title.replace(['\r', '\n'], " ");
    let picture = pictures.then(|| {
      let picture = item.picture.clone().unwrap_or(Picture::Sizes(Vec::new()));
      div()
        .flex_shrink_0()
        .w(px(PICTURE_WIDTH))
        .h(px(PICTURE_WIDTH / PICTURE_ASPECT))
        .child(image_fill(
          &picture,
          PICTURE_WIDTH,
          PICTURE_ASPECT,
          px(2.),
          window,
          cx,
        ))
    });
    let on_select = on_select.clone();
    // A title wraps on two lines at most; the row grows with it.
    let button = Button::new(SharedString::from(format!("{id}-{i}")))
      .h_auto()
      .min_h_8()
      .py_1()
      .child(
        h_flex()
          .flex_1()
          .min_w_0()
          .gap_2()
          .children(picture)
          .child(
            div()
              .flex_shrink_0()
              .w(time_width)
              .whitespace_nowrap()
              .child(format_time(item.start)),
          )
          .child(
            div()
              .min_w_0()
              .max_w(px(TITLE_MAX_WIDTH))
              .whitespace_normal()
              .line_clamp(2)
              .child(title),
          ),
      )
      .when(current == Some(i), |b| b.primary())
      .when(current != Some(i), |b| b.ghost())
      .on_click(move |_, window, cx| on_select(i, window, cx));
    list = list.child(button);
  }
  list
}
