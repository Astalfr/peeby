//! Streams resolved ahead of the click: hovering a video card for a moment resolves its stream
//! (VISIONOS, ~0.2 s), so the watch page can start mpv at once. Stream URLs stay valid 6 h.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Context, Entity, Global};
use innertube::{Client, Stream};

use crate::runtime;
use crate::settings::Settings;

/// The mouse must rest this long on a card: sweeping over a grid fetches nothing.
const HOVER_DELAY: Duration = Duration::from_millis(300);
/// Under the 6 h validity of the URLs, with margin for a long video.
const FRESH_FOR: Duration = Duration::from_secs(4 * 3600);
/// Resolved streams kept; the oldest go first.
const CAPACITY: usize = 64;

pub struct Streams {
  client: Client,
  ready: HashMap<String, (Instant, Stream)>,
  pending: HashSet<String>,
  /// The card under the mouse, if any.
  hovered: Option<String>,
}

#[derive(Clone)]
struct Handle(Entity<Streams>);

impl Global for Handle {}

/// Creates the store and registers it as a global.
pub fn init(client: Client, cx: &mut App) {
  let streams = cx.new(|_| Streams {
    client,
    ready: HashMap::new(),
    pending: HashSet::new(),
    hovered: None,
  });
  cx.set_global(Handle(streams));
}

/// The mouse entered (`true`) or left a card of `video_id`.
pub fn hover(video_id: &str, hovered: bool, cx: &mut App) {
  let Some(Handle(streams)) = cx.try_global::<Handle>().cloned() else {
    return;
  };
  let id = video_id.to_owned();
  streams.update(cx, |streams, cx| {
    if hovered {
      streams.hovered = Some(id.clone());
      cx.spawn(async move |this, cx| {
        cx.background_executor().timer(HOVER_DELAY).await;
        this
          .update(cx, |streams, cx| {
            if streams.hovered.as_deref() == Some(id.as_str()) {
              streams.fetch(id, cx);
            }
          })
          .ok();
      })
      .detach();
    } else if streams.hovered.as_deref() == Some(id.as_str()) {
      streams.hovered = None;
    }
  });
}

/// Resolves the stream of `video_id` now, e.g. the next video of a playlist, so it starts at
/// once when its turn comes.
pub fn warm(video_id: &str, cx: &mut App) {
  if let Some(Handle(streams)) = cx.try_global::<Handle>().cloned() {
    let id = video_id.to_owned();
    streams.update(cx, |streams, cx| streams.fetch(id, cx));
  }
}

/// A stream resolved recently for `video_id`, taken out of the cache.
pub fn take(video_id: &str, cx: &mut App) -> Option<Stream> {
  let Handle(streams) = cx.try_global::<Handle>().cloned()?;
  streams.update(cx, |streams, _| {
    let (at, stream) = streams.ready.remove(video_id)?;
    (at.elapsed() < FRESH_FOR).then_some(stream)
  })
}

impl Streams {
  fn fetch(&mut self, id: String, cx: &mut Context<Self>) {
    let fresh = self
      .ready
      .get(&id)
      .is_some_and(|(at, _)| at.elapsed() < FRESH_FOR);
    if fresh || !self.pending.insert(id.clone()) {
      return;
    }
    let client = self.client.clone();
    let video = id.clone();
    let max_height = Settings::get(cx).max_height;
    let fetch = runtime::spawn(cx, async move { client.stream(&video, max_height).await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |streams, _| {
          streams.pending.remove(&id);
          match result {
            Some(Ok(stream)) => {
              tracing::debug!("prefetched {id}");
              if streams.ready.len() >= CAPACITY
                && let Some(oldest) = streams
                  .ready
                  .iter()
                  .min_by_key(|(_, (at, _))| *at)
                  .map(|(k, _)| k.clone())
              {
                streams.ready.remove(&oldest);
              }
              streams.ready.insert(id, (Instant::now(), stream));
            }
            // The watch page will try again, and fall back to yt-dlp if needed.
            Some(Err(e)) => tracing::debug!("prefetch {id}: {e}"),
            None => {}
          }
        })
        .ok();
    })
    .detach();
  }
}
