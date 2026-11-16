//! The signed-in `WEB_EMBEDDED_PLAYER` client: what plays the videos YouTube's bot check refuses
//! to VISIONOS (an anonymous visitor). Values from yt-dlp 2026.10.26 (`_base.py`, `_video.py`);
//! measurements in docs/youtube-bot-check.md.
//!
//! A play takes the embed page's ytcfg ([`EmbedConfig`]), the player JS's `signatureTimestamp`
//! ([`signature_timestamp`]), a `player` request, then the HLS manifest whose `n` challenge
//! ([`n_challenge`]) a JS solver ([`crate::jsc`]) transforms: unsolved, every segment answers 403.

use serde_json::Value;

use crate::models::{DirectAudio, DirectVideo};

/// `clientName` of the client.
pub(crate) const CLIENT_NAME: &str = "WEB_EMBEDDED_PLAYER";
/// `X-Youtube-Client-Name` of the client.
pub(crate) const CLIENT_NAME_ID: &str = "56";
/// Used when the embed page has no `INNERTUBE_CLIENT_VERSION`.
pub(crate) const CLIENT_VERSION: &str = "2.20260708.00.00";
/// The Safari User-Agent the client is requested with: its HLS manifest lists muxed variants
/// (video and audio), and playlists and segments are fetched with it too.
pub const EMBEDDED_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)";
/// The page the player claims to be embedded in: any non-YouTube URL (yt-dlp).
pub(crate) const EMBED_URL: &str = "https://www.reddit.com/";
/// `tv_samsung` (yt-dlp): `clientName`, `X-Youtube-Client-Name`, `clientVersion`, User-Agent.
pub(crate) const TV_CLIENT_NAME: &str = "TVHTML5";
pub(crate) const TV_CLIENT_NAME_ID: &str = "7";
pub(crate) const TV_CLIENT_VERSION: &str = "5.20260707";
/// Its direct formats are fetched with it too.
pub const TV_USER_AGENT: &str = "Mozilla/5.0 (SMART-TV; Linux; Tizen 2.4.0) AppleWebKit/538.1 (KHTML, like Gecko) Version/2.4.0 TV Safari/538.1";

/// What a `player` request needs from the embed page (`/embed/<id>?html5=1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmbedConfig {
  /// `INNERTUBE_CLIENT_VERSION`.
  pub client_version: Option<String>,
  /// `VISITOR_DATA`.
  pub visitor_data: Option<String>,
  /// Path of the player JS (`jsUrl`, or `PLAYER_JS_URL`), e.g. `/s/player/<id>/…/base.js`.
  pub player_js: Option<String>,
  /// `encryptedHostFlags`: without it, the answer is "video unavailable".
  pub host_flags: Option<String>,
  /// `LOGGED_IN`: whether the page took the session.
  pub logged_in: bool,
}

/// The embed page's ytcfg values.
pub fn parse_embed_config(html: &str) -> EmbedConfig {
  EmbedConfig {
    client_version: string_value(html, "INNERTUBE_CLIENT_VERSION"),
    visitor_data: string_value(html, "VISITOR_DATA"),
    player_js: string_value(html, "jsUrl")
      .or_else(|| string_value(html, "PLAYER_JS_URL"))
      .map(|path| path.replace("\\/", "/"))
      .filter(|path| path.starts_with("/s/player/")),
    host_flags: string_value(html, "encryptedHostFlags"),
    logged_in: html.contains("\"LOGGED_IN\":true"),
  }
}

/// `signatureTimestamp` of a player JS: five digits after `signatureTimestamp:` or `sts:`
/// (yt-dlp's `_extract_signature_timestamp`).
pub fn signature_timestamp(code: &str) -> Option<u32> {
  ["signatureTimestamp", "sts"].iter().find_map(|key| {
    code.match_indices(key).find_map(|(at, _)| {
      let rest = code[at + key.len()..].trim_start().strip_prefix(':')?;
      let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
      (digits.len() == 5).then(|| digits.parse().ok())?
    })
  })
}

/// The `n` challenge in a manifest URL's path (`…/n/<challenge>/…`).
pub fn n_challenge(manifest_url: &str) -> Option<&str> {
  let path = manifest_url.split('?').next()?;
  let start = path.find("/n/")? + "/n/".len();
  let challenge = path[start..].split('/').next()?;
  (!challenge.is_empty()).then_some(challenge)
}

/// `manifest_url` with its `n` challenge replaced by `solved`.
pub fn with_solved_n(manifest_url: &str, challenge: &str, solved: &str) -> String {
  manifest_url.replacen(&format!("/n/{challenge}/"), &format!("/n/{solved}/"), 1)
}

/// Seconds before the stream is served: the pre-roll ads of the answer, each up to when it can
/// be skipped (yt-dlp's `_get_available_at_timestamp`). Media requests answer 403 until then,
/// counted from the `player` answer. Asked with `adPlaybackContext.pyv`, the answer has none.
pub fn ad_wait(player: &Value) -> f64 {
  let mut renderers = Vec::new();
  for placement in player["adPlacements"].as_array().into_iter().flatten() {
    let renderer = &placement["adPlacementRenderer"];
    if renderer["config"]["adPlacementConfig"]["kind"] == "AD_PLACEMENT_KIND_START" {
      renderers.push(&renderer["renderer"]["instreamVideoAdRenderer"]);
    }
  }
  for slot in player["adSlots"].as_array().into_iter().flatten() {
    let slot = &slot["adSlotRenderer"];
    if slot["adSlotMetadata"]["triggerEvent"] != "SLOT_TRIGGER_EVENT_BEFORE_CONTENT" {
      continue;
    }
    let content = &slot["fulfillmentContent"]["fulfilledLayout"]["playerBytesAdLayoutRenderer"]["renderingContent"];
    renderers.push(&content["instreamVideoAdRenderer"]);
    for layout in content["playerBytesSequentialLayoutRenderer"]["sequentialLayouts"]
      .as_array()
      .into_iter()
      .flatten()
    {
      renderers.push(
        &layout["playerBytesAdLayoutRenderer"]["renderingContent"]["instreamVideoAdRenderer"],
      );
    }
  }
  renderers
    .into_iter()
    .filter(|r| r.is_object())
    .filter_map(|r| {
      let skip = r["skipOffsetMilliseconds"].as_f64().map(|ms| ms / 1000.);
      skip.or_else(|| ad_length(r["playerVars"].as_str()?))
    })
    .fold(0., |total, seconds| total + seconds)
}

/// A player answer's direct formats with a URL, as the menu offers them: per height from 1080p
/// up, VP9 (SDR before HDR), else AV1, highest first; and the audio, Opus before AAC, the
/// original track of a dubbed video. `None` without video or audio.
pub fn direct_formats(player: &Value) -> Option<(Vec<DirectVideo>, DirectAudio)> {
  let formats: Vec<&Value> = player["streamingData"]["adaptiveFormats"]
    .as_array()?
    .iter()
    .filter(|f| f["url"].is_string())
    .collect();
  let mime = |f: &Value| f["mimeType"].as_str().unwrap_or_default().to_owned();
  let codecs = |f: &Value| {
    let mime = mime(f);
    mime
      .split("codecs=\"")
      .nth(1)
      .map(|c| c.trim_end_matches('"').to_owned())
      .unwrap_or_default()
  };
  let mut videos: Vec<(u32, (u8, u64), DirectVideo)> = Vec::new();
  for f in formats.iter().filter(|f| mime(f).starts_with("video/")) {
    let (Some(width), Some(height)) = (f["width"].as_u64(), f["height"].as_u64()) else {
      continue;
    };
    // A vertical video's shorter side, as `hls::Variant::height`.
    let height = u32::try_from(width.min(height)).unwrap_or(0);
    let codecs = codecs(f);
    let hdr = f["colorInfo"]["transferCharacteristics"]
      .as_str()
      .is_some_and(|t| t.contains("SMPTEST2084") || t.contains("HLG"));
    let family: u8 = match (codecs.starts_with("vp09") || codecs.starts_with("vp9"), hdr) {
      (true, false) => 3,
      _ if codecs.starts_with("av01") && !hdr => 2,
      (true, true) => 1,
      _ => 0,
    };
    if height < 1080 || family == 0 {
      continue;
    }
    let rank = (family, f["bitrate"].as_u64().unwrap_or(0));
    let video = DirectVideo {
      height,
      frame_rate: f["fps"].as_u64().and_then(|r| u32::try_from(r).ok()),
      codecs,
      hdr,
      length: content_length(f),
      url: f["url"].as_str().unwrap_or_default().to_owned(),
    };
    match videos.iter_mut().find(|(h, _, _)| *h == height) {
      Some(slot) if slot.1 < rank => *slot = (height, rank, video),
      Some(_) => {}
      None => videos.push((height, rank, video)),
    }
  }
  videos.sort_by_key(|(height, _, _)| std::cmp::Reverse(*height));
  let audio = formats
    .iter()
    .filter(|f| mime(f).starts_with("audio/"))
    .filter(|f| f["audioTrack"].is_null() || f["audioTrack"]["audioIsDefault"] == true)
    .max_by_key(|f| (mime(f).contains("opus"), f["bitrate"].as_u64().unwrap_or(0)))
    .map(|f| DirectAudio {
      codecs: codecs(f),
      length: content_length(f),
      url: f["url"].as_str().unwrap_or_default().to_owned(),
    })?;
  let videos: Vec<DirectVideo> = videos.into_iter().map(|(_, _, v)| v).collect();
  (!videos.is_empty()).then_some((videos, audio))
}

/// `contentLength` of a format (a string in the answer).
fn content_length(format: &Value) -> Option<u64> {
  format["contentLength"].as_str()?.parse().ok()
}

/// The `n` parameter of a direct format's query.
pub fn query_n(url: &str) -> Option<&str> {
  let query = url.split_once('?')?.1;
  query.split('&').find_map(|pair| pair.strip_prefix("n="))
}

/// `url` with its query's `n` replaced by `solved`.
pub fn with_query_n(url: &str, solved: &str) -> String {
  let Some((base, query)) = url.split_once('?') else {
    return url.to_owned();
  };
  let query: Vec<String> = query
    .split('&')
    .map(|pair| match pair.strip_prefix("n=") {
      Some(_) => format!("n={solved}"),
      None => pair.to_owned(),
    })
    .collect();
  format!("{base}?{}", query.join("&"))
}

/// `length_seconds` of an ad's `playerVars` (a query string; the last one counts).
fn ad_length(player_vars: &str) -> Option<f64> {
  player_vars
    .rsplit('&')
    .find_map(|pair| pair.strip_prefix("length_seconds="))?
    .parse()
    .ok()
}

/// The string value of `"key":"…"` in a page's ytcfg, unescaped only for `\/`.
fn string_value(html: &str, key: &str) -> Option<String> {
  let pattern = format!("\"{key}\":\"");
  let start = html.find(&pattern)? + pattern.len();
  let end = html[start..].find('"')?;
  Some(html[start..start + end].to_owned()).filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn timestamp_of_either_key() {
    assert_eq!(
      signature_timestamp("a,signatureTimestamp:20767,b"),
      Some(20767)
    );
    assert_eq!(signature_timestamp("x={sts: 20767}"), Some(20767));
    // Not five digits: another field.
    assert_eq!(
      signature_timestamp("sts:123,signatureTimestamp:20767"),
      Some(20767)
    );
    assert_eq!(signature_timestamp("nothing here"), None);
  }

  #[test]
  fn challenge_in_the_path_only() {
    let url = "https://manifest.googlevideo.com/api/manifest/hls_variant/expire/1/n/AbC-12_x/id/9/file/index.m3u8?x=/n/q/";
    assert_eq!(n_challenge(url), Some("AbC-12_x"));
    assert_eq!(
      with_solved_n(url, "AbC-12_x", "solved"),
      "https://manifest.googlevideo.com/api/manifest/hls_variant/expire/1/n/solved/id/9/file/index.m3u8?x=/n/q/"
    );
    assert_eq!(n_challenge("https://example.com/a/b?n=1"), None);
  }

  #[test]
  fn waits_up_to_the_skip() {
    let skippable = json!({ "adPlacements": [{ "adPlacementRenderer": {
      "config": { "adPlacementConfig": { "kind": "AD_PLACEMENT_KIND_START" } },
      "renderer": { "instreamVideoAdRenderer": {
        "skipOffsetMilliseconds": 5000, "playerVars": "length_seconds=15" } } } }] });
    assert_eq!(ad_wait(&skippable), 5.);
    let unskippable = json!({ "adSlots": [{ "adSlotRenderer": {
      "adSlotMetadata": { "triggerEvent": "SLOT_TRIGGER_EVENT_BEFORE_CONTENT" },
      "fulfillmentContent": { "fulfilledLayout": { "playerBytesAdLayoutRenderer": {
        "renderingContent": { "instreamVideoAdRenderer": { "playerVars": "a=1&length_seconds=6" } } } } } } }] });
    assert_eq!(ad_wait(&unskippable), 6.);
    // A mid-roll does not delay the start.
    let mid = json!({ "adPlacements": [{ "adPlacementRenderer": {
      "config": { "adPlacementConfig": { "kind": "AD_PLACEMENT_KIND_MILLISECONDS" } },
      "renderer": { "instreamVideoAdRenderer": { "skipOffsetMilliseconds": 5000 } } } }] });
    assert_eq!(ad_wait(&mid), 0.);
    assert_eq!(ad_wait(&json!({})), 0.);
  }

  #[test]
  fn direct_formats_prefer_sdr_vp9() {
    let format = |height: u64, mime: &str, hdr: bool, bitrate: u64| {
      let mut f = json!({ "url": "https://r/videoplayback?n=x", "mimeType": mime,
        "width": height * 16 / 9, "height": height, "fps": 30, "bitrate": bitrate });
      if hdr {
        f["colorInfo"] =
          json!({ "transferCharacteristics": "COLOR_TRANSFER_CHARACTERISTICS_SMPTEST2084" });
      }
      f
    };
    let player = json!({ "streamingData": { "adaptiveFormats": [
      format(2160, r#"video/webm; codecs="vp09.02.51.10""#, true, 30),
      format(2160, r#"video/mp4; codecs="av01.0.13M.08""#, false, 20),
      format(1440, r#"video/webm; codecs="vp09.00.50.08""#, false, 10),
      format(1440, r#"video/mp4; codecs="av01.0.12M.08""#, false, 12),
      format(1080, r#"video/mp4; codecs="avc1.640028""#, false, 5),
      { "url": "https://r/a", "mimeType": r#"audio/mp4; codecs="mp4a.40.2""#, "bitrate": 130 },
      { "url": "https://r/o", "mimeType": r#"audio/webm; codecs="opus""#, "bitrate": 120 },
      { "url": "https://r/d", "mimeType": r#"audio/webm; codecs="opus""#, "bitrate": 160,
        "audioTrack": { "audioIsDefault": false } },
    ]}});
    let (videos, audio) = direct_formats(&player).unwrap();
    let labels: Vec<String> = videos.iter().map(DirectVideo::label).collect();
    // AV1 SDR before VP9 HDR; VP9 before AV1; H.264 never (the HLS ladder has it).
    assert_eq!(labels, ["2160p AV1", "1440p VP9"]);
    // Opus, the original track (a dub is not the default).
    assert_eq!(audio.url, "https://r/o");
  }

  #[test]
  fn config_values() {
    let html = r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"2.20261113.01.00","VISITOR_DATA":"Cgt2","LOGGED_IN":true,"jsUrl":"\/s\/player\/fad7e4e9\/player_embed_es6.vflset\/fr_FR\/base.js","WEB_PLAYER_CONTEXT_CONFIGS":{"WEB_PLAYER_CONTEXT_CONFIG_ID_EMBEDDED_PLAYER":{"encryptedHostFlags":"flags"}}});"#;
    let config = parse_embed_config(html);
    assert_eq!(config.client_version.as_deref(), Some("2.20261113.01.00"));
    assert_eq!(config.visitor_data.as_deref(), Some("Cgt2"));
    assert_eq!(
      config.player_js.as_deref(),
      Some("/s/player/fad7e4e9/player_embed_es6.vflset/fr_FR/base.js")
    );
    assert_eq!(config.host_flags.as_deref(), Some("flags"));
    assert!(config.logged_in);
    assert!(!parse_embed_config("{}").logged_in);
  }
}
