//! Remote images (thumbnails, avatars): download on the network runtime, disk cache, decode off
//! the UI thread, in-memory LRU of GPU-ready images. Adapted from peeby's image store.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AppContext, Context, Entity, Global, RenderImage};
use innertube::Client;
use tokio::sync::Semaphore;

use crate::runtime;

/// Decoded images kept in memory (BGRA bytes), beyond which the least recently used are dropped.
const MEMORY_BUDGET: usize = 256 * 1024 * 1024;
/// Parallel downloads, so a fast scroll does not queue dozens of requests at once.
const MAX_DOWNLOADS: usize = 8;

/// Handle to the app-wide image store.
#[derive(Clone)]
pub struct Images(pub Entity<ImageStore>);

impl Global for Images {}

/// A decoded image and when it was last asked for, for LRU eviction.
struct Cached {
  image: Arc<RenderImage>,
  used: u64,
}

pub struct ImageStore {
  client: Client,
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
  fn new(client: Client) -> Self {
    let cache_dir = dirs::cache_dir().map(|d| d.join("peebtube").join("images"));
    if let Some(dir) = &cache_dir
      && let Err(e) = std::fs::create_dir_all(dir)
    {
      tracing::warn!("image disk cache disabled: {e}");
    }
    Self {
      client,
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
    self.clock += 1;
    if let Some(cached) = self.ready.get_mut(url) {
      cached.used = self.clock;
      return Some(cached.image.clone());
    }
    if !self.pending.contains(url) && !self.failed.contains(url) {
      self.load(url.to_owned(), cx);
    }
    None
  }

  fn load(&mut self, url: String, cx: &mut Context<Self>) {
    self.pending.insert(url.clone());
    let client = self.client.clone();
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
              client
                .fetch_bytes(&url)
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
        Some(bytes) => executor.spawn(async move { decode(&bytes, None) }).await,
        None => None,
      };
      this
        .update(cx, |store, cx| {
          store.pending.remove(&url);
          match image {
            Some(image) => store.insert(url, image, cx),
            None => {
              store.failed.insert(url);
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
  let mut pixels = rgba.into_raw();
  // GPUI sprites are BGRA: swap the red and blue channels.
  for px in pixels.as_chunks_mut::<4>().0 {
    px.swap(0, 2);
  }
  let buffer = image::RgbaImage::from_raw(w, h, pixels)?;
  Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

/// Creates the store and registers it as a global.
pub fn init(client: Client, cx: &mut gpui::App) {
  let images = Images(cx.new(|_| ImageStore::new(client)));
  cx.set_global(images);
}
