//! Settings page: YouTube's language and region, default quality. Changes apply at once and
//! are saved to `settings.toml`.

use gpui::prelude::*;
use gpui::{Context, WeakEntity, Window, div, px};
use gpui_component::scroll::ScrollableElement;
use gpui_component::v_flex;

use crate::app::AppView;
use crate::settings::{LANGUAGES, QUALITIES, Settings};
use ui::form::{OnClick, choices, row, section};
use ui::widgets::page_title;

pub struct SettingsView {
  nav: WeakEntity<AppView>,
}

impl SettingsView {
  pub fn new(nav: WeakEntity<AppView>) -> Self {
    Self { nav }
  }
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
                 recommandations. L'interface de Peeby reste en français.",
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
                "Réglages et cookies sont dans %APPDATA%\\peeby, les caches (images, \
                 yt-dlp) dans %LOCALAPPDATA%\\peeby.",
                div(),
              )
              .into_any_element(),
            ],
          )),
      )
  }
}
