//! Streams resolved ahead of the click: hovering a video card for a moment resolves its stream
//! (VISIONOS, ~0.2 s), so the watch page can start mpv at once. Stream URLs stay valid 6 h.
//! Not while videos go through web_embedded first (the "Player" setting, ADR 0040).
//!
//! The same hover also asks for the watch page's `next` answer (title, details, related
//! videos) with the session's client, whatever the player, and loads the card's large thumbnail
//! (the player's poster) and the channel's avatar, so the page opens filled
//! (docs/responsiveness-plan.md, item 5). The page takes the answer, or waits for it while it
//! is on its way, rather than asking again.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, AppContext, Context, Entity, Global};
use innertube::{Client, Stream, Thumbnail, WatchPage};

use peeby_core::settings::Settings;
use ui::picture;
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

/// How long a prefetched `next` answer serves: its counts and the user's rating may change.
const PAGE_FRESH_FOR: Duration = Duration::from_secs(5 * 60);
/// A live's page: its chat's continuation and its counts move faster.
const LIVE_PAGE_FRESH_FOR: Duration = Duration::from_secs(30);
/// Prefetched pages kept; the oldest go first.
const PAGE_CAPACITY: usize = 16;
/// The width the watch page asks the poster at (`ui::player`'s poster), and the channel's
/// avatar beside the title.
const POSTER_WIDTH: f32 = 640.;
const AVATAR_SIZE: f32 = 40.;

/// Called with the prefetched page once it arrives, or `None` if the request failed (the page
/// then asks itself).
type PageWaiter = Box<dyn FnOnce(Option<WatchPage>, &mut App)>;

/// The `next` answers asked ahead of the clicks, a global.
#[derive(Default)]
struct Pages {
  ready: HashMap<String, (Instant, WatchPage)>,
  /// Requests on their way, and the watch pages waiting for them.
  pending: HashMap<String, Vec<PageWaiter>>,
}

impl Global for Pages {}

/// What a watch page gets from the prefetch.
pub enum PageClaim {
  /// The answer, already here.
  Ready(Box<WatchPage>),
  /// On its way: the waiter given gets it.
  Waiting,
  /// Nothing asked: the page asks itself.
  Missing,
}

fn page_fresh(at: Instant, page: &WatchPage) -> bool {
  let fresh_for = if page.live_chat.is_some() {
    LIVE_PAGE_FRESH_FOR
  } else {
    PAGE_FRESH_FOR
  };
  at.elapsed() < fresh_for
}

/// The prefetched `next` answer of `video_id` for its watch page, taken out of the cache; while
/// it is on its way, `on_page` gets it when it comes.
pub fn claim_page(
  video_id: &str,
  on_page: impl FnOnce(Option<WatchPage>, &mut App) + 'static,
  cx: &mut App,
) -> PageClaim {
  let pages = cx.default_global::<Pages>();
  if let Some((at, page)) = pages.ready.remove(video_id)
    && page_fresh(at, &page)
  {
    return PageClaim::Ready(Box::new(page));
  }
  match pages.pending.get_mut(video_id) {
    Some(waiters) => {
      waiters.push(Box::new(on_page));
      PageClaim::Waiting
    }
    None => PageClaim::Missing,
  }
}

/// Forgets the pages asked with the previous session's client. Called on every session change.
pub fn forget_pages(cx: &mut App) {
  cx.default_global::<Pages>().ready.clear();
}

/// Asks for `video_id`'s `next` answer with the session's client, unless it is here or on its
/// way.
fn fetch_page(video_id: String, cx: &mut App) {
  let Some(client) = cx.try_global::<crate::save::Session>().map(|s| s.0.clone()) else {
    return;
  };
  let pages = cx.default_global::<Pages>();
  pages.ready.retain(|_, (at, page)| page_fresh(*at, page));
  if pages.ready.contains_key(&video_id) || pages.pending.contains_key(&video_id) {
    return;
  }
  pages.pending.insert(video_id.clone(), Vec::new());
  let id = video_id.clone();
  let fetch = runtime::spawn(cx, async move { client.watch(&id).await });
  cx.spawn(async move |cx| {
    let result = fetch.await;
    cx.update(|cx| {
      let pages = cx.default_global::<Pages>();
      let waiters = pages.pending.remove(&video_id).unwrap_or_default();
      let page = match result {
        Some(Ok(page)) => Some(page),
        Some(Err(e)) => {
          tracing::debug!("prefetch page {video_id}: {e}");
          None
        }
        None => None,
      };
      if waiters.is_empty() {
        if let Some(page) = page {
          tracing::debug!("prefetched page {video_id}");
          if pages.ready.len() >= PAGE_CAPACITY
            && let Some(oldest) = pages
              .ready
              .iter()
              .min_by_key(|(_, (at, _))| *at)
              .map(|(k, _)| k.clone())
          {
            pages.ready.remove(&oldest);
          }
          pages.ready.insert(video_id, (Instant::now(), page));
        }
        return;
      }
      for waiter in waiters {
        waiter(page.clone(), cx);
      }
    });
  })
  .detach();
}

/// What a card knows of its video's pictures, loaded when it is hovered.
pub struct CardPictures {
  /// The video's thumbnails: the player's poster.
  pub thumbnails: Vec<Thumbnail>,
  /// The channel's avatar, beside the title.
  pub avatar: Vec<Thumbnail>,
}

/// Loads what a card's page draws first: the large thumbnail (its poster) and the channel's
/// avatar.
fn preload_pictures(pictures: &CardPictures, cx: &mut App) {
  let poster = crate::convert::picture(&pictures.thumbnails);
  picture::preload(&poster, POSTER_WIDTH, 16. / 9., None, cx);
  if !pictures.avatar.is_empty() {
    let avatar = crate::convert::picture(&pictures.avatar);
    picture::preload(&avatar, AVATAR_SIZE, 1., None, cx);
  }
}

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
pub fn hover(video_id: &str, pictures: &Rc<CardPictures>, hovered: bool, cx: &mut App) {
  let Some(Handle(streams)) = cx.try_global::<Handle>().cloned() else {
    return;
  };
  let id = video_id.to_owned();
  let pictures = pictures.clone();
  streams.update(cx, |streams, cx| {
    if hovered {
      streams.hovered = Some(id.clone());
      cx.spawn(async move |this, cx| {
        cx.background_executor().timer(HOVER_DELAY).await;
        this
          .update(cx, |streams, cx| {
            if streams.hovered.as_deref() == Some(id.as_str()) {
              streams.fetch(id.clone(), cx);
              fetch_page(id, cx);
              preload_pictures(&pictures, cx);
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

/// The storyboard of `video_id`'s prefetched stream, if it has one (left in the cache), for its
/// card's scrubbing.
pub fn storyboard(video_id: &str, cx: &App) -> Option<innertube::Storyboard> {
  let Handle(streams) = cx.try_global::<Handle>()?;
  let (_, stream) = streams.read(cx).ready.get(video_id)?;
  stream.storyboard.clone()
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
    // Videos go through web_embedded first (the "Player" setting, signed in): a VISIONOS stream
    // would not be used, and its anonymous request would only feed the bot check.
    if cx
      .try_global::<crate::save::Session>()
      .is_some_and(|session| crate::watch::embedded_first(&session.0, cx))
    {
      return;
    }
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
