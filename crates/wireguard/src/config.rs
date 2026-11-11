//! A wg-quick configuration file: the `[Interface]` (this side's key and addresses) and its one
//! `[Peer]` (the server's key, endpoint and routed addresses).

use std::fmt;
use std::net::IpAddr;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// A WireGuard configuration. Its keys never show in `Debug`.
#[derive(Clone)]
pub struct Config {
  pub(crate) private_key: [u8; 32],
  /// This side's addresses in the tunnel, with their prefix (`Address`).
  pub(crate) addresses: Vec<(IpAddr, u8)>,
  /// The name servers inside the tunnel (`DNS`'s addresses; its search domains are ignored).
  pub(crate) dns: Vec<IpAddr>,
  /// `MTU`, else 1420 (wg-quick's default on most links).
  pub(crate) mtu: u16,
  pub(crate) peer_key: [u8; 32],
  pub(crate) preshared_key: Option<[u8; 32]>,
  /// `host:port`, resolved when the tunnel starts.
  pub(crate) endpoint: String,
  /// What the peer routes (`AllowedIPs`): the only addresses the proxy carries.
  pub(crate) allowed_ips: Vec<(IpAddr, u8)>,
  /// `PersistentKeepalive`, seconds.
  pub(crate) keepalive: Option<u16>,
}

/// A configuration file that cannot be used. Never quotes the file (it holds keys).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
  /// A required key is missing (`PrivateKey`, `Address`, `PublicKey`, `Endpoint`).
  #[error("missing {0}")]
  Missing(&'static str),
  /// A key's value cannot be read.
  #[error("invalid {0}")]
  Invalid(&'static str),
  /// More than one `[Peer]`: the tunnel has one.
  #[error("more than one [Peer]")]
  SeveralPeers,
}

impl Config {
  /// Reads a wg-quick file. `DNS`'s servers resolve names through the tunnel (its search
  /// domains are ignored); `ListenPort` and the `Pre`/`Post` commands are ignored: nothing runs.
  pub fn parse(text: &str) -> Result<Self, ConfigError> {
    #[derive(PartialEq)]
    enum Section {
      None,
      Interface,
      Peer,
    }
    let mut section = Section::None;
    let mut peers = 0;
    let (mut private_key, mut addresses, mut mtu) = (None, Vec::new(), None);
    let mut dns = Vec::new();
    let (mut peer_key, mut preshared_key, mut endpoint) = (None, None, None);
    let (mut allowed_ips, mut keepalive) = (Vec::new(), None);
    for line in text.lines() {
      let line = line.split('#').next().unwrap_or_default().trim();
      if line.is_empty() {
        continue;
      }
      if line.starts_with('[') {
        section = match line.to_ascii_lowercase().as_str() {
          "[interface]" => Section::Interface,
          "[peer]" => {
            peers += 1;
            Section::Peer
          }
          _ => Section::None,
        };
        continue;
      }
      let Some((key, value)) = line.split_once('=') else {
        continue;
      };
      let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
      match (&section, key.as_str()) {
        (Section::Interface, "privatekey") => private_key = Some(key32(value, "PrivateKey")?),
        (Section::Interface, "address") => addresses.extend(cidrs(value, "Address")?),
        (Section::Interface, "dns") => dns.extend(
          value
            .split(',')
            .filter_map(|entry| entry.trim().parse::<IpAddr>().ok()),
        ),
        (Section::Interface, "mtu") => {
          mtu = Some(
            value
              .parse::<u16>()
              .ok()
              .filter(|mtu| (576..=9000).contains(mtu))
              .ok_or(ConfigError::Invalid("MTU"))?,
          )
        }
        (Section::Peer, "publickey") => peer_key = Some(key32(value, "PublicKey")?),
        (Section::Peer, "presharedkey") => preshared_key = Some(key32(value, "PresharedKey")?),
        (Section::Peer, "endpoint") => {
          let port = value
            .rsplit_once(':')
            .and_then(|(_, p)| p.parse::<u16>().ok());
          if port.is_none_or(|p| p == 0) {
            return Err(ConfigError::Invalid("Endpoint"));
          }
          endpoint = Some(value.to_owned());
        }
        (Section::Peer, "allowedips") => allowed_ips.extend(cidrs(value, "AllowedIPs")?),
        (Section::Peer, "persistentkeepalive") => {
          keepalive = match value {
            "off" => None,
            value => Some(
              value
                .parse::<u16>()
                .map_err(|_| ConfigError::Invalid("PersistentKeepalive"))?,
            )
            .filter(|&s| s > 0),
          }
        }
        _ => {}
      }
    }
    if peers > 1 {
      return Err(ConfigError::SeveralPeers);
    }
    if addresses.is_empty() {
      return Err(ConfigError::Missing("Address"));
    }
    Ok(Self {
      private_key: private_key.ok_or(ConfigError::Missing("PrivateKey"))?,
      addresses,
      dns,
      mtu: mtu.unwrap_or(1420),
      peer_key: peer_key.ok_or(ConfigError::Missing("PublicKey"))?,
      preshared_key,
      endpoint: endpoint.ok_or(ConfigError::Missing("Endpoint"))?,
      allowed_ips,
      keepalive,
    })
  }

  /// The configuration as a short wg-quick file, without what is ignored (comments, search
  /// domains, commands): what is stored (a Credential Manager entry holds at most 2,560 bytes). It holds
  /// the keys: never log it.
  pub fn to_wg_quick(&self) -> String {
    let list = |cidrs: &[(IpAddr, u8)]| {
      cidrs
        .iter()
        .map(|(ip, p)| format!("{ip}/{p}"))
        .collect::<Vec<_>>()
        .join(", ")
    };
    let mut text = format!(
      "[Interface]\nPrivateKey = {}\nAddress = {}\nMTU = {}\n",
      STANDARD.encode(self.private_key),
      list(&self.addresses),
      self.mtu,
    );
    if !self.dns.is_empty() {
      let dns: Vec<String> = self.dns.iter().map(IpAddr::to_string).collect();
      text.push_str(&format!("DNS = {}\n", dns.join(", ")));
    }
    text.push_str(&format!(
      "\n[Peer]\nPublicKey = {}\n",
      STANDARD.encode(self.peer_key)
    ));
    if let Some(key) = &self.preshared_key {
      text.push_str(&format!("PresharedKey = {}\n", STANDARD.encode(key)));
    }
    text.push_str(&format!("Endpoint = {}\n", self.endpoint));
    if !self.allowed_ips.is_empty() {
      text.push_str(&format!("AllowedIPs = {}\n", list(&self.allowed_ips)));
    }
    if let Some(seconds) = self.keepalive {
      text.push_str(&format!("PersistentKeepalive = {seconds}\n"));
    }
    text
  }

  /// The server's `host:port`, for the settings (not a secret, but the user's: never at info).
  pub fn endpoint(&self) -> &str {
    &self.endpoint
  }

  /// This side's addresses, `10.8.0.2/24`.
  pub fn addresses(&self) -> Vec<String> {
    self
      .addresses
      .iter()
      .map(|(ip, p)| format!("{ip}/{p}"))
      .collect()
  }

  /// The name servers inside the tunnel, `192.168.1.50`.
  pub fn dns(&self) -> Vec<String> {
    self.dns.iter().map(IpAddr::to_string).collect()
  }

  /// What the tunnel carries, `192.168.1.0/24`.
  pub fn allowed_ips(&self) -> Vec<String> {
    self
      .allowed_ips
      .iter()
      .map(|(ip, p)| format!("{ip}/{p}"))
      .collect()
  }

  /// Whether the peer routes `ip` (`AllowedIPs`).
  pub fn routes(&self, ip: IpAddr) -> bool {
    self
      .allowed_ips
      .iter()
      .any(|&(net, prefix)| contains(net, prefix, ip))
  }
}

impl fmt::Debug for Config {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("Config")
      .field("addresses", &self.addresses())
      .field("dns", &self.dns())
      .field("mtu", &self.mtu)
      .field("endpoint", &self.endpoint)
      .field("allowed_ips", &self.allowed_ips())
      .field("keepalive", &self.keepalive)
      .finish_non_exhaustive()
  }
}

/// Whether `ip` is in `net/prefix`.
fn contains(net: IpAddr, prefix: u8, ip: IpAddr) -> bool {
  match (net, ip) {
    (IpAddr::V4(net), IpAddr::V4(ip)) => {
      let mask = u32::MAX
        .checked_shl(32 - u32::from(prefix.min(32)))
        .unwrap_or(0);
      u32::from(net) & mask == u32::from(ip) & mask
    }
    (IpAddr::V6(net), IpAddr::V6(ip)) => {
      let mask = u128::MAX
        .checked_shl(128 - u32::from(prefix.min(128)))
        .unwrap_or(0);
      u128::from(net) & mask == u128::from(ip) & mask
    }
    _ => false,
  }
}

/// A base64 key of 32 bytes.
fn key32(value: &str, name: &'static str) -> Result<[u8; 32], ConfigError> {
  STANDARD
    .decode(value)
    .ok()
    .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
    .ok_or(ConfigError::Invalid(name))
}

/// `10.8.0.2/24, fd00::2/64`; an address without a prefix is a host.
fn cidrs(value: &str, name: &'static str) -> Result<Vec<(IpAddr, u8)>, ConfigError> {
  value
    .split(',')
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(|cidr| {
      let (ip, prefix) = cidr.split_once('/').unwrap_or((cidr, ""));
      let ip: IpAddr = ip.parse().map_err(|_| ConfigError::Invalid(name))?;
      let max = if ip.is_ipv4() { 32 } else { 128 };
      let prefix = match prefix {
        "" => max,
        p => p
          .parse::<u8>()
          .ok()
          .filter(|&p| p <= max)
          .ok_or(ConfigError::Invalid(name))?,
      };
      Ok((ip, prefix))
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  // Made-up keys (32 bytes of 1, 2 and 3).
  const KEY1: &str = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
  const KEY2: &str = "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=";
  const KEY3: &str = "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM=";

  fn file() -> String {
    format!(
      "# a comment\n[Interface]\nPrivateKey = {KEY1}\nAddress = 10.8.0.2/24, fd00::2/64\n\
       DNS = 10.8.0.1, home.lan\nMTU = 1380\n\n[Peer]\nPublicKey = {KEY2}\nPresharedKey = {KEY3}\n\
       Endpoint = vpn.example.org:51820\nAllowedIPs = 192.168.1.0/24, 10.8.0.0/24\n\
       PersistentKeepalive = 25\n"
    )
  }

  #[test]
  fn reads_a_wg_quick_file() {
    let config = Config::parse(&file()).unwrap();
    assert_eq!(config.private_key, [1; 32]);
    assert_eq!(config.peer_key, [2; 32]);
    assert_eq!(config.preshared_key, Some([3; 32]));
    assert_eq!(config.addresses(), ["10.8.0.2/24", "fd00::2/64"]);
    assert_eq!(config.dns(), ["10.8.0.1"]);
    assert_eq!(config.mtu, 1380);
    assert_eq!(config.endpoint(), "vpn.example.org:51820");
    assert_eq!(config.allowed_ips(), ["192.168.1.0/24", "10.8.0.0/24"]);
    assert_eq!(config.keepalive, Some(25));
  }

  #[test]
  fn writes_itself_back() {
    let config = Config::parse(&file()).unwrap();
    let text = config.to_wg_quick();
    assert!(text.contains("DNS = 10.8.0.1") && !text.contains('#'));
    let again = Config::parse(&text).unwrap();
    assert_eq!(format!("{again:?}"), format!("{config:?}"));
    assert_eq!(
      (again.private_key, again.peer_key, again.preshared_key),
      (config.private_key, config.peer_key, config.preshared_key)
    );
  }

  #[test]
  fn routes_only_its_allowed_ips() {
    let config = Config::parse(&file()).unwrap();
    assert!(config.routes("192.168.1.50".parse().unwrap()));
    assert!(!config.routes("192.168.2.50".parse().unwrap()));
    assert!(!config.routes("fd00::1".parse().unwrap()));
    let everything = file().replace("192.168.1.0/24, 10.8.0.0/24", "0.0.0.0/0, ::/0");
    let config = Config::parse(&everything).unwrap();
    assert!(config.routes("1.2.3.4".parse().unwrap()));
    assert!(config.routes("2001:db8::1".parse().unwrap()));
  }

  #[test]
  fn hides_its_keys() {
    let debug = format!("{:?}", Config::parse(&file()).unwrap());
    for key in [KEY1, KEY2, KEY3] {
      assert!(!debug.contains(key));
    }
    assert!(!debug.contains("[1, 1"));
  }

  #[test]
  fn says_what_is_wrong() {
    let without = |line: &str| Config::parse(&file().replace(line, "")).err();
    assert_eq!(
      without(&format!("PrivateKey = {KEY1}")),
      Some(ConfigError::Missing("PrivateKey"))
    );
    assert_eq!(
      without("Endpoint = vpn.example.org:51820"),
      Some(ConfigError::Missing("Endpoint"))
    );
    let bad = file().replace(KEY2, "not a key");
    assert_eq!(
      Config::parse(&bad).err(),
      Some(ConfigError::Invalid("PublicKey"))
    );
    let two = format!("{}\n[Peer]\nPublicKey = {KEY2}\n", file());
    assert_eq!(Config::parse(&two).err(), Some(ConfigError::SeveralPeers));
    let error = Config::parse(&bad).unwrap_err().to_string();
    assert!(!error.contains("not a key"));
  }
}
