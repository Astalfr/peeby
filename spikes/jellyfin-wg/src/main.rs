//! Spike: a Jellyfin server reached through a userspace WireGuard tunnel (onetun: boringtun and
//! smoltcp, no driver, no system route), with timings and throughput, to decide whether Peeby
//! can carry its Jellyfin traffic this way (docs/features-plan.md, P4).
//!
//! `cargo run -p spike-jellyfin-wg --release -- <wg.conf> <server-ip:port in the tunnel> [--https <host>] [--peeby]`
//! with `PEEBY_TEST_USER` and `PEEBY_TEST_PASSWORD` set (the account's name and password), or
//! `--peeby`: the debug profile's saved session (`%APPDATA%\peeby-dev\settings.toml`, its token in
//! the Credential Manager), which signs in nothing.
//!
//! What it does: reads the WireGuard file (`[Interface]` PrivateKey, Address; `[Peer]` PublicKey,
//! PresharedKey, Endpoint, PersistentKeepalive), starts the tunnel with a local port forwarded to
//! the server, signs in (this adds a device to the account's sessions), then measures: the
//! first answer (the handshake included), the API's latency, the throughput of 64 MB of a
//! random movie or episode served as is (`/Videos/{id}/stream?static=true`: Direct Play's file),
//! and how fast a range from its middle starts (a seek). It plays nothing and reports no
//! playback. The token and the keys are never printed.

// A spike prints on purpose.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use onetun::config::{Config, PortForwardConfig, PortProtocol, PublicKey, StaticSecret};
use onetun::events::Bus;

/// The local end of the forward.
const LOCAL: &str = "127.0.0.1:18096";
/// Bytes read to measure the throughput.
const SAMPLE: u64 = 64 * 1024 * 1024;

type Error = Box<dyn std::error::Error>;

/// onetun's logs on stderr (its handshakes, its connections).
struct Logger;

impl log::Log for Logger {
  fn enabled(&self, metadata: &log::Metadata) -> bool {
    metadata.level() <= log::Level::Debug && metadata.target().starts_with("onetun")
  }

  fn log(&self, record: &log::Record) {
    if self.enabled(record.metadata()) {
      eprintln!("[{} {}] {}", record.level(), record.target(), record.args());
    }
  }

  fn flush(&self) {}
}

#[tokio::main]
async fn main() -> Result<(), Error> {
  log::set_logger(&Logger).ok();
  log::set_max_level(if std::env::var_os("PEEBY_WG_LOG").is_some() {
    log::LevelFilter::Debug
  } else {
    log::LevelFilter::Info
  });
  let args: Vec<String> = std::env::args().skip(1).collect();
  let (Some(conf), Some(server)) = (args.first(), args.get(1)) else {
    return Err("usage: spike-jellyfin-wg <wg.conf> <server-ip:port> [--https <host>]".into());
  };
  let https = args
    .iter()
    .position(|a| a == "--https")
    .and_then(|i| args.get(i + 1))
    .cloned();
  let saved = args.iter().any(|a| a == "--peeby");
  let server: SocketAddr = server.parse()?;
  let config = config(&std::fs::read_to_string(conf)?, server)?;
  println!(
    "tunnel: endpoint {}, local {LOCAL} -> {server}",
    config.endpoint_addr
  );
  let started = Instant::now();
  onetun::start_tunnels(config, Bus::default())
    .await
    .map_err(|e| format!("tunnel: {e:#}"))?;

  // The client: plain HTTP to the forward, or HTTPS with the server's name resolved to it.
  let local: SocketAddr = LOCAL.parse()?;
  let (base, client) = match &https {
    Some(host) => (
      format!("https://{host}:{}", local.port()),
      reqwest::Client::builder().resolve(host, local).build()?,
    ),
    None => (format!("http://{LOCAL}"), reqwest::Client::new()),
  };
  let device = "MediaBrowser Client=\"Peeby spike\", Device=\"Peeby WG spike\", \
                DeviceId=\"peeby-wg-spike\", Version=\"0.1\"";

  // The first answer through the tunnel: the handshake and a TCP connection.
  // onetun drops a connection made before its handshake is done: tried again until it answers.
  let mut tries = 0;
  let info = loop {
    tries += 1;
    match client
      .get(format!("{base}/System/Info/Public"))
      .timeout(Duration::from_secs(5))
      .send()
      .await
    {
      Ok(info) => break info,
      Err(e) if started.elapsed() < Duration::from_secs(20) => {
        eprintln!("try {tries}: {e}");
        tokio::time::sleep(Duration::from_millis(250)).await;
      }
      Err(e) => return Err(e.into()),
    }
  };
  println!(
    "first answer: {:?} after {:.0} ms",
    info.status(),
    started.elapsed().as_secs_f64() * 1000.
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
  let (authorization, user_id) = if saved {
    let (device_id, user_id, token) = peeby_session()?;
    let device = std::env::var("COMPUTERNAME").unwrap_or_default();
    let device = format!(
      "MediaBrowser Client=\"Peeby\", Device=\"{device}\", DeviceId=\"{device_id}\", Version=\"1.4.2\""
    );
    (format!("{device}, Token=\"{token}\""), user_id)
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
      "{base}/Users/{user_id}/Items?IncludeItemTypes=Movie,Episode&Recursive=true&Limit=1\
       &SortBy=Random&Fields=MediaSources"
    ))
    .header("Authorization", &authorization)
    .send()
    .await?
    .error_for_status()?
    .json()
    .await?;
  let item = &items["Items"][0];
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
  Ok(())
}

/// The debug profile's Jellyfin session: its device id, user id and token (never printed).
fn peeby_session() -> Result<(String, String, String), Error> {
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
  let (server, user, device) = (field("server_url")?, field("user_id")?, field("device_id")?);
  let token = keyring::Entry::new("peeby-dev", &format!("{server}|{user}"))?.get_password()?;
  Ok((device, user, token))
}

/// onetun's configuration from a wg-quick file, forwarding [`LOCAL`] to `server`.
fn config(text: &str, server: SocketAddr) -> Result<Config, Error> {
  let mut section = "";
  let (mut private, mut public, mut preshared) = (None, None, None);
  let (mut address, mut endpoint, mut keepalive) = (None, None, None);
  for line in text.lines() {
    let line = line.split('#').next().unwrap_or_default().trim();
    if line.starts_with('[') {
      section = line;
      continue;
    }
    let Some((key, value)) = line.split_once('=') else {
      continue;
    };
    let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
    match (section, key.as_str()) {
      ("[Interface]", "privatekey") => private = Some(key32(value)?),
      ("[Interface]", "address") => {
        // The first address, without its prefix length.
        let first = value.split(',').next().unwrap_or_default().trim();
        address = Some(
          first
            .split('/')
            .next()
            .unwrap_or_default()
            .parse::<IpAddr>()?,
        );
      }
      ("[Peer]", "publickey") => public = Some(key32(value)?),
      ("[Peer]", "presharedkey") => preshared = Some(key32(value)?),
      ("[Peer]", "endpoint") => {
        endpoint = std::net::ToSocketAddrs::to_socket_addrs(value)?.next();
      }
      ("[Peer]", "persistentkeepalive") => keepalive = value.parse().ok(),
      _ => {}
    }
  }
  Ok(Config {
    port_forwards: vec![PortForwardConfig {
      source: LOCAL.parse()?,
      destination: server,
      protocol: PortProtocol::Tcp,
      remote: false,
    }],
    remote_port_forwards: Vec::new(),
    private_key: Arc::new(StaticSecret::from(private.ok_or("no PrivateKey")?)),
    endpoint_public_key: Arc::new(PublicKey::from(public.ok_or("no peer PublicKey")?)),
    preshared_key: preshared,
    endpoint_addr: endpoint.ok_or("no peer Endpoint")?,
    endpoint_bind_addr: "0.0.0.0:0".parse()?,
    source_peer_ip: address.ok_or("no Interface Address")?,
    keepalive_seconds: keepalive.or(Some(25)),
    max_transmission_unit: 1420,
    log: "info".into(),
    warnings: Vec::new(),
    pcap_file: None,
  })
}

/// A 32-byte WireGuard key from its base64.
fn key32(text: &str) -> Result<[u8; 32], Error> {
  let bytes = base64(text).ok_or("a key is not base64")?;
  bytes.try_into().map_err(|_| "a key is not 32 bytes".into())
}

/// Standard base64, padded or not.
fn base64(text: &str) -> Option<Vec<u8>> {
  let value = |c: u8| -> Option<u32> {
    Some(match c {
      b'A'..=b'Z' => u32::from(c - b'A'),
      b'a'..=b'z' => u32::from(c - b'a') + 26,
      b'0'..=b'9' => u32::from(c - b'0') + 52,
      b'+' => 62,
      b'/' => 63,
      _ => return None,
    })
  };
  let digits: Vec<u32> = text
    .trim()
    .trim_end_matches('=')
    .bytes()
    .map(value)
    .collect::<Option<_>>()?;
  let mut out = Vec::new();
  for chunk in digits.chunks(4) {
    let n = chunk
      .iter()
      .enumerate()
      .fold(0u32, |n, (i, d)| n | (d << (18 - 6 * i)));
    let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
    out.extend_from_slice(&bytes[..chunk.len().saturating_sub(1)]);
  }
  Some(out)
}

#[cfg(test)]
mod tests {
  use super::base64;

  #[test]
  fn base64_decodes_a_key() {
    assert_eq!(base64("TWFu").unwrap(), b"Man");
    assert_eq!(base64("TWE=").unwrap(), b"Ma");
    assert_eq!(base64("TQ==").unwrap(), b"M");
    assert_eq!(base64(&"A".repeat(43)).unwrap().len(), 32);
  }
}
