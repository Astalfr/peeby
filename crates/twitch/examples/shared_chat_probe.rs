//! Finds lives in a Shared Chat session (several channels' chats merged, Twitch's): joins the
//! chats of the top lives anonymously (`justinfan`, read-only) and prints each line whose
//! `source-room-id` is another room's, which only a shared session sends. `--save` writes those
//! lines raw, one a line (public chat lines: rename the chatters before a fixture).
//!
//! `cargo run -p twitch --example shared_chat_probe -- <minutes> (<login>… | top) [--save <file>]`

// An example prints on purpose.
#![allow(clippy::print_stdout)]

use std::io::Write;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use twitch::catalog;

const IRC: &str = "wss://irc-ws.chat.twitch.tv:443";
/// Channels joined from the top lives.
const TOP: usize = 300;
/// Joins sent at a time, then a pause (Twitch's join limit is 20 per 10 s).
const JOIN_BATCH: usize = 18;

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut args: Vec<String> = std::env::args().skip(1).collect();
  let save = args.iter().position(|a| a == "--save").map(|i| {
    let path = args.get(i + 1).cloned().unwrap_or_default();
    args.drain(i..(i + 2).min(args.len()));
    path
  });
  let minutes: u64 = args.first().and_then(|m| m.parse().ok()).unwrap_or(3);
  let logins: Vec<String> = args.into_iter().skip(1).collect();
  twitch::block_on(probe(minutes, logins, save))?
}

async fn probe(
  minutes: u64,
  logins: Vec<String>,
  save: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut channels = Vec::new();
  if logins.first().map(String::as_str) == Some("top") {
    let client = twitch::Client::new(None, None)?;
    let categories =
      catalog::parse_top_categories(&client.gql(&catalog::top_categories_request()).await?)?;
    'categories: for category in categories.iter().take(40) {
      let answer = client
        .gql(&catalog::category_streams_request(&category.id))
        .await?;
      for stream in catalog::parse_category_streams(&answer).unwrap_or_default() {
        let login = stream.user_login.to_ascii_lowercase();
        if !channels.contains(&login) {
          channels.push(login);
        }
        if channels.len() >= TOP {
          break 'categories;
        }
      }
    }
  } else {
    channels = logins.iter().map(|l| l.to_ascii_lowercase()).collect();
  }
  println!("{} channels", channels.len());
  let mut file = match save {
    Some(path) => Some(std::fs::File::create(path)?),
    None => None,
  };
  let (mut socket, _) = tokio_tungstenite::connect_async(IRC).await?;
  socket
    .send(Message::text(
      "CAP REQ :twitch.tv/tags twitch.tv/commands".to_owned(),
    ))
    .await?;
  socket
    .send(Message::text("NICK justinfan41592".to_owned()))
    .await?;
  let until = Instant::now() + Duration::from_secs(minutes * 60);
  let mut pending = channels.into_iter();
  let mut next_join = Instant::now();
  let mut found = std::collections::HashSet::new();
  while Instant::now() < until {
    if Instant::now() >= next_join {
      let batch: Vec<String> = pending.by_ref().take(JOIN_BATCH).collect();
      if !batch.is_empty() {
        let join = batch
          .iter()
          .map(|c| format!("#{c}"))
          .collect::<Vec<_>>()
          .join(",");
        socket.send(Message::text(format!("JOIN {join}"))).await?;
      }
      next_join = Instant::now() + Duration::from_secs(11);
    }
    let next = tokio::time::timeout(Duration::from_secs(1), socket.next()).await;
    let Ok(Some(message)) = next else { continue };
    let Message::Text(text) = message? else {
      continue;
    };
    for line in text.lines() {
      if line.starts_with("PING") {
        socket
          .send(Message::text("PONG :tmi.twitch.tv".to_owned()))
          .await?;
        continue;
      }
      let tag = |name: &str| {
        line
          .split([';', ' '])
          .find_map(|part| part.strip_prefix(&format!("{name}=")))
          .map(str::to_owned)
      };
      let (Some(source), Some(room)) = (tag("source-room-id"), tag("room-id")) else {
        continue;
      };
      if source == room {
        continue;
      }
      let channel = line
        .split(" PRIVMSG #")
        .nth(1)
        .or_else(|| line.split(" USERNOTICE #").nth(1))
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("?")
        .to_owned();
      if found.insert(channel.clone()) {
        println!("shared chat: #{channel} (lines from room {source})");
      }
      if let Some(file) = &mut file {
        writeln!(file, "{line}")?;
      }
    }
  }
  println!("done: {} channels in a shared chat", found.len());
  Ok(())
}
