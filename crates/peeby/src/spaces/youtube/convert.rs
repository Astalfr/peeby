//! YouTube data for the shared UI: thumbnails as pictures, streams as player sources, InnerTube
//! errors in French.

use std::rc::Rc;

use innertube::{Client, Storyboard, Stream, Thumbnail};
use player::StartOptions;
use ui::images::{Download, Fetch};
use ui::picture::{Picture, Size};
use ui::player::{Previews, Quality, Source, Subtitle, Tile};

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

/// French text for an InnerTube error, as shown in the UI (the `Display` text is English).
pub fn error_text(error: &innertube::Error) -> String {
  use innertube::Error;
  match error {
    Error::NotLoggedIn => "YouTube ne reconnaît plus la session : les cookies ont expiré ou ont \
                           changé. Exportez-les à nouveau."
      .into(),
    Error::SignInRequired => "Cette page demande d'être connecté.".into(),
    Error::Cookies(detail) => format!("Fichier de cookies invalide : {detail}"),
    Error::Unplayable { status, reason } if status == "LOGIN_REQUIRED" => {
      format!("Cette vidéo demande d'être connecté. {reason}")
    }
    Error::Unplayable { reason, .. } if !reason.is_empty() => {
      format!("Vidéo indisponible : {reason}")
    }
    Error::Unplayable { status, .. } => format!("Vidéo indisponible ({status})"),
    Error::Http(_) => "Impossible de joindre YouTube. Vérifiez la connexion.".into(),
    Error::Status { status, .. } => format!("YouTube a répondu par une erreur (HTTP {status})."),
    Error::Parse { .. } => "Réponse de YouTube inattendue : son format a peut-être changé.".into(),
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

/// A resolved stream as the player's source: its quality ladder, captions fetched on demand
/// and the storyboard for the seek bar; `player` gives the hardware acceleration setting.
pub fn source(stream: Stream, player: &settings::Player) -> Source {
  let qualities = stream
    .qualities
    .iter()
    .map(|v| Quality {
      label: v.label().into(),
      options: hls_options(&v.url, v.audio_url.as_deref(), stream.user_agent, player),
    })
    .collect();
  Source {
    options: hls_options(
      &stream.video_url,
      stream.audio_url.as_deref(),
      stream.user_agent,
      player,
    ),
    qualities,
    quality: stream
      .qualities
      .iter()
      .position(|q| q.url == stream.video_url),
    subtitles: stream
      .captions
      .into_iter()
      .map(|c| Subtitle {
        // YouTube's names, in the app's language: `Anglais (générés automatiquement)`.
        label: c.name,
        language: c.language,
        url: c.url,
      })
      .collect(),
    previews: stream
      .storyboard
      .map(|board| Rc::new(StoryboardPreviews(board)) as Rc<dyn Previews>),
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
