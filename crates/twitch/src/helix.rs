//! Helix (`api.twitch.tv/helix`), Twitch's public API, with the user's token
//! ([`crate::auth::Session`]): followed channels, categories, streams, search, users.
//!
//! A 401 refreshes the token once (single-flight, [`crate::auth::Session::refresh_after`]) and
//! retries; a 429 is [`Error::RateLimited`]. Answers are parsed leniently: an item missing its id
//! is skipped, never the page. Image URLs are templates ([`sized`]).

use serde_json::Value;

use crate::auth::Session;
use crate::error::{Error, Result};

const BASE: &str = "https://api.twitch.tv/helix/";

/// Items per page (Helix's maximum is 100).
pub const PAGE: &str = "100";

/// A page of results and the cursor of the next one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page<T> {
  /// The items.
  pub items: Vec<T>,
  /// `pagination.cursor`, when there are more.
  pub cursor: Option<String>,
}

/// A live stream.
#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
  /// Stream id.
  pub id: String,
  /// The broadcaster's id, login and shown name.
  pub user_id: String,
  /// See [`Stream::user_id`].
  pub user_login: String,
  /// See [`Stream::user_id`].
  pub user_name: String,
  /// Category.
  pub game_id: String,
  /// See [`Stream::game_id`].
  pub game_name: String,
  /// The broadcast's title.
  pub title: String,
  /// Current viewers.
  pub viewer_count: u64,
  /// Start, milliseconds since the epoch.
  pub started_at: Option<i64>,
  /// Preview template (`{width}x{height}`).
  pub thumbnail_url: String,
  /// Tags.
  pub tags: Vec<String>,
}

/// A user (a channel).
#[derive(Clone, Debug, PartialEq)]
pub struct User {
  /// Numeric id.
  pub id: String,
  /// Login, lowercase.
  pub login: String,
  /// Shown name.
  pub display_name: String,
  /// Avatar URL (300 px).
  pub profile_image_url: String,
  /// Picture shown while offline.
  pub offline_image_url: String,
  /// The channel's description.
  pub description: String,
}

/// A channel the user follows.
#[derive(Clone, Debug, PartialEq)]
pub struct Followed {
  /// The broadcaster's id, login and shown name.
  pub broadcaster_id: String,
  /// See [`Followed::broadcaster_id`].
  pub broadcaster_login: String,
  /// See [`Followed::broadcaster_id`].
  pub broadcaster_name: String,
}

/// A category (a game, "Just Chatting"…).
#[derive(Clone, Debug, PartialEq)]
pub struct Category {
  /// Id.
  pub id: String,
  /// Name.
  pub name: String,
  /// Box art template (`{width}x{height}`, tall 3:4 art).
  pub box_art_url: String,
}

/// A channel found by a search.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelResult {
  /// The broadcaster's id.
  pub id: String,
  /// Login.
  pub login: String,
  /// Shown name.
  pub display_name: String,
  /// On air now.
  pub is_live: bool,
  /// The last category.
  pub game_name: String,
  /// The last title.
  pub title: String,
  /// Avatar URL.
  pub thumbnail_url: String,
}

/// `template` with its size placeholders (`{width}x{height}`, `%{width}x%{height}`) filled.
pub fn sized(template: &str, width: u32, height: u32) -> String {
  template
    .replace("%{width}", &width.to_string())
    .replace("%{height}", &height.to_string())
    .replace("{width}", &width.to_string())
    .replace("{height}", &height.to_string())
}

fn text(v: &Value) -> String {
  v.as_str().unwrap_or_default().to_owned()
}

fn page<T>(answer: &Value, item: impl Fn(&Value) -> Option<T>) -> Page<T> {
  Page {
    items: answer["data"]
      .as_array()
      .map(|list| list.iter().filter_map(item).collect())
      .unwrap_or_default(),
    cursor: answer["pagination"]["cursor"]
      .as_str()
      .filter(|c| !c.is_empty())
      .map(str::to_owned),
  }
}

/// `/streams`, `/streams/followed`.
pub fn parse_streams(answer: &Value) -> Page<Stream> {
  page(answer, |s| {
    let id = s["id"].as_str().filter(|id| !id.is_empty())?;
    Some(Stream {
      id: id.to_owned(),
      user_id: text(&s["user_id"]),
      user_login: text(&s["user_login"]),
      user_name: text(&s["user_name"]),
      game_id: text(&s["game_id"]),
      game_name: text(&s["game_name"]),
      title: text(&s["title"]),
      viewer_count: s["viewer_count"].as_u64().unwrap_or(0),
      started_at: s["started_at"].as_str().and_then(crate::time::epoch_millis),
      thumbnail_url: text(&s["thumbnail_url"]),
      tags: s["tags"]
        .as_array()
        .map(|t| {
          t.iter()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect()
        })
        .unwrap_or_default(),
    })
  })
}

/// `/users`.
pub fn parse_users(answer: &Value) -> Vec<User> {
  page(answer, |u| {
    let id = u["id"].as_str().filter(|id| !id.is_empty())?;
    Some(User {
      id: id.to_owned(),
      login: text(&u["login"]),
      display_name: text(&u["display_name"]),
      profile_image_url: text(&u["profile_image_url"]),
      offline_image_url: text(&u["offline_image_url"]),
      description: text(&u["description"]),
    })
  })
  .items
}

/// `/channels/followed`.
pub fn parse_followed(answer: &Value) -> Page<Followed> {
  page(answer, |f| {
    let id = f["broadcaster_id"].as_str().filter(|id| !id.is_empty())?;
    Some(Followed {
      broadcaster_id: id.to_owned(),
      broadcaster_login: text(&f["broadcaster_login"]),
      broadcaster_name: text(&f["broadcaster_name"]),
    })
  })
}

/// `/games/top`, `/games`, `/search/categories`.
pub fn parse_categories(answer: &Value) -> Page<Category> {
  page(answer, |c| {
    let id = c["id"].as_str().filter(|id| !id.is_empty())?;
    Some(Category {
      id: id.to_owned(),
      name: text(&c["name"]),
      box_art_url: text(&c["box_art_url"]),
    })
  })
}

/// `/search/channels`.
pub fn parse_search_channels(answer: &Value) -> Page<ChannelResult> {
  page(answer, |c| {
    let id = c["id"].as_str().filter(|id| !id.is_empty())?;
    Some(ChannelResult {
      id: id.to_owned(),
      login: text(&c["broadcaster_login"]),
      display_name: text(&c["display_name"]),
      is_live: c["is_live"].as_bool().unwrap_or(false),
      game_name: text(&c["game_name"]),
      title: text(&c["title"]),
      thumbnail_url: text(&c["thumbnail_url"]),
    })
  })
}

/// The Helix client of a signed-in session. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Helix {
  http: reqwest::Client,
  session: Session,
}

impl Helix {
  /// Helix with `session`'s token and its Client-ID.
  pub fn new(session: Session) -> Result<Self> {
    Ok(Self {
      http: reqwest::Client::builder().build()?,
      session,
    })
  }

  /// The session.
  pub fn session(&self) -> &Session {
    &self.session
  }

  /// `GET helix/{path}` with `query`, as JSON; a refused token is refreshed once.
  pub async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
    let url = reqwest::Url::parse_with_params(&format!("{BASE}{path}"), query).map_err(|e| {
      Error::Parse {
        endpoint: "helix",
        detail: e.to_string(),
      }
    })?;
    let mut token = self.session.access_token();
    for attempt in 0..2 {
      let response = self
        .http
        .get(url.clone())
        .header("Client-Id", self.session.auth().client_id())
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await?;
      let status = response.status().as_u16();
      match status {
        200 => {
          let text = response.text().await?;
          return serde_json::from_str(&text).map_err(|e| Error::Parse {
            endpoint: "helix",
            detail: e.to_string(),
          });
        }
        401 if attempt == 0 => token = self.session.refresh_after(&token).await?,
        401 => return Err(Error::SessionExpired),
        429 => return Err(Error::RateLimited),
        _ => {
          return Err(Error::Status {
            status,
            endpoint: "helix",
            code: None,
          });
        }
      }
    }
    Err(Error::SessionExpired)
  }

  /// The signed-in user.
  pub async fn me(&self) -> Result<User> {
    parse_users(&self.get("users", &[]).await?)
      .into_iter()
      .next()
      .ok_or(Error::Parse {
        endpoint: "helix/users",
        detail: "no user".into(),
      })
  }

  /// Users by id, at most 100.
  pub async fn users(&self, ids: &[String]) -> Result<Vec<User>> {
    if ids.is_empty() {
      return Ok(Vec::new());
    }
    let query: Vec<(&str, &str)> = ids.iter().take(100).map(|id| ("id", id.as_str())).collect();
    Ok(parse_users(&self.get("users", &query).await?))
  }

  /// The followed channels on air, most viewers first.
  pub async fn followed_streams(&self, user_id: &str, after: Option<&str>) -> Result<Page<Stream>> {
    let mut query = vec![("user_id", user_id), ("first", PAGE)];
    query.extend(after.map(|a| ("after", a)));
    Ok(parse_streams(&self.get("streams/followed", &query).await?))
  }

  /// The followed channels, most recently followed first.
  pub async fn followed_channels(
    &self,
    user_id: &str,
    after: Option<&str>,
  ) -> Result<Page<Followed>> {
    let mut query = vec![("user_id", user_id), ("first", PAGE)];
    query.extend(after.map(|a| ("after", a)));
    Ok(parse_followed(
      &self.get("channels/followed", &query).await?,
    ))
  }

  /// The categories with the most viewers.
  pub async fn top_categories(&self, after: Option<&str>) -> Result<Page<Category>> {
    let mut query = vec![("first", PAGE)];
    query.extend(after.map(|a| ("after", a)));
    Ok(parse_categories(&self.get("games/top", &query).await?))
  }

  /// A category's live streams, most viewers first.
  pub async fn category_streams(&self, game_id: &str, after: Option<&str>) -> Result<Page<Stream>> {
    let mut query = vec![("game_id", game_id), ("first", PAGE)];
    query.extend(after.map(|a| ("after", a)));
    Ok(parse_streams(&self.get("streams", &query).await?))
  }

  /// Channels matching `text`.
  pub async fn search_channels(&self, text: &str) -> Result<Page<ChannelResult>> {
    let query = [("query", text), ("first", "20")];
    Ok(parse_search_channels(
      &self.get("search/channels", &query).await?,
    ))
  }

  /// Categories matching `text`.
  pub async fn search_categories(&self, text: &str) -> Result<Page<Category>> {
    let query = [("query", text), ("first", "20")];
    Ok(parse_categories(
      &self.get("search/categories", &query).await?,
    ))
  }
}
