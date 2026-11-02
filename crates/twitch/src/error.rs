/// Errors of the Twitch client. Messages never contain a token, a signature or a usher/CDN URL
/// (they carry the user's IP).
#[derive(Debug, thiserror::Error)]
pub enum Error {
  /// Network or HTTP protocol error (the URL is stripped).
  #[error("network error: {0}")]
  Http(#[source] reqwest::Error),
  /// The server answered with an error status not mapped to a variant below.
  #[error("HTTP {status} from {endpoint}{}", code.as_deref().map(|c| format!(" ({c})")).unwrap_or_default())]
  Status {
    /// HTTP status code.
    status: u16,
    /// Endpoint name, e.g. `usher`.
    endpoint: &'static str,
    /// usher's `error_code`, when it gives one.
    code: Option<String>,
  },
  /// The response does not have the expected shape.
  #[error("unexpected response from {endpoint}: {detail}")]
  Parse {
    /// Endpoint name.
    endpoint: &'static str,
    /// What was missing.
    detail: String,
  },
  /// GQL refused the web `auth-token` (HTTP 401): revoked, or the user changed their password.
  #[error("Twitch does not accept the auth-token")]
  NotLoggedIn,
  /// The playback token needs a Client-Integrity token, which only a browser can make (ADR 0010).
  #[error("Twitch asks for an integrity check that needs a browser")]
  IntegrityRequired,
  /// The channel is not live (usher answers 404).
  #[error("the channel is offline")]
  Offline,
  /// No such channel or VOD (the token query answers `null`).
  #[error("no such channel or video")]
  NotFound,
  /// A VOD for the channel's subscribers (`vod_manifest_restricted`, `unauthorized_entitlements`).
  #[error("this video is for the channel's subscribers")]
  SubscriberOnly,
  /// The Helix session is over: the refresh token was refused (revoked, unused for 30 days, or
  /// already spent). The user signs in again.
  #[error("the Twitch session expired")]
  SessionExpired,
  /// Helix's rate limit is spent for now (HTTP 429).
  #[error("too many Twitch requests, try again in a moment")]
  RateLimited,
  /// The device code expired before the user approved it.
  #[error("the sign-in code expired")]
  DeviceExpired,
  /// The user refused the sign-in on twitch.tv.
  #[error("the sign-in was refused")]
  DeviceDenied,
  /// Another answer of `id.twitch.tv`.
  #[error("Twitch sign-in: {0}")]
  Auth(String),
  /// The local playlist proxy could not start.
  #[error("local playlist proxy: {0}")]
  Proxy(String),
  /// Twitch refused a spend of channel points, with its code (`NOT_ENOUGH_POINTS`, `COOLDOWN`…).
  #[error("Twitch refused: {0}")]
  Refused(String),
  /// Any other GQL error, with its message.
  #[error("GQL error: {0}")]
  Gql(String),
}

impl From<reqwest::Error> for Error {
  fn from(e: reqwest::Error) -> Self {
    Self::Http(e.without_url())
  }
}

/// Result alias of the crate.
pub type Result<T> = std::result::Result<T, Error>;
