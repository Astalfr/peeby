//! The shared player alone, in the full-window layout, on a generated video (mpv's `lavfi`): a
//! test pattern with two audio tracks, an intro segment, previous/next buttons and an "up next"
//! offer, as the Jellyfin space sets them. Its events are printed.
//!
//! `cargo run -p ui --example player` · `PEEBY_START=150` starts in the credits ("up next") ·
//! `PEEBY_DISPLAY=secondary` opens it on another screen · `PEEBY_SIZE=640x360` sets the window's
//! size (panels larger than the player scroll).

// The example reports the player's events on the console on purpose.
#![allow(clippy::print_stderr)]

use std::time::Instant;

use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, TitleBar};
use gpui_platform::application;
use player::StartOptions;
use ui::player::segments::{Segment, SegmentKind};
use ui::player::{Layout, PlayerEvent, PlayerView, Prefs, Source};
use ui::{assets, theme};

/// Length of the generated video, seconds.
const DURATION: u32 = 180;

fn main() {
  let start = std::env::var("PEEBY_START")
    .ok()
    .and_then(|s| s.parse::<f64>().ok())
    .unwrap_or(0.0);
  application()
    .with_assets(assets::Assets)
    .run(move |cx: &mut App| {
      gpui_component::init(cx);
      theme::apply(cx);
      let display = (std::env::var("PEEBY_DISPLAY").as_deref() == Ok("secondary"))
        .then(|| {
          let primary = cx.primary_display().map(|d| d.id());
          cx.displays()
            .into_iter()
            .find(|d| Some(d.id()) != primary)
            .map(|d| d.id())
        })
        .flatten();
      let (width, height) = std::env::var("PEEBY_SIZE")
        .ok()
        .and_then(|s| {
          let (w, h) = s.split_once('x')?;
          Some((w.parse::<f32>().ok()?, h.parse::<f32>().ok()?))
        })
        .unwrap_or((1280., 720.));
      let bounds = Bounds::centered(display, size(px(width), px(height)), cx);
      let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_min_size: Some(size(px(320.), px(180.))),
        display_id: display,
        titlebar: Some(TitlebarOptions {
          title: Some("Peeby player".into()),
          ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
      };
      let opened = cx.open_window(options, |window, cx| {
        let player = cx.new(|cx| {
          let mut view = PlayerView::new(
            Prefs {
              autoplay_countdown: 5,
              ..Prefs::default()
            },
            false,
            cx,
          );
          view.set_layout(Layout::Window, cx);
          view.set_title("Mire de test".into(), cx);
          view.set_subtitle("S1:E1 · Générée par mpv".into(), cx);
          view.set_queue(Some((false, true)), cx);
          view.set_up_next(Some("S1:E2 · La suite".into()), cx);
          view.set_segments(
            vec![
              Segment {
                kind: SegmentKind::Intro,
                start: 5.0,
                end: 40.0,
              },
              Segment {
                kind: SegmentKind::Outro,
                start: 150.0,
                end: f64::from(DURATION),
              },
            ],
            cx,
          );
          view.start(source(start), Instant::now(), cx);
          view
        });
        cx.subscribe(&player, |_, event: &PlayerEvent, cx| match event {
          PlayerEvent::Close => {
            eprintln!("event: close");
            cx.quit();
          }
          other => eprintln!("event: {}", describe(other)),
        })
        .detach();
        window.focus(&player.read(cx).focus_handle().clone(), cx);
        cx.new(|cx| Root::new(player, window, cx))
      });
      if let Err(e) = opened {
        eprintln!("cannot open the window: {e}");
        cx.quit();
      }
    });
}

/// A test pattern and two tones as separate audio tracks.
fn source(start: f64) -> Source {
  Source {
    options: StartOptions {
      url: format!("av://lavfi:testsrc2=size=1280x720:rate=30:duration={DURATION}"),
      external_audio: vec![
        format!("av://lavfi:sine=frequency=440:duration={DURATION}"),
        format!("av://lavfi:sine=frequency=660:duration={DURATION}"),
      ],
      start_seconds: start,
      ..StartOptions::default()
    },
    audio_names: vec!["La 440 Hz".into(), "Mi 660 Hz".into()],
    ..Source::default()
  }
}

fn describe(event: &PlayerEvent) -> String {
  match event {
    PlayerEvent::Ended => "ended".into(),
    PlayerEvent::Next => "next".into(),
    PlayerEvent::Previous => "previous".into(),
    PlayerEvent::Theater => "theater".into(),
    PlayerEvent::Sound { volume, muted } => format!("sound {volume} muted={muted}"),
    PlayerEvent::Started => "started".into(),
    PlayerEvent::Paused(paused) => format!("paused={paused}"),
    PlayerEvent::Seeked => "seeked".into(),
    PlayerEvent::TracksChanged { audio, subtitle } => {
      format!("tracks audio={audio:?} subtitle={subtitle:?}")
    }
    PlayerEvent::Close => "close".into(),
  }
}
