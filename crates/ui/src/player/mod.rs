//! The video player shared by every space: picture, auto-hiding controls (seek bar with
//! previews and chapters, volume, buttons, quality, audio, subtitle and chapter menus), skip
//! button, "up next" card, OSD, shortcut help and keyboard shortcuts. It plays a [`Source`] and
//! tells its page what happens through [`PlayerEvent`]s; it reads no setting itself. Embedded in
//! a page ([`Layout::Embedded`], YouTube), filling a page of its own ([`Layout::Page`], Jellyfin)
//! or the window ([`Layout::Window`], the example); fills the window in fullscreen. Controls and
//! behavior come from peeby's player page.

mod catch_up;
pub mod chapter_list;
mod overlays;
mod running;
pub mod segments;
mod source;
mod stats;
mod tracks;

pub use overlays::format_step;
pub use running::{Owner, RunningPlayer, running};
pub use source::{Author, LIVE_DELAY, LiveInfo, Previews, Quality, Rewind, Source, Subtitle, Tile};

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, App, Bounds, ClickEvent, Context, Corners, DispatchPhase,
  Div, Entity, EventEmitter, FocusHandle, FontWeight, Global, ImageSource, KeyDownEvent,
  KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Point,
  ScrollEvent, SharedString, Stateful, Task, WeakEntity, Window, WindowId, canvas, div,
  ease_out_quint, img, linear_color_stop, linear_gradient, px, relative,
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

const HIDE_CONTROLS_AFTER: Duration = Duration::from_secs(1);
/// The controls fade in and out over this long.
const CONTROLS_FADE: Duration = Duration::from_millis(200);
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
/// A recorded live's recording ends a few seconds to a segment behind the live ([`Rewind`]): a
/// seek closer than this to the edge joins the live, and one back from the live starts the
/// recording at least this far behind.
const REWIND_EDGE: f64 = 30.0;
/// A live that starts at its edge by itself (Twitch): a seek closer than this to the edge joins
/// the edge with a restart, low latency included, rather than a seek mpv would make wait.
const EDGE_SNAP: f64 = 3.0;
/// Seconds kept short of the oldest segment of a live's own window when seeking in it: it
/// slides on while mpv seeks.
const WINDOW_MARGIN: f64 = 5.0;
/// mpv reporting the end further than this from the duration: a stream stopped short (its
/// segments stopped coming), not the end of the video.
const EARLY_END_MARGIN: f64 = 10.0;
/// A second early end or stall within this time of a reload is not reloaded again: an early
/// end fails, a stall waits.
const RELOAD_RETRY: Duration = Duration::from_secs(60);
/// A seekable video still buffering after this is stuck, not slow: a failed transfer (often
/// right after a pause, on a connection the server closed) can leave mpv's video demuxer
/// reading ahead without end while nothing more is decoded. A new mpv from the same position
/// plays again.
const STALL_AFTER: Duration = Duration::from_secs(10);
const VOLUME_FADE: Duration = Duration::from_millis(150);
const VOLUME_STEP: f64 = 5.0;
/// The wheel's step over the volume slider, finer than the picture's.
const VOLUME_SLIDER_STEP: f64 = 2.0;
/// Width of the storyboard frame shown over the seek bar.
const PREVIEW_WIDTH: f32 = 176.;
/// Width of a chapter's picture in the seek bar's hover preview.
const HOVER_PICTURE_WIDTH: f32 = 16.;

/// Space between two chapters on the seek bar.
const CHAPTER_GAP: f32 = 2.;
const CONTROL_SIZE: f32 = 40.;
const CONTROL_ICON: f32 = 20.;

/// A button the page adds at the embedded player's top right, among the controls: it shows and
/// fades with them, and keeps them up while hovered (the theater mode's "Open chat").
#[derive(Clone)]
pub struct CornerAction {
  /// Its icon.
  pub icon: Icon,
  /// Its tooltip, in the viewer's language.
  pub tooltip: SharedString,
  /// What a click does.
  pub on_click: Rc<CornerClick>,
}

/// What a [`CornerAction`]'s click runs.
pub type CornerClick = dyn Fn(&mut Window, &mut App);

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
  /// A recorded live ([`Rewind`]) moved between the live and its recording: `rewound` while the
  /// recording plays; `at` is the recording's position, where it starts or where it was left.
  Rewound {
    /// The recording plays now.
    rewound: bool,
    /// Seconds into the recording.
    at: f64,
  },
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

/// The player that started last in each window, as a global the shells observe (a window's
/// mini-player shows only its window's). One sound at a time per window: videos in different
/// windows play together.
#[derive(Default)]
pub struct NowPlaying(Vec<(Option<WindowId>, WeakEntity<PlayerView>)>);

impl Global for NowPlaying {}

/// The player that started last in `window` (the window its page shows in), while it exists.
pub fn now_playing(window: WindowId, cx: &App) -> Option<Entity<PlayerView>> {
  cx.try_global::<NowPlaying>()?
    .0
    .iter()
    .find(|(w, _)| *w == Some(window))?
    .1
    .upgrade()
}

/// `player` is the one that started last in its window.
pub fn is_now_playing(player: &Entity<PlayerView>, cx: &App) -> bool {
  cx.try_global::<NowPlaying>()
    .is_some_and(|playing| playing.0.iter().any(|(_, p)| p == &player.downgrade()))
}

/// A player's picture-in-picture button (or I): the shell showing its page opens the
/// mini-player on it, or closes it (`open: false`), and tells it with
/// [`PlayerView::set_picture_in_picture`].
#[derive(Default)]
pub struct PictureInPicture(Option<(WeakEntity<PlayerView>, bool)>);

impl Global for PictureInPicture {}

impl PictureInPicture {
  /// The player asking, while it exists, and whether it asks to open the mini-player.
  pub fn request(&self) -> Option<(Entity<PlayerView>, bool)> {
    let (player, open) = self.0.as_ref()?;
    Some((player.upgrade()?, *open))
  }
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
  /// The page offers theater mode (button and T); not a page whose player is vertical.
  theater_offered: bool,
  /// The page's button at the top right ([`PlayerView::set_corner_action`]).
  corner_action: Option<CornerAction>,
  /// The page offers picture-in-picture (button and I); not a page of short looping videos.
  pip_page_offered: bool,
  /// The bar shows the seek buttons (back and forward a step); the keys seek either way.
  seek_buttons: bool,
  /// The bar shows the time (position / length, the time left).
  time_shown: bool,
  /// The page offers speed changes (the menu, and 2× while held); not a page of short looping
  /// videos.
  speed_offered: bool,
  layout: Layout,
  /// The mini-player shows the picture (picture-in-picture): the page shows a placeholder.
  in_pip: bool,
  /// The window the page shows in, from the last render (the mini-player draws only the
  /// picture, never this view): its other players pause when this one starts.
  window: Option<WindowId>,
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
  /// The channel shown with the title over the picture.
  author: Option<Author>,
  player: Option<Rc<Player>>,
  video: Option<Entity<VideoView>>,
  /// Why the video cannot play, shown instead of the picture (a spinner while it loads).
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
  /// The boost button beside the volume slider is on: the volume goes up to
  /// [`player::VOLUME_MAX`] (mpv amplifies past 100). Never remembered: each player starts
  /// without it.
  boosted: bool,
  muted: bool,
  /// Another page is on top: the player stays paused, even if it starts after being covered.
  covered: bool,
  /// A multistream's secondary picture ([`PlayerView::set_secondary`]): always muted, it never
  /// takes over its window and draws only its picture.
  secondary: bool,
  osd: Option<SharedString>,
  osd_task: Option<Task<()>>,
  controls_visible: bool,
  /// Counts the controls' show/hide changes, so each one starts a new fade.
  controls_animation: u32,
  /// Removes the controls once their fade-out is over; they are drawn until then.
  controls_fade_task: Option<Task<()>>,
  over_controls: bool,
  /// The mouse is over the mute button, or over the volume slider beside it.
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
  /// A live's recording, for its past ([`Self::set_rewind`]), and the task moving its edge.
  rewind: Option<Rewind>,
  rewind_tick: Option<Task<()>>,
  /// The recording plays, not the live.
  rewound: bool,
  /// A recorded live's own mpv position and duration while the live plays (its timeline, not
  /// the recording's): how far behind its edge it is, and how far back its window goes.
  live_mpv: (f64, f64),
  /// Back from the recording to `n` seconds behind the edge: the live starts at its edge, then
  /// seeks there once its duration is known.
  live_seek_back: Option<f64>,
  /// A drag of the seek bar that would move between the live and the recording: where the
  /// playhead was, the move done on release (each would restart mpv).
  rewind_drag_from: Option<f64>,
  /// A message over the picture from the page, e.g. why it waits (an ad removed from a live).
  notice: Option<SharedString>,
  /// When the stream was last reloaded after ending early ([`EARLY_END_MARGIN`]) or stalling
  /// ([`STALL_AFTER`]).
  last_reload: Option<Instant>,
  /// Reloads the stream if buffering lasts [`STALL_AFTER`].
  stall_task: Option<Task<()>>,
  /// A live's speed kept close to its edge ([`Source::catch_up`]).
  catch_up: catch_up::CatchUp,
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
      theater_offered: true,
      corner_action: None,
      speed_offered: true,
      pip_page_offered: true,
      seek_buttons: true,
      time_shown: true,
      layout: Layout::Embedded,
      in_pip: false,
      window: None,
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
      author: None,
      player: None,
      video: None,
      status: None,
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
      volume: prefs.volume.clamp(0.0, 100.0),
      boosted: false,
      muted: prefs.muted,
      covered: false,
      secondary: false,
      osd: None,
      osd_task: None,
      controls_visible: true,
      controls_animation: 0,
      controls_fade_task: None,
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
      rewind: None,
      rewind_tick: None,
      rewound: false,
      rewind_drag_from: None,
      live_mpv: (0.0, 0.0),
      live_seek_back: None,
      notice: None,
      last_reload: None,
      stall_task: None,
      catch_up: Default::default(),
      _events: None,
    }
  }

  /// Whether the source is a live stream ([`Source::live`]).
  pub fn is_live(&self) -> bool {
    self.source.as_ref().is_some_and(|s| s.live)
  }

  /// How far back the live can be watched, if it keeps its past ([`Source::dvr_window`]): a
  /// recorded live's whole broadcast ([`Rewind`]).
  fn dvr_window(&self) -> Option<f64> {
    if self.rewind.is_some() && self.is_live() {
      return Some(self.duration.max(0.0));
    }
    self
      .source
      .as_ref()
      .filter(|s| s.live)
      .and_then(|s| s.dvr_window)
  }

  /// Whether a recorded live's recording plays rather than the live ([`Rewind`]).
  pub fn rewound(&self) -> bool {
    self.rewound
  }

  /// The live's recording, once the space has it: the seek bar covers the broadcast from then
  /// on, the live's own window ([`Source::dvr_window`], Twitch's last minutes) still sought in
  /// place. Nothing for a source that is not a live.
  pub fn set_rewind(&mut self, rewind: Rewind, cx: &mut Context<Self>) {
    if !self.is_live() {
      return;
    }
    // From now on the duration is the recording's edge; mpv's live duration so far is the
    // live's.
    self.live_mpv = (self.position, self.duration);
    self.rewind = Some(rewind);
    self.sync_rewind();
    self.rewind_tick = Some(cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        let alive = this.update(cx, |this, cx| {
          this.sync_rewind();
          if this.controls_visible {
            cx.notify();
          }
        });
        if alive.is_err() {
          return;
        }
      }
    }));
    cx.notify();
  }

  /// A recorded live's edge, seconds since its recording started.
  fn rewind_edge(&self) -> Option<f64> {
    let rewind = self.rewind.as_ref().filter(|_| self.is_live())?;
    Some(
      std::time::SystemTime::now()
        .duration_since(rewind.started_at)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64()),
    )
  }

  /// A recorded live's timeline: its end is now; on the live, the playhead is as far behind it
  /// as mpv is behind the live's edge (which grows while paused).
  fn sync_rewind(&mut self) {
    let Some(edge) = self.rewind_edge() else {
      return;
    };
    self.duration = edge;
    if !self.rewound && !self.seek_dragging {
      let (position, duration) = self.live_mpv;
      self.position = edge - (duration - position).max(0.0);
    }
  }

  /// mpv starts this live at its edge by itself (Twitch, a `start` of 0): a restart joins the
  /// edge with its low latency, where a seek stops short of it; and a new mpv (another quality)
  /// gets a window only from then on.
  fn starts_at_edge(&self) -> bool {
    self.is_live() && self.live_start() <= 0.0
  }

  /// How far back a recorded live's own window goes while the live plays, seconds: what its
  /// playlist keeps ([`Source::dvr_window`]) of what this mpv has read.
  fn live_back(&self) -> f64 {
    let kept = self
      .source
      .as_ref()
      .and_then(|s| s.dvr_window)
      .unwrap_or(0.0);
    self.live_mpv.1.min(kept)
  }

  /// Plays a recorded live's recording from `at` seconds, at the quality of the live's label.
  fn play_recording(&mut self, at: f64, cx: &mut Context<Self>) {
    let Some(rewind) = &self.rewind else {
      return;
    };
    let recording = &rewind.recording;
    let label = self
      .quality
      .and_then(|i| self.source.as_ref()?.qualities.get(i))
      .map(|q| q.label.clone());
    let options = label
      .and_then(|label| recording.qualities.iter().find(|q| q.label == label))
      .map_or_else(|| recording.options.clone(), |q| q.options.clone());
    // At least 1 s: 0 gives mpv no `start`, and a recording in progress starts at its end then.
    let at = at.max(1.0);
    let options = StartOptions {
      start_seconds: at,
      ..options
    };
    tracing::info!("recorded live: its recording from {at:.0} s");
    if let Some(old) = self.player.take() {
      old.set_paused(true);
    }
    self.captions_added.clear();
    self.rewound = true;
    self.position = at;
    self.paused_since = None;
    cx.emit(PlayerEvent::Rewound { rewound: true, at });
    self.start_with(options, Instant::now(), cx);
  }

  /// Where a recorded live's seek to `target` goes: in what plays (`Some`, the second of its
  /// mpv's timeline), or `None` when it leaves it (from the recording to the live, from the live
  /// into the recording, or the live's edge).
  fn rewind_native(&self, target: f64, edge: f64) -> Option<f64> {
    let behind = edge - target;
    if self.rewound {
      (behind > REWIND_EDGE).then_some(target)
    } else {
      (behind >= EDGE_SNAP && behind <= self.live_back() - WINDOW_MARGIN)
        .then_some(self.live_mpv.1 - behind)
    }
  }

  /// A recorded live's seek to `target` that leaves what plays: from the recording to the live
  /// (at its edge, then that far behind), from the live into the recording, or to the live's
  /// edge.
  fn switch_rewind(&mut self, target: f64, edge: f64, cx: &mut Context<Self>) {
    let behind = edge - target;
    if self.rewound {
      self.play_live(cx);
      if behind >= EDGE_SNAP {
        self.live_seek_back = Some(behind);
      }
    } else if behind >= EDGE_SNAP {
      self.play_recording(target.min(edge - REWIND_EDGE), cx);
    } else if self.paused || !self.at_live_edge() {
      self.play_live(cx);
    } else {
      // Already at the live's edge.
      self.sync_rewind();
    }
  }

  /// Back from a recorded live's recording (or a pause) to the live, at its edge.
  fn play_live(&mut self, cx: &mut Context<Self>) {
    let at = self.position;
    if self.rewound {
      self.rewound = false;
      cx.emit(PlayerEvent::Rewound { rewound: false, at });
    }
    if let Some(old) = self.player.take() {
      old.set_paused(true);
    }
    self.live_mpv = (0.0, 0.0);
    self.live_seek_back = None;
    self.resume_live(cx);
    self.paused = false;
    self.sync_rewind();
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
    // Twitch's plays a few seconds behind its edge, YouTube's `LIVE_DELAY`.
    let near = if self.starts_at_edge() {
      8.0
    } else {
      LIVE_DELAY + 10.0
    };
    self.behind_live() < near
  }

  /// Back to a DVR live's edge: a seek, mpv keeps reloading the playlist. A recorded live goes
  /// back from its recording (or from a pause) to the live.
  fn go_live(&mut self, cx: &mut Context<Self>) {
    if self.rewind.is_some() && self.is_live() {
      if self.rewound || self.paused || !self.at_live_edge() {
        self.play_live(cx);
      }
      return;
    }
    // A restart joins the edge with its low latency; a seek would stop short of it.
    if self.starts_at_edge() {
      self.resume_live(cx);
      return;
    }
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

  /// How far back the DVR live playing can be watched, seconds, once its space knows better
  /// than when it started ([`Source::dvr_window`]). Nothing for another source.
  pub fn set_dvr_window(&mut self, window: f64, cx: &mut Context<Self>) {
    if let Some(source) = self.source.as_mut().filter(|s| s.dvr_window.is_some()) {
      source.dvr_window = Some(window);
      cx.notify();
    }
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

  /// Title shown over the top of the picture while the controls show.
  pub fn set_title(&mut self, title: SharedString, cx: &mut Context<Self>) {
    self.title = title;
    cx.notify();
  }

  /// The channel shown with the title over the picture (embedded layout): its avatar, then its
  /// name under the title.
  pub fn set_author(&mut self, author: Option<Author>, cx: &mut Context<Self>) {
    self.author = author;
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

  /// Plays the video again from its start (a page looping its video at the end).
  pub fn replay(&mut self, cx: &mut Context<Self>) {
    self.seek_to(0.0, cx);
    self.with_player(|p| p.set_paused(false));
  }

  /// The shell opened (or closed) the mini-player on this player: the page shows a
  /// placeholder where the picture was, and the button closes it.
  pub fn set_picture_in_picture(&mut self, in_pip: bool, cx: &mut Context<Self>) {
    if self.in_pip != in_pip {
      self.in_pip = in_pip;
      cx.notify();
    }
  }

  /// Whether the mini-player shows this player's picture.
  pub fn in_picture_in_picture(&self) -> bool {
    self.in_pip
  }

  /// Picture-in-picture needs the app's shell (not the example's window layout) and a video.
  fn pip_offered(&self) -> bool {
    self.pip_page_offered && self.layout != Layout::Window && self.player.is_some()
  }

  /// The button or I: asks the shell to open the mini-player on this player, or to close it.
  /// Out of fullscreen first: the mini-player stands beside the window.
  fn toggle_picture_in_picture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if window.is_fullscreen() {
      window.toggle_fullscreen();
    }
    let open = !self.in_pip;
    cx.set_global(PictureInPicture(Some((cx.weak_entity(), open))));
  }

  /// Whether the page offers picture-in-picture (its button and I); it does by default.
  pub fn offer_picture_in_picture(&mut self, offered: bool, cx: &mut Context<Self>) {
    self.pip_page_offered = offered;
    cx.notify();
  }

  /// Whether the bar shows the seek buttons (← and → seek either way); it does by default.
  pub fn show_seek_buttons(&mut self, shown: bool, cx: &mut Context<Self>) {
    self.seek_buttons = shown;
    cx.notify();
  }

  /// Whether the bar shows the time; it does by default.
  pub fn show_time(&mut self, shown: bool, cx: &mut Context<Self>) {
    self.time_shown = shown;
    cx.notify();
  }

  /// Adds a button at the top right of the embedded player, shown with the controls; `None`
  /// removes it.
  pub fn set_corner_action(&mut self, action: Option<CornerAction>, cx: &mut Context<Self>) {
    self.corner_action = action;
    cx.notify();
  }

  /// Whether the page's button is there ([`PlayerView::set_corner_action`]).
  pub fn has_corner_action(&self) -> bool {
    self.corner_action.is_some()
  }

  /// Whether the page offers theater mode (its button and T); it does by default.
  pub fn offer_theater(&mut self, offered: bool, cx: &mut Context<Self>) {
    self.theater_offered = offered;
    cx.notify();
  }

  /// Whether the page offers speed changes (the menu, and 2× while held); it does by default.
  pub fn offer_speed(&mut self, offered: bool, cx: &mut Context<Self>) {
    self.speed_offered = offered;
    cx.notify();
  }

  /// Plays or pauses. A live paused for a while resumes at its live edge, as Twitch's player:
  /// mpv restarts on the current quality (about 0.7 s, the last frame staying on screen). A DVR
  /// live resumes where it was paused, as YouTube's.
  pub fn toggle_pause(&mut self, cx: &mut Context<Self>) {
    let stale = self.paused
      && self
        .paused_since
        .is_some_and(|t| t.elapsed() > LIVE_RESUME_AFTER);
    let recorded_live = self.rewind.is_some() && self.is_live() && !self.rewound;
    let behind = self.behind_live();
    if recorded_live && stale && self.player.is_some() && behind > self.live_back() - WINDOW_MARGIN
    {
      // As a DVR live: on from where it was paused, in the recording once the live's own window
      // has slid past it.
      self.play_recording(self.position, cx);
    } else if !recorded_live
      && self.starts_at_edge()
      && stale
      && self.player.is_some()
      && self
        .dvr_window()
        .is_some_and(|window| behind > window - WINDOW_MARGIN)
    {
      // The live's window has slid past where it was paused: its edge.
      self.resume_live(cx);
    } else if self.is_live() && self.dvr_window().is_none() && stale && self.player.is_some() {
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
          let can_speed = this.speed_offered && !this.is_live();
          if this.hold.is_some() && !this.paused && can_speed && this.player.is_some() {
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
    tracing::info!("live: back to its edge");
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

  /// Becomes the player that plays in its window, pausing the one that played there before: one
  /// sound at a time per window, other windows' videos play on.
  fn take_over(&self, cx: &mut Context<Self>) {
    let me = cx.entity().downgrade();
    let mut playing = cx
      .try_global::<NowPlaying>()
      .map(|p| p.0.clone())
      .unwrap_or_default();
    playing.retain(|(window, player)| {
      if *player == me {
        return false;
      }
      if *window != self.window {
        return player.upgrade().is_some();
      }
      if let Some(previous) = player.upgrade() {
        previous.read(cx).pause();
      }
      false
    });
    playing.push((self.window, me));
    cx.set_global(NowPlaying(playing));
  }

  /// The page now shows in another window (its tab was dragged there): playing, it takes over
  /// that window; paused, it only leaves the previous one's place.
  fn moved_window(&mut self, cx: &mut Context<Self>) {
    if !self.started || self.secondary {
      return;
    }
    if !self.paused {
      self.take_over(cx);
    } else if let Some(playing) = cx.try_global::<NowPlaying>() {
      let me = cx.entity().downgrade();
      let mut playing = playing.0.clone();
      playing.retain(|(_, p)| *p != me);
      cx.set_global(NowPlaying(playing));
    }
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

  /// Makes the player a multistream's secondary picture, or its main one again. A secondary
  /// player is always muted (the viewer's own mute stays for when it is main again), never
  /// becomes the one playing in its window ([`NowPlaying`]: its sound, the mini-player, a
  /// service's watch time), and draws only its picture, its spinner and its notice: its page
  /// draws what goes over it and takes its clicks. One playing its live's recording goes back to
  /// the live. Made main again, a playing one takes over its window.
  pub fn set_secondary(&mut self, secondary: bool, cx: &mut Context<Self>) {
    if self.secondary == secondary {
      return;
    }
    self.secondary = secondary;
    let muted = secondary || self.muted;
    self.with_player(|p| p.set_muted(muted));
    if secondary {
      self.menu = None;
      self.help = false;
      self.stats = false;
      self.context_menu = None;
      if self.rewound {
        self.play_live(cx);
      }
      let me = cx.entity().downgrade();
      if let Some(playing) = cx.try_global::<NowPlaying>() {
        let mut playing = playing.0.clone();
        playing.retain(|(_, p)| *p != me);
        // At once: the player made main next (a swap) must not find this one there.
        cx.set_global(NowPlaying(playing));
      }
    } else if self.started && !self.paused {
      self.take_over(cx);
    }
    cx.notify();
  }

  /// Whether the player is a multistream's secondary picture ([`Self::set_secondary`]).
  pub fn is_secondary(&self) -> bool {
    self.secondary
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
    self.rewind = None;
    self.rewind_tick = None;
    self.rewound = false;
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
    // A recorded live: its recording at the same label, or the live.
    if self.rewind.is_some() && self.is_live() {
      tracing::info!("quality {}", quality.label);
      self.quality = Some(index);
      self.show_osd(tf!("Quality {}", "Qualité {}", quality.label).into(), cx);
      if self.rewound {
        self.play_recording(self.position, cx);
      } else {
        if let Some(old) = self.player.take() {
          old.set_paused(true);
        }
        self.captions_added.clear();
        self.live_mpv = (0.0, 0.0);
        let options = StartOptions {
          start_seconds: self.live_start(),
          ..quality.options
        };
        self.start_with(options, Instant::now(), cx);
      }
      return;
    }
    let start_seconds = match self.dvr_window() {
      // A live that starts at its edge: the new quality's window starts there too.
      Some(_) if self.starts_at_edge() => self.live_start(),
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
    running::register(&player, cx.weak_entity(), cx);
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
    if self.muted || self.secondary {
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
    self.start_catch_up(cx);
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
          if !self.secondary {
            self.take_over(cx);
          }
          cx.emit(PlayerEvent::Started);
        }
      }
      // While dragging, mpv still reports the positions of the seeks it has not caught up
      // with: the playhead would jump back behind the mouse.
      // A recorded live's playhead follows the live's, on the recording's timeline.
      Event::Position(seconds) if self.rewind.is_some() && !self.rewound => {
        self.live_mpv.0 = seconds;
        self.sync_rewind();
      }
      Event::Position(seconds) if !self.seek_dragging => {
        self.position = seconds;
        // A recorded live's recording: its edge (now) moves with the playhead, else how far
        // behind it plays drops by a second between the edge's ticks, then jumps back.
        if self.rewound {
          self.sync_rewind();
        }
        self.update_up_next(cx);
        self.tell_muted(cx);
      }
      Event::Position(_) => {}
      Event::Cached(seconds) if self.rewind.is_some() && !self.rewound => {
        let edge = self.rewind_edge().unwrap_or(0.0);
        self.cached = edge - (self.live_mpv.1 - seconds).max(0.0);
      }
      Event::Cached(seconds) => self.cached = seconds,
      // A live's duration is only its playlist's window: kept at 0, it turns off the seek
      // bar's math, "up next" and `finished`. A DVR live's is its edge, for the seek bar.
      // A recorded live's is the time since its recording started; the live's own, its edge
      // in its mpv's timeline.
      Event::Duration(seconds) if self.rewind.is_some() && !self.rewound => {
        self.live_mpv.1 = seconds;
        // Back from the recording to a point of the live's window: once it is known.
        if let Some(back) = self
          .live_seek_back
          .take_if(|back| seconds > *back + WINDOW_MARGIN)
        {
          self.live_mpv.0 = seconds - back;
          self.with_player(|p| p.seek_to(seconds - back));
        }
        self.sync_rewind();
      }
      Event::Duration(_) if !self.can_seek() || self.rewind.is_some() => {}
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
        // Paused while stuck: the wait starts over on resume.
        self.watch_stall(cx);
        self.show_controls(cx);
      }
      Event::Buffering(buffering) => {
        self.buffering = buffering;
        self.watch_stall(cx);
      }
      // A secondary player is muted by its page, not by the viewer.
      Event::Muted(_) if self.secondary => {}
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
      Event::Ended(Some(error)) => {
        let message: SharedString =
          tf!("Playback stopped: {error}", "Lecture interrompue : {error}").into();
        // mpv could not open the source: the space may know another way, tried once.
        let fallback = (!self.started)
          .then(|| self.source.as_mut()?.fallback.take())
          .flatten();
        match fallback {
          Some(fallback) => {
            tracing::info!("{error} before loading: trying the source's fallback");
            self.buffering = true;
            cx.defer(move |cx| fallback(message, cx));
          }
          None => self.fail(message, cx),
        }
      }
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

  /// Stops playback with an error rather than skipping to the next video, unless a reload
  /// plays again.
  fn reload_after_early_end(&mut self, cx: &mut Context<Self>) {
    let (position, duration) = (self.position, self.duration);
    if self.reload_here(cx) {
      tracing::warn!("stream ended at {position:.0} s of {duration:.0} s: reloading");
      return;
    }
    tracing::warn!("stream ended at {position:.0} s of {duration:.0} s");
    self.fail(
      t(
        "Playback stopped: the stream ended before the video.",
        "Lecture interrompue : le flux s'est arrêté avant la fin de la vidéo.",
      )
      .into(),
      cx,
    );
  }

  /// Starts or stops the timer that reloads a video stuck buffering ([`STALL_AFTER`]). A live
  /// waits on its edge, and a paused video's buffering is no stall.
  fn watch_stall(&mut self, cx: &mut Context<Self>) {
    if !self.buffering || self.paused || !self.started || self.is_live() || !self.can_seek() {
      self.stall_task = None;
      return;
    }
    if self.stall_task.is_some() {
      return;
    }
    self.stall_task = Some(cx.spawn(async move |this, cx| {
      cx.background_executor().timer(STALL_AFTER).await;
      this
        .update(cx, |this, cx| {
          this.stall_task = None;
          if !this.buffering || this.paused {
            return;
          }
          let position = this.position;
          if this.reload_here(cx) {
            tracing::warn!("stalled at {position:.0} s: reloading");
          } else {
            tracing::warn!("stalled at {position:.0} s, reloaded less than a minute ago");
          }
        })
        .ok();
    }));
  }

  /// Restarts mpv on the playing quality from the current position, unless it was already
  /// reloaded less than [`RELOAD_RETRY`] ago; whether it did.
  fn reload_here(&mut self, cx: &mut Context<Self>) -> bool {
    let recent = self
      .last_reload
      .is_some_and(|at| at.elapsed() < RELOAD_RETRY);
    let options = self.source.as_ref().map(|source| {
      self
        .quality
        .and_then(|i| source.qualities.get(i))
        .map_or_else(|| source.options.clone(), |q| q.options.clone())
    });
    let Some(options) = options.filter(|_| !recent) else {
      return false;
    };
    self.last_reload = Some(Instant::now());
    self.stall_task = None;
    let options = StartOptions {
      start_seconds: self.position,
      ..options
    };
    if let Some(old) = self.player.take() {
      old.set_paused(true);
    }
    self.captions_added.clear();
    self.start_with(options, Instant::now(), cx);
    true
  }

  fn with_player(&self, f: impl FnOnce(&Player)) {
    if let Some(player) = &self.player {
      f(player);
    }
  }

  /// Seeks to `seconds`, within what can be seeked (a timestamp's click). A recorded live moves
  /// between its recording and the live.
  pub fn seek_to(&mut self, seconds: f64, cx: &mut Context<Self>) {
    if let Some(edge) = self.rewind_edge() {
      let target = seconds.clamp(0.0, edge);
      if let Some(at) = self.rewind_native(target, edge) {
        self.rewind_drag_from = None;
        self.position = target;
        if !self.rewound {
          self.live_mpv.0 = at;
        }
        self.with_player(|p| p.seek_to(at));
      } else if self.seek_dragging {
        self.rewind_drag_from.get_or_insert(self.position);
        self.position = target;
      } else {
        self.rewind_drag_from = None;
        self.switch_rewind(target, edge, cx);
      }
      cx.emit(PlayerEvent::Seeked);
      cx.notify();
      return;
    }
    let (start, mut end) = self.seek_range();
    // Closer to a live's edge, mpv would wait for segments that do not exist yet: Twitch's joins
    // its edge with a restart instead.
    if self.dvr_window().is_some() {
      if self.starts_at_edge() && seconds > end - EDGE_SNAP {
        if self.paused || !self.at_live_edge() {
          self.resume_live(cx);
        }
        cx.emit(PlayerEvent::Seeked);
        return cx.notify();
      }
      let margin = if self.starts_at_edge() {
        EDGE_SNAP
      } else {
        LIVE_DELAY
      };
      end = (end - margin).max(start);
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

  /// The volume slider's end: 100, or [`player::VOLUME_MAX`] with the boost.
  fn volume_max(&self) -> f64 {
    if self.boosted {
      player::VOLUME_MAX
    } else {
      100.0
    }
  }

  /// The boost button: past 100 % or back to it (a louder volume comes down to 100 %).
  fn toggle_boost(&mut self, cx: &mut Context<Self>) {
    self.boosted = !self.boosted;
    if self.volume > self.volume_max() {
      self.set_volume(self.volume_max(), cx);
    }
    let text: SharedString = if self.boosted {
      tf!(
        "Volume boost, up to {:.0}%",
        "Volume amplifié, jusqu'à {:.0} %",
        player::VOLUME_MAX
      )
      .into()
    } else {
      t("Volume boost off", "Amplification coupée").into()
    };
    self.show_osd(text, cx);
  }

  fn set_volume(&mut self, volume: f64, cx: &mut Context<Self>) {
    self.volume = volume.clamp(0.0, self.volume_max());
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

  /// Volume from a mouse position on the horizontal slider: left is 0 %, right the maximum.
  fn set_volume_at(&mut self, x: Pixels, cx: &mut Context<Self>) {
    let bounds = self.volume_bounds.get();
    let width = f32::from(bounds.size.width);
    if width > 0.0 {
      let fraction = (f32::from(x - bounds.origin.x) / width).clamp(0.0, 1.0);
      self.set_volume((f64::from(fraction) * self.volume_max()).round(), cx);
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
    // A recorded live's move between the live and its recording, left for the release.
    if self.rewind_drag_from.is_some() {
      self.seek_to(self.position, cx);
    }
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
  /// A boosted volume is kept as 100 %: the boost is never remembered.
  fn remember_sound(&mut self, cx: &mut Context<Self>) {
    let volume = self.volume.min(100.0);
    self.prefs.volume = volume;
    self.prefs.muted = self.muted;
    cx.emit(PlayerEvent::Sound {
      volume,
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
    if !self.controls_visible {
      // Also cuts a running fade-out short: the fade-in starts over from transparent.
      self.controls_visible = true;
      self.controls_animation = self.controls_animation.wrapping_add(1);
      self.controls_fade_task = None;
    }
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
            this.controls_animation = this.controls_animation.wrapping_add(1);
            this.controls_fade_task = Some(cx.spawn(async move |this, cx| {
              cx.background_executor().timer(CONTROLS_FADE).await;
              this
                .update(cx, |this, cx| {
                  this.controls_fade_task = None;
                  cx.notify();
                })
                .ok();
            }));
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
      "t" if !window.is_fullscreen() && !page_layout && self.theater_offered => {
        self.toggle_theater(cx)
      }
      "i" if self.pip_offered() => self.toggle_picture_in_picture(window, cx),
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
    let dvr = self.dvr_window().is_some();
    // A live played at its edge (the badge says "LIVE") shows a full bar: the few seconds of
    // buffer it trails by are not a past to tell. A drag shows where it goes.
    let at_edge = dvr && !self.seek_dragging && self.at_live_edge();
    let progress = if at_edge {
      1.0
    } else if end > start {
      ((self.position - start) / (end - start)).clamp(0.0, 1.0)
    } else {
      0.0
    };
    let cached = if at_edge {
      1.0
    } else if end > start {
      ((self.cached - start) / (end - start)).clamp(0.0, 1.0)
    } else {
      0.0
    };
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

  /// Mute button; hovering it opens a horizontal volume slider beside it, in the row.
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
    // 100 % stays where it was without the boost: the boost lengthens the track.
    let boost = (self.volume_max() / 100.0) as f32;
    let track_width = 64. * boost;
    // Room for the knob at both ends of the track, which the opening clips.
    const PAD_LEFT: f32 = 8.;
    const PAD_RIGHT: f32 = 10.;
    // The boost button after the track, and the gap before it.
    const BOOST_WIDTH: f32 = 46.;
    const BOOST_GAP: f32 = 10.;
    let slider_width = PAD_LEFT + track_width + BOOST_GAP + BOOST_WIDTH + PAD_RIGHT;
    let primary = cx.theme().primary;
    let icon = if self.muted || self.volume == 0.0 {
      Icon::Muted
    } else if self.volume < 50.0 {
      Icon::VolumeLow
    } else {
      Icon::Volume
    };
    let level = if self.muted {
      0.0
    } else {
      self.volume / self.volume_max()
    } as f32;
    // The boosted part of the fill, past 100 %, in the warning color.
    let unity = 1. / boost;
    let over = (level - unity).max(0.);
    let (warning, on_warning) = (cx.theme().warning, cx.theme().warning_foreground);
    let boost_on = self.boosted;
    let boost_button = div()
      .id("volume-boost")
      .flex_none()
      .ml(px(BOOST_GAP))
      .w(px(BOOST_WIDTH))
      .h(px(20.))
      .flex()
      .items_center()
      .justify_center()
      .rounded_full()
      .border_1()
      .text_xs()
      .font_weight(FontWeight::SEMIBOLD)
      .map(|el| {
        if boost_on {
          el.bg(warning).border_color(warning).text_color(on_warning)
        } else {
          el.border_color(gpui::white().opacity(0.4))
            .text_color(gpui::white().opacity(0.8))
            .hover(|el| el.bg(gpui::white().opacity(0.15)))
        }
      })
      .child("Boost")
      // Pressed here, not on the slider: neither a volume change nor a drag.
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, _, _, cx| {
          cx.stop_propagation();
          this.toggle_boost(cx);
        }),
      )
      .tooltip(move |window, cx| {
        Tooltip::new(if boost_on {
          t("Back to 100% at most", "Revenir à 100 % au plus").to_owned()
        } else {
          tf!(
            "Let the volume go up to {:.0}%",
            "Pousser le volume jusqu'à {:.0} %",
            player::VOLUME_MAX
          )
        })
        .build(window, cx)
      });
    let bounds_cell = self.volume_bounds.clone();
    let (dragging, view) = (self.volume_dragging, cx.weak_entity());
    // The slider follows the button in the row and widens from it, so the mouse goes from one
    // to the other without leaving both.
    let slider = div()
      .id("volume-slider")
      .h(px(32.))
      .flex()
      .flex_none()
      .items_center()
      .overflow_hidden()
      .pl(px(PAD_LEFT))
      .pr(px(PAD_RIGHT))
      .cursor_pointer()
      .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
        this.volume_hover.1 = *hovered;
        cx.notify();
      }))
      .on_mouse_down(
        MouseButton::Left,
        cx.listener(|this, event: &MouseDownEvent, _, cx| {
          this.volume_dragging = true;
          this.set_volume_at(event.position.x, cx);
        }),
      )
      // Over the slider, in any layout: the wheel never reaches the picture or the page.
      .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
        cx.stop_propagation();
        let dy = f32::from(event.delta.pixel_delta(window.line_height()).y);
        if dy.abs() > 0.5 {
          let step = if dy > 0.0 {
            VOLUME_SLIDER_STEP
          } else {
            -VOLUME_SLIDER_STEP
          };
          this.set_volume(this.volume + step, cx);
          this.show_controls(cx);
        }
      }))
      .child(
        div()
          .relative()
          .flex_none()
          .w(px(track_width))
          .h(px(4.))
          .rounded_full()
          .bg(gpui::white().opacity(0.25))
          .child(
            div()
              .absolute()
              .top_0()
              .bottom_0()
              .left_0()
              .w(relative(level))
              .min_w(px(4.))
              .rounded_full()
              .bg(primary),
          )
          .when(boost > 1., |el| {
            el.when(over > 0., |el| {
              el.child(
                div()
                  .absolute()
                  .top_0()
                  .bottom_0()
                  .left(relative(unity))
                  .w(relative(over))
                  .rounded_r_full()
                  .bg(warning),
              )
            })
            // The mark of 100 %, where the boost starts.
            .child(
              div()
                .absolute()
                .top(px(-3.))
                .left(relative(unity))
                .ml(px(-1.))
                .w(px(2.))
                .h(px(10.))
                .rounded_full()
                .bg(gpui::white().opacity(0.6)),
            )
          })
          .child(
            div()
              .absolute()
              .left(relative(level))
              .top(px(-5.))
              .ml(px(-7.))
              .size(px(14.))
              .rounded_full()
              .bg(if over > 0. { warning } else { primary })
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
                    |this, position, cx| this.set_volume_at(position.x, cx),
                    |this, _, cx| this.end_volume_drag(cx),
                  );
                }
              },
            )
            .absolute()
            .inset_0(),
          ),
      )
      .child(boost_button);
    div()
      .id("volume")
      .flex()
      .items_center()
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
            el.opacity(p).w(px(slider_width * p))
          },
        ))
      })
  }

  /// Whether the embedded player shows its title bar while the controls show.
  fn has_title_bar(&self) -> bool {
    self.layout == Layout::Embedded && (!self.title.is_empty() || self.author.is_some())
  }

  /// The embedded player's top: the channel's avatar, the title and the channel's name under
  /// it, over a gradient. It takes no mouse events: a click there plays or pauses.
  fn render_title_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
    if !self.has_title_bar() {
      return None;
    }
    let fullscreen = window.is_fullscreen();
    let radii = corner_radii(window, self.theater);
    let author = self.author.as_ref();
    let avatar = author
      .and_then(|a| a.avatar.as_ref())
      .filter(|p| !p.is_empty())
      .map(|p| picture::avatar(p, if fullscreen { 44. } else { 36. }, window, cx));
    let name = author.map(|a| a.name.clone()).filter(|n| !n.is_empty());
    let line = |text: SharedString| {
      div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text)
    };
    Some(
      h_flex()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .when(fullscreen, |el| el.px_6().pt_4())
        .when(!fullscreen, |el| el.px_4().pt_3())
        .when(!fullscreen && self.corner_action.is_some(), |el| {
          el.pr(px(CONTROL_SIZE + 24.))
        })
        .pb_10()
        .gap_3()
        .items_center()
        // Its gradient reaches the player's top corners: rounded like them.
        .rounded_tl(radii.top_left)
        .rounded_tr(radii.top_right)
        .bg(linear_gradient(
          180.,
          linear_color_stop(gpui::black().opacity(0.75), 0.0),
          linear_color_stop(gpui::black().opacity(0.0), 1.0),
        ))
        .text_color(gpui::white())
        .children(avatar)
        .child(
          v_flex()
            .flex_1()
            .min_w_0()
            .when(!self.title.is_empty(), |col| {
              col.child(
                line(self.title.clone())
                  .when(fullscreen, |el| el.text_lg().line_height(px(22.)))
                  .when(!fullscreen, |el| el.text_base().line_height(px(19.)))
                  .font_weight(FontWeight::SEMIBOLD),
              )
            })
            .children(name.map(|name| {
              line(name)
                .text_sm()
                .line_height(px(16.))
                .text_color(gpui::white().opacity(0.8))
            })),
        )
        .into_any_element(),
    )
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
    } else if let Some(bar) = self.render_title_bar(window, cx) {
      layers.push(bar);
    }
    if embedded
      && !fullscreen
      && let Some(action) = self.corner_action.clone()
    {
      let on_click = action.on_click.clone();
      layers.push(
        div()
          .id("corner-action")
          .absolute()
          .top_3()
          .right_4()
          .block_mouse_except_scroll()
          .on_hover(cx.listener(|this, hovered: &bool, _, _| this.over_controls = *hovered))
          .on_mouse_move(cx.listener(|this, _, _, cx| this.show_controls(cx)))
          .child(
            self
              .control_button("corner-action-button", cx)
              .icon(action.icon)
              .tooltip(action.tooltip)
              .on_click(move |_, window, cx| on_click(window, cx)),
          )
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
      .when(can_seek && self.seek_buttons, |row| {
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
        } else if !self.time_shown {
          row
        } else {
          row.child(div().pl_2().text_sm().text_color(white(0.9)).child(format!(
            "{} / {}",
            format_time(self.position),
            format_time(self.duration)
          )))
        }
      })
      .child(div().flex_1())
      .when(
        self.time_shown && !embedded && !live && self.duration > 0.0,
        |row| {
          row.child(
            div()
              .pr_2()
              .text_sm()
              .text_color(white(0.85))
              .child(format!("-{}", format_time(self.duration - self.position))),
          )
        },
      )
      // The first of the right-hand buttons.
      .when(!fullscreen && self.pip_offered(), |row| {
        row.child(
          self
            .control_button("picture-in-picture", cx)
            .icon(Icon::PictureInPicture)
            .tooltip(if self.in_pip {
              t(
                "Exit picture-in-picture (I)",
                "Quitter l'image dans l'image (I)",
              )
            } else {
              t("Picture-in-picture (I)", "Image dans l'image (I)")
            })
            .on_click(
              cx.listener(|this, _, window, cx| this.toggle_picture_in_picture(window, cx)),
            ),
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
      .when(!self.is_live() && self.speed_offered, |row| {
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
      .when(!fullscreen && embedded && self.theater_offered, |row| {
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

impl PlayerView {
  /// A secondary player ([`Self::set_secondary`]): its picture, with its page's notice, a
  /// spinner while it loads, or why it cannot play; no controls, no handlers.
  fn render_secondary(&self, radii: Corners<Pixels>) -> Stateful<Div> {
    let message = if self.failed {
      self.status.clone()
    } else {
      self.notice.clone()
    };
    let overlay = div()
      .absolute()
      .inset_0()
      .flex()
      .items_center()
      .justify_center()
      .p_2();
    let overlay = match message {
      Some(message) => Some(
        overlay.child(
          div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(gpui::black().opacity(0.6))
            .text_xs()
            .text_color(gpui::white())
            .line_clamp(3)
            .child(message),
        ),
      ),
      None if self.player.is_none() || self.buffering => {
        Some(overlay.child(crate::widgets::loader(28., gpui::white())))
      }
      None => None,
    };
    div()
      .id("player")
      .size_full()
      .relative()
      .rounded_tl(radii.top_left)
      .rounded_tr(radii.top_right)
      .rounded_br(radii.bottom_right)
      .rounded_bl(radii.bottom_left)
      .bg(gpui::black())
      .children(
        self
          .video
          .clone()
          .filter(|_| !self.in_pip && !self.failed)
          .map(|video| div().absolute().inset_0().child(video)),
      )
      .children(overlay)
  }
}

impl Render for PlayerView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let id = window.window_handle().window_id();
    if self.window != Some(id) {
      let moved = self.window.is_some();
      self.window = Some(id);
      // Deferred: setting the global wakes the shells, which update windows.
      if moved {
        let me = cx.weak_entity();
        cx.defer(move |cx| {
          me.update(cx, |this, cx| this.moved_window(cx)).ok();
        });
      }
    }
    let show_ui = (self.controls_visible || self.failed) && self.player.is_some();
    // A page of its own fills the panel like theater mode.
    let radii = corner_radii(window, self.theater || self.layout == Layout::Page);
    if let Some(video) = &self.video {
      video.update(cx, |video, _| video.set_corner_radii(radii));
    }
    if self.secondary {
      return self.render_secondary(radii);
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
          .filter(|_| !self.in_pip)
          .map(|video| div().absolute().inset_0().child(video)),
      )
      .when(self.in_pip, |root| {
        root.child(
          v_flex()
            .absolute()
            .inset_0()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(gpui::white().opacity(0.7))
            .child(gpui_component::Icon::new(Icon::PictureInPicture).size(px(48.)))
            .child(t(
              "Playing in picture-in-picture",
              "Lecture en image dans l'image",
            )),
        )
      })
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

    // The page's notice says why the picture waits better than a spinner. Until mpv starts
    // (no player yet) and while it buffers, a spinner stands over the picture.
    if !self.failed {
      if let Some(notice) = self.notice.clone() {
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
                .child(notice),
            ),
        );
      } else if self.player.is_none() || self.buffering {
        root = root.child(
          div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(crate::widgets::loader(48., gpui::white())),
        );
      }
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
    // Under the title bar while it shows, not hidden behind it.
    let title_bar = show_ui && !self.failed && self.has_title_bar();
    if let Some(osd) = osd {
      root = root.child(
        div()
          .absolute()
          .top(px(if title_bar { 84. } else { 24. }))
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
    // Drawn while shown and while their fade-out runs. The wrapper has no handlers, so it
    // takes no mouse events from the picture; its opacity applies to every bar.
    if self.player.is_some() && (self.controls_visible || self.controls_fade_task.is_some()) {
      let (animation, showing) = (self.controls_animation, self.controls_visible);
      let ease = ease_out_quint();
      root = root.child(
        div()
          .absolute()
          .inset_0()
          .children(self.render_controls(window, cx))
          .with_animation(
            SharedString::from(format!("controls-{animation}")),
            Animation::new(CONTROLS_FADE),
            // Eased by hand so that hiding replays the showing curve backwards.
            move |el, t| el.opacity(ease(if showing { t } else { 1. - t })),
          ),
      );
    }
    if show_ui && let Some(menu) = self.menu {
      root = root.child(self.render_menu(menu, window, cx));
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
