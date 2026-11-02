//! YouTube data for the shared UI: thumbnails as pictures, streams as player sources, InnerTube
//! errors in the interface's language.

use std::rc::Rc;

use innertube::{Chapter, Client, Storyboard, Stream, Thumbnail};
use player::StartOptions;
use ui::i18n::t;
use ui::images::{Download, Fetch};
use ui::picture::{Picture, Size};
use ui::player::{LIVE_DELAY, Previews, Quality, Source, Subtitle, Tile};
use ui::tf;

use crate::settings;

/// Downloads images with an InnerTube client (its HTTP client and User-Agent).
pub struct ImageFetch(pub Client);

impl Fetch for ImageFetch {
  fn fetch(&self, url: &str) -> Download {
    let (client, url) = (self.0.clone(), url.to_owned());
    Box::pin(async move { client.fetch_bytes(&url).await.map_err(|e| e.to_string()) })
  }
}

/// YouTube's sizes of an image (thumbnail, avatar) as a picture.
pub fn picture(sizes: &[Thumbnail]) -> Picture {
  Picture::Sizes(
    sizes
      .iter()
      .map(|t| Size {
        url: t.url.clone(),
        width: t.width,
      })
      .collect(),
  )
}

/// The text of an InnerTube error as shown in the UI, in the interface's language (the
/// `Display` text is for the logs). YouTube's own reasons stay in its content language.
pub fn error_text(error: &innertube::Error) -> String {
  use innertube::Error;
  match error {
    Error::NotLoggedIn => t(
      "YouTube no longer recognizes the session: the cookies have expired or changed. \
       Export them again.",
      "YouTube ne reconnaît plus la session : les cookies ont expiré ou ont changé. \
       Exportez-les à nouveau.",
    )
    .into(),
    Error::SignInRequired => t(
      "This page requires signing in.",
      "Cette page demande d'être connecté.",
    )
    .into(),
    Error::Cookies(detail) => tf!(
      "Invalid cookie file: {detail}",
      "Fichier de cookies invalide : {detail}"
    ),
    Error::Unplayable { status, reason } if status == "LOGIN_REQUIRED" => tf!(
      "This video requires signing in. {reason}",
      "Cette vidéo demande d'être connecté. {reason}"
    ),
    Error::Unplayable { reason, .. } if !reason.is_empty() => tf!(
      "Video unavailable: {reason}",
      "Vidéo indisponible : {reason}"
    ),
    Error::Unplayable { status, .. } => tf!(
      "Video unavailable ({status})",
      "Vidéo indisponible ({status})"
    ),
    Error::Http(_) => t(
      "Cannot reach YouTube. Check the connection.",
      "Impossible de joindre YouTube. Vérifiez la connexion.",
    )
    .into(),
    Error::Status { status, .. } => tf!(
      "YouTube answered with an error (HTTP {status}).",
      "YouTube a répondu par une erreur (HTTP {status})."
    ),
    Error::Parse { .. } => t(
      "Unexpected answer from YouTube: its format may have changed.",
      "Réponse de YouTube inattendue : son format a peut-être changé.",
    )
    .into(),
  }
}

/// mpv options for an HLS variant and its audio rendition (YouTube's variants are video only).
fn hls_options(
  video: &str,
  audio: Option<&str>,
  user_agent: &str,
  player: &settings::Player,
) -> StartOptions {
  let mut options = StartOptions {
    url: video.to_owned(),
    external_audio: audio.map(str::to_owned).into_iter().collect(),
    http_headers: vec![format!("User-Agent: {user_agent}")],
    // Opt-in only: the log contains stream URLs, which carry the user's IP.
    log_file: std::env::var_os("PEEBY_MPV_LOG").map(Into::into),
    ..Default::default()
  };
  player.apply(&mut options);
  options
}

/// The watch page's chapters, for the player's seek bar.
pub fn chapters(chapters: &[Chapter]) -> Vec<player::Chapter> {
  chapters
    .iter()
    .map(|c| player::Chapter {
      start: c.start,
      title: Some(c.title.clone()),
    })
    .collect()
}

/// A resolved stream as the player's source: its quality ladder, captions fetched on demand
/// and the storyboard for the seek bar; `player` gives the hardware acceleration setting. A live
/// starts [`LIVE_DELAY`] before the end of its window (mpv's timeline starts at the window's
/// first segment) and, with DVR, can be sought back over its window.
pub fn source(stream: Stream, player: &settings::Player) -> Source {
  let qualities = stream
    .qualities
    .iter()
    .map(|v| Quality {
      label: v.label().into(),
      options: hls_options(&v.url, v.audio_url.as_deref(), stream.user_agent, player),
    })
    .collect();
  let mut options = hls_options(
    &stream.video_url,
    stream.audio_url.as_deref(),
    stream.user_agent,
    player,
  );
  if let Some(live) = stream.live {
    // Never 0, which gives mpv no `start` at all: it would wait on the audio rendition again
    // (a live opened in its first seconds, its window shorter than the delay).
    options.start_seconds = (live.length - LIVE_DELAY).max(1.0);
  }
  Source {
    options,
    qualities,
    quality: stream
      .qualities
      .iter()
      .position(|q| q.url == stream.video_url),
    subtitles: stream
      .captions
      .into_iter()
      .map(|c| Subtitle {
        // YouTube's names, in its content language: `Anglais (générés automatiquement)`.
        label: c.name,
        language: c.language,
        url: c.url,
      })
      .collect(),
    previews: stream
      .storyboard
      .map(|board| Rc::new(StoryboardPreviews(board)) as Rc<dyn Previews>),
    live: stream.live.is_some(),
    dvr_window: stream.live.and_then(|live| live.dvr_window),
    ..Default::default()
  }
}

/// A storyboard as the seek bar's previews.
struct StoryboardPreviews(Storyboard);

impl Previews for StoryboardPreviews {
  fn tile(&self, seconds: f64, duration: f64) -> Option<Tile> {
    let frame = self.0.frame(seconds, duration)?;
    Some(Tile {
      url: frame.url,
      column: frame.column,
      row: frame.row,
    })
  }

  fn tile_size(&self) -> (u32, u32) {
    (self.0.width, self.0.height)
  }
}
