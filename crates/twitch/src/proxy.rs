//! The local playlist proxy between Twitch and mpv (ADR 0010, docs/twitch-plan.md step 3).
//!
//! mpv plays `http://127.0.0.1:<port>/<key>/<variant>.m3u8`. On each reload the proxy fetches the
//! variant's media playlist from Twitch and answers:
//! - for a live, the [`Splicer`]'s playlist: content only, an ad break filled with the backup
//!   playlist (another `playerType`) when there is one;
//! - for a VOD, Twitch's playlist with absolute URIs.
//!
//! Only playlists go through it: segments are fetched by mpv from the CDN directly. The signed
//! upstream URLs stay in the proxy's memory; `key` is random, so another local program cannot guess
//! the paths. The server stops when the [`Proxy`] is dropped.

use std::collections::VecDeque;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::ads;
use crate::client::{Client, Master};
use crate::error::{Error, Result};
use crate::gql::Target;
use crate::splice::{AdState, Splicer};
use crate::usher::Variant;

/// One variant as the proxy serves it.
struct Upstream {
  main: String,
  backup: Option<String>,
  splicer: Mutex<Splicer>,
}

struct Shared {
  client: Client,
  key: String,
  live: bool,
  upstreams: Vec<Upstream>,
  /// The ad state of the variant served last (the one playing).
  ad: Mutex<AdState>,
  /// Where the live playlist served last ends ([`Splicer::edge`]).
  edge: Arc<Mutex<EdgeState>>,
}

/// How long the samples of Twitch's production lag are kept ([`EdgeState`]).
const LAG_WINDOW: Duration = Duration::from_secs(30);

/// The end of the live playlist served last, and how late Twitch lists its segments.
#[derive(Debug, Default)]
struct EdgeState {
  served: Option<Served>,
  /// When each reload was served, and how long after its newest dated segment's end (our
  /// clock minus Twitch's, milliseconds): the reload just after a segment appears comes
  /// closest to the delay from capture to the CDN, plus the clocks' skew.
  samples: VecDeque<(Instant, i64)>,
}

/// [`Splicer::edge`] of a served playlist, with the production lag then.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Served {
  /// The newest dated segment's end, milliseconds since the epoch (Twitch's clock).
  dated: i64,
  /// Seconds listed after it.
  after: f64,
  /// The smallest lag sampled over [`LAG_WINDOW`], milliseconds.
  lag: i64,
}

impl EdgeState {
  /// A reload served at `at` (`now_ms` on our clock), ending as `edge` says.
  fn record(&mut self, (dated, after): (i64, f64), at: Instant, now_ms: i64) {
    self.samples.push_back((at, now_ms - dated));
    while self
      .samples
      .front()
      .is_some_and(|(t, _)| at.saturating_duration_since(*t) > LAG_WINDOW)
    {
      self.samples.pop_front();
    }
    let lag = self.samples.iter().map(|(_, l)| *l).min().unwrap_or(0);
    self.served = Some(Served { dated, after, lag });
  }
}

/// A running proxy for one playback.
pub struct Proxy {
  variants: Vec<Variant>,
  shared: Arc<Shared>,
  server: JoinHandle<()>,
}

impl Drop for Proxy {
  fn drop(&mut self) {
    self.server.abort();
  }
}

impl std::fmt::Debug for Proxy {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Proxy")
      .field("variants", &self.variants)
      .finish_non_exhaustive()
  }
}

impl Proxy {
  /// Starts a proxy for `main`'s variants, with `backup` (a master of the same channel from an
  /// ad-free `playerType`) to fill ad breaks; a live's playlists end with Twitch's prefetch
  /// segments when `low_latency` ([`Splicer`]). Call it on a tokio runtime.
  pub async fn start(
    client: Client,
    target: &Target,
    main: Master,
    backup: Option<Master>,
    low_latency: bool,
  ) -> Result<Self> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
      .await
      .map_err(|e| Error::Proxy(e.to_string()))?;
    let port = listener
      .local_addr()
      .map_err(|e| Error::Proxy(e.to_string()))?
      .port();
    let key = random_key();
    let live = matches!(target, Target::Live(_));
    let mut variants = Vec::with_capacity(main.variants.len());
    let mut upstreams = Vec::with_capacity(main.variants.len());
    for (i, variant) in main.variants.into_iter().enumerate() {
      let backup = backup
        .as_ref()
        .and_then(|b| backup_for(&variant, &b.variants))
        .map(|v| v.url.clone());
      upstreams.push(Upstream {
        main: variant.url.clone(),
        backup,
        splicer: Mutex::new(Splicer::new(low_latency)),
      });
      variants.push(Variant {
        url: format!("http://127.0.0.1:{port}/{key}/{i}.m3u8"),
        ..variant
      });
    }
    let shared = Arc::new(Shared {
      client,
      key,
      live,
      upstreams,
      ad: Mutex::default(),
      edge: Arc::default(),
    });
    let server = tokio::spawn(serve(listener, shared.clone()));
    Ok(Self {
      variants,
      shared,
      server,
    })
  }

  /// The variants, best first, their `url` pointing at the proxy (it holds no secret).
  pub fn variants(&self) -> &[Variant] {
    &self.variants
  }

  /// The ad break of the variant playing, as of its last reload.
  pub fn ad_state(&self) -> AdState {
    *lock(&self.shared.ad)
  }

  /// The broadcast's clock at the end of the live playlist served last, for the delay to the
  /// streamer; it holds no secret and outlives the proxy harmlessly.
  pub fn edge_clock(&self) -> EdgeClock {
    EdgeClock(self.shared.edge.clone())
  }
}

/// Where a live's playlist, as mpv last read it, ends on the broadcast's clock
/// ([`Proxy::edge_clock`]).
#[derive(Clone, Debug)]
pub struct EdgeClock(Arc<Mutex<EdgeState>>);

impl EdgeClock {
  /// The capture time of the playlist's end (Twitch's `PROGRAM-DATE-TIME`, so Twitch's clock,
  /// taken as ours): the newest dated segment's end, plus the segments listed after it as far
  /// as they can be downloaded, which is Twitch's production lag behind now (sampled against
  /// our clock, so a skew between the two does not move that bound). `None` before a playlist
  /// with a date was served (a VOD's).
  pub fn time(&self) -> Option<SystemTime> {
    let served = lock(&self.0).served?;
    edge_time(served, epoch_millis_now())
  }
}

/// [`EdgeClock::time`] at `now_ms`, milliseconds since the epoch on our clock.
fn edge_time(served: Served, now_ms: i64) -> Option<SystemTime> {
  // What exists of the segments after the dated one: up to now, less the production lag.
  let room = (now_ms - served.lag - served.dated).max(0);
  let after = ((served.after.max(0.0) * 1000.0).round() as i64).min(room);
  let end = served.dated + after;
  Some(SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(end).ok()?))
}

/// Now, milliseconds since the epoch.
fn epoch_millis_now() -> i64 {
  SystemTime::now()
    .duration_since(SystemTime::UNIX_EPOCH)
    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// The backup variant standing in for `main`: the same id, else the best landscape one not taller.
fn backup_for<'a>(main: &Variant, backups: &'a [Variant]) -> Option<&'a Variant> {
  backups.iter().find(|b| b.id == main.id).or_else(|| {
    backups
      .iter()
      .filter(|b| !b.portrait && !b.audio_only() && b.height <= main.height)
      .max_by_key(|b| (b.height, b.bandwidth))
  })
}

/// A poisoned lock means a reload panicked mid-update: its state is still usable for the next one.
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
        tracing::warn!("Twitch proxy: accept failed: {e}");
        return;
      }
    }
  }
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
  let response = if method != "GET" {
    reply(405, "")
  } else {
    match index(path, &shared.key) {
      Some(i) if i < shared.upstreams.len() => match playlist(&shared, i).await {
        Ok(body) => reply(200, &body),
        Err(e) => {
          tracing::debug!("Twitch proxy: variant {i}: {e}");
          reply(502, "")
        }
      },
      _ => reply(404, ""),
    }
  };
  // mpv may have closed first (seek, quality change): nothing to do then.
  let _ = stream.write_all(response.as_bytes()).await;
  let _ = stream.shutdown().await;
}

/// The variant index of `/<key>/<index>.m3u8`.
fn index(path: &str, key: &str) -> Option<usize> {
  let rest = path
    .strip_prefix('/')?
    .strip_prefix(key)?
    .strip_prefix('/')?;
  rest.strip_suffix(".m3u8")?.parse().ok()
}

async fn playlist(shared: &Shared, i: usize) -> Result<String> {
  let upstream = &shared.upstreams[i];
  let text = shared.client.media_playlist(&upstream.main).await?;
  if !shared.live {
    return Ok(ads::absolutize(&text, &upstream.main));
  }
  let main = ads::parse(&text, &upstream.main);
  let backup = match (&upstream.backup, main.in_ad()) {
    (Some(url), true) => match shared.client.media_playlist(url).await {
      Ok(text) => Some(ads::parse(&text, url)),
      Err(e) => {
        tracing::debug!("Twitch proxy: backup of variant {i}: {e}");
        None
      }
    },
    _ => None,
  };
  let (body, state, edge) = {
    let mut splicer = lock(&upstream.splicer);
    splicer.update(&main, backup.as_ref());
    (splicer.render(), splicer.ad_state(), splicer.edge())
  };
  if let Some(edge) = edge {
    lock(&shared.edge).record(edge, Instant::now(), epoch_millis_now());
  }
  let previous = std::mem::replace(&mut *lock(&shared.ad), state);
  if previous.active != state.active {
    tracing::info!(
      "Twitch ad break {}{}",
      if state.active { "started" } else { "ended" },
      if state.active && state.covered {
        ", covered by the backup stream"
      } else {
        ""
      }
    );
  }
  Ok(body)
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
  use std::time::{Duration, Instant, SystemTime};

  use super::{EdgeState, Served, edge_time, index, random_key};

  #[test]
  fn edge_counts_what_follows_the_date_as_far_as_it_exists() {
    let at = |ms: u64| SystemTime::UNIX_EPOCH + Duration::from_millis(ms);
    // Two prefetch segments of 2 s after a segment ending at 10 s, Twitch listing 1 s late.
    let served = Served {
      dated: 10_000,
      after: 4.0,
      lag: 1_000,
    };
    assert_eq!(edge_time(served, 11_000), Some(at(10_000)));
    assert_eq!(edge_time(served, 13_000), Some(at(12_000)));
    assert_eq!(edge_time(served, 20_000), Some(at(14_000)));
    // Without prefetch: the date as is.
    let plain = Served {
      after: 0.0,
      ..served
    };
    assert_eq!(edge_time(plain, 20_000), Some(at(10_000)));
    // Our clock behind Twitch's: the lag sampled is negative, the bound follows.
    let behind = Served {
      lag: -2_000,
      ..served
    };
    assert_eq!(edge_time(behind, 9_000), Some(at(11_000)));
    let broken = Served { dated: -1, ..plain };
    assert_eq!(edge_time(broken, 9_000), None);
  }

  #[test]
  fn lag_is_the_smallest_sample_of_the_window() {
    let start = Instant::now();
    let mut state = EdgeState::default();
    state.record((10_000, 4.0), start, 12_500);
    state.record((12_000, 4.0), start + Duration::from_secs(2), 13_200);
    state.record((12_000, 4.0), start + Duration::from_secs(4), 15_200);
    assert_eq!(state.served.map(|s| s.lag), Some(1_200));
    // 40 s on, the old samples have left.
    state.record((52_000, 4.0), start + Duration::from_secs(40), 54_000);
    assert_eq!(state.served.map(|s| s.lag), Some(2_000));
  }

  #[test]
  fn paths() {
    assert_eq!(index("/k/3.m3u8", "k"), Some(3));
    assert_eq!(index("/other/3.m3u8", "k"), None);
    assert_eq!(index("/k/x.m3u8", "k"), None);
    let (a, b) = (random_key(), random_key());
    assert_eq!(a.len(), 32);
    assert_ne!(a, b);
  }
}
