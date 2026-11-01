//! Watch page: the player (autoplay), title and channel, related videos.
//!
//! The stream (VISIONOS HLS, ~0.2 s) and the page metadata (`next`) load in parallel; mpv
//! starts as soon as the stream is known, so playback begins about 1 s after the click.

use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
  Context, Entity, FocusHandle, FontWeight, KeyDownEvent, SharedString, WeakEntity, Window, div,
  px, relative,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{h_flex, v_flex};
use gpui_video::VideoView;
use innertube::{Client, Stream, WatchPage};
use player::{Event, Player, StartOptions};

use crate::app::AppView;
use crate::images::Images;
use crate::ui::{compact_card, error_text, image_fill};
use crate::{runtime, theme};

/// Highest video quality requested.
const MAX_HEIGHT: u32 = 1080;
/// Width of the related videos column.
const RELATED_WIDTH: f32 = 400.;

#[derive(Default)]
struct Playback {
  position: f64,
  duration: f64,
  paused: bool,
  buffering: bool,
}

pub struct WatchView {
  nav: WeakEntity<AppView>,
  focus: FocusHandle,
  page: Option<WatchPage>,
  player: Option<Rc<Player>>,
  video: Option<Entity<VideoView>>,
  playback: Playback,
  /// Why the video cannot play, shown over the player area.
  error: Option<SharedString>,
  /// Another page is on top: the player stays paused, even if it starts after being covered.
  covered: bool,
}

impl WatchView {
  pub fn new(
    client: Client,
    video_id: String,
    nav: WeakEntity<AppView>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let focus = cx.focus_handle();
    window.focus(&focus, cx);
    let view = Self {
      nav,
      focus,
      page: None,
      player: None,
      video: None,
      playback: Playback::default(),
      error: None,
      covered: false,
    };
    view.load_page(client.clone(), video_id.clone(), cx);
    view.load_stream(client, video_id, cx);
    view
  }

  fn load_page(&self, client: Client, video_id: String, cx: &mut Context<Self>) {
    let fetch = runtime::spawn(cx, async move { client.watch(&video_id).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(page)) => this.page = Some(page),
            Some(Err(e)) => tracing::warn!("watch page: {e}"),
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn load_stream(&self, client: Client, video_id: String, cx: &mut Context<Self>) {
    let started = Instant::now();
    let fetch = runtime::spawn(
      cx,
      async move { client.stream(&video_id, MAX_HEIGHT).await },
    );
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      match result {
        Some(Ok(stream)) => {
          tracing::info!(
            "stream after {:.2} s: {} {}",
            started.elapsed().as_secs_f64(),
            stream.resolution,
            stream.codecs
          );
          this
            .update(cx, |this, cx| this.start_player(stream, started, cx))
            .ok();
        }
        Some(Err(e)) => {
          tracing::warn!("stream: {e}");
          this
            .update(cx, |this, cx| {
              this.error = Some(error_text(&e).into());
              cx.notify();
            })
            .ok();
        }
        None => {}
      }
    })
    .detach();
  }

  fn start_player(&mut self, stream: Stream, started: Instant, cx: &mut Context<Self>) {
    let options = StartOptions {
      url: stream.video_url,
      external_audio: stream.audio_url.into_iter().collect(),
      http_headers: vec![format!("User-Agent: {}", stream.user_agent)],
      // Opt-in only: the log contains stream URLs, which carry the user's IP.
      log_file: std::env::var_os("PEEBTUBE_MPV_LOG").map(Into::into),
      ..Default::default()
    };
    let (event_tx, event_rx) = async_channel::unbounded();
    let (frame_tx, frame_rx) = async_channel::unbounded();
    // libmpv and D3D11 setup block for a moment: off the UI thread.
    let start = cx
      .background_executor()
      .spawn(async move { Player::start(options, event_tx, frame_tx) });
    cx.spawn(async move |this, cx| {
      let player = match start.await {
        Ok(player) => Rc::new(player),
        Err(e) => {
          this
            .update(cx, |this, cx| {
              this.error = Some(format!("Lecteur indisponible : {e}").into());
              cx.notify();
            })
            .ok();
          return;
        }
      };
      let attached = this.update(cx, |this, cx| {
        let resize = player.clone();
        this.video = Some(cx.new(|cx| {
          VideoView::new(
            frame_rx,
            Rc::new(move |w, h| resize.set_target_size(w, h)),
            cx,
          )
        }));
        if this.covered {
          tracing::info!("player started on a covered page: paused");
          player.set_paused(true);
        }
        this.player = Some(player);
        cx.notify();
      });
      if attached.is_err() {
        return;
      }
      let mut first_frame = true;
      while let Ok(event) = event_rx.recv().await {
        let alive = this.update(cx, |this, cx| {
          if first_frame && matches!(event, Event::Position(p) if p > 0.0) {
            first_frame = false;
            tracing::info!("playing after {:.2} s", started.elapsed().as_secs_f64());
          }
          this.apply(event);
          cx.notify();
        });
        if alive.is_err() {
          break;
        }
      }
    })
    .detach();
  }

  /// Called when another page covers this one (pauses) or when it is shown again. Playback
  /// does not resume by itself on "back": the user resumes it.
  pub fn set_covered(&mut self, covered: bool) {
    self.covered = covered;
    if covered && let Some(player) = &self.player {
      tracing::info!("watch page covered: player paused");
      player.set_paused(true);
    }
  }

  fn apply(&mut self, event: Event) {
    let p = &mut self.playback;
    match event {
      Event::Position(position) => p.position = position,
      Event::Duration(duration) => p.duration = duration,
      Event::Paused(paused) => p.paused = paused,
      Event::Buffering(buffering) => p.buffering = buffering,
      Event::Ended(Some(e)) => self.error = Some(format!("Lecture interrompue : {e}").into()),
      Event::Loaded
      | Event::Tracks(_)
      | Event::Chapters(_)
      | Event::Muted(_)
      | Event::EndReached
      | Event::Ended(None) => {}
    }
  }

  fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, _cx: &mut Context<Self>) {
    let Some(player) = &self.player else { return };
    match event.keystroke.key.as_str() {
      "space" | "k" => player.toggle_pause(),
      "right" | "l" => player.seek_by(if event.keystroke.key == "l" { 10. } else { 5. }),
      "left" | "j" => player.seek_by(if event.keystroke.key == "j" {
        -10.
      } else {
        -5.
      }),
      "f" => window.toggle_fullscreen(),
      _ => {}
    }
  }

  fn player_area(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let status = match (&self.error, &self.video) {
      (Some(e), _) => Some(e.clone()),
      (None, None) => Some("Chargement…".into()),
      (None, Some(_)) if self.playback.buffering => Some("Mise en mémoire tampon…".into()),
      _ => None,
    };
    let progress = if self.playback.duration > 0. {
      (self.playback.position / self.playback.duration).clamp(0., 1.) as f32
    } else {
      0.
    };
    let player = self.player.clone();
    div()
      .id("player")
      .relative()
      .w_full()
      // 16:9 box: GPUI has no aspect-ratio style, so the height follows the width through
      // padding-top as a share of the width.
      .pt(relative(9. / 16.))
      .rounded(theme::RADIUS_LG)
      .overflow_hidden()
      .bg(gpui::black())
      .on_click(cx.listener(move |_, _, _, _| {
        if let Some(player) = &player {
          player.toggle_pause();
        }
      }))
      .child(div().absolute().inset_0().children(self.video.clone()))
      .children(status.map(|text| {
        div()
          .absolute()
          .inset_0()
          .flex()
          .items_center()
          .justify_center()
          .text_color(theme::muted())
          .child(text)
      }))
      .child(
        div()
          .absolute()
          .left_0()
          .right_0()
          .bottom_0()
          .h(px(3.))
          .bg(gpui::rgba(0xffffff33))
          .child(div().h_full().w(relative(progress)).bg(theme::accent())),
      )
  }

  fn details(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let Some(page) = &self.page else {
      return v_flex();
    };
    let avatar = (!page.channel_avatar.is_empty()).then(|| {
      div().size(px(40.)).flex_none().child(image_fill(
        &page.channel_avatar,
        40.,
        px(20.),
        window,
        cx,
      ))
    });
    let stats = [page.views.clone(), page.date.clone()]
      .into_iter()
      .flatten()
      .collect::<Vec<_>>()
      .join(" · ");
    v_flex()
      .gap_3()
      .child(
        div()
          .text_xl()
          .font_weight(FontWeight::SEMIBOLD)
          .child(page.title.clone()),
      )
      .child(
        h_flex().gap_3().items_center().children(avatar).child(
          v_flex()
            .child(
              div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(page.channel.clone().unwrap_or_default()),
            )
            .child(
              div()
                .text_sm()
                .text_color(theme::muted())
                .child(page.subscribers.clone().unwrap_or_default()),
            ),
        ),
      )
      .child(
        v_flex()
          .p_3()
          .gap_1()
          .rounded(theme::RADIUS_LG)
          .bg(theme::surface())
          .text_sm()
          .child(div().font_weight(FontWeight::SEMIBOLD).child(stats))
          // `line_clamp` limits wrapped lines, not explicit line breaks: cut those first.
          .child(
            div().line_clamp(4).child(
              page
                .description
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n"),
            ),
          ),
      )
  }
}

impl Render for WatchView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let related: Vec<_> = self
      .page
      .iter()
      .flat_map(|page| page.related.iter())
      .map(|v| compact_card(v, &self.nav, window, cx))
      .collect();
    let player = self.player_area(cx);
    let details = self.details(window, cx);
    h_flex()
      .id("watch")
      .size_full()
      .items_start()
      .track_focus(&self.focus)
      .on_key_down(cx.listener(Self::on_key))
      .overflow_y_scrollbar()
      .child(
        h_flex()
          .w_full()
          .p_6()
          .gap_6()
          .items_start()
          .child(
            v_flex()
              .flex_1()
              .min_w_0()
              .gap_4()
              .child(player)
              .child(details),
          )
          .child(
            v_flex()
              .w(px(RELATED_WIDTH))
              .flex_none()
              .gap_3()
              .children(related),
          ),
      )
  }
}
