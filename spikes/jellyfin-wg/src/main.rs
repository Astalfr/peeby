//! Spike: a Jellyfin server reached through a userspace WireGuard tunnel (the `wireguard` crate:
//! boringtun and smoltcp behind a local proxy, no driver, no system route), with timings and
//! throughput (docs/features-plan.md, P4; ADR 0037). Its first version used onetun, whose 64 KB
//! TCP buffers capped it at about 15 Mb/s.
//!
//! `cargo run -p spike-jellyfin-wg --release -- <wg.conf> [server-url] [--peeby]`
//! with `PEEBY_TEST_USER` and `PEEBY_TEST_PASSWORD` set (the account's name and password), or
//! `--peeby`: the debug profile's saved session (`%APPDATA%\peeby-dev\settings.toml`, its token in
//! the Credential Manager, its server when none is given), which signs in nothing.
//!
//! What it does: starts the tunnel, points a reqwest client at its proxy, signs in (this adds a
//! device to the account's sessions) or takes the saved session, then measures: the first answer
//! (the handshake included), the API's latency, the throughput of 64 MB of a random movie or
//! episode served as is (`/Videos/{id}/stream?static=true`: Direct Play's file), and how fast a
//! range from its middle starts (a seek). It plays nothing and reports no playback. The token, the
//! keys and the proxy's URL are never printed.

// A spike prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::time::{Duration, Instant};

/// Bytes read to measure the throughput.
const SAMPLE: u64 = 64 * 1024 * 1024;

type Error = Box<dyn std::error::Error>;

#[tokio::main]
async fn main() -> Result<(), Error> {
  tracing_subscriber::fmt()
    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
    .with_writer(std::io::stderr)
    .init();
  let args: Vec<String> = std::env::args().skip(1).collect();
  let saved = args.iter().any(|a| a == "--peeby");
  let mut positional = args.iter().filter(|a| !a.starts_with("--"));
  let Some(conf) = positional.next() else {
    return Err("usage: spike-jellyfin-wg <wg.conf> [server-url] [--peeby]".into());
  };
  let session = saved.then(peeby_session).transpose()?;
  let base = match (positional.next(), &session) {
    (Some(url), _) => url.trim_end_matches('/').to_owned(),
    (None, Some(session)) => session.server.trim_end_matches('/').to_owned(),
    (None, None) => return Err("no server URL".into()),
  };
  let config = wireguard::Config::parse(&std::fs::read_to_string(conf)?)?;
  println!("tunnel: {config:?}");
  let started = Instant::now();
  let tunnel = wireguard::Tunnel::start(config, &tokio::runtime::Handle::current())?;
  let client = reqwest::Client::builder()
    .proxy(reqwest::Proxy::all(tunnel.proxy_url())?)
    .build()?;
  let device = "MediaBrowser Client=\"Peeby spike\", Device=\"Peeby WG spike\",                 DeviceId=\"peeby-wg-spike\", Version=\"0.1\"";

  // The first answer through the tunnel: the handshake and a TCP connection.
  let info = client
    .get(format!("{base}/System/Info/Public"))
    .timeout(Duration::from_secs(20))
    .send()
    .await?;
  println!(
    "first answer: {:?} after {:.0} ms ({:?})",
    info.status(),
    started.elapsed().as_secs_f64() * 1000.,
    tunnel.status()
  );
  let info: serde_json::Value = info.json().await?;
  println!(
    "server: {} {}",
    info["ServerName"].as_str().unwrap_or("?"),
    info["Version"].as_str().unwrap_or("?")
  );

  // The API's latency, a few times.
  let mut times = Vec::new();
  for _ in 0..5 {
    let at = Instant::now();
    client
      .get(format!("{base}/System/Info/Public"))
      .send()
      .await?
      .bytes()
      .await?;
    times.push(at.elapsed().as_secs_f64() * 1000.);
  }
  println!("API round trips: {times:.0?} ms");

  // The session: Peeby's saved one, or a sign-in.
  let (authorization, user_id) = if let Some(session) = session {
    let device = std::env::var("COMPUTERNAME").unwrap_or_default();
    let device = format!(
      "MediaBrowser Client=\"Peeby\", Device=\"{device}\", DeviceId=\"{}\", Version=\"1.4.2\"",
      session.device_id
    );
    (
      format!("{device}, Token=\"{}\"", session.token),
      session.user_id,
    )
  } else {
    let user = std::env::var("PEEBY_TEST_USER")?;
    let password = std::env::var("PEEBY_TEST_PASSWORD")?;
    let auth: serde_json::Value = client
      .post(format!("{base}/Users/AuthenticateByName"))
      .header("Authorization", device)
      .json(&serde_json::json!({ "Username": user, "Pw": password }))
      .send()
      .await?
      .error_for_status()?
      .json()
      .await?;
    let token = auth["AccessToken"].as_str().ok_or("no token")?.to_owned();
    let user_id = auth["User"]["Id"].as_str().ok_or("no user id")?.to_owned();
    (format!("{device}, Token=\"{token}\""), user_id)
  };

  // A random movie or episode, with its file's size.
  let items: serde_json::Value = client
    .get(format!(
      "{base}/Users/{user_id}/Items?IncludeItemTypes=Movie,Episode&Recursive=true&Limit=20\
       &SortBy=Random&Fields=MediaSources&IsMissing=false"
    ))
    .header("Authorization", &authorization)
    .send()
    .await?
    .error_for_status()?
    .json()
    .await?;
  let item = items["Items"]
    .as_array()
    .and_then(|items| {
      items
        .iter()
        .find(|item| item["MediaSources"][0]["Size"].as_u64().unwrap_or(0) > 0)
    })
    .ok_or("no item with a file")?;
  let id = item["Id"].as_str().ok_or("no item")?;
  let size = item["MediaSources"][0]["Size"].as_u64().unwrap_or(0);
  let bitrate = item["MediaSources"][0]["Bitrate"].as_u64().unwrap_or(0);
  println!(
    "item: {} ({:.0} MB, {:.1} Mb/s)",
    item["Name"].as_str().unwrap_or("?"),
    size as f64 / 1e6,
    bitrate as f64 / 1e6
  );
  let stream = format!("{base}/Videos/{id}/stream?static=true");

  // Throughput from the start, as Direct Play reads.
  let at = Instant::now();
  let mut response = client
    .get(&stream)
    .header("Authorization", &authorization)
    .send()
    .await?
    .error_for_status()?;
  let first_byte = at.elapsed();
  let mut read = 0u64;
  while read < SAMPLE {
    let Some(chunk) = response.chunk().await? else {
      break;
    };
    read += chunk.len() as u64;
  }
  drop(response);
  let seconds = at.elapsed().as_secs_f64();
  let mbps = read as f64 * 8. / seconds / 1e6;
  println!(
    "stream: first byte after {:.0} ms, {:.1} MB in {seconds:.1} s = {mbps:.1} Mb/s ({:.1}× the \
     file's bitrate)",
    first_byte.as_secs_f64() * 1000.,
    read as f64 / 1e6,
    if bitrate > 0 {
      mbps / (bitrate as f64 / 1e6)
    } else {
      0.
    }
  );

  // A seek: a range from the middle, its first 4 MB.
  if size > 0 {
    let at = Instant::now();
    let mut response = client
      .get(&stream)
      .header("Authorization", &authorization)
      .header("Range", format!("bytes={}-", size / 2))
      .send()
      .await?;
    let status = response.status();
    let first_byte = at.elapsed();
    let mut read = 0u64;
    while read < 4 * 1024 * 1024 {
      let Some(chunk) = response.chunk().await? else {
        break;
      };
      read += chunk.len() as u64;
    }
    println!(
      "seek: {status:?}, first byte after {:.0} ms, 4 MB after {:.0} ms",
      first_byte.as_secs_f64() * 1000.,
      at.elapsed().as_secs_f64() * 1000.
    );
  }

  // What the proxy refuses: an address the tunnel does not route.
  let outside = client
    .get("https://example.com/")
    .timeout(Duration::from_secs(10))
    .send()
    .await;
  println!(
    "outside AllowedIPs: {}",
    match outside {
      Ok(response) => format!("answered {} (should not)", response.status()),
      Err(e) => format!("refused (connect error: {})", e.is_connect()),
    }
  );
  println!("tunnel at the end: {:?}", tunnel.status());
  Ok(())
}

/// The debug profile's Jellyfin session (its token never printed).
struct Saved {
  server: String,
  user_id: String,
  device_id: String,
  token: String,
}

fn peeby_session() -> Result<Saved, Error> {
  let dir = std::env::var("APPDATA")?;
  let text = std::fs::read_to_string(format!(r"{dir}\peeby-dev\settings.toml"))?;
  let settings: toml::Value = toml::from_str(&text)?;
  let jellyfin = &settings["jellyfin"];
  let field = |name: &str| {
    jellyfin
      .get(name)
      .and_then(toml::Value::as_str)
      .map(str::to_owned)
      .ok_or(format!("no jellyfin.{name} in the settings"))
  };
  let (server, user_id, device_id) = (field("server_url")?, field("user_id")?, field("device_id")?);
  let token = keyring::Entry::new("peeby-dev", &format!("{server}|{user_id}"))?.get_password()?;
  Ok(Saved {
    server,
    user_id,
    device_id,
    token,
  })
}
