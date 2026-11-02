//! What the player draws over the picture besides its control bar: the menus (quality, audio,
//! subtitles, chapters), the top bar of the full-window layout with its window buttons, the
//! skip button, the "up next" card, the shortcut help and the error panel. From peeby's player
//! page.

use gpui::prelude::*;
use gpui::{
  AnyElement, Context, FontWeight, Pixels, Point, SharedString, Window, WindowControlArea, div,
  linear_color_stop, linear_gradient, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{ActiveTheme, IconName, Sizable, h_flex, v_flex};
use player::TrackKind;

use super::tracks::{SUBTITLE_DELAY_STEP, UpNext};
use super::{CONTROL_ICON, CONTROL_SIZE, Layout, Menu, PlayerView, format_time};
use crate::assets::Icon;
use crate::i18n::t;
use crate::tf;

/// The help's rows, keys and action in English then French; an empty action is the seek line,
/// whose steps come from the preferences (see `render_help`).
const SHORTCUTS: &[(&str, &str, &str, &str)] = &[
  (
    "Space / K",
    "Espace / K",
    "Play or pause",
    "Lecture ou pause",
  ),
  ("← / → · J / L", "← / → · J / L", "", ""),
  (
    "Page Up / Page Down",
    "Page préc. / Page suiv.",
    "Previous or next chapter",
    "Chapitre précédent ou suivant",
  ),
  ("0 – 9", "0 – 9", "Jump to 0% – 90%", "Aller à 0 % – 90 %"),
  ("Home", "Début", "Back to the start", "Retour au début"),
  ("↑ / ↓", "↑ / ↓", "Volume", "Volume"),
  ("M", "M", "Mute", "Couper le son"),
  ("A", "A", "Next audio track", "Piste audio suivante"),
  (
    "C",
    "C",
    "Next subtitles (or off)",
    "Sous-titres suivants (ou désactivés)",
  ),
  (
    "Z / X",
    "Z / X",
    "Subtitle delay −/+ 0.1 s",
    "Décalage des sous-titres −/+ 0,1 s",
  ),
  (
    "S",
    "S",
    "Skip the intro, recap or credits",
    "Passer l'intro, le récap ou le générique",
  ),
  ("P / N", "P / N", "Previous or next", "Précédent ou suivant"),
  ("T", "T", "Theater mode", "Mode cinéma"),
  (
    "F / double-click",
    "F / double-clic",
    "Full screen",
    "Plein écran",
  ),
  (
    "Esc",
    "Échap",
    "Close the menu, leave full screen, then go back",
    "Fermer le menu, quitter le plein écran, puis revenir",
  ),
  ("?", "?", "This help", "Cette aide"),
  (
    "Right-click",
    "Clic droit",
    "Keyboard shortcuts, statistics",
    "Raccourcis clavier, statistiques",
  ),
];

/// Rows of [`SHORTCUTS`] that move the playhead (by their English keys), left out for a live
/// without its past.
const SEEK_KEYS: &[&str] = &["← / → · J / L", "Page Up / Page Down", "0 – 9", "Home"];

/// `10 sec`, `1 min`, `1 min 30 sec` (`10 s`, `1 min`, `1 min 30 s` in French).
pub fn format_step(seconds: u32) -> String {
  match (seconds / 60, seconds % 60) {
    (0, s) => tf!("{s} sec", "{s} s"),
    (m, 0) => tf!("{m} min", "{m} min"),
    (m, s) => tf!("{m} min {s} sec", "{m} min {s} s"),
  }
}

impl PlayerView {
  /// The open menu above the control bar.
  pub(super) fn render_menu(&self, menu: Menu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let (popover, border) = (theme.popover, theme.border);
    let mut list = v_flex().gap_1();
    let entry = |id: SharedString, label: String, selected: bool| {
      Button::new(id)
        .label(label)
        .when(selected, |b| b.primary())
        .when(!selected, |b| b.ghost())
    };
    match menu {
      Menu::Quality => {
        let qualities = self
          .source
          .as_ref()
          .map(|s| s.qualities.clone())
          .unwrap_or_default();
        for (i, quality) in qualities.iter().enumerate() {
          list = list.child(
            entry(
              format!("quality-{i}").into(),
              quality.label.to_string(),
              self.quality == Some(i),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_quality(i, cx))),
          );
        }
      }
      Menu::Audio => {
        let current = self.selected(TrackKind::Audio);
        for track in self.tracks_of(TrackKind::Audio) {
          let id = track.id;
          list = list.child(
            entry(
              format!("audio-{id}").into(),
              self.track_label(&track),
              current == Some(id),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_audio(id, cx))),
          );
        }
      }
      Menu::Subtitles if self.on_demand_subtitles() => {
        list = list.child(
          entry(
            "sub-off".into(),
            t("Off", "Désactivés").into(),
            self.caption.is_none(),
          )
          .on_click(cx.listener(|this, _, _, cx| this.select_caption(None, cx))),
        );
        let subtitles = self
          .source
          .as_ref()
          .map(|s| s.subtitles.clone())
          .unwrap_or_default();
        for (i, caption) in subtitles.iter().enumerate() {
          list = list.child(
            entry(
              format!("sub-{i}").into(),
              caption.label.clone(),
              self.caption == Some(i),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_caption(Some(i), cx))),
          );
        }
        list = list.child(self.delay_line());
      }
      Menu::Subtitles => {
        let current = self.selected(TrackKind::Subtitle);
        list = list.child(
          entry(
            "sub-off".into(),
            t("Off", "Désactivés").into(),
            current.is_none(),
          )
          .on_click(cx.listener(|this, _, _, cx| this.select_subtitle(None, cx))),
        );
        for track in self.tracks_of(TrackKind::Subtitle) {
          let id = track.id;
          list = list.child(
            entry(
              format!("sub-{id}").into(),
              self.track_label(&track),
              current == Some(id),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_subtitle(Some(id), cx))),
          );
        }
        list = list.child(self.delay_line());
      }
      Menu::Chapters => {
        let current = self.chapter_at(self.position + 0.5);
        for i in 0..self.chapters.len() {
          let start = self.chapters[i].start;
          list = list.child(
            entry(
              format!("chapter-{i}").into(),
              format!("{}  ·  {}", format_time(start), self.chapter_title(i)),
              current == Some(i),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
              this.menu = None;
              this.seek_to(start, cx);
            })),
          );
        }
      }
    }
    div()
      .id("menu")
      .occlude()
      .absolute()
      .right_4()
      .bottom(px(84.))
      .max_h(px(380.))
      .min_w(px(240.))
      .overflow_y_scroll()
      .p_2()
      .rounded_lg()
      .bg(popover)
      .border_1()
      .border_color(border)
      .child(list)
  }

  /// The subtitle delay under the subtitle tracks.
  fn delay_line(&self) -> impl IntoElement + use<> {
    div()
      .px_3()
      .pt_1()
      .text_xs()
      .text_color(gpui::white().opacity(0.6))
      .child(tf!(
        "Delay {:+.1} s (Z / X, steps of {SUBTITLE_DELAY_STEP} s)",
        "Décalage {:+.1} s (Z / X, pas de {SUBTITLE_DELAY_STEP} s)",
        self.subtitle_delay
      ))
  }

  /// Top bar of a player that is the page: back, title and subtitle; in the full-window layout
  /// also the window buttons (it doubles as the window's title bar, a drag area).
  pub(super) fn render_top_bar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
    let fullscreen = window.is_fullscreen();
    let window_chrome = self.layout == Layout::Window && !fullscreen;
    let maximized = window.is_maximized();
    div()
      .id("top-bar")
      .occlude()
      .on_hover(cx.listener(|this, hovered: &bool, _, _| this.over_controls = *hovered))
      .on_mouse_move(cx.listener(|this, _, _, cx| this.show_controls(cx)))
      .absolute()
      .top_0()
      .left_0()
      .right_0()
      .pb_10()
      .bg(linear_gradient(
        180.,
        linear_color_stop(gpui::black().opacity(0.75), 0.0),
        linear_color_stop(gpui::black().opacity(0.0), 1.0),
      ))
      // The back button and the title sit on a layer above the drag area, so it cannot take
      // their clicks.
      .child(
        h_flex()
          .h(px(64.))
          .pr_3()
          .gap_1()
          .items_center()
          .child(
            div()
              .id("player-drag")
              .flex_1()
              .h_full()
              .when(window_chrome, |el| {
                el.window_control_area(WindowControlArea::Drag)
              }),
          )
          // A fullscreen window has nothing to drag, minimize or restore: Escape or F leaves it.
          .when(window_chrome, |row| {
            row
              .child(self.window_button(WindowControlArea::Min, IconName::WindowMinimize, cx))
              .child(self.window_button(
                WindowControlArea::Max,
                if maximized {
                  IconName::WindowRestore
                } else {
                  IconName::WindowMaximize
                },
                cx,
              ))
              .child(self.window_button(WindowControlArea::Close, IconName::WindowClose, cx))
          }),
      )
      .child(
        h_flex()
          .id("player-title")
          .occlude()
          .absolute()
          .top_0()
          .left(px(12.))
          .h(px(64.))
          .gap_3()
          .items_center()
          .child(
            self
              .control_button("player-back", cx)
              .icon(Icon::ArrowLeft)
              .tooltip(t("Back (Esc)", "Retour (Échap)"))
              .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
          )
          .child(
            v_flex()
              .child(
                div()
                  .text_lg()
                  .font_weight(FontWeight::SEMIBOLD)
                  .text_color(gpui::white())
                  .child(self.title.clone()),
              )
              .when(!self.subtitle.is_empty(), |el| {
                el.child(
                  div()
                    .text_sm()
                    .text_color(gpui::white().opacity(0.75))
                    .child(self.subtitle.clone()),
                )
              }),
          ),
      )
      .into_any_element()
  }

  /// A window button over the video: round like the player's other buttons; close turns red.
  fn window_button(
    &self,
    area: WindowControlArea,
    icon: IconName,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let id = match area {
      WindowControlArea::Min => "player-minimize",
      WindowControlArea::Max => "player-maximize",
      _ => "player-close",
    };
    let close = matches!(area, WindowControlArea::Close);
    let (hover, active) = if close {
      (cx.theme().danger, cx.theme().danger_active)
    } else {
      (gpui::white().opacity(0.18), gpui::white().opacity(0.26))
    };
    let foreground = if self.hovered_control == Some(id) && !close {
      cx.theme().primary
    } else {
      gpui::white()
    };
    let view = cx.weak_entity();
    div()
      .id(id)
      .size(px(CONTROL_SIZE))
      .flex_none()
      .flex()
      .items_center()
      .justify_center()
      .rounded(cx.theme().radius)
      .text_color(foreground)
      .hover(move |s| s.bg(hover))
      .active(move |s| s.bg(active))
      .on_hover(move |hovered, _, cx| {
        view
          .update(cx, |this, cx| {
            if *hovered {
              this.hovered_control = Some(id);
            } else if this.hovered_control == Some(id) {
              this.hovered_control = None;
            }
            cx.notify();
          })
          .ok();
      })
      .window_control_area(area)
      .child(gpui_component::Icon::new(icon).size(px(CONTROL_ICON)))
  }

  /// The skip button while a segment plays, above the control bar when it shows.
  pub(super) fn render_skip(&self, show_ui: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
    let (label, _) = self.skip_action()?;
    Some(
      div()
        .id("skip")
        .occlude()
        .absolute()
        .right_6()
        .bottom(px(if show_ui { 96. } else { 32. }))
        .child(
          Button::new("skip-button")
            .primary()
            .large()
            .label(format!("{label}  (S)"))
            .on_click(cx.listener(|this, _, _, cx| this.skip(cx))),
        )
        .into_any_element(),
    )
  }

  /// The "up next" card, with its countdown when autoplay is on.
  pub(super) fn render_up_next(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
    let UpNext::Offered(left) = self.up_next else {
      return None;
    };
    let title = self.up_next_title.clone()?;
    let theme = cx.theme();
    let (popover, border, muted) = (theme.popover, theme.border, theme.muted_foreground);
    let status = match left {
      Some(n) => tf!("Playing in {n} s", "Lecture dans {n} s"),
      None => t("Up next", "À suivre").to_owned(),
    };
    Some(
      v_flex()
        .id("up-next")
        .occlude()
        .absolute()
        .right_6()
        .bottom(px(110.))
        .w(px(320.))
        .p_4()
        .gap_3()
        .rounded_lg()
        .bg(popover)
        .border_1()
        .border_color(border)
        .child(div().text_xs().text_color(muted).child(status))
        .child(
          div()
            .font_weight(FontWeight::SEMIBOLD)
            .line_clamp(2)
            .child(title),
        )
        .child(
          h_flex()
            .gap_2()
            .child(
              Button::new("up-next-play")
                .primary()
                .icon(Icon::Next)
                .label(t("Play now", "Lire maintenant"))
                .on_click(cx.listener(|this, _, _, cx| this.play_next(cx))),
            )
            .child(
              Button::new("up-next-cancel")
                .ghost()
                .label(t("Cancel", "Annuler"))
                .on_click(cx.listener(|this, _, _, cx| this.dismiss_up_next(cx))),
            ),
        )
        .into_any_element(),
    )
  }

  /// The keyboard shortcuts (?).
  pub(super) fn render_help(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let (popover, border, muted) = (theme.popover, theme.border, theme.muted_foreground);
    let embedded = self.layout == Layout::Embedded;
    let can_seek = self.can_seek();
    let seek = tf!(
      "Seek back or forward {} (Shift: {})",
      "Reculer ou avancer de {} (Maj : {})",
      format_step(self.prefs.seek_step),
      format_step(self.prefs.seek_step_large)
    );
    // Padded so the panel keeps clear of the player's edges; it scrolls when the player is too
    // small for it.
    div()
      .absolute()
      .inset_0()
      .p_3()
      .flex()
      .items_center()
      .justify_center()
      .child(
        v_flex()
          .id("help")
          .occlude()
          .w(px(520.))
          .max_w_full()
          .max_h_full()
          .overflow_y_scroll()
          .p_6()
          .gap_2()
          .rounded_lg()
          .bg(popover)
          .border_1()
          .border_color(border)
          .child(
            h_flex()
              .pb_2()
              .justify_between()
              .child(
                div()
                  .text_lg()
                  .font_weight(FontWeight::SEMIBOLD)
                  .child(t("Keyboard shortcuts", "Raccourcis clavier")),
              )
              .child(
                Button::new("help-close")
                  .ghost()
                  .xsmall()
                  .icon(IconName::Close)
                  .on_click(cx.listener(|this, _, _, cx| {
                    this.help = false;
                    cx.notify();
                  })),
              ),
          )
          .children(
            SHORTCUTS
              .iter()
              // Theater mode exists in the embedded layout only; a live without DVR does not seek.
              .filter(|(keys, ..)| embedded || *keys != "T")
              .filter(|(keys, ..)| can_seek || !SEEK_KEYS.contains(keys))
              .map(|&(keys_en, keys_fr, action_en, action_fr)| {
                let action: SharedString = if action_en.is_empty() {
                  seek.clone().into()
                } else {
                  t(action_en, action_fr).into()
                };
                h_flex()
                  .gap_4()
                  .child(
                    div()
                      .w(px(190.))
                      .text_sm()
                      .font_weight(FontWeight::MEDIUM)
                      .child(t(keys_en, keys_fr)),
                  )
                  .child(div().text_sm().text_color(muted).child(action))
              }),
          ),
      )
  }

  /// The right-click menu at `at` (relative to the player), kept inside the player.
  pub(super) fn render_context_menu(
    &self,
    at: Point<Pixels>,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    const WIDTH: f32 = 220.;
    const HEIGHT: f32 = 92.;
    let theme = cx.theme();
    let (popover, border) = (theme.popover, theme.border);
    let size = self.bounds.get().size;
    let left = f32::from(at.x)
      .min(f32::from(size.width) - WIDTH - 8.)
      .max(8.);
    let top = f32::from(at.y)
      .min(f32::from(size.height) - HEIGHT - 8.)
      .max(8.);
    let entry = |id: &'static str, label: &'static str, checked: bool| {
      Button::new(id)
        .ghost()
        .w_full()
        .justify_start()
        .label(label)
        .when(checked, |b| b.icon(IconName::Check))
    };
    v_flex()
      .id("context-menu")
      .occlude()
      .absolute()
      .left(px(left))
      .top(px(top))
      .w(px(WIDTH))
      .p_1()
      .gap_0p5()
      .rounded_lg()
      .bg(popover)
      .border_1()
      .border_color(border)
      .shadow_lg()
      .child(
        entry(
          "context-help",
          t("Keyboard shortcuts", "Raccourcis clavier"),
          self.help,
        )
        .on_click(cx.listener(|this, _, _, cx| {
          this.context_menu = None;
          this.help = !this.help;
          this.show_controls(cx);
        })),
      )
      .child(
        entry("context-stats", t("Statistics", "Statistiques"), self.stats).on_click(cx.listener(
          |this, _, _, cx| {
            this.context_menu = None;
            this.toggle_stats(cx);
          },
        )),
      )
      .into_any_element()
  }

  /// Why the video cannot play, with a way back when the player is the page.
  pub(super) fn render_error(&self, error: SharedString, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme();
    let (popover, border, muted) = (theme.popover, theme.border, theme.muted_foreground);
    let page_layout = self.layout.is_page();
    div()
      .absolute()
      .inset_0()
      .flex()
      .items_center()
      .justify_center()
      .child(
        v_flex()
          .id("error")
          .occlude()
          .max_w(px(560.))
          .p_6()
          .gap_4()
          .rounded_lg()
          .bg(popover)
          .border_1()
          .border_color(border)
          .child(
            div()
              .text_lg()
              .font_weight(FontWeight::SEMIBOLD)
              .child(t("Unable to play", "Lecture impossible")),
          )
          .child(div().text_sm().text_color(muted).child(error))
          .when(page_layout, |el| {
            el.child(
              Button::new("error-back")
                .primary()
                .label(t("Back", "Retour"))
                .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
          }),
      )
      .into_any_element()
  }
}
