use url::form_urlencoded::byte_serialize;

/// Identity the client reports to the server in every `Authorization` header.
#[derive(Clone, Debug)]
pub struct ClientInfo {
  /// Application name, e.g. `Peeby`.
  pub client: String,
  /// Human-readable device name, e.g. the hostname.
  pub device: String,
  /// Stable per-installation id. The server ties sessions and Quick Connect to it.
  pub device_id: String,
  /// Application version.
  pub version: String,
}

impl ClientInfo {
  /// Builds the `MediaBrowser …` authorization header value. Values are URL-encoded so quotes
  /// and commas in a device name cannot break the header; the server URL-decodes them.
  pub(crate) fn authorization(&self, token: Option<&str>) -> String {
    let mut header = format!(
      "MediaBrowser Client=\"{}\", Device=\"{}\", DeviceId=\"{}\", Version=\"{}\"",
      encode(&self.client),
      encode(&self.device),
      encode(&self.device_id),
      encode(&self.version),
    );
    if let Some(token) = token {
      header.push_str(&format!(", Token=\"{}\"", encode(token)));
    }
    header
  }
}

fn encode(value: &str) -> String {
  byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn info() -> ClientInfo {
    ClientInfo {
      client: "Peeby".into(),
      device: "Théo's \"PC\", desk".into(),
      device_id: "abc-123".into(),
      version: "0.1.0".into(),
    }
  }

  #[test]
  fn header_without_token() {
    assert_eq!(
      info().authorization(None),
      "MediaBrowser Client=\"Peeby\", Device=\"Th%C3%A9o%27s+%22PC%22%2C+desk\", DeviceId=\"abc-123\", Version=\"0.1.0\""
    );
  }

  #[test]
  fn header_with_token() {
    assert!(
      info()
        .authorization(Some("tok"))
        .ends_with(", Token=\"tok\"")
    );
  }
}
