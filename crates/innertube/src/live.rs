//! The local playlist proxy of YouTube lives (ADR 0025): the whole broadcast, up to YouTube's
//! DVR length (12 h), from the live's DASH segments.
//!
//! VISIONOS's HLS playlists cover an hour at most, and on most lives the HLS segments before it
//! are gone, but the DASH formats of the same answer keep the broadcast since its first segment.
//! Each DASH segment is fetched by number (`&sq=N`) and is a whole MP4 (`ftyp`, `moov`, `moof`,
//! `mdat`), which mpv plays one after the other from an HLS playlist (not WebM: FFmpeg ends its
//! video at the first seek). So mpv plays
//! `http://127.0.0.1:<port>/<key>/<track>.m3u8`, a live media playlist the proxy writes on each
//! reload: segment numbers from the first one kept to the live edge, each URI a short local path
//! (`s/<track>/<sq>`) the proxy answers with a redirect to the signed URL. Twelve hours of signed
//! URLs (about 900 bytes each) would make a playlist of megabytes on every reload.
//!
//! The edge comes from YouTube's headers (`x-head-seqnum`, `x-head-time-millis`) on a `HEAD`
//! request, read again at most every [`HEAD_EVERY`] while mpv reloads; a segment's length is
//! measured from them (4.9675 s on a 29.97 fps live that announces 5). The first segment kept is
//! where YouTube's DVR length starts, unless it is gone (24/7 lives keep less than they announce):
//! a search in the background then finds the first one served (one byte asked of each: a `HEAD`
//! answers 200 even for a gone segment), and the playlists start there once mpv has read its
//! first ones, whose start it was given ([`LiveProxy::settled`]). The signed URLs are resolved again before they expire, and stay in
//! memory: never logged, never in an error. `key` is random, so another local program cannot
//! guess the paths. The server stops when the last [`LiveProxy`] clone is dropped.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::client::{Client, VISIONOS_USER_AGENT};
use crate::error::{Error, Result};
use crate::hls::Variant;
use crate::models::LiveFormat;
use crate::parse::parse_live_formats;

/// The edge is read again at most this often while mpv reloads (so about once per reload). A
/// track waiting at the edge takes a seek only once its playlist grows: an estimate running
/// behind YouTube's edge held a seek back about 5 s.
const HEAD_EVERY: Duration = Duration::from_secs(2);
/// The signed URLs are resolved again this long before they expire.
const REFRESH_BEFORE: Duration = Duration::from_secs(30 * 60);
/// Their lifetime when the answer does not say (`streamingData.expiresInSeconds`, 6 h).
const DEFAULT_EXPIRY: Duration = Duration::from_secs(6 * 3600);
/// A failed refresh is tried again after this.
const REFRESH_RETRY: Duration = Duration::from_secs(60);
/// An edge that has not moved for this long: the live may have ended.
const STALLED_AFTER: Duration = Duration::from_secs(60);
/// Segments the playlist stops before the estimated edge, which may run ahead of YouTube's (the
/// newest segment read is always listed).
const EDGE_MARGIN: u64 = 1;
/// Segments added after the first one found served, should the formats' numbers differ a little.
const FIRST_MARGIN: u64 = 2;
/// Below this edge number, a segment's length is the announced one, not the measured average.
const MEASURE_FROM: u64 = 60;
/// One request's limit, so that a stalled one does not hold a playlist reload.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// A running proxy for one live. Cheap to clone; the last clone dropped stops it.
#[derive(Clone)]
pub struct LiveProxy(Arc<Inner>);

struct Inner {
  shared: Arc<Shared>,
  /// The video tracks as qualities, best first, and the audio track's playlist.
  variants: Vec<Variant>,
  /// The track of the quality picked at the start, whose edge [`LiveProxy::length`] reads.
  main: usize,
  settled: watch::Receiver<Option<f64>>,
  tasks: Vec<JoinHandle<()>>,
}

impl Drop for Inner {
  fn drop(&mut self) {
    for task in &self.tasks {
      task.abort();
    }
  }
}

impl PartialEq for LiveProxy {
  fn eq(&self, other: &Self) -> bool {
    Arc::ptr_eq(&self.0, &other.0)
  }
}

impl std::fmt::Debug for LiveProxy {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("LiveProxy")
      .field("variants", &self.0.variants)
      .field("window", &self.window())
      .finish_non_exhaustive()
  }
}

/// One DASH format served as a playlist.
struct Track {
  format: LiveFormat,
  /// The signed URL, replaced on refresh.
  url: Mutex<String>,
  edge: Mutex<Option<Edge>>,
  /// The newest segment listed so far: a playlist never shrinks at its end.
  listed: std::sync::atomic::AtomicU64,
}

/// A format's live edge as last read.
#[derive(Clone, Copy, Debug)]
struct Edge {
  /// `x-head-seqnum`: the newest segment.
  sq: u64,
  /// Seconds per segment, measured (`x-head-time-millis / x-head-seqnum`) or announced.
  segment: f64,
  read_at: Instant,
  /// When `sq` last grew.
  moved_at: Instant,
}

impl Edge {
  /// The newest segment now, estimated from the last reading.
  fn now(&self, at: Instant) -> u64 {
    let elapsed = at.saturating_duration_since(self.read_at).as_secs_f64();
    self.sq + (elapsed / self.segment) as u64
  }
}

struct Shared {
  client: Client,
  video_id: String,
  http: reqwest::Client,
  key: String,
  tracks: Vec<Track>,
  /// The first segment listed, all tracks alike.
  first: Mutex<u64>,
  /// How far back segments are kept, seconds: the DVR length, then what the search found.
  window: Mutex<f64>,
  refresh_at: Mutex<Instant>,
  refreshing: AtomicBool,
  /// The live has ended: the playlists get `#EXT-X-ENDLIST`.
  ended: AtomicBool,
  /// The tracks mpv starts on (the quality picked and the audio): until each has served a
  /// playlist, a first segment found later is held, as mpv's start was computed from the first
  /// window.
  starting: [usize; 2],
  served: [AtomicBool; 2],
  /// The search's result while held: the first segment kept and the window it leaves.
  held: Mutex<Option<(u64, f64)>>,
  /// Told the window once the search's result applies ([`LiveProxy::settled`]).
  settle: watch::Sender<Option<f64>>,
}

impl LiveProxy {
  /// Starts a proxy for a live's `formats` ([`parse_live_formats`] of its `player` answer), the
  /// quality picked being the best up to `max_height`. Reads the edge of that quality and of
  /// the audio (one `HEAD` each, together), then looks for the first segment kept in the
  /// background. Call it on a tokio runtime.
  pub(crate) async fn start(
    client: Client,
    video_id: &str,
    player: &Value,
    formats: Vec<LiveFormat>,
    max_height: u32,
  ) -> Result<Self> {
    let (videos, audio) = pick(formats).ok_or_else(|| Error::Proxy("no playable format".into()))?;
    let main = videos
      .iter()
      .position(|f| short_side(f) <= max_height)
      .unwrap_or(videos.len() - 1);
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
      .await
      .map_err(|e| Error::Proxy(e.to_string()))?;
    let port = listener
      .local_addr()
      .map_err(|e| Error::Proxy(e.to_string()))?
      .port();
    let key = random_key();
    let http = reqwest::Client::builder()
      .user_agent(VISIONOS_USER_AGENT)
      .timeout(REQUEST_TIMEOUT)
      .build()?;
    // The audio is the last track.
    let audio_index = videos.len();
    let max_dvr = videos[main].max_dvr.max(audio.max_dvr);
    let base = format!("http://127.0.0.1:{port}/{key}");
    let variants = videos
      .iter()
      .enumerate()
      .map(|(i, f)| Variant {
        height: short_side(f),
        frame_rate: f.fps,
        url: format!("{base}/{i}.m3u8"),
        audio_url: Some(format!("{base}/{audio_index}.m3u8")),
        resolution: format!("{}x{}", f.width, f.height),
        codecs: format!("{},{}", f.codecs, audio.codecs),
      })
      .collect();
    let tracks = videos
      .into_iter()
      .chain(std::iter::once(audio))
      .map(|format| Track {
        url: Mutex::new(format.url.clone()),
        format,
        edge: Mutex::new(None),
        listed: std::sync::atomic::AtomicU64::new(0),
      })
      .collect();
    let expiry = player["streamingData"]["expiresInSeconds"]
      .as_str()
      .and_then(|s| s.parse().ok())
      .or_else(|| player["streamingData"]["expiresInSeconds"].as_u64())
      .map_or(DEFAULT_EXPIRY, Duration::from_secs);
    let (settle, settled) = watch::channel(None);
    let shared = Arc::new(Shared {
      client,
      video_id: video_id.to_owned(),
      http,
      key,
      tracks,
      first: Mutex::new(0),
      window: Mutex::new(max_dvr),
      refresh_at: Mutex::new(Instant::now() + expiry.saturating_sub(REFRESH_BEFORE)),
      refreshing: AtomicBool::new(false),
      ended: AtomicBool::new(false),
      starting: [main, audio_index],
      served: [AtomicBool::new(false), AtomicBool::new(false)],
      held: Mutex::new(None),
      settle,
    });
    let (video_edge, audio_edge) =
      tokio::join!(read_edge(&shared, main), read_edge(&shared, audio_index));
    let edge = video_edge?;
    audio_edge?;
    let first = edge.sq.saturating_sub((max_dvr / edge.segment) as u64);
    *lock(&shared.first) = first;
    let search = tokio::spawn(find_first(shared.clone(), main, first, edge.sq));
    let server = tokio::spawn(serve(listener, shared.clone()));
    tracing::debug!(
      "live proxy: {} tracks, segments of {:.4} s, from {first} to {}",
      shared.tracks.len(),
      edge.segment,
      edge.sq
    );
    Ok(Self(Arc::new(Inner {
      shared,
      variants,
      main,
      settled,
      tasks: vec![search, server],
    })))
  }

  /// The qualities, best first, their URLs pointing at the proxy (they hold no secret).
  pub fn variants(&self) -> &[Variant] {
    &self.0.variants
  }

  /// The quality picked at the start (the best up to the height asked for).
  pub fn main(&self) -> &Variant {
    &self.0.variants[self.0.main]
  }

  /// Seconds from the first segment listed to the live edge now: where the edge is in a new
  /// mpv's timeline.
  pub fn length(&self) -> f64 {
    let shared = &self.0.shared;
    let Some(edge) = *lock(&shared.tracks[self.0.main].edge) else {
      return 0.0;
    };
    let (first, last) = shared.range(&edge, Instant::now());
    (last + 1).saturating_sub(first) as f64 * edge.segment
  }

  /// How far back the live can be watched, seconds: YouTube's DVR length until
  /// [`LiveProxy::settled`], then what is really kept.
  pub fn window(&self) -> f64 {
    *lock(&self.0.shared.window)
  }

  /// Waits for the search of the first segment kept (a few requests, in the background), and
  /// gives [`LiveProxy::window`] then.
  pub async fn settled(&self) -> f64 {
    let mut settled = self.0.settled.clone();
    match settled.wait_for(Option::is_some).await {
      Ok(window) => window.unwrap_or_else(|| self.window()),
      Err(_) => self.window(),
    }
  }
}

impl Shared {
  /// A playlist of track `track` was served: the search's result applies once mpv has read
  /// both tracks it started on.
  fn served(&self, track: usize) {
    for (starting, served) in self.starting.iter().zip(&self.served) {
      if *starting == track {
        served.store(true, Ordering::Relaxed);
      }
    }
    self.apply_held();
  }

  /// The search's result, unless mpv has not read its first playlists yet.
  fn apply_held(&self) {
    if !self.served.iter().all(|s| s.load(Ordering::Relaxed)) {
      return;
    }
    let Some((first, window)) = lock(&self.held).take() else {
      return;
    };
    *lock(&self.first) = first;
    *lock(&self.window) = window;
    tracing::debug!("live proxy: segments kept from {first}, {window:.0} s back");
    self.settle.send_replace(Some(window));
  }

  /// The segments listed for an edge read as `edge`: from the first kept (sliding once the
  /// window is full) to the newest one read, or further when the estimate is a margin past it.
  fn range(&self, edge: &Edge, at: Instant) -> (u64, u64) {
    let last = if self.ended.load(Ordering::Relaxed) {
      edge.sq
    } else {
      edge.now(at).saturating_sub(EDGE_MARGIN).max(edge.sq)
    };
    let kept = (*lock(&self.window) / edge.segment) as u64;
    let first = (*lock(&self.first)).max(last.saturating_sub(kept));
    (first.min(last), last)
  }
}

/// The video formats to offer, best first with one per height (AV1 over AVC at equal height),
/// and the best audio one.
fn pick(formats: Vec<LiveFormat>) -> Option<(Vec<LiveFormat>, LiveFormat)> {
  let (audios, mut videos): (Vec<_>, Vec<_>) = formats.into_iter().partition(LiveFormat::audio);
  let audio = audios.into_iter().max_by_key(|f| f.bitrate)?;
  let rank = |f: &LiveFormat| {
    (
      short_side(f),
      f.fps.unwrap_or(0),
      f.codecs.starts_with("av01"),
      f.bitrate,
    )
  };
  videos.sort_by_key(|f| std::cmp::Reverse(rank(f)));
  videos.dedup_by_key(|f| short_side(f));
  (!videos.is_empty()).then_some((videos, audio))
}

/// A video's height, or its width when vertical (as [`Variant::height`]).
fn short_side(format: &LiveFormat) -> u32 {
  match (format.width, format.height) {
    (0, h) | (h, 0) => h,
    (w, h) => w.min(h),
  }
}

/// Reads a track's edge (`HEAD` on its URL), keeps it, and asks for a refresh when it has not
/// moved for [`STALLED_AFTER`] (the live may have ended).
async fn read_edge(shared: &Shared, track: usize) -> Result<Edge> {
  let url = lock(&shared.tracks[track].url).clone();
  let response = shared.http.head(&url).send().await?;
  if !response.status().is_success() {
    return Err(Error::Status {
      status: response.status().as_u16(),
      endpoint: "live head",
    });
  }
  let header =
    |name: &str| -> Option<u64> { response.headers().get(name)?.to_str().ok()?.parse().ok() };
  let sq = header("x-head-seqnum").ok_or_else(|| Error::Parse {
    endpoint: "live head",
    detail: "no x-head-seqnum".into(),
  })?;
  let announced = shared.tracks[track].format.segment;
  let segment = match header("x-head-time-millis") {
    Some(millis) if sq >= MEASURE_FROM => measured(millis, sq, announced),
    _ => announced,
  };
  let now = Instant::now();
  let mut slot = lock(&shared.tracks[track].edge);
  let mut moved_at = match *slot {
    Some(old) if old.sq >= sq => old.moved_at,
    _ => now,
  };
  // Stalled: whether the live has ended, asked once per stalled minute.
  if now.duration_since(moved_at) > STALLED_AFTER {
    *lock(&shared.refresh_at) = now;
    moved_at = now;
  }
  let edge = Edge {
    sq,
    segment,
    read_at: now,
    moved_at,
  };
  *slot = Some(edge);
  Ok(edge)
}

/// A segment's length from the edge's time and number, unless it is far from the announced one
/// (a live whose numbers do not start at its first second).
fn measured(millis: u64, sq: u64, announced: f64) -> f64 {
  let average = millis as f64 / 1000.0 / sq as f64;
  if (average - announced).abs() <= announced * 0.05 {
    average
  } else {
    announced
  }
}

/// Whether segment `sq` of a track is still served, from its first byte: YouTube answers 404 for
/// one gone, or 200 with an empty body (a `HEAD` answers 200 either way).
async fn served(shared: &Shared, track: usize, sq: u64) -> Option<bool> {
  let url = lock(&shared.tracks[track].url).clone();
  let response = shared
    .http
    .get(format!("{url}&sq={sq}"))
    .header(reqwest::header::RANGE, "bytes=0-0")
    .send()
    .await
    .ok()?;
  let status = response.status();
  if status.as_u16() == 404 {
    return Some(false);
  }
  if !status.is_success() {
    return None;
  }
  let body = response.bytes().await.ok()?;
  Some(!body.is_empty())
}

/// The first segment kept, from the DVR length's `first` (served on most lives) or by halving
/// the gap to the edge; the playlists start there and the window shrinks to it.
async fn find_first(shared: Arc<Shared>, track: usize, first: u64, edge: u64) {
  match first_served(first, edge, |sq| served(&shared, track, sq)).await {
    None => {
      let window = *lock(&shared.window);
      shared.settle.send_replace(Some(window));
    }
    // Held until mpv has read its first playlists ([`Shared::apply_held`]).
    Some(found) => {
      let found = (found + FIRST_MARGIN).min(edge);
      let segment = lock(&shared.tracks[track].edge).map_or(5.0, |e| e.segment);
      *lock(&shared.held) = Some((found, (edge - found) as f64 * segment));
      shared.apply_held();
    }
  }
}

/// The first segment served between `first` and `edge` (served), by halving; `None` when `first`
/// is (or its answer is unknown: nothing changes then). An unknown answer in the search counts
/// as gone, which only keeps less.
async fn first_served<F, Fut>(first: u64, edge: u64, served: F) -> Option<u64>
where
  F: Fn(u64) -> Fut,
  Fut: std::future::Future<Output = Option<bool>>,
{
  if served(first).await != Some(false) {
    return None;
  }
  let (mut low, mut high) = (first, edge);
  while high - low > 1 {
    let middle = low + (high - low) / 2;
    if served(middle).await == Some(true) {
      high = middle;
    } else {
      low = middle;
    }
  }
  Some(high)
}

/// Resolves the live again (VISIONOS) for fresh signed URLs, or finds that it has ended.
async fn refresh(shared: Arc<Shared>) {
  let result = shared.client.visionos_player(&shared.video_id).await;
  let next = match result {
    Ok(player) if player["videoDetails"]["isLive"] != true => {
      tracing::info!("live proxy: the live has ended");
      shared.ended.store(true, Ordering::Relaxed);
      None
    }
    Ok(player) => {
      let formats = parse_live_formats(&player);
      for track in &shared.tracks {
        if let Some(format) = formats.iter().find(|f| f.itag == track.format.itag) {
          *lock(&track.url) = format.url.clone();
        }
      }
      tracing::debug!("live proxy: URLs refreshed");
      Some(DEFAULT_EXPIRY.saturating_sub(REFRESH_BEFORE))
    }
    Err(e) => {
      tracing::debug!("live proxy: refresh failed: {e}");
      Some(REFRESH_RETRY)
    }
  };
  if let Some(after) = next {
    *lock(&shared.refresh_at) = Instant::now() + after;
  }
  shared.refreshing.store(false, Ordering::Relaxed);
}

/// A poisoned lock means a request panicked mid-update: its state is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
  mutex
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 128 random bits as hex, from the standard library's randomly seeded hasher.
fn random_key() -> String {
  (0..2)
    .map(|i| {
      let mut hasher = RandomState::new().build_hasher();
      hasher.write_u64(i);
      format!("{:016x}", hasher.finish())
    })
    .collect()
}

async fn serve(listener: TcpListener, shared: Arc<Shared>) {
  loop {
    match listener.accept().await {
      Ok((stream, _)) => {
        tokio::spawn(handle(stream, shared.clone()));
      }
      Err(e) => {
        tracing::warn!("live proxy: accept failed: {e}");
        return;
      }
    }
  }
}

/// What a path asks for.
#[derive(Debug, PartialEq, Eq)]
enum Route {
  /// `/<key>/<track>.m3u8`
  Playlist(usize),
  /// `/<key>/s/<track>/<sq>`
  Segment(usize, u64),
}

fn route(path: &str, key: &str) -> Option<Route> {
  let rest = path
    .strip_prefix('/')?
    .strip_prefix(key)?
    .strip_prefix('/')?;
  if let Some(segment) = rest.strip_prefix("s/") {
    let (track, sq) = segment.split_once('/')?;
    return Some(Route::Segment(track.parse().ok()?, sq.parse().ok()?));
  }
  Some(Route::Playlist(rest.strip_suffix(".m3u8")?.parse().ok()?))
}

/// One HTTP/1.1 request, answered then closed.
async fn handle(mut stream: TcpStream, shared: Arc<Shared>) {
  let mut request = Vec::with_capacity(1024);
  let mut buffer = [0u8; 1024];
  while !request.windows(4).any(|w| w == b"\r\n\r\n") {
    match stream.read(&mut buffer).await {
      Ok(0) | Err(_) => return,
      Ok(n) => request.extend_from_slice(&buffer[..n]),
    }
    if request.len() > 8192 {
      return;
    }
  }
  let line = String::from_utf8_lossy(&request);
  let mut parts = line.split_whitespace();
  let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
  let response = if method != "GET" && method != "HEAD" {
    reply(405, "")
  } else {
    match route(path, &shared.key) {
      Some(Route::Playlist(i)) if i < shared.tracks.len() => match playlist(&shared, i).await {
        Ok(body) => reply(200, &body),
        Err(e) => {
          tracing::debug!("live proxy: track {i}: {e}");
          reply(502, "")
        }
      },
      Some(Route::Segment(i, sq)) if i < shared.tracks.len() => {
        let url = lock(&shared.tracks[i].url).clone();
        format!(
          "HTTP/1.1 302 Found\r\nLocation: {url}&sq={sq}\r\nContent-Length: 0\r\n\
           Cache-Control: no-cache\r\nConnection: close\r\n\r\n"
        )
      }
      _ => reply(404, ""),
    }
  };
  // mpv may have closed first (seek, quality change): nothing to do then.
  let _ = stream.write_all(response.as_bytes()).await;
  let _ = stream.shutdown().await;
}

/// Track `i`'s playlist now: its edge read again when older than [`HEAD_EVERY`], and the URLs
/// refreshed in the background when they are about to expire.
async fn playlist(shared: &Arc<Shared>, i: usize) -> Result<String> {
  let now = Instant::now();
  if now >= *lock(&shared.refresh_at)
    && !shared.ended.load(Ordering::Relaxed)
    && !shared.refreshing.swap(true, Ordering::Relaxed)
  {
    tokio::spawn(refresh(shared.clone()));
  }
  let known = *lock(&shared.tracks[i].edge);
  let edge = match known {
    Some(edge)
      if shared.ended.load(Ordering::Relaxed) || now.duration_since(edge.read_at) < HEAD_EVERY =>
    {
      edge
    }
    // A failed reading keeps the estimate from the last one.
    Some(edge) => read_edge(shared, i).await.unwrap_or(edge),
    None => read_edge(shared, i).await?,
  };
  let (first, last) = shared.range(&edge, now);
  // A reading behind the previous estimate keeps the segments already listed.
  let last = shared.tracks[i]
    .listed
    .fetch_max(last, Ordering::Relaxed)
    .max(last);
  let body = render(
    i,
    first,
    last,
    edge.segment,
    shared.ended.load(Ordering::Relaxed),
  );
  shared.served(i);
  Ok(body)
}

/// A live media playlist of segments `first..=last` of track `track`, each `segment` seconds.
fn render(track: usize, first: u64, last: u64, segment: f64, ended: bool) -> String {
  use std::fmt::Write;
  let count = (last + 1).saturating_sub(first);
  let mut out = String::with_capacity(96 + count as usize * 32);
  let _ = write!(
    out,
    "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:{first}\n",
    segment.ceil() as u64
  );
  for sq in first..=last {
    let _ = write!(out, "#EXTINF:{segment:.4},\ns/{track}/{sq}\n");
  }
  if ended {
    out.push_str("#EXT-X-ENDLIST\n");
  }
  out
}

fn reply(status: u16, body: &str) -> String {
  let reason = match status {
    200 => "OK",
    404 => "Not Found",
    405 => "Method Not Allowed",
    _ => "Bad Gateway",
  };
  format!(
    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/vnd.apple.mpegurl\r\n\
     Content-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{body}",
    body.len()
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn format(
    itag: u32,
    mime: &str,
    codecs: &str,
    height: u32,
    fps: u32,
    bitrate: u64,
  ) -> LiveFormat {
    LiveFormat {
      itag,
      url: format!("https://example.invalid/videoplayback?itag={itag}"),
      mime: mime.into(),
      codecs: codecs.into(),
      width: height * 16 / 9,
      height,
      fps: (height > 0).then_some(fps),
      bitrate,
      segment: 5.0,
      max_dvr: 43200.0,
    }
  }

  #[test]
  fn paths() {
    assert_eq!(route("/k/3.m3u8", "k"), Some(Route::Playlist(3)));
    assert_eq!(route("/k/s/1/1234", "k"), Some(Route::Segment(1, 1234)));
    assert_eq!(route("/other/3.m3u8", "k"), None);
    assert_eq!(route("/k/s/1/x", "k"), None);
    assert_eq!(route("/k/x.m3u8", "k"), None);
    let (a, b) = (random_key(), random_key());
    assert_eq!(a.len(), 32);
    assert_ne!(a, b);
  }

  #[test]
  fn ladder() {
    let (videos, audio) = pick(vec![
      format(299, "video/mp4", "avc1.64002A", 1080, 60, 6_000_000),
      format(399, "video/mp4", "av01.0.09M.08", 1080, 60, 4_000_000),
      format(401, "video/mp4", "av01.0.13M.08", 2160, 60, 20_000_000),
      format(135, "video/mp4", "avc1.4D401F", 480, 30, 1_000_000),
      format(139, "audio/mp4", "mp4a.40.5", 0, 0, 48_000),
      format(140, "audio/mp4", "mp4a.40.2", 0, 0, 128_000),
    ])
    .unwrap();
    let itags: Vec<u32> = videos.iter().map(|f| f.itag).collect();
    // AV1 wins at 1080p60; one per height.
    assert_eq!(itags, [401, 399, 135]);
    assert_eq!(audio.itag, 140);
    assert!(pick(vec![format(140, "audio/mp4", "mp4a.40.2", 0, 0, 1)]).is_none());
  }

  #[test]
  fn segment_length() {
    // 29.97 fps: 4.9675 s a segment, though 5 is announced.
    assert!((measured(15_279_993, 3076, 5.0) - 4.9675).abs() < 1e-3);
    assert!((measured(6_765_676_600, 1_353_139, 5.0) - 5.0).abs() < 1e-4);
    // Numbers that do not start with the broadcast: the announced length.
    assert_eq!(measured(1_000, 3076, 5.0), 5.0);
  }

  #[test]
  fn playlist_text() {
    let text = render(2, 10, 12, 4.9675, false);
    assert_eq!(
      text,
      "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:5\n#EXT-X-MEDIA-SEQUENCE:10\n\
       #EXTINF:4.9675,\ns/2/10\n#EXTINF:4.9675,\ns/2/11\n#EXTINF:4.9675,\ns/2/12\n"
    );
    assert!(render(0, 5, 5, 2.0, true).ends_with("s/0/5\n#EXT-X-ENDLIST\n"));
  }

  #[tokio::test(flavor = "current_thread")]
  async fn first_kept() {
    // Kept from 1_300_000 (a 24/7 live that keeps less than its 12 h).
    let probe = |gone_before: u64| move |sq: u64| async move { Some(sq >= gone_before) };
    assert_eq!(
      first_served(1_344_000, 1_353_000, probe(1_346_123)).await,
      Some(1_346_123)
    );
    // `first` is served: nothing to search.
    assert_eq!(first_served(0, 1_896, probe(0)).await, None);
    // No answer for `first`: nothing changes.
    assert_eq!(first_served(0, 1_896, |_| async { None }).await, None);
  }

  #[test]
  fn edge_estimate() {
    let at = Instant::now();
    let edge = Edge {
      sq: 100,
      segment: 5.0,
      read_at: at,
      moved_at: at,
    };
    assert_eq!(edge.now(at), 100);
    assert_eq!(edge.now(at + Duration::from_secs(12)), 102);
  }
}
