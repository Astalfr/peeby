//! 7TV emotes for the chat (docs/twitch-plan.md, step 9): the global set and a channel's set,
//! keyed by the broadcaster's Twitch user id, matched by word in messages. A channel's alias of an
//! emote is the set entry's `name`, not the emote's own. Tested on saved responses
//! (`tests/fixtures/seventv_*.json`).

use serde_json::Value;

use crate::error::{Error, Result};

const API: &str = "https://7tv.io/v3";

/// An emote of a 7TV set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Emote {
  /// The word it replaces in this set (the channel's alias).
  pub name: String,
  /// 7TV's emote id.
  pub id: String,
  /// Size of its smallest file, pixels (7TV's 1x is 32 px high).
  pub width: u32,
  /// Height of its smallest file, pixels.
  pub height: u32,
  /// Several frames (drawn still for now).
  pub animated: bool,
  /// Drawn over the emote before it (an overlay such as rain).
  pub zero_width: bool,
  /// `https://cdn.7tv.app/emote/<id>`.
  base: String,
  /// WebP files, smallest first: width, name, name of the still first frame.
  files: Vec<(u32, String, String)>,
}

impl Emote {
  /// The URL of the smallest WebP at least `width` pixels wide (else the largest); its still first
  /// frame when `still`.
  pub fn url(&self, width: u32, still: bool) -> Option<String> {
    let (_, name, static_name) = self
      .files
      .iter()
      .find(|(w, _, _)| *w >= width)
      .or_else(|| self.files.last())?;
    Some(format!(
      "{}/{}",
      self.base,
      if still { static_name } else { name }
    ))
  }
}

/// The emotes of a set object (`emote_set` of a user, or a set itself). Entries without a WebP file
/// or a name are skipped.
pub fn parse_set(set: &Value) -> Vec<Emote> {
  let Some(emotes) = set.get("emotes").and_then(Value::as_array) else {
    return Vec::new();
  };
  emotes.iter().filter_map(emote).collect()
}

fn emote(entry: &Value) -> Option<Emote> {
  let name = entry.get("name")?.as_str()?.to_owned();
  let data = entry.get("data")?;
  let host = data.get("host")?;
  let base = host.get("url")?.as_str()?;
  let base = if base.starts_with("//") {
    format!("https:{base}")
  } else {
    base.to_owned()
  };
  let mut files: Vec<(u32, u32, String, String)> = host
    .get("files")?
    .as_array()?
    .iter()
    .filter(|f| f.get("format").and_then(Value::as_str) == Some("WEBP"))
    .filter_map(|f| {
      let name = f.get("name")?.as_str()?.to_owned();
      let static_name = f
        .get("static_name")
        .and_then(Value::as_str)
        .map_or_else(|| name.clone(), str::to_owned);
      let width = u32::try_from(f.get("width")?.as_u64()?).ok()?;
      let height = u32::try_from(f.get("height")?.as_u64()?).ok()?;
      Some((width, height, name, static_name))
    })
    .collect();
  files.sort_by_key(|(w, ..)| *w);
  let (width, height, ..) = files.first()?.clone();
  // The set entry's flag 1 (or the emote's own 256) marks a zero-width emote.
  let flags = |v: &Value| v.get("flags").and_then(Value::as_u64).unwrap_or(0);
  Some(Emote {
    name,
    id: data
      .get("id")
      .or_else(|| entry.get("id"))
      .and_then(Value::as_str)
      .unwrap_or_default()
      .to_owned(),
    width,
    height,
    animated: data
      .get("animated")
      .and_then(Value::as_bool)
      .unwrap_or(false),
    zero_width: flags(entry) & 1 != 0 || flags(data) & 256 != 0,
    base,
    files: files.into_iter().map(|(w, _, n, s)| (w, n, s)).collect(),
  })
}

/// A Twitch user's 7TV response: their set's emotes. A user without 7TV answers 404
/// (`Client::seventv_channel` gives no emotes then).
pub fn parse_user(text: &str) -> Result<Vec<Emote>> {
  let value: Value = serde_json::from_str(text).map_err(|e| Error::Parse {
    endpoint: "7tv",
    detail: e.to_string(),
  })?;
  Ok(value.get("emote_set").map(parse_set).unwrap_or_default())
}

/// The global set's response.
pub fn parse_global(text: &str) -> Result<Vec<Emote>> {
  let value: Value = serde_json::from_str(text).map_err(|e| Error::Parse {
    endpoint: "7tv",
    detail: e.to_string(),
  })?;
  Ok(parse_set(&value))
}

/// The 7TV emotes of the channel whose Twitch user id is `user_id`; empty when it has no 7TV account.
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
      endpoint: "7tv",
      code: None,
    }),
  }
}

/// 7TV's global emotes.
pub(crate) async fn global(http: &reqwest::Client) -> Result<Vec<Emote>> {
  let response = http.get(format!("{API}/emote-sets/global")).send().await?;
  match response.status().as_u16() {
    200 => parse_global(&response.text().await?),
    status => Err(Error::Status {
      status,
      endpoint: "7tv",
      code: None,
    }),
  }
}
