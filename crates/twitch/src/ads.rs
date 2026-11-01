//! Reading Twitch media playlists and telling ad segments from content (Streamlink's rules,
//! docs/twitch.md "Checked live"). A segment is an ad when its `#EXTINF` title contains `Amazon`,
//! or when its `#EXT-X-PROGRAM-DATE-TIME` falls inside an `EXT-X-DATERANGE` of class
//! `twitch-stitched-ad` (or with an id starting with `stitched-ad-`).
//!
//! Content segments are numbered by Twitch's live sequence (`#EXT-X-TWITCH-LIVE-SEQUENCE`, in the
//! header of a plain playlist and after the discontinuity that ends a pre-roll): the same number is
//! the same moment of the stream in every playlist of the channel, which lines up the main and the
//! backup playlists ([`crate::splice`]).

use crate::time::epoch_millis;
use crate::usher::attribute;

/// A segment of a media playlist.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
  /// Absolute URL.
  pub url: String,
  /// `#EXTINF` duration, seconds.
  pub duration: f64,
  /// The `#EXTINF` line, title included.
  pub extinf: String,
  /// The `#EXT-X-PROGRAM-DATE-TIME` line, if any.
  pub date_line: Option<String>,
  /// The init segment (`EXT-X-MAP` URI, absolute) in effect.
  pub map: Option<String>,
  /// Preceded by `#EXT-X-DISCONTINUITY`.
  pub discontinuity: bool,
  /// An ad.
  pub ad: bool,
  /// Twitch's live sequence of a content segment.
  pub live_sequence: Option<u64>,
}

/// A media playlist, read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaPlaylist {
  /// `#EXT-X-TARGETDURATION`.
  pub target_duration: u32,
  /// `#EXT-X-MEDIA-SEQUENCE`.
  pub media_sequence: u64,
  /// `#EXT-X-ENDLIST`: a VOD, or a live that ended.
  pub ended: bool,
  /// Segments in order.
  pub segments: Vec<Segment>,
  /// `#EXT-X-TWITCH-PREFETCH` URLs (segments announced ahead, for low latency).
  pub prefetch: Vec<String>,
  /// Length of the latest stitched ad range, seconds (its `DURATION`).
  pub ad_duration: Option<f64>,
}

impl MediaPlaylist {
  /// Whether the newest segment is an ad: an ad break is running.
  pub fn in_ad(&self) -> bool {
    self.segments.last().is_some_and(|s| s.ad)
  }

  /// Seconds of ad since the last content segment, while [`MediaPlaylist::in_ad`].
  pub fn ad_elapsed(&self) -> f64 {
    self
      .segments
      .iter()
      .rev()
      .take_while(|s| s.ad)
      .map(|s| s.duration)
      .sum()
  }
}

/// An ad range of the playlist, in milliseconds since the epoch.
struct AdRange {
  start: i64,
  end: i64,
}

/// Reads a media playlist fetched from `base` (relative URIs, as in VOD playlists, are resolved
/// against it).
pub fn parse(text: &str, base: &str) -> MediaPlaylist {
  let mut playlist = MediaPlaylist::default();
  let mut ranges = Vec::new();
  let mut header_live = None;
  let mut live: Option<u64> = None;
  let mut map = None;
  let mut pending = Pending::default();

  for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
    if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
      playlist.target_duration = value.parse().unwrap_or(0);
    } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
      playlist.media_sequence = value.parse().unwrap_or(0);
    } else if let Some(value) = line.strip_prefix("#EXT-X-TWITCH-LIVE-SEQUENCE:") {
      let value = value.parse().ok();
      if playlist.segments.is_empty() && !pending.started() {
        header_live = value;
      }
      live = value;
    } else if line == "#EXT-X-ENDLIST" {
      playlist.ended = true;
    } else if line == "#EXT-X-DISCONTINUITY" {
      pending.discontinuity = true;
    } else if let Some(attrs) = line.strip_prefix("#EXT-X-MAP:") {
      map = attribute(attrs, "URI").map(|uri| absolute(uri, base));
    } else if let Some(attrs) = line.strip_prefix("#EXT-X-DATERANGE:") {
      let class = attribute(attrs, "CLASS");
      let id = attribute(attrs, "ID").unwrap_or_default();
      if class == Some("twitch-stitched-ad") || id.starts_with("stitched-ad-") {
        let duration = attribute(attrs, "DURATION").and_then(|d| d.parse::<f64>().ok());
        if let Some(start) = attribute(attrs, "START-DATE").and_then(epoch_millis) {
          let length = duration.unwrap_or(0.0);
          ranges.push(AdRange {
            start,
            end: start + (length * 1000.0).round() as i64,
          });
        }
        playlist.ad_duration = duration.or(playlist.ad_duration);
      }
    } else if line.starts_with("#EXT-X-PROGRAM-DATE-TIME:") {
      pending.date_line = Some(line.to_owned());
    } else if let Some(value) = line.strip_prefix("#EXTINF:") {
      pending.extinf = Some(line.to_owned());
      pending.duration = value
        .split(',')
        .next()
        .and_then(|d| d.parse().ok())
        .unwrap_or(0.0);
    } else if let Some(url) = line.strip_prefix("#EXT-X-TWITCH-PREFETCH:") {
      playlist.prefetch.push(absolute(url, base));
    } else if !line.starts_with('#') {
      let extinf = pending.extinf.take().unwrap_or_default();
      let title = extinf.split_once(',').map_or("", |(_, title)| title);
      let date = pending
        .date_line
        .as_deref()
        .and_then(|l| l.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:"))
        .and_then(epoch_millis);
      let in_range = date.is_some_and(|d| ranges.iter().any(|r| d >= r.start && d < r.end));
      let ad = title.contains("Amazon") || in_range;
      let live_sequence = if ad {
        None
      } else {
        let sequence = live;
        live = live.map(|n| n + 1);
        sequence
      };
      playlist.segments.push(Segment {
        url: absolute(line, base),
        duration: pending.duration,
        extinf,
        date_line: pending.date_line.take(),
        map: map.clone(),
        discontinuity: std::mem::take(&mut pending.discontinuity),
        ad,
        live_sequence,
      });
    }
  }

  // A playlist without any live sequence and without ads (a VOD, a plain live from another
  // source): its media sequence numbers the segments.
  let numbered = playlist.segments.iter().any(|s| s.live_sequence.is_some());
  if header_live.is_none() && !numbered && !playlist.segments.iter().any(|s| s.ad) {
    for (i, segment) in playlist.segments.iter_mut().enumerate() {
      segment.live_sequence = Some(playlist.media_sequence + i as u64);
    }
  }
  playlist
}

/// `text` with every URI (segments, `EXT-X-MAP`) made absolute against `base`, otherwise
/// unchanged: VOD playlists, which have no ads and use relative URIs.
pub fn absolutize(text: &str, base: &str) -> String {
  let mut out = String::with_capacity(text.len() + text.len() / 2);
  for line in text.lines() {
    let line = line.trim_end();
    if !line.is_empty() && !line.starts_with('#') {
      out.push_str(&absolute(line, base));
    } else if let Some(attrs) = line.strip_prefix("#EXT-X-MAP:")
      && let Some(uri) = attribute(attrs, "URI")
    {
      out.push_str(&format!("#EXT-X-MAP:URI=\"{}\"", absolute(uri, base)));
    } else {
      out.push_str(line);
    }
    out.push('\n');
  }
  out
}

/// Tags read before a segment's URI.
#[derive(Default)]
struct Pending {
  extinf: Option<String>,
  duration: f64,
  date_line: Option<String>,
  discontinuity: bool,
}

impl Pending {
  fn started(&self) -> bool {
    self.extinf.is_some() || self.date_line.is_some()
  }
}

/// `uri` made absolute against the playlist's URL `base`.
fn absolute(uri: &str, base: &str) -> String {
  if uri.starts_with("http://") || uri.starts_with("https://") {
    return uri.to_owned();
  }
  reqwest::Url::parse(base)
    .and_then(|base| base.join(uri))
    .map_or_else(|_| uri.to_owned(), |url| url.to_string())
}

#[cfg(test)]
mod tests {
  use crate::time::epoch_millis;

  #[test]
  fn dates() {
    assert_eq!(epoch_millis("1970-01-01T00:00:00.000Z"), Some(0));
    assert_eq!(
      epoch_millis("2000-03-01T00:00:01.5Z"),
      Some(951_868_801_500)
    );
    assert_eq!(
      epoch_millis("2026-11-01T20:10:51Z"),
      Some(1_793_563_851_000)
    );
    assert_eq!(epoch_millis("not a date"), None);
  }
}
