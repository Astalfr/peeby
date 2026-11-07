//! Checks 7TV's EventAPI (`wss://events.7tv.io/v3`), anonymous: resolves the given channels' 7TV
//! sets over REST (or those of the lives of the top categories with `top`), subscribes to their
//! `emote_set.update` (and their 7TV user's `user.update`) up to the Hello's subscription limit,
//! and prints every message for a while. Dispatches are also written, one per line, to the file
//! given with `--save` (7TV's public data: emote sets and the editor who changed them; trim the
//! `actor` before committing one as a fixture). No token is read or sent.
//!
//! `cargo run -p twitch --example seventv_events -- <minutes> (<login>… | top) [--save <file>]`

// A command-line probe reports on the console.
#![allow(clippy::print_stdout)]

use std::io::Write;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use twitch::catalog;

const EVENTS: &str = "wss://events.7tv.io/v3";

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut args: Vec<String> = std::env::args().skip(1).collect();
  let save = args.iter().position(|a| a == "--save").map(|i| {
    let path = args.get(i + 1).cloned().unwrap_or_default();
    args.drain(i..(i + 2).min(args.len()));
    path
  });
  let minutes: u64 = args.first().and_then(|m| m.parse().ok()).unwrap_or(5);
  let logins: Vec<String> = args.into_iter().skip(1).collect();
  twitch::block_on(probe(minutes, logins, save))?
}

async fn probe(
  minutes: u64,
  logins: Vec<String>,
  save: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
  let client = twitch::Client::new(None, None)?;
  let mut ids = Vec::new();
  if logins.first().map(String::as_str) == Some("top") {
    let categories =
      catalog::parse_top_categories(&client.gql(&catalog::top_categories_request()).await?)?;
    'categories: for category in categories.iter().take(10) {
      let answer = client
        .gql(&catalog::category_streams_request(&category.id))
        .await?;
      for stream in catalog::parse_category_streams(&answer).unwrap_or_default() {
        if !ids.contains(&stream.user_id) {
          ids.push(stream.user_id);
        }
        if ids.len() >= 150 {
          break 'categories;
        }
      }
    }
  } else {
    for login in &logins {
      match client.channel(login).await {
        Ok(channel) => ids.push(channel.id),
        Err(e) => println!("{login}: {e}"),
      }
    }
  }
  let http = reqwest::Client::new();
  // (Twitch id, 7TV set id, 7TV user id)
  let mut sets = Vec::new();
  for id in &ids {
    let response = http
      .get(format!("https://7tv.io/v3/users/twitch/{id}"))
      .send()
      .await?;
    if !response.status().is_success() {
      continue;
    }
    let user: Value = serde_json::from_str(&response.text().await?)?;
    let set = user.pointer("/emote_set/id").and_then(Value::as_str);
    let seventv_user = user.pointer("/user/id").and_then(Value::as_str);
    if sets.is_empty() {
      let keys = |v: Option<&Value>| {
        v.and_then(Value::as_object)
          .map(|o| o.keys().cloned().collect::<Vec<_>>())
      };
      println!(
        "first user answer: top keys {:?}, user keys {:?}, emote_set_id {:?}",
        keys(Some(&user)),
        keys(user.get("user")),
        user.get("emote_set_id")
      );
    }
    if let Some(set) = set {
      sets.push((id.clone(), set.to_owned(), seventv_user.map(str::to_owned)));
    }
  }
  println!("{} channels, {} with a 7TV set", ids.len(), sets.len());
  let mut saved = match save {
    Some(path) => Some(std::fs::File::create(path)?),
    None => None,
  };

  let (mut socket, _) = tokio_tungstenite::connect_async(EVENTS).await?;
  let until = Instant::now() + Duration::from_secs(minutes * 60);
  let mut subscribed = false;
  loop {
    let left = until.saturating_duration_since(Instant::now());
    if left.is_zero() {
      println!("time is up");
      return Ok(());
    }
    let frame = match tokio::time::timeout(left, socket.next()).await {
      Err(_) => continue,
      Ok(None) => {
        println!("closed (no frame)");
        return Ok(());
      }
      Ok(Some(frame)) => frame?,
    };
    let text = match frame {
      Message::Text(text) => text.to_string(),
      Message::Close(close) => {
        println!("close frame: {close:?}");
        return Ok(());
      }
      other => {
        println!("other frame: {other:?}");
        continue;
      }
    };
    let message: Value = serde_json::from_str(&text)?;
    let op = message.get("op").and_then(Value::as_u64);
    match op {
      Some(0) => {
        println!("DISPATCH {text}");
        if let Some(file) = saved.as_mut() {
          writeln!(file, "{text}")?;
        }
      }
      Some(2) => println!("heartbeat {}", message["d"]),
      _ => println!("op {op:?}: {text}"),
    }
    if op == Some(1) && !subscribed {
      subscribed = true;
      let limit = message
        .pointer("/d/subscription_limit")
        .and_then(Value::as_i64)
        .unwrap_or(-1);
      let limit = usize::try_from(limit).unwrap_or(usize::MAX);
      let mut count = 0;
      for (_, set, user) in &sets {
        let mut subs = vec![("emote_set.update", set.clone())];
        if let Some(user) = user {
          subs.push(("user.update", user.clone()));
        }
        for (kind, object) in subs {
          if count >= limit {
            break;
          }
          let line = json!({"op": 35, "d": {"type": kind, "condition": {"object_id": object}}});
          socket.send(Message::text(line.to_string())).await?;
          count += 1;
        }
      }
      println!("{count} subscriptions sent (limit {limit})");
    }
  }
}
