//! The statistics panel (right click > Statistics): what mpv decodes and how, read once a
//! second while the panel is open, for debugging playback. Only the properties listed here are
//! read: never `path` or `stream-open-filename`, whose stream URLs carry the user's IP.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, Context, FontWeight, SharedString, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{ActiveTheme, IconName, Sizable, h_flex, v_flex};
use player::{Backend, Player};

use super::{PlayerView, format_time};
use crate::i18n::t;
use crate::tf;

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
      None => vec![(
        t("Player", "Lecteur"),
        t("not started", "pas démarré").to_owned(),
      )],
    };
    self.stats_rows.push((
      t("Position", "Position"),
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
                  .child(t("Statistics", "Statistiques")),
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
    t("Rendering", "Rendu"),
    Some(
      match player.backend() {
        Backend::Gpu => t("GPU (D3D11, zero-copy)", "GPU (D3D11, sans copie)"),
        Backend::Software => t("software (CPU copy)", "logiciel (copie CPU)"),
      }
      .to_owned(),
    ),
  );
  push(t("Quality", "Qualité"), quality);
  push(
    t("Video", "Vidéo"),
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
    t("Colors", "Couleurs"),
    joined(vec![
      get("video-params/colormatrix"),
      get("video-params/primaries"),
      get("video-params/gamma"),
    ]),
  );
  push(
    t("Decoding", "Décodage"),
    Some(match get("hwdec-current") {
      Some(hwdec) if hwdec != "no" => tf!("hardware ({hwdec})", "matériel ({hwdec})"),
      _ => t("software", "logiciel").to_owned(),
    }),
  );
  push(
    t("Frame rate", "Images/s"),
    joined(vec![
      number("container-fps").map(|f| tf!("file {f:.3}", "fichier {f:.3}")),
      number("estimated-vf-fps").map(|f| tf!("shown {f:.2}", "affichées {f:.2}")),
      number("display-fps").map(|f| tf!("display {f:.0} Hz", "écran {f:.0} Hz")),
    ]),
  );
  push(
    t("Dropped frames", "Images perdues"),
    joined(vec![
      get("frame-drop-count").map(|n| tf!("{n} on display", "{n} à l'affichage")),
      get("decoder-frame-drop-count").map(|n| tf!("{n} on decoding", "{n} au décodage")),
    ]),
  );
  push(
    t("A/V sync", "Synchro A/V"),
    number("avsync").map(|s| format!("{:+.1} ms", s * 1000.0)),
  );
  push(
    t("Audio", "Audio"),
    joined(vec![
      get("audio-codec-name"),
      number("audio-params/samplerate").map(|r| format!("{:.1} kHz", r / 1000.0)),
      get("audio-params/hr-channels").or_else(|| get("audio-params/channels")),
    ]),
  );
  push(
    t("Audio output", "Sortie audio"),
    joined(vec![get("current-ao"), get("audio-device")]),
  );
  push(
    t("Bitrate", "Débit"),
    joined(vec![
      number("video-bitrate").map(|b| tf!("video {:.0} kb/s", "vidéo {:.0} kb/s", b / 1000.0)),
      number("audio-bitrate").map(|b| tf!("audio {:.0} kb/s", "audio {:.0} kb/s", b / 1000.0)),
    ]),
  );
  push(
    t("Buffer", "Tampon"),
    joined(vec![
      number("demuxer-cache-duration").map(|s| tf!("{s:.1} s ahead", "{s:.1} s d'avance")),
      number("cache-speed").map(|b| tf!("{:.1} MB/s", "{:.1} Mo/s", b / 1_000_000.0)),
    ]),
  );
  push(t("Container", "Conteneur"), get("file-format"));
  out
}
