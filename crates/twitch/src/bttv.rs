//! BetterTTV emotes for the chat: the global set and a channel's (its own emotes and those it
//! shares), keyed by the broadcaster's Twitch user id, matched by word in messages like 7TV's.
//! Tested on saved responses (`tests/fixtures/bttv_*.json`).

use serde_json::Value;

use crate::error::{Error, Result};

const API: &str = "https://api.betterttv.net/3/cached";

/// Height of BTTV's 1x files, pixels; emotes without a size are square.
const BASE_HEIGHT: u32 = 28;

/// Global emotes drawn over the emote before them (hats, masks…), as BTTV and Chatterino draw them.
const OVERLAYS: [&str; 8] = [
  "SoSnowy",
  "IceCold",
  "SantaHat",
  "TopHat",
  "ReinDeer",
  "CandyCane",
  "cvMask",
  "cvHazmat",
];

/// An emote of a BTTV set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Emote {
  /// The word it replaces.
  pub name: String,
  /// BTTV's emote id.
  pub id: String,
  /// Size of its 1x file, pixels.
  pub width: u32,
  /// Height of its 1x file, pixels.
  pub height: u32,
  /// Several frames.
  pub animated: bool,
  /// Drawn over the emote before it.
  pub zero_width: bool,
}

impl Emote {
  /// The URL of its WebP at `scale` (1, 2 or 3).
  pub fn url(&self, scale: u32) -> String {
    format!(
      "https://cdn.betterttv.net/emote/{}/{}x.webp",
      self.id,
      scale.clamp(1, 3)
    )
  }
}

/// The emotes of a list of set entries. Entries without an id or a code are skipped, and so are
/// modifiers (`w!`, `h!`…): they change the emote after them and are not images of their own.
fn parse_list(list: Option<&Value>) -> Vec<Emote> {
  list
    .and_then(Value::as_array)
    .map(|entries| entries.iter().filter_map(emote).collect())
    .unwrap_or_default()
}

fn emote(entry: &Value) -> Option<Emote> {
  if entry.get("modifier").and_then(Value::as_bool) == Some(true) {
    return None;
  }
  let name = entry.get("code")?.as_str()?.to_owned();
  let id = entry.get("id")?.as_str()?.to_owned();
  let size = |key| {
    entry
      .get(key)
      .and_then(Value::as_u64)
      .and_then(|v| u32::try_from(v).ok())
      .filter(|v| *v > 0)
  };
  let (width, height) = match (size("width"), size("height")) {
    (Some(width), Some(height)) => (width, height),
    _ => (BASE_HEIGHT, BASE_HEIGHT),
  };
  Some(Emote {
    zero_width: OVERLAYS.contains(&name.as_str()),
    name,
    id,
    width,
    height,
    animated: entry
      .get("animated")
      .and_then(Value::as_bool)
      .unwrap_or(false),
  })
}

fn json(text: &str) -> Result<Value> {
  serde_json::from_str(text).map_err(|e| Error::Parse {
    endpoint: "bttv",
    detail: e.to_string(),
  })
}

/// A Twitch user's BTTV response: their own emotes, then the ones they share. A user without BTTV
/// answers 404 (`Client::bttv_channel` gives no emotes then).
pub fn parse_user(text: &str) -> Result<Vec<Emote>> {
  let value = json(text)?;
  let mut emotes = parse_list(value.get("channelEmotes"));
  emotes.extend(parse_list(value.get("sharedEmotes")));
  Ok(emotes)
}

/// The global set's response.
pub fn parse_global(text: &str) -> Result<Vec<Emote>> {
  Ok(parse_list(Some(&json(text)?)))
}

/// The BTTV emotes of the channel whose Twitch user id is `user_id`; empty when it has no BTTV account.
pub(crate) async fn channel(http: &reqwest::Client, user_id: &str) -> Result<Vec<Emote>> {
  let response = http
    .get(format!("{API}/users/twitch/{user_id}"))
    .send()
    .await?;
  match response.status().as_u16() {
    200 => parse_user(&response.text().await?),
    404 => Ok(Vec::new()),
    status => Err(Error::Status {
      status,
      endpoint: "bttv",
      code: None,
    }),
  }
}

/// BTTV's global emotes.
pub(crate) async fn global(http: &reqwest::Client) -> Result<Vec<Emote>> {
  let response = http.get(format!("{API}/emotes/global")).send().await?;
  match response.status().as_u16() {
    200 => parse_global(&response.text().await?),
    status => Err(Error::Status {
      status,
      endpoint: "bttv",
      code: None,
    }),
  }
}
