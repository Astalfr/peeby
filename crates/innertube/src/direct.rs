//! The local proxy of a [`DirectOffer`]'s formats. googlevideo throttles a request for the rest
//! of a file (what mpv sends) to about twice the bitrate (21 Mbit/s for a 1440p60 VP9, measured
//! 2026-11-16): mpv then waits 15–20 s at the start and after a seek. A 10 MB `&range=` request,
//! as yt-dlp makes them (`http_chunk_size`), comes at 200–430 Mbit/s. The proxy serves mpv's
//! ranges as such chunks, one after the other; the signed URLs stay in it.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use crate::error::{Error, Result};
use crate::models::DirectOffer;

/// Bytes per upstream request: yt-dlp's chunk, which googlevideo serves at full speed.
const CHUNK: u64 = 10 * 1024 * 1024;
/// One chunk, at worst on a slow connection.
const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);

/// A running proxy for one offer. Cheap to clone; the last clone dropped stops it.
#[derive(Clone)]
pub struct DirectProxy(Arc<Inner>);

struct Inner {
  /// Local URLs of the offer's videos, in its order, and of its audio.
  videos: Vec<String>,
  audio: String,
  task: JoinHandle<()>,
}

impl Drop for Inner {
  fn drop(&mut self) {
    self.task.abort();
  }
}

impl std::fmt::Debug for DirectProxy {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "DirectProxy({} videos)", self.0.videos.len())
  }
}

/// A format the proxy serves.
struct Format {
  url: String,
  length: u64,
}

struct Shared {
  key: String,
  /// The videos, then the audio last.
  formats: Vec<Format>,
  http: reqwest::Client,
}

impl DirectProxy {
  /// Serves `offer`'s formats on a local port. Fails when one has no `contentLength`.
  pub async fn start(offer: &DirectOffer) -> Result<Self> {
    let length = |l: Option<u64>| l.ok_or_else(|| Error::Proxy("a format has no length".into()));
    let mut formats = Vec::with_capacity(offer.videos.len() + 1);
    for video in &offer.videos {
      formats.push(Format {
        url: video.url.clone(),
        length: length(video.length)?,
      });
    }
    formats.push(Format {
      url: offer.audio.url.clone(),
      length: length(offer.audio.length)?,
    });
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
      .await
      .map_err(|e| Error::Proxy(e.to_string()))?;
    let port = listener
      .local_addr()
      .map_err(|e| Error::Proxy(e.to_string()))?
      .port();
    let key = crate::live::random_key();
    let http = reqwest::Client::builder()
      .user_agent(offer.user_agent)
      .timeout(CHUNK_TIMEOUT)
      .build()?;
    let base = format!("http://127.0.0.1:{port}/{key}");
    let count = formats.len();
    let videos = (0..count - 1).map(|i| format!("{base}/{i}")).collect();
    let audio = format!("{base}/{}", count - 1);
    let shared = Arc::new(Shared { key, formats, http });
    let task = tokio::spawn(serve(listener, shared));
    Ok(Self(Arc::new(Inner {
      videos,
      audio,
      task,
    })))
  }

  /// The local URL of the offer's video `index`.
  pub fn video_url(&self, index: usize) -> Option<&str> {
    self.0.videos.get(index).map(String::as_str)
  }

  /// The local URL of the offer's audio.
  pub fn audio_url(&self) -> &str {
    &self.0.audio
  }
}

async fn serve(listener: TcpListener, shared: Arc<Shared>) {
  loop {
    match listener.accept().await {
      Ok((stream, _)) => {
        tokio::spawn(handle(stream, shared.clone()));
      }
      Err(e) => {
        tracing::warn!("direct proxy: accept failed: {e}");
        return;
      }
    }
  }
}

/// One HTTP/1.1 request: the asked range of a format, then the connection closes.
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
  let text = String::from_utf8_lossy(&request);
  let mut parts = text.split_whitespace();
  let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
  let format = path
    .strip_prefix('/')
    .and_then(|p| p.strip_prefix(shared.key.as_str()))
    .and_then(|p| p.strip_prefix('/'))
    .and_then(|i| i.parse::<usize>().ok())
    .and_then(|i| shared.formats.get(i));
  let Some(format) = format.filter(|_| method == "GET" || method == "HEAD") else {
    let _ = stream
      .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
      .await;
    return;
  };
  let range = text
    .lines()
    .find_map(|line| {
      let (name, value) = line.split_once(':')?;
      name.eq_ignore_ascii_case("range").then(|| value.trim())
    })
    .and_then(|value| parse_range(value, format.length));
  let (first, last) = range.unwrap_or((0, format.length.saturating_sub(1)));
  if first >= format.length || last < first {
    let header = format!(
      "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\n\
       Connection: close\r\n\r\n",
      format.length
    );
    let _ = stream.write_all(header.as_bytes()).await;
    return;
  }
  let status = if range.is_some() {
    format!(
      "206 Partial Content\r\nContent-Range: bytes {first}-{last}/{}",
      format.length
    )
  } else {
    "200 OK".to_owned()
  };
  let header = format!(
    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\
     Content-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
    last - first + 1
  );
  if stream.write_all(header.as_bytes()).await.is_err() || method == "HEAD" {
    return;
  }
  // mpv closes the connection on a seek or a quality change: the next write fails, it stops.
  let mut at = first;
  while at <= last {
    let end = (at + CHUNK - 1).min(last);
    match chunk(&shared.http, &format.url, at, end, &mut stream).await {
      Ok(()) => at = end + 1,
      Err(Failure::Client) => return,
      Err(Failure::Upstream(e)) => {
        tracing::debug!("direct proxy: bytes {at}-{end}: {e}");
        return;
      }
    }
  }
  let _ = stream.shutdown().await;
}

enum Failure {
  /// mpv went away.
  Client,
  /// YouTube did not serve the chunk (the reason only: never the URL).
  Upstream(String),
}

/// Bytes `first..=last` of `url` from YouTube, written to `stream` as they come.
async fn chunk(
  http: &reqwest::Client,
  url: &str,
  first: u64,
  last: u64,
  stream: &mut TcpStream,
) -> std::result::Result<(), Failure> {
  let mut response = http
    .get(format!("{url}&range={first}-{last}"))
    .send()
    .await
    .map_err(|e| Failure::Upstream(e.without_url().to_string()))?;
  if !response.status().is_success() {
    return Err(Failure::Upstream(format!(
      "HTTP {}",
      response.status().as_u16()
    )));
  }
  let mut sent = 0;
  while let Some(bytes) = response
    .chunk()
    .await
    .map_err(|e| Failure::Upstream(e.without_url().to_string()))?
  {
    sent += bytes.len() as u64;
    stream
      .write_all(&bytes)
      .await
      .map_err(|_| Failure::Client)?;
  }
  if sent != last - first + 1 {
    return Err(Failure::Upstream(format!(
      "{sent} bytes of {}",
      last - first + 1
    )));
  }
  Ok(())
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` of a file of `length` bytes, as inclusive bounds.
fn parse_range(value: &str, length: u64) -> Option<(u64, u64)> {
  let spec = value.strip_prefix("bytes=")?.split(',').next()?.trim();
  let (first, last) = spec.split_once('-')?;
  let end = length.checked_sub(1)?;
  match (first.trim(), last.trim()) {
    ("", suffix) => {
      let n: u64 = suffix.parse().ok()?;
      Some((length.saturating_sub(n), end))
    }
    (first, "") => Some((first.parse().ok()?, end)),
    (first, last) => Some((first.parse().ok()?, last.parse::<u64>().ok()?.min(end))),
  }
}

#[cfg(test)]
mod tests {
  use super::parse_range;

  #[test]
  fn ranges_as_mpv_and_ffmpeg_send_them() {
    assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
    assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
    // Past the end: up to the last byte.
    assert_eq!(parse_range("bytes=90-200", 100), Some((90, 99)));
    assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
    assert_eq!(parse_range("items=0-1", 100), None);
    assert_eq!(parse_range("bytes=0-", 0), None);
  }
}
