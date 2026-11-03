//! The subscribed channels on air, for the notification area's list (ADR 0020): the lives at the
//! top of the subscriptions feed, read when the list opens, at most once a minute. Nothing reads
//! them while nobody looks; the inbox is never used for it (reading it clears youtube.com's
//! badge).

use std::time::{Duration, Instant};

use gpui::{Context, Task};
use innertube::{Client, VideoItem};
use ui::i18n::t;
use ui::runtime;

use super::YouTubeSpace;
use super::convert;
use crate::shell::space::{Avatar, LiveEntry, Lives};

/// A list read this recently is shown as it is.
const FRESH: Duration = Duration::from_secs(60);

/// The last reading.
#[derive(Default)]
pub struct Live {
  /// The lives of the last reading; `None` before the first.
  items: Option<Vec<VideoItem>>,
  read: Option<Instant>,
  /// Why the last reading failed (the list says so rather than "nobody").
  error: Option<String>,
  reading: Option<Task<()>>,
}

impl Live {
  /// What the list shows: `signed_in` is the client's session.
  pub fn lives(&self, signed_in: bool) -> Lives {
    if !signed_in {
      return Lives::Unavailable(
        t(
          "Sign in to YouTube in Peeby's settings to see the subscribed channels on air.",
          "Connectez-vous à YouTube dans les réglages de Peeby pour voir les chaînes abonnées \
           en direct.",
        )
        .into(),
      );
    }
    if let Some(error) = &self.error {
      return Lives::Unavailable(error.clone().into());
    }
    let Some(items) = &self.items else {
      return Lives::Loading;
    };
    Lives::On(items.iter().map(entry).collect())
  }

  /// Reads the feed again unless the last reading is fresh or one is under way.
  pub fn refresh(&mut self, client: &Client, cx: &mut Context<YouTubeSpace>) {
    let fresh = self.read.is_some_and(|at| at.elapsed() < FRESH);
    if !client.signed_in() || fresh || self.reading.is_some() {
      return;
    }
    let client = client.clone();
    let fetch = runtime::spawn(cx, async move { client.subscriptions().await });
    let task = cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          this.live.reading = None;
          match result {
            Some(Ok(feed)) => {
              this.live.items = Some(feed.items.into_iter().filter(on_air).collect());
              this.live.read = Some(Instant::now());
              this.live.error = None;
            }
            // Unread: the next opening tries again.
            Some(Err(e)) => {
              tracing::warn!("subscriptions' lives: {e}");
              this.live.error = Some(convert::error_text(&e));
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    });
    self.reading = Some(task);
  }
}

/// A live happening now (a premiere to come has no live badge, a finished one a duration).
fn on_air(video: &VideoItem) -> bool {
  video.live && video.duration.is_none()
}

fn entry(video: &VideoItem) -> LiveEntry {
  let channel = video.channel.clone().unwrap_or_default();
  LiveEntry {
    id: video.id.clone().into(),
    title: video.title.replace(['\n', '\r'], " ").into(),
    detail: None,
    // YouTube's own words, in the session's language: "169 spectateurs".
    viewers: video.metadata.first().cloned().map(Into::into),
    since: None,
    preview: (!video.thumbnails.is_empty()).then(|| convert::picture(&video.thumbnails)),
    avatar: Avatar::of(
      &channel,
      (!video.avatar.is_empty()).then(|| convert::picture(&video.avatar)),
    ),
    channel: channel.into(),
  }
}

#[cfg(test)]
mod tests {
  use super::on_air;

  #[test]
  fn keeps_the_lives_on_air() {
    let text = include_str!("../../../../innertube/tests/fixtures/subscriptions_feed_shelves.json");
    let response = serde_json::from_str(text).unwrap();
    let feed = innertube::parse_subscriptions(&response);
    let lives: Vec<_> = feed.items.iter().filter(|v| on_air(v)).collect();
    assert_eq!(lives.len(), 1);
    assert!(
      feed
        .items
        .iter()
        .any(|v| v.duration.is_some() && !on_air(v))
    );
  }
}
