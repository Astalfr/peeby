//! Anonymous playback resolution, as measured on 2026-11-01 (docs/twitch.md, "Checked live"):
//! the raw `PlaybackAccessToken` query (no persisted hash) with the web Client-ID and
//! `playerType=embed` (no pre-roll, unlike `site`), then the usher v2 master playlist.
//!
//! The token's `value` carries the user's IP: it and every usher or CDN URL stay out of the output.

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// Client-ID of twitch.tv's own web player.
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
/// mpv sends these with every playlist and segment request.
pub const REFERER: &str = "https://player.twitch.tv";

const TOKEN_QUERY: &str = "query PlaybackAccessToken_Template($login: String!, $isLive: Boolean!, \
  $vodID: ID!, $isVod: Boolean!, $playerType: String!, $platform: String!) { \
  streamPlaybackAccessToken(channelName: $login, params: {platform: $platform, playerBackend: \
  \"mediaplayer\", playerType: $playerType}) @include(if: $isLive) { value signature } \
  videoPlaybackAccessToken(id: $vodID, params: {platform: $platform, playerBackend: \"mediaplayer\", \
  playerType: $playerType}) @include(if: $isVod) { value signature } }";

/// What to play.
pub enum Target {
  /// A channel's live, by login.
  Live(String),
  /// A VOD, by id.
  Vod(String),
}

/// The variant handed to mpv.
pub struct Stream {
  /// Media playlist URL (signed, with the user's IP: never print it).
  pub url: String,
  /// `STABLE-VARIANT-ID` (v2) or `VIDEO` group (v1), e.g. `1080p60`.
  pub name: String,
  /// `CODECS` of the variant.
  pub codecs: String,
  /// How many variants the master listed, and how many of them are landscape.
  pub variants: (usize, usize),
}

pub struct Resolver {
  http: reqwest::blocking::Client,
  player_type: String,
}

impl Resolver {
  pub fn new() -> Result<Self, String> {
    let http = reqwest::blocking::Client::builder()
      .build()
      .map_err(|e| e.to_string())?;
    let player_type = std::env::var("PEEBY_PLAYER_TYPE").unwrap_or_else(|_| "embed".into());
    Ok(Self { http, player_type })
  }

  fn gql(&self, body: &Value) -> Result<Value, String> {
    self
      .http
      .post("https://gql.twitch.tv/gql")
      .header("Client-ID", WEB_CLIENT_ID)
      .json(body)
      .send()
      .and_then(|r| r.error_for_status())
      .and_then(|r| r.json())
      .map_err(|e| format!("gql: {}", e.without_url()))
  }

  /// The top live channel, when none is given.
  pub fn top_live(&self) -> Result<String, String> {
    let answer = self.gql(&json!({
      "query": "query { streams(first: 1) { edges { node { broadcaster { login } } } } }"
    }))?;
    answer["data"]["streams"]["edges"][0]["node"]["broadcaster"]["login"]
      .as_str()
      .map(str::to_owned)
      .ok_or_else(|| "no live stream".into())
  }

  /// The latest VOD of `login`.
  pub fn latest_vod(&self, login: &str) -> Result<String, String> {
    let answer = self.gql(&json!({
      "query": "query($login: String!) { user(login: $login) { videos(first: 1) { edges { node { id } } } } }",
      "variables": { "login": login },
    }))?;
    answer["data"]["user"]["videos"]["edges"][0]["node"]["id"]
      .as_str()
      .map(str::to_owned)
      .ok_or_else(|| format!("{login} has no VOD"))
  }

  /// Token then master; prints the time of each.
  pub fn resolve(&self, target: &Target, fixtures: Option<&Path>) -> Result<Stream, String> {
    let started = Instant::now();
    let (live, login, vod) = match target {
      Target::Live(login) => (true, login.as_str(), ""),
      Target::Vod(id) => (false, "", id.as_str()),
    };
    let answer = self.gql(&json!({
      "operationName": "PlaybackAccessToken_Template",
      "query": TOKEN_QUERY,
      "variables": {
        "login": login, "isLive": live, "vodID": vod, "isVod": !live,
        "playerType": self.player_type, "platform": "web",
      },
    }))?;
    let key = if live {
      "streamPlaybackAccessToken"
    } else {
      "videoPlaybackAccessToken"
    };
    let token = &answer["data"][key];
    let (Some(value), Some(signature)) = (token["value"].as_str(), token["signature"].as_str())
    else {
      let errors = answer["errors"].to_string();
      return Err(format!("no playback token (errors: {errors})"));
    };
    eprintln!(
      "token ({}) after {:.2} s",
      self.player_type,
      started.elapsed().as_secs_f64()
    );
    if let Some(dir) = fixtures {
      let name = if live {
        "gql_playback_token_live.json"
      } else {
        "gql_playback_token_vod.json"
      };
      save_redacted_token(&answer, key, &dir.join(name))?;
    }

    let nonce = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map_or(0, |d| d.subsec_nanos() % 10_000_000);
    // `PEEBY_USHER=v1`: the paths yt-dlp still uses.
    let v1 = std::env::var("PEEBY_USHER").as_deref() == Ok("v1");
    let (path, token_names) = match (live, v1) {
      (true, false) => (format!("api/v2/channel/hls/{login}.m3u8"), ("sig", "token")),
      (true, true) => (format!("api/channel/hls/{login}.m3u8"), ("sig", "token")),
      (false, false) => (format!("vod/v2/{vod}.m3u8"), ("nauthsig", "nauth")),
      (false, true) => (format!("vod/{vod}.m3u8"), ("nauthsig", "nauth")),
    };
    let nonce = nonce.to_string();
    let codecs = codecs();
    let url = reqwest::Url::parse_with_params(
      &format!("https://usher.ttvnw.net/{path}"),
      &[
        ("allow_source", "true"),
        ("allow_audio_only", "true"),
        ("fast_bread", "true"),
        ("p", nonce.as_str()),
        ("platform", "web"),
        ("playlist_include_framerate", "true"),
        ("multigroup_video", "true"),
        ("supported_codecs", codecs.as_str()),
        (token_names.0, signature),
        (token_names.1, value),
      ],
    )
    .map_err(|e| format!("usher URL: {e}"))?;
    let response = self
      .http
      .get(url)
      .header("Referer", REFERER)
      .header("Origin", REFERER)
      .send()
      .map_err(|e| format!("usher: {}", e.without_url()))?;
    let status = response.status();
    let master = response
      .text()
      .map_err(|e| format!("usher: {}", e.without_url()))?;
    if !status.is_success() {
      // The body is a JSON array of errors whose `url` repeats the request, token and signature
      // included: keep only the code and the message.
      let errors: Value = serde_json::from_str(&master).unwrap_or(Value::Null);
      let first = &errors[0];
      if let (Some(dir), Some(list)) = (fixtures, errors.as_array()) {
        let redacted: Vec<Value> = list
          .iter()
          .cloned()
          .map(|mut e| {
            e["url"] = Value::String("REDACTED".into());
            e
          })
          .collect();
        let path = dir.join(format!(
          "usher_{}.json",
          if live { "live_error" } else { "vod_restricted" }
        ));
        let text = serde_json::to_string_pretty(&redacted).map_err(|e| e.to_string())?;
        std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("saved {}", path.display());
      }
      return Err(format!(
        "usher: HTTP {status}: {} ({})",
        first["error_code"].as_str().unwrap_or("?"),
        first["error"].as_str().unwrap_or("?"),
      ));
    }
    eprintln!("master after {:.2} s", started.elapsed().as_secs_f64());
    best_variant(&master).ok_or_else(|| "no landscape variant in the master".into())
  }
}

/// `supported_codecs`: `PEEBY_CODECS` (e.g. `av1,h265,h264`), default h264 only.
fn codecs() -> String {
  std::env::var("PEEBY_CODECS").unwrap_or_else(|_| "h264".into())
}

/// The first landscape variant with a video (the master lists the best first). v2 masters name
/// variants with `STABLE-VARIANT-ID` and `IVS-GROUPS`, v1 ones with a `VIDEO` group.
fn best_variant(master: &str) -> Option<Stream> {
  let mut lines = master.lines();
  let mut found = None;
  let (mut total, mut landscape) = (0, 0);
  while let Some(line) = lines.next() {
    let Some(attrs) = line.strip_prefix("#EXT-X-STREAM-INF:") else {
      continue;
    };
    let url = lines.next()?;
    total += 1;
    let name = attribute(attrs, "STABLE-VARIANT-ID")
      .or_else(|| attribute(attrs, "VIDEO"))
      .unwrap_or("?");
    // `PEEBY_VARIANT=<id>` picks a variant, portrait ones too (e.g. an HEVC one).
    let wanted = std::env::var("PEEBY_VARIANT").ok();
    let groups = attribute(attrs, "IVS-GROUPS");
    let portrait = groups.is_some_and(|g| !g.split(',').any(|g| g == "landscape"));
    if !portrait {
      landscape += 1;
    }
    let pick = match &wanted {
      Some(id) => id == name,
      None => !portrait && attribute(attrs, "RESOLUTION").is_some(),
    };
    if found.is_none() && pick {
      found = Some(Stream {
        url: url.to_owned(),
        name: name.to_owned(),
        codecs: attribute(attrs, "CODECS").unwrap_or("?").to_owned(),
        variants: (0, 0),
      });
    }
  }
  found.map(|stream| Stream {
    variants: (total, landscape),
    ..stream
  })
}

/// Value of `name` in an HLS attribute list, quotes removed.
fn attribute<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
  let mut rest = attrs;
  loop {
    let (key, after) = rest.split_once('=')?;
    let (value, next) = if let Some(quoted) = after.strip_prefix('"') {
      let (value, tail) = quoted.split_once('"')?;
      (value, tail.strip_prefix(',').unwrap_or(tail))
    } else {
      after.split_once(',').unwrap_or((after, ""))
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

/// Saves the GQL answer with the token's `value` (JSON with the user's IP and ids) and
/// `signature` replaced. The `value`'s keys are kept, their values blanked, so the shape stays known.
fn save_redacted_token(answer: &Value, key: &str, path: &Path) -> Result<(), String> {
  let mut answer = answer.clone();
  let token = &mut answer["data"][key];
  let keys: Vec<String> = token["value"]
    .as_str()
    .and_then(|v| serde_json::from_str::<Value>(v).ok())
    .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
    .unwrap_or_default();
  let shape: serde_json::Map<String, Value> = keys
    .into_iter()
    .map(|k| (k, Value::String("REDACTED".into())))
    .collect();
  token["value"] = Value::String(Value::Object(shape).to_string());
  token["signature"] = Value::String("REDACTED".into());
  if let Some(extensions) = answer.get_mut("extensions") {
    // `durationMilliseconds` and the operation name are harmless; a request id is not needed.
    if let Some(map) = extensions.as_object_mut() {
      map.remove("requestID");
    }
  }
  let text = serde_json::to_string_pretty(&answer).map_err(|e| e.to_string())?;
  std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
  eprintln!("saved {}", path.display());
  Ok(())
}
