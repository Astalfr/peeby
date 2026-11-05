//! Spike: play a YouTube URL through peeby's player and the zero-copy `gpui-video` surface, and
//! measure the time from "play" to the first frame.
//!
//! Usage: `cargo run -p spike-yt-play --release -- <youtube-url>`
//! - `PEEBY_RESOLVER`: `visionos` (default: direct InnerTube call, muxed HLS, see visionos.rs)
//!   or `ytdl` (libmpv `ytdl_hook` → yt-dlp), or `raw`: the argument is a playlist URL played as
//!   it is, with `PEEBY_AUDIO` as its audio (a live's proxy, from `innertube`'s `live_probe --serve`).
//!   The options below are for `ytdl`.
//! - `PEEBY_START`: start position in seconds (visionos), to time a seek.
//! - Tools are bundled, nothing is installed on the host: `tools/` next to the executable, else
//!   `third_party/yt-dlp/` (filled by `scripts/fetch-tools.ps1`). `PATH` is only a last resort.
//! - `PEEBY_JS`: JS runtime for yt-dlp, `deno` (bundled, default) or `node` (from `PATH`).
//! - `PEEBY_SUB_LANGS`: subtitle languages for yt-dlp, default `none`: mpv opens external subtitles before playing (seconds lost), so load them on demand.
//! - `PEEBY_COOKIES`: optional cookies.txt for yt-dlp (age-restricted, members, Premium).
//! - `PEEBY_FORMAT`: yt-dlp format, default `bv*[height<=?1080]+ba/b`.
//! - `PEEBY_SOFTWARE_VIDEO=1`: force the software surface.
//! - `PEEBY_PIP=1`: after 4 s the video moves into an always-on-top borderless window (GPUI
//!   `WindowKind::PopUp`) at the bottom right of the screen, and back 6 s later: checks that the
//!   zero-copy surface draws in a second window (the mini-player, docs/refactor-plan.md).
//! - `PEEBY_DISPLAY=secondary`: opens the window on a screen that is not the primary one.
//! - The mpv log goes to `target/yt-play-mpv.log`.
//!
//! Keys: space pause · ←/→ seek 10 s · q quit.

// The spike reports its measurements on the console on purpose.
#![allow(clippy::print_stderr)]

mod visionos;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
  App, Bounds, Context, Entity, FocusHandle, KeyDownEvent, Task, Window, WindowBounds,
  WindowHandle, WindowKind, WindowOptions, div, point, prelude::*, px, rgb, size,
};
use gpui_platform::application;
use gpui_video::VideoView;
use player::{Event, Player, StartOptions, Ytdl};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn main() {
  let Some(url) = std::env::args().nth(1) else {
    eprintln!("usage: spike-yt-play <youtube-url>");
    std::process::exit(2);
  };

  let options = match std::env::var("PEEBY_RESOLVER").as_deref() {
    Ok("ytdl") => ytdl_options(url),
    // A playlist URL as it is, with `PEEBY_AUDIO` as its audio rendition (a proxy's).
    Ok("raw") => (
      StartOptions {
        url,
        external_audio: std::env::var("PEEBY_AUDIO").ok().into_iter().collect(),
        http_headers: vec![format!("User-Agent: {}", visionos::USER_AGENT)],
        log_file: Some(Path::new(ROOT).join("target/yt-play-mpv.log")),
        start_seconds: std::env::var("PEEBY_START")
          .ok()
          .and_then(|s| s.parse().ok())
          .unwrap_or(0.0),
        ..Default::default()
      },
      Instant::now(),
    ),
    _ => visionos_options(&url),
  };
  let started = options.1;
  let options = options.0;

  let (event_tx, event_rx) = async_channel::unbounded();
  let (frame_tx, frame_rx) = async_channel::unbounded();
  let player = match Player::start(options, event_tx, frame_tx) {
    Ok(p) => Rc::new(p),
    Err(e) => {
      eprintln!("cannot start the player: {e}");
      std::process::exit(1);
    }
  };
  eprintln!("backend: {:?}", player.backend());

  application().run(move |cx: &mut App| {
    let display = (std::env::var("PEEBY_DISPLAY").as_deref() == Ok("secondary"))
      .then(|| {
        let primary = cx.primary_display().map(|d| d.id());
        cx.displays()
          .into_iter()
          .find(|d| Some(d.id()) != primary)
          .map(|d| d.id())
      })
      .flatten();
    let bounds = Bounds::centered(display, size(px(1280.), px(720.)), cx);
    let opened = cx.open_window(
      WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display,
        ..Default::default()
      },
      |window, cx| {
        let resize = player.clone();
        let video = cx.new(|cx| {
          VideoView::new(
            frame_rx,
            Rc::new(move |w, h| resize.set_target_size(w, h)),
            cx,
          )
        });
        cx.new(|cx| Spike::new(player, video, event_rx, started, window, cx))
      },
    );
    if let Err(e) = opened {
      eprintln!("cannot open the window: {e}");
      cx.quit();
      return;
    }
    // The mini-player window closing must not end the spike: quit with the last window.
    cx.on_window_closed(|cx, _| {
      if cx.windows().is_empty() {
        cx.quit();
      }
    })
    .detach();
    cx.activate(true);
  });
}

#[derive(Default)]
struct Status {
  loaded_after: Option<Duration>,
  playing_after: Option<Duration>,
  position: f64,
  duration: f64,
  paused: bool,
  buffering: bool,
  tracks: Vec<String>,
  error: Option<String>,
}

struct Spike {
  player: Rc<Player>,
  video: Entity<VideoView>,
  focus: FocusHandle,
  status: Status,
  /// The always-on-top window showing the video, while it is open.
  pip: Option<WindowHandle<Pip>>,
  _events: Task<()>,
  _pip: Option<Task<()>>,
}

/// The always-on-top window: the video alone.
struct Pip {
  video: Entity<VideoView>,
}

impl Render for Pip {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .bg(rgb(0x000000))
      .child(self.video.clone())
  }
}

impl Spike {
  fn new(
    player: Rc<Player>,
    video: Entity<VideoView>,
    events: async_channel::Receiver<Event>,
    started: Instant,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let focus = cx.focus_handle();
    window.focus(&focus, cx);
    let task = cx.spawn(async move |this, cx| {
      while let Ok(event) = events.recv().await {
        let alive = this.update(cx, |spike, cx| {
          spike.apply(event, started.elapsed());
          cx.notify();
        });
        if alive.is_err() {
          break;
        }
      }
    });
    let pip = (std::env::var("PEEBY_PIP").as_deref() == Ok("1")).then(|| {
      cx.spawn_in(window, async move |this, cx| {
        cx.background_executor().timer(Duration::from_secs(4)).await;
        this
          .update_in(cx, |spike, window, cx| spike.open_pip(window, cx))
          .ok();
        cx.background_executor().timer(Duration::from_secs(6)).await;
        this.update(cx, |spike, cx| spike.close_pip(cx)).ok();
      })
    });
    Self {
      player,
      video,
      focus,
      status: Status::default(),
      pip: None,
      _events: task,
      _pip: pip,
    }
  }

  /// Moves the video into an always-on-top window at the bottom right of this window's screen.
  fn open_pip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let Some(display) = window.display(cx) else {
      return;
    };
    let screen = display.bounds();
    let pip_size = size(px(480.), px(270.));
    let origin = point(
      screen.origin.x + screen.size.width - pip_size.width - px(24.),
      screen.origin.y + screen.size.height - pip_size.height - px(72.),
    );
    let video = self.video.clone();
    let opened = cx.open_window(
      WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(origin, pip_size))),
        // Windows checks the bounds against this display (the primary one when unset) and
        // falls back to a default placement when they are outside it.
        display_id: Some(display.id()),
        kind: WindowKind::PopUp,
        titlebar: None,
        focus: false,
        ..Default::default()
      },
      |_, cx| cx.new(|_| Pip { video }),
    );
    match opened {
      Ok(handle) => {
        eprintln!("pip: opened");
        self.pip = Some(handle);
      }
      Err(e) => eprintln!("pip: cannot open the window: {e}"),
    }
    cx.notify();
  }

  /// Brings the video back into this window.
  fn close_pip(&mut self, cx: &mut Context<Self>) {
    if let Some(handle) = self.pip.take() {
      handle
        .update(cx, |_, window, _| window.remove_window())
        .ok();
      eprintln!("pip: closed");
    }
    cx.notify();
  }

  fn apply(&mut self, event: Event, elapsed: Duration) {
    let s = &mut self.status;
    match event {
      Event::Loaded => {
        s.loaded_after = Some(elapsed);
        eprintln!("loaded after {:.2} s", elapsed.as_secs_f64());
      }
      Event::Position(p) => {
        s.position = p;
        if p > 0.0 && s.playing_after.is_none() {
          s.playing_after = Some(elapsed);
          eprintln!("playing after {:.2} s", elapsed.as_secs_f64());
        }
      }
      Event::Duration(d) => s.duration = d,
      Event::Paused(p) => s.paused = p,
      Event::Buffering(b) => s.buffering = b,
      Event::Tracks(tracks) => {
        s.tracks = tracks
          .iter()
          .filter(|t| t.selected)
          .map(|t| format!("{:?} {}", t.kind, t.codec.as_deref().unwrap_or("?")))
          .collect();
        eprintln!("tracks: {}", s.tracks.join(", "));
      }
      Event::Ended(Some(e)) => {
        eprintln!("ended with error: {e}");
        s.error = Some(e);
      }
      Event::Ended(None)
      | Event::EndReached
      | Event::Chapters(_)
      | Event::Muted(_)
      | Event::Cached(_) => {}
    }
  }

  fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
    match event.keystroke.key.as_str() {
      "space" => self.player.toggle_pause(),
      "right" => self.player.seek_by(10.0),
      "left" => self.player.seek_by(-10.0),
      "q" => cx.quit(),
      _ => {}
    }
  }

  fn overlay(&self) -> String {
    let s = &self.status;
    if let Some(e) = &s.error {
      return format!("error: {e} (see target/yt-play-mpv.log)");
    }
    let secs =
      |d: Option<Duration>| d.map_or("…".into(), |d| format!("{:.2} s", d.as_secs_f64()));
    format!(
      "{:?} · loaded {} · playing {} · {:.0}/{:.0} s{}{} · {}",
      self.player.backend(),
      secs(s.loaded_after),
      secs(s.playing_after),
      s.position,
      s.duration,
      if s.paused { " · paused" } else { "" },
      if s.buffering { " · buffering" } else { "" },
      s.tracks.join(", "),
    )
  }
}

impl Render for Spike {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .relative()
      .bg(rgb(0x000000))
      .track_focus(&self.focus)
      .on_key_down(cx.listener(Self::on_key))
      .when(self.pip.is_none(), |el| el.child(self.video.clone()))
      .when(self.pip.is_some(), |el| {
        el.flex()
          .items_center()
          .justify_center()
          .text_color(rgb(0xffffff))
          .child("Dans le mini-lecteur")
      })
      .child(
        div()
          .absolute()
          .top_2()
          .left_2()
          .right_2()
          .p_2()
          .rounded_md()
          .bg(gpui::rgba(0x000000aa))
          .text_color(rgb(0xffffff))
          .text_xs()
          .child(self.overlay()),
      )
  }
}

/// Folder with the bundled yt-dlp and JS runtimes (layout of `scripts/fetch-tools.ps1`).
struct Tools {
  dir: Option<PathBuf>,
}

impl Tools {
  fn find() -> Self {
    let beside_exe = std::env::current_exe()
      .ok()
      .and_then(|exe| Some(exe.parent()?.join("tools")));
    let dir = [beside_exe, Some(Path::new(ROOT).join("third_party/yt-dlp"))]
      .into_iter()
      .flatten()
      .find(|dir| dir.join("yt-dlp.exe").exists());
    Self { dir }
  }

  fn yt_dlp(&self) -> Option<PathBuf> {
    Some(self.dir.as_ref()?.join("yt-dlp.exe"))
  }

  /// `--js-runtimes` value for `name`, pointing at the bundled binary.
  fn js_runtime(&self, name: &str) -> Option<String> {
    let file = match name {
      "deno" => "deno.exe",
      // Not bundled: let yt-dlp find it in PATH.
      other => return Some(other.to_owned()),
    };
    let path = self.dir.as_ref()?.join(file);
    path.exists().then(|| format!("{name}:{}", path.display()))
  }
}

/// yt-dlp through mpv's `ytdl_hook`: about 3 s of yt-dlp, plus up to 5 s of `available_at`.
fn ytdl_options(url: String) -> (StartOptions, Instant) {
  let tools = Tools::find();
  let cache_dir = Path::new(ROOT).join("target/yt-dlp-cache");
  let mut raw_options = vec![
    // Isolation from the host: no yt-dlp config file read, no cache written in its profile.
    "ignore-config=".to_owned(),
    format!("cache-dir={}", cache_dir.display()),
  ];
  let js = std::env::var("PEEBY_JS").unwrap_or_else(|_| "deno".into());
  match tools.js_runtime(&js) {
    Some(runtime) => raw_options.push(format!("js-runtimes={runtime}")),
    None => eprintln!("warning: no bundled {js}, yt-dlp will look for a JS runtime in PATH"),
  }
  // Without `sub-langs`, ytdl_hook asks for every language (all automatic captions included),
  // which delays the start by seconds.
  let langs = std::env::var("PEEBY_SUB_LANGS").unwrap_or_else(|_| "none".into());
  raw_options.push(format!("sub-langs={langs}"));
  if let Ok(cookies) = std::env::var("PEEBY_COOKIES") {
    raw_options.push(format!("cookies={cookies}"));
  }
  let log_file = Path::new(ROOT).join("target/yt-play-mpv.log");
  let ytdl_path = tools.yt_dlp();
  eprintln!(
    "yt-dlp: {}",
    ytdl_path
      .as_deref()
      .map_or("PATH (not bundled!)".into(), |p| p.display().to_string())
  );
  eprintln!("yt-dlp options: {}", raw_options.join(" "));
  eprintln!("mpv log: {}", log_file.display());

  let options = StartOptions {
    url,
    force_software: std::env::var("PEEBY_SOFTWARE_VIDEO").as_deref() == Ok("1"),
    ytdl: Some(Ytdl {
      ytdl_path,
      format: Some(
        std::env::var("PEEBY_FORMAT").unwrap_or_else(|_| "bv*[height<=?1080]+ba/b".into()),
      ),
      raw_options,
    }),
    log_file: Some(log_file),
    ..Default::default()
  };

  (options, Instant::now())
}

/// Direct InnerTube call (visionOS client). The visitorData request happens at app startup in the
/// real app, so the clock starts after it.
fn visionos_options(url: &str) -> (StartOptions, Instant) {
  let fail = |e: String| -> ! {
    eprintln!("{e}");
    std::process::exit(1)
  };
  let Some(video_id) = video_id(url) else {
    fail(format!("no video id in {url}"))
  };
  let warmup = Instant::now();
  let resolver = visionos::Resolver::new().unwrap_or_else(|e| fail(e));
  eprintln!(
    "visitorData (app startup): {:.2} s",
    warmup.elapsed().as_secs_f64()
  );
  let started = Instant::now();
  let stream = resolver
    .resolve(video_id, 1080)
    .unwrap_or_else(|e| fail(format!("visionos: {e}")));
  eprintln!(
    "resolved after {:.2} s: {} {}, audio rendition: {}",
    started.elapsed().as_secs_f64(),
    stream.resolution,
    stream.codecs,
    if stream.audio_url.is_some() {
      "yes"
    } else {
      "no"
    }
  );
  let options = StartOptions {
    url: stream.url,
    external_audio: stream.audio_url.into_iter().collect(),
    http_headers: vec![format!("User-Agent: {}", visionos::USER_AGENT)],
    force_software: std::env::var("PEEBY_SOFTWARE_VIDEO").as_deref() == Ok("1"),
    log_file: Some(Path::new(ROOT).join("target/yt-play-mpv.log")),
    start_seconds: std::env::var("PEEBY_START")
      .ok()
      .and_then(|s| s.parse().ok())
      .unwrap_or(0.0),
    ..Default::default()
  };
  (options, started)
}

/// `v=` of a watch URL, or the path of a youtu.be link.
fn video_id(url: &str) -> Option<&str> {
  let id = match url.split_once("v=") {
    Some((_, rest)) => rest,
    None => url.rsplit_once("youtu.be/")?.1,
  };
  id.split(['&', '?', '#']).next().filter(|id| id.len() == 11)
}
