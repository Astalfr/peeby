//! YouTube data for the shared UI: thumbnails as pictures, InnerTube errors in French.

use innertube::{Client, Thumbnail};
use ui::images::{Download, Fetch};
use ui::picture::{Picture, Size};

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
