//! Browser cookies (Netscape `cookies.txt`) and the `SAPISIDHASH` authorization the web
//! client sends with them.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use sha1::{Digest, Sha1};

use crate::error::{Error, Result};

/// Origin the authorization hash is bound to.
pub(crate) const ORIGIN: &str = "https://www.youtube.com";

/// youtube.com cookies of a signed-in browser session. They are as sensitive as a password:
/// `Debug` prints the names only, and they never appear in logs or errors.
#[derive(Clone)]
pub struct Cookies {
  values: BTreeMap<String, String>,
}

impl std::fmt::Debug for Cookies {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_set().entries(self.values.keys()).finish()
  }
}

impl Cookies {
  /// Parses a Netscape cookie file and keeps the youtube.com cookies, whether the domain is
  /// written with or without its leading dot. `#HttpOnly_` lines are cookies, not comments.
  pub fn from_netscape(text: &str) -> Result<Self> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
      let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
      if line.starts_with('#') || line.trim().is_empty() {
        continue;
      }
      let fields: Vec<&str> = line.split('\t').collect();
      let [domain, _, _, _, _, name, value] = fields[..] else {
        continue;
      };
      let domain = domain.trim_start_matches('.');
      if domain == "youtube.com" || domain.ends_with(".youtube.com") {
        values.insert(name.to_owned(), value.trim_end().to_owned());
      }
    }
    let cookies = Self { values };
    if cookies.sapisid().is_none() {
      return Err(Error::Cookies(
        "no SAPISID or __Secure-3PAPISID youtube.com cookie: export them while signed in".into(),
      ));
    }
    Ok(cookies)
  }

  /// `Cookie` header value.
  pub(crate) fn header(&self) -> String {
    self
      .values
      .iter()
      .map(|(k, v)| format!("{k}={v}"))
      .collect::<Vec<_>>()
      .join("; ")
  }

  fn sapisid(&self) -> Option<&String> {
    self
      .values
      .get("SAPISID")
      .or_else(|| self.values.get("__Secure-3PAPISID"))
  }

  /// `Authorization` header as the web client builds it:
  /// `SAPISIDHASH <ts>_<sha1> SAPISID1PHASH <ts>_<sha1> SAPISID3PHASH <ts>_<sha1>`,
  /// with `sha1 = SHA1("<ts> <cookie> <origin>")`.
  pub(crate) fn authorization(&self) -> String {
    let ts = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map_or(0, |d| d.as_secs());
    self.authorization_at(ts)
  }

  fn authorization_at(&self, ts: u64) -> String {
    let hash = |value: &str| {
      let digest = Sha1::digest(format!("{ts} {value} {ORIGIN}").as_bytes());
      let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
      format!("{ts}_{hex}")
    };
    let mut parts = Vec::new();
    if let Some(v) = self.sapisid() {
      parts.push(format!("SAPISIDHASH {}", hash(v)));
    }
    if let Some(v) = self.values.get("__Secure-1PAPISID") {
      parts.push(format!("SAPISID1PHASH {}", hash(v)));
    }
    if let Some(v) = self.values.get("__Secure-3PAPISID") {
      parts.push(format!("SAPISID3PHASH {}", hash(v)));
    }
    parts.join(" ")
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const FILE: &str = "# Netscape HTTP Cookie File\n\
    .youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tabc\n\
    #HttpOnly_youtube.com\tFALSE\t/\tTRUE\t0\t__Secure-3PAPISID\tdef\n\
    .google.com\tTRUE\t/\tTRUE\t0\tSID\tnot-youtube\n";

  #[test]
  fn keeps_youtube_cookies_only() {
    let cookies = Cookies::from_netscape(FILE).unwrap();
    assert_eq!(cookies.header(), "SAPISID=abc; __Secure-3PAPISID=def");
    assert!(!format!("{cookies:?}").contains("abc"));
  }

  #[test]
  fn rejects_signed_out_export() {
    let err = Cookies::from_netscape(".youtube.com\tTRUE\t/\tTRUE\t0\tPREF\tx\n").unwrap_err();
    assert!(matches!(err, Error::Cookies(_)));
  }

  #[test]
  fn authorization_hash() {
    let cookies = Cookies::from_netscape(FILE).unwrap();
    // sha1("1700000000 abc https://www.youtube.com")
    let auth = cookies.authorization_at(1_700_000_000);
    let expected = Sha1::digest(b"1700000000 abc https://www.youtube.com");
    let hex: String = expected.iter().map(|b| format!("{b:02x}")).collect();
    assert!(auth.starts_with(&format!("SAPISIDHASH 1700000000_{hex} ")));
    assert!(auth.contains(" SAPISID3PHASH 1700000000_"));
    assert!(!auth.contains("SAPISID1PHASH"));
  }
}
