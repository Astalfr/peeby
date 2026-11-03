//! Twitch playback as the shared player's [`Source`]:
//! - a live or a VOD: the variants the local proxy serves (a live's ads removed, docs/twitch-plan.md
//!   step 3), one quality each;
//! - a clip: its direct MP4s, one quality each.
//!
//! A VOD's storyboard becomes the seek bar's previews, its muted parts red on the seek bar, its
//! game chapters the player's chapters.

use std::rc::Rc;

use player::StartOptions;
use twitch::Proxy;
use twitch::gql::{self, ClipPlayback, StoryboardLevel};
use twitch::helix::MutedSegment;
use twitch::usher::{Codec, Variant};
use ui::i18n::t;
use ui::picture::Picture;
use ui::player::segments::{Segment, SegmentKind};
use ui::player::{Previews, Quality, Source, Tile};
use ui::tf;

use crate::settings;

/// Qualities offered in the settings, as heights.
pub const QUALITIES: &[u32] = &[1440, 1080, 720, 480, 360];

/// Default highest quality.
pub const DEFAULT_HEIGHT: u32 = 1080;

/// How far ahead mpv downloads a VOD or a clip: unbounded, it would fetch a whole VOD through
/// the proxy at once. A live never gets this far ahead of its edge.
const READAHEAD_SECS: u32 = 60;

/// mpv's options for `url`: Twitch's CDN wants the player's origin.
fn options(url: &str, player: &settings::Player) -> StartOptions {
  let mut options = StartOptions {
    url: url.to_owned(),
    http_headers: vec![
      format!("Referer: {}", twitch::usher::PLAYER_ORIGIN),
      format!("Origin: {}", twitch::usher::PLAYER_ORIGIN),
    ],
    readahead_secs: Some(READAHEAD_SECS),
    ..StartOptions::default()
  };
  player.apply(&mut options);
  options
}

/// The source for `qualities` (label, URL, height), best first: the first one not taller than
/// `max_height` plays.
fn source(
  qualities: Vec<(String, String, Option<u32>)>,
  max_height: u32,
  live: bool,
  player: &settings::Player,
) -> Source {
  let index = qualities
    .iter()
    .position(|(_, _, h)| h.is_some_and(|h| h <= max_height))
    .unwrap_or(0);
  let qualities: Vec<Quality> = qualities
    .into_iter()
    .map(|(label, url, _)| Quality {
      label: label.into(),
      options: options(&url, player),
    })
    .collect();
  Source {
    options: qualities
      .get(index)
      .map(|q| q.options.clone())
      .unwrap_or_default(),
    quality: (!qualities.is_empty()).then_some(index),
    qualities,
    live,
    ..Source::default()
  }
}

/// The codec and height of each quality of a [`proxied`] source, in its order.
pub type QualityCodecs = Vec<(Option<Codec>, Option<u32>)>;

/// A live's or a VOD's source from `proxy`'s variants: the landscape ones with a picture, then
/// "Audio only"; with the codec and height of each quality.
pub fn proxied(
  proxy: &Proxy,
  live: bool,
  max_height: u32,
  player: &settings::Player,
) -> (Source, QualityCodecs) {
  let mut playable: Vec<&Variant> = proxy
    .variants()
    .iter()
    .filter(|v| !v.portrait && !v.audio_only())
    .collect();
  playable.extend(proxy.variants().iter().filter(|v| v.audio_only()).take(1));
  let labels = labels(&playable);
  let qualities = playable
    .iter()
    .zip(labels)
    .map(|(v, label)| (label, v.url.clone(), v.height))
    .collect();
  let codecs = playable.iter().map(|v| (v.codec, v.height)).collect();
  (source(qualities, max_height, live, player), codecs)
}

/// The quality menu's labels: the variant's name, with its codec when another variant has the
/// same name; "Audio only".
fn labels(variants: &[&Variant]) -> Vec<String> {
  variants
    .iter()
    .map(|v| {
      if v.audio_only() {
        return t("Audio only", "Audio seul").to_owned();
      }
      let shared = variants
        .iter()
        .filter(|o| o.name == v.name && !o.audio_only())
        .count()
        > 1;
      match v.codec {
        Some(codec) if shared => format!("{} {}", v.name, codec_name(codec)),
        _ => v.name.clone(),
      }
    })
    .collect()
}

fn codec_name(codec: Codec) -> &'static str {
  match codec {
    Codec::Av1 => "AV1",
    Codec::H265 => "HEVC",
    Codec::H264 => "H.264",
  }
}

/// The settings' name of a codec (`software_codecs`).
pub fn codec_key(codec: Codec) -> &'static str {
  match codec {
    Codec::Av1 => "av1",
    Codec::H265 => "h265",
    Codec::H264 => "h264",
  }
}

/// The codecs to ask usher for, best first: AV1 and HEVC (Twitch's source above 1080p) unless
/// this PC decodes them on the CPU only, and H.264 always.
pub fn codecs(software: &[String]) -> Vec<Codec> {
  [Codec::Av1, Codec::H265, Codec::H264]
    .into_iter()
    .filter(|&c| c == Codec::H264 || !software.iter().any(|s| s == codec_key(c)))
    .collect()
}

/// The H.264 quality to fall back to from quality `current`: the best one not taller, else the
/// best one.
pub fn fallback(qualities: &QualityCodecs, current: usize) -> Option<usize> {
  let height = qualities.get(current)?.1;
  let h264 = |(_, (codec, _)): &(usize, &(Option<Codec>, Option<u32>))| *codec == Some(Codec::H264);
  qualities
    .iter()
    .enumerate()
    .filter(h264)
    .find(|(_, (_, h))| h.zip(height).is_none_or(|(h, height)| h <= height))
    .or_else(|| qualities.iter().enumerate().find(h264))
    .map(|(i, _)| i)
}

/// A clip's source: one quality per MP4.
pub fn clip(clip: &ClipPlayback, max_height: u32, player: &settings::Player) -> Source {
  let qualities = clip
    .qualities
    .iter()
    .map(|q| {
      let label = if q.frame_rate > 30 {
        format!("{}p{}", q.height, q.frame_rate)
      } else {
        format!("{}p", q.height)
      };
      (label, q.url.clone(), Some(q.height))
    })
    .collect();
  source(qualities, max_height, false, player)
}

/// A VOD's storyboard as the seek bar's previews.
pub struct Storyboard(pub StoryboardLevel);

impl Previews for Storyboard {
  fn tile(&self, seconds: f64, _duration: f64) -> Option<Tile> {
    let (url, column, row) = self.0.frame_at(seconds)?;
    Some(Tile {
      url: url.to_owned(),
      column,
      row,
    })
  }

  fn tile_size(&self) -> (u32, u32) {
    (self.0.width, self.0.height)
  }
}

/// The previews of the storyboard's largest level, if any.
pub fn previews(levels: Vec<StoryboardLevel>) -> Option<Rc<dyn Previews>> {
  let level = levels.into_iter().next()?;
  Some(Rc::new(Storyboard(level)) as Rc<dyn Previews>)
}

/// A VOD's muted parts as the player's segments: sorted, and those that touch merged, so that
/// skipping one does not land in the next.
pub fn muted(parts: &[MutedSegment]) -> Vec<Segment> {
  let mut parts = parts.to_vec();
  parts.sort_by_key(|p| p.offset);
  let mut out: Vec<Segment> = Vec::new();
  for part in parts {
    let (start, end) = (part.offset as f64, (part.offset + part.duration) as f64);
    match out.last_mut() {
      Some(last) if start <= last.end => last.end = last.end.max(end),
      _ => out.push(Segment {
        kind: SegmentKind::Muted,
        start,
        end,
      }),
    }
  }
  out
}

/// A VOD's game chapters as the player's, named after their games, with their box art. A single
/// one says nothing that the header does not: none then.
pub fn chapters(chapters: &[gql::Chapter]) -> Vec<(player::Chapter, Option<Picture>)> {
  if chapters.len() < 2 {
    return Vec::new();
  }
  chapters
    .iter()
    .map(|c| {
      let chapter = player::Chapter {
        start: f64::from(c.start),
        title: Some(c.game.clone()),
      };
      (chapter, c.box_art.as_deref().map(super::cards::template))
    })
    .collect()
}

/// What the user reads when a live, a VOD or a clip cannot play.
pub fn describe(error: &twitch::Error, channel: &str) -> String {
  use twitch::Error;
  match error {
    Error::Offline => tf!("{channel} is offline.", "{channel} n'est pas en direct."),
    Error::NotFound => t(
      "This channel or video doesn't exist.",
      "Cette chaîne ou cette vidéo n'existe pas.",
    )
    .into(),
    Error::SubscriberOnly => t(
      "Only for the channel's subscribers.",
      "Réservé aux abonnés de la chaîne.",
    )
    .into(),
    Error::IntegrityRequired => t(
      "Twitch asks for a check Peeby can't do without a browser. Try again later.",
      "Twitch demande une vérification que Peeby ne peut pas faire sans navigateur. Réessayez plus \
       tard.",
    )
    .into(),
    Error::NotLoggedIn => t(
      "Twitch rejects your site token: replace or remove it in the settings.",
      "Twitch refuse votre jeton du site : remplacez-le ou supprimez-le dans les réglages.",
    )
    .into(),
    Error::Http(_) => t(
      "Twitch can't be reached. Check your connection.",
      "Twitch est injoignable. Vérifiez votre connexion.",
    )
    .into(),
    other => tf!("Can't play: {other}", "Lecture impossible : {other}"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn one_game_is_no_chapters() {
    let chapter = |start, game: &str| gql::Chapter {
      start,
      game: game.into(),
      box_art: Some("https://static-cdn.jtvnw.net/ttv-boxart/21779-{width}x{height}.jpg".into()),
    };
    assert!(chapters(&[chapter(0, "Just Chatting")]).is_empty());
    let two = chapters(&[
      chapter(0, "Just Chatting"),
      chapter(1407, "League of Legends"),
    ]);
    assert_eq!(two.len(), 2);
    let (chapter, picture) = &two[1];
    assert_eq!(chapter.start, 1407.0);
    assert_eq!(chapter.title.as_deref(), Some("League of Legends"));
    let url = picture.as_ref().and_then(|p| p.url(52, 72));
    assert_eq!(
      url.as_deref(),
      Some("https://static-cdn.jtvnw.net/ttv-boxart/21779-52x72.jpg")
    );
  }

  #[test]
  fn muted_parts_that_touch_are_one() {
    let part = |offset, duration| MutedSegment { offset, duration };
    // As in helix_videos.json: three in a row from 8833.
    let parts = [
      part(9193, 180),
      part(721, 180),
      part(8833, 180),
      part(9013, 180),
    ];
    let spans: Vec<(f64, f64)> = muted(&parts).iter().map(|s| (s.start, s.end)).collect();
    assert_eq!(spans, vec![(721.0, 901.0), (8833.0, 9373.0)]);
  }

  fn variant(name: &str, codec: Option<Codec>, height: Option<u32>) -> Variant {
    Variant {
      id: name.into(),
      name: name.into(),
      height,
      frame_rate: None,
      bandwidth: 0,
      codecs: String::new(),
      codec,
      portrait: false,
      url: String::new(),
    }
  }

  #[test]
  fn labels_show_the_codec_only_when_names_repeat() {
    let a = variant("1440p60", Some(Codec::H265), Some(1440));
    let b = variant("1080p60", Some(Codec::H265), Some(1080));
    let c = variant("1080p60", Some(Codec::H264), Some(1080));
    let d = variant("audio_only", None, None);
    assert_eq!(
      labels(&[&a, &b, &c, &d]),
      ["1440p60", "1080p60 HEVC", "1080p60 H.264", "Audio only"]
    );
  }

  #[test]
  fn codecs_drop_those_decoded_on_the_cpu() {
    assert_eq!(codecs(&[]), [Codec::Av1, Codec::H265, Codec::H264]);
    assert_eq!(
      codecs(&["av1".to_owned(), "h264".to_owned()]),
      [Codec::H265, Codec::H264]
    );
  }

  #[test]
  fn fallback_takes_the_best_h264_not_taller() {
    let qualities: QualityCodecs = vec![
      (Some(Codec::H265), Some(1440)),
      (Some(Codec::H264), Some(1080)),
      (Some(Codec::H264), Some(720)),
      (None, None),
    ];
    assert_eq!(fallback(&qualities, 0), Some(1));
    assert_eq!(fallback(&qualities, 2), Some(2));
    let small: QualityCodecs = vec![
      (Some(Codec::Av1), Some(360)),
      (Some(Codec::H264), Some(720)),
    ];
    assert_eq!(fallback(&small, 0), Some(1));
  }
}
