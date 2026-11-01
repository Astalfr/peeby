//! What the player draws over the picture besides its control bar: the menus (quality, audio,
//! subtitles, chapters), the top bar of the full-window layout with its window buttons, the
//! skip button, the "up next" card, the shortcut help and the error panel. From peeby's player
//! page.

use gpui::prelude::*;
use gpui::{
  AnyElement, Context, FontWeight, SharedString, Window, WindowControlArea, div, linear_color_stop,
  linear_gradient, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{ActiveTheme, IconName, Sizable, h_flex, v_flex};
use player::TrackKind;

use super::tracks::{SUBTITLE_DELAY_STEP, UpNext};
use super::{CONTROL_ICON, CONTROL_SIZE, Layout, Menu, PlayerView, format_time};
use crate::assets::Icon;

/// The help's rows; the seek steps come from the preferences (see `render_help`).
const SHORTCUTS: &[(&str, &str)] = &[
  ("Espace / K", "Lecture ou pause"),
  ("← / → · J / L", ""),
  ("Page préc. / Page suiv.", "Chapitre précédent ou suivant"),
  ("0 – 9", "Aller à 0 % – 90 %"),
  ("Début", "Retour au début"),
  ("↑ / ↓", "Volume"),
  ("M", "Couper le son"),
  ("A", "Piste audio suivante"),
  ("C", "Sous-titres suivants (ou désactivés)"),
  ("Z / X", "Décalage des sous-titres −/+ 0,1 s"),
  ("S", "Passer l'intro, le récap ou le générique"),
  ("P / N", "Précédent ou suivant"),
  ("T", "Mode cinéma"),
  ("F / double-clic", "Plein écran"),
  (
    "Échap",
    "Fermer le menu, quitter le plein écran, puis revenir",
  ),
  ("?", "Cette aide"),
];

/// `10 s`, `1 min`, `1 min 30 s`.
pub fn format_step(seconds: u32) -> String {
  match (seconds / 60, seconds % 60) {
    (0, s) => format!("{s} s"),
    (m, 0) => format!("{m} min"),
    (m, s) => format!("{m} min {s} s"),
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
            "Désactivés".into(),
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
          entry("sub-off".into(), "Désactivés".into(), current.is_none())
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
      .child(format!(
        "Décalage {:+.1} s (Z / X, pas de {SUBTITLE_DELAY_STEP} s)",
        self.subtitle_delay
      ))
  }

  /// Top bar of the full-window layout: back, title and subtitle, and the window buttons
  /// (it doubles as the window's title bar, a drag area).
  pub(super) fn render_top_bar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
    let fullscreen = window.is_fullscreen();
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
              .when(!fullscreen, |el| {
                el.window_control_area(WindowControlArea::Drag)
              }),
          )
          // A fullscreen window has nothing to drag, minimize or restore: Escape or F leaves it.
          .when(!fullscreen, |row| {
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
              .tooltip("Retour (Échap)")
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
      Some(n) => format!("Lecture dans {n} s"),
      None => "À suivre".to_owned(),
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
                .label("Lire maintenant")
                .on_click(cx.listener(|this, _, _, cx| this.play_next(cx))),
            )
            .child(
              Button::new("up-next-cancel")
                .ghost()
                .label("Annuler")
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
    let seek = format!(
      "Reculer ou avancer de {} (Maj : {})",
      format_step(self.prefs.seek_step),
      format_step(self.prefs.seek_step_large)
    );
    div()
      .absolute()
      .inset_0()
      .flex()
      .items_center()
      .justify_center()
      .child(
        v_flex()
          .id("help")
          .occlude()
          .p_6()
          .gap_2()
          .min_w(px(480.))
          .rounded_lg()
          .bg(popover)
          .border_1()
          .border_color(border)
          .child(
            div()
              .text_lg()
              .font_weight(FontWeight::SEMIBOLD)
              .pb_2()
              .child("Raccourcis clavier"),
          )
          .children(
            SHORTCUTS
              .iter()
              // Theater mode exists in the embedded layout only.
              .filter(|(keys, _)| embedded || *keys != "T")
              .map(|(keys, action)| {
                let action: SharedString = if action.is_empty() {
                  seek.clone().into()
                } else {
                  (*action).into()
                };
                h_flex()
                  .gap_4()
                  .child(
                    div()
                      .w(px(190.))
                      .text_sm()
                      .font_weight(FontWeight::MEDIUM)
                      .child(*keys),
                  )
                  .child(div().text_sm().text_color(muted).child(action))
              }),
          ),
      )
  }

  /// Why the video cannot play, with a way back in the full-window layout.
  pub(super) fn render_error(&self, error: SharedString, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme();
    let (popover, border, muted) = (theme.popover, theme.border, theme.muted_foreground);
    let window_layout = self.layout == Layout::Window;
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
              .child("Lecture impossible"),
          )
          .child(div().text_sm().text_color(muted).child(error))
          .when(window_layout, |el| {
            el.child(
              Button::new("error-back")
                .primary()
                .label("Retour")
                .on_click(cx.listener(|this, _, _, cx| this.close(cx))),
            )
          }),
      )
      .into_any_element()
  }
}
