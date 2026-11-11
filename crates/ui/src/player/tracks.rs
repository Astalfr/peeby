//! What the player picks within the file: audio and subtitle tracks (mpv's, named by the source
//! when it knows better), the subtitle delay, chapters, skippable segments and the "up next"
//! offer near the end. From peeby's player page.

use std::time::Duration;

use gpui::{Context, SharedString};
use player::{Choice, Track, TrackKind};

use super::segments::{self, Segment, SegmentKind};
use super::{PlayerEvent, PlayerView};
use crate::i18n::t;
use crate::picture::Picture;
use crate::text::repair_mojibake;
use crate::tf;

/// Without segment information, "up next" shows this long before the end.
const UP_NEXT_LEAD_SECONDS: f64 = 20.0;
/// mpv's default `chapter-seek-threshold`, seconds.
const CHAPTER_SEEK_THRESHOLD: f64 = 5.0;
/// Z and X change the subtitle delay by this much.
pub(super) const SUBTITLE_DELAY_STEP: f64 = 0.1;

/// The "up next" card.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum UpNext {
  Hidden,
  /// Shown; `Some(n)`: the next item starts in `n` seconds.
  Offered(Option<u32>),
  /// Closed by the user, until they seek back before it.
  Dismissed,
}

/// What the skip button does.
pub(super) enum SkipAction {
  SeekTo(f64),
  Next,
}

impl PlayerView {
  /// mpv's tracks of `kind`, in mpv's order (their ids are 1, 2…).
  pub(super) fn tracks_of(&self, kind: TrackKind) -> Vec<Track> {
    self
      .tracks
      .iter()
      .filter(|t| t.kind == kind)
      .cloned()
      .collect()
  }

  /// The selected track of `kind`, by mpv id.
  pub(super) fn selected(&self, kind: TrackKind) -> Option<i64> {
    self
      .tracks
      .iter()
      .find(|t| t.kind == kind && t.selected)
      .map(|t| t.id)
  }

  /// A track's name: the source's when it has one, else built from the file's tags.
  pub(super) fn track_label(&self, track: &Track) -> String {
    let names = self.source.as_ref().map(|s| match track.kind {
      TrackKind::Audio => &s.audio_names,
      _ => &s.subtitle_names,
    });
    if let Some(name) = names.and_then(|n| n.get(track.id as usize - 1))
      && !name.is_empty()
    {
      return name.clone();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(title) = &track.title {
      parts.push(title.clone());
    }
    if let Some(lang) = &track.lang
      && !parts.iter().any(|p| p.eq_ignore_ascii_case(lang))
    {
      parts.push(lang.clone());
    }
    if parts.is_empty() {
      parts.push(tf!("Track {}", "Piste {}", track.id));
    }
    if let Some(codec) = &track.codec {
      parts.push(codec.to_uppercase());
    }
    parts.join(" · ")
  }

  /// mpv reported its tracks: keep them and tell the page when the playing ones change (mpv
  /// reports the list again for other reasons).
  pub(super) fn on_tracks(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
    let before = (
      self.selected(TrackKind::Audio),
      self.selected(TrackKind::Subtitle),
    );
    self.tracks = tracks;
    let (audio, subtitle) = (
      self.selected(TrackKind::Audio),
      self.selected(TrackKind::Subtitle),
    );
    if (audio, subtitle) != before {
      cx.emit(PlayerEvent::TracksChanged { audio, subtitle });
    }
  }

  pub(super) fn select_audio(&mut self, id: i64, cx: &mut Context<Self>) {
    self.menu = None;
    self.with_player(|p| p.set_audio(Choice::Track(id)));
    cx.notify();
  }

  /// Shows mpv's subtitle track `id`, or none.
  pub(super) fn select_subtitle(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
    self.menu = None;
    self.with_player(|p| p.set_subtitle(id.map_or(Choice::Off, Choice::Track)));
    cx.notify();
  }

  /// A: the next audio track.
  pub(super) fn cycle_audio(&mut self, cx: &mut Context<Self>) {
    let tracks = self.tracks_of(TrackKind::Audio);
    if tracks.len() < 2 {
      return self.show_osd(
        t("Only one audio track", "Une seule piste audio").into(),
        cx,
      );
    }
    let current = self.selected(TrackKind::Audio);
    let pos = tracks.iter().position(|t| Some(t.id) == current);
    let next = tracks[pos.map_or(0, |p| (p + 1) % tracks.len())].clone();
    self.select_audio(next.id, cx);
    let label = self.track_label(&next);
    self.show_osd(tf!("Audio: {label}", "Audio : {label}").into(), cx);
  }

  /// C with the file's own subtitles: off → first → … → last → off.
  pub(super) fn cycle_subtitles(&mut self, cx: &mut Context<Self>) {
    let tracks = self.tracks_of(TrackKind::Subtitle);
    if tracks.is_empty() {
      return self.show_osd(t("No subtitles", "Aucun sous-titre").into(), cx);
    }
    let current = self.selected(TrackKind::Subtitle);
    let next = match tracks.iter().position(|t| Some(t.id) == current) {
      None => Some(tracks[0].clone()),
      Some(p) => tracks.get(p + 1).cloned(),
    };
    let label = next.as_ref().map_or_else(
      || t("Off", "Désactivés").to_owned(),
      |t| self.track_label(t),
    );
    self.select_subtitle(next.map(|t| t.id), cx);
    self.show_osd(
      tf!("Subtitles: {label}", "Sous-titres : {label}").into(),
      cx,
    );
  }

  /// Whether the subtitles come from the source, added on demand (YouTube), rather than from
  /// the file.
  pub(super) fn on_demand_subtitles(&self) -> bool {
    self
      .source
      .as_ref()
      .is_some_and(|s| !s.subtitles.is_empty())
  }

  pub(super) fn add_subtitle_delay(&mut self, delta: f64, cx: &mut Context<Self>) {
    self.subtitle_delay = ((self.subtitle_delay + delta) * 10.0).round() / 10.0;
    self.with_player(|p| p.set_subtitle_delay(self.subtitle_delay));
    self.show_osd(
      tf!(
        "Subtitle delay {:+.1} s",
        "Décalage des sous-titres {:+.1} s",
        self.subtitle_delay
      )
      .into(),
      cx,
    );
  }

  /// mpv reported the chapters: titles repaired, segments guessed from them. Ignored once the
  /// source gave its own.
  pub(super) fn on_chapters(&mut self, mut chapters: Vec<player::Chapter>) {
    if self.given_chapters {
      return;
    }
    for chapter in &mut chapters {
      if let Some(title) = &mut chapter.title {
        *title = repair_mojibake(title).into_owned();
      }
    }
    self.chapters = chapters;
    self.refresh_segments();
  }

  /// The chapter at `seconds`, from 0.
  pub(super) fn chapter_at(&self, seconds: f64) -> Option<usize> {
    self.chapters.iter().rposition(|c| c.start <= seconds)
  }

  /// The title of chapter `index`, or its number.
  pub(super) fn chapter_title(&self, index: usize) -> String {
    self
      .chapters
      .get(index)
      .and_then(|c| c.title.clone())
      .unwrap_or_else(|| tf!("Chapter {}", "Chapitre {}", index + 1))
  }

  /// The picture of chapter `index`, if the source gave one.
  pub(super) fn chapter_picture(&self, index: usize) -> Option<&Picture> {
    self.chapter_pictures.get(index)?.as_ref()
  }

  /// Page Up / Page Down: the previous or next chapter.
  pub(super) fn add_chapter(&mut self, delta: i64, cx: &mut Context<Self>) {
    if self.chapters.is_empty() {
      return self.show_osd(t("No chapters", "Aucun chapitre").into(), cx);
    }
    // As mpv does (`chapter-seek-threshold`): "previous" well into a chapter goes back to its
    // start first.
    let current = self.chapter_at(self.position + 0.5).unwrap_or(0);
    let into = self.position - self.chapters[current].start;
    let step = if delta < 0 && into > CHAPTER_SEEK_THRESHOLD {
      delta + 1
    } else {
      delta
    };
    let target = (current as i64 + step).clamp(0, self.chapters.len() as i64 - 1) as usize;
    if self.given_chapters {
      // mpv does not know them. "Next" in the last one stays there.
      if current as i64 + step < self.chapters.len() as i64 {
        self.seek_to(self.chapters[target].start, cx);
      }
    } else {
      self.with_player(|p| p.add_chapter(delta));
    }
    self.show_osd(self.chapter_title(target).into(), cx);
  }

  /// Chapters the source's server knows (YouTube's): they replace the file's. Segments are not
  /// guessed from their titles, whose words (`Intro`, `Outro`) are a creator's, not a release's.
  pub fn set_chapters(&mut self, chapters: Vec<player::Chapter>, cx: &mut Context<Self>) {
    self.set_chapters_with_pictures(chapters.into_iter().map(|c| (c, None)).collect(), cx);
  }

  /// Chapters from the source's server with a picture each (a game's box art), shown in the
  /// chapters menu and over the seek bar.
  pub fn set_chapters_with_pictures(
    &mut self,
    chapters: Vec<(player::Chapter, Option<Picture>)>,
    cx: &mut Context<Self>,
  ) {
    let (chapters, pictures): (Vec<_>, Vec<_>) = chapters.into_iter().unzip();
    self.given_chapters = true;
    self.chapters = chapters;
    self.chapter_pictures = if pictures.iter().any(Option::is_some) {
      pictures
    } else {
      Vec::new()
    };
    self.refresh_segments();
    cx.notify();
  }

  /// The source's segments when it gave some, else those of the file's chapter titles.
  pub(super) fn refresh_segments(&mut self) {
    if self.given_segments.is_empty() && !self.given_chapters && self.duration > 0.0 {
      self.segments = segments::from_chapters(&self.chapters, self.duration);
    } else {
      self.segments = self.given_segments.clone();
    }
  }

  /// When "up next" appears: where the credits start, else a few seconds before the end.
  pub(super) fn up_next_at(&self) -> f64 {
    segments::tail_start(&self.segments, self.duration)
      .unwrap_or(self.duration - UP_NEXT_LEAD_SECONDS)
      .max(0.0)
  }

  /// Shows, counts down or hides the "up next" card as the position moves.
  pub(super) fn update_up_next(&mut self, cx: &mut Context<Self>) {
    // A DVR live's duration is its edge, not an end.
    if self.up_next_title.is_none() || self.duration <= 0.0 || self.is_live() {
      return;
    }
    let at = self.up_next_at();
    match self.up_next {
      UpNext::Hidden if self.position >= at => {
        if self.prefs.autoplay_next {
          self.up_next = UpNext::Offered(Some(self.prefs.autoplay_countdown));
          self.countdown_task = Some(cx.spawn(async move |this, cx| {
            loop {
              cx.background_executor().timer(Duration::from_secs(1)).await;
              let done = this.update(cx, |this, cx| {
                let UpNext::Offered(Some(left)) = this.up_next else {
                  return true;
                };
                if left <= 1 {
                  this.play_next(cx);
                  return true;
                }
                this.up_next = UpNext::Offered(Some(left - 1));
                cx.notify();
                false
              });
              if done.unwrap_or(true) {
                break;
              }
            }
          }));
        } else {
          self.up_next = UpNext::Offered(None);
        }
      }
      UpNext::Offered(_) | UpNext::Dismissed if self.position < at - 1.0 => {
        self.up_next = UpNext::Hidden;
        self.countdown_task = None;
      }
      _ => {}
    }
  }

  pub(super) fn dismiss_up_next(&mut self, cx: &mut Context<Self>) {
    self.up_next = UpNext::Dismissed;
    self.countdown_task = None;
    cx.notify();
  }

  /// Asks the page for the next item.
  pub(super) fn play_next(&mut self, cx: &mut Context<Self>) {
    self.countdown_task = None;
    cx.emit(PlayerEvent::Next);
  }

  /// The skip button's label and action, while a segment plays.
  pub(super) fn skip_action(&self) -> Option<(&'static str, SkipAction)> {
    let segment = segments::active(&self.segments, self.position)?;
    let in_tail = segments::tail_start(&self.segments, self.duration)
      .is_some_and(|start| segment.start >= start);
    if in_tail && self.up_next_title.is_some() {
      return Some((t("Next episode", "Épisode suivant"), SkipAction::Next));
    }
    let label = match segment.kind {
      SegmentKind::Intro => t("Skip intro", "Passer l'intro"),
      SegmentKind::Recap => t("Skip recap", "Passer le récap"),
      SegmentKind::Outro => t("Skip credits", "Passer le générique"),
      SegmentKind::Preview => t("Skip preview", "Passer l'aperçu"),
      SegmentKind::Commercial => t("Skip ad", "Passer la pub"),
      SegmentKind::Muted => t("Skip muted part", "Passer la partie muette"),
      SegmentKind::Sponsor => t("Skip sponsor", "Passer le sponsor"),
      SegmentKind::SelfPromo => t("Skip self-promotion", "Passer l'autopromo"),
      SegmentKind::Interaction => t("Skip reminder", "Passer le rappel"),
      SegmentKind::EndCards => t("Skip end cards", "Passer l'écran de fin"),
      SegmentKind::Hook => t("Skip hook", "Passer l'accroche"),
      SegmentKind::Filler => t("Skip tangent", "Passer la digression"),
      SegmentKind::OffTopic => t("Skip non-music", "Passer la partie sans musique"),
    };
    Some((label, SkipAction::SeekTo(segment.end)))
  }

  /// A segment to skip on its own started: the playhead jumps to its end, once (seeking back
  /// into it plays it), and the on-screen message says what was skipped.
  pub(super) fn auto_skip(&mut self, cx: &mut Context<Self>) {
    let Some(segment) = segments::active(&self.segments, self.position) else {
      return;
    };
    if !segment.auto_skip || !self.auto_skipped.insert(segment.start.to_bits()) {
      return;
    }
    self.seek_to(segment.end, cx);
    self.show_osd(
      crate::tf!("{} skipped", "{} passé", segment.kind.name()).into(),
      cx,
    );
  }

  /// S: skips the playing segment.
  pub(super) fn skip(&mut self, cx: &mut Context<Self>) {
    match self.skip_action() {
      Some((_, SkipAction::SeekTo(end))) => self.seek_to(end, cx),
      Some((_, SkipAction::Next)) => self.play_next(cx),
      None => {}
    }
  }

  /// Segments the source's server knows (Jellyfin's media segments); without them the player
  /// guesses them from the chapter titles.
  pub fn set_segments(&mut self, segments: Vec<Segment>, cx: &mut Context<Self>) {
    self.given_segments = segments::usable(segments);
    self.auto_skipped.clear();
    self.refresh_segments();
    cx.notify();
  }

  /// Offers the next item (title shown in the card) near the end, and plays it after the
  /// countdown when autoplay is on. `None`: no next item.
  pub fn set_up_next(&mut self, title: Option<SharedString>, cx: &mut Context<Self>) {
    self.up_next_title = title;
    self.up_next = UpNext::Hidden;
    self.countdown_task = None;
    cx.notify();
  }

  /// mpv's selected audio and subtitle tracks, by mpv id (1, 2… per type).
  pub fn selected_tracks(&self) -> (Option<i64>, Option<i64>) {
    (
      self.selected(TrackKind::Audio),
      self.selected(TrackKind::Subtitle),
    )
  }

  /// Whether the video was watched to its credits: past where "up next" shows (never a live).
  pub fn finished(&self) -> bool {
    !self.is_live() && self.duration > 0.0 && self.position >= self.up_next_at() - 1.0
  }
}
