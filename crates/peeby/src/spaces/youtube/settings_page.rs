//! YouTube's part of the settings page: the account (cookie import), YouTube's language and
//! region, and the default quality.

use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, Window};
use gpui_component::v_flex;

use crate::settings::{LANGUAGES, QUALITIES, Settings};
use crate::spaces::youtube::YouTubeSpace;
use crate::spaces::youtube::account::AccountView;
use ui::form::{OnClick, choices, row, section};

/// The part: `account` then the language and the quality.
pub fn part(
  account: Entity<AccountView>,
  _window: &mut Window,
  cx: &mut gpui::Context<YouTubeSpace>,
) -> AnyElement {
  let settings = Settings::get(cx).clone();
  let nav = cx.weak_entity();
  let languages = LANGUAGES.iter().map(|&(language, region, label)| {
    let selected = settings.language == language && settings.region == region;
    let nav = nav.clone();
    let on_click: OnClick = Box::new(move |_, cx: &mut App| {
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
    let on_click: OnClick = Box::new(move |_, cx| Settings::update(cx, |s| s.max_height = height));
    (
      format!("{height}p"),
      settings.max_height == height,
      on_click,
    )
  });
  v_flex()
    .gap_8()
    .child(account)
    .child(section(
      "Contenu et lecture",
      vec![
        row(
          "Langue et région",
          "Langue des titres, des dates et des sous-titres, et région des recommandations. \
           L'interface de Peeby reste en français.",
          choices("language", languages),
        )
        .into_any_element(),
        row(
          "Qualité par défaut",
          "Qualité maximale au lancement d'une vidéo ; le menu du lecteur permet d'en changer à \
           tout moment. Au-delà de 1080p, la vidéo est en VP9 et demande plus de débit.",
          choices("quality", qualities),
        )
        .into_any_element(),
      ],
    ))
    .into_any_element()
}
