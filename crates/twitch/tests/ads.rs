//! Ad detection and splicing against real playlists (tests/fixtures/, captured 2026-11-01): a
//! pre-roll reloaded every 2 s (`live_preroll_00`…`24`, `playerType=site`) and an ad-free backup
//! (`live_backup_embed`, captured at the start of the same pre-roll).

// Test helpers outside #[test] functions: a missing fixture should panic.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use twitch::ads::{self, MediaPlaylist};
use twitch::splice::{Origin, Splicer};

const BASE: &str = "https://video-edge.example.invalid/variant/1.m3u8";

fn playlist(name: &str) -> MediaPlaylist {
  let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
  ads::parse(&std::fs::read_to_string(path).unwrap(), BASE)
}

fn preroll(n: usize) -> MediaPlaylist {
  playlist(&format!("live_preroll_{n:02}.m3u8"))
}

fn backup() -> MediaPlaylist {
  playlist("live_backup_embed.m3u8")
}

#[test]
fn backup_is_plain_live() {
  let b = backup();
  assert_eq!(b.media_sequence, 4536);
  assert_eq!(b.segments.len(), 14);
  assert!(b.segments.iter().all(|s| !s.ad));
  let sequences: Vec<u64> = b.segments.iter().filter_map(|s| s.live_sequence).collect();
  assert_eq!(sequences, (4536..4550).collect::<Vec<_>>());
  assert_eq!(b.prefetch.len(), 2);
  assert!(!b.in_ad());
  assert!(b.segments[0].map.is_some());
}

#[test]
fn preroll_start_is_all_ad() {
  let p = preroll(0);
  assert_eq!(p.segments.len(), 4);
  assert!(p.segments.iter().all(|s| s.ad && s.live_sequence.is_none()));
  assert!(p.in_ad());
  assert_eq!(p.ad_duration, Some(30.235));
  assert!((p.ad_elapsed() - 8.0).abs() < 1e-9);
}

#[test]
fn live_follows_the_preroll_with_its_sequence() {
  let p = preroll(15);
  assert_eq!(p.segments.len(), 18);
  assert_eq!(p.segments.iter().filter(|s| s.ad).count(), 16);
  let live: Vec<_> = p.segments.iter().filter(|s| !s.ad).collect();
  assert_eq!(
    live.iter().map(|s| s.live_sequence).collect::<Vec<_>>(),
    [Some(4567), Some(4568)]
  );
  assert!(live[0].discontinuity);
  assert!(!p.in_ad());
  // The last ad segment is the 0.235 s tail of the 30.235 s ad.
  assert!((p.segments[15].duration - 0.235).abs() < 1e-9);
}

#[test]
fn every_reload_agrees_on_the_ads() {
  for n in 0..25 {
    let p = preroll(n);
    for s in &p.segments {
      // Both rules (title and date range) mark the same segments.
      assert_eq!(
        s.ad,
        s.extinf.contains("Amazon"),
        "reload {n}: {}",
        s.extinf
      );
      assert_eq!(s.ad, s.live_sequence.is_none(), "reload {n}");
    }
    assert_eq!(
      p.segments.iter().filter(|s| s.ad).count(),
      (n + 4).min(16),
      "reload {n}"
    );
  }
}

#[test]
fn backup_covers_the_preroll_at_once() {
  let mut splicer = Splicer::default();
  splicer.update(&preroll(0), Some(&backup()));
  assert_eq!(splicer.len(), 14);
  assert!(splicer.origins().iter().all(|(_, o)| *o == Origin::Backup));
  let state = splicer.ad_state();
  assert!(state.active && state.covered);
  assert_eq!(state.duration, Some(30.235));
  let text = splicer.render();
  assert!(text.contains("#EXT-X-MEDIA-SEQUENCE:4536\n"));
  assert!(!text.contains("Amazon"));
  assert!(!text.contains("#EXT-X-DISCONTINUITY\n"));
}

#[test]
fn main_takes_over_after_the_preroll() {
  let mut splicer = Splicer::default();
  splicer.update(&preroll(0), Some(&backup()));
  // The backup snapshot is old (4536–4549) and the live resumes at 4567: a gap, so a
  // discontinuity. The window (2 minutes, 60 segments) keeps all 16.
  splicer.update(&preroll(15), None);
  assert_eq!(splicer.len(), 16);
  let origins = splicer.origins();
  assert_eq!(origins[0], (4536, Origin::Backup));
  assert_eq!(origins[14], (4567, Origin::Main));
  assert_eq!(origins[15], (4568, Origin::Main));
  assert!(!splicer.ad_state().active);
  let text = splicer.render();
  assert!(text.contains("#EXT-X-MEDIA-SEQUENCE:4536\n"));
  assert_eq!(text.matches("#EXT-X-DISCONTINUITY\n").count(), 1);
  // Each source's init segment is announced where it starts.
  assert_eq!(text.matches("#EXT-X-MAP:").count(), 2);
}

#[test]
fn without_backup_the_ad_is_only_skipped() {
  let mut splicer = Splicer::default();
  let mut elapsed = 0.0;
  for n in 0..15 {
    splicer.update(&preroll(n), None);
    assert!(splicer.is_empty(), "reload {n}");
    let state = splicer.ad_state();
    assert!(state.active && !state.covered);
    assert!(state.elapsed >= elapsed);
    elapsed = state.elapsed;
  }
  for n in 15..25 {
    splicer.update(&preroll(n), None);
  }
  let origins = splicer.origins();
  assert_eq!(origins.first().map(|o| o.0), Some(4567));
  assert_eq!(origins.len(), 11);
  assert!(!splicer.render().contains("Amazon"));
}

#[test]
fn plain_live_passes_through() {
  let mut splicer = Splicer::default();
  splicer.update(&backup(), None);
  let text = splicer.render();
  assert!(text.contains("#EXT-X-MEDIA-SEQUENCE:4536\n"));
  assert_eq!(text.matches("#EXTINF:").count(), 14);
  assert_eq!(text.matches("#EXT-X-MAP:").count(), 1);
  assert!(!text.contains("DISCONTINUITY\n"));
  assert!(!text.contains("#EXT-X-ENDLIST"));
}

#[test]
fn low_latency_appends_the_prefetch_segments_numbered_after_the_content() {
  let live = backup();
  let mut splicer = Splicer::new(true);
  splicer.update(&live, None);
  assert_eq!(splicer.prefetched(), 2);
  let text = splicer.render();
  let urls: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
  assert_eq!(urls.len(), 14 + 2);
  assert_eq!(&urls[14..], live.prefetch.as_slice());
  // The media sequence still starts at the first kept segment.
  assert!(text.contains("#EXT-X-MEDIA-SEQUENCE:4536\n"));
  assert!(!text.contains("#EXT-X-DISCONTINUITY\n"));

  let mut plain = Splicer::new(false);
  plain.update(&live, None);
  assert_eq!(plain.prefetched(), 0);
}

#[test]
fn no_prefetch_during_an_ad_break() {
  let mut splicer = Splicer::new(true);
  splicer.update(&preroll(0), Some(&backup()));
  assert!(splicer.ad_state().active);
  assert_eq!(splicer.prefetched(), 0);
  // After the pre-roll, the live's own prefetch comes back.
  splicer.update(&preroll(24), None);
  assert!(!splicer.ad_state().active);
  assert_eq!(splicer.prefetched(), 2);
}

#[test]
fn edge_is_the_newest_dated_end_then_the_prefetch() {
  // The backup's newest segment: 2026-11-01T20:10:47.158Z, 2 s long.
  let end = twitch::time::epoch_millis("2026-11-01T20:10:49.158Z").unwrap();
  let mut plain = Splicer::new(false);
  plain.update(&backup(), None);
  assert_eq!(plain.edge(), Some((end, 0.0)));
  let mut low = Splicer::new(true);
  low.update(&backup(), None);
  assert_eq!(low.edge(), Some((end, 4.0)));
}
