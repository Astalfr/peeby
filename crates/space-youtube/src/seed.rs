//! What a card knows of a video, handed to its watch page (docs/responsiveness-plan.md, item 1):
//! while the page waits for `next`, it shows the card's title and channel, and the player the
//! card's thumbnail until mpv's first frame. Kept out of `Route` (history and restored tabs keep
//! the id alone): the card leaves it here, keyed by video id, and the page takes it.

use std::collections::HashMap;

use gpui::{App, Global};
use innertube::{Thumbnail, VideoItem};

/// Seeds left unclaimed (tabs opened in the background and never shown) are dropped past this.
const CAPACITY: usize = 32;

/// What the card showed.
#[derive(Clone, Debug)]
pub struct Seed {
  pub title: String,
  pub channel: Option<String>,
  pub avatar: Vec<Thumbnail>,
  pub thumbnails: Vec<Thumbnail>,
}

#[derive(Default)]
struct Seeds(HashMap<String, Seed>);

impl Global for Seeds {}

impl Seeds {
  fn offer(&mut self, video: &VideoItem) {
    if self.0.len() >= CAPACITY {
      self.0.clear();
    }
    self.0.insert(
      video.id.clone(),
      Seed {
        title: video.title.clone(),
        channel: video.channel.clone(),
        avatar: video.avatar.clone(),
        thumbnails: video.thumbnails.clone(),
      },
    );
  }

  fn take(&mut self, video_id: &str) -> Option<Seed> {
    self.0.remove(video_id)
  }
}

/// Before opening `video`'s page from its card (or a queue's row).
pub fn offer(video: &VideoItem, cx: &mut App) {
  cx.default_global::<Seeds>().offer(video);
}

/// The seed of `video_id`'s page, once: a page opened without a card (a link, the command
/// line, a restored tab) has none.
pub fn take(video_id: &str, cx: &mut App) -> Option<Seed> {
  cx.default_global::<Seeds>().take(video_id)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn video(id: &str) -> VideoItem {
    VideoItem {
      id: id.into(),
      title: format!("Title {id}"),
      channel: Some("Channel".into()),
      channel_id: None,
      metadata: Vec::new(),
      duration: None,
      live: false,
      members_only: false,
      thumbnails: Vec::new(),
      avatar: Vec::new(),
      set_video_id: None,
      progress: None,
      not_interested: None,
    }
  }

  #[test]
  fn taken_once() {
    let mut seeds = Seeds::default();
    seeds.offer(&video("a"));
    assert_eq!(seeds.take("a").map(|s| s.title).as_deref(), Some("Title a"));
    assert!(seeds.take("a").is_none());
    assert!(seeds.take("b").is_none());
  }

  #[test]
  fn bounded() {
    let mut seeds = Seeds::default();
    for i in 0..=CAPACITY {
      seeds.offer(&video(&i.to_string()));
    }
    assert!(seeds.0.len() <= CAPACITY);
    // The newest is kept.
    assert!(seeds.take(&CAPACITY.to_string()).is_some());
  }
}
