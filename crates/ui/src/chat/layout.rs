//! The watch page's layout with a chat column, shared by the spaces that play lives:
//! - normally, the player (16:9) and what goes under it scroll on the left, and the chat fills the
//!   page's height on the right;
//! - in theater mode, the player takes the page's width less the chat's, the chat beside it at its
//!   height, and what goes under both scrolls.
//!
//! Without a chat, the player and what goes under it take the whole width.

use gpui::prelude::*;
use gpui::{AnyElement, Pixels, Size, div, px, relative};
use gpui_component::{h_flex, v_flex};

use crate::theme;

/// Width of the chat column.
pub const CHAT_WIDTH: f32 = 340.;
/// Theater mode: room left under the player, and its least height (as YouTube's watch page).
const THEATER_BELOW: f32 = 120.;
const THEATER_MIN: f32 = 240.;

/// The page: `player` (the player's element, filling what it is given), `below` (the header and
/// the rest) and the `chat` column if any. `theater` is the page's size in theater mode.
pub fn watch_layout(
  player: AnyElement,
  below: AnyElement,
  chat: Option<AnyElement>,
  theater: Option<Size<Pixels>>,
) -> AnyElement {
  let chat_width = if chat.is_some() {
    px(CHAT_WIDTH)
  } else {
    px(0.)
  };
  if let Some(page) = theater {
    let width = page.width - chat_width;
    let room = (page.height - px(THEATER_BELOW)).max(px(THEATER_MIN));
    let height = (width * (9. / 16.)).min(room);
    return v_flex()
      .id("watch-layout")
      .size_full()
      .overflow_y_scroll()
      .child(
        h_flex()
          .w_full()
          .flex_none()
          .h(height)
          .child(
            div()
              .flex_1()
              .min_w_0()
              .h_full()
              .rounded_tl(theme::PANEL_INNER_RADIUS)
              .bg(gpui::black())
              .child(player),
          )
          .children(chat.map(|chat| div().w(chat_width).flex_none().h_full().p_2().child(chat))),
      )
      .child(div().w_full().p_6().child(below))
      .into_any_element();
  }
  h_flex()
    .size_full()
    .items_start()
    .child(
      v_flex()
        .id("watch-layout")
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .p_6()
        .gap_5()
        .child(
          // 16:9 through padding-top, as GPUI has no aspect-ratio style.
          div()
            .relative()
            .w_full()
            .flex_none()
            .pt(relative(9. / 16.))
            .rounded(theme::RADIUS_LG)
            .overflow_hidden()
            .child(div().absolute().inset_0().child(player)),
        )
        .child(below),
    )
    .children(chat.map(|chat| {
      div()
        .w(chat_width)
        .flex_none()
        .h_full()
        .py_6()
        .pr_6()
        .child(chat)
    }))
    .into_any_element()
}
