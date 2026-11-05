//! A live's past through its archive VOD (ADR 0026), anonymous: for each live, the recording
//! that `Channel` names (`stream.archiveVideo`), then that VOD's media playlist read twice, 20 s
//! apart (type, end tag, length against the live's age). Prints no token or URL.
//!
//! `cargo run -p twitch --example archive_probe -- (<login>… | top) [--save <dir>]`
//!
//! `top`: the first lives of the top categories. `--save` writes the `Channel` answers of the
//! first live with an archive and the first without one, as fixtures.

// An example reports on the console.
#![allow(clippy::print_stdout)]

use std::path::PathBuf;
use std::time::Duration;

use twitch::usher::Codec;
use twitch::{Client, Target, catalog, gql};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut args: Vec<String> = std::env::args().skip(1).collect();
  let save = match args.iter().position(|a| a == "--save") {
    Some(i) => {
      let dir = args.get(i + 1).map(PathBuf::from);
      args.drain(i..(i + 2).min(args.len()));
      dir
    }
    None => None,
  };
  let client = Client::new(None, None)?;
  client.refresh_client_id().await.ok();
  let mut logins = args;
  if logins.first().map(String::as_str) == Some("top") {
    logins.clear();
    let categories =
      catalog::parse_top_categories(&client.gql(&catalog::top_categories_request()).await?)?;
    for category in categories.iter().take(4) {
      let answer = client
        .gql(&catalog::category_streams_request(&category.id))
        .await?;
      let streams = catalog::parse_category_streams(&answer).unwrap_or_default();
      logins.extend(streams.into_iter().take(2).map(|s| s.user_login));
    }
  }
  let (mut saved_with, mut saved_without) = (false, false);
  let http = reqwest::Client::new();
  for login in logins {
    let answer = client.gql(&gql::channel_request(&login)).await?;
    let channel = match gql::parse_channel(&answer) {
      Ok(channel) => channel,
      Err(e) => {
        println!("{login}: {e}");
        continue;
      }
    };
    let Some(live) = channel.live else {
      println!("{login}: offline");
      continue;
    };
    println!(
      "{login}: live since {:?} · archive {:?}",
      live.started_at, live.archive
    );
    if let Some(dir) = &save {
      let slot = if live.archive.is_some() {
        &mut saved_with
      } else {
        &mut saved_without
      };
      if !*slot {
        *slot = true;
        let name = if live.archive.is_some() {
          "gql_channel_live_archive.json"
        } else {
          "gql_channel_live_no_archive.json"
        };
        std::fs::write(dir.join(name), serde_json::to_string_pretty(&answer)?)?;
        println!("  saved {name}");
      }
    }
    let Some(archive) = live.archive else {
      continue;
    };
    let target = Target::Vod(archive.id.clone());
    let token = match client.playback_token(&target, gql::PLAYER_TYPE).await {
      Ok(token) => token,
      Err(e) => {
        println!("  token: {e}");
        continue;
      }
    };
    let master = match client.master(&target, &token, &[Codec::H264]).await {
      Ok(master) => master,
      Err(e) => {
        println!("  master: {e}");
        continue;
      }
    };
    let Some(variant) = master.variants.iter().find(|v| !v.audio_only()) else {
      continue;
    };
    for round in 0..2 {
      let text = http.get(&variant.url).send().await?.text().await?;
      let length: f64 = text
        .lines()
        .filter_map(|l| l.strip_prefix("#EXTINF:"))
        .filter_map(|l| l.split(',').next()?.parse::<f64>().ok())
        .sum();
      let age = archive.created_at.map(|at| {
        let now = std::time::SystemTime::now()
          .duration_since(std::time::UNIX_EPOCH)
          .unwrap_or_default();
        now.as_millis() as f64 / 1000.0 - at as f64 / 1000.0
      });
      let tags: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("#EXT-X-PLAYLIST-TYPE") || l.starts_with("#EXT-X-ENDLIST"))
        .collect();
      println!(
        "  playlist #{round}: {length:.0} s, the recording's age {:?} s (behind by {:?} s), {tags:?}",
        age.map(f64::round),
        age.map(|a| (a - length).round())
      );
      if round == 0 {
        tokio::time::sleep(Duration::from_secs(20)).await;
      }
    }
  }
  Ok(())
}
