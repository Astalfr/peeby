//! Settings page: YouTube's language and region, default quality. Changes apply at once and
//! are saved to `settings.toml`.

use gpui::prelude::*;
use gpui::{App, Context, FontWeight, SharedString, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{h_flex, v_flex};

use crate::app::AppView;
use crate::settings::{LANGUAGES, QUALITIES, Settings};
use crate::theme;

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

/// A group of choices; the selected one is a primary button.
fn choices(
  id: &str,
  options: impl IntoIterator<Item = (String, bool, OnClick)>,
) -> impl IntoElement {
  h_flex()
    .flex_wrap()
    .gap_2()
    .children(
      options
        .into_iter()
        .enumerate()
        .map(|(i, (label, selected, on_click))| {
          Button::new(SharedString::from(format!("{id}-{i}")))
            .label(label)
            .when(selected, |b| b.primary())
            .when(!selected, |b| b.ghost())
            .on_click(move |_, window, cx| on_click(window, cx))
        }),
    )
}

fn section(title: &str, description: &str, content: impl IntoElement) -> impl IntoElement {
  v_flex()
    .gap_3()
    .child(
      v_flex()
        .gap_1()
        .child(
          div()
            .font_weight(FontWeight::SEMIBOLD)
            .child(title.to_owned()),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme::muted())
            .child(description.to_owned()),
        ),
    )
    .child(content)
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
      .overflow_y_scroll()
      .child(
        v_flex()
          .p_6()
          .gap_8()
          .max_w(px(820.))
          .child(
            div()
              .text_2xl()
              .font_weight(FontWeight::SEMIBOLD)
              .child("Réglages"),
          )
          .child(section(
            "Langue et région de YouTube",
            "Langue des titres, des dates et des sous-titres, et région des recommandations. \
             L'interface de PeebTube reste en français.",
            choices("language", languages),
          ))
          .child(section(
            "Qualité par défaut",
            "Qualité maximale au lancement d'une vidéo ; le menu du lecteur permet d'en changer \
             à tout moment. Au-delà de 1080p, la vidéo est en VP9 et demande plus de débit.",
            choices("quality", qualities),
          ))
          .child(section(
            "Fichiers",
            "Réglages et cookies sont dans %APPDATA%\\peebtube, les caches (images, yt-dlp) dans \
             %LOCALAPPDATA%\\peebtube.",
            div(),
          )),
      )
  }
}
