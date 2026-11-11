//! The chat's last messages from before joining (ADR 0036): IRC sends none, so they come from
//! robotty's recent-messages service (`recent-messages.robotty.de`, anonymous, by channel
//! login), which keeps each channel's last lines as Twitch sent them (tagged `historical=1`).
//! They are parsed as the live lines are, and only what a chat shows is kept: messages, notices
//! and the moderation that removes some of them.

use serde_json::Value;

use super::irc::{self, Event};
use crate::error::{Error, Result};

const API: &str = "https://recent-messages.robotty.de/api/v2/recent-messages";
/// Lines asked for at most (the service keeps 800).
const LIMIT: u32 = 100;

/// The last lines of `login`'s chat, oldest first.
pub(crate) async fn recent(http: &reqwest::Client, login: &str) -> Result<Vec<Event>> {
  let login = login.to_ascii_lowercase();
  let response = http
    .get(format!("{API}/{login}?limit={LIMIT}"))
    .send()
    .await?;
  match response.status().as_u16() {
    200 => {
      let body: Value = response.json().await?;
      Ok(parse_recent(&body))
    }
    status => Err(Error::Status {
      status,
      endpoint: "recent-messages",
      code: None,
    }),
  }
}

/// The events of a recent-messages answer (`{"messages": [raw IRC lines], "error": …}`), oldest
/// first: messages, notices and their removals (a line that is none of these is skipped).
pub fn parse_recent(body: &Value) -> Vec<Event> {
  body["messages"]
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .filter_map(irc::parse)
    .filter(|event| {
      matches!(
        event,
        Event::Message(_)
          | Event::Notice(_)
          | Event::Delete(_)
          | Event::ClearUser(_)
          | Event::Clear
      )
    })
    .collect()
}
