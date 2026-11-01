//! Signing in to Helix with the Device Code Grant (docs/twitch-plan.md step 5): Peeby is a public
//! client on dev.twitch.tv, so there is no secret, and no app token either.
//!
//! - [`Auth::start_device`] gives a code the user enters at `twitch.tv/activate`;
//! - [`Auth::poll`] is called every `interval` seconds until the user approves;
//! - the access token lasts about 4 h; [`Session`] refreshes it when Helix refuses it. Refresh
//!   tokens are **single use**: the new one is handed to the app's `store` callback, which writes it
//!   to the Credential Manager, **before** the new access token is used, and two refreshes never run
//!   at once;
//! - Twitch requires validating the token at startup and every hour ([`Session::validate`]).
//!
//! Tokens never appear in logs or errors; `Debug` hides them.

use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::error::{Error, Result};

/// Peeby's application on dev.twitch.tv (public client, chosen by the user on 2026-11-01). A
/// Client-ID is not a secret.
pub const CLIENT_ID: &str = "zsxaog2l339o1lbld85h17ksscnjwr";

/// Scopes asked for: the followed channels.
pub const SCOPES: &str = "user:read:follows";

/// A device code waiting for the user.
#[derive(Clone)]
pub struct DeviceCode {
  /// Secret of this sign-in attempt (polled with), hidden from `Debug`.
  pub device_code: String,
  /// The code the user types, e.g. `ABCDEFGH`.
  pub user_code: String,
  /// The page that takes it, with the code filled in.
  pub verification_uri: String,
  /// Seconds before the code expires (about 1800).
  pub expires_in: u64,
  /// Seconds between two polls (about 5).
  pub interval: u64,
}

impl std::fmt::Debug for DeviceCode {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("DeviceCode")
      .field("user_code", &self.user_code)
      .field("expires_in", &self.expires_in)
      .finish_non_exhaustive()
  }
}

/// An access token and its refresh token.
#[derive(Clone)]
pub struct Tokens {
  /// Helix `Authorization: Bearer …` (about 4 h).
  pub access: String,
  /// Single use; expires after 30 days unused.
  pub refresh: String,
}

impl std::fmt::Debug for Tokens {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("Tokens(…)")
  }
}

/// Where a sign-in stands after a poll.
#[derive(Debug)]
pub enum Poll {
  /// The user has not approved yet.
  Pending,
  /// Polling too often: wait longer before the next one.
  SlowDown,
  /// Approved.
  Done(Tokens),
}

/// What `/oauth2/validate` says about a token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validation {
  /// The user's login (lowercase).
  pub login: String,
  /// The user's numeric id.
  pub user_id: String,
  /// Seconds left.
  pub expires_in: u64,
  /// Granted scopes.
  pub scopes: Vec<String>,
}

/// Reads a `/oauth2/device` answer.
pub fn parse_device(answer: &Value) -> Result<DeviceCode> {
  let text = |key: &str| {
    answer[key].as_str().map(str::to_owned).ok_or(Error::Parse {
      endpoint: "oauth2/device",
      detail: format!("no {key}"),
    })
  };
  Ok(DeviceCode {
    device_code: text("device_code")?,
    user_code: text("user_code")?,
    verification_uri: text("verification_uri")?,
    expires_in: answer["expires_in"].as_u64().unwrap_or(1800),
    interval: answer["interval"].as_u64().unwrap_or(5).max(1),
  })
}

/// Reads a `/oauth2/token` answer of the device grant, success or not.
pub fn parse_poll(status: u16, answer: &Value) -> Result<Poll> {
  if status == 200 {
    return parse_tokens(answer).map(Poll::Done);
  }
  match answer["message"].as_str().unwrap_or_default() {
    "authorization_pending" => Ok(Poll::Pending),
    "slow_down" => Ok(Poll::SlowDown),
    "expired_token" => Err(Error::DeviceExpired),
    "access_denied" => Err(Error::DeviceDenied),
    other => Err(Error::Auth(format!("HTTP {status}: {other}"))),
  }
}

/// Reads tokens from a `/oauth2/token` answer (HTTP 200).
pub fn parse_tokens(answer: &Value) -> Result<Tokens> {
  match (
    answer["access_token"].as_str(),
    answer["refresh_token"].as_str(),
  ) {
    (Some(access), Some(refresh)) => Ok(Tokens {
      access: access.to_owned(),
      refresh: refresh.to_owned(),
    }),
    _ => Err(Error::Parse {
      endpoint: "oauth2/token",
      detail: "no access or refresh token".into(),
    }),
  }
}

/// The error of a refresh that is not HTTP 200: an invalid or expired refresh token signs out.
pub fn parse_refresh_error(status: u16, answer: &Value) -> Error {
  let message = answer["message"].as_str().unwrap_or_default();
  if status == 400 || status == 401 {
    Error::SessionExpired
  } else {
    Error::Auth(format!("HTTP {status}: {message}"))
  }
}

/// Reads a `/oauth2/validate` answer; 401 is an invalid token.
pub fn parse_validation(status: u16, answer: &Value) -> Result<Validation> {
  if status == 401 {
    return Err(Error::SessionExpired);
  }
  if status != 200 {
    return Err(Error::Status {
      status,
      endpoint: "oauth2/validate",
      code: None,
    });
  }
  Ok(Validation {
    login: answer["login"].as_str().unwrap_or_default().to_owned(),
    user_id: answer["user_id"].as_str().unwrap_or_default().to_owned(),
    expires_in: answer["expires_in"].as_u64().unwrap_or(0),
    scopes: answer["scopes"]
      .as_array()
      .map(|s| {
        s.iter()
          .filter_map(|s| s.as_str().map(str::to_owned))
          .collect()
      })
      .unwrap_or_default(),
  })
}

/// The `id.twitch.tv` endpoints.
#[derive(Clone)]
pub struct Auth {
  http: reqwest::Client,
  client_id: String,
}

impl Auth {
  /// With Peeby's [`CLIENT_ID`], or the settings' override.
  pub fn new(client_id: Option<String>) -> Result<Self> {
    Ok(Self {
      http: reqwest::Client::builder().build()?,
      client_id: client_id.unwrap_or_else(|| CLIENT_ID.to_owned()),
    })
  }

  /// The Client-ID in use (Helix's `Client-Id` header).
  pub fn client_id(&self) -> &str {
    &self.client_id
  }

  async fn post(&self, url: &str, form: &[(&str, &str)]) -> Result<(u16, Value)> {
    // `application/x-www-form-urlencoded` by hand: reqwest's `form` feature is not enabled.
    let body = reqwest::Url::parse_with_params("http://form.invalid/", form)
      .ok()
      .and_then(|u| u.query().map(str::to_owned))
      .unwrap_or_default();
    let response = self
      .http
      .post(url)
      .header("Content-Type", "application/x-www-form-urlencoded")
      .body(body)
      .send()
      .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
  }

  /// A new device code for [`SCOPES`].
  pub async fn start_device(&self) -> Result<DeviceCode> {
    let (status, answer) = self
      .post(
        "https://id.twitch.tv/oauth2/device",
        &[("client_id", &self.client_id), ("scopes", SCOPES)],
      )
      .await?;
    if status != 200 {
      return Err(Error::Auth(format!(
        "HTTP {status}: {}",
        answer["message"].as_str().unwrap_or_default()
      )));
    }
    parse_device(&answer)
  }

  /// One poll of a device code.
  pub async fn poll(&self, device: &DeviceCode) -> Result<Poll> {
    let (status, answer) = self
      .post(
        "https://id.twitch.tv/oauth2/token",
        &[
          ("client_id", &self.client_id),
          ("scopes", SCOPES),
          ("device_code", &device.device_code),
          ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ],
      )
      .await?;
    parse_poll(status, &answer)
  }

  /// New tokens for `refresh`, which is spent: save the new refresh token at once.
  pub async fn refresh(&self, refresh: &str) -> Result<Tokens> {
    let (status, answer) = self
      .post(
        "https://id.twitch.tv/oauth2/token",
        &[
          ("client_id", &self.client_id),
          ("grant_type", "refresh_token"),
          ("refresh_token", refresh),
        ],
      )
      .await?;
    if status != 200 {
      return Err(parse_refresh_error(status, &answer));
    }
    parse_tokens(&answer)
  }

  /// Who `access` belongs to, and whether it is still valid.
  pub async fn validate(&self, access: &str) -> Result<Validation> {
    let response = self
      .http
      .get("https://id.twitch.tv/oauth2/validate")
      .header("Authorization", format!("OAuth {access}"))
      .send()
      .await?;
    let status = response.status().as_u16();
    let text = response.text().await?;
    parse_validation(status, &serde_json::from_str(&text).unwrap_or(Value::Null))
  }

  /// Revokes `access` (sign-out). Twitch answers 200 even for an unknown token.
  pub async fn revoke(&self, access: &str) -> Result<()> {
    let (status, answer) = self
      .post(
        "https://id.twitch.tv/oauth2/revoke",
        &[("client_id", &self.client_id), ("token", access)],
      )
      .await?;
    if status == 200 {
      Ok(())
    } else {
      Err(Error::Auth(format!(
        "HTTP {status}: {}",
        answer["message"].as_str().unwrap_or_default()
      )))
    }
  }
}

/// Saves a new refresh token (the Credential Manager, a blocking call: it runs on a blocking
/// thread). An error is logged; the session goes on in memory.
pub type Store = Arc<dyn Fn(&str) -> std::result::Result<(), String> + Send + Sync>;

/// A signed-in Helix session: the current tokens, refreshed one at a time.
#[derive(Clone)]
pub struct Session {
  auth: Auth,
  tokens: Arc<Mutex<Tokens>>,
  /// Held while a refresh runs: a second caller waits, then finds the new token.
  refreshing: Arc<tokio::sync::Mutex<()>>,
  store: Store,
}

impl std::fmt::Debug for Session {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("Session(…)")
  }
}

impl Session {
  /// A session on `tokens`; `store` saves each new refresh token.
  pub fn new(auth: Auth, tokens: Tokens, store: Store) -> Self {
    Self {
      auth,
      tokens: Arc::new(Mutex::new(tokens)),
      refreshing: Arc::default(),
      store,
    }
  }

  /// A session on the tokens of a sign-in that just succeeded: their refresh token is stored
  /// first.
  pub async fn begin(auth: Auth, tokens: Tokens, store: Store) -> Self {
    save(&store, &tokens.refresh).await;
    Self::new(auth, tokens, store)
  }

  /// A session from a saved refresh token: refreshes it at once (it is spent) and stores the
  /// new one. [`Error::SessionExpired`] when Twitch no longer takes it.
  pub async fn restore(auth: Auth, refresh: &str, store: Store) -> Result<Self> {
    let tokens = auth.refresh(refresh).await?;
    save(&store, &tokens.refresh).await;
    Ok(Self::new(auth, tokens, store))
  }

  /// The `id.twitch.tv` client.
  pub fn auth(&self) -> &Auth {
    &self.auth
  }

  /// The current access token.
  pub fn access_token(&self) -> String {
    self.lock().access.clone()
  }

  fn lock(&self) -> std::sync::MutexGuard<'_, Tokens> {
    // Only whole `Tokens` values are assigned under the lock: a poisoned one still holds a pair.
    self.tokens.lock().unwrap_or_else(|p| p.into_inner())
  }

  /// A fresh access token after Helix refused `stale`. If another caller already refreshed it,
  /// that token is returned without refreshing again.
  pub async fn refresh_after(&self, stale: &str) -> Result<String> {
    let _guard = self.refreshing.lock().await;
    let current = self.lock().clone();
    if current.access != stale {
      return Ok(current.access);
    }
    let tokens = self.auth.refresh(&current.refresh).await?;
    // Saved before use: the old refresh token is spent now.
    save(&self.store, &tokens.refresh).await;
    let access = tokens.access.clone();
    *self.lock() = tokens;
    tracing::info!("Twitch access token refreshed");
    Ok(access)
  }

  /// Validates the current token, refreshing it once if Twitch refuses it (required at startup
  /// and every hour).
  pub async fn validate(&self) -> Result<Validation> {
    let access = self.access_token();
    match self.auth.validate(&access).await {
      Err(Error::SessionExpired) => {
        let access = self.refresh_after(&access).await?;
        self.auth.validate(&access).await
      }
      other => other,
    }
  }

  /// Revokes the access token (sign-out); the refresh token dies with it.
  pub async fn revoke(&self) -> Result<()> {
    self.auth.revoke(&self.access_token()).await
  }
}

async fn save(store: &Store, refresh: &str) {
  let store = store.clone();
  let refresh = refresh.to_owned();
  match tokio::task::spawn_blocking(move || store(&refresh)).await {
    Ok(Ok(())) => {}
    Ok(Err(e)) => tracing::warn!("cannot save the Twitch refresh token: {e}"),
    Err(e) => tracing::warn!("cannot save the Twitch refresh token: {e}"),
  }
}
