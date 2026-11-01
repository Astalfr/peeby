use std::fmt;
use std::time::Duration;

use reqwest::{Method, RequestBuilder, header};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::Snippet;
use crate::models::{
  AuthenticationResult, BaseItem, ImageType, ItemKind, PublicSystemInfo, QueryResult,
  QuickConnectResult, User,
};
use crate::{ClientInfo, Error, ItemId, Result, UserId};

/// A logged-in session: what has to be persisted to stay logged in.
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
  /// Logged-in user.
  pub user_id: UserId,
  /// Access token. Never log it.
  pub access_token: String,
}

impl fmt::Debug for Session {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("Session")
      .field("user_id", &self.user_id)
      .field("access_token", &"<redacted>")
      .finish()
  }
}

/// Jellyfin client for one server.
#[derive(Clone, Debug)]
pub struct Client {
  http: reqwest::Client,
  base: Url,
  info: ClientInfo,
  session: Option<Session>,
}

impl Client {
  /// Creates a client for `server_url` (scheme required; a trailing path such as `/jellyfin` is kept).
  pub fn new(server_url: &str, info: ClientInfo) -> Result<Self> {
    let mut base = Url::parse(server_url.trim())?;
    if !base.path().ends_with('/') {
      let path = format!("{}/", base.path());
      base.set_path(&path);
    }
    base.set_query(None);
    base.set_fragment(None);
    let http = reqwest::Client::builder()
      .connect_timeout(Duration::from_secs(10))
      .timeout(Duration::from_secs(30))
      .build()?;
    Ok(Self {
      http,
      base,
      info,
      session: None,
    })
  }

  /// Server base URL, always ending with `/`.
  pub fn base_url(&self) -> &Url {
    &self.base
  }

  /// Current session, if logged in.
  pub fn session(&self) -> Option<&Session> {
    self.session.as_ref()
  }

  /// Restores a persisted session.
  pub fn set_session(&mut self, session: Option<Session>) {
    self.session = session;
  }

  /// The `Authorization` header value for the current session (also used for the video stream).
  pub fn authorization_header(&self) -> String {
    self
      .info
      .authorization(self.session.as_ref().map(|s| s.access_token.as_str()))
  }

  // --- System -----------------------------------------------------------------------------

  /// Public server information. Works without a session.
  pub async fn public_system_info(&self) -> Result<PublicSystemInfo> {
    self
      .send_json(
        self.request(Method::GET, "System/Info/Public")?,
        "System/Info/Public",
      )
      .await
  }

  /// Like [`Self::public_system_info`], but fails with [`Error::UnsupportedServer`] below 12.0.
  pub async fn check_server(&self) -> Result<PublicSystemInfo> {
    let info = self.public_system_info().await?;
    if info.is_supported() {
      Ok(info)
    } else {
      Err(Error::UnsupportedServer {
        version: info.version.clone().unwrap_or_else(|| "unknown".into()),
      })
    }
  }

  // --- Authentication ---------------------------------------------------------------------

  /// Logs in with a user name and password, and keeps the session.
  pub async fn authenticate_by_name(&mut self, username: &str, password: &str) -> Result<Session> {
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Body<'a> {
      username: &'a str,
      pw: &'a str,
    }
    let endpoint = "Users/AuthenticateByName";
    let request = self.request(Method::POST, endpoint)?.json(&Body {
      username,
      pw: password,
    });
    let result: AuthenticationResult = self.send_json(request, endpoint).await?;
    self.adopt(result, endpoint)
  }

  /// Whether Quick Connect is enabled on the server.
  pub async fn quick_connect_enabled(&self) -> Result<bool> {
    self
      .send_json(
        self.request(Method::GET, "QuickConnect/Enabled")?,
        "QuickConnect/Enabled",
      )
      .await
  }

  /// Starts a Quick Connect request. Show `code` to the user and poll with `secret`.
  pub async fn quick_connect_initiate(&self) -> Result<QuickConnectResult> {
    self
      .send_json(
        self.request(Method::POST, "QuickConnect/Initiate")?,
        "QuickConnect/Initiate",
      )
      .await
  }

  /// Polls a Quick Connect request.
  pub async fn quick_connect_state(&self, secret: &str) -> Result<QuickConnectResult> {
    let request = self
      .request(Method::GET, "QuickConnect/Connect")?
      .query(&[("secret", secret)]);
    self.send_json(request, "QuickConnect/Connect").await
  }

  /// Exchanges an approved Quick Connect secret for a session, and keeps it.
  pub async fn authenticate_with_quick_connect(&mut self, secret: &str) -> Result<Session> {
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Body<'a> {
      secret: &'a str,
    }
    let endpoint = "Users/AuthenticateWithQuickConnect";
    let request = self.request(Method::POST, endpoint)?.json(&Body { secret });
    let result: AuthenticationResult = self.send_json(request, endpoint).await?;
    self.adopt(result, endpoint)
  }

  /// Ends the session on the server and forgets it locally (even if the server call fails).
  pub async fn logout(&mut self) -> Result<()> {
    if self.session.is_none() {
      return Ok(());
    }
    let request = self.request(Method::POST, "Sessions/Logout");
    self.session = None;
    self.send_empty(request?, "Sessions/Logout").await
  }

  // --- Browsing ---------------------------------------------------------------------------

  /// `GET /Users/Me`: the signed-in user (name, avatar tag).
  pub async fn current_user(&self) -> Result<User> {
    let request = self.request(Method::GET, "Users/Me")?;
    self.send_json(request, "Users/Me").await
  }

  /// URL of a user's avatar (`GET /UserImage`); the tag keys caches.
  pub fn user_image_url(&self, user: &UserId, tag: &str) -> Result<Url> {
    let mut url = self.base.join("UserImage")?;
    url
      .query_pairs_mut()
      .append_pair("userId", user.as_str())
      .append_pair("tag", tag);
    Ok(url)
  }

  /// The user's libraries and views, in their configured order.
  pub async fn user_views(&self) -> Result<QueryResult<BaseItem>> {
    let user = self.user_id()?;
    let request = self
      .request(Method::GET, "UserViews")?
      .query(&[("userId", user.as_str())]);
    self.send_json(request, "UserViews").await
  }

  /// Items matching `query`.
  pub async fn items(&self, query: &ItemsQuery) -> Result<QueryResult<BaseItem>> {
    let user = self.user_id()?;
    let request = self
      .request(Method::GET, "Items")?
      .query(&query.to_pairs(user));
    self.send_json(request, "Items").await
  }

  /// One item, with its user data.
  pub async fn item(&self, id: &ItemId) -> Result<BaseItem> {
    let user = self.user_id()?;
    let endpoint = format!("Items/{id}");
    let request = self
      .request(Method::GET, &endpoint)?
      .query(&[("userId", user.as_str())]);
    self.send_json(request, &endpoint).await
  }

  /// "Continue watching": partly watched videos, most recent first.
  pub async fn resume_items(&self, limit: u32) -> Result<QueryResult<BaseItem>> {
    let mut query = self.list_pairs(limit)?;
    with_overview(&mut query);
    query.push(("mediaTypes", "Video".into()));
    let request = self.request(Method::GET, "UserItems/Resume")?.query(&query);
    self.send_json(request, "UserItems/Resume").await
  }

  /// "Next up": the next episode to watch of each series in progress.
  pub async fn next_up(
    &self,
    limit: u32,
    options: &NextUpOptions,
  ) -> Result<QueryResult<BaseItem>> {
    let mut query = self.list_pairs(limit)?;
    with_overview(&mut query);
    if let Some(days) = options.max_days {
      let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
      query.push(("nextUpDateCutoff", iso_date_days_ago(now, days)));
    }
    query.push(("enableRewatching", options.rewatching.to_string()));
    let request = self.request(Method::GET, "Shows/NextUp")?.query(&query);
    self.send_json(request, "Shows/NextUp").await
  }

  /// Recently added items of a library (episodes are grouped by series).
  pub async fn latest(&self, library: &ItemId, limit: u32) -> Result<Vec<BaseItem>> {
    let mut query = self.list_pairs(limit)?;
    query.push(("parentId", library.to_string()));
    let request = self.request(Method::GET, "Items/Latest")?.query(&query);
    self.send_json(request, "Items/Latest").await
  }

  /// Seasons of a series, in order.
  pub async fn seasons(&self, series: &ItemId) -> Result<QueryResult<BaseItem>> {
    let endpoint = format!("Shows/{series}/Seasons");
    let query = self.list_pairs(0)?;
    let request = self.request(Method::GET, &endpoint)?.query(&query);
    self.send_json(request, &endpoint).await
  }

  /// Episodes of one season of a series, in order, with their synopsis.
  pub async fn episodes(&self, series: &ItemId, season: &ItemId) -> Result<QueryResult<BaseItem>> {
    let endpoint = format!("Shows/{series}/Episodes");
    let mut query = self.list_pairs(0)?;
    query.push(("seasonId", season.to_string()));
    query.push(("isMissing", "false".into()));
    with_overview(&mut query);
    let request = self.request(Method::GET, &endpoint)?.query(&query);
    self.send_json(request, &endpoint).await
  }

  /// Downloads an image (or any other GET resource on this server) as raw bytes.
  pub async fn fetch_bytes(&self, url: &Url) -> Result<Vec<u8>> {
    let endpoint = url.path().to_owned();
    let request = self
      .http
      .get(url.clone())
      .header(header::AUTHORIZATION, self.authorization_header());
    Ok(self.send(request, &endpoint).await?.bytes().await?.to_vec())
  }

  /// Common query for item lists: user, images with blur hashes, aspect ratio, page size.
  fn list_pairs(&self, limit: u32) -> Result<Vec<(&'static str, String)>> {
    let user = self.user_id()?;
    let mut pairs = vec![
      ("userId", user.to_string()),
      ("fields", "PrimaryImageAspectRatio".to_owned()),
      ("enableImageTypes", "Primary,Backdrop,Thumb,Logo".into()),
      ("enableImages", "true".into()),
      ("enableUserData", "true".into()),
    ];
    if limit > 0 {
      pairs.push(("limit", limit.to_string()));
    }
    Ok(pairs)
  }

  /// URL of an item image. Image endpoints need no token, so the URL is safe to log and cache.
  pub fn image_url(
    &self,
    id: &ItemId,
    image_type: ImageType,
    options: &ImageOptions,
  ) -> Result<Url> {
    let mut url = self
      .base
      .join(&format!("Items/{id}/Images/{}", image_type.as_str()))?;
    {
      let mut q = url.query_pairs_mut();
      if let Some(w) = options.max_width {
        q.append_pair("maxWidth", &w.to_string());
      }
      if let Some(h) = options.max_height {
        q.append_pair("maxHeight", &h.to_string());
      }
      if let Some(quality) = options.quality {
        q.append_pair("quality", &quality.to_string());
      }
      if let Some(tag) = &options.tag {
        q.append_pair("tag", tag);
      }
      if let Some(blur) = options.blur {
        q.append_pair("blur", &blur.to_string());
      }
      if let Some(index) = options.index {
        q.append_pair("imageIndex", &index.to_string());
      }
      if let Some(format) = options.format {
        q.append_pair("format", format);
      }
      if let Some((w, h)) = options.fill {
        q.append_pair("fillWidth", &w.to_string());
        q.append_pair("fillHeight", &h.to_string());
      }
    }
    if url.query() == Some("") {
      url.set_query(None);
    }
    Ok(url)
  }

  // --- Plumbing ---------------------------------------------------------------------------

  pub(crate) fn user_id(&self) -> Result<&UserId> {
    self
      .session
      .as_ref()
      .map(|s| &s.user_id)
      .ok_or(Error::NotAuthenticated)
  }

  fn adopt(&mut self, result: AuthenticationResult, endpoint: &str) -> Result<Session> {
    let (Some(user), Some(token)) = (result.user, result.access_token) else {
      return Err(Error::Decode {
        endpoint: endpoint.to_owned(),
        source: serde::de::Error::custom("missing User or AccessToken"),
      });
    };
    let session = Session {
      user_id: user.id,
      access_token: token,
    };
    self.session = Some(session.clone());
    Ok(session)
  }

  pub(crate) fn request(&self, method: Method, path: &str) -> Result<RequestBuilder> {
    let url = self.base.join(path)?;
    Ok(
      self
        .http
        .request(method, url)
        .header(header::AUTHORIZATION, self.authorization_header())
        .header(header::ACCEPT, "application/json"),
    )
  }

  pub(crate) async fn send(
    &self,
    request: RequestBuilder,
    endpoint: &str,
  ) -> Result<reqwest::Response> {
    let response = request.send().await?;
    let status = response.status();
    if status.is_success() {
      return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    Err(Error::Status {
      endpoint: endpoint.to_owned(),
      status: status.as_u16(),
      body: Snippet::new(&body),
    })
  }

  pub(crate) async fn send_json<T: DeserializeOwned>(
    &self,
    request: RequestBuilder,
    endpoint: &str,
  ) -> Result<T> {
    let bytes = self.send(request, endpoint).await?.bytes().await?;
    serde_json::from_slice(&bytes).map_err(|source| Error::Decode {
      endpoint: endpoint.to_owned(),
      source,
    })
  }

  pub(crate) async fn send_empty(&self, request: RequestBuilder, endpoint: &str) -> Result<()> {
    self.send(request, endpoint).await.map(drop)
  }
}

/// Options of [`Client::next_up`].
#[derive(Clone, Debug, Default)]
pub struct NextUpOptions {
  /// Leave out series not watched for this many days (`nextUpDateCutoff`); `None`: no limit.
  pub max_days: Option<u32>,
  /// Also suggest series already watched to the end.
  pub rewatching: bool,
}

/// `YYYY-MM-DDT00:00:00Z`, `days` before the Unix time `now` (UTC).
fn iso_date_days_ago(now: u64, days: u32) -> String {
  // Civil date from days since 1970-01-01 (H. Hinnant's algorithm).
  let z = (now / 86_400) as i64 - i64::from(days) + 719_468;
  let era = z.div_euclid(146_097);
  let doe = z - era * 146_097;
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let day = doy - (153 * mp + 2) / 5 + 1;
  let month = if mp < 10 { mp + 3 } else { mp - 9 };
  let year = yoe + era * 400 + i64::from(month <= 2);
  format!("{year:04}-{month:02}-{day:02}T00:00:00Z")
}

/// Adds the synopsis to the `fields` of a list query.
fn with_overview(query: &mut [(&'static str, String)]) {
  if let Some(fields) = query.iter_mut().find(|(k, _)| *k == "fields") {
    fields.1.push_str(",Overview");
  }
}

/// Options for [`Client::image_url`].
#[derive(Clone, Debug, Default)]
pub struct ImageOptions {
  /// Maximum width in pixels; the server scales down.
  pub max_width: Option<u32>,
  /// Maximum height in pixels.
  pub max_height: Option<u32>,
  /// JPEG/WebP quality, 0–100.
  pub quality: Option<u8>,
  /// Image tag, so the URL changes when the image changes (cache key).
  pub tag: Option<String>,
  /// Server-side blur radius, for backgrounds.
  pub blur: Option<u32>,
  /// Index, for image types with several images (backdrops).
  pub index: Option<u32>,
  /// Output format (`ImageFormat`), e.g. `Webp` or `Jpg`. Server default when unset.
  pub format: Option<&'static str>,
  /// Scale and crop so the image covers exactly this box (pixels), whatever its aspect ratio.
  pub fill: Option<(u32, u32)>,
}

/// Filters for [`Client::items`]. Field names follow the API; empty values are not sent.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ItemsQuery {
  /// Only children of this item (library, series, season…).
  pub parent_id: Option<ItemId>,
  /// Item kinds to include.
  pub include_item_types: Vec<ItemKind>,
  /// Search the whole subtree, not only direct children.
  pub recursive: bool,
  /// Full-text search.
  pub search_term: Option<String>,
  /// Sort keys (`ItemSortBy` values), e.g. `SortName`.
  pub sort_by: Vec<String>,
  /// Sort descending instead of ascending.
  pub descending: bool,
  /// Paging: first index.
  pub start_index: Option<u32>,
  /// Paging: page size.
  pub limit: Option<u32>,
  /// Extra fields (`ItemFields` values), e.g. `Overview`, `Genres`.
  pub fields: Vec<String>,
  /// Leave out missing (virtual) episodes, which have no file to play.
  pub exclude_missing: bool,
}

impl ItemsQuery {
  fn to_pairs(&self, user: &UserId) -> Vec<(&'static str, String)> {
    let mut pairs = vec![("userId", user.to_string())];
    if let Some(parent) = &self.parent_id {
      pairs.push(("parentId", parent.to_string()));
    }
    if !self.include_item_types.is_empty() {
      let kinds: Vec<_> = self.include_item_types.iter().map(|k| k.as_str()).collect();
      pairs.push(("includeItemTypes", kinds.join(",")));
    }
    if self.recursive {
      pairs.push(("recursive", "true".into()));
    }
    if let Some(term) = self.search_term.as_deref().filter(|t| !t.trim().is_empty()) {
      pairs.push(("searchTerm", term.trim().to_owned()));
    }
    if !self.sort_by.is_empty() {
      pairs.push(("sortBy", self.sort_by.join(",")));
      let order = if self.descending {
        "Descending"
      } else {
        "Ascending"
      };
      pairs.push(("sortOrder", order.into()));
    }
    if self.exclude_missing {
      pairs.push(("isMissing", "false".into()));
    }
    if let Some(start) = self.start_index {
      pairs.push(("startIndex", start.to_string()));
    }
    if let Some(limit) = self.limit {
      pairs.push(("limit", limit.to_string()));
    }
    // Blur hashes are cheap and drive the image placeholders.
    pairs.push(("enableImageTypes", "Primary,Backdrop,Thumb,Logo".into()));
    let mut fields = self.fields.clone();
    if !fields.iter().any(|f| f == "PrimaryImageAspectRatio") {
      fields.push("PrimaryImageAspectRatio".into());
    }
    pairs.push(("fields", fields.join(",")));
    pairs.push(("enableImages", "true".into()));
    pairs
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn next_up_cutoff_dates() {
    // 2026-11-01T12:00:00Z
    let now = 1_793_534_400;
    assert_eq!(iso_date_days_ago(now, 0), "2026-11-01T00:00:00Z");
    assert_eq!(iso_date_days_ago(now, 1), "2026-10-31T00:00:00Z");
    assert_eq!(iso_date_days_ago(now, 365), "2025-11-01T00:00:00Z");
    assert_eq!(iso_date_days_ago(0, 0), "1970-01-01T00:00:00Z");
  }

  fn client() -> Client {
    let info = ClientInfo {
      client: "Peeby".into(),
      device: "test".into(),
      device_id: "dev-1".into(),
      version: "0.1.0".into(),
    };
    Client::new("https://media.example.org/jellyfin", info).unwrap()
  }

  #[test]
  fn base_url_keeps_sub_path() {
    let c = client();
    assert_eq!(c.base_url().as_str(), "https://media.example.org/jellyfin/");
    let req = c
      .request(Method::GET, "System/Info/Public")
      .unwrap()
      .build()
      .unwrap();
    assert_eq!(
      req.url().as_str(),
      "https://media.example.org/jellyfin/System/Info/Public"
    );
  }

  #[test]
  fn token_goes_in_header_never_in_url() {
    let mut c = client();
    c.set_session(Some(Session {
      user_id: "u1".into(),
      access_token: "secret-token".into(),
    }));
    let req = c
      .request(Method::GET, "UserViews")
      .unwrap()
      .query(&[("userId", "u1")])
      .build()
      .unwrap();
    let auth = req.headers()[header::AUTHORIZATION].to_str().unwrap();
    assert!(auth.contains("Token=\"secret-token\""));
    assert!(!req.url().as_str().contains("secret-token"));
  }

  #[test]
  fn session_debug_is_redacted() {
    let s = Session {
      user_id: "u1".into(),
      access_token: "secret-token".into(),
    };
    assert!(!format!("{s:?}").contains("secret-token"));
  }

  #[test]
  fn items_query_pairs() {
    let q = ItemsQuery {
      parent_id: Some("lib".into()),
      include_item_types: vec![ItemKind::Movie, ItemKind::Series],
      recursive: true,
      sort_by: vec!["SortName".into()],
      limit: Some(50),
      ..Default::default()
    };
    let pairs = q.to_pairs(&"u1".into());
    let get = |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
    assert_eq!(get("parentId"), Some("lib"));
    assert_eq!(get("includeItemTypes"), Some("Movie,Series"));
    assert_eq!(get("recursive"), Some("true"));
    assert_eq!(get("sortOrder"), Some("Ascending"));
    assert_eq!(get("limit"), Some("50"));
    assert_eq!(get("searchTerm"), None);
  }

  #[test]
  fn image_url_has_no_token() {
    let mut c = client();
    c.set_session(Some(Session {
      user_id: "u1".into(),
      access_token: "secret-token".into(),
    }));
    let opts = ImageOptions {
      max_width: Some(300),
      tag: Some("abc".into()),
      ..Default::default()
    };
    let url = c
      .image_url(&"item1".into(), ImageType::Primary, &opts)
      .unwrap();
    assert_eq!(
      url.as_str(),
      "https://media.example.org/jellyfin/Items/item1/Images/Primary?maxWidth=300&tag=abc"
    );
  }
}
