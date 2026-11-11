//! YouTube's part of the settings page: the account (cookie import), the start page, YouTube's
//! content language and region (not the interface's), the original titles, the default quality,
//! SponsorBlock (ADR 0034), and where the notifications show.

use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, Window};
use gpui_component::v_flex;

use crate::account::AccountView;
use crate::{YouTubeSpace, inbox};
use innertube::NotificationKind;
use innertube::SponsorCategory;
use peeby_core::settings::{LANGUAGES, QUALITIES, SegmentAction, Settings, StartPage};
use peeby_core::shell::delivery_row;
use ui::form::{OnClick, chips, choices, row, section, toggle};
use ui::i18n::t;
use ui::widgets::Flash;

/// The part: `account` (under `flash`) then the start page, the content language and the
/// quality.
pub fn part(
  account: Entity<AccountView>,
  flash: Flash,
  _window: &mut Window,
  cx: &mut gpui::Context<YouTubeSpace>,
) -> AnyElement {
  let settings = Settings::get(cx).clone();
  let nav = cx.weak_entity();
  let reload = nav.clone();
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
    .child(flash.wrap("youtube-account-flash", account, cx))
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
          t("Original titles", "Titres originaux"),
          t(
            "Titles in the video's own language, as its channel wrote them, instead of the ones YouTube translates into the content language. Each video takes one more request.",
            "Titres dans la langue de la vidéo, tels que sa chaîne les a écrits, plutôt que ceux que YouTube traduit dans la langue du contenu. Chaque vidéo demande une requête de plus.",
          ),
          toggle("original-titles", settings.original_titles, move |on, cx| {
            Settings::update(cx, |s| s.original_titles = on);
            // New clients; the start page is reloaded with the titles.
            reload.update(cx, |app, cx| app.reload_session(cx)).ok();
          }),
        )
        .into_any_element(),
        row(
          t("Shorts in the feeds", "Shorts dans les flux"),
          t(
            "The rows of Shorts YouTube puts between the videos of Home and the subscriptions.",
            "Les rangées de Shorts que YouTube place entre les vidéos de l'accueil et des abonnements.",
          ),
          toggle("feed-shorts", settings.feed_shorts, |on, cx| {
            Settings::update(cx, |s| s.feed_shorts = on)
          }),
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
    .child(sponsorblock_section(&settings))
    .child(section(
      t("Notifications", "Notifications"),
      vec![
        delivery_row(
          "youtube-notify-video",
          t("New video", "Nouvelle vidéo"),
          t(
            "What YouTube sends for the channels whose bell is on (All or Personalized), Shorts included. Signed in; checked every three minutes.",
            "Ce qu'envoie YouTube pour les chaînes dont la cloche est activée (Toutes ou Personnalisées), Shorts compris. Connecté ; relevé toutes les trois minutes.",
          ),
          |n| &mut n.youtube_video,
          |cx| inbox::test_toast(NotificationKind::Upload, cx),
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
          |cx| inbox::test_toast(NotificationKind::Live, cx),
          cx,
        ),
      ],
    ))
    .into_any_element()
}

/// SponsorBlock: on or off, then what each category does (skipped, a button, ignored), and the
/// attribution SponsorBlock asks for.
fn sponsorblock_section(settings: &Settings) -> impl IntoElement + use<> {
  let sponsorblock = &settings.sponsorblock;
  let mut rows = vec![
    row(
      t("Skip sponsored parts", "Passer les parties sponsorisées"),
      t(
        "Asks SponsorBlock, a database run by its users, which parts of a video are sponsors, reminders or intros (by a prefix of the video's hash: SponsorBlock does not learn which video plays). They show on the seek bar in their color.",
        "Demande à SponsorBlock, une base tenue par ses utilisateurs, quelles parties d'une vidéo sont des sponsors, des rappels ou des intros (par un préfixe du hachage de la vidéo : SponsorBlock ne sait pas laquelle est regardée). Elles s'affichent sur la barre de lecture dans leur couleur.",
      ),
      toggle("sponsorblock", sponsorblock.enabled, |on, cx| {
        Settings::update(cx, |s| s.sponsorblock.enabled = on)
      }),
    )
    .into_any_element(),
  ];
  if sponsorblock.enabled {
    rows.extend(SponsorCategory::ALL.into_iter().map(|category| {
      let key = category.key();
      let (title, description) = category_text(category);
      row(
        title,
        description,
        chips(
          &format!("sponsorblock-{key}"),
          vec![
            (SegmentAction::Skip, t("Skip", "Passer").to_owned()),
            (SegmentAction::Button, t("Button", "Bouton").to_owned()),
            (SegmentAction::Off, t("Off", "Ignorer").to_owned()),
          ],
          &sponsorblock.action(key),
          move |action, cx| {
            Settings::update(cx, |s| {
              if let Some(setting) = s.sponsorblock.action_mut(key) {
                *setting = action;
              }
            })
          },
        ),
      )
      .into_any_element()
    }));
    rows.push(
      row(
        t("Source", "Source"),
        t(
          "Segments from SponsorBlock (sponsor.ajay.app), under CC BY-NC-SA 4.0. A change applies to the next video.",
          "Segments fournis par SponsorBlock (sponsor.ajay.app), sous licence CC BY-NC-SA 4.0. Un changement s'applique à la vidéo suivante.",
        ),
        gpui_component::button::Button::new("sponsorblock-site")
          .label("sponsor.ajay.app")
          .on_click(|_, _, cx| cx.open_url("https://sponsor.ajay.app")),
      )
      .into_any_element(),
    );
  }
  section("SponsorBlock", rows)
}

/// A SponsorBlock category's title and description in the settings, SponsorBlock's own words.
fn category_text(category: SponsorCategory) -> (&'static str, &'static str) {
  match category {
    SponsorCategory::Sponsor => (
      t("Sponsor", "Sponsor"),
      t("A paid promotion.", "Une promotion payée."),
    ),
    SponsorCategory::SelfPromo => (
      t("Self-promotion", "Autopromotion"),
      t(
        "Unpaid or self promotion: merchandise, donations, the channel's other projects.",
        "Promotion non payée ou de la chaîne : produits dérivés, dons, ses autres projets.",
      ),
    ),
    SponsorCategory::Interaction => (
      t("Reminder to subscribe", "Rappel d'abonnement"),
      t(
        "A short reminder to like, subscribe or follow.",
        "Un court rappel pour aimer, s'abonner ou suivre.",
      ),
    ),
    SponsorCategory::Intro => (
      t("Intro", "Intro"),
      t(
        "An intro animation or a pause with no content.",
        "Une animation d'intro ou une pause sans contenu.",
      ),
    ),
    SponsorCategory::Outro => (
      t("End cards", "Écran de fin"),
      t("End cards and credits.", "L'écran de fin et le générique."),
    ),
    SponsorCategory::Preview => (
      t("Preview and recap", "Aperçu et récap"),
      t(
        "A preview of the video or a recap of earlier ones.",
        "Un aperçu de la vidéo ou un récapitulatif des précédentes.",
      ),
    ),
    SponsorCategory::Hook => (
      t("Hook", "Accroche"),
      t(
        "A teaser or greeting before the video starts.",
        "Une accroche ou des salutations avant le début.",
      ),
    ),
    SponsorCategory::Filler => (
      t("Tangent", "Digression"),
      t(
        "A tangent or a joke beside the subject.",
        "Une digression ou une blague hors sujet.",
      ),
    ),
    SponsorCategory::MusicOffTopic => (
      t("Non-music", "Sans musique"),
      t(
        "In a music video, a part without music.",
        "Dans un clip, une partie sans musique.",
      ),
    ),
  }
}
