//! The settings page behind the gear: the app's own settings (general, player, storage,
//! about) and, between them, each space's part under its name, in its accent: the open space's
//! first and unfolded, the others folded. Every change applies at once and is saved to
//! `settings.toml`; most apply to the next video or the next visit of a page.

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, Subscription, Window, div, px};
use gpui_component::button::Button;
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Disableable, Icon, IconName, Sizable, h_flex, v_flex};
use ui::form::{chips, format_size, row, section, toggle};
use ui::player::format_step;
use ui::theme::Accent;
use ui::widgets::page_title;
use ui::{images, theme};

use crate::settings::{Player, Settings};
use crate::shell::space::SpaceHandle;

pub struct SettingsView {
  /// The open space first, then the others in the sidebar's order.
  spaces: Vec<SpaceHandle>,
  /// Whether each space's part is unfolded.
  unfolded: Vec<bool>,
  cache_size: Option<u64>,
  clearing: bool,
  _subscriptions: Vec<Subscription>,
}

impl SettingsView {
  /// The page with `spaces`, `active` the one open behind it.
  pub fn new(mut spaces: Vec<SpaceHandle>, active: usize, cx: &mut Context<Self>) -> Self {
    if active < spaces.len() {
      let open = spaces.remove(active);
      spaces.insert(0, open);
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
      unfolded,
      cache_size: None,
      clearing: false,
      _subscriptions: subscriptions,
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

fn set_player(cx: &mut gpui::App, edit: impl FnOnce(&mut Player)) {
  Settings::update(cx, |s| edit(&mut s.player));
}

fn general_section(settings: &Settings) -> AnyElement {
  section(
    "Général",
    vec![
      row(
        "Menu réduit",
        "La barre latérale ne montre que ses icônes.",
        toggle("sidebar-collapsed", settings.sidebar_collapsed, |v, cx| {
          Settings::update(cx, |s| s.sidebar_collapsed = v)
        }),
      )
      .into_any_element(),
    ],
  )
  .into_any_element()
}

fn player_section(p: &Player) -> AnyElement {
  let countdowns = [5, 10, 15, 20, 30].map(|s| (s, format!("{s} s"))).to_vec();
  let steps = [5, 10, 15, 30].map(|s| (s, format_step(s))).to_vec();
  let large_steps = [30, 60, 120, 300].map(|s| (s, format_step(s))).to_vec();
  section(
    "Lecteur",
    vec![
      row(
        "Lire la suite automatiquement",
        "L'épisode suivant ou la vidéo suivante d'une playlist, après un compte à rebours.",
        toggle("autoplay", p.autoplay_next, |v, cx| {
          set_player(cx, |p| p.autoplay_next = v)
        }),
      )
      .into_any_element(),
      row(
        "Compte à rebours",
        "Avant de lancer la suite.",
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
        "Mémoriser le volume",
        "Garder le volume et la coupure du son d'une vidéo à l'autre.",
        toggle("remember-volume", p.remember_volume, |v, cx| {
          set_player(cx, |p| p.remember_volume = v)
        }),
      )
      .into_any_element(),
      row(
        "Saut",
        "← / → et les boutons autour de lecture/pause.",
        chips("seek", steps, &p.seek_step, |v, cx| {
          set_player(cx, |p| p.seek_step = v)
        }),
      )
      .into_any_element(),
      row(
        "Grand saut",
        "Maj + ← / →.",
        chips("seek-large", large_steps, &p.seek_step_large, |v, cx| {
          set_player(cx, |p| p.seek_step_large = v)
        }),
      )
      .into_any_element(),
      row(
        "Accélération matérielle",
        "Décoder et afficher la vidéo avec la carte graphique. À couper seulement pour un \
         dépannage.",
        toggle("hwaccel", p.hardware_acceleration, |v, cx| {
          set_player(cx, |p| p.hardware_acceleration = v)
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
      Some(bytes) => format!("{} sur le disque.", format_size(bytes)),
      None => "Calcul…".to_owned(),
    };
    section(
      "Stockage",
      vec![
        row(
          "Cache des images",
          &format!("Miniatures, affiches et fonds gardés sur ce PC. {size}"),
          Button::new("clear-cache")
            .small()
            .label("Vider")
            .loading(self.clearing)
            .disabled(self.clearing)
            .on_click(cx.listener(|this, _, _, cx| this.clear_cache(cx))),
        )
        .into_any_element(),
        row(
          "Fichiers",
          "Réglages et cookies YouTube dans %APPDATA%\\peeby, caches (images, polices, \
           yt-dlp) dans %LOCALAPPDATA%\\peeby. Les jetons Jellyfin et Twitch sont dans le \
           Gestionnaire d'identification de Windows.",
          div(),
        )
        .into_any_element(),
      ],
    )
    .into_any_element()
  }
}

fn about_section() -> AnyElement {
  section(
    "À propos",
    vec![
      row(
        "Peeby",
        &format!(
          "Version {}. Lecture avec libmpv ; « J'aime » et « Je n'aime pas » de Return YouTube \
           Dislike (returnyoutubedislike.com).",
          env!("CARGO_PKG_VERSION")
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
    let parts: Vec<AnyElement> = self
      .spaces
      .clone()
      .iter()
      .enumerate()
      .filter_map(|(index, space)| {
        let info = space.info(cx);
        // Built in its accent too, for the colors read while building it.
        let part = theme::with_accent(info.accent, cx, |cx| space.settings(window, cx))?;
        Some(self.space_part(index, info.name, info.accent, part, cx))
      })
      .collect();
    v_flex()
      .id("settings")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        v_flex()
          .w_full()
          .max_w(px(920.))
          .px_8()
          .py_6()
          .gap_8()
          .child(page_title("Réglages"))
          .child(general_section(&settings))
          .child(player_section(&settings.player))
          .children(parts)
          .child(self.storage_section(cx))
          .child(about_section())
          .child(
            div()
              .text_xs()
              .text_color(theme::subtle())
              .child("Les changements sont enregistrés aussitôt."),
          ),
      )
  }
}
