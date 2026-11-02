//! Twitch's watch-time report (spade), as the web player sends it: one `minute-watched` event a
//! minute while a live plays. Twitch credits channel points and the bonus chest from it. Played
//! through Peeby without it, a live earned nothing in 20 minutes (2026-11-02, ADR 0012).
//!
//! The endpoint is `spade_url` in twitch.tv's settings script, linked from its page
//! (Twitch-Channel-Points-Miner reads it the same way). The event carries the viewer's user id:
//! never log it.

use serde_json::json;

/// What a `minute-watched` event says.
#[derive(Clone, PartialEq, Eq)]
pub struct MinuteWatched {
  /// The channel's user id.
  pub channel_id: String,
  /// The channel's login.
  pub channel: String,
  /// The live's id (`stream.id`).
  pub broadcast_id: String,
  /// The viewer's user id.
  pub user_id: String,
}

impl std::fmt::Debug for MinuteWatched {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("MinuteWatched")
      .field("channel", &self.channel)
      .finish_non_exhaustive()
  }
}

/// The settings script's URL in twitch.tv's page.
pub fn settings_url(page: &str) -> Option<&str> {
  [
    "https://assets.twitch.tv/config/settings.",
    "https://static.twitchcdn.net/config/settings.",
  ]
  .iter()
  .find_map(|prefix| {
    let start = page.find(prefix)?;
    let rest = &page[start..];
    let end = rest.find(".js")? + ".js".len();
    let url = &rest[..end];
    url
      .bytes()
      .all(|b| b.is_ascii_alphanumeric() || b"/:.-_".contains(&b))
      .then_some(url)
  })
}

/// `spade_url` in the settings script.
pub fn spade_url(settings: &str) -> Option<String> {
  let start = settings.find("\"spade_url\":\"")? + "\"spade_url\":\"".len();
  let rest = &settings[start..];
  let url = rest[..rest.find('"')?].replace("\\/", "/");
  url.starts_with("https://").then_some(url)
}

/// The form body of `event`: `data=` and the base64 of the JSON list of events.
pub fn body(event: &MinuteWatched) -> String {
  let events = json!([{
    "event": "minute-watched",
    "properties": {
      "channel_id": event.channel_id,
      "broadcast_id": event.broadcast_id,
      // Without `location` (and `player`), spade still answers 204 but no points are credited
      // (twitch-miner-go, PR #42).
      "location": "channel",
      "player": "site",
      "user_id": event.user_id,
      "live": true,
      "channel": event.channel,
    },
  }]);
  let encoded = base64(events.to_string().as_bytes());
  // Form encoding: base64's `+`, `/` and `=` are escaped.
  let mut out = String::from("data=");
  for c in encoded.chars() {
    match c {
      '+' => out.push_str("%2B"),
      '/' => out.push_str("%2F"),
      '=' => out.push_str("%3D"),
      c => out.push(c),
    }
  }
  out
}

/// Standard base64, padded.
fn base64(bytes: &[u8]) -> String {
  const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
  for chunk in bytes.chunks(3) {
    let n = chunk
      .iter()
      .enumerate()
      .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
    for i in 0..4 {
      if i <= chunk.len() {
        out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
      } else {
        out.push('=');
      }
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn base64_matches_the_standard() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
  }
}
