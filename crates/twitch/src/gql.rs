//! GQL (`POST https://gql.twitch.tv/gql`), Twitch's private API: the playback token of a live or
//! a VOD. The raw query is used, not a persisted hash: it answers with the web Client-ID and
//! needs no hash to keep up to date (docs/twitch.md, "Checked live").

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::chat::{ChatLine, EmoteRange};
use crate::error::{Error, Result};

/// Client-ID of twitch.tv's own web player, used until [`client_id_from_page`] reads the current
/// one. It has not changed for years, but Streamlink has had to follow changes of the hashes.
pub const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// `playerType` of the main token: `site` gets a pre-roll, `embed` did not (measured 2026-11-01).
pub const PLAYER_TYPE: &str = "embed";

/// `playerType` of the backup token, whose playlist fills an ad break of the main one: no ads and
/// 1080p60 on 2026-11-01, as `frontpage` (`autoplay` stops at 360p).
pub const BACKUP_PLAYER_TYPE: &str = "popout";

/// yt-dlp's text of the query (`yt_dlp/extractor/twitch.py`), both kinds in one.
const TOKEN_QUERY: &str = "query PlaybackAccessToken_Template($login: String!, $isLive: Boolean!, \
  $vodID: ID!, $isVod: Boolean!, $playerType: String!, $platform: String!) { \
  streamPlaybackAccessToken(channelName: $login, params: {platform: $platform, playerBackend: \
  \"mediaplayer\", playerType: $playerType}) @include(if: $isLive) { value signature } \
  videoPlaybackAccessToken(id: $vodID, params: {platform: $platform, playerBackend: \"mediaplayer\", \
  playerType: $playerType}) @include(if: $isVod) { value signature } }";

/// What to play.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
  /// A channel's live, by login (lowercase).
  Live(String),
  /// A VOD, by id.
  Vod(String),
}

impl Target {
  fn token_key(&self) -> &'static str {
    match self {
      Self::Live(_) => "streamPlaybackAccessToken",
      Self::Vod(_) => "videoPlaybackAccessToken",
    }
  }
}

/// A playback token. `value` is JSON with the user's IP and ids, `signature` signs it: neither
/// may be logged, so `Debug` leaves them out.
#[derive(Clone)]
pub struct PlaybackToken {
  /// The token (usher's `token` or `nauth`).
  pub value: String,
  /// Its signature (usher's `sig` or `nauthsig`).
  pub signature: String,
  /// Qualities kept for the channel's subscribers (`chansub.restricted_bitrates`), e.g.
  /// `1080p60`, `chunked`. All of them: a subscriber-only VOD.
  pub restricted: Vec<String>,
}

impl std::fmt::Debug for PlaybackToken {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("PlaybackToken")
      .field("restricted", &self.restricted)
      .finish_non_exhaustive()
  }
}

impl PlaybackToken {
  /// Whether nothing is left without a subscription: the token lists `audio_only` among the
  /// restricted qualities (it is the last resort of a public stream).
  pub fn subscriber_only(&self) -> bool {
    self.restricted.iter().any(|q| q == "audio_only")
  }
}

/// The request body for `target`'s token.
pub fn token_request(target: &Target, player_type: &str) -> Value {
  let (live, login, vod) = match target {
    Target::Live(login) => (true, login.as_str(), ""),
    Target::Vod(id) => (false, "", id.as_str()),
  };
  json!({
    "operationName": "PlaybackAccessToken_Template",
    "query": TOKEN_QUERY,
    "variables": {
      "login": login,
      "isLive": live,
      "vodID": vod,
      "isVod": !live,
      "playerType": player_type,
      "platform": "web",
    },
  })
}

/// Reads the token from a GQL answer (HTTP 200).
pub fn parse_token(answer: &Value, target: &Target) -> Result<PlaybackToken> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let token = &answer["data"][target.token_key()];
  if token.is_null() {
    return Err(Error::NotFound);
  }
  let (Some(value), Some(signature)) = (token["value"].as_str(), token["signature"].as_str())
  else {
    return Err(Error::Parse {
      endpoint: "gql",
      detail: "token without value or signature".into(),
    });
  };
  // The value is JSON in a string; a value that does not parse still plays.
  let restricted = serde_json::from_str::<Value>(value)
    .ok()
    .and_then(|v| {
      v["chansub"]["restricted_bitrates"].as_array().map(|list| {
        list
          .iter()
          .filter_map(|q| q.as_str().map(str::to_owned))
          .collect()
      })
    })
    .unwrap_or_default();
  Ok(PlaybackToken {
    value: value.to_owned(),
    signature: signature.to_owned(),
    restricted,
  })
}

/// The error of a GQL answer that is not HTTP 200: 401 is a refused auth-token.
pub fn parse_status(status: u16, body: &str) -> Error {
  if status == 401 {
    return Error::NotLoggedIn;
  }
  let answer: Value = serde_json::from_str(body).unwrap_or(Value::Null);
  gql_error(&answer).unwrap_or(Error::Status {
    status,
    endpoint: "gql",
    code: None,
  })
}

/// `errors[0]` of an answer, or a top-level `error`/`message` (the 401 shape). An integrity failure
/// is its own error: Streamlink sees `failed integrity check` in the message. **Not seen yet**: no
/// fixture, anonymous tokens did not ask for one.
pub(crate) fn gql_error(answer: &Value) -> Option<Error> {
  let message = answer["errors"][0]["message"]
    .as_str()
    .or_else(|| answer["message"].as_str())
    .or_else(|| answer["error"].as_str())?;
  Some(if message.to_ascii_lowercase().contains("integrity") {
    Error::IntegrityRequired
  } else {
    Error::Gql(message.to_owned())
  })
}

/// A channel and its live, for the watch page's header.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
  /// Numeric id (third-party emotes and badges are keyed by it).
  pub id: String,
  /// Login, lowercase.
  pub login: String,
  /// Shown name, e.g. `Squeezie`.
  pub display_name: String,
  /// Avatar, 70 px.
  pub avatar_url: Option<String>,
  /// The broadcast's title (kept when offline).
  pub title: String,
  /// The live, when the channel is on air.
  pub live: Option<Live>,
}

/// A channel's live.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
  /// Current viewers.
  pub viewers: u64,
  /// Start, milliseconds since the epoch.
  pub started_at: Option<i64>,
  /// Category (game) name.
  pub category: Option<String>,
}

const CHANNEL_QUERY: &str = "query Channel($login: String!) { user(login: $login) { id login \
  displayName profileImageURL(width: 70) broadcastSettings { title } stream { id type viewersCount \
  createdAt game { id displayName } } } }";

/// The request for `login`'s channel and live.
pub fn channel_request(login: &str) -> Value {
  json!({
    "operationName": "Channel",
    "query": CHANNEL_QUERY,
    "variables": { "login": login },
  })
}

/// Reads a channel; `user: null` is [`Error::NotFound`].
pub fn parse_channel(answer: &Value) -> Result<Channel> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let user = &answer["data"]["user"];
  if user.is_null() {
    return Err(Error::NotFound);
  }
  let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
  let stream = &user["stream"];
  let live = (!stream.is_null()).then(|| Live {
    viewers: stream["viewersCount"].as_u64().unwrap_or(0),
    started_at: stream["createdAt"]
      .as_str()
      .and_then(crate::time::epoch_millis),
    category: stream["game"]["displayName"].as_str().map(str::to_owned),
  });
  Ok(Channel {
    id: text(&user["id"]),
    login: text(&user["login"]),
    display_name: text(&user["displayName"]),
    avatar_url: user["profileImageURL"].as_str().map(str::to_owned),
    title: text(&user["broadcastSettings"]["title"]),
    live,
  })
}

/// The request asking who the auth-token belongs to.
pub fn current_user_request() -> Value {
  json!({
    "query": "query { currentUser { id login displayName profileImageURL(width: 70) } }"
  })
}

/// The account an auth-token belongs to: the space's identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
  /// Numeric id.
  pub id: String,
  /// Login, lowercase.
  pub login: String,
  /// Shown name.
  pub display_name: String,
  /// Avatar, 70 px.
  pub avatar_url: Option<String>,
}

/// The account of `currentUser`; `null` (no or a dead auth-token) is [`Error::NotLoggedIn`].
pub fn parse_account(answer: &Value) -> Result<Account> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let user = &answer["data"]["currentUser"];
  let (Some(id), Some(login)) = (user["id"].as_str(), user["login"].as_str()) else {
    return Err(Error::NotLoggedIn);
  };
  Ok(Account {
    id: id.to_owned(),
    login: login.to_owned(),
    display_name: user["displayName"]
      .as_str()
      .filter(|n| !n.is_empty())
      .unwrap_or(login)
      .to_owned(),
    avatar_url: user["profileImageURL"]
      .as_str()
      .filter(|u| !u.is_empty())
      .map(str::to_owned),
  })
}

/// The login of `currentUser`; `null` (no or a dead auth-token) is [`Error::NotLoggedIn`].
pub fn parse_current_user(answer: &Value) -> Result<String> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  answer["data"]["currentUser"]["login"]
    .as_str()
    .map(str::to_owned)
    .ok_or(Error::NotLoggedIn)
}

/// Whether `token` looks like a web `auth-token`: 30 letters and digits (seen lowercase).
pub fn is_auth_token_shape(token: &str) -> bool {
  token.len() == 30 && token.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The web Client-ID in twitch.tv's page (`clientId="…"` in its inline script).
pub fn client_id_from_page(page: &str) -> Option<&str> {
  let start = page.find("clientId=\"")? + "clientId=\"".len();
  let id = &page[start..];
  let id = &id[..id.find('"')?];
  (id.len() == 30 && id.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(id)
}

/// A clip's playable files, best first. Their URLs carry the playback token: never log them.
#[derive(Clone)]
pub struct ClipPlayback {
  /// Each quality.
  pub qualities: Vec<ClipQuality>,
}

/// One quality of a clip: a direct MP4.
#[derive(Clone)]
pub struct ClipQuality {
  /// Height, e.g. 1080.
  pub height: u32,
  /// Frame rate, rounded.
  pub frame_rate: u32,
  /// `sourceURL` signed with the token (`?sig=…&token=…`).
  pub url: String,
}

impl std::fmt::Debug for ClipPlayback {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let labels: Vec<String> = self
      .qualities
      .iter()
      .map(|q| format!("{}p{}", q.height, q.frame_rate))
      .collect();
    f.debug_struct("ClipPlayback")
      .field("qualities", &labels)
      .finish()
  }
}

const CLIP_QUERY: &str = "query ClipPlayback($slug: ID!) { clip(slug: $slug) { id slug title \
  durationSeconds playbackAccessToken(params: {platform: \"web\", playerBackend: \"mediaplayer\", \
  playerType: \"site\"}) { signature value } videoQualities { frameRate quality sourceURL } } }";

/// The request for a clip's files.
pub fn clip_request(slug: &str) -> Value {
  json!({ "operationName": "ClipPlayback", "query": CLIP_QUERY, "variables": { "slug": slug } })
}

/// Reads a clip's files; `clip: null` is [`Error::NotFound`].
pub fn parse_clip_playback(answer: &Value) -> Result<ClipPlayback> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let clip = &answer["data"]["clip"];
  if clip.is_null() {
    return Err(Error::NotFound);
  }
  let token = &clip["playbackAccessToken"];
  let (Some(value), Some(signature)) = (token["value"].as_str(), token["signature"].as_str())
  else {
    return Err(Error::Parse {
      endpoint: "gql",
      detail: "clip without a playback token".into(),
    });
  };
  let mut qualities: Vec<ClipQuality> = clip["videoQualities"]
    .as_array()
    .map(|list| {
      list
        .iter()
        .filter_map(|q| {
          let source = q["sourceURL"].as_str()?;
          let url =
            reqwest::Url::parse_with_params(source, &[("sig", signature), ("token", value)])
              .ok()?
              .to_string();
          Some(ClipQuality {
            height: q["quality"].as_str()?.parse().ok()?,
            frame_rate: q["frameRate"].as_f64().map_or(30, |f| f.round() as u32),
            url,
          })
        })
        .collect()
    })
    .unwrap_or_default();
  qualities.sort_by_key(|q| std::cmp::Reverse((q.height, q.frame_rate)));
  if qualities.is_empty() {
    return Err(Error::Parse {
      endpoint: "gql",
      detail: "clip without a playable quality".into(),
    });
  }
  Ok(ClipPlayback { qualities })
}

/// The request for a VOD's seek previews (`seekPreviewsURL`).
pub fn seek_previews_request(id: &str) -> Value {
  json!({
    "operationName": "SeekPreviews",
    "query": "query SeekPreviews($id: ID!) { video(id: $id) { id lengthSeconds seekPreviewsURL } }",
    "variables": { "id": id },
  })
}

/// The storyboard's URL, when the VOD has one.
pub fn parse_seek_previews(answer: &Value) -> Option<String> {
  answer["data"]["video"]["seekPreviewsURL"]
    .as_str()
    .filter(|u| !u.is_empty())
    .map(str::to_owned)
}

/// One level of a VOD's storyboard: frames every `interval` seconds on sheets of `cols`×`rows`.
#[derive(Clone, Debug, PartialEq)]
pub struct StoryboardLevel {
  /// Frames in all.
  pub count: u32,
  /// Size of one frame.
  pub width: u32,
  /// See [`StoryboardLevel::width`].
  pub height: u32,
  /// Frames per row and rows per sheet.
  pub cols: u32,
  /// See [`StoryboardLevel::cols`].
  pub rows: u32,
  /// Seconds between frames.
  pub interval: f64,
  /// The sheets, absolute URLs.
  pub images: Vec<String>,
}

impl StoryboardLevel {
  /// The sheet, column and row of the frame at `seconds`.
  pub fn frame_at(&self, seconds: f64) -> Option<(&str, u32, u32)> {
    let per_sheet = self.cols.checked_mul(self.rows).filter(|&n| n > 0)?;
    let index =
      ((seconds.max(0.0) / self.interval.max(1.0)) as u32).min(self.count.saturating_sub(1));
    let sheet = self.images.get((index / per_sheet) as usize)?;
    let within = index % per_sheet;
    Some((sheet, within % self.cols, within / self.cols))
  }
}

/// Reads the storyboard JSON at `url`: its sheets are named relative to it. The level with the
/// largest frames comes first.
pub fn parse_storyboard(text: &str, url: &str) -> Vec<StoryboardLevel> {
  let base = url.rsplit_once('/').map_or(url, |(base, _)| base);
  let levels: Value = serde_json::from_str(text).unwrap_or(Value::Null);
  let mut levels: Vec<StoryboardLevel> = levels
    .as_array()
    .map(|list| {
      list
        .iter()
        .filter_map(|l| {
          let images: Vec<String> = l["images"]
            .as_array()?
            .iter()
            .filter_map(|i| i.as_str().map(|name| format!("{base}/{name}")))
            .collect();
          (!images.is_empty()).then(|| StoryboardLevel {
            count: l["count"].as_u64().unwrap_or(0) as u32,
            width: l["width"].as_u64().unwrap_or(0) as u32,
            height: l["height"].as_u64().unwrap_or(0) as u32,
            cols: l["cols"].as_u64().unwrap_or(0) as u32,
            rows: l["rows"].as_u64().unwrap_or(0) as u32,
            interval: l["interval"].as_f64().unwrap_or(0.0),
            images,
          })
        })
        .collect()
    })
    .unwrap_or_default();
  levels.sort_by_key(|l| std::cmp::Reverse(l.width));
  levels
}

/// A VOD's game chapters (`video.moments`, twitch.tv's chapter button), raw: it answers anonymously
/// with the web Client-ID (checked 2026-11-03). A VOD that kept one game has none. Shared by
/// [`chapters_request`] and [`chapters_many_request`].
const CHAPTERS_FIELDS: &str = "moments(first: 100, momentRequestType: VIDEO_CHAPTER_MARKERS, \
  types: [GAME_CHANGE]) { edges { node { positionMilliseconds description details { ... on \
  GameChangeMomentDetails { game { id displayName boxArtURL } } } } } }";

/// A VOD's chapter: the game played from `start` until the next chapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
  /// Seconds from the VOD's start.
  pub start: u32,
  /// The game's name.
  pub game: String,
  /// The game's box art, a size template (`{width}x{height}`) as Helix's.
  pub box_art: Option<String>,
}

/// The request for VOD `id`'s game chapters.
pub fn chapters_request(id: &str) -> Value {
  json!({
    "operationName": "VideoChapters",
    "query": format!("query VideoChapters($id: ID!) {{ video(id: $id) {{ id {CHAPTERS_FIELDS} }} }}"),
    "variables": { "id": id },
  })
}

/// Reads the chapters in order; a node without a position or a name is skipped. `video: null`
/// is [`Error::NotFound`].
pub fn parse_chapters(answer: &Value) -> Result<Vec<Chapter>> {
  let video = &answer["data"]["video"];
  if video.is_null() {
    return Err(gql_error(answer).unwrap_or(Error::NotFound));
  }
  Ok(chapters_of(video))
}

/// The request for the game chapters of the VODs `ids`, one aliased `video` per VOD (`v0`,
/// `v1`…), as [`positions_request`]. Ids that are not numbers are left out: they go into the
/// query's text.
pub fn chapters_many_request(ids: &[String]) -> Value {
  let fields: String = ids
    .iter()
    .enumerate()
    .filter(|(_, id)| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
    .map(|(i, id)| format!(" v{i}: video(id: \"{id}\") {{ id {CHAPTERS_FIELDS} }}"))
    .collect();
  json!({
    "operationName": "VideoChaptersMany",
    "query": format!("query VideoChaptersMany {{{fields} }}"),
  })
}

/// The chapters in [`chapters_many_request`]'s answer, by VOD id; the VODs that kept one game
/// (no chapters) or were not found are left out.
pub fn parse_chapters_many(answer: &Value) -> Result<HashMap<String, Vec<Chapter>>> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  Ok(
    answer["data"]
      .as_object()
      .into_iter()
      .flatten()
      .filter_map(|(_, video)| {
        let id = video["id"].as_str()?.to_owned();
        let chapters = chapters_of(video);
        (!chapters.is_empty()).then_some((id, chapters))
      })
      .collect(),
  )
}

/// A `video`'s chapters, in order.
fn chapters_of(video: &Value) -> Vec<Chapter> {
  let mut chapters: Vec<Chapter> = video["moments"]["edges"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|edge| {
      let node = &edge["node"];
      let start = u32::try_from(node["positionMilliseconds"].as_u64()? / 1000).ok()?;
      let details = &node["details"]["game"];
      let game = details["displayName"]
        .as_str()
        .or_else(|| node["description"].as_str())
        .filter(|name| !name.is_empty())?;
      Some(Chapter {
        start,
        game: game.to_owned(),
        box_art: details["boxArtURL"]
          .as_str()
          .filter(|url| !url.is_empty())
          .map(str::to_owned),
      })
    })
    .collect();
  chapters.sort_by_key(|c| c.start);
  chapters
}

/// A VOD's chat replay around an offset, a page at a time.
const COMMENTS_QUERY: &str = "query VideoComments($id: ID!, $offset: Int) { video(id: $id) { \
  owner { id } comments(contentOffsetSeconds: $offset) { edges { node { id contentOffsetSeconds \
  createdAt commenter { id login displayName } message { fragments { text emote { emoteID } } \
  userBadges { setID version } userColor } } } pageInfo { hasNextPage } } } }";

/// A page of a VOD's chat: about 60 messages from a little before the offset asked for (a few
/// seconds on a busy chat, up to half a minute on a quiet one), oldest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comments {
  /// The VOD's channel id (its 7TV emotes and badges).
  pub channel_id: Option<String>,
  /// The messages, each with its offset in the VOD.
  pub comments: Vec<Comment>,
  /// Whether messages follow the page's last one.
  pub more: bool,
}

/// A message of a VOD's chat.
#[derive(Clone, Debug, PartialEq)]
pub struct Comment {
  /// Seconds from the VOD's start.
  pub offset: u32,
  /// The message, as the live chat's (its emotes by `char` range).
  pub line: ChatLine,
}

/// The request for `id`'s chat from about `offset` seconds. Only the offset pages: the `after`
/// cursor fails Twitch's integrity check (2026-11-02), so it is never sent.
pub fn comments_request(id: &str, offset: u32) -> Value {
  json!({
    "operationName": "VideoComments",
    "query": COMMENTS_QUERY,
    "variables": { "id": id, "offset": offset },
  })
}

/// Reads a page of comments; `video: null` is [`Error::NotFound`], `comments: null` an empty last
/// page (past the end). A node without its author or text is skipped.
pub fn parse_comments(answer: &Value) -> Result<Comments> {
  let video = &answer["data"]["video"];
  if video.is_null() {
    return Err(gql_error(answer).unwrap_or(Error::NotFound));
  }
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let page = &video["comments"];
  let comments = page["edges"]
    .as_array()
    .map(|edges| edges.iter().filter_map(|e| comment(&e["node"])).collect())
    .unwrap_or_default();
  Ok(Comments {
    channel_id: video["owner"]["id"].as_str().map(str::to_owned),
    comments,
    more: page["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false),
  })
}

fn comment(node: &Value) -> Option<Comment> {
  let author = &node["commenter"];
  let login = author["login"].as_str()?;
  let message = &node["message"];
  let mut line = ChatLine {
    id: node["id"].as_str()?.to_owned(),
    user_id: author["id"].as_str().unwrap_or_default().to_owned(),
    login: login.to_owned(),
    display_name: author["displayName"]
      .as_str()
      .filter(|n| !n.is_empty())
      .unwrap_or(login)
      .to_owned(),
    color: message["userColor"]
      .as_str()
      .filter(|c| !c.is_empty())
      .map(str::to_owned),
    badges: message["userBadges"]
      .as_array()
      .map(|badges| {
        badges
          .iter()
          .filter_map(|b| {
            let set = b["setID"].as_str().filter(|s| !s.is_empty())?;
            Some((set.to_owned(), b["version"].as_str()?.to_owned()))
          })
          .collect()
      })
      .unwrap_or_default(),
    sent: node["createdAt"]
      .as_str()
      .and_then(crate::time::epoch_millis),
    ..ChatLine::default()
  };
  // The fragments' texts end to end, the emotes' ranges counted in `char`s.
  let mut at = 0;
  for fragment in message["fragments"].as_array()? {
    let Some(text) = fragment["text"].as_str() else {
      continue;
    };
    let len = text.chars().count();
    if let Some(id) = fragment["emote"]["emoteID"].as_str() {
      line.emotes.push(EmoteRange {
        id: id.to_owned(),
        start: at,
        end: at + len,
      });
    }
    line.text.push_str(text);
    at += len;
  }
  Some(Comment {
    offset: node["contentOffsetSeconds"]
      .as_u64()
      .and_then(|s| u32::try_from(s).ok())
      .unwrap_or(0),
    line,
  })
}

const VIEWING_QUERY: &str = "query VideoViewing($id: ID!) { currentUser { id } video(id: $id) { id \
  lengthSeconds self { viewingHistory { position } } } }";

/// twitch.tv's own mutation for a VOD's resume position (`updateUserViewedVideo`); it passes
/// without an integrity token (checked 2026-11-02).
const VIEWED_MUTATION: &str = "mutation UpdateUserViewedVideo($input: UpdateUserViewedVideoInput!) \
  { updateUserViewedVideo(input: $input) { __typename } }";

/// Where the viewer stopped in a VOD, on their account (`video.self.viewingHistory`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewing {
  /// The viewer's user id, for [`viewed_request`]; `None` without an auth-token.
  pub user_id: Option<String>,
  /// Seconds from the VOD's start; `None` when never watched or signed out.
  pub position: Option<u32>,
  /// The VOD's length, seconds.
  pub length: u32,
}

/// The request for the viewer's position in VOD `id`.
pub fn viewing_request(id: &str) -> Value {
  json!({
    "operationName": "VideoViewing",
    "query": VIEWING_QUERY,
    "variables": { "id": id },
  })
}

/// Reads [`Viewing`]; `video: null` is [`Error::NotFound`].
pub fn parse_viewing(answer: &Value) -> Result<Viewing> {
  let video = &answer["data"]["video"];
  if video.is_null() {
    return Err(gql_error(answer).unwrap_or(Error::NotFound));
  }
  let as_u32 = |v: &Value| v.as_u64().and_then(|s| u32::try_from(s).ok());
  Ok(Viewing {
    user_id: answer["data"]["currentUser"]["id"]
      .as_str()
      .map(str::to_owned),
    position: as_u32(&video["self"]["viewingHistory"]["position"]),
    length: as_u32(&video["lengthSeconds"]).unwrap_or(0),
  })
}

/// The request for the viewer's positions in the VODs `ids`, one aliased `video` per VOD (`v0`,
/// `v1`…). Ids that are not numbers are left out: they go into the query's text.
pub fn positions_request(ids: &[String]) -> Value {
  let fields: String = ids
    .iter()
    .enumerate()
    .filter(|(_, id)| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
    .map(|(i, id)| {
      format!(" v{i}: video(id: \"{id}\") {{ id self {{ viewingHistory {{ position }} }} }}")
    })
    .collect();
  json!({
    "operationName": "VideoPositions",
    "query": format!("query VideoPositions {{{fields} }}"),
  })
}

/// The positions in [`positions_request`]'s answer, by VOD id: the VODs never watched (or
/// without an auth-token) are left out.
pub fn parse_positions(answer: &Value) -> Result<HashMap<String, u32>> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  Ok(
    answer["data"]
      .as_object()
      .into_iter()
      .flatten()
      .filter_map(|(_, video)| {
        let id = video["id"].as_str()?.to_owned();
        let position = video["self"]["viewingHistory"]["position"].as_u64()?;
        Some((id, u32::try_from(position).ok().filter(|&p| p > 0)?))
      })
      .collect(),
  )
}

/// The request saving `position` seconds as `user_id`'s resume position in VOD `id`.
pub fn viewed_request(user_id: &str, id: &str, position: u32) -> Value {
  json!({
    "operationName": "UpdateUserViewedVideo",
    "query": VIEWED_MUTATION,
    "variables": { "input": {
      "userID": user_id,
      "videoID": id,
      "position": position,
      "videoType": "VOD",
    }},
  })
}

/// Whether the mutation went through.
pub fn parse_viewed(answer: &Value) -> Result<()> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  if answer["data"]["updateUserViewedVideo"].is_null() {
    return Err(Error::Parse {
      endpoint: "gql",
      detail: "updateUserViewedVideo: null".into(),
    });
  }
  Ok(())
}

/// The request for a VOD's info: `broadcastType` is `ARCHIVE` or `HIGHLIGHT`, `seekPreviewsURL` is
/// the storyboard's JSON, `owner.login` the channel's login.
pub fn video_request(id: &str) -> Value {
  json!({
    "query": format!(r#"query {{ video(id: "{}") {{ broadcastType, createdAt, seekPreviewsURL, owner {{ login }} }} }}"#, id),
  })
}
