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
      self.load(url.to_owned(), url.to_owned(), None, cx);
    }
    None
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
      self.load(url.to_owned(), key, Some((width, height)), cx);
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

  /// Loads `url` into the store under `key`, scaled to `size` when given. The disk cache keeps
  /// the downloaded file, whatever the size.
  fn load(&mut self, url: String, key: String, size: Option<(u32, u32)>, cx: &mut Context<Self>) {
    self.pending.insert(key.clone());
    let fetcher = self.fetcher_for(&url);
    let downloads = self.downloads.clone();
    let path = self
      .cache_dir
      .as_ref()
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
        Some(bytes) => executor.spawn(async move { decode(&bytes, size) }).await,
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
/// their frames and delays: an `img` with an id plays them (GPUI's own animation).
pub fn decode(bytes: &[u8], size: Option<(u32, u32)>) -> Option<Arc<RenderImage>> {
  if let Some(frames) = animation(bytes) {
    let frames: Vec<image::Frame> = frames
      .into_iter()
      .map(|frame| {
        let delay = frame.delay();
        let mut buffer = frame.into_buffer();
        if let Some((w, h)) = size {
          buffer = image::DynamicImage::ImageRgba8(buffer)
            .resize_to_fill(w.max(1), h.max(1), image::imageops::FilterType::Lanczos3)
            .into_rgba8();
        }
        for px in buffer.as_chunks_mut::<4>().0 {
          px.swap(0, 2);
        }
        image::Frame::from_parts(buffer, 0, 0, delay)
      })
      .collect();
    return Some(Arc::new(RenderImage::new(frames)));
  }
  let mut image = image::load_from_memory(bytes).ok()?;
  if let Some((w, h)) = size {
    image = image.resize_to_fill(w.max(1), h.max(1), image::imageops::FilterType::Lanczos3);
  }
  let rgba = image.into_rgba8();
  let (w, h) = rgba.dimensions();
  bgra_image(w, h, rgba.into_raw())
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
  fn a_single_frame_gif_is_a_still_image() {
    let image = decode(&gif(&[[0, 255, 0, 255]]), Some((2, 2))).expect("an image");
    assert_eq!(image.frame_count(), 1);
    assert_eq!(image_bytes(&image), 2 * 2 * 4);
  }
}
