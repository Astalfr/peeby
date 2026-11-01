/// Errors of the InnerTube client. Messages never contain cookies or stream URLs.
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// Network or HTTP protocol error (the URL is stripped: stream URLs carry the user's IP).
  #[error("network error: {0}")]
  Http(#[source] reqwest::Error),
  /// The server answered with an error status.
  #[error("HTTP {status} from {endpoint}")]
  Status {
    /// HTTP status code.
    status: u16,
    /// Endpoint name, e.g. `browse`.
    endpoint: &'static str,
  },
  /// The response does not have the expected shape.
  #[error("unexpected response from {endpoint}: {detail}")]
  Parse {
    /// Endpoint name.
    endpoint: &'static str,
    /// What was missing.
    detail: String,
  },
  /// Cookies were sent but YouTube answered as signed out (`logged_in=0`): expired or rotated
  /// cookies. Never treated as an empty feed.
  #[error("YouTube does not accept the session cookies (signed out): import fresh cookies")]
  NotLoggedIn,
  /// The page needs a signed-in session and no cookies are set.
  #[error("this page needs a signed-in session")]
  SignInRequired,
  /// The cookie file is unusable.
  #[error("invalid cookies file: {0}")]
  Cookies(String),
  /// The video cannot be played with this client (`playabilityStatus` other than `OK`).
  #[error("video unavailable ({status}): {reason}")]
  Unplayable {
    /// `playabilityStatus.status`, e.g. `LOGIN_REQUIRED`.
    status: String,
    /// Human-readable reason from YouTube, if any.
    reason: String,
  },
}

impl From<reqwest::Error> for Error {
  fn from(e: reqwest::Error) -> Self {
    Self::Http(e.without_url())
  }
}

/// Result alias of the crate.
pub type Result<T> = std::result::Result<T, Error>;
