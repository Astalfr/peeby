//! The bundled yt-dlp and Deno (ADR 0005), used when the VISIONOS client cannot play a video:
//! mpv's `ytdl_hook` runs yt-dlp, isolated from the host's config and cache.

use std::path::{Path, PathBuf};

use player::{StartOptions, Ytdl};

/// HLS only: DASH (`https`) URLs return 403 until their `available_at`, about 5 s later.
const FORMAT: &str = "bv*[protocol=m3u8_native][height<=?1080]+ba[protocol=m3u8_native]/\
                      b[protocol=m3u8_native][height<=?1080]/bv*[height<=?1080]+ba/b";

/// Folder with `yt-dlp/yt-dlp.exe` and `deno.exe`: `tools/` next to the executable in a
/// release, else `third_party/yt-dlp/` of the source tree (`scripts/fetch-tools.ps1`).
fn tools_dir() -> Option<PathBuf> {
  let beside_exe = std::env::current_exe()
    .ok()
    .and_then(|exe| Some(exe.parent()?.join("tools")));
  let source_tree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/yt-dlp");
  [beside_exe, Some(source_tree)]
    .into_iter()
    .flatten()
    .find(|dir| dir.join("yt-dlp").join("yt-dlp.exe").exists())
}

/// Player options that let yt-dlp resolve `video_id`, or `None` if yt-dlp is not bundled.
/// Blocking (copies the cookie file): call it off the UI thread.
pub fn ytdl_options(video_id: &str) -> Option<StartOptions> {
  let dir = tools_dir()?;
  let cache = dirs::cache_dir()?.join("peebtube").join("yt-dlp");
  if let Err(e) = std::fs::create_dir_all(&cache) {
    tracing::warn!("yt-dlp cache: {e}");
  }
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
  // yt-dlp writes the cookie jar back when it exits: give it a copy, never the user's file.
  if let Some(cookies) = crate::settings::cookies_path().filter(|p| p.exists()) {
    let copy = cache.join("cookies.txt");
    match std::fs::copy(&cookies, &copy) {
      Ok(_) => raw_options.push(format!("cookies={}", copy.display())),
      Err(e) => tracing::warn!("cookies for yt-dlp: {e}"),
    }
  }
  Some(StartOptions {
    url: format!("https://www.youtube.com/watch?v={video_id}"),
    ytdl: Some(Ytdl {
      ytdl_path: Some(dir.join("yt-dlp").join("yt-dlp.exe")),
      format: Some(FORMAT.to_owned()),
      raw_options,
    }),
    // Opt-in only: the log contains stream URLs, which carry the user's IP.
    log_file: std::env::var_os("PEEBTUBE_MPV_LOG").map(Into::into),
    ..Default::default()
  })
}
