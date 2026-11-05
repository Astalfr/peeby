//! What the player plays, whatever the service: the mpv options, the qualities to switch
//! between, subtitles to fetch on demand and preview tiles for the seek bar. Spaces build a
//! [`Source`] from their own stream data (docs/refactor-plan.md, "Target architecture").

use std::rc::Rc;

use gpui::SharedString;
use player::StartOptions;

/// A video to play.
#[derive(Clone, Default)]
pub struct Source {
  /// mpv options of the first quality played, with the HTTP headers the server needs.
  pub options: StartOptions,
  /// Qualities offered in the menu, best first. Empty: no quality menu.
  pub qualities: Vec<Quality>,
  /// Index in `qualities` of the one `options` plays.
  pub quality: Option<usize>,
  /// Subtitle tracks given to mpv the first time they are picked (`sub-add`).
  pub subtitles: Vec<Subtitle>,
  /// Thumbnails shown over the seek bar.
  pub previews: Option<Rc<dyn Previews>>,
  /// Names of the file's audio tracks in mpv's order (id − 1), when the server knows better
  /// ones than the file's tags. Missing or empty names fall back to the tags.
  pub audio_names: Vec<String>,
  /// Same for the subtitle tracks: embedded ones, then the external ones in `options`.
  pub subtitle_names: Vec<String>,
  /// A live stream: no seek bar, no seeking, a live badge instead of the times, and a quality
  /// change joins the live edge. mpv still reports a duration (the playlist's window): it is
  /// ignored. `options.start_seconds` is where the live edge is in a new mpv's timeline (0 when
  /// mpv starts there by itself, as on Twitch; YouTube's lives start at their window's
  /// beginning, so [`LIVE_DELAY`] before its end).
  pub live: bool,
  /// A live whose past can be watched (YouTube's DVR): how far back, seconds. The seek bar
  /// covers that window up to the live edge (mpv's growing duration), a pause keeps the
  /// position, and the live badge brings back to the edge. The space may shorten it later
  /// ([`super::PlayerView::set_dvr_window`]).
  pub dvr_window: Option<f64>,
  /// Kept alive as long as the player holds the source: what serves its URLs, such as a local
  /// playlist proxy, which stops when dropped.
  pub keep_alive: Option<Rc<dyn std::any::Any>>,
}

/// A live whose past plays from another source: its recording, made as it goes (Twitch's VOD
/// of a live in progress), given once the space has it ([`super::PlayerView::set_rewind`]). The
/// seek bar then covers the broadcast, a seek back plays the recording from there, and the live
/// badge (or a seek to the edge) comes back to the live.
#[derive(Clone)]
pub struct Rewind {
  /// When the recording starts (its 0): the seek bar goes from there to now.
  pub started_at: std::time::SystemTime,
  /// The recording as a source, its qualities matched to the live's by label.
  pub recording: Source,
}

/// How far behind its edge a live plays, seconds: mpv waits for segments closer than about two
/// of YouTube's (5 s each).
pub const LIVE_DELAY: f64 = 15.0;

/// What the live badge shows next to it ([`super::PlayerView::set_live_info`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveInfo {
  /// Current viewers.
  pub viewers: Option<u64>,
  /// When the broadcast started: the badge shows the time since.
  pub started_at: Option<std::time::SystemTime>,
}

/// One quality of a source: switching to it restarts mpv with its options at the current
/// position.
#[derive(Clone)]
pub struct Quality {
  /// Shown in the menu and the OSD, e.g. `1080p60`.
  pub label: SharedString,
  /// mpv options for this quality (`start_seconds` is set by the player).
  pub options: StartOptions,
}

/// A subtitle track fetched by mpv when it is picked.
#[derive(Clone, Debug)]
pub struct Subtitle {
  /// Shown in the menu, e.g. `Anglais (générés automatiquement)`.
  pub label: String,
  /// Language code (`en`, `fr-CA`…), matched against the preferred language.
  pub language: String,
  /// URL mpv fetches.
  pub url: String,
}

/// Preview thumbnails cut out of sprite sheets (YouTube storyboards, Jellyfin trickplay).
pub trait Previews {
  /// The tile shown at `seconds` into a video of `duration` seconds.
  fn tile(&self, seconds: f64, duration: f64) -> Option<Tile>;
  /// Size of one tile, in pixels of the sheet.
  fn tile_size(&self) -> (u32, u32);
}

/// One preview: its sheet and where it sits on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
  /// Sheet URL.
  pub url: String,
  /// Column of the tile on the sheet, from 0.
  pub column: u32,
  /// Row of the tile on the sheet, from 0.
  pub row: u32,
}
