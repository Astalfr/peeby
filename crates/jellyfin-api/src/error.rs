use std::fmt;

/// Errors returned by the Jellyfin client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// The server URL could not be parsed.
  #[error("invalid server URL: {0}")]
  InvalidUrl(#[from] url::ParseError),
  /// Network or protocol failure.
  #[error("request failed: {0}")]
  Http(#[from] reqwest::Error),
  /// The server answered with a non-success status.
  #[error("{endpoint} returned HTTP {status}: {body}")]
  Status {
    /// Endpoint path, without query string.
    endpoint: String,
    /// HTTP status code.
    status: u16,
    /// Start of the response body.
    body: Snippet,
  },
  /// The response body did not match the expected model.
  #[error("{endpoint}: unexpected response: {source}")]
  Decode {
    /// Endpoint path, without query string.
    endpoint: String,
    /// The JSON error.
    source: serde_json::Error,
  },
  /// The call needs a logged-in session.
  #[error("not authenticated")]
  NotAuthenticated,
  /// The server would only play this item by transcoding it, which Peeby refuses.
  #[error("the server wants to transcode this file ({})", if reasons.is_empty() { "no reason given".to_owned() } else { reasons.join(", ") })]
  TranscodingRequired {
    /// `TranscodeReasons` reported by the server.
    reasons: Vec<String>,
  },
  /// The server refused playback (`PlaybackInfo.ErrorCode`).
  #[error("the server refused playback: {code}")]
  PlaybackRefused {
    /// e.g. `NotAllowed`, `NoCompatibleStream`.
    code: String,
  },
  /// The server is older than the minimum supported version.
  #[error("Jellyfin {version} is not supported (12.0 or newer is required)")]
  UnsupportedServer {
    /// Version reported by the server.
    version: String,
  },
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, Error>;

/// The first bytes of an error body, so errors stay readable and bounded.
#[derive(Debug)]
pub struct Snippet(pub String);

impl Snippet {
  const MAX: usize = 300;

  pub(crate) fn new(body: &str) -> Self {
    let trimmed = body.trim();
    match trimmed.char_indices().nth(Self::MAX) {
      Some((cut, _)) => Self(format!("{}…", &trimmed[..cut])),
      None => Self(trimmed.to_owned()),
    }
  }
}

impl fmt::Display for Snippet {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if self.0.is_empty() {
      f.write_str("(empty body)")
    } else {
      f.write_str(&self.0)
    }
  }
}
