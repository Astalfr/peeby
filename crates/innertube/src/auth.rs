//! Browser cookies (Netscape `cookies.txt`), the `SAPISIDHASH` authorization the web client
//! sends with them, and the `Set-Cookie` updates YouTube sends back.
//!
//! YouTube rotates some session cookies (`SIDCC`, `__Secure-1PSIDTS`…) in its responses. A
//! client that keeps sending the old values gets its session invalidated after a while, so the
//! updates are applied and the file is written back, as browsers and yt-dlp do.

use std::time::{SystemTime, UNIX_EPOCH};

use sha1::{Digest, Sha1};

use crate::error::{Error, Result};

/// Origin the authorization hash is bound to.
pub(crate) const ORIGIN: &str = "https://www.youtube.com";

/// One line of a Netscape cookie file.
#[derive(Clone, PartialEq, Eq)]
struct Entry {
  domain: String,
  include_subdomains: bool,
  path: String,
  secure: bool,
  /// Unix time, 0 for a session cookie.
  expires: u64,
  name: String,
  value: String,
  http_only: bool,
}

impl Entry {
  fn is_youtube(&self) -> bool {
    let domain = self.domain.trim_start_matches('.');
    domain == "youtube.com" || domain.ends_with(".youtube.com")
  }
}

/// Cookies of a signed-in browser session. They are as sensitive as a password: `Debug`
/// prints the names only, and they never appear in logs or errors.
#[derive(Clone, PartialEq, Eq)]
pub struct Cookies {
  /// Every line of the file, youtube.com or not, so the file is written back whole.
  entries: Vec<Entry>,
}

impl std::fmt::Debug for Cookies {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_set()
      .entries(self.entries.iter().map(|e| &e.name))
      .finish()
  }
}

impl Cookies {
  /// Parses a Netscape cookie file. Only youtube.com cookies are sent (domain written with or
  /// without its leading dot); `#HttpOnly_` lines are cookies, not comments.
  pub fn from_netscape(text: &str) -> Result<Self> {
    let mut entries = Vec::new();
    for line in text.lines() {
      let (line, http_only) = match line.strip_prefix("#HttpOnly_") {
        Some(rest) => (rest, true),
        None => (line, false),
      };
      if line.starts_with('#') || line.trim().is_empty() {
        continue;
      }
      let fields: Vec<&str> = line.split('\t').collect();
      let [domain, subdomains, path, secure, expires, name, value] = fields[..] else {
        continue;
      };
      entries.push(Entry {
        domain: domain.to_owned(),
        include_subdomains: subdomains.eq_ignore_ascii_case("TRUE"),
        path: path.to_owned(),
        secure: secure.eq_ignore_ascii_case("TRUE"),
        expires: expires.trim().parse().unwrap_or(0),
        name: name.to_owned(),
        value: value.trim_end().to_owned(),
        http_only,
      });
    }
    let cookies = Self { entries };
    if cookies.sapisid().is_none() {
      return Err(Error::Cookies(
        "no SAPISID or __Secure-3PAPISID youtube.com cookie: export them while signed in".into(),
      ));
    }
    Ok(cookies)
  }

  /// The file in Netscape format, with the updates applied.
  pub fn to_netscape(&self) -> String {
    let mut text = String::from("# Netscape HTTP Cookie File\n");
    for e in &self.entries {
      let bool_text = |b: bool| if b { "TRUE" } else { "FALSE" };
      text.push_str(&format!(
        "{}{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        if e.http_only { "#HttpOnly_" } else { "" },
        e.domain,
        bool_text(e.include_subdomains),
        e.path,
        bool_text(e.secure),
        e.expires,
        e.name,
        e.value
      ));
    }
    text
  }

  fn youtube(&self) -> impl Iterator<Item = &Entry> {
    self.entries.iter().filter(|e| e.is_youtube())
  }

  fn get(&self, name: &str) -> Option<&str> {
    self
      .youtube()
      .find(|e| e.name == name)
      .map(|e| e.value.as_str())
  }

  /// `Cookie` header value for youtube.com.
  pub(crate) fn header(&self) -> String {
    self
      .youtube()
      .map(|e| format!("{}={}", e.name, e.value))
      .collect::<Vec<_>>()
      .join("; ")
  }

  fn sapisid(&self) -> Option<&str> {
    self
      .get("SAPISID")
      .or_else(|| self.get("__Secure-3PAPISID"))
  }

  /// Applies one `Set-Cookie` header received from www.youtube.com. Returns whether a value
  /// changed. Deletions (`Max-Age=0`, past `Expires`) are ignored: losing a session cookie
  /// by mistake would sign the user out, keeping a stale one at worst does the same.
  pub(crate) fn apply_set_cookie(&mut self, header: &str) -> bool {
    let mut parts = header.split(';');
    let Some((name, value)) = parts.next().and_then(|p| p.trim().split_once('=')) else {
      return false;
    };
    let (name, value) = (name.trim(), value.trim());
    let mut domain = None;
    let mut max_age = None;
    let mut deletes = value.is_empty();
    for attribute in parts {
      let (key, val) = attribute
        .trim()
        .split_once('=')
        .unwrap_or((attribute.trim(), ""));
      if key.eq_ignore_ascii_case("domain") {
        domain = Some(val.trim().trim_start_matches('.').to_ascii_lowercase());
      } else if key.eq_ignore_ascii_case("max-age") {
        max_age = val.trim().parse::<i64>().ok();
      } else if key.eq_ignore_ascii_case("expires") && val.contains("1970") {
        deletes = true;
      }
    }
    if deletes || max_age.is_some_and(|age| age <= 0) {
      return false;
    }
    // Host-only cookies of www.youtube.com, or domain cookies of youtube.com.
    let domain = domain.unwrap_or_else(|| "youtube.com".to_owned());
    if domain != "youtube.com" && !domain.ends_with(".youtube.com") {
      return false;
    }
    let expires = max_age.map(|age| now() + age as u64);
    match self
      .entries
      .iter_mut()
      .find(|e| e.is_youtube() && e.name == name)
    {
      Some(entry) if entry.value == value => false,
      Some(entry) => {
        entry.value = value.to_owned();
        if let Some(expires) = expires {
          entry.expires = expires;
        }
        true
      }
      None => {
        self.entries.push(Entry {
          domain: format!(".{domain}"),
          include_subdomains: true,
          path: "/".into(),
          secure: true,
          expires: expires.unwrap_or(0),
          name: name.to_owned(),
          value: value.to_owned(),
          http_only: false,
        });
        true
      }
    }
  }

  /// `Authorization` header as the web client builds it:
  /// `SAPISIDHASH <ts>_<sha1> SAPISID1PHASH <ts>_<sha1> SAPISID3PHASH <ts>_<sha1>`,
  /// with `sha1 = SHA1("<ts> <cookie> <origin>")`.
  pub(crate) fn authorization(&self) -> String {
    self.authorization_at(now())
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
    if let Some(v) = self.get("__Secure-1PAPISID") {
      parts.push(format!("SAPISID1PHASH {}", hash(v)));
    }
    if let Some(v) = self.get("__Secure-3PAPISID") {
      parts.push(format!("SAPISID3PHASH {}", hash(v)));
    }
    parts.join(" ")
  }
}

fn now() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
  use super::*;

  const FILE: &str = "# Netscape HTTP Cookie File\n\
    .youtube.com\tTRUE\t/\tTRUE\t1800000000\tSAPISID\tabc\n\
    #HttpOnly_youtube.com\tFALSE\t/\tTRUE\t0\t__Secure-3PAPISID\tdef\n\
    .youtube.com\tTRUE\t/\tTRUE\t1800000000\tSIDCC\told\n\
    .google.com\tTRUE\t/\tTRUE\t0\tSID\tnot-youtube\n";

  #[test]
  fn sends_youtube_cookies_only() {
    let cookies = Cookies::from_netscape(FILE).unwrap();
    assert_eq!(
      cookies.header(),
      "SAPISID=abc; __Secure-3PAPISID=def; SIDCC=old"
    );
    assert!(!format!("{cookies:?}").contains("abc"));
  }

  #[test]
  fn writes_back_every_line() {
    let cookies = Cookies::from_netscape(FILE).unwrap();
    let again = Cookies::from_netscape(&cookies.to_netscape()).unwrap();
    assert_eq!(cookies, again);
    assert!(
      cookies
        .to_netscape()
        .contains("#HttpOnly_youtube.com\tFALSE\t/\tTRUE\t0\t__Secure-3PAPISID\tdef")
    );
    assert!(
      cookies
        .to_netscape()
        .contains(".google.com\tTRUE\t/\tTRUE\t0\tSID\tnot-youtube")
    );
  }

  #[test]
  fn applies_rotations() {
    let mut cookies = Cookies::from_netscape(FILE).unwrap();
    assert!(cookies.apply_set_cookie(
      "SIDCC=new; expires=Mon, 01-Nov-2027 13:00:00 GMT; path=/; domain=.youtube.com; priority=high"
    ));
    assert!(cookies.header().contains("SIDCC=new"));
    // Same value: nothing to save.
    assert!(!cookies.apply_set_cookie("SIDCC=new; domain=.youtube.com"));
    // New cookie, host-only.
    assert!(cookies.apply_set_cookie("__Secure-1PSIDTS=ts; Max-Age=3600; Secure; HttpOnly"));
    assert!(cookies.header().contains("__Secure-1PSIDTS=ts"));
    // Deletions and other domains are ignored.
    assert!(!cookies.apply_set_cookie("SAPISID=; Max-Age=0; domain=.youtube.com"));
    assert!(!cookies.apply_set_cookie("SAPISID=x; expires=Thu, 01-Jan-1970 00:00:01 GMT"));
    assert!(!cookies.apply_set_cookie("SID=y; domain=.google.com"));
    assert!(cookies.header().contains("SAPISID=abc"));
  }

  #[test]
  fn rejects_signed_out_export() {
    let err = Cookies::from_netscape(".youtube.com\tTRUE\t/\tTRUE\t0\tPREF\tx\n").unwrap_err();
    assert!(matches!(err, Error::Cookies(_)));
  }

  #[test]
  fn authorization_hash() {
    let cookies = Cookies::from_netscape(FILE).unwrap();
    let auth = cookies.authorization_at(1_700_000_000);
    let expected = Sha1::digest(b"1700000000 abc https://www.youtube.com");
    let hex: String = expected.iter().map(|b| format!("{b:02x}")).collect();
    assert!(auth.starts_with(&format!("SAPISIDHASH 1700000000_{hex} ")));
    assert!(auth.contains(" SAPISID3PHASH 1700000000_"));
    assert!(!auth.contains("SAPISID1PHASH"));
  }
}
