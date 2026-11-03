//! The catalog through GQL, anonymous (docs/twitch.md, "Catalog through GQL instead of Helix"):
//! categories, a category's streams, users, a channel's VODs and clips, searches and chat badges,
//! as [`crate::helix`]'s types. Raw queries with the web Client-ID; a list is its first page only,
//! since the `after` cursor fails Twitch's integrity check (checked 2026-11-03). The followed
//! channels stay Helix's: GQL refuses them to raw queries.

use serde_json::{Value, json};

use crate::Client;
use crate::error::{Error, Result};
use crate::gql;
use crate::helix::{
  Badge, Category, ChannelResult, Clip, Helix, MutedSegment, Stream, User, Video,
};

/// Items in a first page (GQL's maximum for `first`).
pub const PAGE: u32 = 100;

const USER_FIELDS: &str = "id login displayName description profileImageURL(width: 300) offlineImageURL roles { isPartner }";

const STREAM_FIELDS: &str = "id title viewersCount createdAt previewImageURL freeformTags { name } \
  game { id displayName } broadcaster { id login displayName }";

const VIDEO_FIELDS: &str = "id title createdAt viewCount lengthSeconds previewThumbnailURL \
  owner { id login displayName } muteInfo { mutedSegmentConnection { nodes { offset duration } } }";

const CLIP_FIELDS: &str = "id slug title viewCount createdAt thumbnailURL durationSeconds \
  curator { displayName } broadcaster { id displayName }";

const BADGE_FIELDS: &str = "setID version title image1x: imageURL(size: NORMAL) \
  image2x: imageURL(size: DOUBLE) image4x: imageURL(size: QUADRUPLE)";

fn request(name: &str, query: String, variables: Value) -> Value {
  json!({ "operationName": name, "query": query, "variables": variables })
}

/// The categories with the most viewers.
pub fn top_categories_request() -> Value {
  request(
    "TopCategories",
    format!(
      "query TopCategories {{ games(first: {PAGE}, options: {{ sort: VIEWER_COUNT }}) {{ edges {{ \
       node {{ id displayName boxArtURL }} }} }} }}"
    ),
    json!({}),
  )
}

/// Category `id`'s live streams, most viewers first.
pub fn category_streams_request(id: &str) -> Value {
  request(
    "CategoryStreams",
    format!(
      "query CategoryStreams($id: ID!) {{ game(id: $id) {{ streams(first: {PAGE}, options: {{ \
       sort: VIEWER_COUNT }}) {{ edges {{ node {{ {STREAM_FIELDS} }} }} }} }} }}"
    ),
    json!({ "id": id }),
  )
}

/// Users by id.
pub fn users_request(ids: &[String]) -> Value {
  request(
    "Users",
    format!("query Users($ids: [ID!]) {{ users(ids: $ids) {{ {USER_FIELDS} }} }}"),
    json!({ "ids": ids }),
  )
}

/// A user by login.
pub fn user_request(login: &str) -> Value {
  request(
    "User",
    format!("query User($login: String!) {{ user(login: $login) {{ {USER_FIELDS} }} }}"),
    json!({ "login": login }),
  )
}

/// Channel `user_id`'s past broadcasts, newest first.
pub fn videos_request(user_id: &str, first: u32) -> Value {
  request(
    "ChannelVideos",
    format!(
      "query ChannelVideos($id: ID!) {{ user(id: $id) {{ videos(first: {first}, type: ARCHIVE, \
       sort: TIME) {{ edges {{ node {{ {VIDEO_FIELDS} }} }} }} }} }}"
    ),
    json!({ "id": user_id }),
  )
}

/// A VOD by id.
pub fn video_request(id: &str) -> Value {
  request(
    "Video",
    format!("query Video($id: ID!) {{ video(id: $id) {{ {VIDEO_FIELDS} }} }}"),
    json!({ "id": id }),
  )
}

/// Channel `user_id`'s most viewed clips.
pub fn clips_request(user_id: &str, first: u32) -> Value {
  request(
    "ChannelClips",
    format!(
      "query ChannelClips($id: ID!) {{ user(id: $id) {{ clips(first: {first}, criteria: {{ \
       period: ALL_TIME, sort: VIEWS_DESC }}) {{ edges {{ node {{ {CLIP_FIELDS} }} }} }} }} }}"
    ),
    json!({ "id": user_id }),
  )
}

/// A clip by slug.
pub fn clip_request(slug: &str) -> Value {
  request(
    "Clip",
    format!("query Clip($slug: ID!) {{ clip(slug: $slug) {{ {CLIP_FIELDS} }} }}"),
    json!({ "slug": slug }),
  )
}

/// Channels matching `text`.
pub fn search_channels_request(text: &str) -> Value {
  request(
    "SearchChannels",
    "query SearchChannels($text: String!) { searchUsers(userQuery: $text, first: 20) { edges { \
     node { id login displayName profileImageURL(width: 300) stream { id game { displayName } } \
     broadcastSettings { title game { displayName } } } } } }"
      .to_owned(),
    json!({ "text": text }),
  )
}

/// Categories matching `text`.
pub fn search_categories_request(text: &str) -> Value {
  request(
    "SearchCategories",
    "query SearchCategories($text: String!) { searchCategories(query: $text, first: 20) { edges { \
     node { id displayName boxArtURL } } } }"
      .to_owned(),
    json!({ "text": text }),
  )
}

/// The chat badges every channel shares.
pub fn global_badges_request() -> Value {
  request(
    "GlobalBadges",
    format!("query GlobalBadges {{ badges {{ {BADGE_FIELDS} }} }}"),
    json!({}),
  )
}

/// Channel `user_id`'s own chat badges.
pub fn channel_badges_request(user_id: &str) -> Value {
  request(
    "ChannelBadges",
    format!(
      "query ChannelBadges($id: ID!) {{ user(id: $id) {{ broadcastBadges {{ {BADGE_FIELDS} }} }} }}"
    ),
    json!({ "id": user_id }),
  )
}

fn text(v: &Value) -> String {
  v.as_str().unwrap_or_default().to_owned()
}

fn non_empty(v: &Value) -> Option<&str> {
  v.as_str().filter(|s| !s.is_empty())
}

/// The nodes of a connection's `edges`; a missing list is empty.
fn nodes(connection: &Value) -> impl Iterator<Item = &Value> {
  connection["edges"]
    .as_array()
    .into_iter()
    .flatten()
    .map(|edge| &edge["node"])
}

/// `answer`'s error when there is no data at `field` (a list next to an error still counts).
fn failed(answer: &Value, field: &Value) -> Result<()> {
  match gql::gql_error(answer) {
    Some(error) if field.is_null() => Err(error),
    _ => Ok(()),
  }
}

/// [`failed`] for a list, which is never null without an error otherwise: a null list is a query
/// Twitch closed without saying so, [`Error::Parse`], so that Helix is asked.
fn listed(answer: &Value, list: &Value) -> Result<()> {
  failed(answer, list)?;
  if list.is_null() {
    return Err(Error::Parse {
      endpoint: "gql",
      detail: "catalog list: null".into(),
    });
  }
  Ok(())
}

fn category(node: &Value) -> Option<Category> {
  Some(Category {
    id: non_empty(&node["id"])?.to_owned(),
    name: text(&node["displayName"]),
    box_art_url: text(&node["boxArtURL"]),
  })
}

fn stream(node: &Value) -> Option<Stream> {
  let broadcaster = &node["broadcaster"];
  Some(Stream {
    id: non_empty(&node["id"])?.to_owned(),
    user_id: text(&broadcaster["id"]),
    user_login: text(&broadcaster["login"]),
    user_name: text(&broadcaster["displayName"]),
    game_id: text(&node["game"]["id"]),
    game_name: text(&node["game"]["displayName"]),
    title: text(&node["title"]),
    viewer_count: node["viewersCount"].as_u64().unwrap_or(0),
    started_at: node["createdAt"]
      .as_str()
      .and_then(crate::time::epoch_millis),
    thumbnail_url: text(&node["previewImageURL"]),
    tags: node["freeformTags"]
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(|t| non_empty(&t["name"]).map(str::to_owned))
      .collect(),
  })
}

fn user(node: &Value) -> Option<User> {
  Some(User {
    id: non_empty(&node["id"])?.to_owned(),
    login: text(&node["login"]),
    display_name: text(&node["displayName"]),
    profile_image_url: text(&node["profileImageURL"]),
    offline_image_url: text(&node["offlineImageURL"]),
    description: text(&node["description"]),
    partner: node["roles"]["isPartner"].as_bool().unwrap_or(false),
  })
}

fn video(node: &Value) -> Option<Video> {
  let owner = &node["owner"];
  Some(Video {
    id: non_empty(&node["id"])?.to_owned(),
    user_id: text(&owner["id"]),
    user_login: text(&owner["login"]),
    user_name: text(&owner["displayName"]),
    title: text(&node["title"]),
    created_at: node["createdAt"]
      .as_str()
      .and_then(crate::time::epoch_millis),
    view_count: node["viewCount"].as_u64().unwrap_or(0),
    duration: node["lengthSeconds"].as_u64().unwrap_or(0),
    thumbnail_url: text(&node["previewThumbnailURL"]),
    // `muteInfo` or its connection is null without any.
    muted_segments: node["muteInfo"]["mutedSegmentConnection"]["nodes"]
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
}

fn clip(node: &Value) -> Option<Clip> {
  Some(Clip {
    // Helix's clip id is GQL's slug.
    id: non_empty(&node["slug"])?.to_owned(),
    broadcaster_id: text(&node["broadcaster"]["id"]),
    broadcaster_name: text(&node["broadcaster"]["displayName"]),
    creator_name: text(&node["curator"]["displayName"]),
    title: text(&node["title"]),
    view_count: node["viewCount"].as_u64().unwrap_or(0),
    created_at: node["createdAt"]
      .as_str()
      .and_then(crate::time::epoch_millis),
    thumbnail_url: text(&node["thumbnailURL"]),
    duration: node["durationSeconds"].as_f64().unwrap_or(0.0),
  })
}

fn channel_result(node: &Value) -> Option<ChannelResult> {
  let stream = &node["stream"];
  let settings = &node["broadcastSettings"];
  Some(ChannelResult {
    id: non_empty(&node["id"])?.to_owned(),
    login: text(&node["login"]),
    display_name: text(&node["displayName"]),
    is_live: !stream.is_null(),
    game_name: non_empty(&stream["game"]["displayName"])
      .or_else(|| non_empty(&settings["game"]["displayName"]))
      .unwrap_or_default()
      .to_owned(),
    title: text(&settings["title"]),
    thumbnail_url: text(&node["profileImageURL"]),
  })
}

fn badge(node: &Value) -> Option<Badge> {
  Some(Badge {
    set: non_empty(&node["setID"])?.to_owned(),
    version: text(&node["version"]),
    title: text(&node["title"]),
    url_1x: non_empty(&node["image1x"])?.to_owned(),
    url_2x: text(&node["image2x"]),
    url_4x: text(&node["image4x"]),
  })
}

/// [`top_categories_request`]'s answer.
pub fn parse_top_categories(answer: &Value) -> Result<Vec<Category>> {
  let games = &answer["data"]["games"];
  listed(answer, games)?;
  Ok(nodes(games).filter_map(category).collect())
}

/// [`category_streams_request`]'s answer; an unknown category is [`Error::NotFound`].
pub fn parse_category_streams(answer: &Value) -> Result<Vec<Stream>> {
  let game = &answer["data"]["game"];
  if game.is_null() {
    return Err(gql::gql_error(answer).unwrap_or(Error::NotFound));
  }
  listed(answer, &game["streams"])?;
  Ok(nodes(&game["streams"]).filter_map(stream).collect())
}

/// [`users_request`]'s answer: the users found (an unknown id is left out).
pub fn parse_users(answer: &Value) -> Result<Vec<User>> {
  let users = &answer["data"]["users"];
  listed(answer, users)?;
  Ok(
    users
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(user)
      .collect(),
  )
}

/// [`user_request`]'s answer; an unknown login is [`Error::NotFound`].
pub fn parse_user(answer: &Value) -> Result<User> {
  let node = &answer["data"]["user"];
  failed(answer, node)?;
  user(node).ok_or(Error::NotFound)
}

/// [`videos_request`]'s answer.
pub fn parse_videos(answer: &Value) -> Result<Vec<Video>> {
  let videos = &answer["data"]["user"]["videos"];
  listed(answer, videos)?;
  Ok(nodes(videos).filter_map(video).collect())
}

/// [`video_request`]'s answer; an unknown VOD is [`Error::NotFound`].
pub fn parse_video(answer: &Value) -> Result<Video> {
  let node = &answer["data"]["video"];
  failed(answer, node)?;
  video(node).ok_or(Error::NotFound)
}

/// [`clips_request`]'s answer.
pub fn parse_clips(answer: &Value) -> Result<Vec<Clip>> {
  let clips = &answer["data"]["user"]["clips"];
  listed(answer, clips)?;
  Ok(nodes(clips).filter_map(clip).collect())
}

/// [`clip_request`]'s answer; an unknown clip is [`Error::NotFound`].
pub fn parse_clip(answer: &Value) -> Result<Clip> {
  let node = &answer["data"]["clip"];
  failed(answer, node)?;
  clip(node).ok_or(Error::NotFound)
}

/// [`search_channels_request`]'s answer.
pub fn parse_search_channels(answer: &Value) -> Result<Vec<ChannelResult>> {
  let found = &answer["data"]["searchUsers"];
  listed(answer, found)?;
  Ok(nodes(found).filter_map(channel_result).collect())
}

/// [`search_categories_request`]'s answer.
pub fn parse_search_categories(answer: &Value) -> Result<Vec<Category>> {
  let found = &answer["data"]["searchCategories"];
  listed(answer, found)?;
  Ok(nodes(found).filter_map(category).collect())
}

/// [`global_badges_request`]'s or [`channel_badges_request`]'s answer: every version of every set.
pub fn parse_badges(answer: &Value) -> Result<Vec<Badge>> {
  let data = &answer["data"];
  let list = if data["badges"].is_array() {
    &data["badges"]
  } else {
    &data["user"]["broadcastBadges"]
  };
  listed(answer, list)?;
  Ok(
    list
      .as_array()
      .into_iter()
      .flatten()
      .filter_map(badge)
      .collect(),
  )
}

/// The catalog: GQL first, through an anonymous client (a refused auth-token never stops the
/// browsing), then Helix when GQL fails and the user connected it ([`Catalog::with_helix`]):
/// Twitch changing or closing a raw query leaves the pages working. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Catalog {
  gql: Client,
  helix: Option<Helix>,
}

impl Catalog {
  /// The catalog through `client`'s Client-ID, without its auth-token.
  pub fn new(client: &Client) -> Self {
    Self {
      gql: client.anonymous(),
      helix: None,
    }
  }

  /// The same catalog with `helix` as its backup, and for the followed channels.
  pub fn with_helix(&self, helix: Option<Helix>) -> Self {
    Self {
      gql: self.gql.clone(),
      helix,
    }
  }

  /// The Helix client, when the user connected it (the followed channels need it).
  pub fn helix(&self) -> Option<&Helix> {
    self.helix.as_ref()
  }

  /// The answer to `request` read by `parse`; on a GQL failure (not a missing item, not the
  /// network) and with Helix connected, `helix`'s instead.
  async fn or_helix<T, H>(
    &self,
    request: Value,
    parse: fn(&Value) -> Result<T>,
    helix: impl FnOnce(Helix) -> H,
  ) -> Result<T>
  where
    H: Future<Output = Result<T>>,
  {
    let result = match self.gql.gql(&request).await {
      Ok(answer) => parse(&answer),
      Err(e) => Err(e),
    };
    match (result, &self.helix) {
      (
        Err(
          e @ (Error::IntegrityRequired
          | Error::Gql(_)
          | Error::Parse { .. }
          | Error::Status { .. }),
        ),
        Some(backup),
      ) => {
        tracing::warn!("Twitch GQL catalog failed ({e}), asking Helix");
        helix(backup.clone()).await
      }
      (result, _) => result,
    }
  }

  /// The categories with the most viewers.
  pub async fn top_categories(&self) -> Result<Vec<Category>> {
    let request = top_categories_request();
    self
      .or_helix(request, parse_top_categories, |h| async move {
        Ok(h.top_categories(None).await?.items)
      })
      .await
  }

  /// Category `id`'s live streams, most viewers first.
  pub async fn category_streams(&self, id: &str) -> Result<Vec<Stream>> {
    let request = category_streams_request(id);
    self
      .or_helix(request, parse_category_streams, |h| async move {
        Ok(h.category_streams(id, None).await?.items)
      })
      .await
  }

  /// Users by id, at most [`PAGE`].
  pub async fn users(&self, ids: &[String]) -> Result<Vec<User>> {
    if ids.is_empty() {
      return Ok(Vec::new());
    }
    let ids = &ids[..ids.len().min(PAGE as usize)];
    let request = users_request(ids);
    self
      .or_helix(request, parse_users, |h| async move { h.users(ids).await })
      .await
  }

  /// A user by login.
  pub async fn user(&self, login: &str) -> Result<User> {
    let request = user_request(login);
    self
      .or_helix(request, parse_user, |h| async move { h.user(login).await })
      .await
  }

  /// Channel `user_id`'s last `first` past broadcasts, newest first.
  pub async fn videos(&self, user_id: &str, first: u32) -> Result<Vec<Video>> {
    let request = videos_request(user_id, first);
    self
      .or_helix(request, parse_videos, |h| async move {
        Ok(h.videos(user_id, Some("archive"), None).await?.items)
      })
      .await
  }

  /// A VOD by id.
  pub async fn video(&self, id: &str) -> Result<Video> {
    let request = video_request(id);
    self
      .or_helix(request, parse_video, |h| async move { h.video(id).await })
      .await
  }

  /// Channel `user_id`'s `first` most viewed clips.
  pub async fn clips(&self, user_id: &str, first: u32) -> Result<Vec<Clip>> {
    let request = clips_request(user_id, first);
    self
      .or_helix(request, parse_clips, |h| async move {
        Ok(h.clips(user_id, None).await?.items)
      })
      .await
  }

  /// A clip by slug.
  pub async fn clip(&self, slug: &str) -> Result<Clip> {
    let request = clip_request(slug);
    self
      .or_helix(request, parse_clip, |h| async move { h.clip(slug).await })
      .await
  }

  /// Channels matching `text`.
  pub async fn search_channels(&self, text: &str) -> Result<Vec<ChannelResult>> {
    let request = search_channels_request(text);
    self
      .or_helix(request, parse_search_channels, |h| async move {
        Ok(h.search_channels(text).await?.items)
      })
      .await
  }

  /// Categories matching `text`.
  pub async fn search_categories(&self, text: &str) -> Result<Vec<Category>> {
    let request = search_categories_request(text);
    self
      .or_helix(request, parse_search_categories, |h| async move {
        Ok(h.search_categories(text).await?.items)
      })
      .await
  }

  /// The chat badges every channel shares (moderator, VIP, partner…).
  pub async fn global_badges(&self) -> Result<Vec<Badge>> {
    self
      .or_helix(global_badges_request(), parse_badges, |h| async move {
        h.global_badges().await
      })
      .await
  }

  /// Channel `user_id`'s own chat badges (subscriber months, bits).
  pub async fn channel_badges(&self, user_id: &str) -> Result<Vec<Badge>> {
    let request = channel_badges_request(user_id);
    self
      .or_helix(request, parse_badges, |h| async move {
        h.channel_badges(user_id).await
      })
      .await
  }
}
