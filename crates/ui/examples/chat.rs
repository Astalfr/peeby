//! The chat panel alone, fed with generated messages: words, mentions, links, emotes (squares
//! drawn in memory, no network; some blink, some are rings drawn over the emote before them), badges, `/me`, notices, deletions and bans, at a busy channel's
//! rate. Every 5 s it prints the frames drawn and the slowest frame, to check that the list keeps
//! up at [`ui::chat::MAX_MESSAGES`] messages (docs/twitch-plan.md, step 9).
//!
//! `cargo run -p ui --example chat --release` · `PEEBY_RATE=50` messages a second (default 20) ·
//! `PEEBY_DISPLAY=secondary` opens it on another screen.

// The example reports its timings on the console on purpose.
#![allow(clippy::print_stderr)]

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  App, AppContext, Bounds, Hsla, TitlebarOptions, WindowBounds, WindowOptions, canvas, div, px,
  size,
};
use gpui_component::{Root, TitleBar};
use gpui_platform::application;
use ui::chat::{
  Author, Badge, ChatEvent, ChatMessage, ChatPanel, Emote, Fragment, MessageKind, split_text,
};
use ui::images::{self, Download, Fetch};
use ui::picture::Picture;
use ui::{assets, theme};

const WORDS: &[&str] = &[
  "salut",
  "le",
  "chat",
  "c'est",
  "incroyable",
  "gg",
  "wp",
  "quel",
  "match",
  "franchement",
  "non",
  "mais",
  "regardez",
  "ça",
  "trop",
  "fort",
  "il",
  "a",
  "raté",
  "encore",
  "une",
  "fois",
  "allez",
  "on",
  "y",
  "croit",
  "😂",
  "🔥",
  "Pog",
];
const NAMES: &[&str] = &[
  "viewer1",
  "Viewer_Deux",
  "troisieme",
  "QuatreQuatre",
  "cinq5",
  "six_six",
  "SeptSept",
  "huit",
];

/// Draws emotes and badges: `chat://emote/<n>?w=..&h=..` is a colored square, as PNG;
/// `chat://anim/…` a square that blinks (a GIF); `chat://over/…` a ring drawn over an emote.
struct Squares;

impl Fetch for Squares {
  fn fetch(&self, url: &str) -> Download {
    let seed = url
      .bytes()
      .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    let (animated, over) = (
      url.starts_with("chat://anim/"),
      url.starts_with("chat://over/"),
    );
    Box::pin(async move {
      if animated || over {
        let mut bytes = Vec::new();
        {
          let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
          encoder
            .set_repeat(image::codecs::gif::Repeat::Infinite)
            .map_err(|e| e.to_string())?;
          for step in 0..4u32 {
            let frame = image::RgbaImage::from_fn(56, 56, |x, y| {
              let ring = (x as i32 - 28).pow(2) + (y as i32 - 28).pow(2);
              if over {
                let radius = 8 + step as i32 * 5;
                let on = (ring - radius * radius).abs() < 60;
                image::Rgba(if on { [255, 255, 0, 255] } else { [0, 0, 0, 0] })
              } else {
                let shade = 60 + step as u8 * 60;
                image::Rgba([shade, (seed >> 8) as u8, 255 - shade, 255])
              }
            });
            let delay = image::Delay::from_numer_denom_ms(150, 1);
            encoder
              .encode_frame(image::Frame::from_parts(frame, 0, 0, delay))
              .map_err(|e| e.to_string())?;
          }
        }
        return Ok(bytes);
      }
      let mut image = image::RgbaImage::new(56, 56);
      let color = [
        (seed >> 16) as u8,
        (seed >> 8) as u8,
        seed as u8 | 0x40,
        255,
      ];
      for (x, y, px) in image.enumerate_pixels_mut() {
        let edge = x < 4 || y < 4 || x > 51 || y > 51;
        *px = image::Rgba(if edge { [255, 255, 255, 255] } else { color });
      }
      let mut bytes = Vec::new();
      image
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
      Ok(bytes)
    })
  }
}

/// A small pseudo-random generator, so that the example needs no crate.
struct Rng(u64);

impl Rng {
  fn next(&mut self, below: usize) -> usize {
    self.0 ^= self.0 << 13;
    self.0 ^= self.0 >> 7;
    self.0 ^= self.0 << 17;
    (self.0 % below as u64) as usize
  }
}

fn picture(path: String) -> Picture {
  Picture::sized(move |w, h| format!("chat://{path}?w={w}&h={h}"))
}

fn message(n: u64, rng: &mut Rng) -> ChatMessage {
  let name = NAMES[rng.next(NAMES.len())];
  let colors: [Option<Hsla>; 4] = [
    None,
    Some(gpui::rgb(0x0000ff).into()),
    Some(gpui::rgb(0x9acd32).into()),
    Some(gpui::rgb(0xff69b4).into()),
  ];
  let badges = (0..rng.next(3))
    .map(|b| Badge {
      title: "Badge".into(),
      picture: picture(format!("badge/{b}")),
    })
    .collect();
  let mut fragments = Vec::new();
  for _ in 0..1 + rng.next(24) {
    match rng.next(12) {
      0 | 1 => {
        let kind = match rng.next(8) {
          0 => "anim",
          1 if !fragments.is_empty() => "over",
          _ => "emote",
        };
        fragments.push(Fragment::Emote(Emote {
          name: "Kappa".into(),
          picture: picture(format!("{kind}/{}", rng.next(20))),
          aspect: if kind == "emote" && rng.next(4) == 0 {
            2.
          } else {
            1.
          },
          overlay: kind == "over",
        }));
      }
      2 if rng.next(4) == 0 => {
        fragments.extend(split_text(&format!("@{} ", NAMES[rng.next(NAMES.len())])))
      }
      3 if rng.next(8) == 0 => {
        fragments.extend(split_text("https://example.com/un/lien/assez/long "))
      }
      _ => fragments.push(Fragment::Text(
        format!("{} ", WORDS[rng.next(WORDS.len())]).into(),
      )),
    }
  }
  let mut message = ChatMessage::new(
    n.to_string(),
    Some(Author {
      id: name.into(),
      name: name.into(),
      color: colors[rng.next(colors.len())],
      badges,
    }),
    fragments,
  );
  message.kind = match rng.next(40) {
    0 => MessageKind::Action,
    1 => MessageKind::Highlighted,
    2 => MessageKind::Notice(format!("{name} s'est abonné pour 12 mois !").into()),
    _ => MessageKind::Normal,
  };
  message
}

fn main() {
  let rate = std::env::var("PEEBY_RATE")
    .ok()
    .and_then(|s| s.parse::<u64>().ok())
    .unwrap_or(20)
    .max(1);
  application()
    .with_assets(assets::Assets)
    .run(move |cx: &mut App| {
      gpui_component::init(cx);
      theme::apply(cx);
      if let Err(e) = ui::runtime::init(cx) {
        eprintln!("cannot start the network runtime: {e}");
        cx.quit();
        return;
      }
      images::init(Arc::new(Squares), cx);
      let display = (std::env::var("PEEBY_DISPLAY").as_deref() == Ok("secondary"))
        .then(|| {
          let primary = cx.primary_display().map(|d| d.id());
          cx.displays()
            .into_iter()
            .find(|d| Some(d.id()) != primary)
            .map(|d| d.id())
        })
        .flatten();
      let bounds = Bounds::centered(display, size(px(400.), px(800.)), cx);
      let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display,
        window_background: gpui::WindowBackgroundAppearance::Opaque,
        titlebar: Some(TitlebarOptions {
          title: Some("Peeby chat".into()),
          ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
      };
      let opened = cx.open_window(options, |window, cx| {
        let panel = cx.new(ChatPanel::new);
        let feed = panel.downgrade();
        cx.spawn(async move |cx| {
          let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
          let mut n = 0u64;
          let started = Instant::now();
          feed
            .update(cx, |p, cx| {
              p.push(
                [
                  ChatEvent::Status(None),
                  ChatEvent::State(vec!["Mode lent : 3 s".into(), "Emotes uniquement".into()]),
                ],
                cx,
              )
            })
            .ok();
          loop {
            // One batch per frame-ish tick, as a space drains its channel.
            cx.background_executor()
              .timer(Duration::from_millis(50))
              .await;
            let due = started.elapsed().as_millis() as u64 * rate / 1000;
            let mut events = Vec::new();
            while n < due {
              events.push(ChatEvent::Message(message(n, &mut rng)));
              if n % 37 == 36 {
                events.push(ChatEvent::Delete((n - 3).to_string().into()));
              }
              if n % 251 == 250 {
                events.push(ChatEvent::ClearUser("huit".into()));
              }
              n += 1;
            }
            let push = Instant::now();
            if feed.update(cx, |p, cx| p.push(events, cx)).is_err() {
              return;
            }
            let took = push.elapsed();
            if took > Duration::from_millis(4) {
              eprintln!("slow push: {took:?}");
            }
          }
        })
        .detach();
        let host = cx.new(|_| Host {
          panel,
          frames: Frames::default(),
        });
        cx.new(|cx| Root::new(host, window, cx))
      });
      if let Err(e) = opened {
        eprintln!("cannot open the window: {e}");
        cx.quit();
      }
    });
}

/// Frame timings: count and slowest interval, printed every 5 s.
#[derive(Default)]
struct Frames {
  last: Option<Instant>,
  since: Option<Instant>,
  count: u32,
  slowest: Duration,
}

impl Frames {
  fn tick(&mut self) {
    let now = Instant::now();
    if let Some(last) = self.last {
      self.slowest = self.slowest.max(now - last);
    }
    self.last = Some(now);
    self.count += 1;
    let since = *self.since.get_or_insert(now);
    if now - since >= Duration::from_secs(5) {
      eprintln!(
        "frames: {} in 5 s, slowest {:.1} ms",
        self.count,
        self.slowest.as_secs_f64() * 1000.
      );
      self.since = Some(now);
      self.count = 0;
      self.slowest = Duration::ZERO;
    }
  }
}

/// The panel, with a canvas that counts frames.
struct Host {
  panel: gpui::Entity<ChatPanel>,
  frames: Frames,
}

impl Render for Host {
  fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
    let host = cx.entity().downgrade();
    div()
      .size_full()
      .bg(theme::background())
      .p_2()
      .child(self.panel.clone())
      .child(
        canvas(
          |_, _, _| {},
          move |_, _, window, cx| {
            host.update(cx, |h, _| h.frames.tick()).ok();
            window.request_animation_frame();
          },
        )
        .absolute()
        .size_0(),
      )
  }
}
