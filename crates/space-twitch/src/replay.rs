//! A VOD's chat, replayed as Twitch does: the messages appear beside the player when the playhead
//! reaches the second they were sent at, and a seek starts the chat over from the new position
//! (the page fetched there shows the messages just before it).
//!
//! Pages come from GQL by offset only (`Client::vod_comments`): the `after` cursor fails Twitch's
//! integrity check, so a page is asked from the last offset of the one before, the overlap
//! dropped by id. A second with more messages than a page (about 50) loses its tail.
//! The messages are those of [`super::chat`], 7TV emotes and badges included.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{App, Entity, Task, WeakEntity};
use twitch::catalog::Catalog;
use twitch::gql::{Comment, Comments};
use ui::chat::{ChatEvent, ChatPanel};
use ui::i18n::t;
use ui::player::PlayerView;
use ui::runtime;

use super::chat::{Extras, load_extras, message};

/// How often the playhead is read.
const TICK: Duration = Duration::from_millis(250);
/// The next page is asked once the fetched messages end less than this far ahead (seconds).
const AHEAD: f64 = 15.;
/// A forward move longer than this (seconds) is a seek, not playback (at most 4× speed for a tick
/// and a fetch).
const JUMP: f64 = 10.;
/// A backward move longer than this (seconds) is a seek.
const BACK: f64 = 1.5;
/// Before asking again after a network error.
const RETRY: Duration = Duration::from_secs(5);

/// The replay between two ticks.
#[derive(Debug, Default)]
struct Replay {
  /// Fetched and not shown yet, oldest first.
  queue: VecDeque<Comment>,
  /// Where the next page is asked from; `None` once the chat has no more.
  next: Option<u32>,
  /// The last offset fetched, and the ids fetched at it (the next page starts at or before it).
  edge: u32,
  edge_ids: HashSet<String>,
  /// Nothing fetched since the last reset.
  fresh: bool,
  /// The playhead at the last tick.
  position: f64,
}

impl Replay {
  /// Whether `position` is a seek from the last tick's.
  fn jumped(&self, position: f64) -> bool {
    position < self.position - BACK || position > self.position + JUMP
  }

  /// Starts over from `position`: nothing queued, a page to ask there.
  fn reset(&mut self, position: f64) {
    *self = Self {
      next: Some(position as u32),
      fresh: true,
      position,
      ..Self::default()
    };
  }

  /// The offset to ask a page from, when one is needed at `position`.
  fn wants(&self, position: f64) -> Option<u32> {
    let next = self.next?;
    (self.fresh || f64::from(self.edge) < position + AHEAD).then_some(next)
  }

  /// Queues `page`, asked `from`, without what an earlier page already brought.
  fn add(&mut self, page: Comments, from: u32) {
    self.fresh = false;
    let mut last = None;
    for comment in page.comments {
      if comment.offset < self.edge
        || (comment.offset == self.edge && self.edge_ids.contains(&comment.line.id))
      {
        continue;
      }
      if comment.offset > self.edge {
        self.edge = comment.offset;
        self.edge_ids.clear();
      }
      self.edge_ids.insert(comment.line.id.clone());
      last = Some(comment.offset);
      self.queue.push_back(comment);
    }
    // Every page moves on, even one that brought nothing new.
    self.next = page
      .more
      .then(|| last.filter(|&l| l > from).unwrap_or(from + 1));
  }

  /// Takes the messages due at `position`.
  fn due(&mut self, position: f64) -> Vec<Comment> {
    self.position = position;
    let mut due = Vec::new();
    while let Some(comment) = self.queue.front()
      && f64::from(comment.offset) <= position
    {
      due.extend(self.queue.pop_front());
    }
    due
  }

  /// No page is asked again (the VOD has no chat, or Twitch refuses it).
  fn stop(&mut self) {
    self.next = None;
  }
}

/// Replays VOD `id`'s chat into `panel`, following `player`, until the task is dropped.
pub fn start(
  id: String,
  client: twitch::Client,
  catalog: Catalog,
  panel: &Entity<ChatPanel>,
  player: WeakEntity<PlayerView>,
  cx: &mut App,
) -> Task<()> {
  let panel = panel.downgrade();
  let extras: Rc<RefCell<Rc<Extras>>> = Rc::default();
  cx.spawn(async move |cx| {
    let mut replay = Replay::default();
    let mut started = false;
    let mut extras_asked = false;
    let mut not_before = Instant::now();
    // The panel starts as a live's, connecting: nothing to say until the VOD plays.
    if panel
      .update(cx, |p, cx| p.push([ChatEvent::Status(None)], cx))
      .is_err()
    {
      return;
    }
    loop {
      cx.background_executor().timer(TICK).await;
      let Ok((mut position, duration)) = player.update(cx, |p, _| (p.position(), p.duration()))
      else {
        return;
      };
      // Not before the VOD plays: it may never (a subscriber-only one).
      if duration <= 0. {
        continue;
      }
      let mut events = Vec::new();
      if !started || replay.jumped(position) {
        if started {
          events.push(ChatEvent::Clear);
        }
        started = true;
        replay.reset(position);
        events.push(ChatEvent::Status(Some(
          t("Loading chat…", "Chargement du chat…").into(),
        )));
        // A seek bar being dragged moves on every tick: ask once it settles.
        not_before = Instant::now() + TICK;
      } else if let Some(from) = replay.wants(position)
        && Instant::now() >= not_before
      {
        let fetch = {
          let (client, id) = (client.clone(), id.clone());
          cx.update(|cx| runtime::spawn(cx, async move { client.vod_comments(&id, from).await }))
        };
        match fetch.await {
          Some(Ok(page)) => {
            if !extras_asked && let Some(channel) = page.channel_id.clone() {
              extras_asked = true;
              let (client, catalog, extras) = (client.clone(), catalog.clone(), extras.clone());
              cx.update(|cx| load_extras(channel, client, catalog, extras, cx).detach());
            }
            replay.add(page, from);
            events.push(ChatEvent::Status(None));
          }
          // Twitch's answer will not change: stop asking.
          Some(Err(
            e @ (twitch::Error::NotFound
            | twitch::Error::IntegrityRequired
            | twitch::Error::Gql(_)
            | twitch::Error::Parse { .. }),
          )) => {
            tracing::info!("VOD chat: {e}");
            replay.stop();
            events.push(ChatEvent::Status(Some(
              t(
                "This video's chat is unavailable.",
                "Le chat de cette vidéo est indisponible.",
              )
              .into(),
            )));
          }
          Some(Err(e)) => {
            tracing::info!("VOD chat: {e}");
            not_before = Instant::now() + RETRY;
            events.push(ChatEvent::Status(Some(
              t(
                "Chat unavailable · retrying in 5 s",
                "Chat indisponible · nouvel essai dans 5 s",
              )
              .into(),
            )));
          }
          None => return,
        }
        // The playhead may have moved during the fetch: read it again for the messages due.
        let Ok(now) = player.update(cx, |p, _| p.position()) else {
          return;
        };
        position = now;
        if replay.jumped(position) {
          // A seek during the fetch: the next tick starts over.
          if panel.update(cx, |p, cx| p.push(events, cx)).is_err() {
            return;
          }
          continue;
        }
      }
      let known = extras.borrow().clone();
      events.extend(
        replay
          .due(position)
          .iter()
          .map(|c| ChatEvent::Message(message(&c.line, None, &known))),
      );
      if !events.is_empty() && panel.update(cx, |p, cx| p.push(events, cx)).is_err() {
        return;
      }
    }
  })
}

#[cfg(test)]
mod tests {
  use twitch::chat::ChatLine;

  use super::*;

  fn page(comments: &[(u32, &str)], more: bool) -> Comments {
    Comments {
      channel_id: None,
      comments: comments
        .iter()
        .map(|&(offset, id)| Comment {
          offset,
          line: ChatLine {
            id: id.into(),
            ..ChatLine::default()
          },
        })
        .collect(),
      more,
    }
  }

  fn ids(comments: &[Comment]) -> Vec<&str> {
    comments.iter().map(|c| c.line.id.as_str()).collect()
  }

  #[test]
  fn a_reset_shows_what_came_just_before_then_follows_the_playhead() {
    let mut replay = Replay::default();
    replay.reset(600.);
    assert_eq!(replay.wants(600.), Some(600));
    replay.add(page(&[(581, "a"), (599, "b"), (605, "c")], true), 600);
    assert_eq!(ids(&replay.due(600.)), ["a", "b"]);
    assert!(replay.due(604.).is_empty());
    assert_eq!(ids(&replay.due(605.2)), ["c"]);
    // The next page starts from the last offset fetched.
    assert_eq!(replay.wants(605.2), Some(605));
  }

  #[test]
  fn overlapping_pages_bring_each_message_once() {
    let mut replay = Replay::default();
    replay.reset(3600.);
    replay.add(page(&[(3599, "a"), (3604, "b"), (3604, "c")], true), 3600);
    assert_eq!(replay.next, Some(3604));
    // Asked at 3604, the page starts back at 3602.
    replay.add(
      page(
        &[
          (3602, "x"),
          (3604, "b"),
          (3604, "c"),
          (3604, "d"),
          (3607, "e"),
        ],
        true,
      ),
      3604,
    );
    assert_eq!(ids(&replay.due(3610.)), ["a", "b", "c", "d", "e"]);
    assert_eq!(replay.next, Some(3607));
  }

  #[test]
  fn every_page_moves_on_and_the_last_stops() {
    let mut replay = Replay::default();
    replay.reset(10.);
    // A second fuller than a page: the same page again moves one second on.
    replay.add(page(&[(10, "a"), (10, "b")], true), 10);
    replay.add(page(&[(10, "a"), (10, "b")], true), 10);
    assert_eq!(replay.next, Some(11));
    replay.add(page(&[(30, "c")], false), 11);
    assert_eq!(replay.wants(30.), None);
  }

  #[test]
  fn ahead_enough_waits() {
    let mut replay = Replay::default();
    replay.reset(0.);
    replay.add(page(&[(3, "a"), (40, "b")], true), 0);
    assert_eq!(replay.wants(10.), None);
    assert_eq!(replay.wants(30.), Some(40));
  }

  #[test]
  fn seeks_are_told_from_playback() {
    let mut replay = Replay::default();
    replay.reset(100.);
    replay.due(100.);
    assert!(!replay.jumped(101.));
    assert!(!replay.jumped(99.));
    assert!(replay.jumped(97.));
    assert!(replay.jumped(120.));
  }
}
