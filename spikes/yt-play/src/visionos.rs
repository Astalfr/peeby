//! Fast stream resolution without yt-dlp: the InnerTube `player` endpoint with the VISIONOS
//! client returns a server-signed HLS master playlist. There is no JS challenge (no `n`/`sig` to
//! solve) and no `available_at` delay on its variants. Same client context as yt-dlp
//! 2026.10.26 (`yt_dlp/extractor/youtube/_base.py`).

use std::time::Instant;

use serde_json::{Value, json};

/// Safari UA of the visionOS client; segments are fetched with it too.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
const WEB_CLIENT_VERSION: &str = "2.20261029.11.00";

/// The HLS variant chosen for playback. YouTube's variants are video only: the audio is a
/// separate rendition (`EXT-X-MEDIA` group named by the variant's `AUDIO` attribute).
pub struct Stream {
  /// Video media playlist URL.
  pub url: String,
  /// Audio media playlist URL.
  pub audio_url: Option<String>,
  /// `RESOLUTION` of the variant, e.g. `1920x1080`.
  pub resolution: String,
  /// `CODECS` of the variant.
  pub codecs: String,
}

/// HTTP client with a session visitorData: without one, most videos answer `LOGIN_REQUIRED`.
pub struct Resolver {
  http: reqwest::blocking::Client,
  visitor_data: String,
}

impl Resolver {
  /// Gets a visitorData from one anonymous browse request (done once per session, at startup).
  pub fn new() -> Result<Self, String> {
    let http = reqwest::blocking::Client::builder()
      .user_agent(USER_AGENT)
      .build()
      .map_err(|e| e.to_string())?;
    let body = json!({
      "context": { "client": { "clientName": "WEB", "clientVersion": WEB_CLIENT_VERSION } },
      "browseId": "FEwhat_to_watch",
    });
    let response: Value = http
      .post("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false")
      .json(&body)
      .send()
      .and_then(|r| r.json())
      .map_err(|e| format!("visitorData: {e}"))?;
    let visitor_data = response["responseContext"]["visitorData"]
      .as_str()
      .ok_or("visitorData missing from the browse response")?
      .to_owned();
    Ok(Self { http, visitor_data })
  }

  /// Best muxed HLS variant up to `max_height`.
  pub fn resolve(&self, video_id: &str, max_height: u32) -> Result<Stream, String> {
    let started = Instant::now();
    let body = json!({
      "context": { "client": {
        "clientName": "VISIONOS",
        "clientVersion": "1.02",
        "deviceMake": "Apple",
        "deviceModel": "RealityDevice17,1",
        "userAgent": USER_AGENT,
        "osName": "visionOS",
        "osVersion": "26.5.23O471",
        "visitorData": self.visitor_data,
      }},
      "videoId": video_id,
      "contentCheckOk": true,
      "racyCheckOk": true,
    });
    let player: Value = self
      .http
      .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
      .header("X-Youtube-Client-Name", "101")
      .header("X-Youtube-Client-Version", "1.02")
      .header("X-Goog-Visitor-Id", &self.visitor_data)
      .header("Origin", "https://www.youtube.com")
      .json(&body)
      .send()
      .and_then(|r| r.json())
      .map_err(|e| format!("player: {e}"))?;
    let status = &player["playabilityStatus"];
    if status["status"] != "OK" {
      return Err(format!(
        "{}: {}",
        status["status"].as_str().unwrap_or("?"),
        status["reason"].as_str().unwrap_or("")
      ));
    }
    let master_url = player["streamingData"]["hlsManifestUrl"]
      .as_str()
      .ok_or("no hlsManifestUrl (live without HLS, or client blocked)")?;
    let player_time = started.elapsed();

    let master = self
      .http
      .get(master_url)
      .send()
      .and_then(|r| r.text())
      .map_err(|e| format!("HLS master: {e}"))?;
    let stream = best_variant(&master, max_height).ok_or("no HLS variant")?;
    tracing_like(&format!(
      "visionos: player {:.2} s + master {:.2} s",
      player_time.as_secs_f64(),
      (started.elapsed() - player_time).as_secs_f64()
    ));
    Ok(stream)
  }
}

/// Highest variant up to `max_height`; at equal height VP9 (better quality per bit) over AVC.
fn best_variant(master: &str, max_height: u32) -> Option<Stream> {
  let mut lines = master.lines();
  let mut best: Option<((u32, bool, u64), Stream)> = None;
  while let Some(line) = lines.next() {
    let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") else {
      continue;
    };
    let Some(url) = lines.next() else { break };
    let resolution = attribute(attrs, "RESOLUTION").unwrap_or_default();
    let height = resolution
      .split_once('x')
      .and_then(|(_, h)| h.parse().ok())
      .unwrap_or(0);
    let bandwidth = attribute(attrs, "BANDWIDTH")
      .and_then(|b| b.parse().ok())
      .unwrap_or(0);
    if height > max_height {
      continue;
    }
    let codecs = attribute(attrs, "CODECS").unwrap_or_default();
    let rank = (height, codecs.starts_with("vp09"), bandwidth);
    if best.as_ref().is_none_or(|(best_rank, _)| rank > *best_rank) {
      let audio_url = attribute(attrs, "AUDIO").and_then(|group| audio_rendition(master, &group));
      best = Some((
        rank,
        Stream {
          url: url.to_owned(),
          audio_url,
          resolution,
          codecs,
        },
      ));
    }
  }
  best.map(|(_, stream)| stream)
}

/// URI of the audio rendition of `group`.
fn audio_rendition(master: &str, group: &str) -> Option<String> {
  master
    .lines()
    .filter_map(|line| line.strip_prefix("#EXT-X-MEDIA:"))
    .filter(|attrs| attribute(attrs, "TYPE").as_deref() == Some("AUDIO"))
    .find(|attrs| attribute(attrs, "GROUP-ID").as_deref() == Some(group))
    .and_then(|attrs| attribute(attrs, "URI"))
}

/// Value of `name` in an HLS attribute list (`A=1,B="x,y"`).
fn attribute(attrs: &str, name: &str) -> Option<String> {
  let key = format!("{name}=");
  // The name must start the list or follow a comma (`URI` is not `GROUP-URI`); commas inside
  // quoted values are skipped by checking the character before a match only.
  let start = attrs
    .match_indices(&key)
    .find(|(i, _)| *i == 0 || attrs[..*i].ends_with(','))?
    .0
    + key.len();
  let rest = &attrs[start..];
  if let Some(quoted) = rest.strip_prefix('"') {
    Some(quoted[..quoted.find('"')?].to_owned())
  } else {
    Some(rest.split(',').next()?.to_owned())
  }
}

// The spike reports on the console on purpose.
#[allow(clippy::print_stderr)]
fn tracing_like(message: &str) {
  eprintln!("{message}");
}
