//! The playlist mpv sees for a live: the main playlist's content segments, with the gaps an ad
//! break leaves filled by a backup playlist's segments (another `playerType`, without ads). Both
//! number their content by Twitch's live sequence ([`crate::ads`]), which becomes the output's
//! media sequence: it never goes back, and a segment once shown stays the same.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::ads::{MediaPlaylist, Segment};

/// Segments kept in the output, about 30 s: FFmpeg starts a live 3 segments from the end.
const WINDOW: usize = 15;

/// Where an output segment comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
  /// The main playlist.
  Main,
  /// The backup playlist, during an ad break.
  Backup,
}

#[derive(Clone, Debug)]
struct Kept {
  segment: Segment,
  origin: Origin,
}

/// What an ad break looks like from the output.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AdState {
  /// An ad break runs in the main playlist.
  pub active: bool,
  /// Its seconds so far, and its announced length.
  pub elapsed: f64,
  /// Announced length (`DURATION` of the ad range), when known.
  pub duration: Option<f64>,
  /// The backup fills it: the viewer sees no gap.
  pub covered: bool,
}

/// The output playlist of one variant, rebuilt on each reload.
#[derive(Default)]
pub struct Splicer {
  kept: BTreeMap<u64, Kept>,
  /// Discontinuities that left the window (`EXT-X-DISCONTINUITY-SEQUENCE`).
  discontinuities_dropped: u64,
  target_duration: u32,
  ended: bool,
  state: AdState,
}

impl Splicer {
  /// Takes a reload of the main playlist, and of the backup when one was fetched (the caller
  /// fetches it while [`MediaPlaylist::in_ad`]).
  pub fn update(&mut self, main: &MediaPlaylist, backup: Option<&MediaPlaylist>) {
    self.target_duration = self.target_duration.max(main.target_duration);
    self.ended = main.ended;
    self.add(main, Origin::Main);
    let in_ad = main.in_ad();
    let mut covered = false;
    if in_ad && let Some(backup) = backup {
      // Only what comes after the newest kept segment: a shown segment never changes.
      let newest = self.kept.keys().next_back().copied();
      for segment in &backup.segments {
        if let Some(n) = segment.live_sequence
          && newest.is_none_or(|newest| n > newest)
        {
          self.insert(n, segment, Origin::Backup);
        }
      }
      covered = !backup.in_ad() && backup.segments.iter().any(|s| s.live_sequence.is_some());
      self.target_duration = self.target_duration.max(backup.target_duration);
    }
    self.state = AdState {
      active: in_ad,
      elapsed: if in_ad { main.ad_elapsed() } else { 0.0 },
      duration: in_ad.then_some(main.ad_duration).flatten(),
      covered: in_ad && covered,
    };
    self.trim();
  }

  fn add(&mut self, playlist: &MediaPlaylist, origin: Origin) {
    for segment in &playlist.segments {
      if let Some(n) = segment.live_sequence {
        self.insert(n, segment, origin);
      }
    }
  }

  fn insert(&mut self, n: u64, segment: &Segment, origin: Origin) {
    // A sequence number below the window's start is old news: it already left.
    if self.kept.len() >= WINDOW && self.kept.keys().next().is_some_and(|&first| n < first) {
      return;
    }
    self.kept.entry(n).or_insert_with(|| Kept {
      segment: segment.clone(),
      origin,
    });
  }

  fn trim(&mut self) {
    while self.kept.len() > WINDOW {
      let Some((&first, _)) = self.kept.iter().next() else {
        break;
      };
      self.kept.remove(&first);
      // The discontinuity before the new first segment leaves with the removed one.
      if let Some((&second, kept)) = self.kept.iter().next()
        && self.discontinuity_before(second, kept)
      {
        self.discontinuities_dropped += 1;
      }
    }
  }

  /// Whether the output marks a discontinuity before segment `n`: a change of source, a gap, or
  /// one in the source playlist.
  fn discontinuity_before(&self, n: u64, kept: &Kept) -> bool {
    match self.kept.range(..n).next_back() {
      None => false,
      Some((&previous, before)) => {
        before.origin != kept.origin
          || previous + 1 != n
          || before.segment.map != kept.segment.map
          || kept.segment.discontinuity
      }
    }
  }

  /// The ad break as of the last update.
  pub fn ad_state(&self) -> AdState {
    self.state
  }

  /// Number of segments in the output.
  pub fn len(&self) -> usize {
    self.kept.len()
  }

  /// Whether the output has no segment yet (an ad break with no backup).
  pub fn is_empty(&self) -> bool {
    self.kept.is_empty()
  }

  /// The origin of each output segment, in order (tests, statistics).
  pub fn origins(&self) -> Vec<(u64, Origin)> {
    self.kept.iter().map(|(&n, k)| (n, k.origin)).collect()
  }

  /// The playlist text for mpv. Segment URLs are the CDN's, absolute.
  pub fn render(&self) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:6\n");
    let first = self.kept.keys().next().copied().unwrap_or(0);
    let _ = writeln!(out, "#EXT-X-TARGETDURATION:{}", self.target_duration.max(1));
    let _ = writeln!(out, "#EXT-X-MEDIA-SEQUENCE:{first}");
    let _ = writeln!(
      out,
      "#EXT-X-DISCONTINUITY-SEQUENCE:{}",
      self.discontinuities_dropped
    );
    let mut map: Option<&str> = None;
    for (&n, kept) in &self.kept {
      if self.discontinuity_before(n, kept) {
        out.push_str("#EXT-X-DISCONTINUITY\n");
        map = None;
      }
      if let Some(uri) = kept.segment.map.as_deref()
        && map != Some(uri)
      {
        let _ = writeln!(out, "#EXT-X-MAP:URI=\"{uri}\"");
        map = Some(uri);
      }
      if let Some(date) = &kept.segment.date_line {
        out.push_str(date);
        out.push('\n');
      }
      let _ = writeln!(out, "#EXTINF:{:.3},", kept.segment.duration);
      out.push_str(&kept.segment.url);
      out.push('\n');
    }
    if self.ended {
      out.push_str("#EXT-X-ENDLIST\n");
    }
    out
  }
}
