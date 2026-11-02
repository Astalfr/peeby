//! Building blocks of settings pages: titled sections of rounded rows, each with a title and
//! description on the left and its control on the right (choice chips, a switch).

use gpui::prelude::*;
use gpui::{AnyElement, App, FontWeight, SharedString, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::switch::Switch;
use gpui_component::{Sizable, h_flex, v_flex};

use crate::theme;

/// What a choice does when clicked.
pub type OnClick = Box<dyn Fn(&mut Window, &mut App)>;

/// A group of choices; the selected one takes the accent.
pub fn choices(
  id: &str,
  options: impl IntoIterator<Item = (String, bool, OnClick)>,
) -> impl IntoElement {
  h_flex()
    .flex_wrap()
    .justify_end()
    .gap_1()
    .children(
      options
        .into_iter()
        .enumerate()
        .map(|(i, (label, selected, on_click))| {
          Button::new(SharedString::from(format!("{id}-{i}")))
            .small()
            .label(label)
            .when(selected, |b| b.primary())
            .when(!selected, |b| b.ghost())
            .on_click(move |_, window, cx| on_click(window, cx))
        }),
    )
}

/// Choice chips for `options` (value, label), the one equal to `selected` in the accent; a click
/// calls `on_select` with its value.
pub fn chips<T: Clone + PartialEq + 'static>(
  id: &str,
  options: Vec<(T, String)>,
  selected: &T,
  on_select: impl Fn(T, &mut App) + Clone + 'static,
) -> impl IntoElement {
  let options: Vec<(String, bool, OnClick)> = options
    .into_iter()
    .map(|(value, label)| {
      let active = &value == selected;
      let on_select = on_select.clone();
      let on_click: OnClick = Box::new(move |_, cx| on_select(value.clone(), cx));
      (label, active, on_click)
    })
    .collect();
  choices(id, options)
}

/// An on/off switch.
pub fn toggle(id: &str, checked: bool, on_change: impl Fn(bool, &mut App) + 'static) -> Switch {
  Switch::new(SharedString::from(id.to_owned()))
    .checked(checked)
    .on_click(move |checked, _, cx| on_change(*checked, cx))
}

/// `12 MB`, `1.5 GB` (`12 Mo`, `1.5 Go` in French).
pub fn format_size(bytes: u64) -> String {
  let mb = bytes as f64 / (1024. * 1024.);
  if mb >= 1024. {
    let gb = mb / 1024.;
    crate::tf!("{gb:.1} GB", "{gb:.1} Go")
  } else {
    crate::tf!("{mb:.0} MB", "{mb:.0} Mo")
  }
}

/// A settings row: title and description on the left, the control on the right.
pub fn row(title: &str, description: &str, control: impl IntoElement) -> impl IntoElement {
  h_flex()
    .px_5()
    .py_4()
    .gap_6()
    .items_center()
    .rounded(theme::RADIUS)
    .bg(theme::surface())
    .child(
      v_flex()
        .flex_1()
        .min_w_0()
        .gap_0p5()
        .child(
          div()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .child(title.to_owned()),
        )
        .child(
          div()
            .text_xs()
            .text_color(theme::muted())
            .child(description.to_owned()),
        ),
    )
    .child(div().flex_none().max_w(px(420.)).child(control))
}

/// A titled group of rows.
pub fn section(title: &str, rows: Vec<AnyElement>) -> impl IntoElement {
  v_flex()
    .gap_1()
    .child(
      div()
        .pb_2()
        .text_base()
        .font_weight(FontWeight::SEMIBOLD)
        .child(title.to_owned()),
    )
    .children(rows)
}
