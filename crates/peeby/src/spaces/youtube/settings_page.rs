//! YouTube's part of the settings page: the account (cookie import), the start page, YouTube's
//! content language and region (not the interface's), the default quality, and where the
//! notifications show.

use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, Window};
use gpui_component::v_flex;

use crate::settings::{LANGUAGES, QUALITIES, Settings, StartPage};
use crate::shell::delivery_row;
use crate::spaces::youtube::YouTubeSpace;
use crate::spaces::youtube::account::AccountView;
use ui::form::{OnClick, choices, row, section};
use ui::i18n::t;

/// The part: `account` then the start page, the content language and the quality.
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
      // New clients in the new language; the start page is reloaded.
      nav.update(cx, |app, cx| app.reload_session(cx)).ok();
    });
    (label.to_owned(), selected, on_click)
  });
  let start_pages = [
    (t("Home", "Accueil"), StartPage::Home),
    (t("Subscriptions", "Abonnements"), StartPage::Subscriptions),
  ]
  .into_iter()
  .map(|(label, page)| {
    let on_click: OnClick = Box::new(move |_, cx| Settings::update(cx, |s| s.start_page = page));
    (label.to_owned(), settings.start_page == page, on_click)
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
      t("Content and playback", "Contenu et lecture"),
      vec![
        row(
          t("Start page", "Page d'accueil"),
          t(
            "The page shown when Peeby starts.",
            "Page affichée au lancement de Peeby.",
          ),
          choices("start-page", start_pages),
        )
        .into_any_element(),
        row(
          t("Content language", "Langue du contenu"),
          t(
            "Language of YouTube's titles, dates and captions, and region of its \
             recommendations. The interface's language is set under General.",
            "Langue des titres, des dates et des sous-titres de YouTube, et région de ses \
             recommandations. La langue de l'interface se règle dans Général.",
          ),
          choices("language", languages),
        )
        .into_any_element(),
        row(
          t("Default quality", "Qualité par défaut"),
          t(
            "Highest quality when a video starts; the player's menu changes it at any time. \
             Above 1080p, the video is VP9 and needs more bandwidth.",
            "Qualité maximale au lancement d'une vidéo ; le menu du lecteur permet d'en changer à \
             tout moment. Au-delà de 1080p, la vidéo est en VP9 et demande plus de débit.",
          ),
          choices("quality", qualities),
        )
        .into_any_element(),
      ],
    ))
    .child(section(
      t("Notifications", "Notifications"),
      vec![
        delivery_row(
          "youtube-notify-video",
          t("New video", "Nouvelle vidéo"),
          t(
            "What YouTube sends for the channels whose bell is on (All or Personalized),              Shorts included. Signed in; checked every three minutes.",
            "Ce qu'envoie YouTube pour les chaînes dont la cloche est activée (Toutes ou              Personnalisées), Shorts compris. Connecté ; relevé toutes les trois minutes.",
          ),
          |n| &mut n.youtube_video,
          cx,
        ),
        delivery_row(
          "youtube-notify-live",
          t("Live started", "Live commencé"),
          t(
            "A channel whose bell is on goes live, as YouTube announces it.",
            "Une chaîne dont la cloche est activée passe en direct, comme YouTube l'annonce.",
          ),
          |n| &mut n.youtube_live,
          cx,
        ),
      ],
    ))
    .into_any_element()
}
