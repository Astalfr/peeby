//! Where PeebTube keeps its files, and the session cookies.
//!
//! The cookies are a whole browser cookie jar (dozens of values, several KB): too large for the
//! Windows Credential Manager, which peeby uses for its single token. They live in a file under
//! the user's profile instead (`%APPDATA%\peebtube\cookies.txt`), never in logs.

use std::path::PathBuf;

use innertube::Cookies;

/// Language and region of YouTube texts and recommendations.
pub const LANGUAGE: (&str, &str) = ("fr", "FR");

/// `PEEBTUBE_COOKIES`, else `%APPDATA%\peebtube\cookies.txt`.
pub fn cookies_path() -> Option<PathBuf> {
  std::env::var_os("PEEBTUBE_COOKIES")
    .map(PathBuf::from)
    .or_else(|| dirs::config_dir().map(|d| d.join("peebtube").join("cookies.txt")))
}

/// Loads the session cookies, if a file is there. A broken file is logged and ignored, so the
/// app still starts signed out.
pub fn load_cookies() -> Option<Cookies> {
  let path = cookies_path()?;
  let text = std::fs::read_to_string(&path).ok()?;
  match Cookies::from_netscape(&text) {
    Ok(cookies) => Some(cookies),
    Err(e) => {
      tracing::warn!("{}: {e}", path.display());
      None
    }
  }
}
