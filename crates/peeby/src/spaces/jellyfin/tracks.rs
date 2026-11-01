//! Mapping between Jellyfin stream indexes and mpv track ids. From peeby.
//!
//! mpv numbers tracks per type from 1, in container order, and appends external subtitle files
//! in the order they are given. Jellyfin's `MediaStream.Index` is the FFmpeg stream index for
//! embedded streams. So, for one media source:
//! - mpv audio id N = the N-th embedded audio stream by Jellyfin index;
//! - mpv subtitle id N = the N-th embedded subtitle stream, then the external ones in the order
//!   we pass them to mpv ([`TrackMap::external_subtitles`]).
//!
//! [`TrackPrefs`] carries the user's choice to the next episode: the stream with the same
//! language, and preferably the same title (`Signs & Songs` vs `Full`), is picked again.

use jellyfin_api::playback::{MediaSource, MediaStream, StreamKind};
use player::Choice;

/// A selectable track, for menus.
#[derive(Clone, Debug)]
pub struct TrackEntry {
  /// Jellyfin stream index.
  pub index: i32,
  /// Label from the server (`DisplayTitle`), e.g. `Japanese - AAC - Stereo`.
  pub label: String,
  pub language: Option<String>,
  pub title: Option<String>,
  pub forced: bool,
}

/// A stream to look for in another file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamPref {
  pub language: Option<String>,
  pub title: Option<String>,
  pub forced: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SubtitlePref {
  /// The server's default for this file.
  #[default]
  Default,
  Off,
  Like(StreamPref),
}

/// Track choices carried from one episode to the next.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrackPrefs {
  pub audio: Option<StreamPref>,
  pub subtitle: SubtitlePref,
}

#[derive(Clone, Debug, Default)]
pub struct TrackMap {
  /// Jellyfin indexes in mpv audio order.
  audio: Vec<TrackEntry>,
  /// Jellyfin indexes in mpv subtitle order (embedded first, then external).
  subtitles: Vec<TrackEntry>,
  /// External subtitle streams, in the order to hand them to mpv.
  external: Vec<MediaStream>,
}

impl TrackMap {
  pub fn new(source: &MediaSource) -> Self {
    let entry = |s: &MediaStream| TrackEntry {
      index: s.index,
      label: label(s),
      language: s.language.clone(),
      title: s.title.clone(),
      forced: s.is_forced,
    };
    let mut audio: Vec<&MediaStream> = source
      .streams(StreamKind::Audio)
      .filter(|s| !s.is_external)
      .collect();
    audio.sort_by_key(|s| s.index);
    let mut embedded: Vec<&MediaStream> = source
      .streams(StreamKind::Subtitle)
      .filter(|s| !s.is_external)
      .collect();
    embedded.sort_by_key(|s| s.index);
    let external: Vec<MediaStream> = source
      .streams(StreamKind::Subtitle)
      .filter(|s| s.is_external)
      .cloned()
      .collect();
    Self {
      audio: audio.into_iter().map(entry).collect(),
      subtitles: embedded
        .into_iter()
        .chain(external.iter())
        .map(entry)
        .collect(),
      external,
    }
  }

  pub fn audio(&self) -> &[TrackEntry] {
    &self.audio
  }

  pub fn subtitles(&self) -> &[TrackEntry] {
    &self.subtitles
  }

  /// External subtitle streams, to load into mpv in this order.
  pub fn external_subtitles(&self) -> &[MediaStream] {
    &self.external
  }

  /// mpv `aid` for a Jellyfin audio index.
  pub fn mpv_audio(&self, index: Option<i32>) -> Choice {
    match index.and_then(|i| self.audio.iter().position(|t| t.index == i)) {
      Some(pos) => Choice::Track(pos as i64 + 1),
      None => Choice::Auto,
    }
  }

  /// mpv `sid` for a Jellyfin subtitle index; `None` or `-1` means off.
  pub fn mpv_subtitle(&self, index: Option<i32>) -> Choice {
    match index
      .filter(|i| *i >= 0)
      .and_then(|i| self.subtitles.iter().position(|t| t.index == i))
    {
      Some(pos) => Choice::Track(pos as i64 + 1),
      None => Choice::Off,
    }
  }
}

impl TrackMap {
  /// The current choice, to carry to the next episode.
  pub fn prefs(&self, audio: Option<i32>, subtitle: Option<i32>) -> TrackPrefs {
    let pref = |t: &TrackEntry| StreamPref {
      language: t.language.clone(),
      title: t.title.clone(),
      forced: t.forced,
    };
    TrackPrefs {
      audio: audio
        .and_then(|i| self.audio.iter().find(|t| t.index == i))
        .map(pref),
      subtitle: match subtitle.and_then(|i| self.subtitles.iter().find(|t| t.index == i)) {
        Some(track) => SubtitlePref::Like(pref(track)),
        None => SubtitlePref::Off,
      },
    }
  }

  /// Jellyfin indexes to start with: the carried choice where this file has a match,
  /// else the server defaults.
  pub fn apply(
    &self,
    prefs: &TrackPrefs,
    default_audio: Option<i32>,
    default_subtitle: Option<i32>,
  ) -> (Option<i32>, Option<i32>) {
    let audio = prefs
      .audio
      .as_ref()
      .and_then(|p| best_match(&self.audio, p))
      .or(default_audio);
    let subtitle = match &prefs.subtitle {
      SubtitlePref::Default => default_subtitle,
      SubtitlePref::Off => None,
      SubtitlePref::Like(p) => best_match(&self.subtitles, p).or(default_subtitle),
    };
    (audio, subtitle.filter(|i| *i >= 0))
  }
}

/// One spelling per language: ISO 639-2/B codes (`fre`, from MKV tags) and the few 639-1
/// codes some files carry are mapped to ISO 639-2/T (`fra`).
fn normalize_language(code: &str) -> String {
  let code = code.to_ascii_lowercase();
  let mapped = match code.as_str() {
    "fre" | "fr" => "fra",
    "ger" | "de" => "deu",
    "chi" | "zh" => "zho",
    "dut" | "nl" => "nld",
    "cze" | "cs" => "ces",
    "gre" | "el" => "ell",
    "per" | "fa" => "fas",
    "rum" | "ro" => "ron",
    "slo" | "sk" => "slk",
    "en" => "eng",
    "ja" => "jpn",
    "es" => "spa",
    "it" => "ita",
    "ko" => "kor",
    "pt" => "por",
    "ru" => "rus",
    _ => return code,
  };
  mapped.to_owned()
}

/// The track with the same language that looks most like `pref`.
fn best_match(tracks: &[TrackEntry], pref: &StreamPref) -> Option<i32> {
  let same_title = |a: &Option<String>, b: &Option<String>| match (a, b) {
    (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
    _ => false,
  };
  let same_language = |a: &Option<String>, b: &Option<String>| match (a, b) {
    (Some(a), Some(b)) => normalize_language(a) == normalize_language(b),
    _ => false,
  };
  tracks
    .iter()
    .filter(|t| pref.language.is_none() || same_language(&t.language, &pref.language))
    .max_by_key(|t| {
      // Ties keep the first track in file order (`max_by_key` returns the last maximum).
      let score =
        2 * i32::from(same_title(&t.title, &pref.title)) + i32::from(t.forced == pref.forced);
      (score, -t.index)
    })
    .map(|t| t.index)
}

fn label(stream: &MediaStream) -> String {
  let base = stream
    .display_title
    .clone()
    .or_else(|| stream.title.clone())
    .or_else(|| stream.language.clone())
    .unwrap_or_else(|| format!("Piste {}", stream.index));
  if stream.is_external && !base.to_ascii_lowercase().contains("extern") {
    format!("{base} (externe)")
  } else {
    base
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn source() -> MediaSource {
    serde_json::from_value(serde_json::json!({
      "Id": "s",
      "SupportsDirectPlay": true,
      "MediaStreams": [
        { "Index": 0, "Type": "Video", "Codec": "hevc" },
        { "Index": 1, "Type": "Audio", "Codec": "flac", "Language": "jpn" },
        { "Index": 2, "Type": "Audio", "Codec": "aac", "Language": "eng" },
        { "Index": 3, "Type": "Subtitle", "Codec": "ass", "DisplayTitle": "English - ASS" },
        { "Index": 4, "Type": "Subtitle", "Codec": "ass", "DisplayTitle": "Signs" },
        { "Index": 5, "Type": "Subtitle", "Codec": "srt", "IsExternal": true, "DeliveryUrl": "/Videos/x/s/Subtitles/5/0/Stream.srt" }
      ]
    }))
    .unwrap()
  }

  fn episode(subtitles: serde_json::Value) -> TrackMap {
    let mut streams = vec![
      serde_json::json!({ "Index": 0, "Type": "Video" }),
      serde_json::json!({ "Index": 1, "Type": "Audio", "Language": "jpn" }),
      serde_json::json!({ "Index": 2, "Type": "Audio", "Language": "eng" }),
    ];
    streams.extend(subtitles.as_array().unwrap().iter().cloned());
    TrackMap::new(
      &serde_json::from_value(serde_json::json!({
        "Id": "s", "SupportsDirectPlay": true, "MediaStreams": streams
      }))
      .unwrap(),
    )
  }

  #[test]
  fn prefs_follow_language_and_title() {
    let first = episode(serde_json::json!([
      { "Index": 3, "Type": "Subtitle", "Language": "eng", "Title": "Signs & Songs", "IsForced": true },
      { "Index": 4, "Type": "Subtitle", "Language": "eng", "Title": "Full" },
    ]));
    let prefs = first.prefs(Some(2), Some(4));
    // The next file lists its subtitles in another order.
    let next = episode(serde_json::json!([
      { "Index": 3, "Type": "Subtitle", "Language": "eng", "Title": "Full" },
      { "Index": 4, "Type": "Subtitle", "Language": "fre", "Title": "Full" },
      { "Index": 5, "Type": "Subtitle", "Language": "eng", "Title": "Signs & Songs", "IsForced": true },
    ]));
    assert_eq!(next.apply(&prefs, Some(1), Some(5)), (Some(2), Some(3)));
    // Forced signs carry over too.
    let signs = first.prefs(Some(1), Some(3));
    assert_eq!(next.apply(&signs, Some(2), Some(3)), (Some(1), Some(5)));
  }

  #[test]
  fn language_codes_match_across_spellings() {
    let map = episode(serde_json::json!([
      { "Index": 3, "Type": "Subtitle", "Language": "fre", "Title": "Signs", "IsForced": true },
      { "Index": 4, "Type": "Subtitle", "Language": "fre" },
    ]));
    let prefs = TrackPrefs {
      subtitle: SubtitlePref::Like(StreamPref {
        language: Some("fra".into()),
        title: None,
        forced: false,
      }),
      ..TrackPrefs::default()
    };
    // The full (not forced) track wins.
    assert_eq!(map.apply(&prefs, None, None), (None, Some(4)));
    assert_eq!(normalize_language("GER"), "deu");
    assert_eq!(normalize_language("ja"), "jpn");
  }

  #[test]
  fn prefs_fall_back_to_server_defaults() {
    let map = episode(serde_json::json!([
      { "Index": 3, "Type": "Subtitle", "Language": "fre" },
    ]));
    let prefs = TrackPrefs {
      audio: Some(StreamPref {
        language: Some("ger".into()),
        title: None,
        forced: false,
      }),
      subtitle: SubtitlePref::Like(StreamPref {
        language: Some("eng".into()),
        title: None,
        forced: false,
      }),
    };
    assert_eq!(map.apply(&prefs, Some(1), Some(3)), (Some(1), Some(3)));
    let off = TrackPrefs {
      subtitle: SubtitlePref::Off,
      ..TrackPrefs::default()
    };
    assert_eq!(map.apply(&off, Some(1), Some(3)), (Some(1), None));
    assert_eq!(
      map.apply(&TrackPrefs::default(), Some(1), Some(-1)),
      (Some(1), None)
    );
  }

  #[test]
  fn audio_ids_follow_container_order() {
    let map = TrackMap::new(&source());
    assert_eq!(map.mpv_audio(Some(1)), Choice::Track(1));
    assert_eq!(map.mpv_audio(Some(2)), Choice::Track(2));
    assert_eq!(map.mpv_audio(Some(99)), Choice::Auto);
    assert_eq!(map.mpv_audio(None), Choice::Auto);
  }

  #[test]
  fn external_subtitles_come_after_embedded() {
    let map = TrackMap::new(&source());
    assert_eq!(map.mpv_subtitle(Some(3)), Choice::Track(1));
    assert_eq!(map.mpv_subtitle(Some(4)), Choice::Track(2));
    assert_eq!(map.mpv_subtitle(Some(5)), Choice::Track(3));
    assert_eq!(map.mpv_subtitle(Some(-1)), Choice::Off);
    assert_eq!(map.mpv_subtitle(None), Choice::Off);
    assert_eq!(map.external_subtitles().len(), 1);
    assert_eq!(map.subtitles()[2].label, "Piste 5 (externe)");
  }
}
