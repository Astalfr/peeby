//! Jellyfin's part of the settings page (behind the gear): its own playback options, the
//! languages, the home page, and the server, user and sign-out. Each control edits
//! `Settings::jellyfin`, saved at once; most apply to the next video or the next visit of Home.
//! The player's shared options are the shell's ("Lecteur").

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, WeakEntity, Window, div, px};
use gpui_component::button::Button;
use gpui_component::{Sizable, h_flex, v_flex};
use jellyfin_api::Client;
use jellyfin_api::models::PublicSystemInfo;
use ui::form::{chips, row, section, toggle};
use ui::{runtime, theme};

use super::JellyfinSpace;
use super::session::device_name;
use crate::settings::{JellyfinHome, JellyfinPlayback, Settings};

/// Audio languages offered (ISO 639-2), after "the server's".
const AUDIO_LANGUAGES: &[(&str, &str)] = &[
  ("jpn", "Japonais"),
  ("eng", "Anglais"),
  ("fra", "Français"),
  ("spa", "Espagnol"),
  ("deu", "Allemand"),
  ("ita", "Italien"),
  ("kor", "Coréen"),
  ("zho", "Chinois"),
];

/// Subtitle languages offered, after "the server's" and "off".
const SUBTITLE_LANGUAGES: &[(&str, &str)] = &[
  ("fra", "Français"),
  ("eng", "Anglais"),
  ("spa", "Espagnol"),
  ("deu", "Allemand"),
  ("ita", "Italien"),
  ("por", "Portugais"),
];

pub struct SettingsView {
  client: Client,
  nav: WeakEntity<JellyfinSpace>,
  server: Option<PublicSystemInfo>,
}

impl SettingsView {
  pub fn new(client: Client, nav: WeakEntity<JellyfinSpace>, cx: &mut Context<Self>) -> Self {
    cx.observe_global::<Settings>(|_, cx| cx.notify()).detach();
    let info = runtime::spawn(cx, {
      let client = client.clone();
      async move { client.public_system_info().await }
    });
    cx.spawn(async move |this, cx| {
      if let Some(Ok(info)) = info.await {
        this
          .update(cx, |this, cx| {
            this.server = Some(info);
            cx.notify();
          })
          .ok();
      }
    })
    .detach();
    Self {
      client,
      nav,
      server: None,
    }
  }
}

fn set_playback(cx: &mut App, edit: impl FnOnce(&mut JellyfinPlayback)) {
  Settings::update(cx, |s| edit(&mut s.jellyfin.playback));
}

fn set_home(cx: &mut App, edit: impl FnOnce(&mut JellyfinHome)) {
  Settings::update(cx, |s| edit(&mut s.jellyfin.home));
}

impl SettingsView {
  fn playback_section(&self, p: &JellyfinPlayback) -> impl IntoElement {
    let buffers = [150, 300, 600, 1000]
      .map(|mb| (mb, format!("{mb} Mo")))
      .to_vec();
    section(
      "Lecture",
      vec![
        row(
          "Démarrer en plein écran",
          "La lecture s'ouvre en plein écran, et en sort au retour.",
          toggle("fullscreen", p.start_fullscreen, |v, cx| {
            set_playback(cx, |p| p.start_fullscreen = v)
          }),
        )
        .into_any_element(),
        row(
          "Mémoire tampon réseau",
          "Jusqu'où le lecteur lit en avance. Plus aide les connexions lentes ou instables.",
          chips("buffer", buffers, &p.network_buffer_mb, |v, cx| {
            set_playback(cx, |p| p.network_buffer_mb = v)
          }),
        )
        .into_any_element(),
      ],
    )
  }

  fn languages_section(&self, p: &JellyfinPlayback) -> impl IntoElement {
    let mut audio: Vec<(Option<String>, String)> = vec![(None, "Celle du serveur".into())];
    audio.extend(
      AUDIO_LANGUAGES
        .iter()
        .map(|(code, name)| (Some((*code).to_owned()), (*name).to_owned())),
    );
    let mut subtitles: Vec<(Option<String>, String)> = vec![
      (None, "Ceux du serveur".into()),
      (Some("off".into()), "Désactivés".into()),
    ];
    subtitles.extend(
      SUBTITLE_LANGUAGES
        .iter()
        .map(|(code, name)| (Some((*code).to_owned()), (*name).to_owned())),
    );
    let scales = [0.75, 1.0, 1.25, 1.5, 2.0]
      .map(|s| (s, format!("{:.0} %", s * 100.)))
      .to_vec();
    section(
      "Langues et sous-titres",
      vec![
        row(
          "Langue audio",
          "Tant que vous n'avez pas choisi de piste dans la série.",
          chips("audio-lang", audio, &p.audio_language, |v, cx| {
            set_playback(cx, |p| p.audio_language = v)
          }),
        )
        .into_any_element(),
        row(
          "Langue des sous-titres",
          "Les sous-titres complets passent avant ceux des seuls panneaux.",
          chips("sub-lang", subtitles, &p.subtitle_language, |v, cx| {
            set_playback(cx, |p| p.subtitle_language = v)
          }),
        )
        .into_any_element(),
        row(
          "Taille des sous-titres texte",
          "SRT et autres sous-titres texte. Les sous-titres ASS gardent leurs styles.",
          chips("sub-scale", scales, &p.subtitle_scale, |v, cx| {
            set_playback(cx, |p| p.subtitle_scale = v)
          }),
        )
        .into_any_element(),
      ],
    )
  }

  fn home_section(&self, h: &JellyfinHome, cx: &App) -> impl IntoElement {
    let ages = [
      (Some(7), "7 jours"),
      (Some(30), "30 jours"),
      (Some(90), "90 jours"),
      (Some(365), "1 an"),
      (None, "Sans limite"),
    ]
    .map(|(d, l)| (d, l.to_owned()))
    .to_vec();
    let mut rows = vec![
      row(
        "À suivre : séries regardées depuis",
        "Les séries délaissées depuis plus longtemps sortent de « À suivre ».",
        chips("next-up-days", ages, &h.next_up_days, |v, cx| {
          set_home(cx, |h| h.next_up_days = v)
        }),
      )
      .into_any_element(),
      row(
        "À suivre : séries terminées",
        "Proposer à nouveau les séries regardées jusqu'au bout.",
        toggle("rewatching", h.next_up_rewatching, |v, cx| {
          set_home(cx, |h| h.next_up_rewatching = v)
        }),
      )
      .into_any_element(),
      row(
        "Masquer les spoilers",
        "Flouter les images et cacher les résumés des épisodes pas encore vus.",
        toggle("spoilers", h.hide_spoilers, |v, cx| {
          set_home(cx, |h| h.hide_spoilers = v)
        }),
      )
      .into_any_element(),
      row(
        "Bannière",
        "Le titre à reprendre, en haut de l'accueil.",
        toggle("show-hero", h.show_hero, |v, cx| {
          set_home(cx, |h| h.show_hero = v)
        }),
      )
      .into_any_element(),
      row(
        "Reprendre",
        "Les vidéos commencées.",
        toggle("show-continue", h.show_continue_watching, |v, cx| {
          set_home(cx, |h| h.show_continue_watching = v)
        }),
      )
      .into_any_element(),
      row(
        "À suivre",
        "L'épisode suivant de chaque série en cours.",
        toggle("show-next-up", h.show_next_up, |v, cx| {
          set_home(cx, |h| h.show_next_up = v)
        }),
      )
      .into_any_element(),
    ];
    let libraries: Vec<(String, String)> = self
      .nav
      .upgrade()
      .map(|nav| {
        nav
          .read(cx)
          .video_libraries()
          .map(|l| (l.id.to_string(), l.name.clone().unwrap_or_default()))
          .collect()
      })
      .unwrap_or_default();
    for (id, name) in libraries {
      let shown = !h.hidden_latest.contains(&id);
      rows.push(
        row(
          &format!("Derniers ajouts dans {name}"),
          "Une rangée de l'accueil.",
          toggle(&format!("show-latest-{id}"), shown, move |v, cx| {
            let id = id.clone();
            set_home(cx, move |h| {
              h.hidden_latest.retain(|x| x != &id);
              if !v {
                h.hidden_latest.push(id);
              }
            })
          }),
        )
        .into_any_element(),
      );
    }
    section("Accueil", rows)
  }

  fn account_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let server = match &self.server {
      Some(info) => format!(
        "{} · Jellyfin {}",
        info.server_name.clone().unwrap_or_default(),
        info.version.clone().unwrap_or_default()
      ),
      None => "…".to_owned(),
    };
    let user = self
      .nav
      .upgrade()
      .and_then(|nav| nav.read(cx).user_name())
      .unwrap_or_default();
    let line = |label: &str, value: String| {
      h_flex()
        .gap_4()
        .child(
          div()
            .w(px(120.))
            .flex_none()
            .text_sm()
            .text_color(theme::muted())
            .child(label.to_owned()),
        )
        .child(div().text_sm().min_w_0().truncate().child(value))
    };
    section(
      "Compte",
      vec![
        v_flex()
          .px_5()
          .py_4()
          .gap_2()
          .rounded(theme::RADIUS)
          .bg(theme::surface())
          .child(line("Serveur", server))
          .child(line("Adresse", self.client.base_url().to_string()))
          .child(line("Utilisateur", user))
          .child(line("Appareil", device_name()))
          .child(
            h_flex().pt_2().child(
              Button::new("jellyfin-sign-out")
                .small()
                .label("Se déconnecter")
                .on_click(cx.listener(|this, _, _, cx| {
                  this.nav.update(cx, |space, cx| space.log_out(cx)).ok();
                })),
            ),
          )
          .into_any_element(),
      ],
    )
  }
}

impl Render for SettingsView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let jellyfin = Settings::get(cx).jellyfin.clone();
    let (p, h) = (jellyfin.playback, jellyfin.home);
    let sections: Vec<AnyElement> = vec![
      self.account_section(cx).into_any_element(),
      self.playback_section(&p).into_any_element(),
      self.languages_section(&p).into_any_element(),
      self.home_section(&h, cx).into_any_element(),
    ];
    v_flex().gap_8().children(sections)
  }
}
