//! The Twitch space's secrets in the Windows Credential Manager (ADR 0010, as Jellyfin's token):
//! the Helix refresh token (single use: each refresh writes the new one at once) and the web
//! `auth-token` the user pasted (a full-access secret). Neither goes to the settings file, a URL or
//! a log. Credential Manager calls block: they are read before the UI starts and written or deleted
//! off the UI thread.

use std::sync::Arc;

use twitch::auth::Store;

const KEYRING_SERVICE: &str = crate::settings::PROFILE;
const REFRESH: &str = "twitch|refresh";
const AUTH_TOKEN: &str = "twitch|auth-token";

/// What the Credential Manager holds for Twitch.
#[derive(Default)]
pub struct Secrets {
  /// The Helix refresh token of the last session.
  pub refresh: Option<String>,
  /// The web `auth-token`.
  pub auth_token: Option<String>,
}

/// Reads both secrets. Blocking: call it before the UI starts.
pub fn restore() -> Secrets {
  Secrets {
    refresh: read(REFRESH),
    auth_token: read(AUTH_TOKEN),
  }
}

/// The callback the Helix session saves each new refresh token with (on a blocking thread).
pub fn store() -> Store {
  Arc::new(|refresh: &str| write(REFRESH, refresh))
}

/// Saves the web `auth-token`. Blocking.
pub fn store_auth_token(token: &str) -> Result<(), String> {
  write(AUTH_TOKEN, token)
}

/// Forgets the Helix session. Blocking.
pub fn forget_refresh() {
  delete(REFRESH);
}

/// Forgets the web `auth-token`. Blocking.
pub fn forget_auth_token() {
  delete(AUTH_TOKEN);
}

fn entry(name: &str) -> keyring::Result<keyring::Entry> {
  keyring::Entry::new(KEYRING_SERVICE, name)
}

fn read(name: &str) -> Option<String> {
  match entry(name).and_then(|e| e.get_password()) {
    Ok(secret) => Some(secret),
    Err(keyring::Error::NoEntry) => None,
    Err(e) => {
      tracing::warn!("cannot read the saved Twitch secret ({name}): {e}");
      None
    }
  }
}

fn write(name: &str, secret: &str) -> Result<(), String> {
  entry(name)
    .and_then(|e| e.set_password(secret))
    .map_err(|e| e.to_string())
}

fn delete(name: &str) {
  match entry(name).and_then(|e| e.delete_credential()) {
    Ok(()) | Err(keyring::Error::NoEntry) => {}
    Err(e) => tracing::warn!("cannot delete the saved Twitch secret ({name}): {e}"),
  }
}
