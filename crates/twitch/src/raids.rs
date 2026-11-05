//! A channel's outgoing raid, through raw GQL (no persisted hash), checked on 2026-11-04 against
//! GQL's validation, anonymous (`twitch_capture -- gql-probe`; introspection is disabled):
//! `user(login) { raid }` is the raid the channel is about to send, null without one: `id`,
//! `viewerCount`, `targetChannel` and `sourceChannel` (`User`s). No field gives its countdown or
//! when it goes: the source's live ends when it does.

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::gql::gql_error;

const RAID_QUERY: &str = "query Raid($login: String!) { user(login: $login) { raid { id \
  viewerCount targetChannel { id login displayName profileImageURL(width: 70) } \
  sourceChannel { login } } } }";

/// The request for the raid `login`'s channel is about to send.
pub fn raid_request(login: &str) -> Value {
  json!({
    "operationName": "Raid",
    "query": RAID_QUERY,
    "variables": { "login": login },
  })
}

/// A raid about to leave a channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Raid {
  /// Id.
  pub id: String,
  /// The raided channel's login (its live opens when the raid goes).
  pub target_login: String,
  /// The raided channel's shown name.
  pub target_name: String,
  /// The raided channel's avatar, 70 px.
  pub target_avatar: Option<String>,
  /// The viewers it brings.
  pub viewer_count: u64,
}

/// Reads the raid about to leave: `None` without one; an unknown channel is [`Error::NotFound`].
pub fn parse_raid(answer: &Value) -> Result<Option<Raid>> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let user = &answer["data"]["user"];
  if user.is_null() {
    return Err(Error::NotFound);
  }
  let raid = &user["raid"];
  let target = &raid["targetChannel"];
  let (Some(id), Some(login)) = (raid["id"].as_str(), target["login"].as_str()) else {
    return Ok(None);
  };
  Ok(Some(Raid {
    id: id.to_owned(),
    target_login: login.to_owned(),
    target_name: target["displayName"]
      .as_str()
      .filter(|n| !n.is_empty())
      .unwrap_or(login)
      .to_owned(),
    target_avatar: target["profileImageURL"].as_str().map(str::to_owned),
    viewer_count: raid["viewerCount"].as_u64().unwrap_or(0),
  }))
}
