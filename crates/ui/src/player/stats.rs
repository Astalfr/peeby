//! The statistics panel (right click > Statistiques): what mpv decodes and how, read once a
//! second while the panel is open, for debugging playback. Only the properties listed here are
//! read: never `path` or `stream-open-filename`, whose stream URLs carry the user's IP.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, SharedString, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{ActiveTheme, IconName, Sizable, h_flex, v_flex};
use player::{Backend, Player};

use super::{PlayerView, format_time};

/// How often the open panel reads mpv again.
const REFRESH: Duration = Duration::from_secs(1);

/// One line of the panel.
pub(super) type Row = (&'static str, String);

impl PlayerView {
  /// Opens or closes the panel; while open it is refreshed every second.
  pub(super) fn toggle_stats(&mut self, cx: &mut Context<Self>) {
    self.stats = !self.stats;
    if !self.stats {
      self.stats_task = None;
      return cx.notify();
    }
    self.read_stats();
    self.stats_task = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(REFRESH).await;
        let open = this.update(cx, |this, cx| {
          this.read_stats();
          cx.notify();
          this.stats
        });
        if !open.unwrap_or(false) {
          break;
        }
      }
    }));
    cx.notify();
  }

  /// Reads the panel's lines from mpv (on the UI thread: a handful of quick property reads).
  fn read_stats(&mut self) {
    let quality = self
      .quality
      .and_then(|i| self.source.as_ref()?.qualities.get(i))
      .map(|q| q.label.to_string());
    self.stats_rows = match &self.player {
      Some(player) => rows(player, quality),
      None => vec![("Lecteur", "pas démarré".to_owned())],
    };
    self.stats_rows.push((
      "Position",
      format!(
        "{} / {}",
        format_time(self.position),
        format_time(self.duration)
      ),
    ));
  }

  /// The panel, at the player's top left, scrolling when the player is shorter than it.
  pub(super) fn render_stats(&self, cx: &mut Context<Self>) -> AnyElement {
    let theme = cx.theme();
    let (popover, border, muted) = (theme.popover, theme.border, theme.muted_foreground);
    div()
      .absolute()
      .inset_0()
      .p_3()
      // The empty space around the panel lets the clicks through to the picture.
      .child(
        v_flex()
          .id("stats")
          .occlude()
          .w(px(400.))
          .max_w_full()
          .max_h_full()
          .overflow_y_scroll()
          .p_4()
          .gap_1()
          .rounded_lg()
          .bg(popover)
          .border_1()
          .border_color(border)
          .child(
            h_flex()
              .pb_1()
              .justify_between()
              .child(
                div()
                  .font_weight(FontWeight::SEMIBOLD)
                  .child("Statistiques"),
              )
              .child(
                Button::new("stats-close")
                  .ghost()
                  .xsmall()
                  .icon(IconName::Close)
                  .on_click(cx.listener(|this, _, _, cx| this.toggle_stats(cx))),
              ),
          )
          .children(self.stats_rows.iter().map(|(label, value)| {
            h_flex()
              .gap_3()
              .items_start()
              .text_xs()
              .child(
                div()
                  .w(px(120.))
                  .flex_none()
                  .text_color(muted)
                  .child(*label),
              )
              .child(
                div()
                  .flex_1()
                  .min_w_0()
                  .child(SharedString::from(value.clone())),
              )
          })),
      )
      .into_any_element()
  }
}

/// The panel's lines for a running player.
fn rows(player: &Player, quality: Option<String>) -> Vec<Row> {
  let get = |name: &str| player.property(name).filter(|v| !v.is_empty());
  let number = |name: &str| get(name).and_then(|v| v.parse::<f64>().ok());
  let joined = |parts: Vec<Option<String>>| {
    let parts: Vec<String> = parts.into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
  };
  let mut out: Vec<Row> = Vec::new();
  let mut push = |label: &'static str, value: Option<String>| {
    if let Some(value) = value {
      out.push((label, value));
    }
  };
  push(
    "Rendu",
    Some(
      match player.backend() {
        Backend::Gpu => "GPU (D3D11, sans copie)",
        Backend::Software => "logiciel (copie CPU)",
      }
      .to_owned(),
    ),
  );
  push("Qualité", quality);
  push(
    "Vidéo",
    joined(vec![
      get("video-codec"),
      match (get("video-params/w"), get("video-params/h")) {
        (Some(w), Some(h)) => Some(format!("{w}×{h}")),
        _ => None,
      },
      get("video-params/pixelformat"),
    ]),
  );
  push(
    "Couleurs",
    joined(vec![
      get("video-params/colormatrix"),
      get("video-params/primaries"),
      get("video-params/gamma"),
    ]),
  );
  push(
    "Décodage",
    Some(match get("hwdec-current") {
      Some(hwdec) if hwdec != "no" => format!("matériel ({hwdec})"),
      _ => "logiciel".to_owned(),
    }),
  );
  push(
    "Images/s",
    joined(vec![
      number("container-fps").map(|f| format!("fichier {f:.3}")),
      number("estimated-vf-fps").map(|f| format!("affichées {f:.2}")),
      number("display-fps").map(|f| format!("écran {f:.0} Hz")),
    ]),
  );
  push(
    "Images perdues",
    joined(vec![
      get("frame-drop-count").map(|n| format!("{n} à l'affichage")),
      get("decoder-frame-drop-count").map(|n| format!("{n} au décodage")),
    ]),
  );
  push(
    "Synchro A/V",
    number("avsync").map(|s| format!("{:+.1} ms", s * 1000.0)),
  );
  push(
    "Audio",
    joined(vec![
      get("audio-codec-name"),
      number("audio-params/samplerate").map(|r| format!("{:.1} kHz", r / 1000.0)),
      get("audio-params/hr-channels").or_else(|| get("audio-params/channels")),
    ]),
  );
  push(
    "Sortie audio",
    joined(vec![get("current-ao"), get("audio-device")]),
  );
  push(
    "Débit",
    joined(vec![
      number("video-bitrate").map(|b| format!("vidéo {:.0} kb/s", b / 1000.0)),
      number("audio-bitrate").map(|b| format!("audio {:.0} kb/s", b / 1000.0)),
    ]),
  );
  push(
    "Tampon",
    joined(vec![
      number("demuxer-cache-duration").map(|s| format!("{s:.1} s d'avance")),
      number("cache-speed").map(|b| format!("{:.1} Mo/s", b / 1_000_000.0)),
    ]),
  );
  push("Conteneur", get("file-format"));
  out
}
