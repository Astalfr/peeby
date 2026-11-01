//! Direct Play negotiation (`PlaybackInfo` + device profile), stream URLs and playback reporting.
//! See docs/jellyfin/direct-play.md: Peeby never accepts a transcoded stream.

use reqwest::Method;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Client, Error, ItemId, Result};

/// 1 Gbit/s: high enough that bitrate limits never trigger a transcode.
const MAX_BITRATE: i64 = 1_000_000_000;

/// The device profile sent with `PlaybackInfo`: accept every container and codec, and every
/// subtitle format both embedded and external, so the server never transcodes or burns in.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct DeviceProfile {
  name: &'static str,
  max_streaming_bitrate: i64,
  max_static_bitrate: i64,
  direct_play_profiles: Vec<DirectPlayProfile>,
  transcoding_profiles: Vec<()>,
  container_profiles: Vec<()>,
  codec_profiles: Vec<()>,
  subtitle_profiles: Vec<SubtitleProfile>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct DirectPlayProfile {
  #[serde(rename = "Type")]
  kind: &'static str,
  container: &'static str,
  video_codec: &'static str,
  audio_codec: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct SubtitleProfile {
  format: &'static str,
  method: &'static str,
}

/// Containers accepted as is. An empty `Container` does not mean "any" on Jellyfin 12, so
/// everything mpv/FFmpeg can demux is listed explicitly.
pub const CONTAINERS: &str = "mkv,matroska,webm,mp4,m4v,mov,3gp,3g2,mj2,avi,ts,mts,m2ts,mpegts,mpeg,mpg,vob,flv,wmv,asf,ogv,ogg,wtv,dvr-ms";
/// Video codecs decoded locally (FFmpeg, d3d11va when possible).
pub const VIDEO_CODECS: &str = "h264,hevc,h265,av1,vp8,vp9,mpeg1video,mpeg2video,mpeg4,msmpeg4v2,msmpeg4v3,vc1,wmv1,wmv2,wmv3,theora,prores,mjpeg";
/// Audio codecs decoded locally and downmixed (no passthrough).
pub const AUDIO_CODECS: &str = "aac,mp3,mp2,ac3,eac3,dts,dca,truehd,mlp,flac,alac,opus,vorbis,pcm_s16le,pcm_s24le,pcm_s32le,pcm_s16be,pcm_s24be,pcm_f32le,wmav1,wmav2,wmapro,wmalossless";

/// Subtitle formats declared both `Embed` and `External`. Missing ASS here makes the server
/// burn subtitles into the video, which is a full transcode.
pub const SUBTITLE_FORMATS: &[&str] = &[
  "ass", "ssa", "srt", "subrip", "vtt", "webvtt", "pgssub", "pgs", "dvdsub", "dvbsub",
];

impl Default for DeviceProfile {
  fn default() -> Self {
    let subtitle_profiles = SUBTITLE_FORMATS
      .iter()
      .flat_map(|format| {
        [
          SubtitleProfile {
            format,
            method: "Embed",
          },
          SubtitleProfile {
            format,
            method: "External",
          },
        ]
      })
      .collect();
    Self {
      name: "Peeby",
      max_streaming_bitrate: MAX_BITRATE,
      max_static_bitrate: MAX_BITRATE,
      direct_play_profiles: vec![
        DirectPlayProfile {
          kind: "Video",
          container: CONTAINERS,
          video_codec: VIDEO_CODECS,
          audio_codec: AUDIO_CODECS,
        },
        DirectPlayProfile {
          kind: "Audio",
          container: CONTAINERS,
          video_codec: "",
          audio_codec: AUDIO_CODECS,
        },
      ],
      transcoding_profiles: Vec::new(),
      container_profiles: Vec::new(),
      codec_profiles: Vec::new(),
      subtitle_profiles,
    }
  }
}

/// `PlaybackInfoResponse`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackInfo {
  /// Versions of the item (usually one).
  #[serde(default)]
  pub media_sources: Vec<MediaSource>,
  /// Session id to put in the stream URL and in every report.
  pub play_session_id: Option<String>,
  /// Set when the server refuses playback (e.g. `NotAllowed`).
  pub error_code: Option<String>,
}

/// `MediaSourceInfo`, reduced.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSource {
  /// Media source id.
  pub id: String,
  /// Container, e.g. `mkv`.
  pub container: Option<String>,
  /// Duration in ticks.
  pub run_time_ticks: Option<i64>,
  /// Whether the server lets the client read the file as is.
  #[serde(default)]
  pub supports_direct_play: bool,
  /// Present when the server wants to transcode.
  pub transcoding_url: Option<String>,
  /// Streams of the file, in container order (`Index` = FFmpeg stream index for embedded ones).
  #[serde(default)]
  pub media_streams: Vec<MediaStream>,
  /// Attachments (fonts) of the file.
  #[serde(default)]
  pub media_attachments: Vec<MediaAttachment>,
  /// Audio stream the server picked from the user's preferences.
  pub default_audio_stream_index: Option<i32>,
  /// Subtitle stream the server picked from the user's preferences (-1 or absent: none).
  pub default_subtitle_stream_index: Option<i32>,
}

/// `MediaStream`, reduced.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaStream {
  /// Index in `MediaStreams`; for embedded streams, the FFmpeg stream index.
  pub index: i32,
  /// Stream type.
  #[serde(rename = "Type")]
  pub kind: StreamKind,
  /// Codec, e.g. `h264`, `eac3`, `ass`.
  pub codec: Option<String>,
  /// ISO 639 language.
  pub language: Option<String>,
  /// Track title from the file.
  pub title: Option<String>,
  /// Human-readable label built by the server.
  pub display_title: Option<String>,
  /// Default flag of the track.
  #[serde(default)]
  pub is_default: bool,
  /// Forced flag of the track.
  #[serde(default)]
  pub is_forced: bool,
  /// Subtitle file next to the video.
  #[serde(default)]
  pub is_external: bool,
  /// Where to fetch an external subtitle (server-relative).
  pub delivery_url: Option<String>,
  /// Audio channels.
  pub channels: Option<i32>,
}

/// `MediaStreamType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[allow(missing_docs)]
pub enum StreamKind {
  Video,
  Audio,
  Subtitle,
  #[serde(other)]
  Other,
}

/// `MediaAttachment`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct MediaAttachment {
  /// Attachment index.
  pub index: i32,
  /// File name, e.g. `Arial.ttf`.
  pub file_name: Option<String>,
  /// MIME type, e.g. `application/x-truetype-font`.
  pub mime_type: Option<String>,
  /// Codec name, e.g. `ttf`, `otf`.
  pub codec: Option<String>,
  /// Download URL (server-relative).
  pub delivery_url: Option<String>,
}

impl MediaAttachment {
  /// Whether this attachment is a font.
  pub fn is_font(&self) -> bool {
    let mime = self
      .mime_type
      .as_deref()
      .unwrap_or_default()
      .to_ascii_lowercase();
    let codec = self
      .codec
      .as_deref()
      .unwrap_or_default()
      .to_ascii_lowercase();
    let name = self
      .file_name
      .as_deref()
      .unwrap_or_default()
      .to_ascii_lowercase();
    mime.contains("font")
      || mime.contains("opentype")
      || matches!(codec.as_str(), "ttf" | "otf")
      || name.ends_with(".ttf")
      || name.ends_with(".otf")
      || name.ends_with(".ttc")
  }
}

impl MediaSource {
  /// Fails with [`Error::TranscodingRequired`] unless the file can be played as is.
  pub fn ensure_direct_play(&self) -> Result<()> {
    let transcoding = self.transcoding_url.as_deref().filter(|u| !u.is_empty());
    if self.supports_direct_play && transcoding.is_none() {
      return Ok(());
    }
    let reasons = transcoding
      .and_then(|url| Url::parse("http://x").ok()?.join(url).ok())
      .and_then(|url| {
        url
          .query_pairs()
          .find(|(k, _)| k.eq_ignore_ascii_case("TranscodeReasons"))
          .map(|(_, v)| v.into_owned())
      })
      .map(|v| v.split(',').map(str::to_owned).collect())
      .unwrap_or_default();
    Err(Error::TranscodingRequired { reasons })
  }

  /// Streams of one kind, in order.
  pub fn streams(&self, kind: StreamKind) -> impl Iterator<Item = &MediaStream> {
    self.media_streams.iter().filter(move |s| s.kind == kind)
  }
}

/// Playback state for the `Sessions/Playing*` reports.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlaybackReport {
  /// Item being played.
  pub item_id: ItemId,
  /// Media source being played.
  pub media_source_id: String,
  /// From [`PlaybackInfo::play_session_id`].
  pub play_session_id: Option<String>,
  /// Position in ticks.
  pub position_ticks: i64,
  /// Paused or playing.
  pub is_paused: bool,
  /// Selected audio stream (Jellyfin index).
  pub audio_stream_index: Option<i32>,
  /// Selected subtitle stream (Jellyfin index), `None` for off.
  pub subtitle_stream_index: Option<i32>,
  /// Always `DirectPlay`.
  pub play_method: &'static str,
  /// The client can seek.
  pub can_seek: bool,
}

impl Client {
  /// Negotiates playback of `item`. The caller must still call [`MediaSource::ensure_direct_play`].
  pub async fn playback_info(&self, item: &ItemId, start_ticks: i64) -> Result<PlaybackInfo> {
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Body<'a> {
      user_id: &'a str,
      max_streaming_bitrate: i64,
      start_time_ticks: i64,
      device_profile: DeviceProfile,
      enable_direct_play: bool,
      enable_direct_stream: bool,
      enable_transcoding: bool,
      allow_video_stream_copy: bool,
      allow_audio_stream_copy: bool,
      auto_open_live_stream: bool,
    }
    let user = self
      .session()
      .map(|s| s.user_id.to_string())
      .ok_or(Error::NotAuthenticated)?;
    let endpoint = format!("Items/{item}/PlaybackInfo");
    let body = Body {
      user_id: &user,
      max_streaming_bitrate: MAX_BITRATE,
      start_time_ticks: start_ticks,
      device_profile: DeviceProfile::default(),
      enable_direct_play: true,
      enable_direct_stream: false,
      enable_transcoding: false,
      allow_video_stream_copy: false,
      allow_audio_stream_copy: false,
      auto_open_live_stream: false,
    };
    let request = self
      .request(Method::POST, &endpoint)?
      .query(&[("userId", user.as_str())])
      .json(&body);
    let info: PlaybackInfo = self.send_json(request, &endpoint).await?;
    if let Some(code) = &info.error_code {
      return Err(Error::PlaybackRefused { code: code.clone() });
    }
    Ok(info)
  }

  /// URL of the original file (`static=true`: no transcoding, HTTP Range supported).
  /// Authentication goes in the `Authorization` header ([`Client::authorization_header`]).
  pub fn stream_url(
    &self,
    item: &ItemId,
    source: &MediaSource,
    play_session: Option<&str>,
  ) -> Result<Url> {
    let mut url = self.base_url().join(&format!("Videos/{item}/stream"))?;
    {
      let mut q = url.query_pairs_mut();
      q.append_pair("static", "true");
      q.append_pair("mediaSourceId", &source.id);
      if let Some(session) = play_session {
        q.append_pair("playSessionId", session);
      }
    }
    Ok(url)
  }

  /// Absolute URL for a server-relative `DeliveryUrl` (external subtitles, attachments).
  pub fn delivery_url(&self, path: &str) -> Result<Url> {
    Ok(self.base_url().join(path.trim_start_matches('/'))?)
  }

  /// Download URL of an attachment (font) of a media source.
  pub fn attachment_url(
    &self,
    item: &ItemId,
    source: &MediaSource,
    attachment: &MediaAttachment,
  ) -> Result<Url> {
    match &attachment.delivery_url {
      Some(path) => self.delivery_url(path),
      None => Ok(self.base_url().join(&format!(
        "Videos/{item}/{}/Attachments/{}",
        source.id, attachment.index
      ))?),
    }
  }

  /// `POST /Sessions/Playing`: playback started.
  pub async fn report_playing(&self, report: &PlaybackReport) -> Result<()> {
    self.post_report("Sessions/Playing", report).await
  }

  /// `POST /Sessions/Playing/Progress`: position, pause or track change.
  pub async fn report_progress(&self, report: &PlaybackReport) -> Result<()> {
    self.post_report("Sessions/Playing/Progress", report).await
  }

  /// `POST /Sessions/Playing/Stopped`: playback ended; saves the resume position.
  pub async fn report_stopped(&self, report: &PlaybackReport) -> Result<()> {
    self.post_report("Sessions/Playing/Stopped", report).await
  }

  async fn post_report(&self, endpoint: &str, report: &PlaybackReport) -> Result<()> {
    let request = self.request(Method::POST, endpoint)?.json(report);
    self.send_empty(request, endpoint).await
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn device_profile_declares_ass_embed_and_external() {
    let json = serde_json::to_value(DeviceProfile::default()).unwrap();
    let subs = json["SubtitleProfiles"].as_array().unwrap();
    for method in ["Embed", "External"] {
      for format in ["ass", "ssa"] {
        assert!(
          subs
            .iter()
            .any(|s| s["Format"] == format && s["Method"] == method),
          "{format}/{method} missing"
        );
      }
    }
    assert!(
      !subs.iter().any(|s| s["Method"] == "Encode"),
      "burn-in must never be offered"
    );
    assert_eq!(json["TranscodingProfiles"].as_array().unwrap().len(), 0);
    let video = &json["DirectPlayProfiles"][0];
    assert!(
      video["Container"]
        .as_str()
        .unwrap()
        .split(',')
        .any(|c| c == "mkv")
    );
    assert!(
      video["VideoCodec"]
        .as_str()
        .unwrap()
        .split(',')
        .any(|c| c == "hevc")
    );
    assert!(
      video["AudioCodec"]
        .as_str()
        .unwrap()
        .split(',')
        .any(|c| c == "truehd")
    );
  }

  fn source(direct: bool, transcoding: Option<&str>) -> MediaSource {
    serde_json::from_value(serde_json::json!({
      "Id": "src",
      "SupportsDirectPlay": direct,
      "TranscodingUrl": transcoding,
    }))
    .unwrap()
  }

  #[test]
  fn direct_play_guard() {
    assert!(source(true, None).ensure_direct_play().is_ok());
    let err = source(false, Some("/videos/x/master.m3u8?MediaSourceId=a&TranscodeReasons=SubtitleCodecNotSupported,AudioCodecNotSupported"))
      .ensure_direct_play()
      .unwrap_err();
    match err {
      Error::TranscodingRequired { reasons } => assert_eq!(
        reasons,
        ["SubtitleCodecNotSupported", "AudioCodecNotSupported"]
      ),
      other => panic!("{other}"),
    }
    assert!(
      source(true, Some("/videos/x/master.m3u8"))
        .ensure_direct_play()
        .is_err()
    );
  }
}
