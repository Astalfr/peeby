//! Twitch playback as the shared player's [`Source`]:
//! - a live or a VOD: the variants the local proxy serves (a live's ads removed, docs/twitch-plan.md
//!   step 3), one quality each;
//! - a clip: its direct MP4s, one quality each.
//!
//! A VOD's storyboard becomes the seek bar's previews.

use std::rc::Rc;

use player::StartOptions;
use twitch::Proxy;
use twitch::gql::{ClipPlayback, StoryboardLevel};
use twitch::usher::Variant;
use ui::player::{Previews, Quality, Source, Tile};

use crate::settings;

/// Qualities offered in the settings, as heights.
pub const QUALITIES: &[u32] = &[1440, 1080, 720, 480, 360];

/// Default highest quality.
pub const DEFAULT_HEIGHT: u32 = 1080;

/// mpv's options for `url`: Twitch's CDN wants the player's origin.
fn options(url: &str, player: &settings::Player) -> StartOptions {
  let mut options = StartOptions {
    url: url.to_owned(),
    http_headers: vec![
      format!("Referer: {}", twitch::usher::PLAYER_ORIGIN),
      format!("Origin: {}", twitch::usher::PLAYER_ORIGIN),
    ],
    ..StartOptions::default()
  };
  player.apply(&mut options);
  options
}

/// The source for `qualities` (label, URL, height), best first: the first one not taller than
/// `max_height` plays.
fn source(
  qualities: Vec<(String, String, Option<u32>)>,
  max_height: u32,
  live: bool,
  player: &settings::Player,
) -> Source {
  let index = qualities
    .iter()
    .position(|(_, _, h)| h.is_some_and(|h| h <= max_height))
    .unwrap_or(0);
  let qualities: Vec<Quality> = qualities
    .into_iter()
    .map(|(label, url, _)| Quality {
      label: label.into(),
      options: options(&url, player),
    })
    .collect();
  Source {
    options: qualities
      .get(index)
      .map(|q| q.options.clone())
      .unwrap_or_default(),
    quality: (!qualities.is_empty()).then_some(index),
    qualities,
    live,
    ..Source::default()
  }
}

/// A live's or a VOD's source from `proxy`'s variants: the landscape ones with a picture, then
/// "Audio seul".
pub fn proxied(proxy: &Proxy, live: bool, max_height: u32, player: &settings::Player) -> Source {
  let mut playable: Vec<&Variant> = proxy
    .variants()
    .iter()
    .filter(|v| !v.portrait && !v.audio_only())
    .collect();
  playable.extend(proxy.variants().iter().filter(|v| v.audio_only()).take(1));
  let qualities = playable
    .iter()
    .map(|v| {
      let label = if v.audio_only() {
        "Audio seul".to_owned()
      } else {
        v.name.clone()
      };
      (label, v.url.clone(), v.height)
    })
    .collect();
  source(qualities, max_height, live, player)
}

/// A clip's source: one quality per MP4.
pub fn clip(clip: &ClipPlayback, max_height: u32, player: &settings::Player) -> Source {
  let qualities = clip
    .qualities
    .iter()
    .map(|q| {
      let label = if q.frame_rate > 30 {
        format!("{}p{}", q.height, q.frame_rate)
      } else {
        format!("{}p", q.height)
      };
      (label, q.url.clone(), Some(q.height))
    })
    .collect();
  source(qualities, max_height, false, player)
}

/// A VOD's storyboard as the seek bar's previews.
pub struct Storyboard(pub StoryboardLevel);

impl Previews for Storyboard {
  fn tile(&self, seconds: f64, _duration: f64) -> Option<Tile> {
    let (url, column, row) = self.0.frame_at(seconds)?;
    Some(Tile {
      url: url.to_owned(),
      column,
      row,
    })
  }

  fn tile_size(&self) -> (u32, u32) {
    (self.0.width, self.0.height)
  }
}

/// The previews of the storyboard's largest level, if any.
pub fn previews(levels: Vec<StoryboardLevel>) -> Option<Rc<dyn Previews>> {
  let level = levels.into_iter().next()?;
  Some(Rc::new(Storyboard(level)) as Rc<dyn Previews>)
}

/// What the user reads when a live, a VOD or a clip cannot play.
pub fn describe(error: &twitch::Error, channel: &str) -> String {
  use twitch::Error;
  match error {
    Error::Offline => format!("{channel} n'est pas en direct."),
    Error::NotFound => "Cette chaîne ou cette vidéo n'existe pas.".into(),
    Error::SubscriberOnly => "Réservé aux abonnés de la chaîne.".into(),
    Error::IntegrityRequired => {
      "Twitch demande une vérification que Peeby ne peut pas faire sans navigateur. Réessayez plus \
       tard."
        .into()
    }
    Error::NotLoggedIn => {
      "Twitch refuse votre jeton du site : remplacez-le ou supprimez-le dans les réglages.".into()
    }
    Error::Http(_) => "Twitch est injoignable. Vérifiez votre connexion.".into(),
    other => format!("Lecture impossible : {other}"),
  }
}
