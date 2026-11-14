//! Streams resolved ahead of the click: hovering a video card for a moment resolves its stream
//! (VISIONOS, ~0.2 s), so the watch page can start mpv at once. Stream URLs stay valid 6 h.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Context, Entity, Global};
use innertube::{Client, Stream};

use peeby_core::settings::Settings;
use ui::runtime;

use crate::convert::error_text;

/// The mouse must rest this long on a card: sweeping over a grid fetches nothing.
const HOVER_DELAY: Duration = Duration::from_millis(300);
/// Under the 6 h validity of the URLs, with margin for a long video.
const FRESH_FOR: Duration = Duration::from_secs(4 * 3600);
/// A live's window grows while it is younger than its DVR length: past this, a prefetched live
/// would start too far behind its edge (HLS), and its proxy should not run for nothing.
const LIVE_FRESH_FOR: Duration = Duration::from_secs(10);
/// Resolved streams kept; the oldest go first.
const CAPACITY: usize = 64;
/// How long a video VISIONOS refused with `LOGIN_REQUIRED` goes straight to yt-dlp. YouTube's
/// bot check ("Sign in to confirm you're not a bot") refuses some videos to every anonymous
/// client from an IP for hours, while others still play: asking again for those only costs
/// requests (which feed the check) and a second before yt-dlp starts.
const REFUSED_FOR: Duration = Duration::from_secs(30 * 60);

/// The streams resolved ahead of the clicks, a global.
pub struct Streams {
  client: Client,
  ready: HashMap<String, (Instant, Stream)>,
  /// Videos VISIONOS refused with `LOGIN_REQUIRED`: when, and the error to show if yt-dlp fails.
  refused: HashMap<String, (Instant, String)>,
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
    refused: HashMap::new(),
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
    fresh(at, &stream).then_some(stream)
  })
}

/// The error VISIONOS gave `video_id` if it refused it with `LOGIN_REQUIRED` in the last
/// [`REFUSED_FOR`]: the watch page then starts yt-dlp without asking VISIONOS again.
pub fn refused(video_id: &str, cx: &mut App) -> Option<String> {
  let Handle(streams) = cx.try_global::<Handle>().cloned()?;
  streams.update(cx, |streams, _| streams.refused_text(video_id))
}

/// Notes that VISIONOS refused `video_id` with `error` (kept only for `LOGIN_REQUIRED`).
pub fn note_refused(video_id: &str, error: &innertube::Error, cx: &mut App) {
  if let Some(Handle(streams)) = cx.try_global::<Handle>().cloned() {
    streams.update(cx, |streams, _| streams.note_refused(video_id, error));
  }
}

/// Whether a stream resolved `at` can still be played. A live's window may have grown since
/// (until it reaches its DVR length): it would start that much behind its edge.
fn fresh(at: Instant, stream: &Stream) -> bool {
  let fresh_for = if stream.live.is_some() {
    LIVE_FRESH_FOR
  } else {
    FRESH_FOR
  };
  at.elapsed() < fresh_for
}

impl Streams {
  fn refused_text(&mut self, video_id: &str) -> Option<String> {
    self.refused.retain(|_, (at, _)| at.elapsed() < REFUSED_FOR);
    self.refused.get(video_id).map(|(_, text)| text.clone())
  }

  fn note_refused(&mut self, video_id: &str, error: &innertube::Error) {
    if matches!(error, innertube::Error::Unplayable { status, .. } if status == "LOGIN_REQUIRED") {
      self
        .refused
        .insert(video_id.to_owned(), (Instant::now(), error_text(error)));
    }
  }

  fn fetch(&mut self, id: String, cx: &mut Context<Self>) {
    // A stale live holds its proxy (ADR 0025): dropped, it stops.
    self.ready.retain(|_, (at, stream)| fresh(*at, stream));
    let fresh = self
      .ready
      .get(&id)
      .is_some_and(|(at, stream)| fresh(*at, stream));
    if fresh || self.refused_text(&id).is_some() || !self.pending.insert(id.clone()) {
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
            Some(Err(e)) => {
              tracing::debug!("prefetch {id}: {e}");
              streams.note_refused(&id, &e);
            }
            None => {}
          }
        })
        .ok();
    })
    .detach();
  }
}
