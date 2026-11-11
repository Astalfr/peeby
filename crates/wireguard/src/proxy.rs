//! The local HTTP proxy in front of the tunnel: `CONNECT host:port` (HTTPS: TLS runs end to end,
//! the proxy only carries it) and absolute URLs (plain HTTP: the request line is rewritten to a
//! path, then the connection is carried as is).
//!
//! It listens on the loopback, and only for whoever has its secret (Basic credentials, from the
//! proxy URL; a request without them gets a 407 and its challenge, as mpv's FFmpeg expects):
//! another program on this PC cannot use the tunnel. It carries only the addresses the peer
//! routes (`AllowedIPs`).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::Config;
use crate::tunnel::Shared;

/// The proxy's user name, in its URL.
pub(crate) const USER: &str = "peeby";
/// A request's head is at most this long.
const MAX_HEAD: usize = 16 << 10;
/// How long a connection through the tunnel may take to open (a first handshake included).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Serves the proxy until the tunnel is dropped.
pub(crate) async fn serve(
  listener: std::net::TcpListener,
  shared: Arc<Shared>,
  config: Config,
  secret: String,
) {
  let listener = match TcpListener::from_std(listener) {
    Ok(listener) => listener,
    Err(e) => {
      tracing::error!("WireGuard proxy: {e}");
      return;
    }
  };
  let credentials: Arc<str> = STANDARD.encode(format!("{USER}:{secret}")).into();
  let config = Arc::new(config);
  loop {
    match listener.accept().await {
      Ok((client, _)) => {
        let (shared, config, credentials) = (shared.clone(), config.clone(), credentials.clone());
        tokio::spawn(async move {
          // A stopped tunnel ends its connections (dropped, they close): without the driving
          // task they would wait forever.
          tokio::select! {
            carried = carry(client, &shared, &config, &credentials) => {
              if let Err(e) = carried {
                tracing::debug!("WireGuard proxy: {e}");
              }
            }
            () = shared.stopped() => {}
          }
        });
      }
      Err(e) => {
        tracing::debug!("WireGuard proxy: accept: {e}");
        tokio::time::sleep(Duration::from_millis(100)).await;
      }
    }
  }
}

/// One client connection: its first request picks the server, then both ways are carried.
async fn carry(
  mut client: TcpStream,
  shared: &Arc<Shared>,
  config: &Config,
  credentials: &str,
) -> std::io::Result<()> {
  client.set_nodelay(true)?;
  let mut head = Vec::with_capacity(1024);
  let end = loop {
    let mut chunk = [0; 4096];
    let n = client.read(&mut chunk).await?;
    if n == 0 {
      return Ok(());
    }
    head.extend_from_slice(&chunk[..n]);
    if let Some(end) = find(&head, b"\r\n\r\n") {
      break end + 4;
    }
    if head.len() > MAX_HEAD {
      return answer(&mut client, "431 Request Header Fields Too Large").await;
    }
  };
  let Some(request) = Request::parse(&head[..end]) else {
    return answer(&mut client, "400 Bad Request").await;
  };
  if request.credentials != Some(credentials) {
    let challenge = "HTTP/1.1 407 Proxy Authentication Required\r\n\
                     Proxy-Authenticate: Basic realm=\"peeby\"\r\n\
                     Content-Length: 0\r\nConnection: close\r\n\r\n";
    client.write_all(challenge.as_bytes()).await?;
    return client.shutdown().await;
  }
  let Some(remote) = resolve(&request.host, request.port, shared, config).await else {
    tracing::warn!(
      "WireGuard proxy: {} is not routed by the tunnel (AllowedIPs)",
      request.host
    );
    return answer(&mut client, "502 Bad Gateway").await;
  };
  let mut server = shared.connect(remote)?;
  match tokio::time::timeout(CONNECT_TIMEOUT, server.established()).await {
    Ok(Ok(())) => {}
    Ok(Err(e)) => {
      tracing::debug!("WireGuard proxy: {e}");
      return answer(&mut client, "502 Bad Gateway").await;
    }
    Err(_) => return answer(&mut client, "504 Gateway Timeout").await,
  }
  let rest = &head[end..];
  match &request.path {
    None => {
      client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await?
    }
    Some(_) => server.write_all(&request.forwarded(&head[..end])).await?,
  }
  if !rest.is_empty() {
    server.write_all(rest).await?;
  }
  tokio::io::copy_bidirectional(&mut client, &mut server).await?;
  Ok(())
}

/// An answer without a body, then the end of the connection.
async fn answer(client: &mut TcpStream, status: &str) -> std::io::Result<()> {
  let answer = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
  client.write_all(answer.as_bytes()).await?;
  client.shutdown().await
}

/// The address of `host` the tunnel routes: from the tunnel's name server first (`DNS`, as
/// wg-quick does: a name the VPN knows), else from Windows'.
async fn resolve(host: &str, port: u16, shared: &Shared, config: &Config) -> Option<SocketAddr> {
  if let Ok(ip) = host
    .trim_start_matches('[')
    .trim_end_matches(']')
    .parse::<IpAddr>()
  {
    return config.routes(ip).then_some(SocketAddr::new(ip, port));
  }
  if let Some(ip) = shared.lookup(host).await.filter(|&ip| config.routes(ip)) {
    return Some(SocketAddr::new(ip, port));
  }
  tokio::net::lookup_host((host, port))
    .await
    .ok()?
    .find(|addr| config.routes(addr.ip()))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
  haystack.windows(needle.len()).position(|w| w == needle)
}

/// A proxy request's head.
#[derive(Debug, PartialEq)]
struct Request<'a> {
  method: &'a str,
  host: String,
  port: u16,
  /// The path of an absolute URL (plain HTTP); `None` for `CONNECT`.
  path: Option<&'a str>,
  version: &'a str,
  /// The `Proxy-Authorization: Basic` credentials, base64.
  credentials: Option<&'a str>,
}

impl<'a> Request<'a> {
  fn parse(head: &'a [u8]) -> Option<Self> {
    let head = std::str::from_utf8(head).ok()?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?, first.next()?, first.next()?);
    let credentials = lines.find_map(|line| {
      let (name, value) = line.split_once(':')?;
      if !name.trim().eq_ignore_ascii_case("proxy-authorization") {
        return None;
      }
      let (scheme, value) = value.trim().split_once(' ')?;
      scheme.eq_ignore_ascii_case("basic").then_some(value.trim())
    });
    let (authority, path, default_port) = if method.eq_ignore_ascii_case("CONNECT") {
      (target, None, 443)
    } else {
      let rest = target.strip_prefix("http://")?;
      match rest.find('/') {
        Some(slash) => (&rest[..slash], Some(&rest[slash..]), 80),
        None => (rest, Some("/"), 80),
      }
    };
    let (host, port) = match authority.rsplit_once(':') {
      Some((host, port)) if !port.contains(']') => (host, port.parse().ok()?),
      _ => (authority, default_port),
    };
    if host.is_empty() {
      return None;
    }
    Some(Self {
      method,
      host: host.to_owned(),
      port,
      path,
      version,
      credentials,
    })
  }

  /// A plain HTTP request as the server takes it: its path, without the proxy's headers.
  fn forwarded(&self, head: &[u8]) -> Vec<u8> {
    let head = String::from_utf8_lossy(head);
    let mut out = format!(
      "{} {} {}\r\n",
      self.method,
      self.path.unwrap_or("/"),
      self.version
    );
    for line in head.split("\r\n").skip(1) {
      if line.is_empty() {
        continue;
      }
      let name = line.split(':').next().unwrap_or_default().trim();
      if name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("proxy-connection")
      {
        continue;
      }
      out.push_str(line);
      out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.into_bytes()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_a_connect() {
    let head = b"CONNECT media.example.org:443 HTTP/1.1\r\nHost: media.example.org:443\r\n\
                 Proxy-Authorization: Basic cGVlYnk6czNjcjN0\r\n\r\n";
    let request = Request::parse(head).unwrap();
    assert_eq!(request.host, "media.example.org");
    assert_eq!(request.port, 443);
    assert_eq!(request.path, None);
    assert_eq!(request.credentials, Some("cGVlYnk6czNjcjN0"));
  }

  #[test]
  fn reads_an_absolute_url_and_forwards_its_path() {
    let head = b"GET http://10.0.0.5:8096/Videos/1/stream?static=true HTTP/1.1\r\n\
                 Host: 10.0.0.5:8096\r\nProxy-Authorization: Basic x\r\nRange: bytes=0-\r\n\
                 Proxy-Connection: keep-alive\r\n\r\n";
    let request = Request::parse(head).unwrap();
    assert_eq!((request.host.as_str(), request.port), ("10.0.0.5", 8096));
    assert_eq!(request.path, Some("/Videos/1/stream?static=true"));
    let forwarded = String::from_utf8(request.forwarded(head)).unwrap();
    assert_eq!(
      forwarded,
      "GET /Videos/1/stream?static=true HTTP/1.1\r\nHost: 10.0.0.5:8096\r\n\
       Range: bytes=0-\r\n\r\n"
    );
  }

  #[test]
  fn reads_ipv6_and_default_ports() {
    let request = Request::parse(b"CONNECT [fd00::5]:8920 HTTP/1.1\r\n\r\n").unwrap();
    assert_eq!((request.host.as_str(), request.port), ("[fd00::5]", 8920));
    let request = Request::parse(b"GET http://jellyfin HTTP/1.1\r\n\r\n").unwrap();
    assert_eq!(
      (request.host.as_str(), request.port, request.path),
      ("jellyfin", 80, Some("/"))
    );
    assert_eq!(Request::parse(b"GET /relative HTTP/1.1\r\n\r\n"), None);
    assert_eq!(request.credentials, None);
  }
}
