//! What an update is: version numbers, which asset of a GitHub release is the installer, and
//! whether a release is worth downloading. Fetching and installing live in `super::github`.

use std::fmt;
use std::path::PathBuf;

use anyhow::{Context as _, anyhow};

/// A release version, `major.minor.patch`. No pre-release or build suffix: GitHub's latest
/// release is never a pre-release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
  pub major: u64,
  pub minor: u64,
  pub patch: u64,
}

impl Version {
  /// Parses `1.2.3` or a tag like `v1.2.3`.
  pub fn parse(text: &str) -> Option<Self> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.').map(|p| {
      // `u64::from_str` accepts a leading `+`; a version does not.
      if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
        None
      } else {
        p.parse::<u64>().ok()
      }
    });
    let version = Self {
      major: parts.next()??,
      minor: parts.next()??,
      patch: parts.next()??,
    };
    parts.next().is_none().then_some(version)
  }

  /// This build's version.
  pub fn current() -> Self {
    // Cargo guarantees a semver `CARGO_PKG_VERSION`; a pre-release one (never published) falls
    // back to 0.0.0 and sees every release.
    Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self {
      major: 0,
      minor: 0,
      patch: 0,
    })
  }
}

impl fmt::Display for Version {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
  }
}

/// A SHA-256 digest, as lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256(String);

impl Sha256 {
  /// Parses GitHub's asset `digest` (`sha256:<64 hex>`).
  pub fn from_github(digest: &str) -> Option<Self> {
    Self::from_hex(digest.strip_prefix("sha256:")?)
  }

  /// Parses 64 hex digits, in any case.
  pub fn from_hex(hex: &str) -> Option<Self> {
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
      .then(|| Self(hex.to_ascii_lowercase()))
  }

  pub fn as_hex(&self) -> &str {
    &self.0
  }
}

/// One file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
  pub name: String,
  pub url: String,
  /// GitHub's digest of the file, when it gives one.
  pub sha256: Option<Sha256>,
}

/// A published release that can be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
  pub version: Version,
  /// The installer to download.
  pub installer: Asset,
  /// The installer's digest, which the download must match.
  pub sha256: Sha256,
}

/// The installer's file name in release `version`, as CI publishes it (`.github/workflows/ci.yml`,
/// `installer/peeby.iss`).
pub fn installer_name(version: Version) -> String {
  format!("peeby-setup-{version}-x64.exe")
}

impl Release {
  /// The installable release behind a GitHub release: its tag must be a version, and it must
  /// carry that version's installer with a digest.
  pub fn from_assets(tag: &str, assets: Vec<Asset>) -> anyhow::Result<Self> {
    let version =
      Version::parse(tag).ok_or_else(|| anyhow!("release tag {tag:?} is not a version"))?;
    let name = installer_name(version);
    let installer = assets
      .into_iter()
      .find(|a| a.name == name)
      .with_context(|| format!("release {version} has no {name}"))?;
    let sha256 = installer
      .sha256
      .clone()
      .with_context(|| format!("release {version}'s installer has no SHA-256 digest"))?;
    Ok(Self {
      version,
      installer,
      sha256,
    })
  }
}

/// Where the update stands, for this session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
  /// Nothing to offer: not checked yet, or up to date.
  Idle,
  /// A newer release's installer is being downloaded.
  Downloading(Version),
  /// A newer release's installer is downloaded and checked: the title bar's button shows.
  Ready(Version, PathBuf),
  /// The installer is starting; Peeby quits once it has.
  Installing(Version, PathBuf),
}

impl Status {
  /// The version this status is about, if any.
  fn version(&self) -> Option<Version> {
    match self {
      Self::Idle => None,
      Self::Downloading(v) | Self::Ready(v, _) | Self::Installing(v, _) => Some(*v),
    }
  }

  /// Whether `latest` should be downloaded: newer than this build and than what is already on
  /// its way, and nothing is being installed.
  pub fn wants(&self, current: Version, latest: Version) -> bool {
    latest > current
      && !matches!(self, Self::Installing(..))
      && self.version().is_none_or(|v| latest > v)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn v(major: u64, minor: u64, patch: u64) -> Version {
    Version {
      major,
      minor,
      patch,
    }
  }

  const HEX: &str = "725b87edf68cf673a5bd9656f01bdf260aaf12355e5c617552a2ce6d53b75acb";

  fn asset(name: &str, digest: Option<&str>) -> Asset {
    Asset {
      name: name.to_owned(),
      url: format!("https://example.test/{name}"),
      sha256: digest.and_then(Sha256::from_hex),
    }
  }

  #[test]
  fn versions_parse_with_or_without_v() {
    assert_eq!(Version::parse("1.0.0"), Some(v(1, 0, 0)));
    assert_eq!(Version::parse("v12.3.45"), Some(v(12, 3, 45)));
    for bad in [
      "",
      "v",
      "1.2",
      "1.2.3.4",
      "1.2.x",
      "1.2.3-beta",
      "+1.2.3",
      "1..3",
      "vv1.2.3",
    ] {
      assert_eq!(Version::parse(bad), None, "{bad:?}");
    }
  }

  #[test]
  fn versions_compare_numerically() {
    assert!(v(0, 10, 0) > v(0, 9, 9));
    assert!(v(1, 0, 0) > v(0, 99, 99));
    assert!(v(1, 0, 1) > v(1, 0, 0));
    assert_eq!(v(1, 2, 3).to_string(), "1.2.3");
  }

  #[test]
  fn current_is_this_crates_version() {
    assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
  }

  #[test]
  fn digests_come_from_githubs_field() {
    let upper = HEX.to_ascii_uppercase();
    assert_eq!(
      Sha256::from_github(&format!("sha256:{upper}"))
        .map(|d| d.as_hex().to_owned())
        .as_deref(),
      Some(HEX)
    );
    assert_eq!(Sha256::from_github(HEX), None, "no algorithm prefix");
    assert_eq!(Sha256::from_github(&format!("sha512:{HEX}")), None);
    assert_eq!(Sha256::from_github("sha256:abc"), None);
  }

  #[test]
  fn the_installer_is_picked_by_its_exact_name() {
    let assets = vec![
      asset("peeby.exe", Some(HEX)),
      asset("peeby-setup-1.0.1-arm64.exe", Some(HEX)),
      asset("peeby-setup-1.0.1-x64.exe", Some(HEX)),
    ];
    let release = Release::from_assets("v1.0.1", assets).unwrap();
    assert_eq!(release.version, v(1, 0, 1));
    assert_eq!(release.installer.name, "peeby-setup-1.0.1-x64.exe");
    assert_eq!(release.sha256.as_hex(), HEX);
  }

  #[test]
  fn releases_without_a_usable_installer_are_refused() {
    assert!(Release::from_assets("nightly", vec![]).is_err());
    // An installer of another version (a mislabelled release) does not count.
    let other = vec![asset("peeby-setup-1.0.0-x64.exe", Some(HEX))];
    assert!(Release::from_assets("v1.0.1", other).is_err());
    let no_digest = vec![asset("peeby-setup-1.0.1-x64.exe", None)];
    assert!(Release::from_assets("v1.0.1", no_digest).is_err());
  }

  #[test]
  fn only_newer_releases_are_downloaded_once() {
    let current = v(1, 0, 0);
    let idle = Status::Idle;
    assert!(!idle.wants(current, v(0, 9, 9)), "older");
    assert!(!idle.wants(current, v(1, 0, 0)), "the same");
    assert!(idle.wants(current, v(1, 0, 1)));

    let downloading = Status::Downloading(v(1, 0, 1));
    assert!(
      !downloading.wants(current, v(1, 0, 1)),
      "already on its way"
    );
    let ready = Status::Ready(v(1, 0, 1), PathBuf::from("setup.exe"));
    assert!(!ready.wants(current, v(1, 0, 1)), "already downloaded");
    assert!(ready.wants(current, v(1, 1, 0)), "a newer one replaces it");
    let installing = Status::Installing(v(1, 0, 1), PathBuf::from("setup.exe"));
    assert!(
      !installing.wants(current, v(2, 0, 0)),
      "never while installing"
    );
  }
}
