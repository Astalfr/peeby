//! Skippable parts of a video (opening, recap, credits, preview).
//!
//! A space can give the player the segments its server knows (Jellyfin's media segments).
//! Otherwise they are derived from the chapter titles that fansub and BD releases carry
//! (`Opening`, `OP`, `ED`, `Credits`, `Preview`…). A chapter runs until the next one starts.
//! From peeby.

use player::Chapter;

/// Longest opening or recap taken from a chapter title; a longer one is probably mislabeled.
const MAX_INTRO_SECONDS: f64 = 240.0;
/// Shortest segment worth a skip button.
const MIN_SEGMENT_SECONDS: f64 = 5.0;
/// The skip button disappears this close to the end of a segment.
const END_MARGIN_SECONDS: f64 = 1.0;

/// What a segment holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
  /// Opening.
  Intro,
  /// "Previously on…".
  Recap,
  /// End credits.
  Outro,
  /// Preview of the next episode.
  Preview,
  /// An advertisement.
  Commercial,
  /// Audio removed by the server (Twitch's copyright mutes): marked on the seek bar.
  Muted,
}

/// A skippable part of a video.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
  /// What it holds.
  pub kind: SegmentKind,
  /// Seconds.
  pub start: f64,
  /// Seconds.
  pub end: f64,
}

impl Segment {
  /// Whether the segment is credits or a preview: nothing of the story.
  pub fn is_tail(&self) -> bool {
    matches!(self.kind, SegmentKind::Outro | SegmentKind::Preview)
  }
}

/// `segments` sorted, without the ones too short for a skip button.
pub fn usable(mut segments: Vec<Segment>) -> Vec<Segment> {
  segments.retain(|s| s.end - s.start >= MIN_SEGMENT_SECONDS);
  segments.sort_by(|a, b| a.start.total_cmp(&b.start));
  segments
}

/// Segments guessed from chapter titles.
pub fn from_chapters(chapters: &[Chapter], duration: f64) -> Vec<Segment> {
  let titles: Vec<String> = chapters
    .iter()
    .map(|c| c.title.as_deref().unwrap_or_default().to_lowercase())
    .collect();
  // Releases that have an `OP` chapter use `Intro` for the cold open before it, and the ones
  // with an `ED` chapter use `Outro` for the scene after it.
  let has_opening = titles.iter().any(|t| is_opening(t));
  let has_credits = titles.iter().any(|t| is_credits(t));
  let mut out = Vec::new();
  for (i, chapter) in chapters.iter().enumerate() {
    let title = &titles[i];
    let kind = if is_marker_end(title) {
      // `Credits End`, `Preview End`: what follows the part, not the part.
      continue;
    } else if is_opening(title) || (!has_opening && first_word(title) == "intro") {
      SegmentKind::Intro
    } else if is_recap(title) {
      SegmentKind::Recap
    } else if is_credits(title) || (!has_credits && first_word(title) == "outro") {
      SegmentKind::Outro
    } else if is_preview(title) {
      SegmentKind::Preview
    } else {
      continue;
    };
    let end = chapters
      .get(i + 1)
      .map_or(duration, |next| next.start)
      .min(duration.max(chapter.start));
    let segment = Segment {
      kind,
      start: chapter.start,
      end,
    };
    let length = segment.end - segment.start;
    let too_long =
      matches!(kind, SegmentKind::Intro | SegmentKind::Recap) && length > MAX_INTRO_SECONDS;
    if length >= MIN_SEGMENT_SECONDS && !too_long {
      out.push(segment);
    }
  }
  out
}

/// The muted segment at `seconds`, to its very end.
pub fn muted_at(segments: &[Segment], seconds: f64) -> Option<Segment> {
  segments
    .iter()
    .find(|s| s.kind == SegmentKind::Muted && s.start <= seconds && seconds < s.end)
    .copied()
}

/// The segment playing at `position`, while a skip still makes sense.
pub fn active(segments: &[Segment], position: f64) -> Option<Segment> {
  segments
    .iter()
    .find(|s| s.start <= position && position < s.end - END_MARGIN_SECONDS)
    .copied()
}

/// Where the rest of the video is only credits and previews, if it is known: the start of
/// the last run of tail segments that reaches the end.
pub fn tail_start(segments: &[Segment], duration: f64) -> Option<f64> {
  let mut reach = duration;
  let mut start = None;
  for segment in segments.iter().rev() {
    if !segment.is_tail() || segment.end < reach - END_MARGIN_SECONDS * 2.0 {
      break;
    }
    reach = segment.start;
    start = Some(segment.start);
  }
  start
}

fn first_word(title: &str) -> &str {
  title
    .split(|c: char| !c.is_alphanumeric())
    .find(|w| !w.is_empty())
    .unwrap_or_default()
}

/// `op`, `op1`, `op 2`, `opening`, `opening song`…
fn is_opening(title: &str) -> bool {
  let word = first_word(title);
  word == "opening" || numbered(word, "op")
}

/// `ed`, `ed2`, `ending`, `credits`, `credits start`, `end credits`.
fn is_credits(title: &str) -> bool {
  let word = first_word(title);
  matches!(word, "ending" | "credits") || numbered(word, "ed") || title.starts_with("end credits")
}

fn is_marker_end(title: &str) -> bool {
  title
    .split(|c: char| !c.is_alphanumeric())
    .rfind(|w| !w.is_empty())
    == Some("end")
    && title.contains(' ')
}

fn is_preview(title: &str) -> bool {
  matches!(first_word(title), "preview" | "pv" | "next" | "yokoku")
}

fn is_recap(title: &str) -> bool {
  matches!(first_word(title), "recap" | "previously")
}

/// `prefix` alone or followed by digits only (`op`, `op1`, `ed02`).
fn numbered(word: &str, prefix: &str) -> bool {
  word
    .strip_prefix(prefix)
    .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn chapters(list: &[(&str, f64)]) -> Vec<Chapter> {
    list
      .iter()
      .map(|(title, start)| Chapter {
        start: *start,
        title: Some((*title).to_owned()),
      })
      .collect()
  }

  fn kinds(segments: &[Segment]) -> Vec<(SegmentKind, f64, f64)> {
    segments.iter().map(|s| (s.kind, s.start, s.end)).collect()
  }

  #[test]
  fn fansub_chapters() {
    let c = chapters(&[
      ("Prologue", 0.0),
      ("Opening", 69.0),
      ("Part A", 160.0),
      ("Ending", 1379.0),
      ("Preview", 1470.0),
    ]);
    let s = from_chapters(&c, 1480.0);
    assert_eq!(
      kinds(&s),
      vec![
        (SegmentKind::Intro, 69.0, 160.0),
        (SegmentKind::Outro, 1379.0, 1470.0),
        (SegmentKind::Preview, 1470.0, 1480.0),
      ]
    );
    assert_eq!(tail_start(&s, 1480.0), Some(1379.0));
  }

  #[test]
  fn intro_is_the_cold_open_when_there_is_an_op() {
    let c = chapters(&[
      ("Intro", 0.0),
      ("OP", 46.0),
      ("Episode", 135.0),
      ("ED", 1356.0),
    ]);
    let s = from_chapters(&c, 1435.0);
    assert_eq!(
      kinds(&s),
      vec![
        (SegmentKind::Intro, 46.0, 135.0),
        (SegmentKind::Outro, 1356.0, 1435.0),
      ]
    );
  }

  #[test]
  fn intro_without_op_is_the_opening() {
    let c = chapters(&[("Scene 1", 0.0), ("Intro", 54.0), ("Scene 3", 141.0)]);
    assert_eq!(
      kinds(&from_chapters(&c, 1427.0)),
      vec![(SegmentKind::Intro, 54.0, 141.0)]
    );
  }

  #[test]
  fn credits_followed_by_story_are_not_the_tail() {
    let c = chapters(&[("Episode", 0.0), ("Credits", 1291.0), ("Episode", 1381.0)]);
    let s = from_chapters(&c, 1420.0);
    assert_eq!(kinds(&s), vec![(SegmentKind::Outro, 1291.0, 1381.0)]);
    assert_eq!(tail_start(&s, 1420.0), None);
  }

  #[test]
  fn outro_after_ending_is_story() {
    let c = chapters(&[
      ("Intro", 0.0),
      ("Opening", 60.0),
      ("Part A", 150.0),
      ("Ending", 1250.0),
      ("Outro", 1340.0),
      ("Preview", 1400.0),
    ]);
    let s = from_chapters(&c, 1420.0);
    assert_eq!(
      kinds(&s),
      vec![
        (SegmentKind::Intro, 60.0, 150.0),
        (SegmentKind::Outro, 1250.0, 1340.0),
        (SegmentKind::Preview, 1400.0, 1420.0),
      ]
    );
    // The scene after the credits is not skipped by "up next".
    assert_eq!(tail_start(&s, 1420.0), Some(1400.0));
  }

  #[test]
  fn start_and_end_markers() {
    let c = chapters(&[
      ("Prologue", 0.0),
      ("Opening", 80.0),
      ("Episode", 170.0),
      ("Credits Start", 1300.0),
      ("Credits End", 1390.0),
      ("Preview End", 1420.0),
    ]);
    let s = from_chapters(&c, 1430.0);
    assert_eq!(
      kinds(&s),
      vec![
        (SegmentKind::Intro, 80.0, 170.0),
        (SegmentKind::Outro, 1300.0, 1390.0),
      ]
    );
  }

  #[test]
  fn generic_and_mislabeled_chapters_are_ignored() {
    let c = chapters(&[
      ("Chapter 1", 0.0),
      ("Chapter 2", 100.0),
      ("Opening", 400.0),
      ("Opera house", 900.0),
      ("Edge", 1000.0),
    ]);
    // The "Opening" chapter lasts 500 s: too long to be one.
    assert!(from_chapters(&c, 1400.0).is_empty());
  }

  #[test]
  fn numbered_markers() {
    assert!(is_opening("op1"));
    assert!(is_opening("op 2"));
    assert!(is_opening("opening song"));
    assert!(!is_opening("opera"));
    assert!(is_credits("ed02"));
    assert!(is_credits("end credits"));
    assert!(!is_credits("edge"));
    assert!(!is_credits("post-credits"));
  }

  #[test]
  fn muted_segment_lasts_to_its_end() {
    let s = vec![
      Segment {
        kind: SegmentKind::Intro,
        start: 0.0,
        end: 50.0,
      },
      Segment {
        kind: SegmentKind::Muted,
        start: 10.0,
        end: 100.0,
      },
    ];
    assert_eq!(muted_at(&s, 5.0), None);
    assert_eq!(muted_at(&s, 99.5).map(|s| s.start), Some(10.0));
    assert_eq!(muted_at(&s, 100.0), None);
  }

  #[test]
  fn active_segment_hides_near_its_end() {
    let s = vec![Segment {
      kind: SegmentKind::Intro,
      start: 10.0,
      end: 100.0,
    }];
    assert_eq!(active(&s, 9.0), None);
    assert!(active(&s, 10.0).is_some());
    assert!(active(&s, 98.0).is_some());
    assert_eq!(active(&s, 99.5), None);
  }
}
