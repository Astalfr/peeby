//! The account's watch history and resume positions, kept by YouTube itself (ADR 0013): what
//! youtube.com's player reports, Peeby reports, so a video resumes where it stopped on any device.
//!
//! - Writing: a signed-in WEB `player` request gives the stats URLs ([`Client::tracking`], in the
//!   background, never before the first frame). When playback starts, one `playback` ping adds the
//!   video to the history; then `watchtime` pings report the watched segments: every
//!   [`PING_EVERY`] while playing, on pause, when the page goes and at the end. Their position is
//!   the resume position. Not for lives, and nothing signed out.
//! - Reading: YouTube's cards carry it (`innertube::Progress`), the click resumes from there, as
//!   on youtube.com. A video opened without a card (link, command line, notification) starts at
//!   0, unless it was played in this session: [`Resume`] keeps the last reported positions,
//!   newer than the cards already loaded.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{App, Context, Entity, Global};
use innertube::{Client, Tracking, VideoItem, Watchtime};
use ui::player::{PlayerEvent, PlayerView};
use ui::runtime;

/// How often a playing video reports its segment (youtube.com: every 40 s after the first ones).
pub const PING_EVERY: Duration = Duration::from_secs(30);

/// Closer to the end than this, a video counts as watched: the next playback starts over.
const END_MARGIN: f64 = 10.0;

/// Where videos resume, for the session.
#[derive(Default)]
pub struct Resume {
  /// Last position reported per video id; `None` once watched to the end.
  played: HashMap<String, Option<u32>>,
  /// Cards clicked whose page has not opened yet (a tab opened in the background waits until it
  /// shows): where YouTube resumes each video.
  offered: HashMap<String, u32>,
}

impl Global for Resume {}

/// Where a card resumes `video` ([`offer`]).
pub fn card_resume(video: &VideoItem) -> Option<u32> {
  video.progress.and_then(|p| p.resume)
}

/// Before opening `video_id`'s page from its card: the page resumes where the card says.
pub fn offer(video_id: &str, resume: Option<u32>, cx: &mut App) {
  let offered = &mut cx.default_global::<Resume>().offered;
  match resume {
    Some(seconds) => offered.insert(video_id.to_owned(), seconds),
    None => offered.remove(video_id),
  };
}

/// Where the page of `video_id` starts: this session's last position, else its card's.
pub fn start_at(video_id: &str, cx: &mut App) -> Option<f64> {
  let resume = cx.default_global::<Resume>();
  let offered = resume.offered.remove(video_id);
  match resume.played.get(video_id) {
    Some(played) => *played,
    None => offered,
  }
  .filter(|&s| s > 0)
  .map(f64::from)
}

fn remember(video_id: &str, position: f64, duration: f64, cx: &mut App) {
  let watched = duration > 0.0 && position >= duration - END_MARGIN;
  // Truncation on purpose: a resume a fraction of a second early.
  let seconds = (!watched).then_some(position as u32);
  cx.default_global::<Resume>()
    .played
    .insert(video_id.to_owned(), seconds);
}

/// What one watch page reports.
/// Asks for the stats URLs of `reporter`'s video (signed in), then pings every [`PING_EVERY`]
/// while the view lives; `parts` gives the view's reporter and player.
pub fn report<V: 'static>(
  reporter: &Reporter,
  parts: fn(&mut V) -> (&mut Reporter, Entity<PlayerView>),
  cx: &mut Context<V>,
) {
  let Some(tracking) = reporter.fetch_tracking(cx) else {
    return;
  };
  cx.spawn(async move |this, cx| {
    match tracking.await {
      Some(Ok(tracking)) => {
        let set = this.update(cx, |this, cx| {
          let (reporter, player) = parts(this);
          reporter.set_tracking(tracking, &player, cx);
        });
        if set.is_err() {
          return;
        }
      }
      Some(Err(e)) => {
        tracing::warn!("YouTube history: no stats URLs: {e}");
        return;
      }
      None => return,
    }
    loop {
      cx.background_executor().timer(PING_EVERY).await;
      let ticked = this.update(cx, |this, cx| {
        let (reporter, player) = parts(this);
        reporter.tick(&player, cx);
      });
      if ticked.is_err() {
        break;
      }
    }
  })
  .detach();
}

pub struct Reporter {
  client: Client,
  video_id: String,
  /// From the WEB `player` request, once it answered.
  tracking: Option<Tracking>,
  /// The player started a video (not a live): pings may go.
  playing: bool,
  /// The `playback` ping went.
  announced: bool,
  /// Where the segment being watched started.
  from: f64,
  /// The last ping was the final one: nothing more for this playback.
  finished: bool,
}

impl Reporter {
  /// A reporter for `video_id`; it reports once [`Reporter::set_tracking`] got the URLs.
  pub fn new(client: Client, video_id: String) -> Self {
    Self {
      client,
      video_id,
      tracking: None,
      playing: false,
      announced: false,
      from: 0.0,
      finished: false,
    }
  }

  /// The stats URLs of the video, in the background; `None` signed out.
  pub fn fetch_tracking(
    &self,
    cx: &App,
  ) -> Option<impl Future<Output = Option<Result<Tracking, innertube::Error>>> + use<>> {
    if !self.client.signed_in() {
      return None;
    }
    let (client, id) = (self.client.clone(), self.video_id.clone());
    Some(runtime::spawn(
      cx,
      async move { client.tracking(&id).await },
    ))
  }

  /// The URLs came: the `playback` ping goes if the video already plays.
  pub fn set_tracking(&mut self, tracking: Tracking, player: &Entity<PlayerView>, cx: &mut App) {
    self.tracking = Some(tracking);
    self.announce(player, cx);
  }

  /// Follows the player's events.
  pub fn on_event(&mut self, event: &PlayerEvent, player: &Entity<PlayerView>, cx: &mut App) {
    match event {
      PlayerEvent::Started => {
        self.playing = !player.read(cx).is_live();
        self.from = player.read(cx).position();
        self.announce(player, cx);
      }
      PlayerEvent::Paused(true) => self.report(player, true, false, cx),
      PlayerEvent::Paused(false) => self.from = player.read(cx).position(),
      // The segment before the seek ended at the last ping; the next one starts here.
      PlayerEvent::Seeked => self.from = player.read(cx).position(),
      PlayerEvent::Ended | PlayerEvent::Next | PlayerEvent::Previous => {
        self.report(player, true, true, cx)
      }
      _ => {}
    }
  }

  /// The periodic ping, while playing.
  pub fn tick(&mut self, player: &Entity<PlayerView>, cx: &mut App) {
    if !player.read(cx).is_paused() {
      self.report(player, false, false, cx);
    }
  }

  /// The page goes: the last ping.
  pub fn close(&mut self, player: &Entity<PlayerView>, cx: &mut App) {
    self.report(player, true, true, cx);
  }

  /// The `playback` ping, once both the player started and the URLs came.
  fn announce(&mut self, player: &Entity<PlayerView>, cx: &mut App) {
    if !self.playing || self.announced {
      return;
    }
    let Some(tracking) = self.tracking.clone() else {
      return;
    };
    self.announced = true;
    let position = player.read(cx).position();
    let client = self.client.clone();
    drop(runtime::spawn(cx, async move {
      if let Err(e) = client.report_playback(&tracking, position).await {
        tracing::warn!("YouTube history: {e}");
      }
    }));
  }

  /// A `watchtime` ping from the segment's start to the current position.
  fn report(&mut self, player: &Entity<PlayerView>, paused: bool, last: bool, cx: &mut App) {
    if !self.announced || self.finished {
      return;
    }
    let Some(tracking) = self.tracking.clone() else {
      return;
    };
    let (position, duration) = {
      let player = player.read(cx);
      (player.position(), player.duration())
    };
    let segment = Watchtime {
      from: self.from.min(position),
      to: position,
      paused,
      last,
    };
    self.from = position;
    self.finished = last;
    remember(&self.video_id, position, duration, cx);
    let client = self.client.clone();
    drop(runtime::spawn(cx, async move {
      if let Err(e) = client.report_watchtime(&tracking, segment).await {
        tracing::warn!("YouTube watch time: {e}");
      }
    }));
  }
}
