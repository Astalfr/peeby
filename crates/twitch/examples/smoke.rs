//! Live check of every endpoint, counts only (no token, URL or id printed).
//!
//! `cargo run -p twitch --example smoke -- [login]` (default `squeezie`). Also checks an offline
//! channel (`twitch`) and an unknown one.

// An example reports on the console.
#![allow(clippy::print_stdout)]

use twitch::usher::Codec;
use twitch::{Client, Error, Target, gql};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  let login = std::env::args().nth(1).unwrap_or_else(|| "squeezie".into());
  let client = Client::new(None, None)?;

  match client.refresh_client_id().await {
    Ok(()) => println!("client id: refreshed"),
    Err(e) => println!("client id: {e}"),
  }

  let target = Target::Live(login.clone());
  match client.playback_token(&target, gql::PLAYER_TYPE).await {
    Ok(token) => {
      println!("live token: ok, {} restricted", token.restricted.len());
      match client
        .master(&target, &token, &[Codec::Av1, Codec::H265, Codec::H264])
        .await
      {
        Ok(master) => println!(
          "live master ({:?}): {} variants, {} landscape with video",
          master.version,
          master.variants.len(),
          master
            .variants
            .iter()
            .filter(|v| !v.portrait && !v.audio_only())
            .count()
        ),
        Err(Error::Offline) => println!("live master: {login} is offline"),
        Err(e) => println!("live master: {e}"),
      }
    }
    Err(e) => println!("live token: {e}"),
  }

  let offline = Target::Live("twitch".into());
  let token = client.playback_token(&offline, gql::PLAYER_TYPE).await?;
  match client.master(&offline, &token, &[Codec::H264]).await {
    Err(Error::Offline) => println!("offline channel: Offline, as expected"),
    Ok(_) => println!("offline channel: live right now"),
    Err(e) => println!("offline channel: {e}"),
  }

  // A device code is asked for and never approved: it expires by itself.
  let auth = twitch::auth::Auth::new(None)?;
  match auth.start_device().await {
    Ok(device) => {
      println!("device code: ok, expires in {} s", device.expires_in);
      match auth.poll(&device).await {
        Ok(twitch::auth::Poll::Pending) => println!("device poll: pending, as expected"),
        other => println!("device poll: {other:?}"),
      }
    }
    Err(e) => println!("device code: {e}"),
  }
  match auth.validate("0123456789abcdefghij0123456789").await {
    Err(Error::SessionExpired) => println!("validate: invalid token refused, as expected"),
    other => println!("validate: {other:?}"),
  }

  let missing = Target::Live("peeby_no_such_channel_0000".into());
  match client.playback_token(&missing, gql::PLAYER_TYPE).await {
    Err(Error::NotFound) => println!("unknown channel: NotFound, as expected"),
    other => println!("unknown channel: {:?}", other.map(|_| "a token")),
  }
  Ok(())
}
