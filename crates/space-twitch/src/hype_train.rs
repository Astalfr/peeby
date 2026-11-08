//! A live's Hype Train, pinned over its chat as Twitch shows it (ADR 0032): a bar in the
//! channel's color, filled with the level's progress, its level in a badge, the share done and
//! the level's time left, and that time along its edge. Unfolded, the level's progress in points,
//! the emotes it gives, the conductors and the channel's record. A train about to start ("3 more
//! contributions…") shows the same way, and an ended one stays [`ENDED_SHOWN`] with the levels it
//! completed and the emotes it unlocked. The state is `twitch::hype_train`'s, held by `points::PointsView`: read from GQL when
//! the page opens (and after each PubSub reconnection), then moved by the PubSub topic.
//!
//! When a train starts or levels up, the user's mascot (`assets/hype-train.webp`, its corners
//! rounded) crosses its bar bouncing as a ball, a whole turn in the air at each bounce: in from
//! the left, a few seconds 20 px from the bar's left edge with "The Hype Train is coming!" (or the
//! new level), out on the right ([`Intro`]; none with reduced motion).

use std::cell::Cell;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
  AnyElement, App, ElementId, FontWeight, Hsla, ImageSource, ObjectFit, RenderImage, SharedString,
  Window, div, img, px, relative,
};
use gpui_component::{h_flex, v_flex};
use twitch::hype_train::{Approaching, Conductor, Emote, HypeTrain, Source, State};

use ui::assets::Icon;
use ui::chat::{LEAD, Pinned, Unfolded};
use ui::i18n::t;
use ui::picture::{self, Picture};
use ui::{tf, theme};

use super::cards;
use super::points::{eased_share, percent};

/// How long an ended train stays pinned with the level it reached, from its end.
pub const ENDED_SHOWN: Duration = Duration::from_secs(30);
/// How long past a level's end (or an approach's) Peeby waits for Twitch's message before it
/// takes the train as ended (or the approach as gone).
pub const GRACE_MS: i64 = 10_000;
/// How long an approach lasts (Twitch's `kickoff.duration`, 5 minutes on every channel seen),
/// for its time line: Twitch gives only its end.
const APPROACH_MS: i64 = 5 * 60 * 1000;
/// Twitch's purple, for a channel without its own color.
const PURPLE: u32 = 0x9147ff;
/// A Golden Kappa Train's gold.
const GOLD: u32 = 0xe6a700;
/// Side of an emote reward, logical pixels.
const EMOTE: f32 = 28.;
/// Side of a conductor's avatar, logical pixels.
const AVATAR: f32 = 24.;
/// Height of the card's progress bar, logical pixels.
const BAR: f32 = 8.;

/// The mascot that crosses the bar when a train starts or levels up (the user's picture).
const MASCOT: &[u8] = include_bytes!("../assets/hype-train.webp");
/// The mascot's side on screen, logical pixels.
const MASCOT_SIDE: f32 = 28.;
/// Its side in the drawn steps, pixels: sharp up to a 3× scale.
const MASCOT_PIXELS: u32 = 96;
/// Steps of its turn.
const MASCOT_STEPS: usize = 48;
/// How long it takes to come in from the left to [`MASCOT_STOP`], seconds. Each step lasts
/// whole bounces, so that each starts and ends on the ground.
const INTRO_IN: f32 = 2. * HOP;
/// How long it stays there with its message.
const INTRO_HOLD: f32 = 4. * HOP;
/// How long it takes to leave on the right.
const INTRO_OUT: f32 = 1.2;
/// Where it stops: its picture's left edge from the bar's, logical pixels.
const MASCOT_STOP: f32 = 20.;
/// One bounce, seconds: the mascot makes a whole turn in the air at each.
const HOP: f32 = 0.7;
/// How high it bounces while it travels, and while it waits, logical pixels.
const HOP_HIGH: f32 = 22.;
const HOP_LOW: f32 = 16.;
/// The bar's height at rest (two lines of text), and what it grows by during the announcement
/// to give the bounces room, logical pixels.
const BAR_REST: f32 = 52.;
const BAR_GROWTH: f32 = 9.;
/// How long the bar takes to grow, and to shrink back at the end, seconds.
const BAR_EASE: f32 = 0.3;
/// How long it stays squashed on landing, as a share of a bounce, on each side of the contact.
const SQUASH: f32 = 0.14;
/// How much it flattens at the contact (its height loses this share, its width gains more than it).
const SQUASH_DEPTH: f32 = 0.22;
/// How much it stretches upward just after take-off and just before landing.
const STRETCH: f32 = 0.06;

/// The announcement playing over the train's bar: what it says, and since when. Its clock
/// starts once the mascot's steps are drawn; until then the bar stays covered.
#[derive(Clone, Debug)]
pub struct Intro {
  since: Cell<Option<Instant>>,
  text: SharedString,
  /// Space between the mascot and the text, logical pixels.
  gap: f32,
}

impl Intro {
  /// The whole animation, seconds.
  pub const LENGTH: f32 = INTRO_IN + INTRO_HOLD + INTRO_OUT;

  /// `text` beside the mascot, `gap` pixels from it.
  pub fn new(text: SharedString, gap: f32) -> Self {
    Self {
      since: Cell::new(None),
      text,
      gap,
    }
  }

  /// Seconds since it started; `None` while it waits for the mascot's steps (it starts at the
  /// first call that finds them drawn).
  fn elapsed(&self) -> Option<f32> {
    if self.since.get().is_none() && mascot_frames().is_some() {
      self.since.set(Some(Instant::now()));
    }
    Some(self.since.get()?.elapsed().as_secs_f32())
  }

  /// Whether it is still playing (or waiting to).
  pub fn playing(&self) -> bool {
    self.elapsed().is_none_or(|t| t < Self::LENGTH)
  }
}

/// The mascot's steps, once drawn.
fn mascot_frames() -> Option<&'static Vec<Arc<RenderImage>>> {
  MASCOT_FRAMES.get().filter(|f| !f.is_empty())
}

/// The mascot's turn, drawn once in the background ([`warm_up`]); empty when its picture cannot
/// be read.
static MASCOT_FRAMES: OnceLock<Vec<Arc<RenderImage>>> = OnceLock::new();

/// Draws the mascot's turn in the background, once per run, before a train needs it.
pub fn warm_up() {
  static STARTED: OnceLock<()> = OnceLock::new();
  if STARTED.set(()).is_err() {
    return;
  }
  std::thread::spawn(|| {
    // Rounded as an app icon: a fifth of its side.
    let frames = ui::spin::rotations(
      MASCOT,
      MASCOT_PIXELS,
      MASCOT_PIXELS as f32 / 5.,
      MASCOT_STEPS,
    )
    .unwrap_or_default();
    MASCOT_FRAMES.set(frames).ok();
  });
}

/// The announcement at `t` seconds: the mascot, turning on itself and bouncing, comes in from the
/// left, stops [`MASCOT_STOP`] pixels from the bar's left edge with `text` beside it, then leaves
/// on the right. A layer in the bar's colors covers the bar meanwhile. `None` once over, or while
/// its steps are not drawn yet ([`cover`] then).
fn intro_element(text: SharedString, gap: f32, t: f32, accent: Hsla) -> Option<AnyElement> {
  let frames = mascot_frames()?;
  if t >= Intro::LENGTH {
    return None;
  }
  let ease_out = |x: f32| 1. - (1. - x).powi(3);
  // Leaving: a little slower at first, then the same distance at each bounce.
  let leave = |x: f32| 0.25 * x * x + 0.75 * x;
  // The mascot's center as a share of the bar, and how far left of that point its box starts
  // (in its own widths): wholly left of the bar at first, wholly right of it at the end.
  let boxed = MASCOT_SIDE * std::f32::consts::SQRT_2;
  // Its box's left edge: a share of the bar plus pixels. Wholly left of the bar at first, its
  // picture's left edge at [`MASCOT_STOP`] pixels while it waits, wholly right of the bar at the
  // end (its box is wider than the picture: the turned corners).
  let stop = MASCOT_STOP - (boxed - MASCOT_SIDE) / 2.;
  let (share, offset) = if t < INTRO_IN {
    let p = ease_out(t / INTRO_IN);
    (0., -boxed + (stop + boxed) * p)
  } else if t < INTRO_IN + INTRO_HOLD {
    (0., stop)
  } else {
    let p = leave((t - INTRO_IN - INTRO_HOLD) / INTRO_OUT);
    (p, stop * (1. - p))
  };
  // It bounces as a ball on the bar's bottom edge: a parabola from contact to contact, high
  // while it travels, lower while it waits, a whole turn in the air at each bounce (the turn
  // eased: fast at take-off, slowing at the top, fast again into the landing), squashed at each
  // at each contact.
  let moving = t < INTRO_IN || t >= INTRO_IN + INTRO_HOLD;
  let phase = (t / HOP).fract();
  let height = if moving { HOP_HIGH } else { HOP_LOW };
  let lift = height * 4. * phase * (1. - phase);
  let turn = phase - (std::f32::consts::TAU * phase).sin() / (std::f32::consts::TAU * 1.6);
  let step = (turn.rem_euclid(1.) * frames.len() as f32) as usize % frames.len();
  let contact = phase.min(1. - phase);
  // Squishy: flattened and widened around the contact (eased, so that it sinks in and springs
  // back softly), then a little stretched upward as it leaves the ground and as it comes down.
  let squash = if contact < SQUASH {
    let k = 1. - contact / SQUASH;
    SQUASH_DEPTH * k * k * (3. - 2. * k)
  } else {
    0.
  };
  let stretch = if (SQUASH..SQUASH * 2.5).contains(&contact) {
    STRETCH * (std::f32::consts::PI * (contact - SQUASH) / (SQUASH * 1.5)).sin()
  } else {
    0.
  };
  let (width, height) = (1. + 1.1 * squash - 0.5 * stretch, 1. - squash + stretch);
  let frame = frames[step].clone();
  // The cover and the text fade in and out.
  let fade = |start: f32, length: f32| ((t - start) / length).clamp(0., 1.);
  let cover = fade(0., 0.2).min(1. - fade(Intro::LENGTH - 0.25, 0.25));
  let message = fade(INTRO_IN - 0.1, 0.25).min(1. - fade(INTRO_IN + INTRO_HOLD - 0.3, 0.3));
  Some(
    div()
      .size_full()
      .relative()
      .child(
        div()
          .absolute()
          .inset_0()
          .opacity(cover)
          .bg(theme::surface())
          .child(div().size_full().bg(accent.opacity(0.22))),
      )
      .child(
        div()
          .absolute()
          .top_0()
          .bottom_0()
          .left(px(MASCOT_STOP + MASCOT_SIDE + gap))
          .right_0()
          .flex()
          .items_center()
          .opacity(message)
          .child(
            div()
              .text_size(px(16.))
              .font_weight(FontWeight::BOLD)
              .truncate()
              .child(text),
          ),
      )
      .child(
        div()
          .absolute()
          .top_0()
          .bottom_0()
          .left(relative(share))
          .ml(px(offset))
          .w(px(boxed))
          .flex()
          .items_end()
          .justify_center()
          // On the bar's bottom edge, the turned picture's corners reaching below it.
          .pb(px(lift - (boxed - MASCOT_SIDE) / 2. + 4.))
          .child(
            img(ImageSource::Render(frame))
              // Stretched to its box: `img` keeps the picture's ratio otherwise, and the squash
              // would only shrink it.
              .object_fit(ObjectFit::Fill)
              .w(px(boxed * width))
              .h(px(boxed * height))
              .flex_none(),
          ),
      )
      .into_any_element(),
  )
}

/// The announcement's cover alone, while it waits for the mascot's steps: the bar does not show
/// its text before the announcement.
fn cover(accent: Hsla) -> AnyElement {
  div()
    .size_full()
    .bg(theme::surface())
    .child(div().size_full().bg(accent.opacity(0.22)))
    .into_any_element()
}

/// How much of [`BAR_GROWTH`] the bar has at `t`: it grows at the start (ease out) and shrinks
/// back at the end (ease in), 0 to 1.
fn bar_growth(t: f32) -> f32 {
  let grow = (t / BAR_EASE).clamp(0., 1.);
  let shrink = ((Intro::LENGTH - t) / BAR_EASE).clamp(0., 1.);
  let ease = |x: f32| 1. - (1. - x) * (1. - x);
  ease(grow).min(ease(shrink))
}

/// The pinned item for the channel's train or its approach, if one shows at `now` (milliseconds
/// since the epoch), with the announcement playing over the train's bar.
pub fn pinned(
  state: &State,
  unfolded: &Unfolded,
  intro: Option<&Intro>,
  now: i64,
  window: &mut Window,
  cx: &mut App,
) -> Option<Pinned> {
  if let Some(train) = state.train.as_ref().filter(|t| shown(t, now)) {
    let mut item = train_item(train, unfolded, now, window, cx);
    if let Some(intro) = intro {
      match intro.elapsed() {
        Some(t) => {
          item.overlay = intro_element(intro.text.clone(), intro.gap, t, item.accent);
          if item.overlay.is_some() {
            item.bar_height = Some(BAR_REST + BAR_GROWTH * bar_growth(t));
          }
          if t < Intro::LENGTH {
            window.request_animation_frame();
          }
        }
        // Covered until the mascot is ready, checked at each frame.
        None => {
          item.overlay = Some(cover(item.accent));
          window.request_animation_frame();
        }
      }
    }
    return Some(item);
  }
  let approaching = state.approaching.as_ref()?;
  Some(approaching_item(approaching, unfolded, now, window, cx))
}

/// The pinned item's id, while one shows.
pub fn shown_id(state: &State, now: i64) -> Option<&str> {
  match &state.train {
    Some(train) if shown(train, now) => Some(&train.id),
    _ => state.approaching.as_ref().map(|a| a.id.as_str()),
  }
}

/// A train shows while it runs and [`ENDED_SHOWN`] after its end.
fn shown(train: &HypeTrain, now: i64) -> bool {
  match train.ended_at {
    None => true,
    Some(end) => now - end < ENDED_SHOWN.as_millis() as i64,
  }
}

/// The share left of an ended train's [`ENDED_SHOWN`], 1 to 0.
fn ended_left(end: i64, now: i64) -> f32 {
  let shown = ENDED_SHOWN.as_millis() as f32;
  (1. - (now - end) as f32 / shown).clamp(0., 1.)
}

/// The train's color: gold for a Golden Kappa Train, else the channel's, else Twitch's purple.
fn accent(color: Option<&str>, golden: bool) -> Hsla {
  let rgb = if golden {
    GOLD
  } else {
    color
      .and_then(|hex| u32::from_str_radix(hex, 16).ok())
      .unwrap_or(PURPLE)
  };
  gpui::rgb(rgb).into()
}

/// What a train is called.
fn kind(golden: bool, treasure: bool) -> &'static str {
  if golden {
    "Golden Kappa Train"
  } else if treasure {
    t("Treasure Train", "Train au trésor")
  } else {
    "Hype Train"
  }
}

fn train_item(
  train: &HypeTrain,
  unfolded: &Unfolded,
  now: i64,
  window: &mut Window,
  cx: &mut App,
) -> Pinned {
  let accent = accent(train.color.as_deref(), train.golden_kappa);
  let level = train.progress.level;
  let ended = train.ended_at;
  let record = if train.record_beaten() {
    t(" · new record!", " · nouveau record !")
  } else {
    ""
  };
  let done = train.levels_done();
  let (title, status) = match ended {
    Some(_) if done == 0 => (
      t(
        "The train didn't complete level 1",
        "Le train n'a pas complété le niveau 1",
      )
      .to_owned(),
      t("Ended", "Terminé").to_owned(),
    ),
    Some(_) => (
      tf!(
        "Level {done} completed{record}",
        "Niveau {done} complété{record}"
      ),
      t("Ended", "Terminé").to_owned(),
    ),
    None => {
      let share = percent(format!("{:.0}", train.progress.share() * 100.));
      let left = train
        .remaining(now)
        .map(|s| format!(" · {}", ui::player::format_time(s as f64)))
        .unwrap_or_default();
      (
        tf!("Level {level}{record}", "Niveau {level}{record}"),
        format!("{share}{left}"),
      )
    }
  };
  // The level's progress slides there as contributions come; full once ended.
  let fill = match ended {
    Some(_) => 1.,
    None => eased_share(
      format!("hype-fill-{}", train.id),
      f64::from(train.progress.share()),
      window,
      cx,
    ),
  };
  Pinned {
    id: train.id.clone().into(),
    icon: Icon::Train,
    accent,
    kind: kind(train.golden_kappa, train.treasure).into(),
    title: title.into(),
    status: status.into(),
    body: unfolded
      .is(&train.id)
      .then(|| train_body(train, accent, window, cx)),
    time_left: match ended {
      Some(end) => Some(ended_left(end, now)),
      None => train.time_left(now),
    },
    lead: Some(match ended {
      Some(_) if done == 0 => train_badge(accent),
      Some(_) => level_badge(done, accent),
      None => level_badge(level, accent),
    }),
    fill: Some(fill),
    overlay: None,
    bar_height: None,
  }
}

fn approaching_item(
  approaching: &Approaching,
  unfolded: &Unfolded,
  now: i64,
  window: &mut Window,
  cx: &mut App,
) -> Pinned {
  let accent = accent(approaching.color.as_deref(), approaching.golden_kappa);
  let left = approaching.left();
  let title = if cards::plural(u64::from(left)) {
    tf!(
      "{left} more contributions start a {}",
      "Encore {left} contributions pour lancer un {}",
      kind(approaching.golden_kappa, approaching.treasure)
    )
  } else {
    tf!(
      "{left} more contribution starts a {}",
      "Encore {left} contribution pour lancer un {}",
      kind(approaching.golden_kappa, approaching.treasure)
    )
  };
  let time = approaching
    .expires_at
    .map(|end| ((end - now) / 1000).max(0))
    .map(|s| format!(" · {}", ui::player::format_time(s as f64)))
    .unwrap_or_default();
  let share = if approaching.goal == 0 {
    0.
  } else {
    approaching.count as f32 / approaching.goal as f32
  };
  Pinned {
    id: approaching.id.clone().into(),
    icon: Icon::Train,
    accent,
    kind: "Hype Train".into(),
    title: title.into(),
    status: tf!("Approaching{time}", "En approche{time}").into(),
    body: unfolded
      .is(&approaching.id)
      .then(|| approaching_body(approaching, accent, window, cx)),
    time_left: approaching
      .expires_at
      .map(|end| ((end - now) as f32 / APPROACH_MS as f32).clamp(0., 1.)),
    lead: Some(train_badge(accent)),
    fill: Some(eased_share(
      format!("hype-fill-{}", approaching.id),
      f64::from(share),
      window,
      cx,
    )),
    overlay: None,
    bar_height: None,
  }
}

/// The level in a square of the train's color, as Twitch's banner shows it.
fn level_badge(level: u32, accent: Hsla) -> AnyElement {
  badge(accent)
    .text_size(px(if level >= 100 { 10. } else { 13. }))
    .font_weight(FontWeight::BOLD)
    .child(level.to_string())
    .into_any_element()
}

/// The train's icon in a square of its color.
fn train_badge(accent: Hsla) -> AnyElement {
  let ink = ink(accent);
  badge(accent)
    .child(
      gpui_component::Icon::new(Icon::Train)
        .size(px(16.))
        .text_color(ink),
    )
    .into_any_element()
}

fn badge(accent: Hsla) -> gpui::Div {
  div()
    .size(px(LEAD))
    .flex()
    .items_center()
    .justify_center()
    .rounded(px(6.))
    .bg(accent)
    .text_color(ink(accent))
}

/// Text that reads on `color`: black on a light one, white on a dark one.
fn ink(color: Hsla) -> Hsla {
  let rgb = color.to_rgb();
  let luminance = 0.2126 * rgb.r + 0.7152 * rgb.g + 0.0722 * rgb.b;
  if luminance > 0.6 {
    gpui::black()
  } else {
    gpui::white()
  }
}

/// A bar of `share` in `accent`.
fn bar(share: f32, accent: Hsla) -> AnyElement {
  div()
    .h(px(BAR))
    .w_full()
    .rounded_full()
    .bg(theme::surface_hover())
    .overflow_hidden()
    .child(
      div()
        .h_full()
        .w(relative(share.clamp(0., 1.)))
        .rounded_full()
        .bg(accent),
    )
    .into_any_element()
}

fn label(text: impl Into<gpui::SharedString>) -> AnyElement {
  div()
    .text_xs()
    .font_weight(FontWeight::SEMIBOLD)
    .text_color(theme::muted())
    .child(text.into())
    .into_any_element()
}

fn note(text: impl Into<gpui::SharedString>) -> AnyElement {
  div()
    .text_xs()
    .text_color(theme::muted())
    .child(text.into())
    .into_any_element()
}

/// `1 point`, `1,200 points`.
fn points(n: u64) -> String {
  let count = cards::grouped(n);
  if cards::plural(n) {
    format!("{count} points")
  } else {
    format!("{count} point")
  }
}

fn train_body(train: &HypeTrain, accent: Hsla, window: &mut Window, cx: &mut App) -> AnyElement {
  let p = &train.progress;
  let level = p.level;
  let ended = train.ended_at.is_some();
  let share = if ended {
    1.
  } else {
    eased_share(
      format!("hype-bar-{}", train.id),
      f64::from(p.share()),
      window,
      cx,
    )
  };
  let record = match train.record {
    Some(_) if train.record_beaten() => {
      Some(t("New channel record!", "Nouveau record de la chaîne !").to_owned())
    }
    Some(record) => Some(tf!(
      "Channel record: level {record}",
      "Record de la chaîne : niveau {record}"
    )),
    None => None,
  };
  let total = points(p.total);
  v_flex()
    .gap_2()
    .child(
      v_flex()
        .gap_1()
        .child(
          h_flex()
            .justify_between()
            .gap_2()
            .child(
              div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(if ended {
                  let done = train.levels_done();
                  if cards::plural(u64::from(done)) {
                    tf!("{done} levels completed", "{done} niveaux complétés")
                  } else {
                    tf!("{done} level completed", "{done} niveau complété")
                  }
                } else {
                  tf!("Level {level}", "Niveau {level}")
                }),
            )
            .when(!ended, |row| {
              row.child(
                div()
                  .text_xs()
                  .text_color(theme::muted())
                  .child(format!("{} / {}", cards::grouped(p.value), points(p.goal))),
              )
            }),
        )
        .child(bar(share, accent))
        .child(note(match record {
          Some(record) => tf!("Total: {total} · {record}", "Total : {total} · {record}"),
          None => tf!("Total: {total}", "Total : {total}"),
        })),
    )
    // Under way, what the level gives; ended, what the train gave.
    .children(match ended {
      false => (!p.rewards.is_empty()).then(|| {
        rewards(
          tf!("Level {level} rewards", "Récompenses du niveau {level}"),
          &p.rewards,
          window,
          cx,
        )
      }),
      true => (!train.unlocked.is_empty()).then(|| {
        rewards(
          t("Emotes unlocked", "Emotes débloquées").into(),
          &train.unlocked,
          window,
          cx,
        )
      }),
    })
    .children((!train.conductors.is_empty()).then(|| conductors(&train.conductors, window, cx)))
    .when(train.golden_kappa, |el| {
      el.child(note(t(
        "Golden Kappa Train: its contributors get the Golden Kappa emote for 24 hours.",
        "Golden Kappa Train : ses contributeurs reçoivent l'emote Golden Kappa pour 24 heures.",
      )))
    })
    .when(train.treasure, |el| {
      el.child(note(t(
        "Treasure Train: subscriptions cost less while it runs.",
        "Train au trésor : les abonnements coûtent moins cher tant qu'il roule.",
      )))
    })
    .when(!ended, |el| {
      el.child(note(t(
        "Subscribe, gift subs or cheer bits to move it forward: each level unlocks its emotes for the contributors.",
        "Abonnez-vous, offrez des abonnements ou envoyez des bits pour le faire avancer : chaque niveau débloque ses emotes pour les contributeurs.",
      )))
    })
    .into_any_element()
}

fn approaching_body(
  approaching: &Approaching,
  accent: Hsla,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  let share = if approaching.goal == 0 {
    0.
  } else {
    approaching.count as f32 / approaching.goal as f32
  };
  let (count, goal) = (approaching.count, approaching.goal);
  v_flex()
    .gap_2()
    .child(
      v_flex()
        .gap_1()
        .child(div().text_xs().text_color(theme::muted()).child(tf!(
          "{count} of {goal} contributions",
          "{count} contributions sur {goal}"
        )))
        .child(bar(share, accent)),
    )
    .children((!approaching.rewards.is_empty()).then(|| {
      rewards(
        t("Level 1 rewards", "Récompenses du niveau 1").into(),
        &approaching.rewards,
        window,
        cx,
      )
    }))
    .child(note(t(
      "Subscribe, gift subs or cheer bits in time to start it.",
      "Abonnez-vous, offrez des abonnements ou envoyez des bits à temps pour le lancer.",
    )))
    .into_any_element()
}

/// A level's emotes under `title`, each named in its tooltip.
fn rewards(title: String, emotes: &[Emote], window: &mut Window, cx: &mut App) -> AnyElement {
  v_flex()
    .gap_1()
    .child(label(title))
    .child(
      h_flex()
        .flex_wrap()
        .gap_1()
        .children(emotes.iter().map(|emote| emote_image(emote, window, cx))),
    )
    .into_any_element()
}

fn emote_image(emote: &Emote, window: &mut Window, cx: &mut App) -> AnyElement {
  // The fake chat's are drawn in memory, under its own scheme.
  let url = if emote.id.contains("://") {
    emote.id.clone()
  } else {
    twitch::chat::irc::emote_url(&emote.id, 2)
  };
  let token: gpui::SharedString = emote.token.clone().into();
  div()
    .id(ElementId::Name(format!("hype-emote-{}", emote.id).into()))
    .size(px(EMOTE))
    .flex()
    .items_center()
    .justify_center()
    .children(picture::logo(
      &Picture::sized(move |_, _| url.clone()),
      EMOTE,
      EMOTE,
      window,
      cx,
    ))
    .tooltip(move |window, cx| {
      gpui_component::tooltip::Tooltip::new(token.clone()).build(window, cx)
    })
    .into_any_element()
}

/// The train's conductors: the top contributor of subscriptions, then of bits.
fn conductors(conductors: &[Conductor], window: &mut Window, cx: &mut App) -> AnyElement {
  v_flex()
    .gap_1()
    .child(label(t("Conductors", "Conducteurs")))
    .children(conductors.iter().map(|c| {
      let avatar = c
        .avatar
        .clone()
        .map(|url| picture::avatar(&Picture::sized(move |_, _| url.clone()), AVATAR, window, cx));
      h_flex()
        .gap_2()
        .items_center()
        .child(div().size(px(AVATAR)).flex_none().children(avatar))
        .child(
          div()
            .flex_1()
            .min_w_0()
            .text_sm()
            .truncate()
            .child(c.name.clone()),
        )
        .child(note(match c.source {
          Source::Subs => t("Subs", "Abonnements"),
          Source::Bits => "Bits",
          Source::Other => t("Other", "Autre"),
        }))
    }))
    .into_any_element()
}
