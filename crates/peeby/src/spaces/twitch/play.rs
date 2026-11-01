//! A Twitch live as the shared player's [`Source`]: the variants the local proxy serves (ads
//! removed, docs/twitch-plan.md step 3), one quality each, and mpv's headers for the CDN.

use player::StartOptions;
use twitch::Proxy;
use twitch::usher::Variant;
use ui::player::{Quality, Source};

use crate::settings;

/// Qualities offered in the settings, as heights.
pub const QUALITIES: &[u32] = &[1440, 1080, 720, 480, 360];

/// Default highest quality.
pub const DEFAULT_HEIGHT: u32 = 1080;

/// The player's source for `proxy`'s variants: the landscape ones with a picture, best first, then
/// "Audio seul"; the first one not taller than `max_height` plays.
pub fn source(proxy: &Proxy, max_height: u32, player: &settings::Player) -> Source {
  let options = |variant: &Variant| {
    let mut options = StartOptions {
      url: variant.url.clone(),
      // Segments come from Twitch's CDN, which wants the player's origin.
      http_headers: vec![
        format!("Referer: {}", twitch::usher::PLAYER_ORIGIN),
        format!("Origin: {}", twitch::usher::PLAYER_ORIGIN),
      ],
      ..StartOptions::default()
    };
    player.apply(&mut options);
    options
  };
  let mut playable: Vec<&Variant> = proxy
    .variants()
    .iter()
    .filter(|v| !v.portrait && !v.audio_only())
    .collect();
  playable.extend(proxy.variants().iter().filter(|v| v.audio_only()).take(1));
  let qualities: Vec<Quality> = playable
    .iter()
    .map(|v| Quality {
      label: if v.audio_only() {
        "Audio seul".into()
      } else {
        v.name.clone().into()
      },
      options: options(v),
    })
    .collect();
  let index = playable
    .iter()
    .position(|v| v.height.is_some_and(|h| h <= max_height))
    .unwrap_or(0);
  Source {
    options: qualities
      .get(index)
      .map(|q| q.options.clone())
      .unwrap_or_default(),
    quality: (!qualities.is_empty()).then_some(index),
    qualities,
    live: true,
    ..Source::default()
  }
}

/// What the user reads when a live cannot play.
pub fn describe(error: &twitch::Error, channel: &str) -> String {
  use twitch::Error;
  match error {
    Error::Offline => format!("{channel} n'est pas en direct."),
    Error::NotFound => format!("La chaîne {channel} n'existe pas."),
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
