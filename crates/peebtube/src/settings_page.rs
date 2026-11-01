//! Settings page: YouTube's language and region, default quality. Changes apply at once and
//! are saved to `settings.toml`.

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, FontWeight, SharedString, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Sizable, h_flex, v_flex};

use crate::app::AppView;
use crate::settings::{LANGUAGES, QUALITIES, Settings};
use crate::theme;
use crate::ui::page_title;

pub struct SettingsView {
  nav: WeakEntity<AppView>,
}

impl SettingsView {
  pub fn new(nav: WeakEntity<AppView>) -> Self {
    Self { nav }
  }
}

/// What a choice does when clicked.
type OnClick = Box<dyn Fn(&mut Window, &mut App)>;

/// A group of choices; the selected one is red.
fn choices(
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

/// A settings row: title and description on the left, the control on the right.
fn row(title: &str, description: &str, control: impl IntoElement) -> impl IntoElement {
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
fn section(title: &str, rows: Vec<AnyElement>) -> impl IntoElement {
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

impl Render for SettingsView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let settings = Settings::get(cx).clone();
    let languages = LANGUAGES.iter().map(|&(language, region, label)| {
      let selected = settings.language == language && settings.region == region;
      let nav = self.nav.clone();
      let on_click: OnClick = Box::new(move |_, cx| {
        Settings::update(cx, |s| {
          s.language = language.to_owned();
          s.region = region.to_owned();
        });
        // New clients in the new language; Home is reloaded.
        nav.update(cx, |app, cx| app.reload_session(cx)).ok();
      });
      (label.to_owned(), selected, on_click)
    });
    let qualities = QUALITIES.iter().map(|&height| {
      let on_click: OnClick =
        Box::new(move |_, cx| Settings::update(cx, |s| s.max_height = height));
      (
        format!("{height}p"),
        settings.max_height == height,
        on_click,
      )
    });
    v_flex()
      .id("settings")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        v_flex()
          .px_8()
          .py_6()
          .gap_8()
          .max_w(px(900.))
          .child(page_title("Réglages"))
          .child(section(
            "YouTube",
            vec![
              row(
                "Langue et région",
                "Langue des titres, des dates et des sous-titres, et région des \
                 recommandations. L'interface de PeebTube reste en français.",
                choices("language", languages),
              )
              .into_any_element(),
            ],
          ))
          .child(section(
            "Lecture",
            vec![
              row(
                "Qualité par défaut",
                "Qualité maximale au lancement d'une vidéo ; le menu du lecteur permet d'en \
                 changer à tout moment. Au-delà de 1080p, la vidéo est en VP9 et demande plus \
                 de débit.",
                choices("quality", qualities),
              )
              .into_any_element(),
            ],
          ))
          .child(section(
            "Stockage",
            vec![
              row(
                "Fichiers",
                "Réglages et cookies sont dans %APPDATA%\\peebtube, les caches (images, \
                 yt-dlp) dans %LOCALAPPDATA%\\peebtube.",
                div(),
              )
              .into_any_element(),
            ],
          )),
      )
  }
}
