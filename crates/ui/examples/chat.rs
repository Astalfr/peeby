//! The chat panel alone, fed with generated messages: words, mentions, links, emotes (squares
//! drawn in memory, no network; some blink, some are rings drawn over the emote before them), badges, `/me`, notices, deletions and bans, at a busy channel's
//! rate. Every 5 s it prints the frames drawn and the slowest frame, to check that the list keeps
//! up at [`ui::chat::MAX_MESSAGES`] messages (docs/twitch-plan.md, step 9).
//!
//! `cargo run -p ui --example chat --release` · `PEEBY_RATE=50` messages a second (default 20) ·
//! `PEEBY_DISPLAY=secondary` opens it on another screen.

// The example reports its timings on the console on purpose.
#![allow(clippy::print_stderr)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  App, AppContext, Bounds, Hsla, TitlebarOptions, WindowBounds, WindowOptions, canvas, div, px,
  size,
};
use gpui_component::{Root, TitleBar};
use gpui_platform::application;
use ui::chat::fake::{Rng, Squares};
use ui::chat::{
  Author, Badge, ChatEvent, ChatMessage, ChatPanel, ChatPanelEvent, Composing, Emote, EmoteGroup,
  Fragment, MessageKind, split_text,
};
use ui::images;
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

fn picture(path: String) -> Picture {
  Picture::sized(move |w, h| format!("chat://{path}?w={w}&h={h}"))
}

fn message(n: u64, rng: &mut Rng) -> ChatMessage {
  let name = NAMES[rng.below(NAMES.len())];
  let colors: [Option<Hsla>; 4] = [
    None,
    Some(gpui::rgb(0x0000ff).into()),
    Some(gpui::rgb(0x9acd32).into()),
    Some(gpui::rgb(0xff69b4).into()),
  ];
  let badges = (0..rng.below(3))
    .map(|b| Badge {
      title: "Badge".into(),
      picture: picture(format!("badge/{b}")),
    })
    .collect();
  let mut fragments = Vec::new();
  for _ in 0..1 + rng.below(24) {
    match rng.below(12) {
      0 | 1 => {
        let kind = match rng.below(8) {
          0 => "anim",
          1 if !fragments.is_empty() => "over",
          _ => "emote",
        };
        fragments.push(Fragment::Emote(Emote {
          name: "Kappa".into(),
          picture: picture(format!("{kind}/{}", rng.below(20))),
          aspect: if kind == "emote" && rng.below(4) == 0 {
            2.
          } else {
            1.
          },
          overlay: kind == "over",
          modifiers: Default::default(),
        }));
      }
      2 if rng.below(4) == 0 => {
        fragments.extend(split_text(&format!("@{} ", NAMES[rng.below(NAMES.len())])))
      }
      3 if rng.below(8) == 0 => {
        fragments.extend(split_text("https://example.com/un/lien/assez/long "))
      }
      _ => fragments.push(Fragment::Text(
        format!("{} ", WORDS[rng.below(WORDS.len())]).into(),
      )),
    }
  }
  let mut message = ChatMessage::new(
    n.to_string(),
    Some(Author {
      id: name.into(),
      name: name.into(),
      color: colors[rng.below(colors.len())],
      badges,
      avatar: None,
      channel: None,
    }),
    fragments,
  );
  message.kind = match rng.below(40) {
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
        // The writing area too: what is sent comes back as a message, the picker has 60 emotes.
        let panel = cx.new(|cx| {
          let mut panel = ChatPanel::new(cx).with_composer(500);
          panel.set_composing(Composing::Ready, cx);
          let emotes = |kind: &'static str, count: usize| -> Vec<Emote> {
            (0..count)
              .map(|i| Emote {
                name: format!("{kind}{i}").into(),
                picture: picture(format!("{kind}/{i}")),
                aspect: if i % 7 == 0 { 2. } else { 1. },
                overlay: false,
                modifiers: Default::default(),
              })
              .collect()
          };
          panel.set_emotes(
            vec![
              EmoteGroup {
                title: "Channel".into(),
                emotes: emotes("emote", 40),
              },
              EmoteGroup {
                title: "Animated".into(),
                emotes: emotes("anim", 20),
              },
            ],
            cx,
          );
          panel
        });
        cx.subscribe(&panel, |panel, event: &ChatPanelEvent, cx| {
          let ChatPanelEvent::Send(text) = event else {
            return;
          };
          let me = Author {
            id: "me".into(),
            name: "me".into(),
            color: None,
            badges: Vec::new(),
            avatar: None,
            channel: None,
          };
          let echo = ChatMessage::new(format!("me-{text}"), Some(me), split_text(text));
          panel.update(cx, |p, cx| p.push([ChatEvent::Message(echo)], cx));
        })
        .detach();
        let feed = panel.downgrade();
        cx.spawn(async move |cx| {
          let mut rng = Rng::new(0x9e37_79b9_7f4a_7c15);
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
      .bg(theme::surface())
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
