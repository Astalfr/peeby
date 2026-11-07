//! The bundled yt-dlp and Deno (ADR 0005), used when the VISIONOS client cannot play a video:
//! mpv's `ytdl_hook` runs yt-dlp, isolated from the host's config and cache. yt-dlp updates
//! itself once a day, since YouTube breaks old versions.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use player::{StartOptions, Ytdl};

/// HLS only: DASH (`https`) URLs return 403 until their `available_at`, about 5 s later.
const FORMAT: &str = "bv*[protocol=m3u8_native][height<=?1080]+ba[protocol=m3u8_native]/\
                      b[protocol=m3u8_native][height<=?1080]/bv*[height<=?1080]+ba/b";

/// How often yt-dlp checks for a new release.
const UPDATE_EVERY: Duration = Duration::from_secs(24 * 3600);
/// `CREATE_NO_WINDOW`: no console window flashes when yt-dlp runs.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// The plain copy of the cookies given to yt-dlp, in its cache folder.
const COOKIE_COPY: &str = "cookies.txt";

/// Folder with `yt-dlp.exe` and `deno.exe`: `tools/` next to the executable in a
/// release, else `third_party/yt-dlp/` of the source tree (`scripts/fetch-tools.ps1`).
fn tools_dir() -> Option<PathBuf> {
  let beside_exe = std::env::current_exe()
    .ok()
    .and_then(|exe| Some(exe.parent()?.join("tools")));
  let source_tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/yt-dlp");
  [beside_exe, Some(source_tree)]
    .into_iter()
    .flatten()
    .find(|dir| dir.join("yt-dlp.exe").exists())
}

/// Player options that let yt-dlp resolve `video_id`, or `None` if yt-dlp is not bundled.
/// Blocking (writes the cookies' copy): call it off the UI thread.
pub fn ytdl_options(video_id: &str) -> Option<StartOptions> {
  let dir = tools_dir()?;
  let cache = cache_dir()?;
  let mut raw_options = vec![
    // Isolation from the host: no yt-dlp config file read, no cache in its profile.
    "ignore-config=".to_owned(),
    format!("cache-dir={}", cache.display()),
    // mpv opens external subtitles before it starts playing: none at start.
    "sub-langs=none".to_owned(),
  ];
  let deno = dir.join("deno.exe");
  if deno.exists() {
    raw_options.push(format!("js-runtimes=deno:{}", deno.display()));
  }
  // yt-dlp reads a plain jar and writes it back when it exits: give it a plain copy, never the
  // session's (encrypted) file. The copy is deleted at the next start and on sign-out (ADR 0022).
  if let Some(cookies) = peeby_core::settings::load_cookies() {
    let copy = cache.join(COOKIE_COPY);
    match std::fs::write(&copy, cookies.to_netscape()) {
      Ok(()) => raw_options.push(format!("cookies={}", copy.display())),
      Err(e) => tracing::warn!("cookies for yt-dlp: {e}"),
    }
  }
  Some(StartOptions {
    url: format!("https://www.youtube.com/watch?v={video_id}"),
    ytdl: Some(Ytdl {
      ytdl_path: Some(dir.join("yt-dlp.exe")),
      format: Some(FORMAT.to_owned()),
      raw_options,
    }),
    readahead_secs: Some(super::convert::READAHEAD_SECS),
    ..Default::default()
  })
}

/// Deletes the plain copy of the cookies given to yt-dlp, if one is left. Blocking.
pub fn remove_cookie_copy() {
  let Some(copy) = dirs::cache_dir().map(|d| d.join("peeby").join("yt-dlp").join(COOKIE_COPY))
  else {
    return;
  };
  match std::fs::remove_file(&copy) {
    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
      tracing::warn!("cookies for yt-dlp: {e}");
    }
    _ => {}
  }
}

/// yt-dlp's cache folder in the app's profile, created if needed.
fn cache_dir() -> Option<PathBuf> {
  let cache = dirs::cache_dir()?.join("peeby").join("yt-dlp");
  if let Err(e) = std::fs::create_dir_all(&cache) {
    tracing::warn!("yt-dlp cache: {e}");
  }
  Some(cache)
}

/// Runs `yt-dlp -U` if the last check is more than a day old. yt-dlp verifies the release's
/// SHA2-256SUMS itself. Blocking (network, a few seconds): run it in the background.
pub fn update_ytdlp() {
  let (Some(dir), Some(cache)) = (tools_dir(), cache_dir()) else {
    return;
  };
  let stamp = cache.join("last-update");
  let recent = std::fs::metadata(&stamp)
    .and_then(|m| m.modified())
    .ok()
    .and_then(|at| SystemTime::now().duration_since(at).ok())
    .is_some_and(|age| age < UPDATE_EVERY);
  if recent {
    return;
  }
  let output = std::process::Command::new(dir.join("yt-dlp.exe"))
    .args(["--ignore-config", "-U"])
    .creation_flags(CREATE_NO_WINDOW)
    .output();
  match output {
    Ok(output) => {
      let text = String::from_utf8_lossy(&output.stdout);
      let last = text.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("");
      tracing::info!("yt-dlp update: {last}");
      if let Err(e) = std::fs::write(&stamp, last) {
        tracing::warn!("yt-dlp update stamp: {e}");
      }
    }
    Err(e) => tracing::warn!("yt-dlp update: {e}"),
  }
}
