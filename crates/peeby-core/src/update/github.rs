//! Releases from the project's GitHub repository (public REST API, anonymous): the latest one,
//! its installer downloaded and checked, and the installer started.

use std::io::Write as _;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, bail};
use serde::Deserialize;
use sha2::Digest as _;

use super::release::{Asset, Release, Sha256};

/// GitHub's REST API.
const API: &str = "https://api.github.com";
/// The repository Peeby's releases are published in (public: the check is anonymous).
pub const REPOSITORY: &str = "Astalfr/peeby";
/// How long GitHub may stay silent (connecting, or between two reads) before a check or a
/// download fails. A slow download that keeps going is not cut.
const STALL_TIMEOUT: Duration = Duration::from_secs(30);
/// The uninstaller Inno Setup puts next to the executable.
const UNINSTALLER: &str = "unins000.exe";

/// The part of GitHub's release object read here.
#[derive(Deserialize)]
struct GitHubRelease {
  tag_name: String,
  #[serde(default)]
  draft: bool,
  #[serde(default)]
  prerelease: bool,
  #[serde(default)]
  assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
  name: String,
  browser_download_url: String,
  #[serde(default)]
  digest: Option<String>,
}

/// The installable release in GitHub's answer to `releases/latest`; `None` for a draft or a
/// pre-release.
fn parse_latest(body: &[u8]) -> anyhow::Result<Option<Release>> {
  let release: GitHubRelease =
    serde_json::from_slice(body).context("GitHub's answer is unreadable")?;
  if release.draft || release.prerelease {
    return Ok(None);
  }
  let assets = release
    .assets
    .into_iter()
    .map(|a| Asset {
      name: a.name,
      url: a.browser_download_url,
      sha256: a.digest.as_deref().and_then(Sha256::from_github),
    })
    .collect();
  Release::from_assets(&release.tag_name, assets).map(Some)
}

/// Whether `exe` was installed by Peeby's installer, which puts its uninstaller next to it. Only
/// such a copy updates itself (not `target\release`).
pub fn installed_by_setup(exe: &Path) -> bool {
  exe
    .parent()
    .is_some_and(|dir| dir.join(UNINSTALLER).is_file())
}

/// The HTTP client of the checks and downloads.
pub fn http() -> anyhow::Result<reqwest::Client> {
  reqwest::Client::builder()
    // GitHub refuses requests without a user agent.
    .user_agent(concat!("peeby/", env!("CARGO_PKG_VERSION")))
    .connect_timeout(STALL_TIMEOUT)
    .read_timeout(STALL_TIMEOUT)
    .build()
    .context("cannot create the update client")
}

/// The latest published release; `None` when there is none yet (or only a draft or a
/// pre-release).
pub async fn latest(http: &reqwest::Client) -> anyhow::Result<Option<Release>> {
  let response = http
    .get(format!("{API}/repos/{REPOSITORY}/releases/latest"))
    .header("accept", "application/vnd.github+json")
    .header("x-github-api-version", "2022-11-28")
    .send()
    .await
    .map_err(|e| e.without_url())?;
  // No release published yet.
  if response.status() == reqwest::StatusCode::NOT_FOUND {
    return Ok(None);
  }
  if !response.status().is_success() {
    bail!("GitHub answered {}", response.status().as_u16());
  }
  let body = response.bytes().await.map_err(|e| e.without_url())?;
  parse_latest(&body)
}

/// Downloads `release`'s installer into `dir`, checks its digest and returns its path. A failed
/// or corrupt download leaves nothing behind.
pub async fn download(
  http: &reqwest::Client,
  release: &Release,
  dir: &Path,
) -> anyhow::Result<PathBuf> {
  std::fs::create_dir_all(dir)?;
  let path = dir.join(&release.installer.name);
  let partial = path.with_extension("exe.part");
  if let Err(e) = write_installer(http, release, &partial).await {
    std::fs::remove_file(&partial).ok();
    return Err(e);
  }
  std::fs::rename(&partial, &path)?;
  Ok(path)
}

/// Streams the installer into `partial`, hashing it on the way.
async fn write_installer(
  http: &reqwest::Client,
  release: &Release,
  partial: &Path,
) -> anyhow::Result<()> {
  let mut response = http
    .get(&release.installer.url)
    .send()
    .await
    .map_err(|e| e.without_url())?;
  if !response.status().is_success() {
    bail!("GitHub answered {}", response.status().as_u16());
  }
  let mut file = std::fs::File::create(partial)?;
  let mut hasher = sha2::Sha256::new();
  while let Some(chunk) = response.chunk().await.map_err(|e| e.without_url())? {
    hasher.update(&chunk);
    file.write_all(&chunk)?;
  }
  file.sync_all()?;
  check_digest(&hasher.finalize(), &release.sha256)
}

fn check_digest(actual: &[u8], expected: &Sha256) -> anyhow::Result<()> {
  let actual: String = actual.iter().map(|b| format!("{b:02x}")).collect();
  if actual != expected.as_hex() {
    bail!(
      "the downloaded installer is corrupt (SHA-256 {actual}, expected {})",
      expected.as_hex()
    );
  }
  Ok(())
}

/// Deletes the installers left in `dir` by earlier updates. Blocking. The one that just ran may
/// still be closing: it goes at the next start.
pub fn clear_downloads(dir: &Path) {
  let Ok(entries) = std::fs::read_dir(dir) else {
    return;
  };
  for entry in entries.flatten() {
    std::fs::remove_file(entry.path()).ok();
  }
}

/// `DETACHED_PROCESS`: the installer outlives Peeby, with no console.
const DETACHED_PROCESS: u32 = 0x0000_0008;

/// Starts the installer without its wizard (a progress window only). It closes what is left of
/// Peeby, keeps the install folder, and starts Peeby again (`/relaunch=1`, see
/// `installer/peeby.iss`). Peeby quits right after.
pub fn launch_installer(path: &Path) -> anyhow::Result<()> {
  std::process::Command::new(path)
    .args([
      "/SILENT",
      "/SUPPRESSMSGBOXES",
      "/NORESTART",
      "/CLOSEAPPLICATIONS",
      "/relaunch=1",
    ])
    .creation_flags(DETACHED_PROCESS)
    .spawn()
    .map(drop)
    .context("cannot start the installer")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::update::release::Version;

  /// A real `releases/latest` answer (another project's, its assets renamed), trimmed.
  const LATEST: &str = include_str!("../../tests/fixtures/github_release_latest.json");

  #[test]
  fn the_latest_release_and_its_installer_are_read() {
    let release = parse_latest(LATEST.as_bytes()).unwrap().unwrap();
    assert_eq!(release.version, Version::parse("1.0.1").unwrap());
    assert_eq!(release.installer.name, "peeby-setup-1.0.1-x64.exe");
    assert_eq!(
      release.installer.url,
      "https://github.com/Astalfr/peeby/releases/download/v1.0.1/peeby-setup-1.0.1-x64.exe"
    );
    assert_eq!(
      release.sha256.as_hex(),
      "725b87edf68cf673a5bd9656f01bdf260aaf12355e5c617552a2ce6d53b75acb"
    );
  }

  #[test]
  fn drafts_and_pre_releases_are_ignored() {
    let mut json: serde_json::Value = serde_json::from_str(LATEST).unwrap();
    json["prerelease"] = true.into();
    let body = serde_json::to_vec(&json).unwrap();
    assert_eq!(parse_latest(&body).unwrap(), None);
    json["prerelease"] = false.into();
    json["draft"] = true.into();
    let body = serde_json::to_vec(&json).unwrap();
    assert_eq!(parse_latest(&body).unwrap(), None);
  }

  #[test]
  fn a_release_without_digest_or_an_unreadable_answer_is_an_error() {
    let mut json: serde_json::Value = serde_json::from_str(LATEST).unwrap();
    json["assets"][0]["digest"] = serde_json::Value::Null;
    assert!(parse_latest(&serde_json::to_vec(&json).unwrap()).is_err());
    assert!(parse_latest(b"<html>").is_err());
  }

  #[test]
  fn a_corrupt_download_is_refused() {
    let digest = sha2::Sha256::digest(b"an installer");
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    let expected = Sha256::from_hex(&hex).unwrap();
    assert!(check_digest(&digest, &expected).is_ok());
    let other = sha2::Sha256::digest(b"another file");
    assert!(check_digest(&other, &expected).is_err());
  }

  #[test]
  fn only_a_copy_installed_by_setup_updates_itself() {
    let dir = std::env::temp_dir().join(format!("peeby-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("peeby.exe");
    assert!(!installed_by_setup(&exe), "e.g. target\\release");
    std::fs::write(dir.join(UNINSTALLER), b"").unwrap();
    assert!(installed_by_setup(&exe));
    clear_downloads(&dir);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    std::fs::remove_dir(&dir).unwrap();
  }
}
