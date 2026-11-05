//! Spike: play a Twitch live or VOD through peeby's player and measure the time from the click
//! (before the playback token) to the first frame (docs/twitch-plan.md, step 1).
//!
//! Usage: `cargo run -p spike-twitch-play --release -- [login | twitch.tv/videos/<id> | vod:<login>]`
//! - no argument: the top live channel; `vod:<login>`: that channel's latest VOD.
//! - `PEEBY_PLAYER_TYPE`: `playerType` of the token, default `embed` (`site` gets a pre-roll).
//! - `PEEBY_CODECS`: `supported_codecs`, default `h264` (e.g. `av1,h265,h264`).
//! - `PEEBY_VARIANT`: the variant to play by id (e.g. `1080p60-portrait`), else the best landscape one.
//! - `PEEBY_FIXTURES=1`: saves the GQL answer, redacted, in `crates/twitch/tests/fixtures/`.
//! - `PEEBY_QUIT_AFTER`: quits after that many seconds (unattended runs).
//! - `PEEBY_START`: start position in seconds (a VOD, e.g. a live's recording in progress).
//! - `PEEBY_SEEK=<after>:<back>`: `<after>` seconds in, seeks `<back>` seconds back (a live's kept
//!   minutes, `twitch::LIVE_KEPT`), and prints every 5 s the position, mpv's duration and how far
//!   behind it the playhead is.
//! - `PEEBY_LOW_LATENCY=0`: without Twitch's prefetch segments (default with them). A live prints,
//!   every 10 s, how long ago the segment FFmpeg starts at (third from the end) was captured.
//! - `PEEBY_DISPLAY=secondary`: opens the window on a screen that is not the primary one.
//! - The mpv log goes to `target/twitch-play-mpv.log` (it holds stream URLs: never share it).
//!
//! Keys: space pause · ←/→ seek 10 s (VODs) · q quit.

// The spike reports its measurements on the console on purpose.
#![allow(clippy::print_stderr)]

mod twitch;

use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
  App, Bounds, Context, FocusHandle, KeyDownEvent, Task, Window, WindowBounds, WindowOptions, div,
  prelude::*, px, rgb, size,
};
use gpui_platform::application;
use gpui_video::VideoView;
use player::{Event, Player, StartOptions};
use twitch::{Resolver, Target};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn main() {
  let fail = |e: String| -> ! {
    eprintln!("{e}");
    std::process::exit(1)
  };
  let resolver = Resolver::new().unwrap_or_else(|e| fail(e));
  let arg = std::env::args().nth(1);
  let target = match arg.as_deref() {
    None => Target::Live(resolver.top_live().unwrap_or_else(|e| fail(e))),
    Some(arg) => match arg.split_once("videos/") {
      Some((_, id)) => Target::Vod(id.trim_end_matches('/').to_owned()),
      None => match arg.strip_prefix("vod:") {
        Some(login) => Target::Vod(resolver.latest_vod(login).unwrap_or_else(|e| fail(e))),
        None => Target::Live(arg.rsplit('/').next().unwrap_or(arg).to_owned()),
      },
    },
  };
  match &target {
    Target::Live(login) => eprintln!("live: {login}"),
    Target::Vod(id) => eprintln!("vod: {id}"),
  }
  let live = matches!(target, Target::Live(_));

  let fixtures = (std::env::var("PEEBY_FIXTURES").as_deref() == Ok("1"))
    .then(|| Path::new(ROOT).join("crates/twitch/tests/fixtures"));
  let started = Instant::now();
  let url = if std::env::var("PEEBY_PROXY").as_deref() == Ok("1") {
    through_proxy(&target).unwrap_or_else(|e| fail(e))
  } else {
    let stream = resolver
      .resolve(&target, fixtures.as_deref())
      .unwrap_or_else(|e| fail(e));
    eprintln!(
      "variant {} ({}), {} variants, {} landscape",
      stream.name, stream.codecs, stream.variants.0, stream.variants.1
    );
    stream.url
  };

  let options = StartOptions {
    url,
    http_headers: vec![
      format!("Referer: {}", twitch::REFERER),
      format!("Origin: {}", twitch::REFERER),
    ],
    force_software: std::env::var("PEEBY_SOFTWARE_VIDEO").as_deref() == Ok("1"),
    log_file: Some(Path::new(ROOT).join("target/twitch-play-mpv.log")),
    start_seconds: std::env::var("PEEBY_START")
      .ok()
      .and_then(|s| s.parse().ok())
      .unwrap_or(0.0),
    ..Default::default()
  };
  let (event_tx, event_rx) = async_channel::unbounded();
  let (frame_tx, frame_rx) = async_channel::unbounded();
  let player = match Player::start(options, event_tx, frame_tx) {
    Ok(p) => Rc::new(p),
    Err(e) => fail(format!("cannot start the player: {e}")),
  };
  eprintln!("backend: {:?}", player.backend());
  let seek = std::env::var("PEEBY_SEEK").ok().and_then(|s| {
    let (after, back) = s.split_once(':')?;
    Some((after.parse::<u64>().ok()?, back.parse::<f64>().ok()?))
  });
  let watched = player.clone();
  let quit_after = std::env::var("PEEBY_QUIT_AFTER")
    .ok()
    .and_then(|s| s.parse().ok())
    .map(Duration::from_secs);

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
        cx.new(|cx| Spike::new(player, video, event_rx, started, live, window, cx))
      },
    );
    if let Err(e) = opened {
      eprintln!("cannot open the window: {e}");
      cx.quit();
      return;
    }
    if let Some((after, back)) = seek {
      let watched = watched.clone();
      cx.spawn(async move |cx| {
        let number = |name: &str| watched.property(name).and_then(|v| v.parse::<f64>().ok());
        for tick in 1..=120u64 {
          cx.background_executor().timer(Duration::from_secs(5)).await;
          if tick * 5 == after.next_multiple_of(5)
            && let Some(position) = number("time-pos")
          {
            eprintln!("seek {back} s back from {position:.1}");
            watched.seek_to(position - back);
          }
          let (position, duration) = (number("time-pos"), number("duration"));
          eprintln!(
            "t={:>3} s position {position:?} duration {duration:?} behind {:?}",
            tick * 5,
            position
              .zip(duration)
              .map(|(p, d)| ((d - p) * 10.0).round() / 10.0)
          );
        }
      })
      .detach();
    }
    if let Some(after) = quit_after {
      cx.spawn(async move |cx| {
        cx.background_executor().timer(after).await;
        cx.update(|cx| cx.quit());
      })
      .detach();
    }
    cx.on_window_closed(|cx, _| cx.quit()).detach();
    cx.activate(true);
  });
}

/// `PEEBY_PROXY=1`: the `twitch` crate's `Client::play` (main and backup tokens and masters
/// together, then the local proxy). The proxy runs on a thread of its own until the spike quits,
/// and reports ad breaks. Returns the best landscape variant's local URL.
fn through_proxy(target: &Target) -> Result<String, String> {
  let target = match target {
    Target::Live(login) => ::twitch::Target::Live(login.clone()),
    Target::Vod(id) => ::twitch::Target::Vod(id.clone()),
  };
  // `PEEBY_PLAYER_TYPE=site` gets the pre-roll, which the backup then covers.
  let main_type =
    std::env::var("PEEBY_PLAYER_TYPE").unwrap_or_else(|_| ::twitch::gql::PLAYER_TYPE.into());
  let low_latency = std::env::var("PEEBY_LOW_LATENCY").as_deref() != Ok("0");
  let (tx, rx) = std::sync::mpsc::channel();
  let started = Instant::now();
  std::thread::spawn(move || {
    let runtime = match tokio::runtime::Builder::new_current_thread()
      .enable_all()
      .build()
    {
      Ok(runtime) => runtime,
      Err(e) => {
        let _ = tx.send(Err(e.to_string()));
        return;
      }
    };
    runtime.block_on(async move {
      let client = match ::twitch::Client::new(None, None) {
        Ok(client) => client,
        Err(e) => {
          let _ = tx.send(Err(e.to_string()));
          return;
        }
      };
      let proxy = match client
        .play_with(
          &target,
          &[::twitch::usher::Codec::H264],
          &main_type,
          ::twitch::gql::BACKUP_PLAYER_TYPE,
          low_latency,
        )
        .await
      {
        Ok(proxy) => proxy,
        Err(e) => {
          let _ = tx.send(Err(e.to_string()));
          return;
        }
      };
      let best = proxy
        .variants()
        .iter()
        .find(|v| !v.portrait && !v.audio_only())
        .map(|v| (v.url.clone(), format!("{v:?}")));
      let playlist = best.as_ref().map(|(url, _)| url.clone());
      let _ = tx.send(best.ok_or_else(|| "no landscape variant".to_owned()));
      let mut last = proxy.ad_state();
      let mut ticks = 0u32;
      loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        ticks += 1;
        if ticks.is_multiple_of(20)
          && let Some(url) = &playlist
          && let Some(behind) = start_behind(url).await
        {
          eprintln!("start point: {behind:.1} s behind (low latency {low_latency})");
        }
        let state = proxy.ad_state();
        if state.active != last.active || state.covered != last.covered {
          eprintln!(
            "ad: active {}, covered {}, {:.1}/{:?} s",
            state.active, state.covered, state.elapsed, state.duration
          );
        }
        last = state;
      }
    });
  });
  let (url, name) = rx
    .recv()
    .map_err(|_| "the proxy thread stopped".to_owned())??;
  eprintln!(
    "proxy ready after {:.2} s: {name}",
    started.elapsed().as_secs_f64()
  );
  Ok(url)
}

/// How long ago the segment FFmpeg starts a live at (third from the end of `url`'s playlist) was
/// captured: its `PROGRAM-DATE-TIME`, or the last one plus the durations after it (prefetch
/// segments have none).
async fn start_behind(url: &str) -> Option<f64> {
  let text = reqwest::get(url).await.ok()?.text().await.ok()?;
  // (capture time in ms when known, duration in s) of each segment.
  let mut segments: Vec<(Option<i64>, f64)> = Vec::new();
  let mut date = None;
  let mut duration = 0.0;
  for line in text.lines() {
    if let Some(d) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
      date = ::twitch::time::epoch_millis(d);
    } else if let Some(d) = line.strip_prefix("#EXTINF:") {
      duration = d.split(',').next()?.parse().ok()?;
    } else if !line.starts_with('#') && !line.is_empty() {
      segments.push((date.take(), duration));
    }
  }
  let start = segments.len().checked_sub(3)?;
  // Extrapolate from the last dated segment at or before the start.
  let (dated, (millis, _)) = segments[..=start]
    .iter()
    .enumerate()
    .rev()
    .find_map(|(i, (d, len))| d.map(|d| (i, (d, *len))))?;
  let after: f64 = segments[dated..start].iter().map(|(_, len)| len).sum();
  let captured = millis as f64 / 1000. + after;
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .ok()?
    .as_secs_f64();
  Some(now - captured)
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
  video: gpui::Entity<VideoView>,
  focus: FocusHandle,
  live: bool,
  status: Status,
  _events: Task<()>,
}

impl Spike {
  fn new(
    player: Rc<Player>,
    video: gpui::Entity<VideoView>,
    events: async_channel::Receiver<Event>,
    started: Instant,
    live: bool,
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
    Self {
      player,
      video,
      focus,
      live,
      status: Status::default(),
      _events: task,
    }
  }

  fn apply(&mut self, event: Event, elapsed: Duration) {
    let s = &mut self.status;
    match event {
      Event::Loaded => {
        s.loaded_after = Some(elapsed);
        eprintln!("loaded after {:.2} s", elapsed.as_secs_f64());
      }
      Event::Position(p) => {
        // A live starts at a position past 0 (the window's start): the first change counts.
        if s.playing_after.is_none() && s.loaded_after.is_some() && p > 0.0 && p != s.position {
          s.playing_after = Some(elapsed);
          eprintln!("playing after {:.2} s", elapsed.as_secs_f64());
          // Whether the GPU decodes this codec (HEVC and AV1 checks, step 10).
          eprintln!(
            "decoder: {} · hwdec {}",
            self.player.property("video-codec").unwrap_or_default(),
            self.player.property("hwdec-current").unwrap_or_default()
          );
        }
        s.position = p;
      }
      Event::Duration(d) => {
        if s.duration == 0.0 {
          eprintln!("duration: {d:.0} s");
        }
        s.duration = d;
      }
      Event::Paused(p) => s.paused = p,
      Event::Buffering(b) => {
        if b != s.buffering {
          eprintln!(
            "{} at {:.2} s",
            if b { "buffering" } else { "buffered" },
            elapsed.as_secs_f64()
          );
        }
        s.buffering = b;
      }
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
      Event::Ended(None) => eprintln!("ended"),
      Event::EndReached | Event::Chapters(_) | Event::Muted(_) | Event::Cached(_) => {}
    }
  }

  fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
    match event.keystroke.key.as_str() {
      "space" => self.player.toggle_pause(),
      "right" if !self.live => self.player.seek_by(10.0),
      "left" if !self.live => self.player.seek_by(-10.0),
      "q" => cx.quit(),
      _ => {}
    }
  }

  fn overlay(&self) -> String {
    let s = &self.status;
    if let Some(e) = &s.error {
      return format!("error: {e} (see target/twitch-play-mpv.log)");
    }
    let secs =
      |d: Option<Duration>| d.map_or("…".into(), |d| format!("{:.2} s", d.as_secs_f64()));
    format!(
      "{} · {:?} · loaded {} · playing {} · {:.0}/{:.0} s{}{} · {}",
      if self.live { "LIVE" } else { "VOD" },
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
      .child(self.video.clone())
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
