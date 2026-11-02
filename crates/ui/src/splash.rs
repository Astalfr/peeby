//! The opening animation (docs/design.md, "Motion"): the logo's three cards fall one after the
//! other, spinning into their angles, then the portrait's card lands on them. Every landing
//! squashes the stack and bounces it, the portrait's hardest, and "Peeby" springs up under it.
//! The settled stack fades into the real logo, then the whole screen fades out while the logo
//! zooms in.
//!
//! Everything is a function of the time since the first frame, so one render draws any moment.
//! Only SVGs turn in GPUI (`Transformation`), so the cards are a one-color mask
//! ([`assets::LOGO_CARD`]) and the portrait, already turned, a picture that moves and stretches.
//! Their places are those of docs/logo/onglets.html, whose render gives `logo.png` and the
//! portrait layer.

use std::f32::consts::PI;
use std::sync::Arc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
  AnyElement, FontWeight, ImageSource, ObjectFit, RenderImage, Transformation, Window, div, img,
  point, px, radians, rgb, size, svg,
};

use crate::{assets, images, theme};

const PORTRAIT_PNG: &[u8] = include_bytes!("../assets/logo-portrait.png");

/// The logo's side on screen, in logical pixels.
const SIZE: f32 = 220.;

// The logo's drawing, in onglets.html's units: its 512 px canvas maps `p` to
// `256 + FIT · (p − CENTER)`.
const FIT: f32 = 0.95;
const CENTER: (f32, f32) = (271., 257.);
/// A card's side before `FIT`.
const CARD: f32 = 360.;
/// Where the stack rests, in canvas units: squashes keep this point still.
const FLOOR: (f32, f32) = (256., 462.);

/// A layer of the logo and how it comes in.
struct Layer {
  /// Its center, in onglets.html's units.
  center: (f32, f32),
  /// Its resting angle, in degrees.
  angle: f32,
  /// Turn added at the top of the fall, in degrees (cards only).
  spin: f32,
  /// When it starts falling, in seconds.
  start: f32,
  /// How long it falls.
  fall: f32,
  /// How hard its landing squashes the stack (0.1 = 10 %).
  impact: f32,
  /// How high it bounces back after landing, in canvas units.
  rebound: f32,
}

impl Layer {
  fn landing(&self) -> f32 {
    self.start + self.fall
  }
}

/// The cards, back to front (Twitch, Jellyfin, YouTube), then the portrait.
const CARDS: [(u32, Layer); 3] = [
  (
    0x9146ff,
    Layer {
      center: (314., 244.),
      angle: 16.,
      spin: -38.,
      start: 0.10,
      fall: 0.32,
      impact: 0.12,
      rebound: 34.,
    },
  ),
  (
    0x00a4dc,
    Layer {
      center: (296., 258.),
      angle: 9.,
      spin: 30.,
      start: 0.30,
      fall: 0.32,
      impact: 0.12,
      rebound: 34.,
    },
  ),
  (
    0xff4e45,
    Layer {
      center: (278., 272.),
      angle: 2.,
      spin: -26.,
      start: 0.50,
      fall: 0.32,
      impact: 0.13,
      rebound: 34.,
    },
  ),
];
const PORTRAIT: Layer = Layer {
  center: (230., 276.),
  angle: -7.,
  spin: 0.,
  start: 0.75,
  fall: 0.36,
  impact: 0.22,
  rebound: 46.,
};

// Springs: e^(−DAMP·τ) times a wave, τ the time since the landing.
/// How fast the stack's squash dies out, and its pulsation.
const SQUASH_DAMP: f32 = 9.;
const SQUASH_PULSE: f32 = 26.;
/// Share of the squash given back as width.
const SQUASH_WIDEN: f32 = 0.5;
/// How fast a layer's rebounds die out, and how long the first one lasts.
const REBOUND_DAMP: f32 = 6.;
const REBOUND_PERIOD: f32 = 0.16;
/// A landed card's wobble around its angle, in degrees, and its pulsation.
const WOBBLE: f32 = 6.;
const WOBBLE_PULSE: f32 = 22.;
/// The whole stack's hop after the portrait lands, in canvas units, and how long it lasts.
const HOP: f32 = 28.;
const HOP_DELAY: f32 = 0.05;
const HOP_PERIOD: f32 = 0.22;
/// Stretch of a falling layer at the end of its fall (height; width loses half of it).
const STRETCH: f32 = 0.10;

/// When the drawn stack starts fading into `logo.png`, and for how long.
const SETTLED: f32 = 1.80;
const CROSSFADE: f32 = 0.15;
/// When the screen starts fading out (or a click), and for how long.
const LEAVE: f32 = 2.05;
const FADE_OUT: f32 = 0.30;
/// The logo's size at the end of the fade out, easing out from 1.
const ZOOM: f32 = 1.3;

/// Where the animation's clock stands.
enum Clock {
  /// Not drawn yet: the clock starts at the first frame, so that a slow start skips nothing.
  Waiting,
  Started(Instant),
  /// Always this moment (the example's still frames).
  Frozen(f32),
}

/// The opening animation over the window, until it ends or is skipped.
pub struct Splash {
  clock: Clock,
  /// When a click asked to leave, in the animation's time.
  skipped: Option<f32>,
  /// The portrait's layer, decoded once for a device-pixel size.
  portrait: Decoded,
  /// `logo.png`, likewise, at its largest (zoomed) size.
  logo: Decoded,
}

/// A picture decoded for a device-pixel size, kept while that size holds.
#[derive(Default)]
struct Decoded(Option<(u32, Option<Arc<RenderImage>>)>);

impl Decoded {
  fn get(&mut self, bytes: &[u8], side: u32) -> Option<Arc<RenderImage>> {
    if self.0.as_ref().is_none_or(|(s, _)| *s != side) {
      self.0 = Some((side, images::decode(bytes, Some((side, side)))));
    }
    self.0.as_ref()?.1.clone()
  }
}

impl Default for Splash {
  fn default() -> Self {
    Self::new()
  }
}

impl Splash {
  /// The animation, from its start.
  pub fn new() -> Self {
    Self {
      clock: Clock::Waiting,
      skipped: None,
      portrait: Decoded::default(),
      logo: Decoded::default(),
    }
  }

  /// The animation stopped at `at` seconds.
  pub fn frozen(at: f32) -> Self {
    Self {
      clock: Clock::Frozen(at),
      ..Self::new()
    }
  }

  /// Fades out now.
  pub fn skip(&mut self) {
    let now = self.time();
    if self.skipped.is_none() && now < LEAVE {
      self.skipped = Some(now);
    }
  }

  fn time(&self) -> f32 {
    match self.clock {
      Clock::Waiting => 0.,
      Clock::Started(at) => at.elapsed().as_secs_f32(),
      Clock::Frozen(at) => at,
    }
  }

  /// The animation's frame filling the window, or `None` once it is over.
  pub fn render(&mut self, window: &mut Window) -> Option<AnyElement> {
    if matches!(self.clock, Clock::Waiting) {
      self.clock = Clock::Started(Instant::now());
    }
    let t = self.time();
    let leave = self.skipped.unwrap_or(LEAVE);
    if t >= leave + FADE_OUT {
      return None;
    }
    if !matches!(self.clock, Clock::Frozen(_)) {
      window.request_animation_frame();
    }
    let leaving = ((t - leave) / FADE_OUT).clamp(0., 1.);
    let opacity = 1. - leaving;
    // Around the logo's center, in canvas units.
    let zoom = 1. + (ZOOM - 1.) * (1. - (1. - leaving).powi(3));

    let viewport = window.viewport_size();
    let (width, height) = (f32::from(viewport.width), f32::from(viewport.height));
    let (left, top) = ((width - SIZE) / 2., (height - SIZE) / 2. - 24.);
    // Canvas units to pixels, and how far above its place a layer starts: above the window.
    let k = SIZE / 512.;
    let drop = (top + SIZE) / k;

    let stack = Stack::at(t);
    let logo_shown = ((t - SETTLED) / CROSSFADE).clamp(0., 1.);
    let mut layers: Vec<AnyElement> = Vec::new();
    if logo_shown < 1. {
      for (color, layer) in &CARDS {
        if let Some(element) = card(*color, layer, t, drop, &stack, zoom, k) {
          layers.push(element);
        }
      }
      if let Some(element) = self.portrait(t, drop, &stack, zoom, k, window) {
        layers.push(element);
      }
    }
    let side = (SIZE * ZOOM * window.scale_factor()).round() as u32;
    if logo_shown > 0.
      && let Some(image) = self.logo.get(assets::LOGO_PNG, side)
    {
      let edge = 256. * (1. - zoom);
      layers.push(
        img(ImageSource::Render(image))
          .absolute()
          .left(px(edge * k))
          .top(px((edge + stack.hop) * k))
          .size(px(512. * zoom * k))
          .object_fit(ObjectFit::Fill)
          .opacity(logo_shown)
          .into_any_element(),
      );
    }

    // "Peeby" springs up once the portrait is down.
    let since = t - PORTRAIT.landing() - 0.08;
    let name = (since > 0.).then(|| {
      let spring = 1. - (-7. * since).exp() * (16. * since).cos();
      div()
        .absolute()
        .left_0()
        .w(px(width))
        .top(px(top + SIZE + 18. + 30. * (1. - spring)))
        .flex()
        .justify_center()
        .text_size(px(30.))
        .font_weight(FontWeight::BOLD)
        .text_color(theme::foreground())
        .opacity((since / 0.15).min(1.))
        .child("Peeby")
    });

    Some(
      div()
        .absolute()
        .inset_0()
        .size_full()
        .bg(theme::neutral_background())
        .opacity(opacity)
        .child(
          div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .size(px(SIZE))
            .children(layers),
        )
        .children(name)
        .into_any_element(),
    )
  }

  /// The portrait's layer at `t`: the whole canvas, moved and stretched like the portrait.
  fn portrait(
    &mut self,
    t: f32,
    drop: f32,
    stack: &Stack,
    zoom: f32,
    k: f32,
    window: &Window,
  ) -> Option<AnyElement> {
    let pose = Pose::of(&PORTRAIT, t, drop, stack)?;
    let side = (SIZE * ZOOM * window.scale_factor()).round() as u32;
    let image = self.portrait.get(PORTRAIT_PNG, side)?;
    // Points move as `pivot + (p − pivot) · scale + offset`, then zoom around the center: the
    // canvas's corner and side follow.
    let (pivot, scale) = pose.pivot_scale(&PORTRAIT);
    let x = zoomed(pivot.0 * (1. - scale.0) + pose.offset.0, zoom);
    let y = zoomed(pivot.1 * (1. - scale.1) + pose.offset.1, zoom);
    Some(
      img(ImageSource::Render(image))
        .absolute()
        .left(px(x * k))
        .top(px(y * k))
        .w(px(512. * scale.0 * zoom * k))
        .h(px(512. * scale.1 * zoom * k))
        .object_fit(ObjectFit::Fill)
        .into_any_element(),
    )
  }
}

/// A card at `t`: the mask at its resting place, moved, turned and squashed at paint time (an SVG
/// keeps its size, so it is rasterized once).
fn card(
  color: u32,
  layer: &Layer,
  t: f32,
  drop: f32,
  stack: &Stack,
  zoom: f32,
  k: f32,
) -> Option<AnyElement> {
  let pose = Pose::of(layer, t, drop, stack)?;
  let rest = canvas(layer.center);
  let side = CARD * FIT;
  let (pivot, scale) = pose.pivot_scale(layer);
  let center = (
    zoomed(pivot.0 + (rest.0 - pivot.0) * scale.0 + pose.offset.0, zoom),
    zoomed(pivot.1 + (rest.1 - pivot.1) * scale.1 + pose.offset.1, zoom),
  );
  let scale = (scale.0 * zoom, scale.1 * zoom);
  Some(
    svg()
      .path(assets::LOGO_CARD)
      .absolute()
      .left(px((rest.0 - side / 2.) * k))
      .top(px((rest.1 - side / 2.) * k))
      .size(px(side * k))
      .text_color(rgb(color))
      .with_transformation(
        Transformation::rotate(radians(pose.angle.to_radians()))
          .with_scaling(size(scale.0, scale.1))
          .with_translation(point(
            px((center.0 - rest.0) * k),
            px((center.1 - rest.1) * k),
          )),
      )
      .into_any_element(),
  )
}

/// A canvas coordinate zoomed by `zoom` around the canvas's center.
fn zoomed(v: f32, zoom: f32) -> f32 {
  256. + (v - 256.) * zoom
}

/// A point of onglets.html in the 512 canvas.
fn canvas(p: (f32, f32)) -> (f32, f32) {
  (256. + FIT * (p.0 - CENTER.0), 256. + FIT * (p.1 - CENTER.1))
}

/// The stack's squash and hop at a moment, from every landing so far.
struct Stack {
  /// Squash: the height lost, as a share (negative: stretched).
  squash: f32,
  /// The stack's height above its floor after the portrait's landing (negative: up).
  hop: f32,
}

impl Stack {
  fn at(t: f32) -> Self {
    let squash = CARDS
      .iter()
      .map(|(_, layer)| layer)
      .chain([&PORTRAIT])
      .map(|layer| {
        let since = t - layer.landing();
        if since < 0. {
          0.
        } else {
          layer.impact * (-SQUASH_DAMP * since).exp() * (SQUASH_PULSE * since).cos()
        }
      })
      .sum();
    let since = t - PORTRAIT.landing() - HOP_DELAY;
    let hop = if since < 0. {
      0.
    } else {
      -HOP * bounce(since, REBOUND_DAMP, HOP_PERIOD)
    };
    Self { squash, hop }
  }
}

/// Damped rebounds: 0 at `since` = 0, up and down with shorter and lower arcs.
fn bounce(since: f32, damp: f32, period: f32) -> f32 {
  (-damp * since).exp() * (PI * since / period).sin().abs()
}

/// Where a layer is at a moment, relative to its resting place.
struct Pose {
  /// Moved by, in canvas units.
  offset: (f32, f32),
  /// Angle, in degrees.
  angle: f32,
  /// Stretched while it falls (its own center), else the stack's squash (around the floor).
  falling: Option<(f32, f32)>,
  stack: (f32, f32),
}

impl Pose {
  /// `None` before it starts falling.
  fn of(layer: &Layer, t: f32, drop: f32, stack: &Stack) -> Option<Self> {
    let since = t - layer.start;
    if since < 0. {
      return None;
    }
    if since < layer.fall {
      // Gravity: slow at the top, fastest when it lands.
      let u = since / layer.fall;
      let stretch = STRETCH * u * u;
      return Some(Self {
        offset: (0., -drop * (1. - u * u)),
        angle: layer.angle + layer.spin * (1. - u) * (1. - u),
        falling: Some((1. - stretch / 2., 1. + stretch)),
        stack: (1., 1.),
      });
    }
    let landed = since - layer.fall;
    let wobble = if layer.spin == 0. {
      0.
    } else {
      layer.spin.signum() * WOBBLE * (-REBOUND_DAMP * landed).exp() * (WOBBLE_PULSE * landed).sin()
    };
    Some(Self {
      offset: (
        0.,
        -layer.rebound * bounce(landed, REBOUND_DAMP, REBOUND_PERIOD) + stack.hop,
      ),
      angle: layer.angle + wobble,
      falling: None,
      stack: (1. + SQUASH_WIDEN * stack.squash, 1. - stack.squash),
    })
  }

  /// The point the layer scales around and its scale.
  fn pivot_scale(&self, layer: &Layer) -> ((f32, f32), (f32, f32)) {
    match self.falling {
      Some(stretch) => (canvas(layer.center), stretch),
      None => (FLOOR, self.stack),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// When the drawn stack fades into `logo.png`, every layer is at rest to a fraction of a
  /// pixel: the swap does not show.
  #[test]
  fn settled_before_the_crossfade() {
    let stack = Stack::at(SETTLED);
    let k = SIZE / 512.;
    for layer in CARDS.iter().map(|(_, l)| l).chain([&PORTRAIT]) {
      let Some(pose) = Pose::of(layer, SETTLED, 1000., &stack) else {
        panic!("a layer has not fallen yet");
      };
      assert!(pose.falling.is_none());
      assert!(pose.offset.1.abs() * k < 0.5, "offset {:?}", pose.offset);
      assert!(
        (pose.angle - layer.angle).abs() < 0.2,
        "angle {}",
        pose.angle
      );
      assert!(stack.squash.abs() < 0.005, "squash {}", stack.squash);
    }
  }

  /// Each layer lands after the one under it, and nothing has landed at the start.
  #[test]
  fn falls_in_order() {
    let landings: Vec<f32> = CARDS
      .iter()
      .map(|(_, l)| l.landing())
      .chain([PORTRAIT.landing()])
      .collect();
    assert!(landings.windows(2).all(|w| w[0] < w[1]));
    assert!(landings[3] < SETTLED && SETTLED + CROSSFADE <= LEAVE);
    assert_eq!(Stack::at(0.).squash, 0.);
  }
}
