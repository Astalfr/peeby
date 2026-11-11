//! The Jellyfin session: who this app is to the server, the access token in the Windows
//! Credential Manager (ADR 0009), and the image route that sends the token with image requests.
//!
//! The token never goes to the settings file, a URL or a log. Credential Manager calls block (an
//! OS call): the token is read before the UI starts, and written or deleted in the background.

use std::sync::Arc;

use gpui::App;
use jellyfin_api::{Client, ClientInfo, Session, Url, UserId};
use ui::images::{self, Download, Fetch};

use peeby_core::settings::{self, Settings};

const KEYRING_SERVICE: &str = peeby_core::settings::PROFILE;

/// What the app tells the server about itself.
pub fn client_info(device_id: &str) -> ClientInfo {
  ClientInfo {
    client: "Peeby".into(),
    device: device_name(),
    device_id: device_id.to_owned(),
    version: env!("CARGO_PKG_VERSION").into(),
  }
}

/// This PC's name, shown in the server's list of devices.
pub fn device_name() -> String {
  std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC Windows".into())
}

/// A new installation id, made once and kept in the settings.
pub fn new_device_id() -> String {
  uuid::Uuid::new_v4().simple().to_string()
}

/// The saved session's client, if a server, a user and their token are saved. Blocking (it reads
/// the Credential Manager): call it before the UI starts. A token the server no longer accepts
/// shows at the first request (HTTP 401), which sends the space back to the login page.
pub fn restore(settings: &settings::Jellyfin) -> Option<Client> {
  let server = settings.server_url.as_deref()?;
  let user = UserId(settings.user_id.clone()?);
  let token = load_token(server, &user)?;
  let mut client = Client::new(server, client_info(&settings.device_id)).ok()?;
  client.set_session(Some(Session {
    user_id: user,
    access_token: token,
  }));
  Some(client)
}

/// Keeps a new session: the server and user in the settings, the token in the Credential Manager
/// (in the background).
pub fn save(client: &Client, cx: &mut App) {
  let Some(session) = client.session().cloned() else {
    return;
  };
  let server = client.base_url().to_string();
  Settings::update(cx, |s| {
    s.jellyfin.server_url = Some(server.clone());
    s.jellyfin.user_id = Some(session.user_id.to_string());
  });
  cx.background_executor()
    .spawn(async move {
      if let Err(e) = store_token(&server, &session.user_id, &session.access_token) {
        tracing::warn!("cannot save the Jellyfin token, sign in again next time: {e}");
      }
    })
    .detach();
}

/// Forgets the session: the user and the token go, the server address stays for the next sign-in.
pub fn forget(cx: &mut App) {
  let jellyfin = &Settings::get(cx).jellyfin;
  if let (Some(server), Some(user)) = (jellyfin.server_url.clone(), jellyfin.user_id.clone()) {
    cx.background_executor()
      .spawn(async move { delete_token(&server, &UserId(user)) })
      .detach();
  }
  Settings::update(cx, |s| s.jellyfin.user_id = None);
}

fn token_entry(server_url: &str, user: &UserId) -> keyring::Result<keyring::Entry> {
  keyring::Entry::new(KEYRING_SERVICE, &format!("{server_url}|{user}"))
}

fn load_token(server_url: &str, user: &UserId) -> Option<String> {
  match token_entry(server_url, user).and_then(|entry| entry.get_password()) {
    Ok(token) => Some(token),
    Err(keyring::Error::NoEntry) => None,
    Err(e) => {
      tracing::warn!("cannot read the saved Jellyfin token: {e}");
      None
    }
  }
}

fn store_token(server_url: &str, user: &UserId, token: &str) -> keyring::Result<()> {
  token_entry(server_url, user)?.set_password(token)
}

fn delete_token(server_url: &str, user: &UserId) {
  match token_entry(server_url, user).and_then(|entry| entry.delete_credential()) {
    Ok(()) | Err(keyring::Error::NoEntry) => {}
    Err(e) => tracing::warn!("cannot delete the saved Jellyfin token: {e}"),
  }
}

/// The WireGuard configuration's entry (ADR 0037): the whole wg-quick file, keys included.
fn vpn_entry() -> keyring::Result<keyring::Entry> {
  keyring::Entry::new(KEYRING_SERVICE, "jellyfin|wireguard")
}

/// The saved WireGuard configuration, if one was imported. Blocking (the Credential Manager):
/// call it before the UI starts. Its text holds keys: never log it.
pub fn load_vpn() -> Option<String> {
  match vpn_entry().and_then(|entry| entry.get_password()) {
    Ok(text) => Some(text),
    Err(keyring::Error::NoEntry) => None,
    Err(e) => {
      tracing::warn!("cannot read the saved WireGuard configuration: {e}");
      None
    }
  }
}

/// Saves the WireGuard configuration (`Some`), or forgets it, in the background.
pub fn store_vpn(text: Option<String>, cx: &mut App) {
  cx.background_executor()
    .spawn(async move {
      let result = vpn_entry().and_then(|entry| match &text {
        Some(text) => entry.set_password(text),
        None => match entry.delete_credential() {
          Err(keyring::Error::NoEntry) => Ok(()),
          other => other,
        },
      });
      if let Err(e) = result {
        tracing::warn!("cannot save the WireGuard configuration: {e}");
      }
    })
    .detach();
}

/// Downloads the server's images (posters, trickplay tiles) with the session's `Authorization`
/// header, never with a token in the URL.
struct ImageFetch(Client);

impl Fetch for ImageFetch {
  fn fetch(&self, url: &str) -> Download {
    let client = self.0.clone();
    let url = Url::parse(url);
    Box::pin(async move {
      let url = url.map_err(|e| e.to_string())?;
      client.fetch_bytes(&url).await.map_err(|e| e.to_string())
    })
  }
}

/// Routes the images of `client`'s server through it, or back to the default fetcher (`None`
/// after signing out of `server`).
pub fn route_images(server: &str, client: Option<&Client>, cx: &mut App) {
  let fetcher = client.map(|c| Arc::new(ImageFetch(c.clone())) as Arc<dyn Fetch>);
  images::route(server, fetcher, cx);
}
