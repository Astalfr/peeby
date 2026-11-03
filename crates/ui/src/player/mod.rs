//! The video player shared by every space: picture, auto-hiding controls (seek bar with
//! previews and chapters, volume, buttons, quality, audio, subtitle and chapter menus), skip
//! button, "up next" card, OSD, shortcut help and keyboard shortcuts. It plays a [`Source`] and
//! tells its page what happens through [`PlayerEvent`]s; it reads no setting itself. Embedded in
//! a page ([`Layout::Embedded`], YouTube), filling a page of its own ([`Layout::Page`], Jellyfin)
//! or the window ([`Layout::Window`], the example); fills the window in fullscreen. Controls and
//! behavior come from peeby's player page.

pub mod chapter_list;
mod overlays;
pub mod segments;
mod source;
mod stats;
mod tracks;

pub use overlays::format_step;
pub use source::{LIVE_DELAY, LiveInfo, Previews, Quality, Source, Subtitle, Tile};

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Context, Corners, DispatchPhase,
  Entity, EventEmitter, FocusHandle, FontWeight, Global, ImageSource, KeyDownEvent, KeyUpEvent,
  MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point, ScrollEvent,
  SharedString, Task, WeakEntity, Window, canvas, div, ease_out_quint, img, linear_color_stop,
  linear_gradient, px, relative,
};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_component::tooltip::Tooltip;
use gpui_component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use gpui_video::VideoView;
use player::{Chapter, Choice, Event, Player, StartOptions, Track};
use segments::{Segment, SegmentKind};
use tracks::{SUBTITLE_DELAY_STEP, UpNext};

use crate::assets::Icon;
use crate::i18n::{Lang, lang, t};
use crate::images::Images;
use crate::picture::{self, Picture};
use crate::{motion, tf, theme};

const HIDE_CONTROLS_AFTER: Duration = Duration::from_secs(3);
const OSD_DURATION: Duration = Duration::from_millis(1500);
const PAUSE_PULSE: Duration = Duration::from_millis(600);
/// Space or the left button held on the picture this long plays at [`HOLD_SPEED`] until the
/// release; released sooner, it plays or pauses.
const HOLD_DELAY: Duration = Duration::from_millis(350);
const HOLD_SPEED: f64 = 2.0;
/// The speeds of the speed menu.
const SPEEDS: [f64; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
/// A live paused longer than this resumes at its live edge rather than where it stopped (shorter:
/// a double click's two toggles).
const LIVE_RESUME_AFTER: Duration = Duration::from_secs(1);
/// mpv reporting the end further than this from the duration: a stream stopped short (its
/// segments stopped coming), not the end of the video.
const EARLY_END_MARGIN: f64 = 10.0;
/// A second early end within this time of a reload is not reloaded again: it fails.
const EARLY_END_RETRY: Duration = Duration::from_secs(60);
const VOLUME_FADE: Duration = Duration::from_millis(150);
const VOLUME_STEP: f64 = 5.0;
/// Width of the storyboard frame shown over the seek bar.
const PREVIEW_WIDTH: f32 = 176.;
/// Width of a chapter's picture in the seek bar's hover preview.
const HOVER_PICTURE_WIDTH: f32 = 16.;

/// Space between two chapters on the seek bar.
const CHAPTER_GAP: f32 = 2.;
const CONTROL_SIZE: f32 = 40.;
const CONTROL_ICON: f32 = 20.;

/// What the player tells its page.
pub enum PlayerEvent {
  /// The video played to its end.
  Ended,
  /// The user asked for the next or the previous video of the queue.
  Next,
  /// See [`PlayerEvent::Next`].
  Previous,
  /// The user asked to switch theater mode (button or T): the page saves it, lays itself out
  /// again and calls [`PlayerView::set_theater`].
  Theater,
  /// The volume or the mute changed, to keep for the next videos.
  Sound {
    /// 0 to 100.
    volume: f64,
    /// Muted.
    muted: bool,
  },
  /// The file is loaded and plays (once per source): the moment to report playback.
  Started,
  /// Paused or resumed.
  Paused(bool),
  /// The user moved the playhead.
  Seeked,
  /// mpv's selected tracks, by mpv id (1, 2… per type), after any change.
  TracksChanged {
    /// Audio track.
    audio: Option<i64>,
    /// Subtitle track, `None` when off.
    subtitle: Option<i64>,
  },
  /// The user asked to leave the player (full-window layout: back button, Escape, Backspace),
  /// or mpv quit.
  Close,
}

/// The player that started last, as a global the shell observes (its mini-player shows only
/// that one).
#[derive(Default)]
pub struct NowPlaying(Option<WeakEntity<PlayerView>>);

impl Global for NowPlaying {}

/// The player that started last, while it exists.
pub fn now_playing(cx: &App) -> Option<Entity<PlayerView>> {
  cx.try_global::<NowPlaying>()?.0.as_ref()?.upgrade()
}

/// The audio output every player plays to ([`player::AudioDevice::name`]; `None`: Windows'
/// default), a global the app sets from its settings: changing it moves the players playing.
#[derive(Default)]
pub struct AudioOutput(pub Option<String>);

impl Global for AudioOutput {}

/// The audio output the players play to.
pub fn audio_output(cx: &App) -> Option<String> {
  cx.try_global::<AudioOutput>()?.0.clone()
}

/// Where the player sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
  /// In a page with other content (YouTube's watch page): theater mode, the wheel scrolls the
  /// page unless in fullscreen.
  Embedded,
  /// A page of its own in the content panel (Jellyfin): a top bar with back, title and subtitle
  /// over the picture; Escape and Backspace close it; the wheel sets the volume.
  Page,
  /// The whole window (the example): like [`Layout::Page`], and the top bar has the window
  /// buttons and moves the window.
  Window,
}

impl Layout {
  /// Whether the player is the whole page (back button, Escape closes it).
  fn is_page(self) -> bool {
    self != Self::Embedded
  }
}

/// The user's preferences the player starts with.
#[derive(Clone, Debug)]
pub struct Prefs {
  /// 0 to 100.
  pub volume: f64,
  /// Muted.
  pub muted: bool,
  /// Language picked first by the subtitles key (C) among subtitles added on demand, e.g. `fr`.
  pub subtitle_language: Option<String>,
  /// Arrows and J/L seek this many seconds…
  pub seek_step: u32,
  /// …and this many with Shift.
  pub seek_step_large: u32,
  /// Plays the next item when the "up next" countdown ends.
  pub autoplay_next: bool,
  /// Length of that countdown, seconds.
  pub autoplay_countdown: u32,
}

impl Default for Prefs {
  fn default() -> Self {
    Self {
      volume: 100.0,
      muted: false,
      subtitle_language: None,
      seek_step: 10,
      seek_step_large: 60,
      autoplay_next: true,
      autoplay_countdown: 10,
    }
  }
}

/// The open menu of the control bar.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Menu {
  Quality,
  Speed,
  Audio,
  Subtitles,
  Chapters,
}

/// What is held down for the 2× speed ([`HOLD_DELAY`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hold {
  Space,
  Mouse,
}

/// The player: see the module documentation.
pub struct PlayerView {
  focus: FocusHandle,
  prefs: Prefs,
  /// The page is in theater mode: the player spans it and squares its corners but one.
  theater: bool,
  layout: Layout,
  /// Second line of the title (episode, year), shown in the full-window layout.
  subtitle: SharedString,
  /// mpv's tracks of the playing file.
  tracks: Vec<Track>,
  subtitle_delay: f64,
  chapters: Vec<Chapter>,
  /// The chapters come from the source's server (YouTube's), not from the file: mpv's list
  /// (empty for a stream, sent again by each new mpv) no longer replaces them.
  given_chapters: bool,
  /// The given chapters' pictures (a game's box art), one per chapter; empty when they have none.
  chapter_pictures: Vec<Option<Picture>>,
  /// Segments from the source's server; without them, `segments` comes from the chapters.
  given_segments: Vec<Segment>,
  segments: Vec<Segment>,
  /// The start of the muted segment playing, told once on entering it.
  muted_at: Option<f64>,
  /// The next item, offered in the "up next" card near the end.
  up_next_title: Option<SharedString>,
  up_next: UpNext,
  countdown_task: Option<Task<()>>,
  /// The shortcut help is open.
  help: bool,
  /// The right-click menu is open, at this point of the player.
  context_menu: Option<Point<Pixels>>,
  /// The statistics panel is open; its lines and the task refreshing them.
  stats: bool,
  stats_rows: Vec<stats::Row>,
  stats_task: Option<Task<()>>,
  /// Where the player is in the window, for the right-click menu.
  bounds: Rc<Cell<Bounds<Pixels>>>,
  title: SharedString,
  player: Option<Rc<Player>>,
  video: Option<Entity<VideoView>>,
  /// Shown instead of the picture: loading, or why the video cannot play.
  status: Option<SharedString>,
  failed: bool,
  position: f64,
  duration: f64,
  /// How far the file is loaded, on the position's timeline: drawn ahead of the playhead.
  cached: f64,
  paused: bool,
  /// When playback was paused, for a live's resume ([`LIVE_RESUME_AFTER`]).
  paused_since: Option<Instant>,
  started: bool,
  /// The last play/pause change, shown as a short pulse in the middle of the picture; the
  /// counter gives each pulse its own animation id, so a new change restarts it.
  pause_pulse: Option<(u32, bool)>,
  /// Space or the left button held on the picture, and the task switching to [`HOLD_SPEED`]
  /// once it lasts [`HOLD_DELAY`].
  hold: Option<Hold>,
  hold_task: Option<Task<()>>,
  /// Playing at [`HOLD_SPEED`] until the hold ends.
  fast: bool,
  /// The speed picked in the menu, kept across mpv restarts (qualities, next videos).
  speed: f64,
  /// The last left-button press on the picture ended a 2× hold: its click neither plays nor
  /// pauses.
  hold_ended: bool,
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
  /// The left button went down on the seek bar and is still held: the playhead follows the
  /// mouse horizontally wherever it is, and mpv's positions are ignored until the release.
  seek_dragging: bool,
  /// What is being played: its qualities and subtitle tracks.
  source: Option<Source>,
  /// Index of the playing quality in `source.qualities`.
  quality: Option<usize>,
  /// Selected subtitle track, index in `source.subtitles`.
  caption: Option<usize>,
  /// Subtitle URLs already given to mpv, in mpv's track order (external subtitles get the ids
  /// 1, 2… since the streams carry none).
  captions_added: Vec<String>,
  menu: Option<Menu>,
  /// Playing a queue: whether it has a previous and a next video (buttons, P and N keys).
  queue: Option<(bool, bool)>,
  /// Viewers and start time next to the live badge, and the task redrawing the uptime.
  live_info: Option<LiveInfo>,
  live_tick: Option<Task<()>>,
  /// A message over the picture from the page, e.g. why it waits (an ad removed from a live).
  notice: Option<SharedString>,
  /// When the stream was last reloaded after ending early ([`EARLY_END_MARGIN`]).
  early_end_reload: Option<Instant>,
  _events: Option<Task<()>>,
}

impl EventEmitter<PlayerEvent> for PlayerView {}

impl PlayerView {
  /// A player waiting for its source ([`Self::start`]).
  pub fn new(prefs: Prefs, theater: bool, cx: &mut Context<Self>) -> Self {
    // A storyboard sheet arriving redraws the seek bar's preview.
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |this: &mut Self, _, cx| {
        if this.seek_hover.is_some() {
          cx.notify();
        }
      })
      .detach();
    }
    cx.observe_global::<AudioOutput>(|this: &mut Self, cx| {
      let device = audio_output(cx);
      this.with_player(|p| p.set_audio_device(device.as_deref()));
    })
    .detach();
    Self {
      focus: cx.focus_handle(),
      theater,
      layout: Layout::Embedded,
      subtitle: SharedString::default(),
      tracks: Vec::new(),
      subtitle_delay: 0.0,
      chapters: Vec::new(),
      given_chapters: false,
      chapter_pictures: Vec::new(),
      given_segments: Vec::new(),
      segments: Vec::new(),
      muted_at: None,
      up_next_title: None,
      up_next: UpNext::Hidden,
      countdown_task: None,
      help: false,
      context_menu: None,
      stats: false,
      stats_rows: Vec::new(),
      stats_task: None,
      bounds: Rc::default(),
      title: SharedString::default(),
      player: None,
      video: None,
      status: Some(t("Loading…", "Chargement…").into()),
      failed: false,
      position: 0.0,
      duration: 0.0,
      cached: 0.0,
      paused: false,
      paused_since: None,
      started: false,
      pause_pulse: None,
      hold: None,
      hold_task: None,
      fast: false,
      speed: 1.0,
      hold_ended: false,
      buffering: false,
      volume: prefs.volume,
      muted: prefs.muted,
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
      seek_dragging: false,
      prefs,
      source: None,
      quality: None,
      caption: None,
      captions_added: Vec::new(),
      menu: None,
      queue: None,
      live_info: None,
      live_tick: None,
      notice: None,
      early_end_reload: None,
      _events: None,
    }
  }

  /// Whether the source is a live stream ([`Source::live`]).
  pub fn is_live(&self) -> bool {
    self.source.as_ref().is_some_and(|s| s.live)
  }

  /// How far back the live can be watched, if it keeps its past ([`Source::dvr_window`]).
  fn dvr_window(&self) -> Option<f64> {
    self
      .source
      .as_ref()
      .filter(|s| s.live)
      .and_then(|s| s.dvr_window)
  }

  /// Whether the timeline can be sought: a video, or a live with its past.
  fn can_seek(&self) -> bool {
    !self.is_live() || self.dvr_window().is_some()
  }

  /// What the seek bar covers, in mpv's timeline: the whole video, or a DVR live's window up to
  /// its edge (mpv's duration grows with the live while the oldest segments drop out).
  fn seek_range(&self) -> (f64, f64) {
    let end = self.duration.max(0.0);
    match self.dvr_window() {
      Some(window) => ((end - window).max(0.0), end),
      None => (0.0, end),
    }
  }

  /// How far behind the live edge a DVR live plays, seconds.
  fn behind_live(&self) -> f64 {
    (self.duration - self.position).max(0.0)
  }

  /// Whether a DVR live plays at its edge (the playhead trails mpv's duration, which grows by a
  /// whole segment at a time).
  fn at_live_edge(&self) -> bool {
    self.behind_live() < LIVE_DELAY + 10.0
  }

  /// Back to a DVR live's edge: a seek, mpv keeps reloading the playlist.
  fn go_live(&mut self, cx: &mut Context<Self>) {
    self.seek_to(self.duration - LIVE_DELAY, cx);
    self.show_osd(t("Back to live", "Retour au direct").into(), cx);
  }

  /// Where the live edge is in a new mpv's timeline ([`Source::live`]).
  fn live_start(&self) -> f64 {
    self
      .source
      .as_ref()
      .map_or(0.0, |s| s.options.start_seconds)
  }

  /// Viewers and start time shown next to the live badge; the uptime ticks every second.
  pub fn set_live_info(&mut self, info: Option<LiveInfo>, cx: &mut Context<Self>) {
    let ticking = info.as_ref().is_some_and(|i| i.started_at.is_some());
    self.live_info = info;
    self.live_tick = ticking.then(|| {
      cx.spawn(async move |this, cx| {
        loop {
          cx.background_executor().timer(Duration::from_secs(1)).await;
          let alive = this.update(cx, |this, cx| {
            if this.controls_visible {
              cx.notify();
            }
          });
          if alive.is_err() {
            break;
          }
        }
      })
    });
    cx.notify();
  }

  /// A message over the picture, or none: the page says why the picture waits (an ad removed
  /// from a live, with its countdown). It replaces the loading message meanwhile.
  pub fn set_notice(&mut self, notice: Option<SharedString>, cx: &mut Context<Self>) {
    if self.notice != notice {
      self.notice = notice;
      cx.notify();
    }
  }

  /// Focus of the player, for its keys.
  pub fn focus_handle(&self) -> &FocusHandle {
    &self.focus
  }

  /// Title shown over the picture in fullscreen and in the full-window layout.
  pub fn set_title(&mut self, title: SharedString, cx: &mut Context<Self>) {
    self.title = title;
    cx.notify();
  }

  /// Second line of the title in the full-window layout (episode, year).
  pub fn set_subtitle(&mut self, subtitle: SharedString, cx: &mut Context<Self>) {
    self.subtitle = subtitle;
    cx.notify();
  }

  /// Where the player sits; [`Layout::Embedded`] until set.
  pub fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
    self.layout = layout;
    cx.notify();
  }

  /// Playback position, seconds.
  pub fn position(&self) -> f64 {
    self.position
  }

  /// Length of the video, seconds (0 until known).
  pub fn duration(&self) -> f64 {
    self.duration
  }

  /// Whether playback is paused.
  pub fn is_paused(&self) -> bool {
    self.paused
  }

  /// The picture, for a mini-player showing this player elsewhere.
  pub fn video(&self) -> Option<Entity<VideoView>> {
    self.video.clone()
  }

  /// An mpv property as text, for diagnostics (`hwdec-current`, `video-codec`…); `None` before
  /// mpv starts. Never read `path` nor `stream-open-filename`: stream URLs carry the user's IP.
  pub fn property(&self, name: &str) -> Option<String> {
    self.player.as_ref()?.property(name)
  }

  /// The index of the quality playing in the source's list.
  pub fn quality(&self) -> Option<usize> {
    self.quality
  }

  /// Plays the source's quality `index`, as if picked in the menu (a page falling back from a
  /// codec the GPU does not decode).
  pub fn set_quality(&mut self, index: usize, cx: &mut Context<Self>) {
    self.select_quality(index, cx);
  }

  /// Plays or pauses. A live paused for a while resumes at its live edge, as Twitch's player:
  /// mpv restarts on the current quality (about 0.7 s, the last frame staying on screen). A DVR
  /// live resumes where it was paused, as YouTube's.
  pub fn toggle_pause(&mut self, cx: &mut Context<Self>) {
    let stale = self.paused
      && self
        .paused_since
        .is_some_and(|t| t.elapsed() > LIVE_RESUME_AFTER);
    if self.is_live() && self.dvr_window().is_none() && stale && self.player.is_some() {
      self.resume_live(cx);
    } else {
      self.with_player(Player::toggle_pause);
    }
  }

  /// Space or the left button went down: once held for [`HOLD_DELAY`], a playing video (not a
  /// live, which has nothing ahead) plays at [`HOLD_SPEED`] until the release.
  fn start_hold(&mut self, hold: Hold, cx: &mut Context<Self>) {
    self.hold = Some(hold);
    // Redrawn so that the window listens for the button's release.
    cx.notify();
    self.hold_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(HOLD_DELAY).await;
      this
        .update(cx, |this, cx| {
          this.hold_task = None;
          if this.hold.is_some() && !this.paused && !this.is_live() && this.player.is_some() {
            this.fast = true;
            this.with_player(|p| p.set_speed(HOLD_SPEED));
            cx.notify();
          }
        })
        .ok();
    }));
  }

  /// Plays at `speed`, picked in the speed menu. A live has nothing ahead: it keeps 1×.
  fn select_speed(&mut self, speed: f64, cx: &mut Context<Self>) {
    self.menu = None;
    self.speed = speed;
    if !self.fast {
      self.with_player(|p| p.set_speed(speed));
    }
    self.show_osd(tf!("Speed {}", "Vitesse {}", speed_label(speed)).into(), cx);
  }

  /// Ends the hold, back to the menu's speed; true when it was playing at [`HOLD_SPEED`] (the
  /// release then neither plays nor pauses).
  fn end_hold(&mut self, cx: &mut Context<Self>) -> bool {
    self.hold = None;
    self.hold_task = None;
    let fast = std::mem::take(&mut self.fast);
    if fast {
      let speed = self.speed;
      self.with_player(|p| p.set_speed(speed));
      cx.notify();
    }
    fast
  }

  /// The left button held on the picture was released, wherever the mouse is.
  fn end_mouse_hold(&mut self, _: Point<Pixels>, cx: &mut Context<Self>) {
    if self.hold == Some(Hold::Mouse) {
      self.hold_ended = self.end_hold(cx);
    }
  }

  /// Restarts a paused live at its live edge.
  fn resume_live(&mut self, cx: &mut Context<Self>) {
    let Some(source) = &self.source else {
      return;
    };
    let options = match self.quality.and_then(|i| source.qualities.get(i)) {
      Some(quality) => quality.options.clone(),
      None => source.options.clone(),
    };
    tracing::info!("live resumed after a pause: back to its live edge");
    self.paused_since = None;
    self.show_osd(t("Back to live", "Retour au direct").into(), cx);
    // Already paused: its last frame stays on screen until the new player's first one.
    self.player = None;
    self.captions_added.clear();
    let options = StartOptions {
      start_seconds: self.live_start(),
      ..options
    };
    self.start_with(options, Instant::now(), cx);
  }

  /// Pauses.
  pub fn pause(&self) {
    self.with_player(|p| p.set_paused(true));
  }

  /// Becomes the player that plays, pausing the previous one: one sound at a time.
  fn take_over(&self, cx: &mut Context<Self>) {
    let me = cx.entity();
    if let Some(previous) = now_playing(cx)
      && previous != me
    {
      previous.read(cx).pause();
    }
    cx.set_global(NowPlaying(Some(me.downgrade())));
  }

  /// Asks the page to close the player (full-window layout).
  fn close(&mut self, cx: &mut Context<Self>) {
    cx.emit(PlayerEvent::Close);
  }

  /// Shows the previous and next buttons (a queue's videos, a series' episodes), enabled when
  /// there is such an item.
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

  /// Switches theater mode on or off (the corners follow).
  pub fn set_theater(&mut self, theater: bool, cx: &mut Context<Self>) {
    self.theater = theater;
    cx.notify();
  }

  /// Starts mpv on `source`; `clicked` is when the user opened the video, for the timing log.
  pub fn start(&mut self, source: Source, clicked: Instant, cx: &mut Context<Self>) {
    self.quality = source.quality;
    tracing::info!(
      "{} qualities ({}), {} subtitle tracks",
      source.qualities.len(),
      source
        .qualities
        .iter()
        .map(|q| q.label.as_ref())
        .collect::<Vec<_>>()
        .join(" "),
      source.subtitles.len()
    );
    let options = source.options.clone();
    self.source = Some(source);
    self.start_with(options, clicked, cx);
  }

  /// Plays another quality from the current position (a live: from its live edge; a DVR live:
  /// as far behind it): mpv restarts on the new variant (about 0.7 s), and the subtitle track is
  /// added again.
  fn select_quality(&mut self, index: usize, cx: &mut Context<Self>) {
    self.menu = None;
    if self.quality == Some(index) {
      return cx.notify();
    }
    let Some(quality) = self
      .source
      .as_ref()
      .and_then(|s| s.qualities.get(index).cloned())
    else {
      return;
    };
    let start_seconds = match self.dvr_window() {
      // The new mpv's timeline starts at the window's current first segment: the broadcast's
      // start while the window grows, `window` before the edge once it slides.
      // At least 1 s: 0 gives mpv no `start`, and a YouTube live then waits on its audio.
      Some(window) => {
        let length = self.duration.min(window);
        (length - self.behind_live().max(LIVE_DELAY)).max(1.0)
      }
      None if self.is_live() => self.live_start(),
      None => self.position,
    };
    let options = StartOptions {
      start_seconds,
      ..quality.options
    };
    tracing::info!("quality {}", quality.label);
    self.quality = Some(index);
    self.show_osd(tf!("Quality {}", "Qualité {}", quality.label).into(), cx);
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
    let label = match index.and_then(|i| self.source.as_ref()?.subtitles.get(i)) {
      Some(subtitle) => subtitle.label.clone(),
      None => t("Off", "Désactivés").to_owned(),
    };
    self.apply_caption();
    self.show_osd(
      tf!("Subtitles: {label}", "Sous-titres : {label}").into(),
      cx,
    );
  }

  fn apply_caption(&mut self) {
    let Some(player) = self.player.clone() else {
      return;
    };
    let caption = self
      .caption
      .and_then(|i| self.source.as_ref()?.subtitles.get(i).cloned());
    match caption {
      None => player.set_subtitle(Choice::Off),
      Some(caption) => match self.captions_added.iter().position(|u| *u == caption.url) {
        Some(added) => player.set_subtitle(Choice::Track(added as i64 + 1)),
        None => {
          player.add_subtitle(&caption.url, &caption.label, &caption.language);
          self.captions_added.push(caption.url);
        }
      },
    }
  }

  /// C: off → the first track in the preferred language (else the first one) → off.
  fn toggle_captions(&mut self, cx: &mut Context<Self>) {
    if self.caption.is_some() {
      return self.select_caption(None, cx);
    }
    let Some(source) = &self.source else { return };
    if source.subtitles.is_empty() {
      return self.show_osd(t("No subtitles", "Aucun sous-titre").into(), cx);
    }
    let language = self.prefs.subtitle_language.clone().unwrap_or_default();
    let index = source
      .subtitles
      .iter()
      .position(|c| {
        !language.is_empty()
          && (c.language == language || c.language.starts_with(&format!("{language}-")))
      })
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
    let options = StartOptions {
      audio_device: options.audio_device.or_else(|| audio_output(cx)),
      ..options
    };
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
              this.fail(
                tf!("Player unavailable: {e}", "Lecteur indisponible : {e}").into(),
                cx,
              )
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
    // A new mpv starts with an empty cache.
    self.cached = 0.0;
    player.set_volume(self.volume);
    if self.muted {
      player.set_muted(true);
    }
    if self.speed != 1.0 && !self.is_live() {
      player.set_speed(self.speed);
    }
    if self.covered {
      tracing::info!("player started on a covered page: paused");
      player.set_paused(true);
    }
    self.player = Some(player);
    self.status = None;
    // After a quality change: the subtitle track goes to the new mpv. Only for subtitles added
    // on demand: a file's own tracks were picked by the source's `sid` (Jellyfin), which
    // `apply_caption` would turn off.
    if self.on_demand_subtitles() {
      self.apply_caption();
    }
    self.show_controls(cx);
  }

  fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
    match event {
      Event::Loaded => {
        self.buffering = false;
        if !self.started {
          self.started = true;
          self.take_over(cx);
          cx.emit(PlayerEvent::Started);
        }
      }
      // While dragging, mpv still reports the positions of the seeks it has not caught up
      // with: the playhead would jump back behind the mouse.
      Event::Position(seconds) if !self.seek_dragging => {
        self.position = seconds;
        self.update_up_next(cx);
        self.tell_muted(cx);
      }
      Event::Position(_) => {}
      Event::Cached(seconds) => self.cached = seconds,
      // A live's duration is only its playlist's window: kept at 0, it turns off the seek
      // bar's math, "up next" and `finished`. A DVR live's is its edge, for the seek bar.
      Event::Duration(_) if !self.can_seek() => {}
      Event::Duration(seconds) => {
        self.duration = seconds;
        self.refresh_segments();
      }
      Event::Paused(paused) => {
        // Not for the state the file opens in.
        if self.started && paused != self.paused {
          let count = self
            .pause_pulse
            .map_or(0, |(count, _)| count.wrapping_add(1));
          self.pause_pulse = Some((count, paused));
        }
        if paused != self.paused {
          cx.emit(PlayerEvent::Paused(paused));
          self.paused_since = paused.then(Instant::now);
        }
        self.paused = paused;
        self.show_controls(cx);
      }
      Event::Buffering(buffering) => self.buffering = buffering,
      Event::Muted(muted) => self.muted = muted,
      Event::Tracks(tracks) => self.on_tracks(tracks, cx),
      Event::Chapters(chapters) => self.on_chapters(chapters),
      Event::EndReached if self.ended_early() => self.reload_after_early_end(cx),
      Event::EndReached => {
        self.position = self.position.max(self.duration);
        let autoplay = self.up_next_title.is_some()
          && self.prefs.autoplay_next
          && self.up_next != UpNext::Dismissed;
        if autoplay {
          self.play_next(cx);
        } else {
          cx.emit(PlayerEvent::Ended);
        }
      }
      Event::Ended(Some(error)) => self.fail(
        tf!("Playback stopped: {error}", "Lecture interrompue : {error}").into(),
        cx,
      ),
      Event::Ended(None) => {
        if self.layout.is_page() {
          self.close(cx);
        }
      }
    }
    cx.notify();
  }

  /// Whether mpv reached the end of a video well before its duration: one of its streams (the
  /// video or the separate audio of a YouTube variant) stopped, and the other ran on alone or
  /// a seek went past where it stopped. Not a reason to play the next video.
  fn ended_early(&self) -> bool {
    !self.is_live() && self.duration > 0.0 && self.position < self.duration - EARLY_END_MARGIN
  }

  /// Restarts mpv on the playing quality from the current position, once: a second early end
  /// soon after stops playback with an error rather than skipping to the next video.
  fn reload_after_early_end(&mut self, cx: &mut Context<Self>) {
    let options = self.source.as_ref().map(|source| {
      self
        .quality
        .and_then(|i| source.qualities.get(i))
        .map_or_else(|| source.options.clone(), |q| q.options.clone())
    });
    let retried = self
      .early_end_reload
      .is_some_and(|at| at.elapsed() < EARLY_END_RETRY);
    let Some(options) = options.filter(|_| !retried) else {
      tracing::warn!(
        "stream ended at {:.0} s of {:.0} s",
        self.position,
        self.duration
      );
      return self.fail(
        t(
          "Playback stopped: the stream ended before the video.",
          "Lecture interrompue : le flux s'est arrêté avant la fin de la vidéo.",
        )
        .into(),
        cx,
      );
    };
    tracing::warn!(
      "stream ended at {:.0} s of {:.0} s: reloading",
      self.position,
      self.duration
    );
    self.early_end_reload = Some(Instant::now());
    let options = StartOptions {
      start_seconds: self.position,
      ..options
    };
    if let Some(old) = self.player.take() {
      old.set_paused(true);
    }
    self.captions_added.clear();
    self.start_with(options, Instant::now(), cx);
  }

  fn with_player(&self, f: impl FnOnce(&Player)) {
    if let Some(player) = &self.player {
      f(player);
    }
  }

  /// Seeks to `seconds`, within what can be seeked (a timestamp's click).
  pub fn seek_to(&mut self, seconds: f64, cx: &mut Context<Self>) {
    let (start, mut end) = self.seek_range();
    // Closer to a live's edge, mpv would wait for segments that do not exist yet.
    if self.dvr_window().is_some() {
      end = (end - LIVE_DELAY).max(start);
    }
    let target = seconds.clamp(start, end);
    self.position = target;
    self.with_player(|p| p.seek_to(target));
    cx.emit(PlayerEvent::Seeked);
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
    self.show_osd(
      tf!("Volume {:.0}%", "Volume {:.0} %", self.volume).into(),
      cx,
    );
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

  /// Seeks to the mouse's horizontal position on the seek bar, clamped to its ends.
  fn seek_at(&mut self, x: Pixels, cx: &mut Context<Self>) {
    if let Some(fraction) = self.seek_fraction(x) {
      self.seek_hover = Some(fraction);
      self.seek_to(self.seek_time(fraction), cx);
    }
  }

  fn end_seek_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
    self.seek_dragging = false;
    // Released away from the bar: no hover-out comes, the bar was left during the drag.
    if !self.seek_bounds.get().dilate(px(8.)).contains(&position) {
      self.seek_hover = None;
    }
    self.show_controls(cx);
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
        t("Muted", "Son coupé")
      } else {
        t("Unmuted", "Son activé")
      }
      .into(),
      cx,
    );
  }

  /// Tells the page, which keeps it for the next video (and the next launch).
  fn remember_sound(&mut self, cx: &mut Context<Self>) {
    self.prefs.volume = self.volume;
    self.prefs.muted = self.muted;
    cx.emit(PlayerEvent::Sound {
      volume: self.volume,
      muted: self.muted,
    });
  }

  /// Entering a muted segment (by playing or seeking) says so, else silence looks like a fault.
  fn tell_muted(&mut self, cx: &mut Context<Self>) {
    let start = segments::muted_at(&self.segments, self.position).map(|s| s.start);
    if start.is_some() && start != self.muted_at {
      self.show_osd(
        t("Muted (copyright)", "Son coupé (droits d'auteur)").into(),
        cx,
      );
    }
    self.muted_at = start;
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
            || this.help
            || this.over_controls
            || this.volume_hover.1
            || this.volume_dragging
            || this.seek_dragging;
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

  fn seek_step(&self, large: bool) -> f64 {
    f64::from(if large {
      self.prefs.seek_step_large
    } else {
      self.prefs.seek_step
    })
  }

  fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    let keystroke = &event.keystroke;
    let shift = keystroke.modifiers.shift;
    if keystroke.key_char.as_deref() == Some("?") {
      self.help = !self.help;
      cx.stop_propagation();
      return self.show_controls(cx);
    }
    let page_layout = self.layout.is_page();
    let key = keystroke.key.as_str();
    // A live without its past has nowhere to seek to: those keys go on to the page.
    let seek_key = matches!(
      key,
      "left" | "j" | "right" | "l" | "pageup" | "pagedown" | "home"
    ) || (key.len() == 1 && key.bytes().all(|b| b.is_ascii_digit()));
    if seek_key && !self.can_seek() {
      return;
    }
    match key {
      // Plays or pauses on its release, unless held long enough for the 2× speed.
      "space" => {
        if !event.is_held && self.hold.is_none() {
          self.start_hold(Hold::Space, cx);
        }
      }
      "k" => self.toggle_pause(cx),
      "left" | "j" => self.seek_by(-self.seek_step(shift), cx),
      "right" | "l" => self.seek_by(self.seek_step(shift), cx),
      "pageup" => self.add_chapter(-1, cx),
      "pagedown" => self.add_chapter(1, cx),
      "home" => self.seek_to(self.seek_range().0, cx),
      key @ ("0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") if !shift => {
        let tenth = key.parse::<f64>().unwrap_or(0.0);
        self.seek_to(self.seek_time(tenth / 10.0), cx);
      }
      "up" => self.set_volume(self.volume + VOLUME_STEP, cx),
      "down" => self.set_volume(self.volume - VOLUME_STEP, cx),
      "m" => self.toggle_mute(cx),
      "a" => self.cycle_audio(cx),
      "c" if self.on_demand_subtitles() => self.toggle_captions(cx),
      "c" => self.cycle_subtitles(cx),
      "z" => self.add_subtitle_delay(-SUBTITLE_DELAY_STEP, cx),
      "x" => self.add_subtitle_delay(SUBTITLE_DELAY_STEP, cx),
      "s" => self.skip(cx),
      "n" if self.queue.is_some_and(|(_, next)| next) => self.play_next(cx),
      "p" if self.queue.is_some_and(|(previous, _)| previous) => cx.emit(PlayerEvent::Previous),
      "escape" if self.help || self.menu.is_some() || self.context_menu.is_some() => {
        self.help = false;
        self.menu = None;
        self.context_menu = None;
      }
      "t" if !window.is_fullscreen() && !page_layout => self.toggle_theater(cx),
      "f" => window.toggle_fullscreen(),
      "escape" if window.is_fullscreen() => window.toggle_fullscreen(),
      "escape" | "backspace" if page_layout => self.close(cx),
      _ => return,
    }
    cx.stop_propagation();
    self.show_controls(cx);
  }

  fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
    if event.keystroke.key == "space" && self.hold == Some(Hold::Space) {
      if !self.end_hold(cx) {
        self.toggle_pause(cx);
      }
      cx.stop_propagation();
    }
  }

  /// Left button down on the picture: a hold may start ([`Self::start_hold`]), unless the press
  /// closes a menu.
  fn on_video_mouse_down(&mut self, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
    self.hold_ended = false;
    let closing = self.menu.is_some() || self.help || self.context_menu.is_some();
    if !closing && self.hold.is_none() {
      self.start_hold(Hold::Mouse, cx);
    }
  }

  /// Right click on the picture: the menu with the shortcuts and the statistics, where the
  /// mouse is.
  fn on_right_click(
    &mut self,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    window.focus(&self.focus, cx);
    self.menu = None;
    self.context_menu = Some(event.position - self.bounds.get().origin);
    self.show_controls(cx);
  }

  /// Asks the page to switch theater mode.
  fn toggle_theater(&mut self, cx: &mut Context<Self>) {
    cx.emit(PlayerEvent::Theater);
  }

  /// Click on the picture: play/pause; double-click: fullscreen (undoing the first click's pause).
  /// A press held for the 2× speed is no click.
  fn on_video_click(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
    window.focus(&self.focus, cx);
    // Released before the window listened for the release: the hold ends here.
    let held =
      std::mem::take(&mut self.hold_ended) | (self.hold == Some(Hold::Mouse) && self.end_hold(cx));
    if held {
      return;
    }
    let closed = self.menu.take().is_some()
      | std::mem::take(&mut self.help)
      | self.context_menu.take().is_some();
    if closed {
      return cx.notify();
    }
    self.toggle_pause(cx);
    if event.click_count() == 2 {
      window.toggle_fullscreen();
    }
  }

  /// Mouse wheel: volume, in fullscreen or when the player is the page (embedded, the wheel
  /// scrolls the page).
  fn on_wheel(&mut self, event: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
    if !window.is_fullscreen() && self.layout == Layout::Embedded {
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
    let (start, end) = self.seek_range();
    (width > 0.0 && end > start)
      .then(|| (f32::from(x - bounds.origin.x) / width).clamp(0.0, 1.0) as f64)
  }

  /// The time at `fraction` of the seek bar.
  fn seek_time(&self, fraction: f64) -> f64 {
    let (start, end) = self.seek_range();
    start + fraction * (end - start)
  }

  // --- Rendering ----------------------------------------------------------------------------

  /// The preview tile at `seconds`, cropped out of its sheet, shown over the seek bar. A dark
  /// box while the sheet loads; `None` when the source has no previews.
  fn render_storyboard_frame(&self, seconds: f64, cx: &mut Context<Self>) -> Option<AnyElement> {
    let previews = self.source.as_ref()?.previews.clone()?;
    let frame = previews.tile(seconds, self.duration)?;
    let (tile_width, tile_height) = previews.tile_size();
    let scale = PREVIEW_WIDTH / tile_width.max(1) as f32;
    let height = tile_height as f32 * scale;
    let sheet = cx
      .try_global::<Images>()
      .cloned()
      .and_then(|images| images.0.update(cx, |store, cx| store.get(&frame.url, cx)));
    // Sized from the sheet itself: the last one is cut after its last frame.
    let image = sheet.map(|sheet| {
      let size = sheet.size(0);
      img(ImageSource::Render(sheet))
        .absolute()
        .left(px(-(frame.column as f32) * PREVIEW_WIDTH))
        .top(px(-(frame.row as f32) * height))
        .w(px(size.width.0 as f32 * scale))
        .h(px(size.height.0 as f32 * scale))
        .object_fit(ObjectFit::Fill)
    });
    Some(
      div()
        .p(px(2.))
        .rounded_md()
        .bg(gpui::white())
        .child(
          div()
            .relative()
            .w(px(PREVIEW_WIDTH))
            .h(px(height))
            .overflow_hidden()
            .bg(gpui::black())
            .children(image),
        )
        .into_any_element(),
    )
  }

  fn render_seek_bar(
    &mut self,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> impl IntoElement + use<> {
    let primary = cx.theme().primary;
    let (start, end) = self.seek_range();
    let progress = if end > start {
      ((self.position - start) / (end - start)).clamp(0.0, 1.0)
    } else {
      0.0
    };
    let cached = if end > start {
      ((self.cached - start) / (end - start)).clamp(0.0, 1.0)
    } else {
      0.0
    };
    let dvr = self.dvr_window().is_some();
    // The track thickens on hover, eased rather than at once.
    let height = motion::tween(
      "seek-height",
      4.,
      if self.seek_hover.is_some() { 6. } else { 4. },
      theme::FAST,
      window,
      cx,
    );
    // GPUI clips children to rectangles, so each fill carries its piece's rounding and is
    // never narrower than tall (nor wider than a short piece), or its square ends stick out.
    let fill = |fraction: f64, height: f32, color: gpui::Hsla| {
      div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .w(relative(fraction as f32))
        .min_w(px(height))
        .max_w(relative(1.))
        .rounded(px(height / 2.))
        .bg(color)
    };
    let bounds_cell = self.seek_bounds.clone();
    let (dragging, view) = (self.seek_dragging, cx.weak_entity());
    let preview = self.seek_hover.map(|fraction| {
      let seconds = self.seek_time(fraction);
      let bar_width = f32::from(self.seek_bounds.get().size.width);
      let thumbnail = self.render_storyboard_frame(seconds, cx);
      let index = self.chapter_at(seconds);
      let chapter = index.and_then(|i| self.chapters[i].title.clone());
      // A chapter's name needs the room of a thumbnail, even without one.
      let width = if thumbnail.is_some() || chapter.is_some() {
        PREVIEW_WIDTH + 4.
      } else {
        72.
      };
      let left = (bar_width * fraction as f32 - width / 2.).clamp(0., (bar_width - width).max(0.));
      let chapter = chapter.map(|title| {
        let picture = index.and_then(|i| self.chapter_picture(i)).map(|p| {
          div()
            .flex_shrink_0()
            .w(px(HOVER_PICTURE_WIDTH))
            .h(px(HOVER_PICTURE_WIDTH / chapter_list::PICTURE_ASPECT))
            .child(picture::image_fill(
              p,
              HOVER_PICTURE_WIDTH,
              chapter_list::PICTURE_ASPECT,
              px(2.),
              window,
              cx,
            ))
        });
        h_flex()
          .max_w_full()
          .gap_1p5()
          .justify_center()
          .children(picture)
          .child(
            div()
              .min_w_0()
              .text_xs()
              .text_center()
              .opacity(0.8)
              .child(title),
          )
      });
      let muted = segments::muted_at(&self.segments, seconds).is_some();
      v_flex()
        .absolute()
        .bottom(px(22.))
        .left(px(left))
        .w(px(width))
        .items_center()
        .gap_1()
        .children(thumbnail)
        .child(
          v_flex()
            .max_w_full()
            .items_center()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(gpui::black().opacity(0.8))
            .text_color(gpui::white())
            .child(div().text_sm().child(if dvr {
              // A live's past is told by how far behind the edge it is, as YouTube does.
              format!("-{}", format_time(end - seconds))
            } else {
              format_time(seconds)
            }))
            .children(chapter)
            .when(muted, |el| {
              el.child(
                div()
                  .text_xs()
                  .text_color(theme::muted_audio())
                  .child(t("Muted", "Son coupé")),
              )
            }),
        )
    });
    // One piece of track per chapter, apart by a gap as on YouTube; the hovered one is
    // thicker. Without chapters, a single piece.
    let span = end - start;
    let mut edges = vec![0.0];
    for chapter in &self.chapters {
      let edge = (chapter.start - start) / span;
      if span > 0.0 && chapter.start - start > 1.0 && edge < 1.0 && edge > edges[edges.len() - 1] {
        edges.push(edge);
      }
    }
    // Muted parts, as fractions of the bar.
    let muted: Vec<(f64, f64)> = self
      .segments
      .iter()
      .filter(|s| s.kind == SegmentKind::Muted && span > 0.0)
      .map(|s| ((s.start - start) / span, (s.end - start) / span))
      .filter(|&(from, to)| to > 0.0 && from < 1.0)
      .collect();
    let hover = self.seek_hover;
    let pieces = (0..edges.len())
      .map(|i| {
        let (from, to) = (edges[i], edges.get(i + 1).copied().unwrap_or(1.0));
        let hovered = edges.len() > 1 && hover.is_some_and(|f| from <= f && f < to);
        let grown = motion::tween(
          ("seek-piece", i),
          0.,
          if hovered { 4. } else { 0. },
          theme::FAST,
          window,
          cx,
        );
        let height = height + grown;
        let local = |fraction: f64| ((fraction - from) / (to - from)).clamp(0.0, 1.0);
        // Red over the track, bright where played (as the progress is) and dim ahead. A band
        // that reaches an end of the piece takes its rounding there, square elsewhere.
        let band = |a: f64, b: f64, color: gpui::Hsla| {
          let radius = px(height / 2.);
          div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(relative(a as f32))
            .w(relative((b - a) as f32))
            .when(a <= 0.0, |el| el.rounded_l(radius).min_w(radius))
            .when(b >= 1.0, |el| el.rounded_r(radius).min_w(radius))
            .bg(color)
        };
        let played = local(progress);
        let bands = muted
          .iter()
          .map(|&(a, b)| (local(a), local(b)))
          .filter(|(a, b)| b > a)
          .flat_map(|(a, b)| {
            let ahead =
              (b > played).then(|| band(a.max(played), b, theme::muted_audio().opacity(0.6)));
            let behind = (played > a).then(|| band(a, b.min(played), theme::muted_audio()));
            ahead.into_iter().chain(behind)
          })
          .collect::<Vec<_>>();
        div()
          .absolute()
          .top_0()
          .bottom_0()
          .left(relative(from as f32))
          .w(relative((to - from) as f32))
          .when(i + 1 < edges.len(), |el| el.pr(px(CHAPTER_GAP)))
          .flex()
          .items_center()
          .child(
            div()
              .relative()
              .w_full()
              .h(px(height))
              .rounded(px(height / 2.))
              .bg(gpui::white().opacity(0.25))
              // What is loaded ahead of the playhead, under the hover's fill.
              .when(local(cached) > played, |el| {
                el.child(fill(local(cached), height, gpui::white().opacity(0.25)))
              })
              .children(
                hover
                  .map(local)
                  .filter(|&f| f > 0.0)
                  .map(|f| fill(f, height, gpui::white().opacity(0.25))),
              )
              .when(played > 0.0, |el| el.child(fill(played, height, primary)))
              .children(bands),
          )
      })
      .collect::<Vec<_>>();

    div()
      .id("seek")
      .relative()
      .w_full()
      .py_2()
      .cursor_pointer()
      .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
        // During a drag the window listener moves the playhead (and the preview with it).
        if !this.seek_dragging {
          this.seek_hover = this.seek_fraction(event.position.x);
          cx.notify();
        }
      }))
      .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
        if !hovered && !this.seek_dragging {
          this.seek_hover = None;
          cx.notify();
        }
      }))
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, event: &MouseDownEvent, _, cx| {
          this.seek_dragging = this.seek_fraction(event.position.x).is_some();
          this.seek_at(event.position.x, cx);
        }),
      )
      .child(
        div()
          .relative()
          .h(px(height))
          .w_full()
          .children(pieces)
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
            canvas(
              move |bounds, _, _| bounds_cell.set(bounds),
              move |_, _, window, _| {
                if dragging {
                  listen_to_drag(
                    &view,
                    window,
                    |this, position, cx| this.seek_at(position.x, cx),
                    PlayerView::end_seek_drag,
                  );
                }
              },
            )
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
                  listen_to_drag(
                    &view,
                    window,
                    |this, position, cx| this.set_volume_at(position.y, cx),
                    |this, _, cx| this.end_volume_drag(cx),
                  );
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
          .tooltip(t("Mute (M)", "Couper le son (M)"))
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
    let theater = self.theater;
    let white = |opacity: f32| gpui::white().opacity(opacity);
    let has_captions =
      self.on_demand_subtitles() || !self.tracks_of(player::TrackKind::Subtitle).is_empty();
    let has_audio_choice = self.tracks_of(player::TrackKind::Audio).len() > 1;
    let has_qualities = self.source.as_ref().is_some_and(|s| s.qualities.len() > 1);
    let embedded = self.layout == Layout::Embedded;
    let live = self.is_live();
    let can_seek = self.can_seek();
    let step = self.prefs.seek_step;
    let mut layers = Vec::new();

    if !embedded {
      layers.push(self.render_top_bar(window, cx));
    } else if fullscreen && !self.title.is_empty() {
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
            .tooltip(t("Previous (P)", "Précédent (P)"))
            .disabled(!previous)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(PlayerEvent::Previous))),
        )
      })
      .child(
        self
          .control_button("play-pause", cx)
          .icon(if self.paused { Icon::Play } else { Icon::Pause })
          .tooltip(if self.paused {
            t("Play (K)", "Lecture (K)")
          } else {
            t("Pause (K)", "Pause (K)")
          })
          .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_pause(cx);
            this.show_controls(cx);
          })),
      )
      .when_some(self.queue, |row, (_, next)| {
        row.child(
          self
            .control_button("next", cx)
            .icon(Icon::Next)
            .tooltip(t("Next (N)", "Suivant (N)"))
            .disabled(!next)
            .on_click(cx.listener(|this, _, _, cx| this.play_next(cx))),
        )
      })
      .when(can_seek, |row| {
        row
          .child(
            self
              .control_button("back-10", cx)
              .icon(Icon::Back10)
              .tooltip(tf!("Back {} (←)", "Reculer de {} (←)", format_step(step)))
              .on_click(cx.listener(|this, _, _, cx| this.seek_by(-this.seek_step(false), cx))),
          )
          .child(
            self
              .control_button("fwd-10", cx)
              .icon(Icon::Forward10)
              .tooltip(tf!(
                "Forward {} (→)",
                "Avancer de {} (→)",
                format_step(step)
              ))
              .on_click(cx.listener(|this, _, _, cx| this.seek_by(this.seek_step(false), cx))),
          )
      })
      .child(self.render_volume(cx))
      .map(|row| {
        if live {
          row.child(self.render_live_badge(cx))
        } else {
          row.child(div().pl_2().text_sm().text_color(white(0.9)).child(format!(
            "{} / {}",
            format_time(self.position),
            format_time(self.duration)
          )))
        }
      })
      .child(div().flex_1())
      .when(!embedded && !live && self.duration > 0.0, |row| {
        row.child(
          div()
            .pr_2()
            .text_sm()
            .text_color(white(0.85))
            .child(format!("-{}", format_time(self.duration - self.position))),
        )
      })
      .when(!self.chapters.is_empty(), |row| {
        row.child(
          self
            .control_button("chapters-menu", cx)
            .icon(Icon::Chapters)
            .tooltip(t(
              "Chapters (Page Up / Page Down)",
              "Chapitres (Page préc. / Page suiv.)",
            ))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Chapters, cx))),
        )
      })
      .when(has_audio_choice, |row| {
        row.child(
          self
            .control_button("audio-menu", cx)
            .icon(Icon::Audio)
            .tooltip(t("Audio (A)", "Audio (A)"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Audio, cx))),
        )
      })
      .when(has_captions, |row| {
        row.child(
          self
            .control_button("subs-menu", cx)
            .icon(Icon::Subtitles)
            .tooltip(t("Subtitles (C)", "Sous-titres (C)"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Subtitles, cx))),
        )
      })
      .when(!self.is_live(), |row| {
        row.child(
          self
            .control_button("speed-menu", cx)
            .icon(Icon::Speed)
            .tooltip(t("Speed", "Vitesse"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Speed, cx))),
        )
      })
      .when(has_qualities, |row| {
        row.child(
          self
            .control_button("quality-menu", cx)
            .icon(Icon::Settings)
            .tooltip(t("Quality", "Qualité"))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(Menu::Quality, cx))),
        )
      })
      // In fullscreen the picture fills the window anyway.
      .when(!fullscreen && embedded, |row| {
        row.child(
          self
            .control_button("theater", cx)
            .icon(Icon::Theater)
            .tooltip(if theater {
              t("Exit theater mode (T)", "Quitter le mode cinéma (T)")
            } else {
              t("Theater mode (T)", "Mode cinéma (T)")
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
            t("Exit full screen (F)", "Quitter le plein écran (F)")
          } else {
            t("Full screen (F)", "Plein écran (F)")
          })
          .on_click(|_, window, _| window.toggle_fullscreen()),
      );

    let seek_bar = can_seek.then(|| self.render_seek_bar(window, cx));
    layers.push(
      v_flex()
        .id("bottom-bar")
        // Clicks stop here, the wheel goes on: it scrolls the page under an embedded player.
        .block_mouse_except_scroll()
        .on_hover(cx.listener(|this, hovered: &bool, _, _| this.over_controls = *hovered))
        .on_mouse_move(cx.listener(|this, _, _, cx| this.show_controls(cx)))
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .px_4()
        .pt_8()
        .pb_2()
        // Its gradient reaches the player's bottom corners: rounded like them.
        .rounded_bl(corner_radii(window, self.theater).bottom_left)
        .rounded_br(corner_radii(window, self.theater).bottom_right)
        .bg(linear_gradient(
          180.,
          linear_color_stop(gpui::black().opacity(0.0), 0.0),
          linear_color_stop(gpui::black().opacity(0.8), 1.0),
        ))
        .children(seek_bar)
        .child(buttons)
        .into_any_element(),
    );
    layers
  }

  /// "LIVE" on the accent, then the time since the broadcast started and the viewers, when
  /// the page gave them ([`Self::set_live_info`]).
  fn render_live_badge(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
    let primary = cx.theme().primary;
    let info = self.live_info.clone().unwrap_or_default();
    let uptime = info
      .started_at
      .and_then(|start| std::time::SystemTime::now().duration_since(start).ok())
      .map(|elapsed| format_time(elapsed.as_secs_f64()));
    let viewers = info.viewers.map(|n| {
      let count = group_thousands(n);
      // French's singular covers 0 and 1, English's only 1.
      match (lang(), n) {
        (Lang::En, 1) | (Lang::Fr, 0 | 1) => tf!("{count} viewer", "{count} spectateur"),
        _ => tf!("{count} viewers", "{count} spectateurs"),
      }
    });
    let dvr = self.dvr_window().is_some();
    let behind = (dvr && !self.at_live_edge()).then(|| self.behind_live());
    let badge = h_flex()
      .id("live-badge")
      .gap_1()
      .items_center()
      .px_2()
      .py(px(2.))
      .rounded_sm()
      .bg(if behind.is_some() {
        gpui::white().opacity(0.2)
      } else {
        primary
      })
      .text_xs()
      .font_weight(FontWeight::BOLD)
      .text_color(gpui::white())
      .child(div().size(px(6.)).rounded_full().bg(if behind.is_some() {
        gpui::white().opacity(0.5)
      } else {
        gpui::white()
      }))
      .child(t("LIVE", "EN DIRECT"))
      .when(dvr, |badge| {
        badge
          .cursor_pointer()
          .hover(|s| s.bg(primary))
          .tooltip(|window, cx| {
            Tooltip::new(t("Go to live", "Revenir au direct")).build(window, cx)
          })
          .on_click(cx.listener(|this, _, _, cx| {
            this.go_live(cx);
            this.show_controls(cx);
          }))
      });
    h_flex()
      .pl_2()
      .gap_2()
      .items_center()
      .text_sm()
      .text_color(gpui::white().opacity(0.9))
      .child(badge)
      .children(behind.map(|b| format!("-{}", format_time(b))))
      .children(uptime)
      .children(viewers.map(|v| div().text_color(gpui::white().opacity(0.75)).child(v)))
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

type DragHandler = fn(&mut PlayerView, Point<Pixels>, &mut Context<PlayerView>);

/// Follows a left-button drag (volume slider, seek bar) over the whole window until the release.
/// Registered at each paint while the drag lasts, as GPUI drops window listeners every frame.
fn listen_to_drag(
  view: &WeakEntity<PlayerView>,
  window: &mut Window,
  on_move: DragHandler,
  on_end: DragHandler,
) {
  let move_view = view.clone();
  window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
    if phase == DispatchPhase::Capture {
      move_view
        .update(cx, |this, cx| {
          if event.pressed_button == Some(MouseButton::Left) {
            on_move(this, event.position, cx);
          } else {
            // The button was released where no mouse up reached the window.
            on_end(this, event.position, cx);
          }
        })
        .ok();
    }
  });
  let up_view = view.clone();
  window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
      up_view
        .update(cx, |this, cx| on_end(this, event.position, cx))
        .ok();
    }
  });
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

/// Radii of the player's corners: rounded in the page like the other cards, square in
/// fullscreen. In theater mode the player fills the content panel's rounded top-left corner,
/// so it takes that corner's inner radius. A parent's `overflow_hidden` clips to a
/// rectangle, so every layer reaching a corner rounds itself.
fn corner_radii(window: &Window, theater: bool) -> Corners<Pixels> {
  if window.is_fullscreen() {
    Corners::default()
  } else if theater {
    Corners {
      top_left: theme::PANEL_INNER_RADIUS,
      ..Corners::default()
    }
  } else {
    Corners::all(theme::RADIUS_LG)
  }
}

/// `1.25×` in English, `1,25×` in French; `Normal` for 1×.
fn speed_label(speed: f64) -> String {
  if speed == 1.0 {
    return t("Normal", "Normale").to_string();
  }
  let number = format!("{speed}");
  let number = match lang() {
    Lang::Fr => number.replace('.', ","),
    Lang::En => number,
  };
  format!("{number}×")
}

/// `12,345` in English, `12 345` in French (with a narrow no-break space).
fn group_thousands(n: u64) -> String {
  let separator = lang().pick(',', '\u{202f}');
  let digits = n.to_string();
  let mut out = String::with_capacity(digits.len() + digits.len() / 3 * 3);
  for (i, digit) in digits.chars().enumerate() {
    if i > 0 && (digits.len() - i).is_multiple_of(3) {
      out.push(separator);
    }
    out.push(digit);
  }
  out
}

/// `1:02:03` or `12:34`.
pub fn format_time(seconds: f64) -> String {
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
    // A page of its own fills the panel like theater mode.
    let radii = corner_radii(window, self.theater || self.layout == Layout::Page);
    if let Some(video) = &self.video {
      video.update(cx, |video, _| video.set_corner_radii(radii));
    }

    let mut root = div()
      .id("player")
      .size_full()
      .relative()
      .rounded_tl(radii.top_left)
      .rounded_tr(radii.top_right)
      .rounded_br(radii.bottom_right)
      .rounded_bl(radii.bottom_left)
      .bg(gpui::black())
      .track_focus(&self.focus)
      .on_key_down(cx.listener(Self::on_key))
      .on_key_up(cx.listener(Self::on_key_up))
      .child({
        let bounds = self.bounds.clone();
        let (mouse_hold, view) = (self.hold == Some(Hold::Mouse), cx.weak_entity());
        canvas(
          move |b, _, _| bounds.set(b),
          move |_, _, window, _| {
            // The release of a left button held on the picture, wherever it happens.
            if mouse_hold {
              listen_to_drag(&view, window, |_, _, _| {}, PlayerView::end_mouse_hold);
            }
          },
        )
        .absolute()
        .inset_0()
      })
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
          .on_mouse_down(MouseButton::Left, cx.listener(Self::on_video_mouse_down))
          .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_click))
          .on_scroll(cx.listener(Self::on_wheel))
          .on_mouse_move(cx.listener(|this, _, _, cx| this.show_controls(cx))),
      );

    // The page's notice says why the picture waits better than "loading".
    let status = if self.failed {
      None
    } else {
      self
        .notice
        .clone()
        .or_else(|| self.status.clone())
        .or_else(|| self.buffering.then(|| t("Loading…", "Chargement…").into()))
    };
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
    // No key up comes once the focus or the window is gone: the 2× speed ends there.
    if self.hold == Some(Hold::Space)
      && !(self.focus.is_focused(window) && window.is_window_active())
    {
      self.end_hold(cx);
    }
    let osd = if self.fast {
      Some(t("2× speed", "Vitesse 2×").into())
    } else {
      self.osd.clone()
    };
    if let Some(osd) = osd {
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
    if self.failed {
      let error = self.status.clone().unwrap_or_default();
      if self.layout.is_page() {
        root = root.child(self.render_top_bar(window, cx));
      }
      return root.child(self.render_error(error, cx));
    }
    // Skip and "up next" stay visible when the other controls hide.
    let up_next = self.render_up_next(cx);
    let skip = if up_next.is_none() {
      self.render_skip(show_ui, cx)
    } else {
      None
    };
    root = root.children(skip).children(up_next);
    if show_ui {
      root = root.children(self.render_controls(window, cx));
      if let Some(menu) = self.menu {
        root = root.child(self.render_menu(menu, window, cx));
      }
    }
    if self.stats {
      root = root.child(self.render_stats(cx));
    }
    if self.help {
      root = root.child(self.render_help(cx));
    }
    if let Some(at) = self.context_menu {
      root = root.child(self.render_context_menu(at, cx));
    }
    root
  }
}

#[cfg(test)]
mod tests {
  use super::{group_thousands, speed_label};

  #[test]
  fn thousands() {
    assert_eq!(group_thousands(7), "7");
    assert_eq!(group_thousands(999), "999");
    // Tests see the default language, English.
    assert_eq!(group_thousands(12_345), "12,345");
    assert_eq!(group_thousands(1_234_567), "1,234,567");
  }

  #[test]
  fn speeds() {
    assert_eq!(speed_label(1.0), "Normal");
    assert_eq!(speed_label(0.75), "0.75×");
    assert_eq!(speed_label(2.0), "2×");
  }
}
