//! The Jellyfin space's settings (its account entry in the sidebar): playback, languages, home,
//! storage, advanced options, and the server, user and sign-out. Each control edits
//! `Settings::jellyfin`, saved at once; most apply to the next video or the next visit of Home.
//! Step 8 of docs/refactor-plan.md moves them behind the gear, with the shared ones.

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, SharedString, WeakEntity, Window, div, px};
use gpui_component::button::Button;
use gpui_component::scroll::ScrollableElement;
use gpui_component::switch::Switch;
use gpui_component::{Disableable, Sizable, h_flex, v_flex};
use jellyfin_api::Client;
use jellyfin_api::models::PublicSystemInfo;
use ui::form::{OnClick, choices, row, section};
use ui::player::format_step;
use ui::widgets::page_title;
use ui::{images, runtime, theme};

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
  cache_size: Option<u64>,
  clearing: bool,
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
    let mut view = Self {
      client,
      nav,
      server: None,
      cache_size: None,
      clearing: false,
    };
    view.measure_cache(cx);
    view
  }

  fn measure_cache(&mut self, cx: &mut Context<Self>) {
    let size = cx.background_spawn(async { images::disk_cache_size() });
    cx.spawn(async move |this, cx| {
      let size = size.await;
      this
        .update(cx, |this, cx| {
          this.cache_size = Some(size);
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn clear_cache(&mut self, cx: &mut Context<Self>) {
    self.clearing = true;
    cx.notify();
    let clear = cx.background_spawn(async { images::clear_disk_cache() });
    cx.spawn(async move |this, cx| {
      clear.await;
      this
        .update(cx, |this, cx| {
          this.clearing = false;
          this.measure_cache(cx);
        })
        .ok();
    })
    .detach();
  }
}

fn set_playback(cx: &mut App, edit: impl FnOnce(&mut JellyfinPlayback)) {
  Settings::update(cx, |s| edit(&mut s.jellyfin.playback));
}

fn set_home(cx: &mut App, edit: impl FnOnce(&mut JellyfinHome)) {
  Settings::update(cx, |s| edit(&mut s.jellyfin.home));
}

/// Choice chips for `options`, the one equal to `selected` in the accent; a click calls
/// `on_select` with its value.
fn chips<T: Clone + PartialEq + 'static>(
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

fn toggle(id: &str, checked: bool, on_change: impl Fn(bool, &mut App) + 'static) -> Switch {
  Switch::new(SharedString::from(id.to_owned()))
    .checked(checked)
    .on_click(move |checked, _, cx| on_change(*checked, cx))
}

fn format_size(bytes: u64) -> String {
  let mb = bytes as f64 / (1024. * 1024.);
  if mb >= 1024. {
    format!("{:.1} Go", mb / 1024.)
  } else {
    format!("{mb:.0} Mo")
  }
}

impl SettingsView {
  fn playback_section(&self, p: &JellyfinPlayback) -> impl IntoElement {
    let countdowns = [5, 10, 15, 20, 30].map(|s| (s, format!("{s} s"))).to_vec();
    let steps = [5, 10, 15, 30].map(|s| (s, format_step(s))).to_vec();
    let large_steps = [30, 60, 120, 300].map(|s| (s, format_step(s))).to_vec();
    section(
      "Lecture",
      vec![
        row(
          "Lire l'épisode suivant automatiquement",
          "Quand le générique commence, après un compte à rebours.",
          toggle("autoplay", p.autoplay_next, |v, cx| {
            set_playback(cx, |p| p.autoplay_next = v)
          }),
        )
        .into_any_element(),
        row(
          "Compte à rebours",
          "Avant de lancer l'épisode suivant.",
          div()
            .when(!p.autoplay_next, |el| el.opacity(0.5))
            .child(chips(
              "countdown",
              countdowns,
              &p.autoplay_countdown,
              |v, cx| set_playback(cx, |p| p.autoplay_countdown = v),
            )),
        )
        .into_any_element(),
        row(
          "Démarrer en plein écran",
          "La lecture s'ouvre en plein écran, et en sort au retour.",
          toggle("fullscreen", p.start_fullscreen, |v, cx| {
            set_playback(cx, |p| p.start_fullscreen = v)
          }),
        )
        .into_any_element(),
        row(
          "Mémoriser le volume",
          "Garder le volume et la coupure du son d'une vidéo à l'autre.",
          toggle("remember-volume", p.remember_volume, |v, cx| {
            set_playback(cx, |p| p.remember_volume = v)
          }),
        )
        .into_any_element(),
        row(
          "Saut",
          "← / → et les boutons autour de lecture/pause.",
          chips("seek", steps, &p.seek_step, |v, cx| {
            set_playback(cx, |p| p.seek_step = v)
          }),
        )
        .into_any_element(),
        row(
          "Grand saut",
          "Maj + ← / →.",
          chips("seek-large", large_steps, &p.seek_step_large, |v, cx| {
            set_playback(cx, |p| p.seek_step_large = v)
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
          "",
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

  fn storage_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let size = match self.cache_size {
      Some(bytes) => format!("{} sur le disque.", format_size(bytes)),
      None => "Calcul…".to_owned(),
    };
    section(
      "Stockage",
      vec![
        row(
          "Cache des images",
          &format!("Affiches, miniatures et fonds gardés sur ce PC. {size}"),
          Button::new("clear-cache")
            .small()
            .label("Vider")
            .loading(self.clearing)
            .disabled(self.clearing)
            .on_click(cx.listener(|this, _, _, cx| this.clear_cache(cx))),
        )
        .into_any_element(),
      ],
    )
  }

  fn advanced_section(&self, p: &JellyfinPlayback) -> impl IntoElement {
    let buffers = [150, 300, 600, 1000]
      .map(|mb| (mb, format!("{mb} Mo")))
      .to_vec();
    section(
      "Avancé",
      vec![
        row(
          "Mémoire tampon réseau",
          "Jusqu'où le lecteur lit en avance. Plus aide les connexions lentes ou instables.",
          chips("buffer", buffers, &p.network_buffer_mb, |v, cx| {
            set_playback(cx, |p| p.network_buffer_mb = v)
          }),
        )
        .into_any_element(),
        row(
          "Accélération matérielle",
          "Décoder et afficher la vidéo avec la carte graphique. À couper seulement pour un \
           dépannage.",
          toggle("hwaccel", p.hardware_acceleration, |v, cx| {
            set_playback(cx, |p| p.hardware_acceleration = v)
          }),
        )
        .into_any_element(),
      ],
    )
  }

  fn about_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
      "À propos",
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
          .child(line("Peeby", env!("CARGO_PKG_VERSION").to_owned()))
          .child(
            h_flex().pt_2().child(
              Button::new("about-sign-out")
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
      self.playback_section(&p).into_any_element(),
      self.languages_section(&p).into_any_element(),
      self.home_section(&h, cx).into_any_element(),
      self.storage_section(cx).into_any_element(),
      self.advanced_section(&p).into_any_element(),
      self.about_section(cx).into_any_element(),
    ];
    v_flex()
      .id("jellyfin-settings")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        v_flex()
          .w_full()
          .max_w(px(920.))
          .p_8()
          .gap_8()
          .child(page_title("Réglages de Jellyfin"))
          .children(sections),
      )
  }
}
