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
#[derive(Clone, Debug, Default, PartialEq)]
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
  /// A Twitch partner (`broadcaster_type`), shown with the verified badge.
  pub partner: bool,
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

/// A VOD (a past broadcast, a highlight, an upload).
#[derive(Clone, Debug, PartialEq)]
pub struct Video {
  /// Id.
  pub id: String,
  /// The broadcaster's id, login and shown name.
  pub user_id: String,
  /// See [`Video::user_id`].
  pub user_login: String,
  /// See [`Video::user_id`].
  pub user_name: String,
  /// Title.
  pub title: String,
  /// Publication, milliseconds since the epoch.
  pub created_at: Option<i64>,
  /// Views.
  pub view_count: u64,
  /// Length, seconds (`3h8m33s` in the answer).
  pub duration: u64,
  /// Thumbnail template (`%{width}x%{height}`); empty or `404_processing` while being processed.
  pub thumbnail_url: String,
  /// Parts whose audio Twitch removed (copyrighted music), in the order given.
  pub muted_segments: Vec<MutedSegment>,
}

/// A part of a VOD whose audio Twitch removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MutedSegment {
  /// Start, seconds from the VOD's start.
  pub offset: u64,
  /// Length, seconds.
  pub duration: u64,
}

/// A clip.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
  /// Id, also its slug for GQL.
  pub id: String,
  /// The broadcaster's id and shown name.
  pub broadcaster_id: String,
  /// See [`Clip::broadcaster_id`].
  pub broadcaster_name: String,
  /// Who made it.
  pub creator_name: String,
  /// Title.
  pub title: String,
  /// Views.
  pub view_count: u64,
  /// Creation, milliseconds since the epoch.
  pub created_at: Option<i64>,
  /// Direct thumbnail URL.
  pub thumbnail_url: String,
  /// Length, seconds.
  pub duration: f64,
}

/// `3h8m33s`, `45m2s`, `59s` in seconds; anything else counts as 0.
pub fn parse_duration(text: &str) -> u64 {
  let mut total = 0;
  let mut number = 0;
  for c in text.chars() {
    match c {
      '0'..='9' => number = number * 10 + u64::from(c as u8 - b'0'),
      'h' => (total, number) = (total + number * 3600, 0),
      'm' => (total, number) = (total + number * 60, 0),
      's' => (total, number) = (total + number, 0),
      _ => return 0,
    }
  }
  total
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
      partner: u["broadcaster_type"] == "partner",
    })
  })
  .items
}

/// A chat badge's version: shown before a chatter's name when their IRC `badges` tag has
/// `set/version`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
  /// `subscriber`, `moderator`…
  pub set: String,
  /// `0`, `3012`…
  pub version: String,
  /// What it means ("Moderator", "2-Year Subscriber"…).
  pub title: String,
  /// The 18 px image.
  pub url_1x: String,
  /// The 36 px image.
  pub url_2x: String,
  /// The 72 px image.
  pub url_4x: String,
}

/// `/chat/badges/global`, `/chat/badges`: every version of every set.
pub fn parse_badges(answer: &Value) -> Vec<Badge> {
  let Some(sets) = answer["data"].as_array() else {
    return Vec::new();
  };
  sets
    .iter()
    .flat_map(|set| {
      let name = text(&set["set_id"]);
      set["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(move |v| {
          let url_1x = v["image_url_1x"].as_str().filter(|u| !u.is_empty())?;
          Some(Badge {
            set: name.clone(),
            version: text(&v["id"]),
            title: text(&v["title"]),
            url_1x: url_1x.to_owned(),
            url_2x: text(&v["image_url_2x"]),
            url_4x: text(&v["image_url_4x"]),
          })
        })
    })
    .filter(|b| !b.set.is_empty())
    .collect()
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

/// `/videos`.
pub fn parse_videos(answer: &Value) -> Page<Video> {
  page(answer, |v| {
    let id = v["id"].as_str().filter(|id| !id.is_empty())?;
    Some(Video {
      id: id.to_owned(),
      user_id: text(&v["user_id"]),
      user_login: text(&v["user_login"]),
      user_name: text(&v["user_name"]),
      title: text(&v["title"]),
      created_at: v["created_at"].as_str().and_then(crate::time::epoch_millis),
      view_count: v["view_count"].as_u64().unwrap_or(0),
      duration: parse_duration(v["duration"].as_str().unwrap_or_default()),
      thumbnail_url: text(&v["thumbnail_url"]),
      // `null` without any.
      muted_segments: v["muted_segments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
          Some(MutedSegment {
            offset: m["offset"].as_u64()?,
            duration: m["duration"].as_u64().filter(|&d| d > 0)?,
          })
        })
        .collect(),
    })
  })
}

/// `/clips`.
pub fn parse_clips(answer: &Value) -> Page<Clip> {
  page(answer, |c| {
    let id = c["id"].as_str().filter(|id| !id.is_empty())?;
    Some(Clip {
      id: id.to_owned(),
      broadcaster_id: text(&c["broadcaster_id"]),
      broadcaster_name: text(&c["broadcaster_name"]),
      creator_name: text(&c["creator_name"]),
      title: text(&c["title"]),
      view_count: c["view_count"].as_u64().unwrap_or(0),
      created_at: c["created_at"].as_str().and_then(crate::time::epoch_millis),
      thumbnail_url: text(&c["thumbnail_url"]),
      duration: c["duration"].as_f64().unwrap_or(0.0),
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

  /// A user by login.
  pub async fn user(&self, login: &str) -> Result<User> {
    parse_users(&self.get("users", &[("login", login)]).await?)
      .into_iter()
      .next()
      .ok_or(Error::NotFound)
  }

  /// A channel's past broadcasts (`type=archive`), newest first; `None`: every type.
  pub async fn videos(
    &self,
    user_id: &str,
    kind: Option<&str>,
    after: Option<&str>,
  ) -> Result<Page<Video>> {
    let mut query = vec![("user_id", user_id), ("first", "40")];
    query.extend(kind.map(|k| ("type", k)));
    query.extend(after.map(|a| ("after", a)));
    Ok(parse_videos(&self.get("videos", &query).await?))
  }

  /// A VOD by id.
  pub async fn video(&self, id: &str) -> Result<Video> {
    parse_videos(&self.get("videos", &[("id", id)]).await?)
      .items
      .into_iter()
      .next()
      .ok_or(Error::NotFound)
  }

  /// A channel's most viewed clips.
  pub async fn clips(&self, broadcaster_id: &str, after: Option<&str>) -> Result<Page<Clip>> {
    let mut query = vec![("broadcaster_id", broadcaster_id), ("first", "40")];
    query.extend(after.map(|a| ("after", a)));
    Ok(parse_clips(&self.get("clips", &query).await?))
  }

  /// A clip by id.
  pub async fn clip(&self, id: &str) -> Result<Clip> {
    parse_clips(&self.get("clips", &[("id", id)]).await?)
      .items
      .into_iter()
      .next()
      .ok_or(Error::NotFound)
  }

  /// The chat badges every channel shares (moderator, VIP, partner…).
  pub async fn global_badges(&self) -> Result<Vec<Badge>> {
    Ok(parse_badges(&self.get("chat/badges/global", &[]).await?))
  }

  /// A channel's own chat badges (subscriber months, bits).
  pub async fn channel_badges(&self, broadcaster_id: &str) -> Result<Vec<Badge>> {
    Ok(parse_badges(
      &self
        .get("chat/badges", &[("broadcaster_id", broadcaster_id)])
        .await?,
    ))
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
