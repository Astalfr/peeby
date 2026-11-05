//! Checks whether Twitch's PubSub still takes an anonymous listener for channels' raids
//! (`raid.<channel id>`), as its web player used: connects, asks for the topics of the given
//! channels (or of 500 lives of the top categories with `top`), and prints every message for a
//! while; raid messages are also written, one per line, to the file given with `--save` (they
//! carry public channel data only). Anonymous: no token is read or sent.
//!
//! `cargo run -p twitch --example raid_probe -- <minutes> (<login>… | top) [--save <file>]`

// A command-line probe reports on the console.
#![allow(clippy::print_stdout)]

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use twitch::catalog;

const PUBSUB: &str = "wss://pubsub-edge.twitch.tv/v1";
/// PubSub's limits: 50 topics a connection, 10 connections an address.
const TOPICS_PER_CONNECTION: usize = 50;
const CONNECTIONS: usize = 10;

type Saved = Arc<Mutex<Option<std::fs::File>>>;

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
    'categories: for category in categories.iter().take(30) {
      let answer = client
        .gql(&catalog::category_streams_request(&category.id))
        .await?;
      for stream in catalog::parse_category_streams(&answer).unwrap_or_default() {
        if !ids.contains(&stream.user_id) {
          ids.push(stream.user_id);
        }
        if ids.len() >= TOPICS_PER_CONNECTION * CONNECTIONS {
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
  println!("{} channels", ids.len());
  let saved: Saved = Arc::new(Mutex::new(match save {
    Some(path) => Some(std::fs::File::create(path)?),
    None => None,
  }));
  let until = Instant::now() + Duration::from_secs(minutes * 60);
  let listeners = ids
    .chunks(TOPICS_PER_CONNECTION)
    .take(CONNECTIONS)
    .enumerate()
    .map(|(i, chunk)| {
      let topics: Vec<String> = chunk.iter().map(|id| format!("raid.{id}")).collect();
      listen(i, topics, until, saved.clone())
    });
  for result in futures_util::future::join_all(listeners).await {
    if let Err(e) = result {
      println!("listener: {e}");
    }
  }
  println!("done");
  Ok(())
}

async fn listen(
  n: usize,
  topics: Vec<String>,
  until: Instant,
  saved: Saved,
) -> Result<(), Box<dyn std::error::Error>> {
  let (mut socket, _) = tokio_tungstenite::connect_async(PUBSUB).await?;
  let listen = serde_json::json!({
    "type": "LISTEN",
    "nonce": format!("peeby-{n}"),
    "data": { "topics": topics },
  });
  socket.send(Message::text(listen.to_string())).await?;
  let mut ping = Instant::now();
  while Instant::now() < until {
    let next = tokio::time::timeout(Duration::from_secs(30), socket.next()).await;
    match next {
      Ok(Some(Ok(Message::Text(text)))) => {
        let text = text.to_string();
        if text.contains("\"MESSAGE\"") {
          println!("[{n}] {text}");
          if let Ok(mut file) = saved.lock()
            && let Some(file) = file.as_mut()
          {
            writeln!(file, "{text}")?;
          }
        } else if !text.contains("PONG") {
          println!("[{n}] {text}");
        }
      }
      Ok(Some(Ok(Message::Close(frame)))) => {
        println!("[{n}] closed: {frame:?}");
        return Ok(());
      }
      Ok(Some(Ok(_))) => {}
      Ok(Some(Err(e))) => {
        println!("[{n}] error: {e}");
        return Ok(());
      }
      Ok(None) => {
        println!("[{n}] ended");
        return Ok(());
      }
      Err(_) => {}
    }
    // PubSub drops a client without a PING every 5 minutes.
    if ping.elapsed() > Duration::from_secs(240) {
      ping = Instant::now();
      socket
        .send(Message::text(r#"{"type":"PING"}"#.to_owned()))
        .await?;
    }
  }
  Ok(())
}
