//! Remote images (thumbnails, posters, avatars, emotes, animated ones with their frames): download on the network runtime, disk cache,
//! decode off the UI thread, in-memory LRU of GPU-ready images, and BlurHash placeholders.
//! Shared by every space: the app gives the store a [`Fetch`], and a space whose server needs
//! its own requests (Jellyfin's `Authorization` header) adds one for its server's URLs
//! ([`route`]), so the store depends on no service client.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use gpui::{AppContext, Context, Entity, Global, RenderImage};
use tokio::sync::Semaphore;

use crate::runtime;

/// How an image is turned before it is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Turn {
  /// As it is.
  #[default]
  None,
  /// Mirrored left to right.
  FlipX,
  /// Mirrored top to bottom.
  FlipY,
  /// A quarter turn counterclockwise.
  Left,
  /// A quarter turn clockwise.
  Right,
}

/// A color filter on an image, as CSS writes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Tint {
  /// Its own colors.
  #[default]
  None,
  /// `grayscale(1) brightness(0.7) contrast(2.5)`.
  Cursed,
  /// `sepia(0.5) hue-rotate(θ) saturate(2.5)`, θ going round in [`PARTY_PERIOD_MS`]: a still
  /// image becomes [`PARTY_FRAMES`] frames, an animated one takes each frame's hue from its time.
  Party,
}

/// What is done to an image's pixels after decoding (off the UI thread), cached apart from the
/// image itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Effects {
  /// Its orientation.
  pub turn: Turn,
  /// Its colors.
  pub tint: Tint,
}

impl Effects {
  /// Nothing is done.
  pub fn is_none(&self) -> bool {
    *self == Self::default()
  }
}

/// How [`ImageStore::get_fit`] shrinks an image to the box it is drawn in, keeping its ratio.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fit {
  /// Until it just covers the box (`ObjectFit::Cover`): nothing is cropped, the GPU crops.
  Cover,
  /// Until it just fits inside the box (a logo).
  Contain,
}

/// How a decoded image is scaled.
#[derive(Clone, Copy, Debug)]
enum Scale {
  /// As it is.
  None,
  /// Scaled and cropped to exactly this size.
  Fill(u32, u32),
  /// Shrunk (never enlarged) to this box, keeping its ratio.
  Fit(Fit, u32, u32),
}

impl From<Option<(u32, u32)>> for Scale {
  fn from(size: Option<(u32, u32)>) -> Self {
    size.map_or(Self::None, |(w, h)| Self::Fill(w, h))
  }
}

/// A drawn size (device pixels) rounded up to a step of about 1/16 of it, so that resizing a
/// window does not decode an image again for every pixel; the GPU's last shrink, by at most
/// that much, does not show.
fn bucket(pixels: u32) -> u32 {
  let pixels = pixels.max(1);
  let step = (pixels.next_power_of_two() / 16).max(8);
  pixels.div_ceil(step) * step
}

/// One round of [`Tint::Party`]'s hue.
pub const PARTY_PERIOD_MS: u32 = 1500;
/// Frames a still image gets for one round of [`Tint::Party`].
const PARTY_FRAMES: u32 = 20;

/// Decoded images kept in memory (BGRA bytes), beyond which the least recently used are dropped.
const MEMORY_BUDGET: usize = 256 * 1024 * 1024;
/// Parallel downloads, so a fast scroll does not queue dozens of requests at once.
const MAX_DOWNLOADS: usize = 8;

/// A download in progress: the image's bytes, or why it failed.
pub type Download = Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send>>;

/// Downloads images. Runs on the network runtime (tokio).
pub trait Fetch: Send + Sync {
  /// Starts downloading `url`.
  fn fetch(&self, url: &str) -> Download;
}

/// Handle to the app-wide image store.
#[derive(Clone)]
pub struct Images(pub Entity<ImageStore>);

impl Global for Images {}

/// A decoded image and when it was last asked for, for LRU eviction.
struct Cached {
  image: Arc<RenderImage>,
  used: u64,
}

/// The image store: see the module documentation.
pub struct ImageStore {
  fetcher: Arc<dyn Fetch>,
  /// Fetchers for the URLs that start with a prefix (a server's address), before `fetcher`.
  routes: Vec<(String, Arc<dyn Fetch>)>,
  downloads: Arc<Semaphore>,
  cache_dir: Option<PathBuf>,
  ready: HashMap<String, Cached>,
  /// Ticks on every lookup; `Cached::used` holds its value at the image's last use.
  clock: u64,
  bytes: usize,
  pending: HashSet<String>,
  failed: HashSet<String>,
  /// Decoded BlurHashes by hash and aspect ratio (`None`: the hash is invalid).
  blurhashes: HashMap<String, Option<Arc<RenderImage>>>,
  /// The last key [`Self::get_fit`] drew for an image (URL and effects), shown while the image
  /// is decoded for a new size.
  shown: HashMap<String, String>,
}

impl ImageStore {
  fn new(fetcher: Arc<dyn Fetch>) -> Self {
    let cache_dir = disk_cache_dir();
    if let Some(dir) = &cache_dir
      && let Err(e) = std::fs::create_dir_all(dir)
    {
      tracing::warn!("image disk cache disabled: {e}");
    }
    Self {
      fetcher,
      routes: Vec::new(),
      downloads: Arc::new(Semaphore::new(MAX_DOWNLOADS)),
      cache_dir,
      ready: HashMap::new(),
      clock: 0,
      bytes: 0,
      pending: HashSet::new(),
      failed: HashSet::new(),
      blurhashes: HashMap::new(),
      shown: HashMap::new(),
    }
  }

  /// The fetcher of `url`: the route whose prefix it starts with, else the default one.
  fn fetcher_for(&self, url: &str) -> Arc<dyn Fetch> {
    self
      .routes
      .iter()
      .find(|(prefix, _)| url.starts_with(prefix.as_str()))
      .map_or_else(|| self.fetcher.clone(), |(_, fetch)| fetch.clone())
  }

  /// A small image decoded from a BlurHash, to show while the real one loads.
  pub fn blurhash(&mut self, hash: &str, aspect: f32) -> Option<Arc<RenderImage>> {
    let (w, h) = if aspect >= 1. {
      (32, (32. / aspect).max(1.) as u32)
    } else {
      ((32. * aspect).max(1.) as u32, 32)
    };
    let key = format!("{hash}#{w}x{h}");
    if let Some(image) = self.blurhashes.get(&key) {
      return image.clone();
    }
    let image = blurhash::decode(hash, w, h, 1.)
      .ok()
      .and_then(|rgba| bgra_image(w, h, rgba));
    self.blurhashes.insert(key, image.clone());
    image
  }

  /// The image at `url` if it is ready; otherwise starts loading it and returns `None`.
  /// Observers of the store are notified when it arrives.
  pub fn get(&mut self, url: &str, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
    if let Some(image) = self.touch(url) {
      return Some(image);
    }
    if !self.pending.contains(url) && !self.failed.contains(url) {
      self.load(
        url.to_owned(),
        url.to_owned(),
        Scale::None,
        Effects::default(),
        cx,
      );
    }
    None
  }

  /// Like [`Self::get`], with `effects` done to its pixels (the downloaded file is the same).
  pub fn get_with(
    &mut self,
    url: &str,
    effects: Effects,
    cx: &mut Context<Self>,
  ) -> Option<Arc<RenderImage>> {
    if effects.is_none() {
      return self.get(url, cx);
    }
    let key = effects_key(url, effects);
    if let Some(image) = self.touch(&key) {
      return Some(image);
    }
    if !self.pending.contains(&key) && !self.failed.contains(&key) {
      self.load(url.to_owned(), key, Scale::None, effects, cx);
    }
    None
  }

  /// Like [`Self::get_with`], shrunk on the CPU (Lanczos) to the box it is drawn in, `width`×
  /// `height` device pixels, as `fit` says. GPUI samples without mipmaps, so a large image drawn
  /// small looks pixelated: images drawn smaller than their file go through here. While a new
  /// size decodes (a window being resized), the last size drawn is returned.
  pub fn get_fit(
    &mut self,
    url: &str,
    effects: Effects,
    fit: Fit,
    width: u32,
    height: u32,
    cx: &mut Context<Self>,
  ) -> Option<Arc<RenderImage>> {
    let base = effects_key(url, effects);
    let (width, height) = (bucket(width), bucket(height));
    let key = format!("{base}#{fit:?}-{width}x{height}");
    if let Some(image) = self.touch(&key) {
      if self.shown.get(&base) != Some(&key) {
        self.shown.insert(base, key);
      }
      return Some(image);
    }
    if !self.pending.contains(&key) && !self.failed.contains(&key) {
      self.load(
        url.to_owned(),
        key,
        Scale::Fit(fit, width, height),
        effects,
        cx,
      );
    }
    let shown = self.shown.get(&base)?.clone();
    self.touch(&shown)
  }

  /// Like [`Self::get`], but scaled and cropped on the CPU to exactly `width`×`height` pixels.
  /// For images the server cannot resize (avatars): GPUI samples without mipmaps, so a large
  /// image drawn small looks pixelated.
  pub fn get_sized(
    &mut self,
    url: &str,
    width: u32,
    height: u32,
    cx: &mut Context<Self>,
  ) -> Option<Arc<RenderImage>> {
    let key = format!("{url}#{width}x{height}");
    if let Some(image) = self.touch(&key) {
      return Some(image);
    }
    if !self.pending.contains(&key) && !self.failed.contains(&key) {
      self.load(
        url.to_owned(),
        key,
        Scale::Fill(width, height),
        Effects::default(),
        cx,
      );
    }
    None
  }

  /// The ready image under `key`, marked as just used. Every card asks on every frame, so this
  /// stays O(1); the scan for the least recently used image happens on eviction only.
  fn touch(&mut self, key: &str) -> Option<Arc<RenderImage>> {
    self.clock += 1;
    let cached = self.ready.get_mut(key)?;
    cached.used = self.clock;
    Some(cached.image.clone())
  }

  /// Loads `url` into the store under `key`, scaled as `scale` says, with `effects`. The disk
  /// cache keeps the downloaded file, whatever the size and the effects.
  fn load(
    &mut self,
    url: String,
    key: String,
    scale: Scale,
    effects: Effects,
    cx: &mut Context<Self>,
  ) {
    self.pending.insert(key.clone());
    let fetcher = self.fetcher_for(&url);
    let downloads = self.downloads.clone();
    // Only downloads go to the disk cache: what another scheme names is made in memory (the
    // fake chat's squares), and a stale copy would hide a change to it.
    let path = self
      .cache_dir
      .as_ref()
      .filter(|_| url.starts_with("http://") || url.starts_with("https://"))
      .map(|dir| dir.join(cache_name(&url)));
    let executor = cx.background_executor().clone();

    cx.spawn(async move |this, cx| {
      // 1. Disk cache.
      let cached = match path.clone() {
        Some(path) => {
          executor
            .spawn(async move { std::fs::read(path).ok() })
            .await
        }
        None => None,
      };
      // 2. Network, at most MAX_DOWNLOADS at a time.
      let bytes = match cached {
        Some(bytes) => Some(bytes),
        None => {
          let fetch = this.update(cx, |_, cx| {
            let url = url.clone();
            runtime::spawn(cx, async move {
              let _permit = downloads.acquire_owned().await.ok()?;
              fetcher
                .fetch(&url)
                .await
                .map_err(|e| tracing::debug!("image: {e}"))
                .ok()
            })
          });
          let bytes = match fetch {
            Ok(fetch) => fetch.await.flatten(),
            Err(_) => return,
          };
          if let (Some(bytes), Some(path)) = (&bytes, path) {
            let bytes = bytes.clone();
            executor
              .spawn(async move { std::fs::write(path, bytes).ok() })
              .detach();
          }
          bytes
        }
      };
      // 3. Decode off the UI thread.
      let image = match bytes {
        Some(bytes) => {
          executor
            .spawn(async move { decode_scaled(&bytes, scale, effects) })
            .await
        }
        None => None,
      };
      this
        .update(cx, |store, cx| {
          store.pending.remove(&key);
          match image {
            Some(image) => store.insert(key, image, cx),
            None => {
              store.failed.insert(key);
            }
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn insert(&mut self, key: String, image: Arc<RenderImage>, cx: &mut Context<Self>) {
    self.bytes += image_bytes(&image);
    self.clock += 1;
    let used = self.clock;
    if let Some(old) = self.ready.insert(key, Cached { image, used }) {
      self.bytes = self.bytes.saturating_sub(image_bytes(&old.image));
      cx.drop_image(old.image, None);
    }
    while self.bytes > MEMORY_BUDGET && self.ready.len() > 1 {
      let Some(oldest) = self
        .ready
        .iter()
        .min_by_key(|(_, cached)| cached.used)
        .map(|(key, _)| key.clone())
      else {
        break;
      };
      if let Some(cached) = self.ready.remove(&oldest) {
        self.bytes = self.bytes.saturating_sub(image_bytes(&cached.image));
        // Free its GPU atlas entry too.
        cx.drop_image(cached.image, None);
      }
    }
  }
}

/// The store's key of `url` with `effects`.
fn effects_key(url: &str, effects: Effects) -> String {
  if effects.is_none() {
    url.to_owned()
  } else {
    format!("{url}#fx-{:?}-{:?}", effects.turn, effects.tint)
  }
}

fn image_bytes(image: &RenderImage) -> usize {
  (0..image.frame_count())
    .map(|frame| {
      let size = image.size(frame);
      size.width.0 as usize * size.height.0 as usize * 4
    })
    .sum()
}

fn cache_name(key: &str) -> String {
  let mut hasher = DefaultHasher::new();
  key.hash(&mut hasher);
  format!("{:016x}.img", hasher.finish())
}

/// Frames kept of an animated image; the rest are dropped (a long GIF would fill the memory).
const MAX_FRAMES: usize = 300;

/// Decodes an image, scaled to fill `size` (device pixels) when given. Animated GIFs and WebPs keep
/// their frames and delays: the chat panel plays them (`chat::panel`), an `img` with an id too
/// (GPUI's own animation, only while the window is active).
pub fn decode(bytes: &[u8], size: Option<(u32, u32)>) -> Option<Arc<RenderImage>> {
  decode_with(bytes, size, Effects::default())
}

/// [`decode`], then `effects` done to every frame.
pub fn decode_with(
  bytes: &[u8],
  size: Option<(u32, u32)>,
  effects: Effects,
) -> Option<Arc<RenderImage>> {
  decode_scaled(bytes, size.into(), effects)
}

/// [`decode_with`], scaled as `scale` says.
fn decode_scaled(bytes: &[u8], scale: Scale, effects: Effects) -> Option<Arc<RenderImage>> {
  // The box is the image as drawn, after a quarter turn.
  let scale = match (scale, effects.turn) {
    (Scale::Fit(fit, w, h), Turn::Left | Turn::Right) => Scale::Fit(fit, h, w),
    (scale, _) => scale,
  };
  let resize = |buffer: image::RgbaImage| match scale {
    Scale::None => buffer,
    Scale::Fill(w, h) => image::DynamicImage::ImageRgba8(buffer)
      .resize_to_fill(w.max(1), h.max(1), image::imageops::FilterType::Lanczos3)
      .into_rgba8(),
    Scale::Fit(fit, w, h) => shrink(buffer, fit, w, h),
  };
  // Each frame with its delay and its start (milliseconds, for the party's hue); `None` for a
  // still image.
  let frames: Vec<(image::RgbaImage, Option<image::Delay>, u32)> = match animation(bytes) {
    Some(frames) => {
      let mut start = 0u32;
      frames
        .into_iter()
        .map(|frame| {
          let delay = frame.delay();
          let at = start;
          start = start.saturating_add(delay_ms(delay));
          (resize(frame.into_buffer()), Some(delay), at)
        })
        .collect()
    }
    None => {
      let still = resize(image::load_from_memory(bytes).ok()?.into_rgba8());
      if effects.tint == Tint::Party {
        let step = PARTY_PERIOD_MS / PARTY_FRAMES;
        let delay = image::Delay::from_numer_denom_ms(step, 1);
        (0..PARTY_FRAMES)
          .map(|i| (still.clone(), Some(delay), i * step))
          .collect()
      } else {
        vec![(still, None, 0)]
      }
    }
  };
  let frames: Vec<image::Frame> = frames
    .into_iter()
    .map(|(buffer, delay, at)| {
      let mut buffer = apply(buffer, effects, at);
      for px in buffer.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
      }
      match delay {
        Some(delay) => image::Frame::from_parts(buffer, 0, 0, delay),
        None => image::Frame::new(buffer),
      }
    })
    .collect();
  Some(Arc::new(RenderImage::new(frames)))
}

/// `buffer` shrunk to the `width`×`height` box as `fit` says, keeping its ratio; as it is when
/// it is not larger. Filtered with premultiplied alpha, so that the color of transparent pixels
/// (often black) does not bleed into an emote's or a logo's edges.
fn shrink(buffer: image::RgbaImage, fit: Fit, width: u32, height: u32) -> image::RgbaImage {
  let (w, h) = buffer.dimensions();
  if w == 0 || h == 0 {
    return buffer;
  }
  let (sx, sy) = (width as f32 / w as f32, height as f32 / h as f32);
  let s = match fit {
    Fit::Cover => sx.max(sy),
    Fit::Contain => sx.min(sy),
  };
  if s >= 1. {
    return buffer;
  }
  let size = |n: u32| ((n as f32 * s).ceil() as u32).clamp(1, n);
  let (nw, nh) = (size(w), size(h));
  let filter = image::imageops::FilterType::Lanczos3;
  if buffer.pixels().all(|px| px[3] == 255) {
    return image::imageops::resize(&buffer, nw, nh, filter);
  }
  let premultiplied = image::Rgba32FImage::from_fn(w, h, |x, y| {
    let [r, g, b, a] = buffer.get_pixel(x, y).0.map(|c| f32::from(c) / 255.);
    image::Rgba([r * a, g * a, b * a, a])
  });
  let shrunk = image::imageops::resize(&premultiplied, nw, nh, filter);
  image::RgbaImage::from_fn(nw, nh, |x, y| {
    let [r, g, b, a] = shrunk.get_pixel(x, y).0.map(|c| c.clamp(0., 1.));
    let straight = |c: f32| if a > 0. { (c / a).min(1.) } else { 0. };
    image::Rgba([straight(r), straight(g), straight(b), a].map(|c| (c * 255.).round() as u8))
  })
}

fn delay_ms(delay: image::Delay) -> u32 {
  let (numer, denom) = delay.numer_denom_ms();
  numer.checked_div(denom).unwrap_or(0)
}

/// `effects` done to an RGBA frame shown at `at` milliseconds.
fn apply(buffer: image::RgbaImage, effects: Effects, at: u32) -> image::RgbaImage {
  use image::imageops;
  let mut buffer = match effects.turn {
    Turn::None => buffer,
    Turn::FlipX => imageops::flip_horizontal(&buffer),
    Turn::FlipY => imageops::flip_vertical(&buffer),
    Turn::Left => imageops::rotate270(&buffer),
    Turn::Right => imageops::rotate90(&buffer),
  };
  let filters = match effects.tint {
    Tint::None => return buffer,
    Tint::Cursed => vec![
      Filter::Matrix(grayscale()),
      Filter::Linear(0.7, 0.),
      Filter::Linear(2.5, 0.5 - 0.5 * 2.5),
    ],
    Tint::Party => {
      let turn = (at % PARTY_PERIOD_MS) as f32 / PARTY_PERIOD_MS as f32;
      vec![
        Filter::Matrix(sepia(0.5)),
        Filter::Matrix(hue_rotate(turn * std::f32::consts::TAU)),
        Filter::Matrix(saturate(2.5)),
      ]
    }
  };
  for px in buffer.pixels_mut() {
    let mut rgb = [px[0], px[1], px[2]].map(|c| f32::from(c) / 255.);
    for filter in &filters {
      rgb = filter.on(rgb);
    }
    for (channel, value) in px.0.iter_mut().zip(rgb) {
      *channel = (value * 255.).round() as u8;
    }
  }
  buffer
}

/// One of CSS's filter functions on straight sRGB values in 0..=1, clamped after each, as a
/// browser chains them.
enum Filter {
  /// A color matrix, a row per output channel.
  Matrix([[f32; 3]; 3]),
  /// `c × slope + intercept` on each channel (brightness, contrast).
  Linear(f32, f32),
}

impl Filter {
  fn on(&self, rgb: [f32; 3]) -> [f32; 3] {
    let out = match self {
      Self::Matrix(m) => m.map(|row| row[0] * rgb[0] + row[1] * rgb[1] + row[2] * rgb[2]),
      Self::Linear(slope, intercept) => rgb.map(|c| c * slope + intercept),
    };
    out.map(|c| c.clamp(0., 1.))
  }
}

/// CSS `grayscale(1)`.
fn grayscale() -> [[f32; 3]; 3] {
  let row = [0.2126, 0.7152, 0.0722];
  [row, row, row]
}

/// CSS `sepia(amount)`.
fn sepia(amount: f32) -> [[f32; 3]; 3] {
  let a = 1. - amount;
  [
    [0.393 + 0.607 * a, 0.769 - 0.769 * a, 0.189 - 0.189 * a],
    [0.349 - 0.349 * a, 0.686 + 0.314 * a, 0.168 - 0.168 * a],
    [0.272 - 0.272 * a, 0.534 - 0.534 * a, 0.131 + 0.869 * a],
  ]
}

/// CSS `hue-rotate(angle)`, radians.
fn hue_rotate(angle: f32) -> [[f32; 3]; 3] {
  let (sin, cos) = angle.sin_cos();
  [
    [
      0.213 + cos * 0.787 - sin * 0.213,
      0.715 - cos * 0.715 - sin * 0.715,
      0.072 - cos * 0.072 + sin * 0.928,
    ],
    [
      0.213 - cos * 0.213 + sin * 0.143,
      0.715 + cos * 0.285 + sin * 0.140,
      0.072 - cos * 0.072 - sin * 0.283,
    ],
    [
      0.213 - cos * 0.213 - sin * 0.787,
      0.715 - cos * 0.715 + sin * 0.715,
      0.072 + cos * 0.928 + sin * 0.072,
    ],
  ]
}

/// CSS `saturate(amount)`.
fn saturate(s: f32) -> [[f32; 3]; 3] {
  [
    [0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s],
    [0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s],
    [0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s],
  ]
}

/// The frames of an animated GIF or WebP (full canvases, RGBA), at most [`MAX_FRAMES`]; `None` for
/// a still image or another format.
fn animation(bytes: &[u8]) -> Option<Vec<image::Frame>> {
  use image::AnimationDecoder;
  let cursor = std::io::Cursor::new(bytes);
  let frames = match image::guess_format(bytes).ok()? {
    image::ImageFormat::Gif => image::codecs::gif::GifDecoder::new(cursor)
      .ok()?
      .into_frames(),
    image::ImageFormat::WebP => {
      let decoder = image::codecs::webp::WebPDecoder::new(cursor).ok()?;
      if !decoder.has_animation() {
        return None;
      }
      decoder.into_frames()
    }
    _ => return None,
  };
  let frames: Vec<image::Frame> = frames.take(MAX_FRAMES).filter_map(Result::ok).collect();
  // A single frame is drawn as a still image.
  (frames.len() > 1).then_some(frames)
}

/// GPUI sprites are BGRA: swap the red and blue channels of RGBA pixels.
pub fn bgra_image(width: u32, height: u32, mut pixels: Vec<u8>) -> Option<Arc<RenderImage>> {
  for px in pixels.as_chunks_mut::<4>().0 {
    px.swap(0, 2);
  }
  let buffer = image::RgbaImage::from_raw(width, height, pixels)?;
  Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// A PNG copy of the image at `url` on disk, `width`×`height` pixels (cropped to fill), for
/// what reads a file rather than bytes (Windows' notifications take neither a URL nor WebP).
/// Kept in the disk cache, so "Clear" removes it too; `None` when the image cannot be had.
pub fn local_png(
  url: &str,
  (width, height): (u32, u32),
  cx: &gpui::App,
) -> impl Future<Output = Option<PathBuf>> + use<> {
  let store = cx.try_global::<Images>().map(|images| images.0.read(cx));
  let fetcher = store.map(|store| store.fetcher_for(url));
  let dir = disk_cache_dir();
  let url = url.to_owned();
  let task = fetcher.zip(dir).map(|(fetcher, dir)| {
    runtime::spawn(cx, async move {
      // A square keeps its former name, so the avatars already written stay found.
      let size = if width == height {
        width.to_string()
      } else {
        format!("{width}x{height}")
      };
      let png = dir.join(format!(
        "{}-{size}.png",
        cache_name(&url).trim_end_matches(".img")
      ));
      let cached = dir.join(cache_name(&url));
      let (done, cached) = tokio::task::spawn_blocking({
        let png = png.clone();
        move || (png.is_file(), std::fs::read(cached).ok())
      })
      .await
      .ok()?;
      if done {
        return Some(png);
      }
      let bytes = match cached {
        Some(bytes) => bytes,
        None => fetcher
          .fetch(&url)
          .await
          .map_err(|e| tracing::debug!("image: {e}"))
          .ok()?,
      };
      // Decoding, scaling and encoding are CPU work: off the network runtime's two threads.
      tokio::task::spawn_blocking(move || {
        let image = image::load_from_memory(&bytes)
          .map_err(|e| tracing::debug!("image: {e}"))
          .ok()?
          .resize_to_fill(width, height, image::imageops::FilterType::Lanczos3);
        std::fs::create_dir_all(&dir).ok()?;
        image
          .save_with_format(&png, image::ImageFormat::Png)
          .map_err(|e| tracing::warn!("cannot save {}: {e}", png.display()))
          .ok()?;
        Some(png)
      })
      .await
      .ok()
      .flatten()
    })
  });
  async move { task?.await.flatten() }
}

/// Folder of the image disk cache.
pub fn disk_cache_dir() -> Option<PathBuf> {
  dirs::cache_dir().map(|d| d.join("peeby").join("images"))
}

/// Total size of the image disk cache, bytes. Blocking: call it off the UI thread.
pub fn disk_cache_size() -> u64 {
  let Some(entries) = disk_cache_dir().and_then(|dir| std::fs::read_dir(dir).ok()) else {
    return 0;
  };
  entries
    .filter_map(|e| e.ok()?.metadata().ok())
    .filter(|m| m.is_file())
    .map(|m| m.len())
    .sum()
}

/// Deletes the cached image files. Blocking: call it off the UI thread.
pub fn clear_disk_cache() {
  let Some(entries) = disk_cache_dir().and_then(|dir| std::fs::read_dir(dir).ok()) else {
    return;
  };
  for entry in entries.flatten() {
    if entry.path().is_file()
      && let Err(e) = std::fs::remove_file(entry.path())
    {
      tracing::warn!("cannot delete {}: {e}", entry.path().display());
    }
  }
}

/// Creates the store and registers it as a global.
pub fn init(fetcher: Arc<dyn Fetch>, cx: &mut gpui::App) {
  let images = Images(cx.new(|_| ImageStore::new(fetcher)));
  cx.set_global(images);
}

/// Fetches the URLs that start with `prefix` with `fetcher` (a signed-in server), or with the
/// default fetcher again when `None` (signed out). Images that failed under the previous route
/// are tried again.
pub fn route(prefix: &str, fetcher: Option<Arc<dyn Fetch>>, cx: &mut gpui::App) {
  let Some(images) = cx.try_global::<Images>().cloned() else {
    return;
  };
  images.0.update(cx, |store, _| {
    store.routes.retain(|(p, _)| p != prefix);
    if let Some(fetcher) = fetcher {
      store.routes.push((prefix.to_owned(), fetcher));
    }
    store.failed.retain(|url| !url.starts_with(prefix));
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A GIF of `colors.len()` 4×4 frames, 50 ms each.
  fn gif(colors: &[[u8; 4]]) -> Vec<u8> {
    let mut bytes = Vec::new();
    {
      let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
      for color in colors {
        let buffer = image::RgbaImage::from_pixel(4, 4, image::Rgba(*color));
        let delay = image::Delay::from_numer_denom_ms(50, 1);
        encoder
          .encode_frame(image::Frame::from_parts(buffer, 0, 0, delay))
          .expect("a frame");
      }
    }
    bytes
  }

  #[test]
  fn animated_gifs_keep_their_frames_as_bgra() {
    let image = decode(&gif(&[[255, 0, 0, 255], [0, 0, 255, 255]]), None).expect("an image");
    assert_eq!(image.frame_count(), 2);
    // BGRA: red is stored last.
    assert_eq!(&image.as_bytes(0).expect("frame 0")[..4], &[0, 0, 255, 255]);
    assert_eq!(&image.as_bytes(1).expect("frame 1")[..4], &[255, 0, 0, 255]);
    assert_eq!(image_bytes(&image), 2 * 4 * 4 * 4);
  }

  #[test]
  fn effects_turn_and_tint_the_pixels() {
    // 2×1: red then blue.
    let mut png = Vec::new();
    image::RgbaImage::from_fn(2, 1, |x, _| {
      image::Rgba(if x == 0 {
        [255, 0, 0, 255]
      } else {
        [0, 0, 255, 255]
      })
    })
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .expect("a PNG");
    let with = |turn, tint| decode_with(&png, None, Effects { turn, tint }).expect("an image");
    // Blue first now (BGRA).
    let flipped = with(Turn::FlipX, Tint::None);
    assert_eq!(&flipped.as_bytes(0).expect("frame")[..4], &[255, 0, 0, 255]);
    let turned = with(Turn::Right, Tint::None);
    assert_eq!((turned.size(0).width.0, turned.size(0).height.0), (1, 2));
    // Gray: the same in every channel, alpha kept.
    let cursed = with(Turn::None, Tint::Cursed);
    let px = &cursed.as_bytes(0).expect("frame")[..4];
    assert!(px[0] == px[1] && px[1] == px[2] && px[3] == 255);
    let party = with(Turn::None, Tint::Party);
    assert_eq!(party.frame_count(), PARTY_FRAMES as usize);
    assert_ne!(party.as_bytes(0), party.as_bytes(PARTY_FRAMES as usize / 2));
  }

  #[test]
  fn buckets_round_up_by_about_a_sixteenth() {
    assert_eq!(bucket(0), 8);
    assert_eq!(bucket(40), 40);
    assert_eq!(bucket(41), 48);
    assert_eq!(bucket(168), 176);
    assert_eq!(bucket(1000), 1024);
    assert!((1..4000).all(|n| bucket(n) >= n && bucket(n) <= n + n / 8 + 8));
  }

  #[test]
  fn fits_shrink_keeping_the_ratio_and_never_enlarge() {
    let wide = image::RgbaImage::from_pixel(400, 100, image::Rgba([10, 20, 30, 255]));
    let cover = shrink(wide.clone(), Fit::Cover, 100, 100);
    assert_eq!(cover.dimensions(), (400, 100));
    let cover = shrink(wide.clone(), Fit::Cover, 100, 50);
    assert_eq!(cover.dimensions(), (200, 50));
    let contain = shrink(wide.clone(), Fit::Contain, 100, 100);
    assert_eq!(contain.dimensions(), (100, 25));
    assert_eq!(
      shrink(wide, Fit::Contain, 800, 800).dimensions(),
      (400, 100)
    );
  }

  #[test]
  fn transparent_pixels_do_not_darken_the_edges() {
    // White on the left, transparent black on the right.
    let half = image::RgbaImage::from_fn(64, 8, |x, _| {
      image::Rgba(if x < 32 {
        [255, 255, 255, 255]
      } else {
        [0, 0, 0, 0]
      })
    });
    let shrunk = shrink(half, Fit::Contain, 16, 2);
    for px in shrunk.pixels().filter(|px| px[3] > 8) {
      assert!(px[0] > 240, "{px:?}");
    }
  }

  #[test]
  fn a_single_frame_gif_is_a_still_image() {
    let image = decode(&gif(&[[0, 255, 0, 255]]), Some((2, 2))).expect("an image");
    assert_eq!(image.frame_count(), 1);
    assert_eq!(image_bytes(&image), 2 * 2 * 4);
  }
}
