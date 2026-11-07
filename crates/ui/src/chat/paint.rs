//! A name's paint: a gradient over the letters of an author's name (7TV's paints, say), as the
//! web draws it with `background-clip: text`. GPUI fills a glyph with one color, so each letter
//! takes the gradient's color at its middle: letters are laid out as a row of equal cells
//! ([`CELL`] wide, [`LINE`] high, in ems), which is close enough for a name. An image paint is
//! stretched over the name (7TV's `background-size: 100% 100%`), and each letter takes the
//! average color of its slice of the image ([`image_colors`]).

use gpui::{Hsla, Rgba, SharedString};

use super::readable;

/// Width of a letter, in ems, for placing it on the gradient.
const CELL: f32 = 0.55;
/// Height of the line, in ems.
const LINE: f32 = 1.25;
/// The faintest a painted letter is drawn: a paint's translucent stops let the page show through,
/// which would make the name hard to read on the dark background.
const LEAST_ALPHA: f32 = 0.6;

/// A gradient over a name.
#[derive(Clone, Debug, PartialEq)]
pub struct Paint {
  /// How the stops are laid over the name.
  pub fill: PaintFill,
  /// Where (0 to 1 along the gradient) and which color, in order; one stop is a plain color.
  pub stops: Vec<(f32, Rgba)>,
  /// The gradient repeats past its last stop (CSS `repeating-*-gradient`).
  pub repeat: bool,
}

/// How a [`Paint`]'s stops are laid over the name.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintFill {
  /// Along a line at this CSS angle (degrees: 0 towards the top, 90 towards the right).
  Linear(f32),
  /// From the middle out to the farthest corner: a circle, else an ellipse.
  Radial {
    /// A circle rather than an ellipse of the name's proportions.
    circle: bool,
  },
  /// An image at this URL, stretched over the name; the stops are not used.
  Image(SharedString),
}

impl Paint {
  /// A plain color.
  pub fn solid(color: Rgba) -> Self {
    Self {
      fill: PaintFill::Linear(90.),
      stops: vec![(0., color)],
      repeat: false,
    }
  }

  /// The color of each of a name's `letters`, left to right; `None` for an image paint, whose
  /// colors come from its pixels ([`image_colors`]).
  pub fn colors(&self, letters: usize) -> Option<Vec<Hsla>> {
    if matches!(self.fill, PaintFill::Image(_)) {
      return None;
    }
    let width = letters as f32 * CELL;
    Some(
      (0..letters)
        .map(|i| {
          let x = (i as f32 + 0.5) * CELL;
          shown(self.at(self.position(x, width)))
        })
        .collect(),
    )
  }

  /// Where the point `x` of the line's middle is along the gradient, 0 at its start and 1 at its
  /// end, for a name `width` wide.
  fn position(&self, x: f32, width: f32) -> f32 {
    let (dx, dy) = (x - width / 2., 0.);
    match &self.fill {
      PaintFill::Image(_) => 0.5,
      &PaintFill::Linear(angle) => {
        // CSS: the gradient line crosses the middle at `angle`, long enough for the corners to
        // reach its ends.
        let (sin, cos) = angle.to_radians().sin_cos();
        let length = (width * sin).abs() + (LINE * cos).abs();
        if length <= f32::EPSILON {
          return 0.5;
        }
        (dx * sin - dy * cos) / length + 0.5
      }
      &PaintFill::Radial { circle } => {
        let (half_w, half_h) = (width / 2., LINE / 2.);
        if circle {
          let radius = half_w.hypot(half_h);
          if radius <= f32::EPSILON {
            return 0.;
          }
          dx.hypot(dy) / radius
        } else {
          // The farthest-corner ellipse has the box's proportions, √2 times larger.
          let (rx, ry) = (half_w * 2_f32.sqrt(), half_h * 2_f32.sqrt());
          if rx <= f32::EPSILON || ry <= f32::EPSILON {
            return 0.;
          }
          (dx / rx).hypot(dy / ry)
        }
      }
    }
  }

  /// The gradient's color at `t` along it.
  fn at(&self, t: f32) -> Rgba {
    let (Some(&(first, first_color)), Some(&(last, last_color))) =
      (self.stops.first(), self.stops.last())
    else {
      return Rgba::default();
    };
    let t = if self.repeat && last - first > f32::EPSILON {
      first + (t - first).rem_euclid(last - first)
    } else {
      t
    };
    if t <= first {
      return first_color;
    }
    if t >= last {
      return last_color;
    }
    for pair in self.stops.windows(2) {
      let [(a, from), (b, to)] = [pair[0], pair[1]];
      if t <= b {
        let k = if b - a > f32::EPSILON {
          (t - a) / (b - a)
        } else {
          1.
        };
        return mix(from, to, k);
      }
    }
    last_color
  }
}

/// The color of each of a name's `letters` from an image paint's pixels (`bgra`, `width` x
/// `height`, rows top to bottom), stretched over the name: the average of each letter's vertical
/// slice of the image, weighted by the pixels' opacity. A fully transparent slice takes the color
/// of the letter before it (white for the first).
pub fn image_colors(bgra: &[u8], width: usize, height: usize, letters: usize) -> Vec<Hsla> {
  let mut previous = Rgba {
    r: 1.,
    g: 1.,
    b: 1.,
    a: 1.,
  };
  if width == 0 || height == 0 || bgra.len() < width * height * 4 {
    return vec![shown(previous); letters];
  }
  let mut colors = Vec::with_capacity(letters);
  for i in 0..letters {
    // The columns under the letter, at least one.
    let from = (i * width / letters).min(width - 1);
    let to = ((i + 1) * width / letters).max(from + 1).min(width);
    let (mut sum, mut weight, mut count) = ([0f32; 3], 0f32, 0f32);
    for y in 0..height {
      for x in from..to {
        let at = (y * width + x) * 4;
        let [b, g, r, a] = [bgra[at], bgra[at + 1], bgra[at + 2], bgra[at + 3]].map(f32::from);
        let a = a / 255.;
        sum[0] += r / 255. * a;
        sum[1] += g / 255. * a;
        sum[2] += b / 255. * a;
        weight += a;
        count += 1.;
      }
    }
    if weight > f32::EPSILON {
      previous = Rgba {
        r: sum[0] / weight,
        g: sum[1] / weight,
        b: sum[2] / weight,
        a: weight / count,
      };
    }
    colors.push(shown(previous));
  }
  colors
}

/// A paint's color as a letter is drawn in it: opaque enough, and light enough for the dark
/// background.
fn shown(color: Rgba) -> Hsla {
  let mut color: Hsla = color.into();
  color.a = color.a.max(LEAST_ALPHA);
  readable(color)
}

fn mix(from: Rgba, to: Rgba, k: f32) -> Rgba {
  let lerp = |a: f32, b: f32| a + (b - a) * k;
  Rgba {
    r: lerp(from.r, to.r),
    g: lerp(from.g, to.g),
    b: lerp(from.b, to.b),
    a: lerp(from.a, to.a),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn rgb(r: f32, g: f32, b: f32) -> Rgba {
    Rgba { r, g, b, a: 1. }
  }

  #[test]
  fn a_left_to_right_gradient_runs_along_the_name() {
    let paint = Paint {
      fill: PaintFill::Linear(90.),
      stops: vec![(0., rgb(1., 0., 0.)), (1., rgb(0., 0., 1.))],
      repeat: false,
    };
    // 90°: the line is the name's width, plus nothing of its height.
    let width = 10. * CELL;
    assert!((paint.position(0., width) - 0.).abs() < 1e-4);
    assert!((paint.position(width, width) - 1.).abs() < 1e-4);
    assert!((paint.position(width / 2., width) - 0.5).abs() < 1e-4);
    let middle = paint.at(0.5);
    assert!((middle.r - 0.5).abs() < 1e-4 && (middle.b - 0.5).abs() < 1e-4);
    // 270° runs the other way.
    let back = Paint {
      fill: PaintFill::Linear(270.),
      ..paint.clone()
    };
    assert!((back.position(0., width) - 1.).abs() < 1e-4);
    // Every letter gets a color.
    assert_eq!(paint.colors(7).map(|c| c.len()), Some(7));
    let image = Paint {
      fill: PaintFill::Image("https://example.com/paint.webp".into()),
      stops: Vec::new(),
      repeat: false,
    };
    assert_eq!(image.colors(7), None);
  }

  #[test]
  fn a_top_to_bottom_gradient_gives_every_letter_its_middle() {
    let paint = Paint {
      fill: PaintFill::Linear(180.),
      stops: vec![(0., rgb(1., 0., 0.)), (1., rgb(0., 0., 1.))],
      repeat: false,
    };
    for x in [0., 1., 3.] {
      assert!((paint.position(x, 3.) - 0.5).abs() < 1e-4);
    }
  }

  #[test]
  fn stops_hold_their_ends_and_repeat_when_asked() {
    let mut paint = Paint {
      fill: PaintFill::Linear(90.),
      stops: vec![(0.25, rgb(1., 0., 0.)), (0.75, rgb(0., 1., 0.))],
      repeat: false,
    };
    assert_eq!(paint.at(0.), rgb(1., 0., 0.));
    assert_eq!(paint.at(1.), rgb(0., 1., 0.));
    paint.repeat = true;
    // 1.25 is 1.0 past the first stop: twice round the 0.5-long period, back at its start.
    assert_eq!(paint.at(1.25), rgb(1., 0., 0.));
    // 0.875 is a quarter into its second period.
    let quarter = paint.at(0.875);
    assert!((quarter.r - 0.75).abs() < 1e-4 && (quarter.g - 0.25).abs() < 1e-4);
  }

  #[test]
  fn a_radial_gradient_grows_from_the_middle() {
    let paint = Paint {
      fill: PaintFill::Radial { circle: true },
      stops: vec![(0., rgb(1., 1., 1.)), (1., rgb(0., 0., 0.))],
      repeat: false,
    };
    let width = 8. * CELL;
    assert!(paint.position(width / 2., width) < 1e-4);
    let edge = paint.position(0., width);
    assert!(edge > 0.5 && edge < 1.);
    let ellipse = Paint {
      fill: PaintFill::Radial { circle: false },
      ..paint
    };
    assert!((ellipse.position(0., width) - 1. / 2_f32.sqrt()).abs() < 1e-4);
  }

  #[test]
  fn faint_stops_still_read() {
    let paint = Paint::solid(Rgba {
      r: 1.,
      g: 1.,
      b: 1.,
      a: 0.1,
    });
    assert!(paint.colors(1).expect("a gradient")[0].a >= LEAST_ALPHA);
  }

  #[test]
  fn an_image_is_stretched_and_each_letter_averages_its_slice() {
    // 4 x 2 BGRA: red, red, blue, transparent on both rows.
    let red = [0, 0, 255, 255];
    let blue = [255, 0, 0, 255];
    let clear = [0, 0, 0, 0];
    let row = [red, red, blue, clear].concat();
    let pixels = [row.clone(), row].concat();
    // Two letters: red, then half blue and half transparent, which averages to blue alone (made
    // lighter to read, so compared by hue).
    let two = image_colors(&pixels, 4, 2, 2);
    assert_eq!(two.len(), 2);
    assert!(two[0].h < 0.02 || two[0].h > 0.98, "{:?}", two[0]);
    assert!((two[1].h - 2. / 3.).abs() < 0.02, "{:?}", two[1]);
    // More letters than columns: each still gets one; a transparent slice keeps the color before.
    let eight = image_colors(&pixels, 4, 2, 8);
    assert_eq!(eight.len(), 8);
    assert_eq!(eight[7], eight[5]);
    // Nothing to read: white.
    assert_eq!(image_colors(&[], 0, 0, 3).len(), 3);
  }
}
