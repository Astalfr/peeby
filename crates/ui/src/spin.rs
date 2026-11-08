//! A picture turning on itself. GPUI rotates SVGs only, not images: [`rotations`] draws the
//! picture once per step of a turn, ahead of time, and the caller paints the step of the moment.

use std::f32::consts::{SQRT_2, TAU};
use std::sync::Arc;

use gpui::RenderImage;
use image::RgbaImage;
use image::imageops::FilterType;

/// `count` steps of a whole turn of the picture in `bytes` (any format `image` reads), cropped
/// to a square of `side` pixels with corners rounded by `radius` pixels. Each step is a square
/// of [`turned_side`]`(side)` pixels, the picture centered in it, transparent around. `None`
/// when the bytes are not a picture. Slow (tens of milliseconds): run it in the background.
pub fn rotations(
  bytes: &[u8],
  side: u32,
  radius: f32,
  count: usize,
) -> Option<Vec<Arc<RenderImage>>> {
  let picture = image::load_from_memory(bytes).ok()?;
  let mut square = picture
    .resize_to_fill(side, side, FilterType::Lanczos3)
    .to_rgba8();
  round_corners(&mut square, radius);
  let premultiplied = premultiply(&square);
  let out = turned_side(side);
  (0..count)
    .map(|step| {
      let angle = TAU * step as f32 / count as f32;
      let pixels = rotate(&premultiplied, side, out, angle);
      crate::images::bgra_image(out, out, pixels)
    })
    .collect()
}

/// The side of a step of [`rotations`]: the picture's diagonal, so that no corner is cut.
pub fn turned_side(side: u32) -> u32 {
  (side as f32 * SQRT_2).ceil() as u32
}

/// Makes the corners outside a rounded square transparent, with a pixel of smoothing.
fn round_corners(image: &mut RgbaImage, radius: f32) {
  let side = image.width() as f32;
  let radius = radius.clamp(0., side / 2.);
  for (x, y, px) in image.enumerate_pixels_mut() {
    let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
    // Distance into a corner's circle, when the pixel is in a corner's square.
    let dx = (radius - cx).max(cx - (side - radius)).max(0.);
    let dy = (radius - cy).max(cy - (side - radius)).max(0.);
    if dx > 0. && dy > 0. {
      let outside = (dx * dx + dy * dy).sqrt() - radius;
      let coverage = (0.5 - outside).clamp(0., 1.);
      px[3] = (f32::from(px[3]) * coverage).round() as u8;
    }
  }
}

/// RGBA as premultiplied floats, so that sampling between pixels does not darken the edges.
fn premultiply(image: &RgbaImage) -> Vec<[f32; 4]> {
  image
    .pixels()
    .map(|p| {
      let a = f32::from(p[3]) / 255.;
      [
        f32::from(p[0]) * a,
        f32::from(p[1]) * a,
        f32::from(p[2]) * a,
        a,
      ]
    })
    .collect()
}

/// `source` (`side`² premultiplied pixels) turned by `angle` radians around its center, in the
/// center of an `out`² RGBA canvas (straight alpha), sampled bilinearly.
fn rotate(source: &[[f32; 4]], side: u32, out: u32, angle: f32) -> Vec<u8> {
  let (sin, cos) = angle.sin_cos();
  let half_in = side as f32 / 2.;
  let half_out = out as f32 / 2.;
  let at = |x: i64, y: i64| -> [f32; 4] {
    if x < 0 || y < 0 || x >= i64::from(side) || y >= i64::from(side) {
      return [0.; 4];
    }
    source[(y as usize) * side as usize + x as usize]
  };
  let mut pixels = Vec::with_capacity((out * out * 4) as usize);
  for y in 0..out {
    for x in 0..out {
      // The canvas pixel's center, turned back into the picture.
      let (dx, dy) = (x as f32 + 0.5 - half_out, y as f32 + 0.5 - half_out);
      let sx = cos * dx + sin * dy + half_in - 0.5;
      let sy = -sin * dx + cos * dy + half_in - 0.5;
      let (x0, y0) = (sx.floor(), sy.floor());
      let (fx, fy) = (sx - x0, sy - y0);
      let (x0, y0) = (x0 as i64, y0 as i64);
      let mut sum = [0f32; 4];
      for (ox, oy, weight) in [
        (0, 0, (1. - fx) * (1. - fy)),
        (1, 0, fx * (1. - fy)),
        (0, 1, (1. - fx) * fy),
        (1, 1, fx * fy),
      ] {
        let p = at(x0 + ox, y0 + oy);
        for (s, v) in sum.iter_mut().zip(p) {
          *s += v * weight;
        }
      }
      let a = sum[3];
      let straight = |v: f32| {
        if a > 0. {
          (v / a).round().clamp(0., 255.) as u8
        } else {
          0
        }
      };
      pixels.extend([
        straight(sum[0]),
        straight(sum[1]),
        straight(sum[2]),
        (a * 255.).round().clamp(0., 255.) as u8,
      ]);
    }
  }
  pixels
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_turned_square_keeps_its_center_and_rounds_its_corners() {
    let mut square = RgbaImage::from_pixel(20, 20, image::Rgba([200, 10, 10, 255]));
    round_corners(&mut square, 6.);
    assert_eq!(square.get_pixel(0, 0)[3], 0);
    assert_eq!(square.get_pixel(10, 10)[3], 255);
    let out = turned_side(20);
    assert_eq!(out, 29);
    let pixels = rotate(&premultiply(&square), 20, out, TAU / 8.);
    let center = ((out / 2 * out + out / 2) * 4) as usize;
    assert_eq!(&pixels[center..center + 4], &[200, 10, 10, 255]);
    // The canvas' corner is outside the turned square.
    assert_eq!(pixels[3], 0);
  }
}
