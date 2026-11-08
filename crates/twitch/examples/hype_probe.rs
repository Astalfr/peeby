//! Looks for Hype Trains on the lives of the top categories (about 1,000), anonymous: prints each
//! channel with a train approaching or under way, its level and progress. With `--save <file>`,
//! saves the app's answer (`twitch::hype_train::hype_train_request`) for the first train under
//! way, its conductors renamed `viewerN` (ids and avatars replaced), for a fixture; with
//! `--approaching`, for the first approaching one instead. Read only: no token is read or sent.
//!
//! `cargo run -p twitch --example hype_probe -- [--save <file>] [--approaching] [login…]`

// A command-line probe reports on the console.
#![allow(clippy::print_stdout)]

use serde_json::{Value, json};
use twitch::{catalog, hype_train};

/// `users(logins:)` takes up to 100 logins.
const BATCH: usize = 100;
const SCAN: &str = "query Scan($logins: [String!]) { users(logins: $logins) { login channel { \
  hypeTrain { approaching { id goal participants } execution { id isActive progress { goal \
  progression level { value } } } } } } }";

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let mut args: Vec<String> = std::env::args().skip(1).collect();
  let save = args.iter().position(|a| a == "--save").map(|i| {
    let path = args.get(i + 1).cloned().unwrap_or_default();
    args.drain(i..(i + 2).min(args.len()));
    path
  });
  let approaching = args.iter().any(|a| a == "--approaching");
  args.retain(|a| a != "--approaching");
  twitch::block_on(probe(args, save, approaching))?
}

async fn probe(
  mut logins: Vec<String>,
  save: Option<String>,
  approaching: bool,
) -> Result<(), Box<dyn std::error::Error>> {
  let client = twitch::Client::new(None, None)?;
  if logins.is_empty() {
    let categories =
      catalog::parse_top_categories(&client.gql(&catalog::top_categories_request()).await?)?;
    for category in categories.iter().take(40) {
      let answer = client
        .gql(&catalog::category_streams_request(&category.id))
        .await?;
      for stream in catalog::parse_category_streams(&answer).unwrap_or_default() {
        if !logins.contains(&stream.user_login) {
          logins.push(stream.user_login);
        }
      }
    }
  }
  println!("{} channels", logins.len());
  let mut found: Option<String> = None;
  for batch in logins.chunks(BATCH) {
    let answer = client
      .gql(&json!({ "query": SCAN, "variables": { "logins": batch } }))
      .await?;
    for user in answer["data"]["users"].as_array().into_iter().flatten() {
      let login = user["login"].as_str().unwrap_or_default();
      let hype = &user["channel"]["hypeTrain"];
      let a = &hype["approaching"];
      if !a.is_null() {
        println!(
          "{login}: approaching, {} of {}",
          a["participants"].as_array().map_or(0, Vec::len),
          a["goal"]
        );
        if approaching {
          found.get_or_insert_with(|| login.to_owned());
        }
      }
      let e = &hype["execution"];
      if e["isActive"].as_bool() == Some(true) {
        let p = &e["progress"];
        println!(
          "{login}: level {}, {} / {}",
          p["level"]["value"], p["progression"], p["goal"]
        );
        if !approaching {
          found.get_or_insert_with(|| login.to_owned());
        }
      }
    }
  }
  if let (Some(path), Some(login)) = (save, found) {
    let mut answer = client.gql(&hype_train::hype_train_request(&login)).await?;
    anonymize(&mut answer);
    std::fs::write(&path, serde_json::to_string_pretty(&answer)? + "\n")?;
    let state = hype_train::parse_hype_train(&answer)?;
    println!("saved {login}'s to {path}: {state:#?}");
  }
  Ok(())
}

/// Renames the conductors `viewerN` (their ids and avatars replaced), replaces the approaching
/// train's participants' ids, and drops request ids.
fn anonymize(answer: &mut Value) {
  if let Some(extensions) = answer.as_object_mut() {
    extensions.remove("extensions");
  }
  let hype = &mut answer["data"]["user"]["channel"]["hypeTrain"];
  if let Some(participants) = hype["approaching"]["participants"].as_array_mut() {
    for (i, id) in participants.iter_mut().enumerate() {
      *id = json!(format!("{}", 100_000_001 + i));
    }
  }
  if hype["execution"].is_null() {
    return;
  }
  for (i, conductor) in hype["execution"]["conductors"]
    .as_array_mut()
    .into_iter()
    .flatten()
    .enumerate()
  {
    let name = format!("viewer{}", i + 1);
    conductor["user"] = json!({
      "id": format!("{}", 100_000_001 + i),
      "login": name,
      "displayName": name.to_uppercase(),
      "profileImageURL": "https://static-cdn.jtvnw.net/user-default-pictures-uv/de130ab0-def7-11e9-b668-784f43822e80-profile_image-50x50.png",
    });
  }
}
