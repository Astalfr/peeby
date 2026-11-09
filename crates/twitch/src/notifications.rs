//! The account's notification center (twitch.tv's bell), through raw GQL (no persisted hash),
//! checked on 2026-11-09 against GQL's validation, anonymous (`twitch_capture -- gql-probe`;
//! introspection is disabled), and read signed in (`twitch_capture -- notifications`):
//! - `currentUser { notifications(first, after, displayType, language) }` (`displayType`:
//!   `OnsiteNotificationDisplayType`, `VIEWER` is the viewer's list): `summary { unseenCount
//!   lastSeenAt }`, `pageInfo { hasNextPage }`, `edges { cursor node }`. `currentUser` is null
//!   without an auth-token.
//! - A node (`OnsiteNotification`): `id`, `body` (plain text, `**bold**` seen around a drop's
//!   reward), `type` (a campaign's name, e.g. `drops_lifecycle_062625_p1`), `category`
//!   (`watch_streaks`, `recommended_content`, `promotions`, `drops_and_quests`, `transactional`
//!   seen), `isRead`, `createdAt`, `updatedAt`, `thumbnailURL`, `renderStyle`,
//!   `destinationType`, `actions { id type label url body modalID }` (the first one with a `url`
//!   is the click), `extra` (a union, not read).
//! - Reading the list writes nothing: no mutation marking it read or seen was found among the
//!   guessed names, so Peeby keeps its own "seen" and twitch.tv's count stays as it is.

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::gql::gql_error;

/// Notifications read at once (the panel shows them all).
pub const PAGE: usize = 20;

const QUERY: &str = "query Notifications($first: Int!) { currentUser { id notifications(first: \
  $first, displayType: VIEWER) { summary { unseenCount lastSeenAt } edges { node { id body type \
  category isRead createdAt updatedAt thumbnailURL renderStyle destinationType actions { id type \
  label url } } } } } }";

/// One notification of the account's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
  /// Id, stable across readings.
  pub id: String,
  /// The text, in runs bold or not, without its markup.
  pub spans: Vec<Span>,
  /// Twitch's kind (`type`), e.g. `streamup`; empty when missing.
  pub kind: String,
  /// Twitch's group (`category`), e.g. `watch_streaks`; empty when missing.
  pub category: String,
  /// Read on twitch.tv (or elsewhere).
  pub read: bool,
  /// When it was made, milliseconds since the epoch.
  pub created_at: Option<i64>,
  /// Its picture (a channel's avatar, a clip's or a video's preview).
  pub thumbnail: Option<String>,
  /// Where a click goes: the first action with a URL.
  pub url: Option<String>,
}

/// A run of a notification's text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
  /// The text.
  pub text: String,
  /// Twitch shows it in bold (a name, a reward).
  pub bold: bool,
}

impl Notification {
  /// The whole text.
  pub fn text(&self) -> String {
    self.spans.iter().map(|s| s.text.as_str()).collect()
  }
}

/// A reading of the list, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
  /// The notifications, newest first.
  pub notifications: Vec<Notification>,
  /// twitch.tv's unseen count (its bell's badge).
  pub unseen: usize,
  /// When twitch.tv's bell was last opened, milliseconds since the epoch.
  pub last_seen_at: Option<i64>,
}

/// The request for the newest `first` notifications.
pub fn request(first: usize) -> Value {
  json!({
    "operationName": "Notifications",
    "query": QUERY,
    "variables": { "first": first },
  })
}

/// Reads the list; `currentUser` null (no or a dead auth-token) is [`Error::NotLoggedIn`]. A
/// node without an id or a text is skipped.
pub fn parse(answer: &Value) -> Result<Page> {
  if let Some(error) = gql_error(answer) {
    return Err(error);
  }
  let user = &answer["data"]["currentUser"];
  if user.is_null() {
    return Err(Error::NotLoggedIn);
  }
  let list = &user["notifications"];
  let mut notifications: Vec<Notification> = list["edges"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|edge| notification(&edge["node"]))
    .collect();
  // Newest first, whatever order they came in.
  notifications.sort_by_key(|n| std::cmp::Reverse(n.created_at));
  let summary = &list["summary"];
  Ok(Page {
    notifications,
    unseen: summary["unseenCount"]
      .as_u64()
      .and_then(|n| usize::try_from(n).ok())
      .unwrap_or(0),
    last_seen_at: summary["lastSeenAt"]
      .as_str()
      .and_then(crate::time::epoch_millis),
  })
}

fn notification(node: &Value) -> Option<Notification> {
  let id = node["id"].as_str().filter(|s| !s.is_empty())?.to_owned();
  let spans = spans(node["body"].as_str()?);
  if spans.is_empty() {
    return None;
  }
  let url = node["actions"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|a| a["url"].as_str())
    .find(|u| u.starts_with("https://"))
    .map(str::to_owned);
  Some(Notification {
    id,
    spans,
    kind: node["type"].as_str().unwrap_or_default().to_owned(),
    category: node["category"].as_str().unwrap_or_default().to_owned(),
    read: node["isRead"].as_bool().unwrap_or(false),
    created_at: node["createdAt"]
      .as_str()
      .and_then(crate::time::epoch_millis),
    thumbnail: node["thumbnailURL"]
      .as_str()
      .filter(|u| u.starts_with("https://"))
      .map(str::to_owned),
    url,
  })
}

/// A body's text in runs, bold or not: Twitch writes bold as `**Cyber Glasses**` (seen) or
/// `<b>`/`<strong>` (not seen yet); other tags are dropped, entities decoded, white space folded.
fn spans(body: &str) -> Vec<Span> {
  let mut spans: Vec<Span> = Vec::new();
  let mut bold = false;
  let mut current = String::new();
  let flush = |current: &mut String, bold: bool, spans: &mut Vec<Span>| {
    if !current.is_empty() {
      spans.push(Span {
        text: decode(current),
        bold,
      });
      current.clear();
    }
  };
  let mut rest = body;
  while !rest.is_empty() {
    if let Some(after) = rest.strip_prefix("**") {
      flush(&mut current, bold, &mut spans);
      bold = !bold;
      rest = after;
      continue;
    }
    if rest.starts_with('<')
      && let Some(end) = rest.find('>')
    {
      let tag = rest[1..end].trim().to_ascii_lowercase();
      match tag.as_str() {
        "b" | "strong" | "/b" | "/strong" => {
          flush(&mut current, bold, &mut spans);
          bold = !tag.starts_with('/');
        }
        "br" | "br/" | "br /" => current.push(' '),
        _ => {}
      }
      rest = &rest[end + 1..];
      continue;
    }
    let mut chars = rest.chars();
    if let Some(c) = chars.next() {
      current.push(c);
    }
    rest = chars.as_str();
  }
  flush(&mut current, bold, &mut spans);
  // White space folded across the runs, none at either end.
  let mut folded: Vec<Span> = Vec::new();
  let mut space = false;
  for span in spans {
    let mut text = String::new();
    for c in span.text.chars() {
      if c.is_whitespace() {
        space = true;
      } else {
        let starts = folded.is_empty() && text.is_empty();
        if space && !starts {
          text.push(' ');
        }
        space = false;
        text.push(c);
      }
    }
    if !text.is_empty() {
      folded.push(Span {
        text,
        bold: span.bold,
      });
    }
  }
  folded
}

/// The few HTML entities a body uses.
fn decode(text: &str) -> String {
  text
    .replace("&lt;", "<")
    .replace("&gt;", ">")
    .replace("&quot;", "\"")
    .replace("&#39;", "'")
    .replace("&#x27;", "'")
    .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
  use super::{Span, spans};

  fn span(text: &str, bold: bool) -> Span {
    Span {
      text: text.to_owned(),
      bold,
    }
  }

  #[test]
  fn bold_runs_and_markup() {
    assert_eq!(
      spans("You can get **Cyber Glasses**  for AION 2."),
      vec![
        span("You can get", false),
        span(" Cyber Glasses", true),
        span(" for AION 2.", false)
      ]
    );
    assert_eq!(
      spans("<b>Streamer</b> is live: Tom &amp; Jerry <br/>again"),
      vec![
        span("Streamer", true),
        span(" is live: Tom & Jerry again", false)
      ]
    );
  }
}
