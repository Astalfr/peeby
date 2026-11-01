//! GQL (`POST https://gql.twitch.tv/gql`), Twitch's private API: the playback token of a live or
//! a VOD. The raw query is used, not a persisted hash: it answers with the web Client-ID and
//! needs no hash to keep up to date (docs/twitch.md, "Checked live").

use serde_json::{Value, json};

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
fn gql_error(answer: &Value) -> Option<Error> {
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

/// The request asking who the auth-token belongs to.
pub fn current_user_request() -> Value {
  json!({ "query": "query { currentUser { id login } }" })
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
