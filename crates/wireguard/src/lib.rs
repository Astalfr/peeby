//! A userspace WireGuard tunnel behind a local HTTP proxy (ADR 0037).
//!
//! [`Tunnel::start`] runs WireGuard's protocol (boringtun) over a UDP socket and a TCP/IP stack
//! (smoltcp) inside it, without a network adapter or administrator rights. The other side of the
//! tunnel is reached through a local HTTP proxy ([`Tunnel::proxy_url`]): `CONNECT` for HTTPS,
//! absolute URLs for plain HTTP, so that an HTTP client (reqwest) and mpv (`http-proxy`) only
//! need to be pointed at it. The proxy listens on the loopback only, asks for a secret of its own
//! (in the URL's credentials), and carries only the addresses the peer's `AllowedIPs` route.
//! The proxy is always there, the VPN only when used: the session with the peer (UDP, handshakes,
//! keepalives) opens at the first connection and closes after [`IDLE`] without one.
//!
//! The configuration is a wg-quick file ([`Config::parse`]): its keys are secrets, never in a log
//! or an error message, and so is the proxy's URL (it holds the proxy's secret).

mod config;
mod proxy;
mod tunnel;

pub use config::{Config, ConfigError};
pub use tunnel::{IDLE, Status, Tunnel};

/// A tunnel that cannot start.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// The proxy's local socket.
  #[error("cannot open the tunnel's local proxy: {0}")]
  Io(#[from] std::io::Error),
}
