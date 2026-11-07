//! What a chat needs to run without a service, for the `chat` example and the app's fake chat
//! (debug builds): emote and badge images drawn in memory ([`Squares`]) and a small seeded
//! generator ([`Rng`]), so that neither needs the network nor a crate.

use std::io::Cursor;

use crate::images::{Download, Fetch};

/// Draws emotes and badges from their URL alone: a colored square as PNG, picked by the URL, its
/// top left corner white (so that a flip or a turn shows); a URL with `/anim/` is a square that
/// blinks (a GIF), one with `/over/` a ring that grows, on a transparent background (drawn over
/// the emote before it). Any scheme works: give it a prefix
/// with `images::route`.
pub struct Squares;

impl Fetch for Squares {
  fn fetch(&self, url: &str) -> Download {
    let seed = url
      .bytes()
      .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    let (animated, over) = (url.contains("/anim/"), url.contains("/over/"));
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
                image::Rgba(if corner(x, y) {
                  [255, 255, 255, 255]
                } else {
                  [shade, (seed >> 8) as u8, 255 - shade, 255]
                })
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
        let edge = x < 4 || y < 4 || x > 51 || y > 51 || corner(x, y);
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

/// The top left corner's triangle of a 56-pixel square.
fn corner(x: u32, y: u32) -> bool {
  x + y < 22
}

/// A small pseudo-random generator (xorshift): the same seed gives the same sequence.
pub struct Rng(u64);

impl Rng {
  /// A generator from `seed` (0 is replaced: xorshift stays at 0).
  pub fn new(seed: u64) -> Self {
    Self(if seed == 0 {
      0x9e37_79b9_7f4a_7c15
    } else {
      seed
    })
  }

  /// A number in `0..below` (0 when `below` is 0).
  pub fn below(&mut self, below: usize) -> usize {
    self.0 ^= self.0 << 13;
    self.0 ^= self.0 >> 7;
    self.0 ^= self.0 << 17;
    if below == 0 {
      0
    } else {
      (self.0 % below as u64) as usize
    }
  }

  /// `true` once in `n` times on average.
  pub fn one_in(&mut self, n: usize) -> bool {
    self.below(n) == 0
  }

  /// One of `items`.
  pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
    items.get(self.below(items.len()))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_same_seed_gives_the_same_numbers() {
    let (mut a, mut b) = (Rng::new(42), Rng::new(42));
    let first: Vec<usize> = (0..8).map(|_| a.below(100)).collect();
    let second: Vec<usize> = (0..8).map(|_| b.below(100)).collect();
    assert_eq!(first, second);
    assert!(first.iter().all(|&n| n < 100));
    assert_eq!(Rng::new(0).below(0), 0);
    assert!(Rng::new(1).pick::<u8>(&[]).is_none());
  }
}
