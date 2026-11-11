//! Jellyfin's part of the settings page (behind the gear): its own playback options, the
//! languages, the home page, the server, user and sign-out, and the VPN (`vpn.rs`). Each control edits
//! `Settings::jellyfin`, saved at once; most apply to the next video or the next visit of Home.
//! The player's shared options are the shell's ("Player").

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Entity, WeakEntity, Window, div, px};
use gpui_component::button::Button;
use gpui_component::{Sizable, h_flex, v_flex};
use jellyfin_api::Client;
use jellyfin_api::models::PublicSystemInfo;
use ui::form::{chips, row, section, toggle};
use ui::i18n::t;
use ui::{runtime, tf, theme};

use super::JellyfinSpace;
use super::session::device_name;
use super::vpn::{Vpn, VpnView};
use peeby_core::settings::{JellyfinHome, JellyfinPlayback, Settings};

/// Audio languages offered (ISO 639-2), after "the server's".
const AUDIO_LANGUAGES: &[&str] = &["jpn", "eng", "fra", "spa", "deu", "ita", "kor", "zho"];

/// Subtitle languages offered, after "the server's" and "off".
const SUBTITLE_LANGUAGES: &[&str] = &["fra", "eng", "spa", "deu", "ita", "por"];

/// The name of a language offered above, in the interface's language.
fn language_name(code: &str) -> &'static str {
  match code {
    "jpn" => t("Japanese", "Japonais"),
    "eng" => t("English", "Anglais"),
    "fra" => t("French", "Français"),
    "spa" => t("Spanish", "Espagnol"),
    "deu" => t("German", "Allemand"),
    "ita" => t("Italian", "Italien"),
    "kor" => t("Korean", "Coréen"),
    "zho" => t("Chinese", "Chinois"),
    "por" => t("Portuguese", "Portugais"),
    _ => "?",
  }
}

/// The languages `codes` as chips' options.
fn language_options(codes: &[&str]) -> impl Iterator<Item = (Option<String>, String)> {
  codes
    .iter()
    .map(|code| (Some((*code).to_owned()), language_name(code).to_owned()))
}

pub struct SettingsView {
  client: Client,
  /// The VPN section (the space's: it also shows signed out).
  vpn: Entity<VpnView>,
  nav: WeakEntity<JellyfinSpace>,
  server: Option<PublicSystemInfo>,
  /// The server's info was asked for.
  asked: bool,
}

impl SettingsView {
  pub fn new(
    client: Client,
    vpn: Entity<VpnView>,
    nav: WeakEntity<JellyfinSpace>,
    cx: &mut Context<Self>,
  ) -> Self {
    cx.observe_global::<Settings>(|_, cx| cx.notify()).detach();
    let mut view = Self {
      client,
      vpn,
      nav,
      server: None,
      asked: false,
    };
    view.ask_server(cx);
    view
  }

  /// Asks the server's name and version once, unless that opens the VPN: the settings page
  /// alone does not connect it (ADR 0037); it is asked once the tunnel is up for another reason.
  fn ask_server(&mut self, cx: &mut Context<Self>) {
    if self.asked || !self.reachable(cx) {
      return;
    }
    self.asked = true;
    let info = runtime::spawn(cx, {
      let client = self.client.clone();
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
  }

  /// Whether a request now leaves without opening the VPN: straight to the server, or through a
  /// session already open.
  fn reachable(&self, cx: &App) -> bool {
    self.client.proxy().is_none()
      || self
        .nav
        .upgrade()
        .and_then(|nav| nav.read(cx).vpn().map(Vpn::connected))
        .unwrap_or(false)
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
      .map(|mb| (mb, tf!("{mb} MB", "{mb} Mo")))
      .to_vec();
    section(
      t("Playback", "Lecture"),
      vec![
        row(
          t("Start in full screen", "Démarrer en plein écran"),
          t(
            "Playback opens in full screen, and leaves it on the way back.",
            "La lecture s'ouvre en plein écran, et en sort au retour.",
          ),
          toggle("fullscreen", p.start_fullscreen, |v, cx| {
            set_playback(cx, |p| p.start_fullscreen = v)
          }),
        )
        .into_any_element(),
        row(
          t("Network buffer", "Mémoire tampon réseau"),
          t(
            "How far ahead the player reads. More helps slow or unstable connections.",
            "Jusqu'où le lecteur lit en avance. Plus aide les connexions lentes ou instables.",
          ),
          chips("buffer", buffers, &p.network_buffer_mb, |v, cx| {
            set_playback(cx, |p| p.network_buffer_mb = v)
          }),
        )
        .into_any_element(),
      ],
    )
  }

  fn languages_section(&self, p: &JellyfinPlayback) -> impl IntoElement {
    let mut audio: Vec<(Option<String>, String)> =
      vec![(None, t("The server's", "Celle du serveur").into())];
    audio.extend(language_options(AUDIO_LANGUAGES));
    let mut subtitles: Vec<(Option<String>, String)> = vec![
      (None, t("The server's", "Ceux du serveur").into()),
      (Some("off".into()), t("Off", "Désactivés").into()),
    ];
    subtitles.extend(language_options(SUBTITLE_LANGUAGES));
    let scales = [0.75, 1.0, 1.25, 1.5, 2.0]
      .map(|s| {
        let percent = s * 100.;
        (s, tf!("{percent:.0}%", "{percent:.0} %"))
      })
      .to_vec();
    section(
      t("Languages and subtitles", "Langues et sous-titres"),
      vec![
        row(
          t("Audio language", "Langue audio"),
          t(
            "Until you pick a track in the series.",
            "Tant que vous n'avez pas choisi de piste dans la série.",
          ),
          chips("audio-lang", audio, &p.audio_language, |v, cx| {
            set_playback(cx, |p| p.audio_language = v)
          }),
        )
        .into_any_element(),
        row(
          t("Subtitle language", "Langue des sous-titres"),
          t(
            "Full subtitles come before signs-only ones.",
            "Les sous-titres complets passent avant ceux des seuls panneaux.",
          ),
          chips("sub-lang", subtitles, &p.subtitle_language, |v, cx| {
            set_playback(cx, |p| p.subtitle_language = v)
          }),
        )
        .into_any_element(),
        row(
          t("Text subtitle size", "Taille des sous-titres texte"),
          t(
            "SRT and other text subtitles. ASS subtitles keep their styles.",
            "SRT et autres sous-titres texte. Les sous-titres ASS gardent leurs styles.",
          ),
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
      (Some(7), t("7 days", "7 jours")),
      (Some(14), t("14 days", "14 jours")),
      (Some(30), t("30 days", "30 jours")),
      (Some(90), t("90 days", "90 jours")),
      (Some(365), t("1 year", "1 an")),
      (None, t("No limit", "Sans limite")),
    ]
    .map(|(d, l)| (d, l.to_owned()))
    .to_vec();
    let mut rows = vec![
      row(
        t(
          "Next up: series watched within",
          "À suivre : séries regardées depuis",
        ),
        t(
          "Series left aside for longer drop out of \"Next up\".",
          "Les séries délaissées depuis plus longtemps sortent de « À suivre ».",
        ),
        chips("next-up-days", ages, &h.next_up_days, |v, cx| {
          set_home(cx, |h| h.next_up_days = v)
        }),
      )
      .into_any_element(),
      row(
        t("Next up: finished series", "À suivre : séries terminées"),
        t(
          "Suggest again the series watched to the end.",
          "Proposer à nouveau les séries regardées jusqu'au bout.",
        ),
        toggle("rewatching", h.next_up_rewatching, |v, cx| {
          set_home(cx, |h| h.next_up_rewatching = v)
        }),
      )
      .into_any_element(),
      row(
        t("Hide spoilers", "Masquer les spoilers"),
        t(
          "Blur the pictures and hide the synopses of episodes not watched yet.",
          "Flouter les images et cacher les résumés des épisodes pas encore vus.",
        ),
        toggle("spoilers", h.hide_spoilers, |v, cx| {
          set_home(cx, |h| h.hide_spoilers = v)
        }),
      )
      .into_any_element(),
      row(
        t("Banner", "Bannière"),
        t(
          "The title to resume, at the top of Home.",
          "Le titre à reprendre, en haut de l'accueil.",
        ),
        toggle("show-hero", h.show_hero, |v, cx| {
          set_home(cx, |h| h.show_hero = v)
        }),
      )
      .into_any_element(),
      row(
        t("Continue watching", "Reprendre"),
        t("The videos you started.", "Les vidéos commencées."),
        toggle("show-continue", h.show_continue_watching, |v, cx| {
          set_home(cx, |h| h.show_continue_watching = v)
        }),
      )
      .into_any_element(),
      row(
        t("Next up", "À suivre"),
        t(
          "The next episode of each series in progress.",
          "L'épisode suivant de chaque série en cours.",
        ),
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
          &tf!("Latest in {name}", "Derniers ajouts dans {name}"),
          t("A row of Home.", "Une rangée de l'accueil."),
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
    section(t("Home", "Accueil"), rows)
  }

  fn account_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
    let server = match &self.server {
      Some(info) => format!(
        "{} · Jellyfin {}",
        info.server_name.clone().unwrap_or_default(),
        info.version.clone().unwrap_or_default()
      ),
      None if self.asked => "…".to_owned(),
      None => t(
        "Not asked while the VPN is on standby",
        "Non demandé tant que le VPN est en veille",
      )
      .to_owned(),
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
      t("Account", "Compte"),
      vec![
        v_flex()
          .px_5()
          .py_4()
          .gap_2()
          .rounded(theme::RADIUS)
          .bg(theme::surface())
          .child(line(t("Server", "Serveur"), server))
          .child(line(
            t("Address", "Adresse"),
            self.client.base_url().to_string(),
          ))
          .child(line(t("User", "Utilisateur"), user))
          .child(line(t("Device", "Appareil"), device_name()))
          .child(
            h_flex().pt_2().child(
              Button::new("jellyfin-sign-out")
                .small()
                .label(t("Sign out", "Se déconnecter"))
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
    // Behind a VPN on standby, once its session opened.
    self.ask_server(cx);
    let jellyfin = Settings::get(cx).jellyfin.clone();
    let (p, h) = (jellyfin.playback, jellyfin.home);
    let sections: Vec<AnyElement> = vec![
      self.account_section(cx).into_any_element(),
      self.vpn.clone().into_any_element(),
      self.playback_section(&p).into_any_element(),
      self.languages_section(&p).into_any_element(),
      self.home_section(&h, cx).into_any_element(),
    ];
    v_flex().gap_8().children(sections)
  }
}
