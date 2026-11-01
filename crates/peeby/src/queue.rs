//! Play queue of a playlist: the order of play (shuffled or not), repeat, and the next videos
//! of long playlists, loaded as the end comes near. Shared by the watch pages that play it:
//! each one shows the queue and asks it for the next video when its own ends; the queue
//! answers with a [`QueueEvent::Play`], which replaces that page with the chosen video's.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use gpui::{Context, EventEmitter, Subscription};
use innertube::{Client, VideoItem};

use crate::prefetch;
use crate::save::{self, EntryRemoved};
use ui::runtime;

/// Load the next videos when fewer than this many are left to play.
const LOAD_AHEAD: usize = 5;

pub enum QueueEvent {
  /// Play the current video (it changed).
  Play,
}

pub struct Queue {
  client: Client,
  pub playlist_id: String,
  pub title: String,
  items: Vec<VideoItem>,
  /// Indexes in `items`, in play order.
  order: Vec<usize>,
  /// Place of the current video in `order`.
  position: usize,
  shuffle: bool,
  repeat: bool,
  continuation: Option<String>,
  loading: bool,
  /// The end was reached while the next videos were loading: play the first of them.
  advance_when_loaded: bool,
  rng: Rng,
  _edits: Subscription,
}

impl EventEmitter<QueueEvent> for Queue {}

impl Queue {
  /// A queue of a playlist's `items`, starting at `items[start]`, or at a random video when
  /// shuffled without `start`. `continuation` loads the playlist's next videos.
  #[allow(clippy::too_many_arguments)] // The playlist's parts, then where to start.
  pub fn new(
    client: Client,
    playlist_id: String,
    title: String,
    items: Vec<VideoItem>,
    continuation: Option<String>,
    start: Option<usize>,
    shuffle: bool,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut rng = Rng::new();
    let start = start
      .filter(|&i| i < items.len())
      .unwrap_or_else(|| if shuffle { rng.below(items.len()) } else { 0 });
    // An entry removed from this playlist (here or on its page) leaves the queue.
    let edits = save::edits(cx);
    let edits = cx.subscribe(&edits, |queue, _, removed: &EntryRemoved, cx| {
      if removed.playlist_id == queue.playlist_id {
        queue.drop_entry(&removed.set_video_id);
        cx.notify();
      }
    });
    let mut queue = Self {
      client,
      playlist_id,
      title,
      order: (0..items.len()).collect(),
      position: start,
      items,
      shuffle: false,
      repeat: false,
      continuation,
      loading: false,
      advance_when_loaded: false,
      rng,
      _edits: edits,
    };
    if shuffle {
      queue.reshuffle();
    }
    queue
  }

  pub fn items(&self) -> &[VideoItem] {
    &self.items
  }

  /// Index in [`Self::items`] of the video playing.
  pub fn current(&self) -> Option<usize> {
    self.order.get(self.position).copied()
  }

  pub fn current_video(&self) -> Option<&VideoItem> {
    self.items.get(self.current()?)
  }

  /// Videos in play order, as indexes in [`Self::items`].
  pub fn order(&self) -> &[usize] {
    &self.order
  }

  /// 1-based place of the current video in the play order.
  pub fn position(&self) -> usize {
    self.position + 1
  }

  pub fn shuffle(&self) -> bool {
    self.shuffle
  }

  pub fn repeat(&self) -> bool {
    self.repeat
  }

  /// Whether more videos are still to be loaded from YouTube.
  pub fn has_more(&self) -> bool {
    self.continuation.is_some()
  }

  pub fn has_previous(&self) -> bool {
    self.position > 0
  }

  pub fn has_next(&self) -> bool {
    self.position + 1 < self.order.len()
      || self.continuation.is_some()
      || (self.repeat && !self.order.is_empty())
  }

  /// Called by the page that starts playing the current video: resolves the next video's
  /// stream ahead, and loads more videos when few are left.
  pub fn started(&mut self, cx: &mut Context<Self>) {
    if let Some(&next) = self.order.get(self.position + 1) {
      let id = self.items[next].id.clone();
      prefetch::warm(&id, cx);
    }
    if self.order.len() - self.position <= LOAD_AHEAD {
      self.load_more(cx);
    }
  }

  /// The next video: the following one, the first again with repeat on, or the first of the
  /// next page once it has loaded. Nothing at the end of the playlist.
  pub fn next(&mut self, cx: &mut Context<Self>) {
    if self.position + 1 < self.order.len() {
      self.play_at(self.position + 1, cx);
    } else if self.continuation.is_some() {
      self.advance_when_loaded = true;
      self.load_more(cx);
    } else if self.repeat && !self.order.is_empty() {
      if self.shuffle {
        self.position = 0;
        let first = self.rng.below(self.order.len());
        self.order.swap(0, first);
        self.shuffle_after_current();
      }
      self.play_at(0, cx);
    }
  }

  pub fn previous(&mut self, cx: &mut Context<Self>) {
    if self.position > 0 {
      self.play_at(self.position - 1, cx);
    }
  }

  /// Plays `items[index]`, chosen in the list.
  pub fn jump(&mut self, index: usize, cx: &mut Context<Self>) {
    if let Some(position) = self.order.iter().position(|&i| i == index) {
      self.play_at(position, cx);
    }
  }

  /// Drops the entry `set_video_id` (removed from the playlist), unless it is playing.
  fn drop_entry(&mut self, set_video_id: &str) {
    let Some(index) = self
      .items
      .iter()
      .position(|v| v.set_video_id.as_deref() == Some(set_video_id))
    else {
      return;
    };
    if self.current() == Some(index) {
      return;
    }
    self.items.remove(index);
    if let Some(place) = self.order.iter().position(|&i| i == index) {
      self.order.remove(place);
      if place < self.position {
        self.position -= 1;
      }
    }
    for i in &mut self.order {
      if *i > index {
        *i -= 1;
      }
    }
  }

  pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
    if self.shuffle {
      self.shuffle = false;
      let current = self.current().unwrap_or(0);
      self.order = (0..self.items.len()).collect();
      self.position = current;
    } else {
      self.reshuffle();
    }
    cx.notify();
  }

  pub fn toggle_repeat(&mut self, cx: &mut Context<Self>) {
    self.repeat = !self.repeat;
    cx.notify();
  }

  fn play_at(&mut self, position: usize, cx: &mut Context<Self>) {
    self.position = position;
    if self.current().is_some() {
      cx.emit(QueueEvent::Play);
    }
    cx.notify();
  }

  /// Shuffled order with the current video first, so it keeps playing.
  fn reshuffle(&mut self) {
    self.shuffle = true;
    let current = self.current().unwrap_or(0);
    self.order = (0..self.items.len()).collect();
    self.position = 0;
    if current < self.order.len() {
      self.order.swap(0, current);
    }
    self.shuffle_after_current();
  }

  /// Fisher-Yates over the videos after the current one.
  fn shuffle_after_current(&mut self) {
    let start = self.position + 1;
    for i in (start + 1..self.order.len()).rev() {
      let j = start + self.rng.below(i - start + 1);
      self.order.swap(i, j);
    }
  }

  fn load_more(&mut self, cx: &mut Context<Self>) {
    let Some(token) = self.continuation.clone() else {
      return;
    };
    if self.loading {
      return;
    }
    self.loading = true;
    let client = self.client.clone();
    let fetch = runtime::spawn(cx, async move { client.playlist_more(&token).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |queue, cx| {
          queue.loading = false;
          match result {
            Some(Ok(page)) => {
              tracing::info!("queue: {} more videos", page.items.len());
              queue.append(page.items);
              queue.continuation = page.continuation;
            }
            Some(Err(e)) => {
              tracing::warn!("queue: {e}");
              // Stop at what is loaded rather than retrying in a loop.
              queue.continuation = None;
            }
            None => {}
          }
          if std::mem::take(&mut queue.advance_when_loaded) {
            queue.next(cx);
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// New videos join the end of the order, or random places after the current video when
  /// shuffled: shuffling only mixes the videos loaded so far.
  fn append(&mut self, videos: Vec<VideoItem>) {
    for video in videos {
      let index = self.items.len();
      self.items.push(video);
      if self.shuffle {
        let slot = self.position + 1 + self.rng.below(self.order.len() - self.position);
        self.order.insert(slot, index);
      } else {
        self.order.push(index);
      }
    }
  }
}

/// A small xorshift generator: shuffling needs no more, and it avoids a dependency.
struct Rng(u64);

impl Rng {
  fn new() -> Self {
    // `RandomState` is seeded randomly per process and per instance.
    let seed = RandomState::new().build_hasher().finish();
    Self(seed | 1)
  }

  fn next(&mut self) -> u64 {
    self.0 ^= self.0 << 13;
    self.0 ^= self.0 >> 7;
    self.0 ^= self.0 << 17;
    self.0
  }

  /// A number in `0..n` (0 when `n` is 0).
  fn below(&mut self, n: usize) -> usize {
    if n == 0 {
      0
    } else {
      (self.next() % n as u64) as usize
    }
  }
}

#[cfg(test)]
mod tests {
  use super::Rng;

  #[test]
  fn rng_stays_in_range() {
    let mut rng = Rng::new();
    assert_eq!(rng.below(0), 0);
    assert!((0..1000).all(|_| rng.below(7) < 7));
  }
}
