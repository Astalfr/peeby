//! Every mpv alive, for the settings' debug section: the player of each page, and the players a
//! page replaced (another quality, a reload) as long as something still holds them, such as
//! the previous picture.

use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use gpui::{App, Global, SharedString, WeakEntity};
use player::{Backend, Player};

use super::PlayerView;

/// The players started, held weakly: an entry whose mpv is gone is dropped.
#[derive(Default)]
struct Running(Vec<Entry>);

impl Global for Running {}

struct Entry {
  player: Weak<Player>,
  view: WeakEntity<PlayerView>,
  started: Instant,
}

/// Where a running player stands with its page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
  /// Its page plays it.
  Page,
  /// Its page plays another one since; something else holds this one.
  Replaced,
  /// Its page is closed; something else holds it.
  Closed,
}

/// One running mpv, as the debug section shows it.
#[derive(Clone, Debug)]
pub struct RunningPlayer {
  /// Its page's title, empty when the page has none or is closed.
  pub title: SharedString,
  /// Where it stands with its page.
  pub owner: Owner,
  /// It is the player that started last (the mini-player's).
  pub now_playing: bool,
  /// Time since it started.
  pub age: Duration,
  /// mpv's `time-pos`, seconds.
  pub position: Option<f64>,
  /// mpv's `duration`, seconds.
  pub duration: Option<f64>,
  /// Paused (`pause`).
  pub paused: bool,
  /// Waiting on its cache (`paused-for-cache`).
  pub buffering: bool,
  /// Rendering path.
  pub backend: Backend,
  /// mpv's `hwdec-current`: `no` when decoding in software.
  pub hwdec: Option<String>,
}

/// Records `player`, which `view` just started.
pub(super) fn register(player: &Rc<Player>, view: WeakEntity<PlayerView>, cx: &mut App) {
  let running = cx.default_global::<Running>();
  running.0.retain(|e| e.player.strong_count() > 0);
  running.0.push(Entry {
    player: Rc::downgrade(player),
    view,
    started: Instant::now(),
  });
}

/// The mpv players alive, oldest first. Reads a few of each one's properties.
pub fn running(cx: &App) -> Vec<RunningPlayer> {
  let Some(running) = cx.try_global::<Running>() else {
    return Vec::new();
  };
  let now_playing = super::now_playing(cx).map(|v| v.entity_id());
  running
    .0
    .iter()
    .filter_map(|entry| {
      let player = entry.player.upgrade()?;
      let view = entry.view.upgrade();
      let (title, owner) = match &view {
        Some(view) => {
          let view = view.read(cx);
          let plays = view.player.as_ref().is_some_and(|p| Rc::ptr_eq(p, &player));
          let owner = if plays { Owner::Page } else { Owner::Replaced };
          (view.title.clone(), owner)
        }
        None => (SharedString::default(), Owner::Closed),
      };
      let number = |name: &str| player.property(name).and_then(|v| v.parse::<f64>().ok());
      let flag = |name: &str| player.property(name).is_some_and(|v| v == "yes");
      Some(RunningPlayer {
        title,
        owner,
        now_playing: owner == Owner::Page
          && view.is_some_and(|v| Some(v.entity_id()) == now_playing),
        age: entry.started.elapsed(),
        position: number("time-pos"),
        duration: number("duration"),
        paused: flag("pause"),
        buffering: flag("paused-for-cache"),
        backend: player.backend(),
        hwdec: player.property("hwdec-current"),
      })
    })
    .collect()
}
