use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::auth::{Cookies, ORIGIN};
use crate::error::{Error, Result};
use crate::hls;
use crate::models::{Feed, Stream, WatchPage};
use crate::parse::{parse_feed, parse_watch};

/// WEB client version used until [`Client::refresh_client_version`] reads the current one from
/// the page's ytcfg. YouTube ships a new one every week; old ones keep working for months.
const WEB_CLIENT_VERSION: &str = "2.20261029.11.00";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// VISIONOS client, values from yt-dlp 2026.10.26 (`yt_dlp/extractor/youtube/_base.py`), the
/// source of truth for them. It returns a server-signed HLS master playlist: no JS challenge,
/// no `available_at` delay (ADR 0003).
const VISIONOS_CLIENT_NAME_ID: &str = "101";
const VISIONOS_CLIENT_VERSION: &str = "1.02";
/// Safari UA of the VISIONOS client; HLS playlists and segments are fetched with it too.
pub const VISIONOS_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

#[derive(Default)]
struct State {
  /// visitorData of the WEB session (`X-Goog-Visitor-Id`), from the first response or ytcfg.
  visitor_data: Option<String>,
  /// visitorData for VISIONOS, always from an anonymous request: most videos answer
  /// `LOGIN_REQUIRED` without one, and one tied to the signed-in session does not help.
  resolver_visitor: Option<String>,
  web_version: Option<String>,
}

/// InnerTube client. Cheap to clone; clones share the session state.
#[derive(Clone)]
pub struct Client {
  http: reqwest::Client,
  cookies: Option<Arc<Cookies>>,
  language: Arc<(String, String)>,
  state: Arc<Mutex<State>>,
}

impl Client {
  /// A client, signed in when `cookies` are given. `hl`/`gl` pick the language and region of
  /// texts and recommendations (e.g. `fr`, `FR`).
  pub fn new(cookies: Option<Cookies>, hl: &str, gl: &str) -> Result<Self> {
    let http = reqwest::Client::builder()
      .user_agent(WEB_USER_AGENT)
      .build()?;
    Ok(Self {
      http,
      cookies: cookies.map(Arc::new),
      language: Arc::new((hl.to_owned(), gl.to_owned())),
      state: Arc::default(),
    })
  }

  /// Whether requests carry session cookies.
  pub fn signed_in(&self) -> bool {
    self.cookies.is_some()
  }

  /// Home recommendations. Signed out, YouTube returns no videos, only a
  /// [`Feed::message`]. Signed in but answered as signed out: [`Error::NotLoggedIn`].
  pub async fn home(&self) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "browseId": "FEwhat_to_watch" }), true)
      .await?;
    self.feed(&response)
  }

  /// Next page of a feed.
  pub async fn continuation(&self, token: &str) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "continuation": token }), true)
      .await?;
    self.feed(&response)
  }

  /// Watch page: metadata, related videos, comments token.
  pub async fn watch(&self, video_id: &str) -> Result<WatchPage> {
    let response = self
      .web("next", json!({ "videoId": video_id }), true)
      .await?;
    Ok(parse_watch(video_id, &response))
  }

  /// Best HLS stream up to `max_height` pixels. Takes about 0.2 s; the URLs stay valid for
  /// 6 hours. Retries once with a fresh visitorData on `LOGIN_REQUIRED`.
  pub async fn stream(&self, video_id: &str, max_height: u32) -> Result<Stream> {
    match self.visionos_stream(video_id, max_height).await {
      Err(Error::Unplayable { status, .. }) if status == "LOGIN_REQUIRED" => {
        tracing::debug!("VISIONOS: LOGIN_REQUIRED, retrying with a new visitorData");
        self.state().resolver_visitor = None;
        self.visionos_stream(video_id, max_height).await
      }
      result => result,
    }
  }

  /// Reads the current WEB client version and a visitorData from the home page's ytcfg.
  /// Call it once in the background at startup.
  pub async fn refresh_client_version(&self) -> Result<()> {
    let mut request = self.http.get(ORIGIN);
    if let Some(cookies) = &self.cookies {
      request = request.header("Cookie", cookies.header());
    }
    let response = request.send().await?;
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "ytcfg",
      });
    }
    let html = response.text().await?;
    let version = ytcfg_string(&html, "INNERTUBE_CLIENT_VERSION").ok_or(Error::Parse {
      endpoint: "ytcfg",
      detail: "INNERTUBE_CLIENT_VERSION missing".into(),
    })?;
    let mut state = self.state();
    tracing::debug!("WEB client version {version}");
    state.web_version = Some(version);
    if state.visitor_data.is_none() {
      state.visitor_data = ytcfg_string(&html, "VISITOR_DATA");
    }
    Ok(())
  }

  /// Downloads an image (thumbnails, avatars).
  pub async fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
    let response = self.http.get(url).send().await?;
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "image",
      });
    }
    Ok(response.bytes().await?.to_vec())
  }

  fn state(&self) -> std::sync::MutexGuard<'_, State> {
    // A poisoned lock only means a panic elsewhere; the state stays usable.
    self
      .state
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
  }

  fn feed(&self, response: &Value) -> Result<Feed> {
    let feed = parse_feed(response);
    if self.cookies.is_some() && !feed.logged_in {
      return Err(Error::NotLoggedIn);
    }
    Ok(feed)
  }

  /// A WEB client request, with the session cookies when `signed` and cookies are set.
  async fn web(&self, endpoint: &'static str, mut body: Value, signed: bool) -> Result<Value> {
    let (version, visitor) = {
      let state = self.state();
      (
        state
          .web_version
          .clone()
          .unwrap_or_else(|| WEB_CLIENT_VERSION.to_owned()),
        state.visitor_data.clone(),
      )
    };
    let (hl, gl) = &*self.language;
    body["context"] = json!({ "client": {
      "clientName": "WEB",
      "clientVersion": version,
      "hl": hl,
      "gl": gl,
    }});
    let mut request = self
      .http
      .post(format!("{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"))
      .header("Origin", ORIGIN)
      .header("X-Origin", ORIGIN)
      .header("X-Youtube-Client-Name", "1")
      .header("X-Youtube-Client-Version", &version)
      .json(&body);
    let cookies = self.cookies.as_ref().filter(|_| signed);
    if let (Some(visitor), true) = (&visitor, signed) {
      request = request.header("X-Goog-Visitor-Id", visitor);
    }
    if let Some(cookies) = cookies {
      request = request
        .header("Cookie", cookies.header())
        .header("Authorization", cookies.authorization())
        .header("X-Goog-AuthUser", "0");
    }
    let response = self.send(endpoint, request).await?;
    if signed && let Some(visitor) = response["responseContext"]["visitorData"].as_str() {
      self
        .state()
        .visitor_data
        .get_or_insert_with(|| visitor.to_owned());
    }
    Ok(response)
  }

  async fn send(&self, endpoint: &'static str, request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request.send().await?;
    let status = response.status();
    if !status.is_success() {
      return Err(Error::Status {
        status: status.as_u16(),
        endpoint,
      });
    }
    Ok(response.json().await?)
  }

  /// visitorData for VISIONOS. Signed out, the session's one is anonymous and reused.
  async fn resolver_visitor(&self) -> Result<String> {
    {
      let mut state = self.state();
      if state.resolver_visitor.is_none() && self.cookies.is_none() {
        state.resolver_visitor = state.visitor_data.clone();
      }
      if let Some(visitor) = state.resolver_visitor.clone() {
        return Ok(visitor);
      }
    }
    // Any anonymous request returns one; this browse is the cheapest known.
    let response = self
      .web("browse", json!({ "browseId": "FEwhat_to_watch" }), false)
      .await?;
    let visitor = response["responseContext"]["visitorData"]
      .as_str()
      .map(str::to_owned)
      .ok_or(Error::Parse {
        endpoint: "browse",
        detail: "visitorData missing".into(),
      })?;
    self.state().resolver_visitor = Some(visitor.clone());
    Ok(visitor)
  }

  async fn visionos_stream(&self, video_id: &str, max_height: u32) -> Result<Stream> {
    let visitor = self.resolver_visitor().await?;
    let (hl, gl) = &*self.language;
    let body = json!({
      "context": { "client": {
        "clientName": "VISIONOS",
        "clientVersion": VISIONOS_CLIENT_VERSION,
        "deviceMake": "Apple",
        "deviceModel": "RealityDevice17,1",
        "userAgent": VISIONOS_USER_AGENT,
        "osName": "visionOS",
        "osVersion": "26.5.23O471",
        "hl": hl,
        "gl": gl,
        "visitorData": visitor,
      }},
      "videoId": video_id,
      "contentCheckOk": true,
      "racyCheckOk": true,
    });
    let request = self
      .http
      .post(format!("{ORIGIN}/youtubei/v1/player?prettyPrint=false"))
      .header("User-Agent", VISIONOS_USER_AGENT)
      .header("Origin", ORIGIN)
      .header("X-Youtube-Client-Name", VISIONOS_CLIENT_NAME_ID)
      .header("X-Youtube-Client-Version", VISIONOS_CLIENT_VERSION)
      .header("X-Goog-Visitor-Id", &visitor)
      .json(&body);
    let player = self.send("player", request).await?;
    let master_url = hls_manifest_url(&player)?;

    let response = self
      .http
      .get(master_url)
      .header("User-Agent", VISIONOS_USER_AGENT)
      .send()
      .await?;
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "hls master",
      });
    }
    let master = response.text().await?;
    let variant = hls::best_variant(&master, max_height).ok_or(Error::Parse {
      endpoint: "hls master",
      detail: "no variant".into(),
    })?;
    Ok(Stream {
      video_url: variant.url,
      audio_url: variant.audio_url,
      resolution: variant.resolution,
      codecs: variant.codecs,
      user_agent: VISIONOS_USER_AGENT,
    })
  }
}

/// `streamingData.hlsManifestUrl` of a player response, or why there is none.
pub fn hls_manifest_url(player: &Value) -> Result<&str> {
  let status = &player["playabilityStatus"];
  if status["status"] != "OK" {
    return Err(Error::Unplayable {
      status: status["status"].as_str().unwrap_or("?").to_owned(),
      reason: status["reason"].as_str().unwrap_or_default().to_owned(),
    });
  }
  player["streamingData"]["hlsManifestUrl"]
    .as_str()
    .ok_or(Error::Parse {
      endpoint: "player",
      detail: "no hlsManifestUrl".into(),
    })
}

/// `"KEY":"value"` in the ytcfg of a page.
fn ytcfg_string(html: &str, key: &str) -> Option<String> {
  let pattern = format!("\"{key}\":\"");
  let start = html.find(&pattern)? + pattern.len();
  let end = html[start..].find('"')?;
  Some(html[start..start + end].to_owned())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ytcfg_value() {
    let html = r#"ytcfg.set({"INNERTUBE_CLIENT_VERSION":"2.20261029.11.00","X":1});"#;
    assert_eq!(
      ytcfg_string(html, "INNERTUBE_CLIENT_VERSION").as_deref(),
      Some("2.20261029.11.00")
    );
    assert_eq!(ytcfg_string(html, "VISITOR_DATA"), None);
  }
}
