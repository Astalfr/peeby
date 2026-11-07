//! A VOD's resume position, kept on the viewer's Twitch account (ADR 0013), as twitch.tv's player
//! keeps it: read when the VOD opens (`video.self.viewingHistory`), saved with
//! `updateUserViewedVideo` every [`SAVE_EVERY`] while playing, on pause and when the page goes.
//! Needs the web auth-token; without it nothing is read or saved.

use std::time::Duration;

use gpui::{App, Entity};
use twitch::gql::Viewing;
use ui::player::{PlayerEvent, PlayerView};
use ui::runtime;

/// How often a playing VOD saves its position.
pub const SAVE_EVERY: Duration = Duration::from_secs(30);

/// Closer to the end than this, a VOD counts as watched: it starts over.
const END_MARGIN: u32 = 30;

/// Where a VOD starts: its saved position, unless it was watched to the end.
pub fn start_at(viewing: &Viewing) -> Option<f64> {
  let position = viewing.position.filter(|&p| p > 0)?;
  (viewing.length == 0 || position + END_MARGIN < viewing.length).then_some(f64::from(position))
}

/// Saves the position of one VOD.
pub struct Saver {
  client: twitch::Client,
  video_id: String,
  user_id: String,
  /// The last position saved, seconds: the same one is not sent again.
  saved: Option<u32>,
}

impl Saver {
  /// A saver for `video_id`, when the auth-token is the account's `viewing.user_id`.
  pub fn new(client: twitch::Client, video_id: String, viewing: &Viewing) -> Option<Self> {
    Some(Self {
      client,
      video_id,
      user_id: viewing.user_id.clone()?,
      saved: viewing.position,
    })
  }

  /// Follows the player's events.
  pub fn on_event(&mut self, event: &PlayerEvent, player: &Entity<PlayerView>, cx: &mut App) {
    if matches!(event, PlayerEvent::Paused(true) | PlayerEvent::Ended) {
      self.save(player, cx);
    }
  }

  /// The periodic save, while playing.
  pub fn tick(&mut self, player: &Entity<PlayerView>, cx: &mut App) {
    if !player.read(cx).is_paused() {
      self.save(player, cx);
    }
  }

  /// Saves the player's position, if it moved.
  pub fn save(&mut self, player: &Entity<PlayerView>, cx: &mut App) {
    let position = player.read(cx).position();
    self.save_at(position, cx);
  }

  /// Saves `position`, seconds, if it moved.
  pub fn save_at(&mut self, position: f64, cx: &mut App) {
    // Truncation on purpose: whole seconds, a resume a fraction early.
    let position = position as u32;
    if position == 0 || self.saved == Some(position) {
      return;
    }
    self.saved = Some(position);
    let (client, user, id) = (
      self.client.clone(),
      self.user_id.clone(),
      self.video_id.clone(),
    );
    drop(runtime::spawn(cx, async move {
      if let Err(e) = client.save_vod_position(&user, &id, position).await {
        tracing::warn!("VOD position: {e}");
      }
    }));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn viewing(position: Option<u32>, length: u32) -> Viewing {
    Viewing {
      user_id: Some("1".into()),
      position,
      length,
    }
  }

  #[test]
  fn resumes_unless_watched_to_the_end() {
    assert_eq!(start_at(&viewing(Some(2940), 29136)), Some(2940.0));
    assert_eq!(start_at(&viewing(Some(29120), 29136)), None);
    assert_eq!(start_at(&viewing(Some(0), 29136)), None);
    assert_eq!(start_at(&viewing(None, 29136)), None);
  }

  #[test]
  fn signed_out_saves_nothing() {
    let client = twitch::Client::new(None, None).unwrap();
    let signed_out = Viewing {
      user_id: None,
      ..viewing(None, 10)
    };
    assert!(Saver::new(client, "1".into(), &signed_out).is_none());
  }
}
