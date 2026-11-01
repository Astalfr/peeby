//! usher (`usher.ttvnw.net`), the HLS master playlists, and their variants.
//!
//! v2 paths (`/api/v2/channel/hls/`, `/vod/v2/`) as Streamlink 8.2.1; v1 still answers and is the
//! fallback. The two formats differ: v2 names its variants with `STABLE-VARIANT-ID`, `IVS-NAME`
//! and `IVS-GROUPS` (`landscape`, `portrait`), v1 with `EXT-X-MEDIA` video groups. Audio is muxed in
//! every variant (docs/twitch.md, "Checked live").

use reqwest::Url;
use serde_json::Value;

use crate::error::{Error, Result};
use crate::gql::{PlaybackToken, Target};

/// mpv and the proxy send it as `Referer` and `Origin` with every playlist and segment request.
pub const PLAYER_ORIGIN: &str = "https://player.twitch.tv";

/// Path version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
  /// `/api/v2/channel/hls/`, `/vod/v2/`.
  V2,
  /// `/api/channel/hls/`, `/vod/`.
  V1,
}

/// A video codec usher can be asked for (`supported_codecs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Codec {
  /// AV1 (`av01`).
  Av1,
  /// HEVC (`hev1`, `hvc1`).
  H265,
  /// AVC (`avc1`), what every channel sends.
  H264,
}

impl Codec {
  fn param(self) -> &'static str {
    match self {
      Self::Av1 => "av1",
      Self::H265 => "h265",
      Self::H264 => "h264",
    }
  }

  /// The video codec of a `CODECS` attribute; `None` for audio only.
  pub fn of(codecs: &str) -> Option<Self> {
    codecs.split(',').find_map(|c| {
      let c = c.trim();
      if c.starts_with("avc1") {
        Some(Self::H264)
      } else if c.starts_with("hev1") || c.starts_with("hvc1") {
        Some(Self::H265)
      } else if c.starts_with("av01") {
        Some(Self::Av1)
      } else {
        None
      }
    })
  }
}

/// The master playlist URL of `target`. It carries the token and its signature: never log it.
/// `nonce` is usher's `p`, a random number per request.
pub fn master_url(
  target: &Target,
  token: &PlaybackToken,
  codecs: &[Codec],
  version: Version,
  nonce: u32,
) -> Result<Url> {
  let (path, sig, tok) = match (target, version) {
    (Target::Live(login), Version::V2) => {
      (format!("api/v2/channel/hls/{login}.m3u8"), "sig", "token")
    }
    (Target::Live(login), Version::V1) => (format!("api/channel/hls/{login}.m3u8"), "sig", "token"),
    (Target::Vod(id), Version::V2) => (format!("vod/v2/{id}.m3u8"), "nauthsig", "nauth"),
    (Target::Vod(id), Version::V1) => (format!("vod/{id}.m3u8"), "nauthsig", "nauth"),
  };
  let codecs = if codecs.is_empty() {
    Codec::H264.param().to_owned()
  } else {
    codecs
      .iter()
      .map(|c| c.param())
      .collect::<Vec<_>>()
      .join(",")
  };
  let mut url =
    Url::parse(&format!("https://usher.ttvnw.net/{path}")).map_err(|e| Error::Parse {
      endpoint: "usher",
      detail: format!("master URL: {e}"),
    })?;
  url
    .query_pairs_mut()
    .append_pair("allow_source", "true")
    .append_pair("allow_audio_only", "true")
    .append_pair("fast_bread", "true")
    .append_pair("p", &nonce.to_string())
    .append_pair("platform", "web")
    .append_pair("playlist_include_framerate", "true")
    .append_pair("multigroup_video", "true")
    .append_pair("supported_codecs", &codecs)
    .append_pair(sig, &token.signature)
    .append_pair(tok, &token.value);
  Ok(url)
}

/// The error of a usher answer that is not HTTP 200. Its body repeats the request URL (token and
/// signature included) in `url`: only `error_code` is kept.
pub fn parse_error(status: u16, body: &str, target: &Target) -> Error {
  let errors: Value = serde_json::from_str(body).unwrap_or(Value::Null);
  let code = errors[0]["error_code"].as_str();
  match (status, code) {
    (403, Some("vod_manifest_restricted" | "unauthorized_entitlements")) => Error::SubscriberOnly,
    (404, _) => match target {
      Target::Live(_) => Error::Offline,
      Target::Vod(_) => Error::NotFound,
    },
    _ => Error::Status {
      status,
      endpoint: "usher",
      code: code.map(str::to_owned),
    },
  }
}

/// One variant of a master playlist.
#[derive(Clone, PartialEq)]
pub struct Variant {
  /// `STABLE-VARIANT-ID` (v2) or `VIDEO` group (v1), unique in the master: `1080p60`,
  /// `1080p60-portrait`, `audio_only`.
  pub id: String,
  /// Shown name: `IVS-NAME` (v2) or the `EXT-X-MEDIA` `NAME` (v1), e.g. `1080p60`.
  pub name: String,
  /// Height in pixels; `None` for audio only.
  pub height: Option<u32>,
  /// `FRAME-RATE` rounded.
  pub frame_rate: Option<u32>,
  /// `BANDWIDTH`.
  pub bandwidth: u64,
  /// `CODECS`.
  pub codecs: String,
  /// Video codec; `None` for audio only.
  pub codec: Option<Codec>,
  /// Part of the portrait ladder only (`IVS-GROUPS="portrait"`), of a channel that sends both.
  pub portrait: bool,
  /// Media playlist URL (signed, with the user's IP: never log it).
  pub url: String,
}

impl std::fmt::Debug for Variant {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{} {}", self.id, self.codecs)
  }
}

impl Variant {
  /// Audio only, no picture.
  pub fn audio_only(&self) -> bool {
    self.height.is_none()
  }
}

/// The variants of a master, in its order (best first).
pub fn variants(master: &str) -> Vec<Variant> {
  let mut names = Vec::new();
  let mut variants = Vec::new();
  let mut lines = master.lines().map(str::trim);
  while let Some(line) = lines.next() {
    if let Some(attrs) = line.strip_prefix("#EXT-X-MEDIA:") {
      // v1: the shown name of each video group.
      if attribute(attrs, "TYPE") == Some("VIDEO")
        && let (Some(group), Some(name)) = (attribute(attrs, "GROUP-ID"), attribute(attrs, "NAME"))
      {
        names.push((group.to_owned(), name.to_owned()));
      }
      continue;
    }
    let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") else {
      continue;
    };
    let Some(url) = lines.next().filter(|l| !l.starts_with('#')) else {
      break;
    };
    let id = attribute(attrs, "STABLE-VARIANT-ID").or_else(|| attribute(attrs, "VIDEO"));
    let Some(id) = id else {
      continue;
    };
    let name = attribute(attrs, "IVS-NAME")
      .map(str::to_owned)
      .or_else(|| names.iter().find(|(g, _)| g == id).map(|(_, n)| n.clone()))
      .unwrap_or_else(|| id.to_owned());
    let height = attribute(attrs, "RESOLUTION")
      .and_then(|r| r.split_once('x'))
      .and_then(|(_, h)| h.parse().ok());
    let codecs = attribute(attrs, "CODECS").unwrap_or_default().to_owned();
    let portrait = attribute(attrs, "IVS-GROUPS")
      .is_some_and(|groups| !groups.split(',').any(|g| g == "landscape"));
    variants.push(Variant {
      id: id.to_owned(),
      name,
      height,
      frame_rate: attribute(attrs, "FRAME-RATE")
        .and_then(|f| f.parse::<f64>().ok())
        .map(|f| f.round() as u32),
      bandwidth: attribute(attrs, "BANDWIDTH")
        .and_then(|b| b.parse().ok())
        .unwrap_or(0),
      codec: Codec::of(&codecs),
      codecs,
      portrait,
      url: url.to_owned(),
    });
  }
  variants
}

/// Value of `name` in an HLS attribute list, quotes removed.
pub(crate) fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
  let mut rest = attrs;
  loop {
    let (key, after) = rest.split_once('=')?;
    let (value, next) = match after.strip_prefix('"') {
      Some(quoted) => {
        let (value, tail) = quoted.split_once('"')?;
        (value, tail.strip_prefix(',').unwrap_or(tail))
      }
      None => after.split_once(',').unwrap_or((after, "")),
    };
    if key == name {
      return Some(value);
    }
    if next.is_empty() {
      return None;
    }
    rest = next;
  }
}
