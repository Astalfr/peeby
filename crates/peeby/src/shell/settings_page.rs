//! The settings page behind the gear: the app's own settings (general, player, storage,
//! debug, about) and, between them, each space's part under its name, in its accent: the open space's
//! first and unfolded, the others folded. Every change applies at once and is saved to
//! `settings.toml`; most apply to the next video or the next visit of a page.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, ScrollHandle, Subscription, Task, Window, div, px};
use gpui_component::button::Button;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Disableable, Icon, IconName, Sizable, h_flex, v_flex};
use ui::form::{OnClick, chips, choices, format_size, row, section, toggle};
use ui::i18n::{Lang, t};
use ui::player::{Owner, RunningPlayer, format_step, format_time};
use ui::theme::Accent;
use ui::widgets::page_title;
use ui::{images, tf, theme};

use crate::desktop;
use crate::settings::{Delivery, Notifications, Player, Settings};
use crate::shell::space::{SpaceHandle, SpaceInfo};

pub struct SettingsView {
  /// The open space first, then the others in the sidebar's order.
  spaces: Vec<SpaceHandle>,
  /// Every space in the tabs' order, for the startup tabs.
  in_order: Vec<SpaceInfo>,
  /// The index in the tabs' order of each of `spaces`.
  order: Vec<usize>,
  /// Whether each space's part is unfolded.
  unfolded: Vec<bool>,
  scroll: ScrollHandle,
  /// The part (an index in `spaces`) to scroll to at the next render.
  scroll_to: Option<usize>,
  cache_size: Option<u64>,
  clearing: bool,
  _subscriptions: Vec<Subscription>,
  /// Redraws the page every second, for the debug section's players.
  _tick: Task<()>,
}

impl SettingsView {
  /// The page with `spaces`, `active` the one open behind it.
  pub fn new(mut spaces: Vec<SpaceHandle>, active: usize, cx: &mut Context<Self>) -> Self {
    let in_order = spaces.iter().map(|space| space.info(cx)).collect();
    let mut order: Vec<usize> = (0..spaces.len()).collect();
    if active < spaces.len() {
      let open = spaces.remove(active);
      spaces.insert(0, open);
      order.remove(active);
      order.insert(0, active);
    }
    let unfolded = (0..spaces.len()).map(|i| i == 0).collect();
    let mut subscriptions = vec![cx.observe_global::<Settings>(|_, cx| cx.notify())];
    // A space's part changes with it (sign-in, the libraries arriving).
    for space in &spaces {
      let this = cx.weak_entity();
      subscriptions.push(space.observe(
        cx,
        Box::new(move |cx| {
          this.update(cx, |_, cx| cx.notify()).ok();
        }),
      ));
    }
    let mut view = Self {
      spaces,
      in_order,
      order,
      unfolded,
      scroll: ScrollHandle::new(),
      scroll_to: None,
      cache_size: None,
      clearing: false,
      _subscriptions: subscriptions,
      _tick: cx.spawn(async move |this, cx| {
        loop {
          cx.background_executor()
            .timer(std::time::Duration::from_secs(1))
            .await;
          if this.update(cx, |_, cx| cx.notify()).is_err() {
            break;
          }
        }
      }),
    };
    view.measure_cache(cx);
    view
  }

  /// Unfolds the part of space `index` (in the tabs' order) and scrolls to it: its account
  /// comes first (a sign-in request).
  pub fn reveal(&mut self, index: usize, cx: &mut Context<Self>) {
    let Some(at) = self.order.iter().position(|&i| i == index) else {
      return;
    };
    if let Some(unfolded) = self.unfolded.get_mut(at) {
      *unfolded = true;
    }
    self.scroll_to = Some(at);
    cx.notify();
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

fn set_player(cx: &mut gpui::App, edit: impl FnOnce(&mut Player)) {
  Settings::update(cx, |s| edit(&mut s.player));
}

/// Switches the interface to `lang` and re-renders every window (the mini-player's too).
fn set_language(lang: Lang, cx: &mut gpui::App) {
  Settings::update(cx, |s| s.interface_language = lang.code().to_owned());
  ui::i18n::set(lang);
  cx.refresh_windows();
}

/// The startup tabs: a chip per space, those that open a tab lit; a click adds or removes one,
/// never the last.
fn startup_tabs(settings: &Settings, spaces: &[SpaceInfo]) -> impl IntoElement {
  let options: Vec<(String, bool, OnClick)> = spaces
    .iter()
    .map(|info| {
      let id = info.id;
      let on = settings.opens_at_startup(id);
      let on_click: OnClick = Box::new(move |_, cx| {
        Settings::update(cx, |s| {
          if !s.opens_at_startup(id) {
            s.startup_tabs.push(id.to_owned());
          } else if s.startup_tabs.len() > 1 {
            s.startup_tabs.retain(|t| t != id);
          }
        })
      });
      (info.name.to_string(), on, on_click)
    })
    .collect();
  choices("startup-tabs", options)
}

fn general_section(settings: &Settings, spaces: &[SpaceInfo]) -> AnyElement {
  let languages = Lang::ALL.map(|l| (l, l.name().to_owned())).to_vec();
  section(
    t("General", "Général"),
    vec![
      row(
        t("Language", "Langue"),
        t(
          "Language of Peeby's interface. What the services send (titles, YouTube's dates) \
           keeps the language of their settings.",
          "Langue de l'interface de Peeby. Ce que les services envoient (titres, dates de \
           YouTube) garde la langue de leurs réglages.",
        ),
        chips(
          "interface-language",
          languages,
          &settings.interface_lang(),
          set_language,
        ),
      )
      .into_any_element(),
      row(
        t("Startup tabs", "Onglets au démarrage"),
        t(
          "The services that open a tab when Peeby starts.",
          "Les services qui ouvrent un onglet au démarrage de Peeby.",
        ),
        startup_tabs(settings, spaces),
      )
      .into_any_element(),
      row(
        t("Collapsed menu", "Menu réduit"),
        t(
          "The sidebar shows only its icons.",
          "La barre latérale ne montre que ses icônes.",
        ),
        toggle("sidebar-collapsed", settings.sidebar_collapsed, |v, cx| {
          Settings::update(cx, |s| s.sidebar_collapsed = v)
        }),
      )
      .into_any_element(),
      row(
        t("Keep running in the background", "Rester en arrière-plan"),
        t(
          "Closing the window keeps Peeby in the notification area, for the notifications; its icon's menu quits. Off, closing the window quits.",
          "Fermer la fenêtre garde Peeby dans la zone de notification, pour les notifications ; le menu de son icône le ferme. Désactivé, fermer la fenêtre quitte Peeby.",
        ),
        toggle("close-to-tray", settings.close_to_tray, |v, cx| {
          Settings::update(cx, |s| s.close_to_tray = v)
        }),
      )
      .into_any_element(),
      row(
        t("Start with Windows", "Démarrer avec Windows"),
        t(
          "Peeby starts in the notification area when you sign in to Windows.",
          "Peeby démarre dans la zone de notification à l'ouverture de session Windows.",
        ),
        toggle(
          "start-with-windows",
          settings.start_with_windows,
          desktop::set_start_with_windows,
        ),
      )
      .into_any_element(),
    ],
  )
  .into_any_element()
}

/// The row of a kind of notification, for a space's part: two switches, Peeby's bell and
/// Windows' notifications; `kind` picks the kind's setting.
pub fn delivery_row(
  id: &str,
  title: &str,
  description: &str,
  kind: fn(&mut Notifications) -> &mut Delivery,
  cx: &gpui::App,
) -> AnyElement {
  let delivery = *kind(&mut Settings::get(cx).notifications.clone());
  let switch = |suffix: &str, label: &'static str, on: bool, set: fn(&mut Delivery, bool)| {
    h_flex()
      .gap_2()
      .items_center()
      .child(div().text_sm().text_color(theme::muted()).child(label))
      .child(toggle(&format!("{id}-{suffix}"), on, move |v, cx| {
        Settings::update(cx, |s| set(kind(&mut s.notifications), v))
      }))
  };
  row(
    title,
    description,
    h_flex()
      .gap_5()
      .child(switch("app", "Peeby", delivery.in_app, |d, v| d.in_app = v))
      .child(switch("windows", "Windows", delivery.windows, |d, v| {
        d.windows = v
      })),
  )
  .into_any_element()
}

fn player_section(p: &Player) -> AnyElement {
  let countdowns = [5, 10, 15, 20, 30].map(|s| (s, format!("{s} s"))).to_vec();
  let steps = [5, 10, 15, 30].map(|s| (s, format_step(s))).to_vec();
  let large_steps = [30, 60, 120, 300].map(|s| (s, format_step(s))).to_vec();
  section(
    t("Player", "Lecteur"),
    vec![
      row(
        t("Autoplay next", "Lire la suite automatiquement"),
        t(
          "The next episode or the next video of a playlist, after a countdown.",
          "L'épisode suivant ou la vidéo suivante d'une playlist, après un compte à rebours.",
        ),
        toggle("autoplay", p.autoplay_next, |v, cx| {
          set_player(cx, |p| p.autoplay_next = v)
        }),
      )
      .into_any_element(),
      row(
        t("Countdown", "Compte à rebours"),
        t("Before playing the next one.", "Avant de lancer la suite."),
        div()
          .when(!p.autoplay_next, |el| el.opacity(0.5))
          .child(chips(
            "countdown",
            countdowns,
            &p.autoplay_countdown,
            |v, cx| set_player(cx, |p| p.autoplay_countdown = v),
          )),
      )
      .into_any_element(),
      row(
        t("Remember the volume", "Mémoriser le volume"),
        t(
          "Keep the volume and the mute from one video to the next.",
          "Garder le volume et la coupure du son d'une vidéo à l'autre.",
        ),
        toggle("remember-volume", p.remember_volume, |v, cx| {
          set_player(cx, |p| p.remember_volume = v)
        }),
      )
      .into_any_element(),
      row(
        t("Seek step", "Saut"),
        t(
          "← / → and the buttons around play/pause.",
          "← / → et les boutons autour de lecture/pause.",
        ),
        chips("seek", steps, &p.seek_step, |v, cx| {
          set_player(cx, |p| p.seek_step = v)
        }),
      )
      .into_any_element(),
      row(
        t("Large seek step", "Grand saut"),
        t("Shift + ← / →.", "Maj + ← / →."),
        chips("seek-large", large_steps, &p.seek_step_large, |v, cx| {
          set_player(cx, |p| p.seek_step_large = v)
        }),
      )
      .into_any_element(),
      row(
        t("Hardware acceleration", "Accélération matérielle"),
        t(
          "Decode and display the video with the graphics card. Turn it off only to \
           troubleshoot.",
          "Décoder et afficher la vidéo avec la carte graphique. À couper seulement pour un \
           dépannage.",
        ),
        toggle("hwaccel", p.hardware_acceleration, |v, cx| {
          set_player(cx, |p| p.hardware_acceleration = v)
        }),
      )
      .into_any_element(),
      row(
        t("mpv logs", "Logs de mpv"),
        t(
          "Write the player's log for each video to %LOCALAPPDATA%\\peeby\\logs, kept 7 days, \
           from the next video on. Only to troubleshoot: it holds the stream addresses, which \
           carry your IP address.",
          "Écrire le log du lecteur de chaque vidéo dans %LOCALAPPDATA%\\peeby\\logs, gardé \
           7 jours, à partir de la vidéo suivante. Seulement pour un dépannage : il contient les \
           adresses des flux, qui portent votre adresse IP.",
        ),
        toggle("mpv-log", p.mpv_log, |v, cx| {
          set_player(cx, |p| p.mpv_log = v)
        }),
      )
      .into_any_element(),
    ],
  )
  .into_any_element()
}

impl SettingsView {
  /// Space `index`'s part, under its name in its accent; a click on the name folds or unfolds it.
  fn space_part(
    &self,
    index: usize,
    name: gpui::SharedString,
    accent: Accent,
    part: AnyElement,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let unfolded = self.unfolded.get(index).copied().unwrap_or(false);
    let header = h_flex()
      .id(gpui::SharedString::from(format!("settings-space-{index}")))
      .gap_2()
      .items_center()
      .cursor_pointer()
      .text_color(theme::foreground())
      .hover(|el| el.text_color(accent.hover))
      .on_click(cx.listener(move |this, _, _, cx| {
        if let Some(unfolded) = this.unfolded.get_mut(index) {
          *unfolded = !*unfolded;
          cx.notify();
        }
      }))
      .child(div().w(px(3.)).h(px(20.)).rounded_full().bg(accent.base))
      .child(div().text_xl().font_weight(FontWeight::BOLD).child(name))
      .child(
        Icon::new(if unfolded {
          IconName::ChevronDown
        } else {
          IconName::ChevronRight
        })
        .small()
        .text_color(theme::muted()),
      );
    v_flex()
      .gap_4()
      .child(header)
      .when(unfolded, |el| el.child(theme::tinted(accent, part)))
      .into_any_element()
  }

  fn storage_section(&self, cx: &mut Context<Self>) -> AnyElement {
    let size = match self.cache_size {
      Some(bytes) => {
        let size = format_size(bytes);
        tf!("{size} on disk.", "{size} sur le disque.")
      }
      None => t("Measuring…", "Calcul…").to_owned(),
    };
    section(
      t("Storage", "Stockage"),
      vec![
        row(
          t("Image cache", "Cache des images"),
          &tf!(
            "Thumbnails, posters and backdrops kept on this PC. {size}",
            "Miniatures, affiches et fonds gardés sur ce PC. {size}"
          ),
          Button::new("clear-cache")
            .small()
            .label(t("Clear", "Vider"))
            .loading(self.clearing)
            .disabled(self.clearing)
            .on_click(cx.listener(|this, _, _, cx| this.clear_cache(cx))),
        )
        .into_any_element(),
        row(
          t("Files", "Fichiers"),
          t(
            "Settings and YouTube cookies in %APPDATA%\\peeby, caches (images, fonts, yt-dlp) \
             in %LOCALAPPDATA%\\peeby. The Jellyfin and Twitch tokens are in the Windows \
             Credential Manager.",
            "Réglages et cookies YouTube dans %APPDATA%\\peeby, caches (images, polices, \
             yt-dlp) dans %LOCALAPPDATA%\\peeby. Les jetons Jellyfin et Twitch sont dans le \
             Gestionnaire d'identification de Windows.",
          ),
          div(),
        )
        .into_any_element(),
      ],
    )
    .into_any_element()
  }
}

/// The mpv players alive, to find one that should be gone (a page closed, a quality replaced).
fn debug_section(players: &[RunningPlayer]) -> AnyElement {
  let rows = if players.is_empty() {
    vec![
      row(
        t("mpv players", "Lecteurs mpv"),
        t("None running.", "Aucun en cours."),
        div(),
      )
      .into_any_element(),
    ]
  } else {
    players
      .iter()
      .enumerate()
      .map(|(i, p)| {
        let n = i + 1;
        let title = if p.title.is_empty() {
          tf!("Player {n}", "Lecteur {n}")
        } else {
          p.title.to_string()
        };
        row(&title, &player_line(p), div()).into_any_element()
      })
      .collect()
  };
  section(t("Debug", "Débogage"), rows).into_any_element()
}

/// Position, state, decoding, age and owner of a running player.
fn player_line(p: &RunningPlayer) -> String {
  let position = match (p.position, p.duration) {
    (Some(pos), Some(dur)) if dur > 0.0 => format!("{} / {}", format_time(pos), format_time(dur)),
    (Some(pos), _) => format_time(pos),
    _ => "–".to_owned(),
  };
  let state = if p.buffering {
    t("buffering", "en attente")
  } else if p.paused {
    t("paused", "en pause")
  } else {
    t("playing", "en lecture")
  };
  let backend = match p.backend {
    player::Backend::Gpu => "GPU",
    player::Backend::Software => "CPU",
  };
  let decoding = match p.hwdec.as_deref() {
    None | Some("") | Some("no") => t("software decoding", "décodage logiciel").to_owned(),
    Some(hwdec) => hwdec.to_owned(),
  };
  let minutes = p.age.as_secs() / 60;
  let age = tf!("started {minutes} min ago", "lancé il y a {minutes} min");
  let owner = match p.owner {
    Owner::Page if p.now_playing => t("its page, mini-player's", "sa page, celui du mini-lecteur"),
    Owner::Page => t("its page", "sa page"),
    Owner::Replaced => t(
      "replaced on its page, still held",
      "remplacé sur sa page, encore retenu",
    ),
    Owner::Closed => t(
      "its page is closed, still held",
      "sa page est fermée, encore retenu",
    ),
  };
  format!("{position} · {state} · {backend}, {decoding} · {age} · {owner}")
}

fn about_section() -> AnyElement {
  let version = env!("CARGO_PKG_VERSION");
  section(
    t("About", "À propos"),
    vec![
      row(
        "Peeby",
        &tf!(
          "Version {version}. Playback with libmpv; likes and dislikes from Return YouTube \
           Dislike (returnyoutubedislike.com).",
          "Version {version}. Lecture avec libmpv ; « J'aime » et « Je n'aime pas » de Return \
           YouTube Dislike (returnyoutubedislike.com)."
        ),
        div(),
      )
      .into_any_element(),
    ],
  )
  .into_any_element()
}

impl Render for SettingsView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let settings = Settings::get(cx).clone();
    let players = ui::player::running(cx);
    // The page's blocks are the scrolled element's own children, so that a part can be
    // scrolled to by its index.
    let mut blocks: Vec<AnyElement> = vec![
      page_title(t("Settings", "Réglages")).into_any_element(),
      general_section(&settings, &self.in_order),
      player_section(&settings.player),
    ];
    for (index, space) in self.spaces.clone().iter().enumerate() {
      let info = space.info(cx);
      // Built in its accent too, for the colors read while building it.
      let Some(part) = theme::with_accent(info.accent, cx, |cx| space.settings(window, cx)) else {
        continue;
      };
      if self.scroll_to == Some(index) {
        self.scroll_to = None;
        self.scroll.scroll_to_top_of_item(blocks.len());
      }
      blocks.push(self.space_part(index, info.name, info.accent, part, cx));
    }
    blocks.extend([
      self.storage_section(cx),
      debug_section(&players),
      about_section(),
      div()
        .text_xs()
        .text_color(theme::subtle())
        .child(t(
          "Changes are saved at once.",
          "Les changements sont enregistrés aussitôt.",
        ))
        .into_any_element(),
    ]);
    div()
      .size_full()
      .relative()
      .child(
        v_flex()
          .id("settings")
          .size_full()
          .overflow_y_scroll()
          .track_scroll(&self.scroll)
          .px_8()
          .py_6()
          .gap_8()
          .children(
            blocks
              .into_iter()
              .map(|block| div().w_full().max_w(px(920.)).child(block)),
          ),
      )
      .vertical_scrollbar(&self.scroll)
  }
}
