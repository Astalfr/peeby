//! The video player: picture, auto-hiding controls (seek bar, volume, buttons), OSD and
//! keyboard shortcuts. Adapted from peeby's `player_page.rs` (controls, seek bar, volume
//! slider, control buttons), without its Jellyfin parts (reporting, episodes, segments,
//! tracks). Embedded in the watch page; fills the window in fullscreen.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, App, Bounds, ClickEvent, Context, DispatchPhase, Entity, EventEmitter,
  FocusHandle, FontWeight, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
  Pixels, ScrollEvent, SharedString, Task, WeakEntity, Window, canvas, div, ease_out_quint,
  linear_color_stop, linear_gradient, px, relative,
};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_video::VideoView;
use innertube::Stream;
use innertube::hls::Variant;
use player::{Choice, Event, Player, StartOptions};

use crate::assets::Icon;
use crate::settings::Settings;

const HIDE_CONTROLS_AFTER: Duration = Duration::from_secs(3);
const OSD_DURATION: Duration = Duration::from_millis(1500);
const PAUSE_PULSE: Duration = Duration::from_millis(600);
const VOLUME_FADE: Duration = Duration::from_millis(150);
const VOLUME_STEP: f64 = 5.0;
/// Arrow keys seek 5 s, J and L 10 s, as on YouTube.
const SEEK_SMALL: f64 = 5.0;
const SEEK_LARGE: f64 = 10.0;
const CONTROL_SIZE: f32 = 40.;
const CONTROL_ICON: f32 = 20.;

/// What the player tells the watch page.
pub enum PlayerEvent {
  /// The video played to its end.
  Ended,
  /// The user asked for the next or the previous video of the queue.
  Next,
  Previous,
  /// Theater mode was switched on or off (`Settings::theater`): the page lays itself out again.
  Theater,
}

/// The open menu of the control bar.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
  Quality,
  Subtitles,
}

pub struct PlayerView {
  focus: FocusHandle,
  title: SharedString,
  player: Option<Rc<Player>>,
  video: Option<Entity<VideoView>>,
  /// Shown instead of the picture: loading, or why the video cannot play.
  status: Option<SharedString>,
  failed: bool,
  position: f64,
  duration: f64,
  paused: bool,
  started: bool,
  /// The last play/pause change, shown as a short pulse in the middle of the picture; the
  /// counter gives each pulse its own animation id, so a new change restarts it.
  pause_pulse: Option<(u32, bool)>,
  buffering: bool,
  volume: f64,
  muted: bool,
  /// Another page is on top: the player stays paused, even if it starts after being covered.
  covered: bool,
  osd: Option<SharedString>,
  osd_task: Option<Task<()>>,
  controls_visible: bool,
  over_controls: bool,
  /// The mouse is over the mute button, or over the volume slider above it.
  volume_hover: (bool, bool),
  /// The left button went down on the volume slider and is still held: the drag goes on
  /// wherever the mouse is, and the slider stays open.
  volume_dragging: bool,
  /// The slider is open (or opening); it is drawn while open or while its closing animation runs.
  volume_open: bool,
  /// Counts the slider's open/close changes, so each one starts a new animation.
  volume_animation: u32,
  /// Removes the slider once its closing animation is over.
  volume_close_task: Option<Task<()>>,
  volume_bounds: Rc<Cell<Bounds<Pixels>>>,
  /// The player button under the mouse: its icon turns to the accent color.
  hovered_control: Option<&'static str>,
  hide_task: Option<Task<()>>,
  seek_bounds: Rc<Cell<Bounds<Pixels>>>,
  /// Mouse position over the seek bar, as a fraction of the duration.
  seek_hover: Option<f64>,
  /// The stream being played (VISIONOS): its qualities and subtitle tracks.
  stream: Option<Stream>,
  /// Index of the playing quality in `stream.qualities`.
  quality: Option<usize>,
  /// Selected subtitle track, index in `stream.captions`.
  caption: Option<usize>,
  /// Caption URLs already given to mpv, in mpv's track order (external subtitles get the ids
  /// 1, 2… since the HLS streams carry none).
  captions_added: Vec<String>,
  menu: Option<Menu>,
  /// Playing a queue: whether it has a previous and a next video (buttons, P and N keys).
  queue: Option<(bool, bool)>,
  _events: Option<Task<()>>,
}

impl EventEmitter<PlayerEvent> for PlayerView {}

impl PlayerView {
  pub fn new(cx: &mut Context<Self>) -> Self {
    let sound = Settings::get(cx).clone();
    Self {
      focus: cx.focus_handle(),
      title: SharedString::default(),
      player: None,
      video: None,
      status: Some("Chargement…".into()),
      failed: false,
      position: 0.0,
      duration: 0.0,
      paused: false,
      started: false,
      pause_pulse: None,
      buffering: false,
      volume: sound.volume,
      muted: sound.muted,
      covered: false,
      osd: None,
      osd_task: None,
      controls_visible: true,
      over_controls: false,
      volume_hover: (false, false),
      volume_dragging: false,
      volume_open: false,
      volume_animation: 0,
      volume_close_task: None,
      volume_bounds: Rc::default(),
      hovered_control: None,
      hide_task: None,
      seek_bounds: Rc::default(),
      seek_hover: None,
      stream: None,
      quality: None,
      caption: None,
      captions_added: Vec::new(),
      menu: None,
      queue: None,
      _events: None,
    }
  }

  pub fn focus_handle(&self) -> &FocusHandle {
    &self.focus
  }

  /// Title shown over the picture in fullscreen.
  pub fn set_title(&mut self, title: SharedString, cx: &mut Context<Self>) {
    self.title = title;
    cx.notify();
  }

  /// Shows the previous and next buttons of a queue, enabled when it has such videos.
  pub fn set_queue(&mut self, queue: Option<(bool, bool)>, cx: &mut Context<Self>) {
    self.queue = queue;
    cx.notify();
  }

  /// Shows why the video cannot play.
  pub fn fail(&mut self, message: SharedString, cx: &mut Context<Self>) {
    self.status = Some(message);
    self.failed = true;
    self.buffering = false;
    self.controls_visible = true;
    cx.notify();
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

  /// Starts mpv on `stream`; `clicked` is when the user opened the video, for the timing log.
  pub fn start(&mut self, stream: Stream, clicked: Instant, cx: &mut Context<Self>) {
    self.quality = stream
      .qualities
      .iter()
      .position(|q| q.url == stream.video_url);
    tracing::info!(
      "{} qualities ({}), {} subtitle tracks",
      stream.qualities.len(),
      stream
        .qualities
        .iter()
        .map(Variant::label)
        .collect::<Vec<_>>()
        .join(" "),
      stream.captions.len()
    );
    let options = stream_options(
      &stream.video_url,
      stream.audio_url.as_deref(),
      stream.user_agent,
      0.0,
    );
    self.stream = Some(stream);
    self.start_with(options, clicked, cx);
  }

  /// Plays another quality from the current position: mpv restarts on the new variant
  /// (about 0.7 s), and the subtitle track is added again.
  fn select_quality(&mut self, index: usize, cx: &mut Context<Self>) {
    self.menu = None;
    if self.quality == Some(index) {
      return cx.notify();
    }
    let Some(stream) = &self.stream else { return };
    let Some(variant) = stream.qualities.get(index).cloned() else {
      return;
    };
    let options = stream_options(
      &variant.url,
      variant.audio_url.as_deref(),
      stream.user_agent,
      self.position,
    );
    tracing::info!("quality {}", variant.label());
    self.quality = Some(index);
    self.show_osd(format!("Qualité {}", variant.label()).into(), cx);
    // The old player lives on in the old VideoView until the new one replaces it: pause it so
    // the sounds do not overlap. Its last frame stays on screen meanwhile.
    if let Some(old) = self.player.take() {
      old.set_paused(true);
    }
    self.captions_added.clear();
    self.start_with(options, Instant::now(), cx);
  }

  /// Shows subtitle track `index` of the stream, or none. Tracks are fetched by mpv the first
  /// time they are picked.
  fn select_caption(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
    self.menu = None;
    self.caption = index;
    let label = match index.and_then(|i| self.stream.as_ref()?.captions.get(i)) {
      Some(caption) => caption_label(caption),
      None => "Désactivés".to_owned(),
    };
    self.apply_caption();
    self.show_osd(format!("Sous-titres : {label}").into(), cx);
  }

  fn apply_caption(&mut self) {
    let Some(player) = self.player.clone() else {
      return;
    };
    let caption = self
      .caption
      .and_then(|i| self.stream.as_ref()?.captions.get(i).cloned());
    match caption {
      None => player.set_subtitle(Choice::Off),
      Some(caption) => match self.captions_added.iter().position(|u| *u == caption.url) {
        Some(added) => player.set_subtitle(Choice::Track(added as i64 + 1)),
        None => {
          player.add_subtitle(&caption.url, &caption_label(&caption), &caption.language);
          self.captions_added.push(caption.url);
        }
      },
    }
  }

  /// C: off → the first track in the app's language (else the first one) → off.
  fn toggle_captions(&mut self, cx: &mut Context<Self>) {
    if self.caption.is_some() {
      return self.select_caption(None, cx);
    }
    let Some(stream) = &self.stream else { return };
    if stream.captions.is_empty() {
      return self.show_osd("Aucun sous-titre".into(), cx);
    }
    let hl = Settings::get(cx).language.clone();
    let hl = hl.as_str();
    let index = stream
      .captions
      .iter()
      .position(|c| c.language == hl || c.language.starts_with(&format!("{hl}-")))
      .unwrap_or(0);
    self.select_caption(Some(index), cx);
  }

  fn toggle_menu(&mut self, menu: Menu, cx: &mut Context<Self>) {
    self.menu = if self.menu == Some(menu) {
      None
    } else {
      Some(menu)
    };
    self.show_controls(cx);
  }

  /// Starts mpv with prepared options (e.g. the yt-dlp fallback).
  pub fn start_with(&mut self, options: StartOptions, clicked: Instant, cx: &mut Context<Self>) {
    let (event_tx, event_rx) = async_channel::unbounded();
    let (frame_tx, frame_rx) = async_channel::unbounded();
    // libmpv and D3D11 setup block for a moment: off the UI thread.
    let start = cx
      .background_executor()
      .spawn(async move { Player::start(options, event_tx, frame_tx) });
    self._events = Some(cx.spawn(async move |this, cx| {
      let player = match start.await {
        Ok(player) => Rc::new(player),
        Err(e) => {
          this
            .update(cx, |this, cx| {
              this.fail(format!("Lecteur indisponible : {e}").into(), cx)
            })
            .ok();
          return;
        }
      };
      let attached = this.update(cx, |this, cx| this.attach(player, frame_rx, cx));
      if attached.is_err() {
        return;
      }
      let mut first_frame = true;
      while let Ok(event) = event_rx.recv().await {
        let alive = this.update(cx, |this, cx| {
          if first_frame && matches!(event, Event::Position(p) if p > 0.0) {
            first_frame = false;
            tracing::info!("playing after {:.2} s", clicked.elapsed().as_secs_f64());
          }
          this.on_event(event, cx);
        });
        if alive.is_err() {
          break;
        }
      }
    }));
  }

  fn attach(
    &mut self,
    player: Rc<Player>,
    frames: async_channel::Receiver<player::Frame>,
    cx: &mut Context<Self>,
  ) {
    let resize = player.clone();
    self.video = Some(cx.new(|cx| {
      VideoView::new(
        frames,
        Rc::new(move |w, h| resize.set_target_size(w, h)),
        cx,
      )
    }));
    player.set_volume(self.volume);
    if self.muted {
      player.set_muted(true);
    }
    if self.covered {
      tracing::info!("player started on a covered page: paused");
      player.set_paused(true);
    }
    self.player = Some(player);
    self.status = None;
    // After a quality change: the subtitle track goes to the new mpv.
    self.apply_caption();
    self.show_controls(cx);
  }

  fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
    match event {
      Event::Loaded => {
        self.buffering = false;
        self.started = true;
      }
      Event::Position(seconds) => self.position = seconds,
      Event::Duration(seconds) => self.duration = seconds,
      Event::Paused(paused) => {
        // Not for the state the file opens in.
        if self.started && paused != self.paused {
          let count = self
            .pause_pulse
            .map_or(0, |(count, _)| count.wrapping_add(1));
          self.pause_pulse = Some((count, paused));
        }
        self.paused = paused;
        self.show_controls(cx);
      }
      Event::Buffering(buffering) => self.buffering = buffering,
      Event::Muted(muted) => self.muted = muted,
      Event::EndReached => {
        self.position = self.position.max(self.duration);
        cx.emit(PlayerEvent::Ended);
      }
      Event::Ended(Some(error)) => self.fail(format!("Lecture interrompue : {error}").into(), cx),
      Event::Ended(None) | Event::Tracks(_) | Event::Chapters(_) => {}
    }
    cx.notify();
  }

  fn with_player(&self, f: impl FnOnce(&Player)) {
    if let Some(player) = &self.player {
      f(player);
    }
  }

  fn seek_to(&mut self, seconds: f64, cx: &mut Context<Self>) {
    let target = seconds.clamp(0.0, self.duration.max(0.0));
    self.position = target;
    self.with_player(|p| p.seek_to(target));
    cx.notify();
  }

  fn seek_by(&mut self, delta: f64, cx: &mut Context<Self>) {
    self.seek_to(self.position + delta, cx);
  }

  fn set_volume(&mut self, volume: f64, cx: &mut Context<Self>) {
    self.volume = volume.clamp(0.0, 100.0);
    self.with_player(|p| p.set_volume(self.volume));
    if self.muted {
      self.toggle_mute(cx);
    }
    self.remember_sound(cx);
    self.show_osd(format!("Volume {:.0} %", self.volume).into(), cx);
  }

  /// Volume from a mouse position on the vertical slider: bottom is 0 %, top 100 %.
  fn set_volume_at(&mut self, y: Pixels, cx: &mut Context<Self>) {
    let bounds = self.volume_bounds.get();
    let height = f32::from(bounds.size.height);
    if height > 0.0 {
      let fraction = (1.0 - f32::from(y - bounds.origin.y) / height).clamp(0.0, 1.0);
      self.set_volume((f64::from(fraction) * 100.0).round(), cx);
    }
  }

  fn end_volume_drag(&mut self, cx: &mut Context<Self>) {
    self.volume_dragging = false;
    // Restarts the hide timer, which skipped hiding the controls during the drag.
    self.show_controls(cx);
  }

  fn toggle_mute(&mut self, cx: &mut Context<Self>) {
    self.muted = !self.muted;
    self.with_player(|p| p.set_muted(self.muted));
    self.remember_sound(cx);
    self.show_osd(
      if self.muted {
        "Son coupé"
      } else {
        "Son activé"
      }
      .into(),
      cx,
    );
  }

  /// Saved in the settings, so the next video (and the next launch) plays at this level.
  fn remember_sound(&self, cx: &mut App) {
    let (volume, muted) = (self.volume, self.muted);
    Settings::update(cx, |s| {
      s.volume = volume;
      s.muted = muted;
    });
  }

  fn show_osd(&mut self, text: SharedString, cx: &mut Context<Self>) {
    self.osd = Some(text);
    cx.notify();
    self.osd_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(OSD_DURATION).await;
      this
        .update(cx, |this, cx| {
          this.osd = None;
          cx.notify();
        })
        .ok();
    }));
  }

  fn show_controls(&mut self, cx: &mut Context<Self>) {
    self.controls_visible = true;
    cx.notify();
    self.hide_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(HIDE_CONTROLS_AFTER).await;
      this
        .update(cx, |this, cx| {
          let busy = this.paused
            || this.failed
            || this.menu.is_some()
            || this.over_controls
            || this.volume_hover.1
            || this.volume_dragging;
          if !busy {
            this.controls_visible = false;
            // The buttons go away without a hover-out.
            this.hovered_control = None;
            gpui_video::hide_cursor_until_mouse_moves();
            cx.notify();
          }
        })
        .ok();
    }));
  }

  fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    let keystroke = &event.keystroke;
    match keystroke.key.as_str() {
      "space" | "k" => self.with_player(Player::toggle_pause),
      "left" => self.seek_by(-SEEK_SMALL, cx),
      "right" => self.seek_by(SEEK_SMALL, cx),
      "j" => self.seek_by(-SEEK_LARGE, cx),
      "l" => self.seek_by(SEEK_LARGE, cx),
      "home" => self.seek_to(0.0, cx),
      key @ ("0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
        if !keystroke.modifiers.shift =>
      {
        let tenth = key.parse::<f64>().unwrap_or(0.0);
        self.seek_to(self.duration * tenth / 10.0, cx);
      }
      "up" => self.set_volume(self.volume + VOLUME_STEP, cx),
      "down" => self.set_volume(self.volume - VOLUME_STEP, cx),
      "m" => self.toggle_mute(cx),
      "c" => self.toggle_captions(cx),
      "n" if self.queue.is_some_and(|(_, next)| next) => cx.emit(PlayerEvent::Next),
      "p" if self.queue.is_some_and(|(previous, _)| previous) => cx.emit(PlayerEvent::Previous),
      "escape" if self.menu.is_some() => self.menu = None,
      "t" if !window.is_fullscreen() => self.toggle_theater(cx),
      "f" => window.toggle_fullscreen(),
      "escape" if window.is_fullscreen() => window.toggle_fullscreen(),
      _ => return,
    }
    cx.stop_propagation();
    self.show_controls(cx);
  }

  /// Switches theater mode, kept in the settings so the next videos open the same way.
  fn toggle_theater(&mut self, cx: &mut Context<Self>) {
    Settings::update(cx, |s| s.theater = !s.theater);
    cx.emit(PlayerEvent::Theater);
    cx.notify();
  }

  /// Click on the picture: play/pause; double-click: fullscreen (undoing the first click's pause).
  fn on_video_click(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    window.focus(&self.focus, cx);
    if self.menu.take().is_some() {
      return cx.notify();
    }
    self.with_player(Player::toggle_pause);
    if event.click_count() == 2 {
      window.toggle_fullscreen();
    }
  }

  /// Mouse wheel: volume, in fullscreen only (in the page the wheel scrolls the page).
  fn on_wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
    if !window.is_fullscreen() {
      return;
    }
    let dy = f32::from(event.delta.pixel_delta(window.line_height()).y);
    if dy.abs() > 0.5 {
      let step = if dy > 0.0 { VOLUME_STEP } else { -VOLUME_STEP };
      self.set_volume(self.volume + step, cx);
      self.show_controls(cx);
      cx.stop_propagation();
    }
  }

  fn seek_fraction(&self, x: Pixels) -> Option<f64> {
    let bounds = self.seek_bounds.get();
    let width = f32::from(bounds.size.width);
    (width > 0.0 && self.duration > 0.0)
      .then(|| (f32::from(x - bounds.origin.x) / width).clamp(0.0, 1.0) as f64)
  }

  // --- Rendering ----------------------------------------------------------------------------

  fn render_seek_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let primary = cx.theme().primary;
    let progress = if self.duration > 0.0 {
      (self.position / self.duration).clamp(0.0, 1.0)
    } else {
      0.0
    };
    let height = if self.seek_hover.is_some() { 6. } else { 4. };
    let radius = px(height / 2.);
    let fill = |fraction: f64, color: gpui::Hsla| {
      div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .w(relative(fraction as f32))
        .min_w(px(height))
        .rounded(radius)
        .bg(color)
    };
    let bounds_cell = self.seek_bounds.clone();
    let preview = self.seek_hover.map(|fraction| {
      let seconds = fraction * self.duration;
      let bar_width = f32::from(self.seek_bounds.get().size.width);
      let width = 72.;
      let left = (bar_width * fraction as f32 - width / 2.).clamp(0., (bar_width - width).max(0.));
      div()
        .absolute()
        .bottom(px(22.))
        .left(px(left))
        .w(px(width))
        .flex()
        .justify_center()
        .child(
          div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(gpui::black().opacity(0.8))
            .text_sm()
            .text_color(gpui::white())
            .child(format_time(seconds)),
        )
    });

    div()
      .id("seek")
      .relative()
      .w_full()
      .py_2()
      .cursor_pointer()
      .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
        this.seek_hover = this.seek_fraction(event.position.x);
        cx.notify();
      }))
      .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
        if !hovered {
          this.seek_hover = None;
          cx.notify();
        }
      }))
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, event: &MouseDownEvent, _, cx| {
          if let Some(fraction) = this.seek_fraction(event.position.x) {
            this.seek_to(fraction * this.duration, cx);
          }
        }),
      )
      .child(
        div()
          .relative()
          .h(px(height))
          .w_full()
          .rounded(radius)
          .bg(gpui::white().opacity(0.25))
          // GPUI clips children to rectangles, so each fill carries the track's rounding
          // and is never narrower than tall, or its square ends stick out of the track.
          .children(
            self
              .seek_hover
              .map(|f| fill(f, gpui::white().opacity(0.25))),
          )
          .when(progress > 0.0, |el| el.child(fill(progress, primary)))
          // A knob at the playhead while the bar is hovered.
          .when(self.seek_hover.is_some(), |el| {
            el.child(
              div()
                .absolute()
                .top(px(height / 2. - 7.))
                .left(relative(progress as f32))
                .ml(px(-7.))
                .size(px(14.))
                .rounded_full()
                .bg(primary),
            )
          })
          .child(
            canvas(move |bounds, _, _| bounds_cell.set(bounds), |_, _, _, _| {})
              .absolute()
              .inset_0(),
          ),
      )
      .children(preview)
  }

  /// Mute button; hovering it opens a vertical volume slider above it.
  fn render_volume(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    // Decided here rather than in the hover handlers: going from the button to the slider sends
    // a hover-out and a hover-in in a row, which must not close and reopen it.
    let open = self.volume_hover.0 || self.volume_hover.1 || self.volume_dragging;
    if open != self.volume_open {
      self.volume_open = open;
      self.volume_animation = self.volume_animation.wrapping_add(1);
      self.volume_close_task = (!open).then(|| {
        cx.spawn(async move |this, cx| {
          cx.background_executor().timer(VOLUME_FADE).await;
          this
            .update(cx, |this, cx| {
              this.volume_close_task = None;
              cx.notify();
            })
            .ok();
        })
      });
    }
    let shown = self.volume_open || self.volume_close_task.is_some();
    let (animation, opening) = (self.volume_animation, self.volume_open);
    const TRACK_HEIGHT: f32 = 90.;
    let primary = cx.theme().primary;
    let icon = if self.muted || self.volume == 0.0 {
      Icon::Muted
    } else if self.volume < 50.0 {
      Icon::VolumeLow
    } else {
      Icon::Volume
    };
    let level = if self.muted { 0.0 } else { self.volume / 100.0 } as f32;
    let bounds_cell = self.volume_bounds.clone();
    let (dragging, view) = (self.volume_dragging, cx.weak_entity());
    // The slider sits right on top of the button, so the mouse goes from one to the other
    // without leaving both.
    let slider = div()
      .id("volume-slider")
      .occlude()
      .absolute()
      .bottom(relative(1.))
      .left_0()
      .right_0()
      .py_3()
      .flex()
      .justify_center()
      .rounded_md()
      .bg(gpui::black().opacity(0.75))
      .cursor_pointer()
      .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
        this.volume_hover.1 = *hovered;
        cx.notify();
      }))
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, event: &MouseDownEvent, _, cx| {
          this.volume_dragging = true;
          this.set_volume_at(event.position.y, cx);
        }),
      )
      .child(
        div()
          .relative()
          .w(px(4.))
          .h(px(TRACK_HEIGHT))
          .rounded_full()
          .bg(gpui::white().opacity(0.25))
          .child(
            div()
              .absolute()
              .left_0()
              .right_0()
              .bottom_0()
              .h(relative(level))
              .min_h(px(4.))
              .rounded_full()
              .bg(primary),
          )
          .child(
            div()
              .absolute()
              .bottom(relative(level))
              .left(px(-5.))
              .mb(px(-7.))
              .size(px(14.))
              .rounded_full()
              .bg(primary)
              .border_2()
              .border_color(gpui::white()),
          )
          .child(
            canvas(
              move |bounds, _, _| bounds_cell.set(bounds),
              move |_, _, window, _| {
                if dragging {
                  listen_to_volume_drag(&view, window);
                }
              },
            )
            .absolute()
            .inset_0(),
          ),
      );
    div()
      .id("volume")
      .relative()
      .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
        this.volume_hover.0 = *hovered;
        cx.notify();
      }))
      .child(
        self
          .control_button("mute", cx)
          .icon(icon)
          .tooltip("Couper le son (M)")
          .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
      )
      .when(shown, |el| {
        let ease = ease_out_quint();
        el.child(slider.with_animation(
          SharedString::from(format!("volume-slider-{animation}")),
          Animation::new(VOLUME_FADE),
          // Eased by hand so that closing replays the opening curve backwards.
          move |el, t| {
            let p = ease(if opening { t } else { 1. - t });
            el.opacity(p).mb(px(-8. * (1. - p)))
          },
        ))
      })
  }

  fn render_controls(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Vec<gpui::AnyElement> {
    let fullscreen = window.is_fullscreen();
    let theater = Settings::get(cx).theater;
    let white = |opacity: f32| gpui::white().opacity(opacity);
    let has_captions = self.stream.as_ref().is_some_and(|s| !s.captions.is_empty());
    let has_qualities = self.stream.as_ref().is_some_and(|s| s.qualities.len() > 1);
    let mut layers = Vec::new();

    if fullscreen && !self.title.is_empty() {
      layers.push(
        div()
          .absolute()
          .top_0()
          .left_0()
          .right_0()
          .px_6()
          .pt_4()
          .pb_10()
          .bg(linear_gradient(
            180.,
            linear_color_stop(gpui::black().opacity(0.75), 0.0),
            linear_color_stop(gpui::black().opacity(0.0), 1.0),
          ))
          .text_lg()
          .font_weight(FontWeight::SEMIBOLD)
          .text_color(gpui::white())
          .child(self.title.clone())
          .into_any_element(),
      );
    }

    let buttons = h_flex()
      .gap_1()
      .items_center()
      .when_some(self.queue, |row, (previous, _)| {
        row.child(
          self
            .control_button("previous", cx)
            .icon(Icon::Previous)
            .tooltip("Vidéo précédente (P)")
            .disabled(!previous)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(PlayerEvent::Previous))),
        )
      })
      .child(
        self
          .control_button("play-pause", cx)
          .icon(if self.paused { Icon::Play } else { Icon::Pause })
          .tooltip(if self.paused {
            "Lecture (K)"
          } else {
            "Pause (K)"
          })
          .on_click(cx.listener(|this, _, _, cx| {
            this.with_player(Player::toggle_pause);
            this.show_controls(cx);
          })),
      )
      .when_some(self.queue, |row, (_, next)| {
        row.child(
          self
            .control_button("next", cx)
            .icon(Icon::Next)
            .tooltip("Vidéo suivante (N)")
            .disabled(!next)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(PlayerEvent::Next))),
        )
      })
      .child(
        self
          .control_button("back-10", cx)
          .icon(Icon::Back10)
          .tooltip("Reculer de 10 s (J)")
          .on_click(cx.listener(|this, _, _, cx| this.seek_by(-SEEK_LARGE, cx))),
      )
      .child(
        self
          .control_button("fwd-10", cx)
          .icon(Icon::Forward10)
          .tooltip("Avancer de 10 s (L)")
          .on_click(cx.listener(|this, _, _, cx| this.seek_by(SEEK_LARGE, cx))),
      )
      .child(self.render_volume(cx))
      .child(div().pl_2().text_sm().text_color(white(0.9)).child(format!(
        "{} / {}",
        format_time(self.position),
        format_time(self.duration)
      )))
      .child(div().flex_1())
      .when(has_captions, |row| {
        row.child(
          self
            .control_button("subs-menu", cx)
            .icon(Icon::Subtitles)
            .tooltip("Sous-titres (C)")
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Subtitles, cx))),
        )
      })
      .when(has_qualities, |row| {
        row.child(
          self
            .control_button("quality-menu", cx)
            .icon(Icon::Settings)
            .tooltip("Qualité")
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Quality, cx))),
        )
      })
      // In fullscreen the picture fills the window anyway.
      .when(!fullscreen, |row| {
        row.child(
          self
            .control_button("theater", cx)
            .icon(Icon::Theater)
            .tooltip(if theater {
              "Quitter le mode cinéma (T)"
            } else {
              "Mode cinéma (T)"
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle_theater(cx))),
        )
      })
      .child(
        self
          .control_button("fullscreen", cx)
          .icon(if fullscreen {
            Icon::Minimize
          } else {
            Icon::Maximize
          })
          .tooltip(if fullscreen {
            "Quitter le plein écran (F)"
          } else {
            "Plein écran (F)"
          })
          .on_click(|_, window, _| window.toggle_fullscreen()),
      );

    layers.push(
      v_flex()
        .id("bottom-bar")
        .occlude()
        .on_hover(cx.listener(|this, hovered: &bool, _, _| this.over_controls = *hovered))
        .on_mouse_move(cx.listener(|this, _, _, cx| this.show_controls(cx)))
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .px_4()
        .pt_8()
        .pb_2()
        .bg(linear_gradient(
          180.,
          linear_color_stop(gpui::black().opacity(0.0), 0.0),
          linear_color_stop(gpui::black().opacity(0.8), 1.0),
        ))
        .child(self.render_seek_bar(cx))
        .child(buttons)
        .into_any_element(),
    );
    layers
  }

  /// An icon button of the player: white icon that turns to the accent color on hover, over a
  /// light background, so the hovered button stands out over the video. gpui-component's
  /// buttons keep one foreground for every state, so the view tracks the hovered button itself.
  fn control_button(&self, id: &'static str, cx: &mut Context<Self>) -> ControlButton {
    let foreground = if self.hovered_control == Some(id) {
      cx.theme().primary
    } else {
      gpui::white()
    };
    let button = Button::new(id)
      .custom(
        ButtonCustomVariant::new(cx)
          .foreground(foreground)
          .hover(gpui::white().opacity(0.18))
          .active(gpui::white().opacity(0.26)),
      )
      // The button draws its icon at 3/4 of its size; the explicit size then sets the box.
      .with_size(px(CONTROL_ICON / 0.75))
      .size(px(CONTROL_SIZE));
    let view = cx.weak_entity();
    ControlButton {
      id,
      button,
      on_hover: Box::new(move |hovered, _, cx| {
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
      }),
    }
  }
}

fn listen_to_volume_drag(view: &WeakEntity<PlayerView>, window: &mut Window) {
  let move_view = view.clone();
  window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
    if phase == DispatchPhase::Capture {
      move_view
        .update(cx, |this, cx| {
          if event.pressed_button == Some(MouseButton::Left) {
            this.set_volume_at(event.position.y, cx);
          } else {
            // The button was released where no mouse up reached the window.
            this.end_volume_drag(cx);
          }
        })
        .ok();
    }
  });
  let up_view = view.clone();
  window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
      up_view.update(cx, |this, cx| this.end_volume_drag(cx)).ok();
    }
  });
}

impl PlayerView {
  /// Quality or subtitle list above the control bar (peeby's track menu).
  fn render_menu(&self, menu: Menu, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let (popover, border) = (theme.popover, theme.border);
    let mut list = v_flex().gap_1();
    let entry = |id: SharedString, label: String, selected: bool| {
      Button::new(id)
        .label(label)
        .when(selected, |b| b.primary())
        .when(!selected, |b| b.ghost())
    };
    let no_variants: Vec<Variant> = Vec::new();
    let no_captions: Vec<innertube::Caption> = Vec::new();
    let stream = self.stream.as_ref();
    match menu {
      Menu::Quality => {
        for (i, variant) in stream
          .map_or(&no_variants, |s| &s.qualities)
          .iter()
          .enumerate()
        {
          list = list.child(
            entry(
              format!("quality-{i}").into(),
              variant.label(),
              self.quality == Some(i),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_quality(i, cx))),
          );
        }
      }
      Menu::Subtitles => {
        list = list.child(
          entry(
            "sub-off".into(),
            "Désactivés".into(),
            self.caption.is_none(),
          )
          .on_click(cx.listener(|this, _, _, cx| this.select_caption(None, cx))),
        );
        for (i, caption) in stream
          .map_or(&no_captions, |s| &s.captions)
          .iter()
          .enumerate()
        {
          list = list.child(
            entry(
              format!("sub-{i}").into(),
              caption_label(caption),
              self.caption == Some(i),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_caption(Some(i), cx))),
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
      .max_h(px(320.))
      .min_w(px(220.))
      .overflow_y_scroll()
      .p_2()
      .rounded_lg()
      .bg(popover)
      .border_1()
      .border_color(border)
      .child(list)
  }
}

/// Player options for an HLS variant and its audio rendition.
fn stream_options(
  video: &str,
  audio: Option<&str>,
  user_agent: &str,
  start_seconds: f64,
) -> StartOptions {
  StartOptions {
    url: video.to_owned(),
    external_audio: audio.map(str::to_owned).into_iter().collect(),
    http_headers: vec![format!("User-Agent: {user_agent}")],
    start_seconds,
    // Opt-in only: the log contains stream URLs, which carry the user's IP.
    log_file: std::env::var_os("PEEBTUBE_MPV_LOG").map(Into::into),
    ..Default::default()
  }
}

/// `Anglais`, `Anglais (générés automatiquement)`: YouTube's names, in the app's language.
fn caption_label(caption: &innertube::Caption) -> String {
  caption.name.clone()
}

/// A player button in a wrapper that reports its hover: the button's own `on_hover` is taken
/// by its tooltip, and GPUI allows only one per element.
#[derive(IntoElement)]
struct ControlButton {
  id: &'static str,
  button: Button,
  on_hover: Box<HoverListener>,
}

type HoverListener = dyn Fn(&bool, &mut Window, &mut App);

impl ControlButton {
  fn icon(mut self, icon: Icon) -> Self {
    self.button = self.button.icon(icon);
    self
  }

  fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
    self.button = self.button.tooltip(tooltip);
    self
  }

  fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
    self.button = self.button.on_click(handler);
    self
  }

  fn disabled(mut self, disabled: bool) -> Self {
    self.button = self.button.disabled(disabled);
    self
  }
}

impl RenderOnce for ControlButton {
  fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
    div()
      .id(SharedString::from(format!("{}-hover", self.id)))
      .flex_none()
      .on_hover(self.on_hover)
      .child(self.button)
  }
}

/// `1:02:03` or `12:34`.
fn format_time(seconds: f64) -> String {
  let total = seconds.max(0.0) as u64;
  let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
  if h > 0 {
    format!("{h}:{m:02}:{s:02}")
  } else {
    format!("{m}:{s:02}")
  }
}

impl Render for PlayerView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let show_ui = (self.controls_visible || self.failed) && self.player.is_some();

    let mut root = div()
      .id("player")
      .size_full()
      .relative()
      .bg(gpui::black())
      .track_focus(&self.focus)
      .on_key_down(cx.listener(Self::on_key))
      .children(
        self
          .video
          .clone()
          .map(|video| div().absolute().inset_0().child(video)),
      )
      // Picture area: clicks, double-clicks, wheel and mouse moves (the bars occlude it).
      .child(
        div()
          .id("picture")
          .absolute()
          .inset_0()
          .on_click(cx.listener(Self::on_video_click))
          .on_scroll(cx.listener(Self::on_wheel))
          .on_mouse_move(cx.listener(|this, _, _, cx| this.show_controls(cx))),
      );

    let status = self
      .status
      .clone()
      .or_else(|| self.buffering.then(|| "Chargement…".into()));
    if let Some(status) = status {
      root = root.child(
        div()
          .absolute()
          .inset_0()
          .flex()
          .items_center()
          .justify_center()
          .child(
            div()
              .max_w(px(560.))
              .px_4()
              .py_2()
              .rounded_md()
              .bg(gpui::black().opacity(0.6))
              .text_color(gpui::white())
              .child(status),
          ),
      );
    }
    if let Some((count, paused)) = self.pause_pulse {
      // The new state's icon grows and fades out, like a ripple. Once done it stays fully
      // transparent; it has no handlers, so it takes no mouse events.
      root = root.child(
        div()
          .absolute()
          .inset_0()
          .flex()
          .items_center()
          .justify_center()
          .child(
            div()
              .flex()
              .items_center()
              .justify_center()
              .rounded_full()
              .bg(gpui::black().opacity(0.5))
              .text_color(gpui::white())
              .child(
                gpui_component::Icon::new(if paused { Icon::Pause } else { Icon::Play })
                  .size(px(32.)),
              )
              .with_animation(
                SharedString::from(format!("pause-pulse-{count}")),
                Animation::new(PAUSE_PULSE).with_easing(ease_out_quint()),
                |el, t| el.size(px(64. + 32. * t)).opacity(1. - t),
              ),
          ),
      );
    }
    if let Some(osd) = self.osd.clone() {
      root = root.child(
        div()
          .absolute()
          .top(px(24.))
          .left_0()
          .right_0()
          .flex()
          .justify_center()
          .child(
            div()
              .px_4()
              .py_2()
              .rounded_md()
              .bg(gpui::black().opacity(0.7))
              .text_color(gpui::white())
              .child(osd),
          ),
      );
    }
    if show_ui && !self.failed {
      root = root.children(self.render_controls(window, cx));
      if let Some(menu) = self.menu {
        root = root.child(self.render_menu(menu, cx));
      }
    }
    root
  }
}
