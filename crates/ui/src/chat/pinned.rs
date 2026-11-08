//! What a channel pins over its chat, as Twitch does with its predictions and polls: a stack of
//! bars at the top of the message list, drawn over the messages (the list keeps its layout and
//! its following), a line along each one's bottom edge for the time it has left. A bar shows its kind, its title and its state in a line; a click unfolds its
//! card under it and folds the others, so that two items never cover the chat together. The
//! space keeps which one is open ([`Unfolded`]) and builds the open one's body; new items come
//! folded.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, ElementId, FontWeight, Hsla, SharedString, Window, div, px, relative};
use gpui_component::{IconName, h_flex, v_flex};

use crate::assets::Icon;
use crate::theme;

/// Height of the stack at most, as a share of the message list's: the newest lines stay in
/// sight.
const MAX_SHARE: f32 = 0.7;
/// Side of an item's lead image (an avatar), logical pixels.
pub const LEAD: f32 = 24.;
/// Height of the time-left line along an item's bottom edge, logical pixels.
const TIME_LINE: f32 = 3.;
/// Opacity of a bar's progress fill over the surface.
const FILL_OPACITY: f32 = 0.22;

/// One pinned item.
pub struct Pinned {
  /// Its id, for [`Unfolded`].
  pub id: SharedString,
  /// Its icon, before the bar's text.
  pub icon: Icon,
  /// Its color (the icon, the open card's edge).
  pub accent: Hsla,
  /// What it is ("Prediction").
  pub kind: SharedString,
  /// Its question, in a line folded and whole unfolded.
  pub title: SharedString,
  /// Its state in a few words, folded or not ("1:24", "Your bet: 500 on “Yes”").
  pub status: SharedString,
  /// The card, for the open item only (`None` folded).
  pub body: Option<AnyElement>,
  /// The share of its time left (1 at its start, 0 at its end), drawn as a line along its bottom
  /// edge; `None` once it no longer counts down (closed, ended).
  pub time_left: Option<f32>,
  /// Drawn in the icon's place, 24 px (a channel's avatar); the icon when `None`.
  pub lead: Option<AnyElement>,
  /// A progress (0 to 1) filling the bar from the left in a tint of the accent, as Twitch's
  /// Hype Train banner does; `None` for a plain bar.
  pub fill: Option<f32>,
}

/// Which item is unfolded, by id: held by the space across its reloads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unfolded(Option<SharedString>);

impl Unfolded {
  /// Whether `id` is the unfolded one.
  pub fn is(&self, id: &str) -> bool {
    self.0.as_deref() == Some(id)
  }

  /// A click on `id`'s bar: unfolds it (folding the other), or folds it again.
  pub fn toggle(&mut self, id: &str) {
    self.0 = if self.is(id) {
      None
    } else {
      Some(id.to_owned().into())
    };
  }

  /// Folds an item that is gone (ended, removed): the next one with its id comes folded.
  pub fn keep_only<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
    if let Some(open) = &self.0
      && !ids.into_iter().any(|id| id == open.as_ref())
    {
      self.0 = None;
    }
  }
}

/// What a click on a bar does, with the item's id.
pub type OnToggle = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// The stack of `items` (nothing when empty), drawn by [`crate::chat::ChatPanel`] over its
/// messages.
pub fn pinned_stack(items: Vec<Pinned>, on_toggle: OnToggle) -> AnyElement {
  if items.is_empty() {
    return div().absolute().into_any_element();
  }
  v_flex()
    .absolute()
    .top_0()
    .left_0()
    .right_0()
    .max_h(relative(MAX_SHARE))
    .p_2()
    .gap_1()
    .overflow_hidden()
    .children(
      items
        .into_iter()
        .map(|item| item_element(item, on_toggle.clone())),
    )
    .into_any_element()
}

fn item_element(item: Pinned, on_toggle: OnToggle) -> AnyElement {
  let open = item.body.is_some();
  let id = item.id.clone();
  let bar = h_flex()
    .id(ElementId::Name(format!("pinned-bar-{}", item.id).into()))
    .flex_none()
    .relative()
    .px_3()
    .py_2()
    .gap_2()
    .items_center()
    .cursor_pointer()
    .hover(|s| s.bg(theme::surface_hover()))
    .on_click(move |_, window, cx| on_toggle(&id, window, cx))
    // Under the bar's content: drawn first.
    .children(item.fill.map(|share| {
      div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .w(relative(share.clamp(0., 1.)))
        .bg(item.accent.opacity(FILL_OPACITY))
    }))
    .child(match item.lead {
      Some(lead) => div()
        .size(px(LEAD))
        .flex_none()
        .child(lead)
        .into_any_element(),
      None => gpui_component::Icon::new(item.icon)
        .size(px(16.))
        .text_color(item.accent)
        .into_any_element(),
    })
    .child(
      v_flex()
        .flex_1()
        .min_w_0()
        .child(
          div()
            .text_xs()
            .text_color(theme::muted())
            .truncate()
            .child(format!("{} · {}", item.kind, item.status)),
        )
        // Folded, the title in a line; open, the card shows it whole.
        .when(!open, |el| {
          el.child(
            div()
              .text_sm()
              .font_weight(FontWeight::SEMIBOLD)
              .truncate()
              .child(item.title.clone()),
          )
        }),
    )
    .child(
      gpui_component::Icon::new(if open {
        IconName::ChevronUp
      } else {
        IconName::ChevronDown
      })
      .size(px(16.))
      .text_color(theme::muted()),
    );
  v_flex()
    .when(open, |el| el.flex_shrink(1.).min_h_0())
    .when(!open, |el| el.flex_none())
    .relative()
    .rounded(theme::RADIUS)
    .bg(theme::surface())
    .border_1()
    .border_color(if open { item.accent } else { theme::border() })
    .shadow_md()
    .overflow_hidden()
    // Over the messages: nothing under it gets the pointer (an emote's preview, the wheel).
    .occlude()
    .child(bar)
    .children(item.body.map(|body| {
      v_flex()
        .id(ElementId::Name(format!("pinned-body-{}", item.id).into()))
        .min_h_0()
        .flex_shrink(1.)
        .overflow_y_scroll()
        .px_3()
        .pb_3()
        .gap_2()
        .child(
          div()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .child(item.title),
        )
        .child(body)
    }))
    // Room for the time line, under the card too: drawn over its bottom edge.
    .when(item.time_left.is_some(), |el| el.pb(px(TIME_LINE)))
    .children(item.time_left.map(|left| time_line(left, item.accent)))
    .into_any_element()
}

/// The time-left line: the bottom border of a layer over the whole item, rounded as its inside
/// so that the line follows the frame's corners (GPUI clips to rectangles, and a line 3 px high
/// cannot round itself to an 8 px corner). The share left is that layer cut on the right, which a
/// rectangle does: the track under it, then the accent over the share left.
fn time_line(left: f32, accent: Hsla) -> AnyElement {
  // Inside the item's 1 px border.
  let inner = theme::RADIUS - px(1.);
  let edge = |color: Hsla| {
    div()
      .absolute()
      .top_0()
      .bottom_0()
      .left_0()
      .rounded(inner)
      .border_b(px(TIME_LINE))
      .border_color(color)
  };
  let left = left.clamp(0., 1.);
  div()
    .absolute()
    .inset_0()
    .child(edge(theme::surface_hover()).right_0())
    .when(left > 0., |el| {
      el.child(
        div()
          .absolute()
          .top_0()
          .bottom_0()
          .left_0()
          .w(relative(left))
          .overflow_hidden()
          // As wide as the item, so that its corners are the item's.
          .child(edge(accent).w(relative(1. / left))),
      )
    })
    .into_any_element()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn one_item_unfolds_at_a_time_and_gone_ones_fold() {
    let mut unfolded = Unfolded::default();
    assert!(!unfolded.is("prediction"));
    unfolded.toggle("prediction");
    assert!(unfolded.is("prediction"));
    // Opening the poll folds the prediction.
    unfolded.toggle("poll");
    assert!(unfolded.is("poll") && !unfolded.is("prediction"));
    unfolded.keep_only(["poll", "prediction"]);
    assert!(unfolded.is("poll"));
    unfolded.keep_only(["prediction"]);
    assert_eq!(unfolded, Unfolded::default());
    unfolded.toggle("prediction");
    unfolded.toggle("prediction");
    assert!(!unfolded.is("prediction"));
  }
}
