//! The watch page's layout with a chat column, shared by the spaces that play lives:
//! - normally, the player (16:9) and what goes under it scroll on the left, and the chat fills the
//!   page's height on the right, in a card;
//! - in theater mode (as Twitch's), the player fills the whole page but the chat's column (mpv
//!   letterboxes the video), the chat beside it on the page's height, flush against it, and what
//!   goes under the player is hidden.
//!
//! Without a chat, the player and what goes under it take the whole width.

use gpui::prelude::*;
use gpui::{AnyElement, div, px, relative};
use gpui_component::{h_flex, v_flex};

use crate::theme;

/// Width of the chat column.
pub const CHAT_WIDTH: f32 = 340.;

/// The page: `player` (the player's element, filling what it is given), `below` (the header and
/// the rest) and the `chat` column if any.
pub fn watch_layout(
  player: AnyElement,
  below: AnyElement,
  chat: Option<AnyElement>,
  theater: bool,
) -> AnyElement {
  let chat_width = px(CHAT_WIDTH);
  if theater {
    return h_flex()
      .size_full()
      .child(
        div()
          .flex_1()
          .min_w_0()
          .h_full()
          .rounded_tl(theme::PANEL_INNER_RADIUS)
          .bg(gpui::black())
          .child(player),
      )
      .children(chat.map(|chat| {
        div()
          .w(chat_width)
          .flex_none()
          .h_full()
          .bg(theme::surface())
          .child(chat)
      }))
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
        .child(
          div()
            .size_full()
            .rounded(theme::RADIUS_LG)
            .bg(theme::surface())
            .border_1()
            .border_color(theme::border())
            .overflow_hidden()
            .child(chat),
        )
    }))
    .into_any_element()
}
