//! Remote images (thumbnails, posters, avatars): download on the network runtime, disk cache,
//! decode off the UI thread, in-memory LRU of GPU-ready images. Shared by every space: the app
//! gives the store a [`Fetch`], so it depends on no service client.

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
  downloads: Arc<Semaphore>,
  cache_dir: Option<PathBuf>,
  ready: HashMap<String, Cached>,
  /// Ticks on every lookup; `Cached::used` holds its value at the image's last use.
  clock: u64,
  bytes: usize,
  pending: HashSet<String>,
  failed: HashSet<String>,
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
      downloads: Arc::new(Semaphore::new(MAX_DOWNLOADS)),
      cache_dir,
      ready: HashMap::new(),
      clock: 0,
      bytes: 0,
      pending: HashSet::new(),
      failed: HashSet::new(),
    }
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
    let fetcher = self.fetcher.clone();
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
  let size = image.size(0);
  size.width.0 as usize * size.height.0 as usize * 4
}

fn cache_name(key: &str) -> String {
  let mut hasher = DefaultHasher::new();
  key.hash(&mut hasher);
  format!("{:016x}.img", hasher.finish())
}

/// Decodes an image, scaled to fill `size` (device pixels) when given.
pub fn decode(bytes: &[u8], size: Option<(u32, u32)>) -> Option<Arc<RenderImage>> {
  let mut image = image::load_from_memory(bytes).ok()?;
  if let Some((w, h)) = size {
    image = image.resize_to_fill(w.max(1), h.max(1), image::imageops::FilterType::Lanczos3);
  }
  let rgba = image.into_rgba8();
  let (w, h) = rgba.dimensions();
  bgra_image(w, h, rgba.into_raw())
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
