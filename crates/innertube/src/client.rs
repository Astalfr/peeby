use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use crate::auth::{Cookies, ORIGIN};
use crate::error::{Error, Result};
use crate::hls;
use crate::models::{
  Channel, CommentPage, Feed, LikeStatus, NotificationPage, Playlist, PlaylistChoice,
  PlaylistSummary, Stream, Votes, WatchPage,
};
use crate::parse::{
  parse_captions, parse_channels, parse_comments, parse_feed, parse_notifications, parse_playlist,
  parse_playlist_choices, parse_playlist_page, parse_playlists, parse_storyboard,
  parse_unseen_count, parse_watch, tracking_param,
};

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

/// Session cookies and the file the rotated values are written back to.
struct Session {
  cookies: Mutex<Cookies>,
  file: Option<PathBuf>,
}

/// InnerTube client. Cheap to clone; clones share the session state.
#[derive(Clone)]
pub struct Client {
  http: reqwest::Client,
  session: Option<Arc<Session>>,
  language: Arc<(String, String)>,
  state: Arc<Mutex<State>>,
}

impl Client {
  /// A client, signed in when `cookies` are given. `hl`/`gl` pick the language and region of
  /// texts and recommendations (e.g. `fr`, `FR`). YouTube rotates some cookies in its
  /// responses: when `cookie_file` is given, the updated jar is written back to it, otherwise
  /// the session is invalidated after a while.
  pub fn new(
    cookies: Option<Cookies>,
    cookie_file: Option<PathBuf>,
    hl: &str,
    gl: &str,
  ) -> Result<Self> {
    let http = reqwest::Client::builder()
      .user_agent(WEB_USER_AGENT)
      .build()?;
    Ok(Self {
      http,
      session: cookies.map(|cookies| {
        Arc::new(Session {
          cookies: Mutex::new(cookies),
          file: cookie_file,
        })
      }),
      language: Arc::new((hl.to_owned(), gl.to_owned())),
      state: Arc::default(),
    })
  }

  /// Whether requests carry session cookies.
  pub fn signed_in(&self) -> bool {
    self.session.is_some()
  }

  /// Home recommendations. Signed out, YouTube returns no videos, only a
  /// [`Feed::message`]. Signed in but answered as signed out: [`Error::NotLoggedIn`].
  pub async fn home(&self) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "browseId": "FEwhat_to_watch" }), true)
      .await?;
    self.feed(&response)
  }

  /// Latest videos of the subscribed channels. Needs cookies.
  pub async fn subscriptions(&self) -> Result<Feed> {
    self.require_session()?;
    let response = self
      .web("browse", json!({ "browseId": "FEsubscriptions" }), true)
      .await?;
    self.feed(&response)
  }

  /// All subscribed channels, in YouTube's order. Needs cookies.
  pub async fn subscribed_channels(&self) -> Result<Vec<Channel>> {
    self.require_session()?;
    let response = self
      .web("browse", json!({ "browseId": "FEchannels" }), true)
      .await?;
    if parse_feed(&response).logged_in {
      Ok(parse_channels(&response))
    } else {
      Err(Error::NotLoggedIn)
    }
  }

  /// The user's playlists, "Watch later" and "Liked videos" first. Needs cookies.
  pub async fn playlists(&self) -> Result<Vec<PlaylistSummary>> {
    self.require_session()?;
    let response = self
      .web(
        "browse",
        json!({ "browseId": "FEplaylist_aggregation" }),
        true,
      )
      .await?;
    if tracking_param(&response, "logged_in").as_deref() == Some("1") {
      Ok(parse_playlists(&response))
    } else {
      Err(Error::NotLoggedIn)
    }
  }

  /// A playlist's header and first videos (100 at most). Private playlists, "Watch later" and
  /// "Liked videos" need cookies.
  pub async fn playlist(&self, id: &str) -> Result<Playlist> {
    let response = self
      .web("browse", json!({ "browseId": format!("VL{id}") }), true)
      .await?;
    self.check_session(&response)?;
    Ok(parse_playlist(id, &response))
  }

  /// The next videos of a playlist, from [`Playlist::continuation`] or a previous page.
  pub async fn playlist_more(&self, token: &str) -> Result<Feed> {
    let response = self
      .web("browse", json!({ "continuation": token }), true)
      .await?;
    self.check_session(&response)?;
    Ok(parse_playlist_page(&response))
  }

  /// The playlists `video_id` can be saved to, and whether each one holds it already. Needs
  /// cookies.
  pub async fn playlist_choices(&self, video_id: &str) -> Result<Vec<PlaylistChoice>> {
    self.require_session()?;
    let body = json!({ "videoIds": [video_id], "excludeWatchLater": false });
    let response = self.web("playlist/get_add_to_playlist", body, true).await?;
    self.check_session(&response)?;
    Ok(parse_playlist_choices(&response))
  }

  /// Adds `video_id` at the end of a playlist (`WL` for "Watch later"). Needs cookies.
  pub async fn add_to_playlist(&self, playlist_id: &str, video_id: &str) -> Result<()> {
    let action = json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id });
    self.edit_playlist(playlist_id, action).await
  }

  /// Removes `video_id` from a playlist. Needs cookies.
  pub async fn remove_from_playlist(&self, playlist_id: &str, video_id: &str) -> Result<()> {
    let action = json!({ "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": video_id });
    self.edit_playlist(playlist_id, action).await
  }

  /// Removes one entry from a playlist the user can edit ([`crate::VideoItem::set_video_id`]),
  /// as the playlist page's own menu does. Needs cookies.
  pub async fn remove_playlist_entry(&self, playlist_id: &str, set_video_id: &str) -> Result<()> {
    let action = json!({ "action": "ACTION_REMOVE_VIDEO", "setVideoId": set_video_id });
    self.edit_playlist(playlist_id, action).await
  }

  /// `browse/edit_playlist` with one action, the body the "Save to playlist" dialog's
  /// endpoints carry (see `tests/fixtures/add_to_playlist.json`). No response of it is saved
  /// yet: an HTTP success counts as done unless the response has a `status` other than
  /// `STATUS_SUCCEEDED`.
  async fn edit_playlist(&self, playlist_id: &str, action: Value) -> Result<()> {
    self.require_session()?;
    let body = json!({ "playlistId": playlist_id, "actions": [action] });
    let response = self.web("browse/edit_playlist", body, true).await?;
    match response["status"].as_str() {
      None | Some("STATUS_SUCCEEDED") => Ok(()),
      Some(status) => Err(Error::Parse {
        endpoint: "browse/edit_playlist",
        detail: format!("status {status}"),
      }),
    }
  }

  /// "Videos" tab of a channel, newest first.
  pub async fn channel_videos(&self, channel_id: &str) -> Result<Feed> {
    // Protobuf selecting the "Videos" tab, as the web client sends it.
    let body = json!({ "browseId": channel_id, "params": "EgZ2aWRlb3PyBgQKAjoA" });
    let response = self.web("browse", body, true).await?;
    self.feed(&response)
  }

  /// A page of comments or of replies. `token` comes from
  /// [`WatchPage::comments_continuation`], [`CommentPage::continuation`] or
  /// [`crate::Comment::replies_continuation`].
  pub async fn comments(&self, token: &str) -> Result<CommentPage> {
    let response = self
      .web("next", json!({ "continuation": token }), true)
      .await?;
    Ok(parse_comments(&response))
  }

  /// Sets the user's rating of `video_id`: `like/like`, `like/dislike`, or `like/removelike`
  /// for [`LikeStatus::Indifferent`]. `params` come from [`WatchPage::like_params`], as the
  /// page's buttons send them. Needs cookies. No response of these endpoints is saved yet: an
  /// HTTP success counts as done.
  pub async fn rate(&self, video_id: &str, status: LikeStatus, params: Option<&str>) -> Result<()> {
    self.require_session()?;
    let endpoint = match status {
      LikeStatus::Like => "like/like",
      LikeStatus::Dislike => "like/dislike",
      LikeStatus::Indifferent => "like/removelike",
    };
    let mut body = json!({ "target": { "videoId": video_id } });
    if let Some(params) = params {
      body["params"] = params.into();
    }
    self.web(endpoint, body, true).await?;
    Ok(())
  }

  /// Sets how much YouTube notifies the user about a subscribed channel. `params` come from
  /// [`crate::ChannelNotifications::params`], as the bell's menu sends them. Needs cookies. No
  /// response of this endpoint is saved yet: an HTTP success counts as done.
  pub async fn set_channel_notifications(&self, params: &str) -> Result<()> {
    self.require_session()?;
    let body = json!({ "params": params });
    self
      .web("notification/modify_channel_preference", body, true)
      .await?;
    Ok(())
  }

  /// The first page of the notifications inbox, as youtube.com's bell shows it. Needs cookies.
  /// youtube.com reads it with `browse` (`FEnotifications_inbox`): the older
  /// `notification/get_notification_menu` now answers an empty inbox (checked 2026-11-01).
  pub async fn notifications(&self) -> Result<NotificationPage> {
    self.require_session()?;
    let response = self
      .web(
        "browse",
        json!({ "browseId": "FEnotifications_inbox" }),
        true,
      )
      .await?;
    self.check_session(&response)?;
    Ok(parse_notifications(&response))
  }

  /// The next notifications, from [`NotificationPage::continuation`]. Needs cookies.
  pub async fn notifications_more(&self, token: &str) -> Result<NotificationPage> {
    self.require_session()?;
    let response = self
      .web("browse", json!({ "continuation": token }), true)
      .await?;
    self.check_session(&response)?;
    Ok(parse_notifications(&response))
  }

  /// How many notifications arrived since the user last opened the inbox (youtube.com's
  /// badge). Needs cookies.
  pub async fn unseen_notifications(&self) -> Result<u32> {
    self.require_session()?;
    let response = self
      .web("notification/get_unseen_count", json!({}), true)
      .await?;
    self.check_session(&response)?;
    parse_unseen_count(&response).ok_or(Error::Parse {
      endpoint: "notification/get_unseen_count",
      detail: "unseenCount missing".into(),
    })
  }

  /// Records a click on a notification, which marks it read. `token` is
  /// [`crate::Notification::read_token`]. Needs cookies. No response of this endpoint is saved
  /// yet: an HTTP success counts as done.
  pub async fn mark_notification_read(&self, token: &str) -> Result<()> {
    self.require_session()?;
    let body = json!({ "serializedRecordNotificationInteractionsRequest": token });
    self
      .web("notification/record_interactions", body, true)
      .await?;
    Ok(())
  }

  /// Likes and estimated dislikes from Return YouTube Dislike.
  pub async fn votes(&self, video_id: &str) -> Result<Votes> {
    let response = self
      .http
      .get(format!(
        "https://returnyoutubedislikeapi.com/votes?videoId={video_id}"
      ))
      .send()
      .await?;
    if !response.status().is_success() {
      return Err(Error::Status {
        status: response.status().as_u16(),
        endpoint: "returnyoutubedislike",
      });
    }
    let value: Value = response.json().await?;
    match (value["likes"].as_u64(), value["dislikes"].as_u64()) {
      (Some(likes), Some(dislikes)) => Ok(Votes { likes, dislikes }),
      _ => Err(Error::Parse {
        endpoint: "returnyoutubedislike",
        detail: "likes or dislikes missing".into(),
      }),
    }
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
    if let Some(session) = &self.session {
      request = request.header("Cookie", lock(&session.cookies).header());
    }
    let response = request.send().await?;
    self.keep_rotated_cookies(&response);
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

  /// Applies the `Set-Cookie` headers of a signed-in response and writes the jar back if a
  /// value changed. The file is small: written synchronously, on the network runtime.
  fn keep_rotated_cookies(&self, response: &reqwest::Response) {
    let Some(session) = &self.session else { return };
    let mut cookies = lock(&session.cookies);
    let mut changed = false;
    for header in response.headers().get_all(reqwest::header::SET_COOKIE) {
      if let Ok(header) = header.to_str() {
        changed |= cookies.apply_set_cookie(header);
      }
    }
    if !changed {
      return;
    }
    tracing::debug!("session cookies rotated");
    if let Some(file) = &session.file {
      // Written next to the file then renamed, so a crash never leaves a truncated jar.
      let temporary = file.with_extension("txt.tmp");
      let written = std::fs::write(&temporary, cookies.to_netscape())
        .and_then(|()| std::fs::rename(&temporary, file));
      if let Err(e) = written {
        tracing::warn!("cannot save the rotated cookies: {e}");
      }
    }
  }

  fn require_session(&self) -> Result<()> {
    if self.session.is_some() {
      Ok(())
    } else {
      Err(Error::SignInRequired)
    }
  }

  fn feed(&self, response: &Value) -> Result<Feed> {
    let feed = parse_feed(response);
    if self.session.is_some() && !feed.logged_in {
      return Err(Error::NotLoggedIn);
    }
    Ok(feed)
  }

  /// With cookies, a response served as signed out means the session is gone.
  fn check_session(&self, response: &Value) -> Result<()> {
    if self.session.is_some() && tracking_param(response, "logged_in").as_deref() != Some("1") {
      return Err(Error::NotLoggedIn);
    }
    Ok(())
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
    let session = self.session.as_ref().filter(|_| signed);
    if let (Some(visitor), true) = (&visitor, signed) {
      request = request.header("X-Goog-Visitor-Id", visitor);
    }
    if let Some(session) = session {
      let cookies = lock(&session.cookies);
      request = request
        .header("Cookie", cookies.header())
        .header("Authorization", cookies.authorization())
        .header("X-Goog-AuthUser", "0");
    }
    let response = self.send(endpoint, request, signed).await?;
    if signed && let Some(visitor) = response["responseContext"]["visitorData"].as_str() {
      self
        .state()
        .visitor_data
        .get_or_insert_with(|| visitor.to_owned());
    }
    Ok(response)
  }

  async fn send(
    &self,
    endpoint: &'static str,
    request: reqwest::RequestBuilder,
    signed: bool,
  ) -> Result<Value> {
    let response = request.send().await?;
    if signed {
      self.keep_rotated_cookies(&response);
    }
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
      if state.resolver_visitor.is_none() && self.session.is_none() {
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
    let player = self.send("player", request, false).await?;
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
    let qualities = hls::ladder(&master);
    let variant = qualities
      .iter()
      .find(|v| v.height <= max_height)
      .or(qualities.last())
      .cloned()
      .ok_or(Error::Parse {
        endpoint: "hls master",
        detail: "no variant".into(),
      })?;
    Ok(Stream {
      video_url: variant.url,
      audio_url: variant.audio_url,
      resolution: variant.resolution,
      codecs: variant.codecs,
      user_agent: VISIONOS_USER_AGENT,
      captions: parse_captions(&player),
      storyboard: parse_storyboard(&player),
      qualities,
    })
  }
}

/// A poisoned lock only means a panic elsewhere; the value stays usable.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
  mutex
    .lock()
    .unwrap_or_else(std::sync::PoisonError::into_inner)
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
