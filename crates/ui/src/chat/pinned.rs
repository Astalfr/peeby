//! What a channel pins over its chat, as Twitch does with its predictions and polls: a stack of
//! bars at the top of the message list, drawn over the messages (the list keeps its layout and
//! its following). A bar shows its kind, its title and its state in a line; a click unfolds its
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
    .px_3()
    .py_2()
    .gap_2()
    .items_center()
    .cursor_pointer()
    .hover(|s| s.bg(theme::surface_hover()))
    .on_click(move |_, window, cx| on_toggle(&id, window, cx))
    .child(
      gpui_component::Icon::new(item.icon)
        .size(px(16.))
        .text_color(item.accent),
    )
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
