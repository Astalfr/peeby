//! The local proxy, against a VPN server that never answers (a bare UDP socket on the loopback):
//! no network is used.

// Test helpers outside #[test] functions: a failure there should panic.
#![allow(clippy::unwrap_used)]

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

// Made-up keys.
const PRIVATE: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
const PEER: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";

async fn tunnel() -> (wireguard::Tunnel, UdpSocket) {
  let peer = UdpSocket::bind("127.0.0.1:0").await.unwrap();
  let text = format!(
    "[Interface]\nPrivateKey = {PRIVATE}\nAddress = 10.8.0.2/24\n\n[Peer]\nPublicKey = {PEER}\n\
     Endpoint = {}\nAllowedIPs = 10.8.0.0/24\n",
    peer.local_addr().unwrap()
  );
  let config = wireguard::Config::parse(&text).unwrap();
  let tunnel = wireguard::Tunnel::start(config, &tokio::runtime::Handle::current()).unwrap();
  (tunnel, peer)
}

/// `http://peeby:secret@127.0.0.1:port` as the address and the Basic credentials.
fn split(proxy: &str) -> (String, String) {
  let rest = proxy.strip_prefix("http://").unwrap();
  let (credentials, address) = rest.rsplit_once('@').unwrap();
  let encoded = base64_encode(credentials.as_bytes());
  (address.to_owned(), encoded)
}

fn base64_encode(bytes: &[u8]) -> String {
  const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  let mut out = String::new();
  for chunk in bytes.chunks(3) {
    let n = chunk
      .iter()
      .enumerate()
      .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
    for i in 0..4 {
      if i <= chunk.len() {
        out.push(char::from(TABLE[(n >> (18 - 6 * i)) as usize & 63]));
      } else {
        out.push('=');
      }
    }
  }
  out
}

async fn ask(address: &str, request: &str) -> String {
  let mut stream = TcpStream::connect(address).await.unwrap();
  stream.write_all(request.as_bytes()).await.unwrap();
  let mut answer = Vec::new();
  tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut answer))
    .await
    .unwrap()
    .unwrap();
  String::from_utf8(answer).unwrap()
}

#[tokio::test]
async fn asks_for_its_secret() {
  let (tunnel, _peer) = tunnel().await;
  let (address, _) = split(tunnel.proxy_url());
  let answer = ask(&address, "CONNECT 10.8.0.1:443 HTTP/1.1\r\n\r\n").await;
  assert!(answer.starts_with("HTTP/1.1 407"), "{answer}");
  assert!(answer.contains("Proxy-Authenticate: Basic"));
  let wrong = "CONNECT 10.8.0.1:443 HTTP/1.1\r\nProxy-Authorization: Basic cGVlYnk6bm8=\r\n\r\n";
  assert!(ask(&address, wrong).await.starts_with("HTTP/1.1 407"));
}

#[tokio::test]
async fn carries_only_its_allowed_ips() {
  let (tunnel, _peer) = tunnel().await;
  let (address, credentials) = split(tunnel.proxy_url());
  let request =
    format!("CONNECT 192.0.2.1:443 HTTP/1.1\r\nProxy-Authorization: Basic {credentials}\r\n\r\n");
  let answer = ask(&address, &request).await;
  assert!(answer.starts_with("HTTP/1.1 502"), "{answer}");
}

#[tokio::test]
async fn sends_a_handshake_and_ends_its_connections_when_dropped() {
  let (tunnel, peer) = tunnel().await;
  // The first handshake initiation (148 bytes, type 1).
  let mut datagram = [0; 2048];
  let n = tokio::time::timeout(Duration::from_secs(5), peer.recv(&mut datagram))
    .await
    .unwrap()
    .unwrap();
  assert_eq!((n, datagram[0]), (148, 1));
  assert_eq!(tunnel.status().handshake, None);

  // A connection inside the routes waits for a peer that never answers...
  let (address, credentials) = split(tunnel.proxy_url());
  let mut stream = TcpStream::connect(&address).await.unwrap();
  let request =
    format!("CONNECT 10.8.0.1:443 HTTP/1.1\r\nProxy-Authorization: Basic {credentials}\r\n\r\n");
  stream.write_all(request.as_bytes()).await.unwrap();
  let mut answer = [0; 64];
  let waiting = tokio::time::timeout(Duration::from_millis(500), stream.read(&mut answer)).await;
  assert!(waiting.is_err(), "answered before the peer did");

  // ...and the tunnel's end ends it.
  drop(tunnel);
  let mut rest = Vec::new();
  let ended = tokio::time::timeout(Duration::from_secs(20), stream.read_to_end(&mut rest)).await;
  assert!(ended.is_ok(), "still open after the tunnel stopped");
}
