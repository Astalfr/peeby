//! Where Peeby keeps its files: the settings (`%APPDATA%\peeby\settings.toml`) and the
//! YouTube session cookies.
//!
//! The cookies are a whole browser cookie jar (dozens of values, several KB): too large for the
//! Windows Credential Manager, which holds Jellyfin's single token (ADR 0009). They live in a file
//! under the user's profile instead (`%APPDATA%\peeby\cookies.dat`, encrypted with DPAPI,
//! ADR 0022), never in logs.
//!
//! A debug build has a profile of its own ([`PROFILE`]), so that it runs next to the release
//! one without sharing its cookie jar or its single-use Twitch refresh token.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, Global, Task};
use innertube::Cookies;
use serde::{Deserialize, Serialize};

/// This build's profile: the folder of its settings and cookies, its Credential Manager service
/// and its single instance. `peeby-dev` for a debug build, signed in on its own; the caches
/// (`%LOCALAPPDATA%\peeby`) stay shared.
pub const PROFILE: &str = if cfg!(debug_assertions) {
  "peeby-dev"
} else {
  "peeby"
};

/// The app's name where Windows shows it (window, notification area, toasts, `Run` key).
pub const APP_NAME: &str = if cfg!(debug_assertions) {
  "Peeby (dev)"
} else {
  "Peeby"
};

/// Languages offered for YouTube: (language, region, label).
pub const LANGUAGES: &[(&str, &str, &str)] = &[
  ("fr", "FR", "Français (France)"),
  ("fr", "BE", "Français (Belgique)"),
  ("fr", "CA", "Français (Canada)"),
  ("fr", "CH", "Français (Suisse)"),
  ("en", "US", "English (United States)"),
  ("en", "GB", "English (United Kingdom)"),
  ("de", "DE", "Deutsch (Deutschland)"),
  ("es", "ES", "Español (España)"),
  ("it", "IT", "Italiano (Italia)"),
  ("pt", "BR", "Português (Brasil)"),
  ("ja", "JP", "日本語 (日本)"),
];

/// Default qualities offered, as video heights.
pub const QUALITIES: &[u32] = &[2160, 1440, 1080, 720, 480, 360];

/// How the subscriptions page shows its videos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeedLayout {
  /// Cards in a wrapping grid.
  #[default]
  Grid,
  /// One row per video, thumbnail on the left.
  List,
}

/// The YouTube page shown when Peeby starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StartPage {
  /// The recommendations.
  #[default]
  Home,
  /// The latest videos of the subscribed channels.
  Subscriptions,
}

/// The user's settings, a GPUI global. Unknown or missing keys take their default, so an old
/// or hand-edited file still loads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
  /// Language of Peeby's interface (`ui::i18n::Lang::code`), e.g. `en`.
  pub interface_language: String,
  /// Language of YouTube's texts (`hl`), e.g. `fr`.
  pub language: String,
  /// Region of the recommendations (`gl`), e.g. `FR`.
  pub region: String,
  /// Highest quality picked when a video starts, as a height in pixels.
  pub max_height: u32,
  /// YouTube's titles in the videos' own language, not translated into `language`.
  pub original_titles: bool,
  /// Player volume, 0–100, kept when `player.remember_volume` is on.
  pub volume: f64,
  /// Player muted.
  pub muted: bool,
  /// The player, in every space.
  pub player: Player,
  /// Layout of the subscriptions page.
  pub subscriptions_layout: FeedLayout,
  /// YouTube page shown at startup and after a session change.
  pub start_page: StartPage,
  /// Playlists shown and played last video first.
  pub reversed_playlists: Vec<String>,
  /// Watch page in theater mode: the player across the whole width, above the details.
  pub theater: bool,
  /// Sidebar reduced to its icons.
  pub sidebar_collapsed: bool,
  /// The Jellyfin space.
  pub jellyfin: Jellyfin,
  /// The Twitch space.
  pub twitch: Twitch,
  /// Where each kind of notification shows.
  pub notifications: Notifications,
  /// Closing the window keeps Peeby running in the notification area; its icon's menu quits.
  pub close_to_tray: bool,
  /// Peeby starts with Windows, in the notification area (the `Run` key holds it, ADR 0020).
  pub start_with_windows: bool,
  /// The spaces (`SpaceInfo::id`) that open a tab when Peeby starts. With none of them
  /// available, the first space opens one.
  pub startup_tabs: Vec<String>,
}

/// `[notifications]`: where each kind shows (ADR 0020).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notifications {
  /// A followed Twitch channel went live.
  pub twitch_live: Delivery,
  /// A followed Twitch live changed category.
  pub twitch_category: Delivery,
  /// A YouTube channel the bell follows uploaded a video (a Short included).
  pub youtube_video: Delivery,
  /// A YouTube channel the bell follows went live.
  pub youtube_live: Delivery,
}

/// Where a kind of notification shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Delivery {
  /// In Peeby's bell.
  pub in_app: bool,
  /// As a Windows notification.
  pub windows: bool,
}

impl Default for Delivery {
  fn default() -> Self {
    Self {
      in_app: true,
      windows: true,
    }
  }
}

/// `[twitch]`: the Twitch space. Its tokens are not here: the Helix refresh token and the web
/// `auth-token` live in the Windows Credential Manager (`spaces::twitch::session`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Twitch {
  /// Login of the signed-in user (shown before the first validation answers); `None` signed out.
  pub login: Option<String>,
  /// Overrides Peeby's Client-ID on dev.twitch.tv (Helix and sign-in).
  pub client_id: Option<String>,
  /// Overrides the web Client-ID used for GQL, if Twitch changes it and the page refresh fails.
  pub web_client_id: Option<String>,
  /// Highest quality picked when a stream starts, as a height in pixels.
  pub max_height: Option<u32>,
  /// Lives start closer to the broadcast (Twitch's prefetch segments); on when unset.
  pub low_latency: Option<bool>,
  /// With `low_latency`, lives keep close to the broadcast: the player's catch-up
  /// (`play::CATCH_UP_BUFFER`) plays a little faster or slower to hold a small buffer; on when
  /// unset.
  pub catch_up: Option<bool>,
  /// A live's bonus chest is claimed on its own, through twitch.tv's chat in a hidden window
  /// (ADR 0012); on when unset.
  pub auto_claim_bonus: Option<bool>,
  /// Codecs this PC was found to decode on the CPU only (`h265`, `av1`): no longer asked for.
  pub software_codecs: Vec<String>,
}

/// `[player]`: how the player behaves, in every space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Player {
  /// Play the next item (episode, video of a playlist) after a countdown, near the end.
  pub autoplay_next: bool,
  /// Countdown length, seconds.
  pub autoplay_countdown: u32,
  /// Keep the volume and mute state between videos and runs.
  pub remember_volume: bool,
  /// ← / → step, seconds.
  pub seek_step: u32,
  /// Shift + ← / → step, seconds.
  pub seek_step_large: u32,
  /// GPU decoding and rendering; off forces software (troubleshooting).
  pub hardware_acceleration: bool,
  /// Audio output (`player::AudioDevice::name`), picked from the top bar; `None` follows
  /// Windows' default.
  pub audio_device: Option<String>,
  /// Write each mpv's log to [`logs_dir`] (troubleshooting). Off by default: the log holds
  /// stream URLs, which carry the user's IP.
  pub mpv_log: bool,
}

impl Default for Player {
  fn default() -> Self {
    Self {
      autoplay_next: true,
      autoplay_countdown: 10,
      remember_volume: true,
      seek_step: 10,
      seek_step_large: 60,
      hardware_acceleration: true,
      audio_device: None,
      mpv_log: false,
    }
  }
}

impl Player {
  /// The player's starting preferences: these settings, and the volume of `settings` when it
  /// is remembered.
  pub fn prefs(settings: &Settings) -> ui::player::Prefs {
    let player = &settings.player;
    let remember = player.remember_volume;
    ui::player::Prefs {
      volume: if remember { settings.volume } else { 100. },
      muted: remember && settings.muted,
      subtitle_language: None,
      seek_step: player.seek_step,
      seek_step_large: player.seek_step_large,
      autoplay_next: player.autoplay_next,
      autoplay_countdown: player.autoplay_countdown,
    }
  }

  /// Keeps the player's volume for the next videos, when the setting asks for it.
  pub fn save_sound(volume: f64, muted: bool, cx: &mut App) {
    if Settings::get(cx).player.remember_volume {
      Settings::update(cx, |s| {
        s.volume = volume;
        s.muted = muted;
      });
    }
  }

  /// Applies the hardware acceleration setting (and `PEEBY_SOFTWARE_VIDEO=1`) to mpv's options.
  pub fn apply(&self, options: &mut player::StartOptions) {
    let software = !self.hardware_acceleration;
    options.software_decoding = software;
    options.force_software =
      software || std::env::var("PEEBY_SOFTWARE_VIDEO").as_deref() == Ok("1");
    // `PEEBY_MPV_LOG` names the file; the setting, one per video in `logs_dir`.
    options.log_file = std::env::var_os("PEEBY_MPV_LOG")
      .map(Into::into)
      .or_else(|| self.mpv_log.then(new_mpv_log).flatten());
  }
}

/// How long the logs (Peeby's and mpv's) are kept.
const LOGS_KEPT: Duration = Duration::from_secs(7 * 24 * 3600);

/// Folder of the logs, Peeby's and mpv's: `%LOCALAPPDATA%\peeby\logs`.
pub fn logs_dir() -> Option<PathBuf> {
  Some(dirs::cache_dir()?.join("peeby").join("logs"))
}

/// A new log file for one mpv, named after the time it starts (mpv creates it, and the
/// player its folder).
fn new_mpv_log() -> Option<PathBuf> {
  let millis = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .ok()?
    .as_millis();
  Some(logs_dir()?.join(format!("mpv-{millis}.log")))
}

/// Deletes the logs older than [`LOGS_KEPT`]. Blocking: off the UI thread.
pub fn prune_logs() {
  let Some(entries) = logs_dir().and_then(|dir| std::fs::read_dir(dir).ok()) else {
    return;
  };
  for entry in entries.flatten() {
    let old = entry
      .metadata()
      .and_then(|m| m.modified())
      .ok()
      .and_then(|at| at.elapsed().ok())
      .is_some_and(|age| age > LOGS_KEPT);
    if old && let Err(e) = std::fs::remove_file(entry.path()) {
      tracing::warn!("old log: {e}");
    }
  }
}

/// `[jellyfin]`: the Jellyfin space's server, user and preferences. The access token is not here:
/// it lives in the Windows Credential Manager (`spaces::jellyfin::session`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Jellyfin {
  /// Server address of the last session, kept after signing out.
  pub server_url: Option<String>,
  /// User of the current session; `None` when signed out.
  pub user_id: Option<String>,
  /// Stable id of this installation, sent to the server with every request.
  pub device_id: String,
  /// Playback preferences.
  pub playback: JellyfinPlayback,
  /// Home page preferences.
  pub home: JellyfinHome,
}

/// `[jellyfin.playback]`: Jellyfin's own playback preferences (the shared ones are `[player]`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JellyfinPlayback {
  /// Enter fullscreen when playback starts.
  pub start_fullscreen: bool,
  /// Preferred audio language (ISO 639-2, e.g. `jpn`); `None`: the server's choice.
  pub audio_language: Option<String>,
  /// Preferred subtitle language (ISO 639-2) or `off`; `None`: the server's choice.
  pub subtitle_language: Option<String>,
  /// Size factor of text subtitles (SRT, VTT). ASS subtitles are never restyled.
  pub subtitle_scale: f64,
  /// Read-ahead buffer of the player, MiB (files from the server, not YouTube's HLS).
  pub network_buffer_mb: u32,
}

impl Default for JellyfinPlayback {
  fn default() -> Self {
    Self {
      start_fullscreen: false,
      audio_language: None,
      subtitle_language: None,
      subtitle_scale: 1.0,
      network_buffer_mb: 300,
    }
  }
}

/// `[jellyfin.home]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct JellyfinHome {
  /// "À suivre" leaves out series not watched for this many days; `None`: no limit.
  pub next_up_days: Option<u32>,
  /// "À suivre" also suggests series watched to the end.
  pub next_up_rewatching: bool,
  /// The banner at the top.
  pub show_hero: bool,
  /// The "Reprendre" row.
  pub show_continue_watching: bool,
  /// The "À suivre" row.
  pub show_next_up: bool,
  /// Libraries whose "Derniers ajouts" row is hidden.
  pub hidden_latest: Vec<String>,
  /// Blur the stills and hide the synopses of unwatched episodes.
  pub hide_spoilers: bool,
}

impl Default for JellyfinHome {
  fn default() -> Self {
    Self {
      next_up_days: Some(365),
      next_up_rewatching: false,
      show_hero: true,
      show_continue_watching: true,
      show_next_up: true,
      hidden_latest: Vec::new(),
      hide_spoilers: false,
    }
  }
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      interface_language: "en".into(),
      language: "fr".into(),
      region: "FR".into(),
      max_height: 1080,
      original_titles: true,
      volume: 100.0,
      muted: false,
      subscriptions_layout: FeedLayout::Grid,
      start_page: StartPage::Home,
      reversed_playlists: Vec::new(),
      theater: false,
      sidebar_collapsed: false,
      player: Player::default(),
      jellyfin: Jellyfin::default(),
      twitch: Twitch::default(),
      notifications: Notifications::default(),
      close_to_tray: true,
      start_with_windows: false,
      startup_tabs: vec!["youtube".into()],
    }
  }
}

impl Global for Settings {}

/// Delay before the settings file is written.
const SAVE_DELAY: Duration = Duration::from_millis(500);

/// The pending write; replacing it cancels the previous one.
// The task is only held, never read: dropping it is what cancels the write.
struct PendingSave(#[allow(dead_code)] Task<()>);

impl Global for PendingSave {}

impl Settings {
  /// Reads the settings file, or the defaults if there is none. A broken file is logged and
  /// replaced by the defaults. Blocking (small file): call it before the UI starts.
  pub fn load() -> Self {
    let Some(path) = settings_path() else {
      return Self::default();
    };
    match std::fs::read_to_string(&path) {
      Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
        tracing::warn!("{}: {e}", path.display());
        Self::default()
      }),
      // First start: the interface follows Windows' display language.
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self {
        interface_language: os_lang(crate::desktop::sys::ui_primary_language())
          .code()
          .into(),
        ..Self::default()
      },
      Err(_) => Self::default(),
    }
  }

  /// Whether space `id` opens a tab when Peeby starts.
  pub fn opens_at_startup(&self, id: &str) -> bool {
    self.startup_tabs.iter().any(|s| s == id)
  }

  /// The interface's language.
  pub fn interface_lang(&self) -> ui::i18n::Lang {
    ui::i18n::Lang::from_code(&self.interface_language)
  }

  /// The current settings.
  pub fn get(cx: &App) -> &Self {
    cx.global::<Self>()
  }

  /// Changes the settings and writes the file in the background, once changes pause: a
  /// volume drag makes dozens of changes, and each new one cancels the pending write.
  pub fn update(cx: &mut App, change: impl FnOnce(&mut Self)) {
    let mut settings = cx.global::<Self>().clone();
    change(&mut settings);
    if &settings == cx.global::<Self>() {
      return;
    }
    cx.set_global(settings.clone());
    let write = cx.spawn(async move |cx| {
      cx.background_executor().timer(SAVE_DELAY).await;
      let saved = cx
        .background_executor()
        .spawn(async move { settings.save() })
        .await;
      if let Err(e) = saved {
        tracing::warn!("cannot save the settings: {e}");
      }
    });
    cx.set_global(PendingSave(write));
  }

  /// Writes the settings now, bypassing the pending write: Peeby is about to quit (an update).
  /// Blocking (small file): call it off the UI thread.
  pub fn save_now(&self) -> std::io::Result<()> {
    self.save()
  }

  fn save(&self) -> std::io::Result<()> {
    let path = settings_path().ok_or_else(|| std::io::Error::other("no profile folder"))?;
    let text = toml::to_string(self).map_err(std::io::Error::other)?;
    write_atomically(&path, &text)
  }
}

/// The interface's language for Windows' primary display language (a `PRIMARYLANGID`):
/// French for `LANG_FRENCH`, English otherwise.
fn os_lang(primary: u16) -> ui::i18n::Lang {
  const LANG_FRENCH: u16 = 0x0c;
  if primary == LANG_FRENCH {
    ui::i18n::Lang::Fr
  } else {
    ui::i18n::Lang::En
  }
}

fn settings_path() -> Option<PathBuf> {
  dirs::config_dir().map(|d| d.join(PROFILE).join("settings.toml"))
}

/// Writes next to `path` then renames, so a crash never leaves a truncated file.
fn write_atomically(path: &std::path::Path, text: impl AsRef<[u8]>) -> std::io::Result<()> {
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let temporary = path.with_extension("tmp");
  std::fs::write(&temporary, text)?;
  std::fs::rename(&temporary, path)
}

/// DPAPI's entropy for the cookie jar: another program that decrypts this user's DPAPI data
/// does not get the jar without it (ADR 0022).
const JAR_ENTROPY: &[u8] = b"peeby youtube cookie jar";

/// `PEEBY_COOKIES`, else `%APPDATA%\peeby\cookies.dat` (`peeby-dev` for a debug build): the
/// cookie jar encrypted with DPAPI for the current Windows user (ADR 0022).
pub fn cookies_path() -> Option<PathBuf> {
  std::env::var_os("PEEBY_COOKIES")
    .map(PathBuf::from)
    .or_else(|| dirs::config_dir().map(|d| d.join(PROFILE).join("cookies.dat")))
}

/// The plain `cookies.txt` of before ADR 0022, encrypted into [`cookies_path`] then deleted.
fn plain_cookies_path() -> Option<PathBuf> {
  if std::env::var_os("PEEBY_COOKIES").is_some() {
    return None;
  }
  dirs::config_dir().map(|d| d.join(PROFILE).join("cookies.txt"))
}

/// Loads the session cookies, if a jar is there. A broken one is logged and ignored, so the
/// app still starts signed out. A plain jar (the former `cookies.txt`, or a plain file given
/// with `PEEBY_COOKIES`) is encrypted on the way. Blocking.
pub fn load_cookies() -> Option<Cookies> {
  if let Some(plain) = plain_cookies_path().filter(|p| p.exists()) {
    migrate_plain_cookies(&plain);
  }
  let path = cookies_path()?;
  let bytes = std::fs::read(&path).ok()?;
  let (text, plain) = match crate::desktop::sys::unprotect(&bytes, JAR_ENTROPY) {
    Ok(text) => (text, false),
    Err(_) => (bytes, true),
  };
  let cookies = String::from_utf8(text)
    .map_err(|_| innertube::Error::Cookies("neither plain text nor encrypted for this user".into()))
    .and_then(|text| Cookies::from_netscape(&text));
  match cookies {
    Ok(cookies) => {
      if plain {
        match save_cookies(&cookies) {
          Ok(()) => tracing::info!("{}: plain cookie jar encrypted", path.display()),
          Err(e) => tracing::warn!("{}: cannot encrypt the cookie jar: {e}", path.display()),
        }
      }
      Some(cookies)
    }
    Err(e) => {
      tracing::warn!("{}: {e}", path.display());
      None
    }
  }
}

/// Encrypts the former plain `cookies.txt` into [`cookies_path`], unless a jar is already
/// there, then deletes it: from then on the cookies exist in clear only in memory.
fn migrate_plain_cookies(plain: &std::path::Path) {
  let encrypted = cookies_path().is_some_and(|p| p.exists());
  if !encrypted {
    let cookies = std::fs::read_to_string(plain)
      .map_err(|e| e.to_string())
      .and_then(|text| Cookies::from_netscape(&text).map_err(|e| e.to_string()))
      .and_then(|cookies| save_cookies(&cookies).map_err(|e| e.to_string()));
    if let Err(e) = cookies {
      // Kept: a jar that cannot be moved stays where the user can still find it.
      return tracing::warn!("{}: cannot encrypt the cookie jar: {e}", plain.display());
    }
    tracing::info!("{}: cookie jar encrypted", plain.display());
  }
  if let Err(e) = std::fs::remove_file(plain) {
    tracing::warn!(
      "{}: cannot delete the plain cookie jar: {e}",
      plain.display()
    );
  }
}

/// Saves the session's cookies, encrypted. Blocking.
pub fn save_cookies(cookies: &Cookies) -> std::io::Result<()> {
  let path = cookies_path().ok_or_else(|| std::io::Error::other("no profile folder"))?;
  let sealed = crate::desktop::sys::protect(cookies.to_netscape().as_bytes(), JAR_ENTROPY)
    .map_err(std::io::Error::other)?;
  write_atomically(&path, sealed)
}

/// Where the client saves the rotated cookies: [`save_cookies`].
pub fn cookie_saver() -> innertube::SaveCookies {
  std::sync::Arc::new(save_cookies)
}

/// Deletes the session's cookies (signing out), and the former plain jar if it is still there.
/// Blocking.
pub fn delete_cookies() -> std::io::Result<()> {
  for path in [cookies_path(), plain_cookies_path()].into_iter().flatten() {
    match std::fs::remove_file(&path) {
      Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
      _ => {}
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::Settings;

  #[test]
  fn first_start_follows_windows_language() {
    assert_eq!(super::os_lang(0x0c), ui::i18n::Lang::Fr);
    assert_eq!(super::os_lang(0x09), ui::i18n::Lang::En);
    assert_eq!(super::os_lang(0x07), ui::i18n::Lang::En);
  }

  #[test]
  fn partial_file_takes_defaults() {
    let settings: Settings = toml::from_str(
      "max_height = 720\nunknown = 1\n[jellyfin]\ndevice_id = \"abc\"\n[jellyfin.home]\nshow_hero = false\n",
    )
    .unwrap();
    assert_eq!(settings.max_height, 720);
    assert_eq!(settings.language, "fr");
    assert_eq!(settings.interface_language, "en");
    assert_eq!(settings.jellyfin.device_id, "abc");
    assert!(!settings.jellyfin.home.show_hero);
    assert!(settings.jellyfin.home.show_next_up);
    assert_eq!(settings.player.seek_step, 10);
    assert!(settings.player.hardware_acceleration);
    assert!(!settings.player.mpv_log);
    assert_eq!(settings.jellyfin.server_url, None);
    assert!(settings.close_to_tray && !settings.start_with_windows);
    assert!(settings.notifications.twitch_category.windows);
    assert_eq!(settings.startup_tabs, vec!["youtube"]);
    assert!(settings.opens_at_startup("youtube") && !settings.opens_at_startup("twitch"));
    let again: Settings = toml::from_str(&toml::to_string(&settings).unwrap()).unwrap();
    assert_eq!(settings, again);
  }
}
