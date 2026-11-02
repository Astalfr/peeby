//! Playback engine abstraction and its libmpv backend (network playback, decoding, A/V sync, ASS subtitles).
//!
//! [`Player::start`] opens a URL with libmpv and renders frames for the UI:
//! - **GPU** (preferred): mpv renders with its D3D11 API into a private texture, which is copied
//!   into a ring of keyed-mutex shared textures; the UI draws those directly ([`Frame::GpuTextures`],
//!   [`Frame::GpuFrame`]).
//! - **Software** (fallback): mpv renders BGRA pixels into memory ([`Frame::Cpu`]).
//!
//! See docs/player/mpv.md and docs/player/video-surface.md.

mod d3d;
// FFI module: the only place with `unsafe` besides d3d.rs.
#[allow(dead_code)]
mod sys;

use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::d3d::{RenderTarget, SharedTexture, VideoDevice};
use crate::sys::{Mpv, MpvEvent, RenderBackend};

/// Number of shared textures in the GPU ring.
const RING: usize = 3;
/// A size change after this long without one recreates the ring at once; closer ones (a resize by
/// the mouse, one per move) wait until the size has not changed for this long, frames coming
/// meanwhile (stretched) at the old size.
const RESIZE_SETTLE: Duration = Duration::from_millis(60);
/// Position events are sent at most this often (seconds of media time).
const POSITION_STEP: f64 = 0.25;

/// Which track to select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
  /// Let mpv decide (file defaults and language preferences).
  Auto,
  /// No track (subtitles off).
  Off,
  /// The mpv track id (`aid`/`sid`, 1-based per type).
  Track(i64),
}

impl Choice {
  fn as_option(self) -> String {
    match self {
      Self::Auto => "auto".into(),
      Self::Off => "no".into(),
      Self::Track(id) => id.to_string(),
    }
  }
}

/// What to play and how.
#[derive(Clone, Debug, Default)]
pub struct StartOptions {
  /// File or URL.
  pub url: String,
  /// Extra HTTP headers for the stream and external subtitles, e.g. `Authorization: …`.
  pub http_headers: Vec<String>,
  /// Resume position in seconds.
  pub start_seconds: f64,
  /// Audio track.
  pub audio: Option<Choice>,
  /// Subtitle track.
  pub subtitle: Option<Choice>,
  /// External subtitle files or URLs, added after the embedded tracks, in this order.
  pub external_subtitles: Vec<String>,
  /// External audio files or URLs (`audio-files`), e.g. the audio rendition of an HLS variant.
  pub external_audio: Vec<String>,
  /// Folder with extra fonts for subtitles (fonts of external .ass files).
  pub fonts_dir: Option<PathBuf>,
  /// Skip the GPU path.
  pub force_software: bool,
  /// Decode on the CPU (`hwdec=no`), for troubleshooting.
  pub software_decoding: bool,
  /// Size factor of text subtitles (`sub-scale`); ASS is left alone (`sub-ass-override=no`).
  pub subtitle_scale: Option<f64>,
  /// Read-ahead buffer, MiB (`demuxer-max-bytes`); default 300.
  pub buffer_mb: Option<u32>,
  /// Path of `libmpv-2.dll`; default: next to the executable.
  pub libmpv_path: Option<PathBuf>,
  /// Resolve page URLs (YouTube…) through yt-dlp with mpv's `ytdl_hook`; `None` disables it.
  pub ytdl: Option<Ytdl>,
  /// Write mpv's log to this file (`log-file`), for troubleshooting.
  pub log_file: Option<PathBuf>,
  /// Audio output, an [`AudioDevice::name`] (`audio-device`); `None` follows Windows' default.
  pub audio_device: Option<String>,
}

/// An audio output mpv can play to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
  /// mpv's name for it, e.g. `wasapi/{0.0.0.00000000}.{…}`.
  pub name: String,
  /// What Windows calls it, e.g. `Speakers (Realtek(R) Audio)`.
  pub description: String,
}

/// The audio outputs of this PC (WASAPI endpoints), from an mpv made only to ask (a few tens of
/// milliseconds: call it off the UI thread). `libmpv_path` as [`StartOptions::libmpv_path`].
pub fn audio_devices(libmpv_path: Option<PathBuf>) -> Result<Vec<AudioDevice>, String> {
  let dll = libmpv_path.unwrap_or_else(default_libmpv_path);
  let api = load_api(&dll)?;
  let options = [
    ("vo", "null"),
    ("terminal", "no"),
    ("idle", "yes"),
    ("video", "no"),
  ];
  let mpv = Mpv::new(api, &options, RenderBackend::None)?;
  // A node list: read item by item through its sub-properties.
  let count = mpv
    .property("audio-device-list/count")
    .and_then(|n| n.parse::<usize>().ok())
    .unwrap_or(0);
  let devices = (0..count)
    .filter_map(|i| {
      let name = mpv.property(&format!("audio-device-list/{i}/name"))?;
      let description = mpv
        .property(&format!("audio-device-list/{i}/description"))
        .unwrap_or_else(|| name.clone());
      // `auto` is the caller's "default" entry; other drivers (OpenAL, SDL) repeat WASAPI's.
      name
        .starts_with("wasapi/")
        .then_some(AudioDevice { name, description })
    })
    .collect();
  Ok(devices)
}

/// yt-dlp settings for mpv's `ytdl_hook`.
#[derive(Clone, Debug, Default)]
pub struct Ytdl {
  /// yt-dlp executable; default: `yt-dlp` from `PATH`.
  pub ytdl_path: Option<PathBuf>,
  /// yt-dlp format selector (`ytdl-format`), e.g. `bv*[height<=?1080]+ba/b`.
  pub format: Option<String>,
  /// Extra yt-dlp options as `name=value` (`--name value`), e.g. `cookies=D:/yt/cookies.txt`.
  pub raw_options: Vec<String>,
}

/// Rendering path in use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
  /// Zero copy through D3D11 shared textures.
  Gpu,
  /// BGRA frames in memory.
  Software,
}

/// Frames for the UI.
#[derive(Debug)]
pub enum Frame {
  /// A new ring of shared textures (legacy DXGI shared handles), after start or a resize.
  GpuTextures {
    /// Increases with every new ring; frames of older rings must be ignored.
    generation: u64,
    /// Texture width in pixels.
    width: u32,
    /// Texture height in pixels.
    height: u32,
    /// One handle per ring slot.
    handles: Vec<u64>,
  },
  /// Ring slot `index` of `generation` holds the newest frame.
  GpuFrame {
    /// Ring the slot belongs to.
    generation: u64,
    /// Slot index in the ring.
    index: usize,
  },
  /// A software frame, BGRA with opaque alpha, `width * 4` bytes per row.
  Cpu {
    /// Width in pixels.
    width: u32,
    /// Height in pixels.
    height: u32,
    /// BGRA pixels.
    pixels: Vec<u8>,
  },
}

/// Track types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum TrackKind {
  Video,
  Audio,
  Subtitle,
}

/// A track as mpv sees it.
#[derive(Clone, Debug)]
pub struct Track {
  /// mpv id within its type.
  pub id: i64,
  /// Type.
  pub kind: TrackKind,
  /// FFmpeg stream index for embedded tracks (matches Jellyfin's `MediaStream.Index`).
  pub ff_index: Option<i64>,
  /// Loaded from a separate file or URL.
  pub external: bool,
  /// Title.
  pub title: Option<String>,
  /// Language.
  pub lang: Option<String>,
  /// Codec.
  pub codec: Option<String>,
  /// Currently selected.
  pub selected: bool,
}

/// A chapter of the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Chapter {
  /// Start, seconds.
  pub start: f64,
  /// Title from the container (`Opening`, `Part A`…), if any.
  pub title: Option<String>,
}

/// Playback events for the UI.
#[derive(Clone, Debug)]
pub enum Event {
  /// The file is open.
  Loaded,
  /// Current position, seconds.
  Position(f64),
  /// Duration, seconds.
  Duration(f64),
  /// Paused or playing.
  Paused(bool),
  /// Waiting for the network.
  Buffering(bool),
  /// The track list changed (or a selection changed).
  Tracks(Vec<Track>),
  /// The chapter list is known (or changed).
  Chapters(Vec<Chapter>),
  /// Muted or not.
  Muted(bool),
  /// Playback reached the end of the file. With `keep-open` mpv stays paused on the last
  /// frame instead of unloading the file, so this comes from `eof-reached`.
  EndReached,
  /// Playback stopped: `None` when the file was unloaded normally, or an error message.
  Ended(Option<String>),
}

#[derive(Default)]
struct Wake {
  dirty: bool,
  resized: bool,
}

#[derive(Default)]
struct Shared {
  wake: Mutex<Wake>,
  cond: Condvar,
  quit: AtomicBool,
  target: Mutex<(u32, u32)>,
}

impl Shared {
  fn poke(&self, resized: bool) {
    if let Ok(mut wake) = self.wake.lock() {
      wake.dirty = true;
      wake.resized |= resized;
    }
    self.cond.notify_one();
  }

  fn quitting(&self) -> bool {
    self.quit.load(Ordering::SeqCst)
  }

  /// Blocks until mpv has a new frame, the target was resized, `timeout` passed or the player
  /// stops. Returns `None` on quit, else whether a resize happened.
  fn wait(&self, timeout: Option<Duration>) -> Option<bool> {
    let deadline = timeout.map(|t| Instant::now() + t);
    let mut wake = self.wake.lock().ok()?;
    while !wake.dirty && !self.quitting() {
      match deadline {
        Some(deadline) => {
          let left = deadline.saturating_duration_since(Instant::now());
          if left.is_zero() {
            break;
          }
          wake = self.cond.wait_timeout(wake, left).ok()?.0;
        }
        None => wake = self.cond.wait(wake).ok()?,
      }
    }
    wake.dirty = false;
    let resized = std::mem::take(&mut wake.resized);
    (!self.quitting()).then_some(resized)
  }
}

unsafe extern "C" fn on_mpv_update(ctx: *mut c_void) {
  // SAFETY: ctx is `Arc::as_ptr` of the Shared kept alive by `Player` until mpv is destroyed.
  let shared = unsafe { &*(ctx as *const Shared) };
  shared.poke(false);
}

/// A playing file. Dropping it stops playback and frees mpv.
pub struct Player {
  mpv: Arc<Mpv>,
  shared: Arc<Shared>,
  threads: Vec<JoinHandle<()>>,
  backend: Backend,
}

impl Player {
  /// Starts playback, GPU path first, software if it fails or `force_software` is set.
  pub fn start(
    options: StartOptions,
    events: async_channel::Sender<Event>,
    frames: async_channel::Sender<Frame>,
  ) -> Result<Self, String> {
    let dll = options
      .libmpv_path
      .clone()
      .unwrap_or_else(default_libmpv_path);

    let mut gpu = None;
    if !options.force_software {
      match start_gpu(&dll, &options) {
        Ok(started) => gpu = Some(started),
        Err(e) => tracing::warn!("GPU video path unavailable, using software rendering: {e}"),
      }
    }
    let (mpv, device, backend) = match gpu {
      Some((mpv, device)) => (mpv, Some(device), Backend::Gpu),
      None => {
        let api = load_api(&dll)?;
        let mpv = create_mpv(
          api,
          &mpv_options(&options, Backend::Software),
          RenderBackend::Software,
        )?;
        (mpv, None, Backend::Software)
      }
    };
    tracing::info!("video backend: {backend:?}");

    let mpv = Arc::new(mpv);
    let shared = Arc::new(Shared::default());
    mpv.set_update_callback(on_mpv_update, Arc::as_ptr(&shared) as *mut c_void);
    for (name, format) in [
      ("time-pos", sys::FORMAT_DOUBLE),
      ("duration", sys::FORMAT_DOUBLE),
      ("pause", sys::FORMAT_FLAG),
      ("paused-for-cache", sys::FORMAT_FLAG),
      ("track-list", sys::FORMAT_NONE),
      ("chapter-list", sys::FORMAT_NONE),
      ("eof-reached", sys::FORMAT_FLAG),
      ("mute", sys::FORMAT_FLAG),
    ] {
      mpv.observe(name, format)?;
    }
    // List options are filled with `change-list … append`, which adds one item verbatim
    // (setting them as plain options would split the Authorization header on its commas).
    for header in &options.http_headers {
      mpv.command(&["change-list", "http-header-fields", "append", header])?;
    }
    for sub in &options.external_subtitles {
      mpv.command(&["change-list", "sub-files", "append", sub])?;
    }
    for audio in &options.external_audio {
      mpv.command(&["change-list", "audio-files", "append", audio])?;
    }
    // Same for the key-value lists: a path with a comma would otherwise be split.
    if let Some(ytdl) = &options.ytdl {
      if let Some(path) = &ytdl.ytdl_path {
        let opt = format!("ytdl_hook-ytdl_path={}", path.display());
        mpv.command(&["change-list", "script-opts", "append", &opt])?;
      }
      for raw in &ytdl.raw_options {
        mpv.command(&["change-list", "ytdl-raw-options", "append", raw])?;
      }
    }
    mpv.command(&["loadfile", &options.url])?;

    let spawn = |name: &str, f: Box<dyn FnOnce() + Send>| {
      std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map_err(|e| e.to_string())
    };
    let render = {
      let (mpv, shared) = (mpv.clone(), shared.clone());
      match device {
        Some(device) => spawn(
          "mpv-render",
          Box::new(move || render_gpu(&mpv, &shared, &device, &frames)),
        )?,
        None => spawn(
          "mpv-render",
          Box::new(move || render_software(&mpv, &shared, &frames)),
        )?,
      }
    };
    let event_thread = {
      let (mpv, shared) = (mpv.clone(), shared.clone());
      spawn(
        "mpv-events",
        Box::new(move || event_loop(&mpv, &shared, &events)),
      )?
    };
    Ok(Self {
      mpv,
      shared,
      threads: vec![render, event_thread],
      backend,
    })
  }

  /// Rendering path in use.
  pub fn backend(&self) -> Backend {
    self.backend
  }

  /// Size, in physical pixels, of the area the video is drawn in.
  pub fn set_target_size(&self, width: u32, height: u32) {
    let changed = match self.shared.target.lock() {
      Ok(mut t) if *t != (width, height) => {
        *t = (width, height);
        true
      }
      _ => false,
    };
    if changed {
      self.shared.poke(true);
    }
  }

  /// Pauses or resumes.
  pub fn set_paused(&self, paused: bool) {
    self.set("pause", if paused { "yes" } else { "no" });
  }

  /// Toggles pause.
  pub fn toggle_pause(&self) {
    self.run(&["cycle", "pause"]);
  }

  /// Seeks to an absolute position in seconds.
  pub fn seek_to(&self, seconds: f64) {
    self.run(&["seek", &format!("{seconds:.3}"), "absolute"]);
  }

  /// Seeks relative to the current position.
  pub fn seek_by(&self, seconds: f64) {
    self.run(&["seek", &format!("{seconds:.3}"), "relative"]);
  }

  /// Selects the audio track.
  pub fn set_audio(&self, choice: Choice) {
    self.set("aid", &choice.as_option());
  }

  /// Selects the subtitle track.
  pub fn set_subtitle(&self, choice: Choice) {
    self.set("sid", &choice.as_option());
  }

  /// Adds an external subtitle file or URL and selects it (`sub-add <url> select <title> <lang>`).
  pub fn add_subtitle(&self, url: &str, title: &str, lang: &str) {
    self.run(&["sub-add", url, "select", title, lang]);
  }

  /// Jumps `delta` chapters forward (negative: back).
  pub fn add_chapter(&self, delta: i64) {
    self.run(&["add", "chapter", &delta.to_string()]);
  }

  /// Mutes or unmutes.
  pub fn set_muted(&self, muted: bool) {
    self.set("mute", if muted { "yes" } else { "no" });
  }

  /// Volume, 0–100.
  pub fn set_volume(&self, percent: f64) {
    self.set("volume", &format!("{:.0}", percent.clamp(0.0, 100.0)));
  }

  /// Plays to another audio output ([`AudioDevice::name`]; `None`: Windows' default). mpv
  /// reopens its audio output, a short silence.
  pub fn set_audio_device(&self, device: Option<&str>) {
    self.set("audio-device", device.unwrap_or("auto"));
  }

  /// Subtitle delay in seconds.
  pub fn set_subtitle_delay(&self, seconds: f64) {
    self.set("sub-delay", &format!("{seconds:.3}"));
  }

  /// Reads an mpv property as text, for diagnostics (`video-codec`, `hwdec-current`,
  /// `frame-drop-count`…); `None` when unavailable. Never read or log `path` or
  /// `stream-open-filename`: stream URLs carry the user's IP.
  pub fn property(&self, name: &str) -> Option<String> {
    self.mpv.property(name)
  }

  fn set(&self, name: &str, value: &str) {
    if let Err(e) = self.mpv.set_property(name, value) {
      tracing::warn!("{e}");
    }
  }

  fn run(&self, args: &[&str]) {
    if let Err(e) = self.mpv.command(args) {
      tracing::warn!("{e}");
    }
  }
}

impl Drop for Player {
  fn drop(&mut self) {
    self.shared.quit.store(true, Ordering::SeqCst);
    self.shared.poke(false);
    self.mpv.wakeup();
    for thread in self.threads.drain(..) {
      let _ = thread.join();
    }
    // The last Arc<Mpv> goes here: render context first, then the handle.
  }
}

fn default_libmpv_path() -> PathBuf {
  let beside_exe = std::env::current_exe()
    .ok()
    .and_then(|exe| Some(exe.parent()?.join("libmpv-2.dll")));
  match beside_exe {
    Some(path) if path.exists() => path,
    _ => PathBuf::from(concat!(
      env!("CARGO_MANIFEST_DIR"),
      "/../../third_party/mpv/libmpv-2.dll"
    )),
  }
}

fn load_api(dll: &std::path::Path) -> Result<sys::Api, String> {
  let api = sys::Api::load(dll)?;
  api.check_version()?;
  Ok(api)
}

fn start_gpu(dll: &std::path::Path, options: &StartOptions) -> Result<(Mpv, VideoDevice), String> {
  let device = VideoDevice::new().map_err(|e| format!("D3D11 device: {e}"))?;
  let api = load_api(dll)?;
  let mpv = create_mpv(
    api,
    &mpv_options(options, Backend::Gpu),
    RenderBackend::D3d11 {
      device: device.as_raw(),
    },
  )?;
  Ok((mpv, device))
}

fn mpv_options(options: &StartOptions, backend: Backend) -> Vec<(&'static str, String)> {
  let mut opts: Vec<(&'static str, String)> = vec![
    ("vo", "libmpv".into()),
    (
      "hwdec",
      if options.software_decoding {
        "no"
      } else if backend == Backend::Gpu {
        "auto"
      } else {
        "auto-copy"
      }
      .into(),
    ),
    ("terminal", "no".into()),
    ("keep-open", "yes".into()),
    ("idle", "yes".into()),
    ("cache", "yes".into()),
    (
      "demuxer-max-bytes",
      format!("{}MiB", options.buffer_mb.unwrap_or(300).clamp(50, 4096)),
    ),
    ("demuxer-readahead-secs", "20".into()),
    ("network-timeout", "30".into()),
    ("embeddedfonts", "yes".into()),
    ("sub-ass-override", "no".into()),
    ("audio-channels", "auto-safe".into()),
    ("input-default-bindings", "no".into()),
  ];
  if options.start_seconds > 0.0 {
    opts.push(("start", format!("{:.3}", options.start_seconds)));
  }
  if let Some(scale) = options.subtitle_scale {
    opts.push(("sub-scale", format!("{:.2}", scale.clamp(0.5, 3.0))));
  }
  if let Some(dir) = &options.fonts_dir {
    opts.push(("sub-fonts-dir", dir.display().to_string()));
  }
  if let Some(audio) = options.audio {
    opts.push(("aid", audio.as_option()));
  }
  if let Some(sub) = options.subtitle {
    opts.push(("sid", sub.as_option()));
  }
  match &options.ytdl {
    Some(ytdl) => {
      opts.push(("ytdl", "yes".into()));
      if let Some(format) = &ytdl.format {
        opts.push(("ytdl-format", format.clone()));
      }
    }
    None => opts.push(("ytdl", "no".into())),
  }
  if let Some(device) = &options.audio_device {
    opts.push(("audio-device", device.clone()));
  }
  if let Some(path) = &options.log_file {
    opts.push(("log-file", path.display().to_string()));
  }
  opts
}

/// Creates mpv with owned option pairs.
fn create_mpv(
  api: sys::Api,
  options: &[(&'static str, String)],
  backend: RenderBackend,
) -> Result<Mpv, String> {
  let borrowed: Vec<(&str, &str)> = options.iter().map(|(k, v)| (*k, v.as_str())).collect();
  Mpv::new(api, &borrowed, backend)
}

fn render_gpu(
  mpv: &Mpv,
  shared: &Shared,
  device: &VideoDevice,
  frames: &async_channel::Sender<Frame>,
) {
  let mut target: Option<RenderTarget> = None;
  let mut ring: Vec<SharedTexture> = Vec::new();
  // The ring before, kept until the next one is made: the UI shows its last frame until the
  // current ring has one, and a freed texture would draw as nothing.
  let mut _previous: Vec<SharedTexture> = Vec::new();
  let mut size = (0, 0);
  let mut generation = 0u64;
  let mut next = 0usize;
  // When the size last changed, and whether a ring at the new size is still to make.
  let mut changed_at: Option<Instant> = None;
  let mut pending = false;
  loop {
    let timeout = changed_at
      .filter(|_| pending)
      .map(|at| RESIZE_SETTLE.saturating_sub(at.elapsed()));
    let Some(resized) = shared.wait(timeout) else {
      break;
    };
    let quiet = changed_at.is_none_or(|at| at.elapsed() >= RESIZE_SETTLE);
    if resized {
      changed_at = Some(Instant::now());
      pending = true;
    }
    let flags = mpv.render_update();
    let (width, height) = shared.target.lock().map(|t| *t).unwrap_or_default();
    if width == 0 || height == 0 {
      continue;
    }
    // At once after a quiet spell (or for the first ring), else once the size settles.
    let settled = changed_at.is_some_and(|at| at.elapsed() >= RESIZE_SETTLE);
    let due = target.is_none() || (pending && (quiet || settled));
    let recreate = due && (target.is_none() || size != (width, height));
    if due {
      pending = false;
    }
    if flags & sys::RENDER_UPDATE_FRAME == 0 && !recreate {
      continue;
    }
    if recreate {
      let created = device.create_target(width, height).and_then(|t| {
        let slots = (0..RING)
          .map(|_| device.create_shared(width, height))
          .collect::<windows::core::Result<Vec<_>>>()?;
        Ok((t, slots))
      });
      match created {
        Ok((t, slots)) => {
          target = Some(t);
          _previous = std::mem::replace(&mut ring, slots);
          size = (width, height);
          generation += 1;
          let handles = ring.iter().map(|t| t.handle).collect();
          if frames
            .try_send(Frame::GpuTextures {
              generation,
              width,
              height,
              handles,
            })
            .is_err()
          {
            return;
          }
        }
        Err(e) => {
          tracing::error!("cannot create video textures: {e}");
          continue;
        }
      }
    }
    let Some(target) = &target else { continue };
    // Blocks until the frame's display time; no shared texture is locked meanwhile.
    if let Err(e) = mpv.render_d3d11(target.as_raw()) {
      tracing::warn!("render: {e}");
      continue;
    }
    next = (next + 1) % RING;
    let slot = &ring[next];
    if !slot.acquire(50) {
      tracing::debug!("video slot busy, frame skipped");
      continue;
    }
    device.copy(slot, target);
    slot.release();
    if frames
      .try_send(Frame::GpuFrame {
        generation,
        index: next,
      })
      .is_err()
    {
      return;
    }
  }
}

fn render_software(mpv: &Mpv, shared: &Shared, frames: &async_channel::Sender<Frame>) {
  while let Some(resized) = shared.wait(None) {
    let flags = mpv.render_update();
    if flags & sys::RENDER_UPDATE_FRAME == 0 && !resized {
      continue;
    }
    let (width, height) = shared.target.lock().map(|t| *t).unwrap_or_default();
    if width == 0 || height == 0 {
      continue;
    }
    let mut pixels = vec![0u8; width as usize * height as usize * 4];
    if let Err(e) = mpv.render_sw(width, height, &mut pixels, true) {
      tracing::warn!("render: {e}");
      continue;
    }
    // `bgr0` leaves the 4th byte undefined; GPUI blends sprites, so force it opaque.
    for px in pixels.as_chunks_mut::<4>().0 {
      px[3] = 0xff;
    }
    // If the UI is behind, drop frames instead of queueing megabytes.
    if frames.len() > 1 {
      continue;
    }
    if frames
      .try_send(Frame::Cpu {
        width,
        height,
        pixels,
      })
      .is_err()
    {
      return;
    }
  }
}

fn event_loop(mpv: &Mpv, shared: &Shared, events: &async_channel::Sender<Event>) {
  let send = |event: Event| events.try_send(event).is_ok();
  let mut last_position = f64::NEG_INFINITY;
  while !shared.quitting() {
    let ok = match mpv.next_event(-1.0) {
      MpvEvent::Shutdown => return,
      MpvEvent::FileLoaded => send(Event::Loaded),
      MpvEvent::Double(name, Some(value)) if name == "time-pos" => {
        if (value - last_position).abs() >= POSITION_STEP {
          last_position = value;
          send(Event::Position(value))
        } else {
          true
        }
      }
      MpvEvent::Double(name, Some(value)) if name == "duration" => send(Event::Duration(value)),
      MpvEvent::Flag(name, Some(value)) if name == "pause" => send(Event::Paused(value)),
      MpvEvent::Flag(name, Some(value)) if name == "paused-for-cache" => {
        send(Event::Buffering(value))
      }
      MpvEvent::Flag(name, Some(value)) if name == "mute" => send(Event::Muted(value)),
      MpvEvent::Flag(name, Some(true)) if name == "eof-reached" => send(Event::EndReached),
      MpvEvent::Changed(name) if name == "track-list" => send(Event::Tracks(read_tracks(mpv))),
      MpvEvent::Changed(name) if name == "chapter-list" => {
        send(Event::Chapters(read_chapters(mpv)))
      }
      MpvEvent::EndFile { reason: 0, .. } => send(Event::Ended(None)),
      MpvEvent::EndFile { reason: 4, error } => {
        send(Event::Ended(Some(format!("playback error ({error})"))))
      }
      _ => true,
    };
    if !ok {
      return;
    }
  }
}

fn read_chapters(mpv: &Mpv) -> Vec<Chapter> {
  let count: usize = mpv
    .property("chapter-list/count")
    .and_then(|c| c.parse().ok())
    .unwrap_or(0);
  (0..count)
    .filter_map(|i| {
      Some(Chapter {
        start: mpv
          .property(&format!("chapter-list/{i}/time"))?
          .parse()
          .ok()?,
        title: mpv
          .property(&format!("chapter-list/{i}/title"))
          .filter(|t| !t.is_empty()),
      })
    })
    .collect()
}

fn read_tracks(mpv: &Mpv) -> Vec<Track> {
  let count: usize = mpv
    .property("track-list/count")
    .and_then(|c| c.parse().ok())
    .unwrap_or(0);
  (0..count)
    .filter_map(|i| {
      let p = |field: &str| mpv.property(&format!("track-list/{i}/{field}"));
      let kind = match p("type")?.as_str() {
        "video" => TrackKind::Video,
        "audio" => TrackKind::Audio,
        "sub" => TrackKind::Subtitle,
        _ => return None,
      };
      Some(Track {
        id: p("id")?.parse().ok()?,
        kind,
        ff_index: p("ff-index").and_then(|v| v.parse().ok()),
        external: p("external").as_deref() == Some("yes"),
        title: p("title"),
        lang: p("lang"),
        codec: p("codec"),
        selected: p("selected").as_deref() == Some("yes"),
      })
    })
    .collect()
}
