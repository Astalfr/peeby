//! Where Peeby keeps its files: the settings (`%APPDATA%\peeby\settings.toml`) and the
//! YouTube session cookies.
//!
//! The cookies are a whole browser cookie jar (dozens of values, several KB): too large for the
//! Windows Credential Manager, which holds Jellyfin's single token (ADR 0009). They live in a file
//! under the user's profile instead (`%APPDATA%\peeby\cookies.txt`), never in logs.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{App, Global, Task};
use innertube::Cookies;
use serde::{Deserialize, Serialize};

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
      Err(_) => Self::default(),
    }
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

  fn save(&self) -> std::io::Result<()> {
    let path = settings_path().ok_or_else(|| std::io::Error::other("no profile folder"))?;
    let text = toml::to_string(self).map_err(std::io::Error::other)?;
    write_atomically(&path, &text)
  }
}

fn settings_path() -> Option<PathBuf> {
  dirs::config_dir().map(|d| d.join("peeby").join("settings.toml"))
}

/// Writes next to `path` then renames, so a crash never leaves a truncated file.
fn write_atomically(path: &std::path::Path, text: &str) -> std::io::Result<()> {
  if let Some(dir) = path.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let temporary = path.with_extension("tmp");
  std::fs::write(&temporary, text)?;
  std::fs::rename(&temporary, path)
}

/// `PEEBY_COOKIES`, else `%APPDATA%\peeby\cookies.txt`.
pub fn cookies_path() -> Option<PathBuf> {
  std::env::var_os("PEEBY_COOKIES")
    .map(PathBuf::from)
    .or_else(|| dirs::config_dir().map(|d| d.join("peeby").join("cookies.txt")))
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

/// Saves imported cookies as the session file. Blocking.
pub fn save_cookies(cookies: &Cookies) -> std::io::Result<()> {
  let path = cookies_path().ok_or_else(|| std::io::Error::other("no profile folder"))?;
  write_atomically(&path, &cookies.to_netscape())
}

#[cfg(test)]
mod tests {
  use super::Settings;

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
    assert_eq!(settings.jellyfin.server_url, None);
    let again: Settings = toml::from_str(&toml::to_string(&settings).unwrap()).unwrap();
    assert_eq!(settings, again);
  }
}
