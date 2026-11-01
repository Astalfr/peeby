//! Twitch's section of the bell: the followed channels that went live while Peeby runs, from the
//! sidebar's reading of the followed lives (every two minutes). The first reading only learns who
//! is on air, so that opening Peeby does not announce every live at once. Opening the panel sees
//! them; a click opens the live.

use std::collections::{HashSet, VecDeque};
use std::time::SystemTime;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, ElementId, SharedString, WeakEntity, Window, div, px};
use gpui_component::popover::PopoverState;
use gpui_component::{h_flex, v_flex};
use twitch::helix::Stream;
use ui::images::Images;
use ui::picture;
use ui::theme;

use super::cards::{self, Avatars};
use super::{Route, TwitchSpace};
use crate::shell::space::SpaceEvent;

/// Lives kept in the panel.
const KEPT: usize = 30;
const AVATAR: f32 = 40.;

/// A followed channel that went live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WentLive {
  pub user_id: String,
  pub login: String,
  pub name: String,
  pub title: String,
  pub category: String,
  /// When Peeby saw it.
  pub at: SystemTime,
}

/// The lives seen going on air, newest first, and how many the user has not seen.
#[derive(Default)]
pub struct Bell {
  /// Who was on air at the last reading; `None` before the first.
  known: Option<HashSet<String>>,
  entries: VecDeque<WentLive>,
  unseen: usize,
}

impl Bell {
  /// A new reading of the followed lives (`live`, every page read) at `now`.
  pub fn update(&mut self, live: &[Stream], now: SystemTime) {
    let current: HashSet<String> = live.iter().map(|s| s.user_id.clone()).collect();
    if let Some(known) = &self.known {
      for stream in live.iter().filter(|s| !known.contains(&s.user_id)) {
        // A channel back on air replaces its older entry.
        self.entries.retain(|e| e.user_id != stream.user_id);
        self.entries.push_front(WentLive {
          user_id: stream.user_id.clone(),
          login: stream.user_login.clone(),
          name: stream.user_name.clone(),
          title: stream.title.clone(),
          category: stream.game_name.clone(),
          at: now,
        });
        self.unseen += 1;
      }
      self.entries.truncate(KEPT);
      self.unseen = self.unseen.min(self.entries.len());
    }
    self.known = Some(current);
  }

  /// Lives not seen yet.
  pub fn unseen(&self) -> usize {
    self.unseen
  }

  /// Everything is seen.
  pub fn see(&mut self) {
    self.unseen = 0;
  }

  /// The lives, newest first.
  pub fn entries(&self) -> Vec<WentLive> {
    self.entries.iter().cloned().collect()
  }
}

/// The section of the open panel. It lives in the popover's element state: made when the panel
/// opens, with the lives as they were (the new ones keep their dot while it is open).
pub fn panel(
  nav: WeakEntity<TwitchSpace>,
  popover: WeakEntity<PopoverState>,
  entries: Vec<WentLive>,
  unseen: usize,
  avatars: Avatars,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  window
    .use_keyed_state(ElementId::Name("twitch-bell".into()), cx, |_, cx| {
      BellPanel::new(nav, popover, entries, unseen, avatars, cx)
    })
    .into_any_element()
}

struct BellPanel {
  nav: WeakEntity<TwitchSpace>,
  popover: WeakEntity<PopoverState>,
  entries: Vec<WentLive>,
  unseen: usize,
  avatars: Avatars,
}

impl BellPanel {
  fn new(
    nav: WeakEntity<TwitchSpace>,
    popover: WeakEntity<PopoverState>,
    entries: Vec<WentLive>,
    unseen: usize,
    avatars: Avatars,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    // Opening the panel sees the lives; the space is being drawn now, so later.
    let see = nav.clone();
    cx.spawn(async move |_, cx| {
      see.update(cx, |space, cx| space.see_bell(cx)).ok();
    })
    .detach();
    Self {
      nav,
      popover,
      entries,
      unseen,
      avatars,
    }
  }

  fn open(&mut self, login: String, window: &mut Window, cx: &mut Context<Self>) {
    self
      .popover
      .update(cx, |popover, cx| popover.dismiss(window, cx))
      .ok();
    self
      .nav
      .update(cx, |space, cx| {
        cx.emit(SpaceEvent::Activate);
        space.open(Route::Watch(login), cx);
      })
      .ok();
  }
}

impl Render for BellPanel {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.entries.is_empty() {
      return div()
        .px_2()
        .text_sm()
        .text_color(theme::muted())
        .child("Aucune chaîne suivie n'est passée en direct depuis l'ouverture de Peeby.")
        .into_any_element();
    }
    let now = SystemTime::now();
    let mut list = v_flex()
      .id("twitch-bell-list")
      .max_h(px(560.))
      .overflow_y_scroll()
      .gap_0p5();
    for (i, entry) in self.entries.iter().enumerate() {
      let avatar = self
        .avatars
        .get(&entry.user_id)
        .map(|url| picture::avatar(&cards::template(url), AVATAR, window, cx));
      let login = entry.login.clone();
      let line = [entry.category.clone(), ago(entry.at, now)]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
      list = list.child(
        h_flex()
          .id(ElementId::Name(
            format!("twitch-bell-{}", entry.user_id).into(),
          ))
          .w_full()
          .px_2()
          .py_2()
          .gap_3()
          .items_start()
          .rounded(theme::RADIUS)
          .cursor_pointer()
          .hover(|s| s.bg(theme::surface_hover()))
          .on_click(cx.listener(move |this, _, window, cx| this.open(login.clone(), window, cx)))
          .child(
            div()
              .mt(px(16.))
              .size(px(6.))
              .flex_none()
              .rounded_full()
              .when(i < self.unseen, |el| el.bg(theme::accent())),
          )
          .child(
            div()
              .size(px(AVATAR))
              .flex_none()
              .rounded_full()
              .bg(theme::surface_hover())
              .children(avatar),
          )
          .child(
            v_flex()
              .flex_1()
              .min_w_0()
              .gap_1()
              .child(
                div()
                  .text_sm()
                  .child(SharedString::from(format!("{} est en direct", entry.name))),
              )
              .child(
                div()
                  .text_sm()
                  .text_color(theme::muted())
                  .line_clamp(2)
                  .child(entry.title.clone()),
              )
              .child(div().text_xs().text_color(theme::subtle()).child(line)),
          ),
      );
    }
    list.into_any_element()
  }
}

/// "à l'instant", "il y a 5 min", "il y a 2 h".
fn ago(at: SystemTime, now: SystemTime) -> String {
  let minutes = now.duration_since(at).map_or(0, |d| d.as_secs() / 60);
  match minutes {
    0 => "à l'instant".into(),
    1..=59 => format!("il y a {minutes} min"),
    _ => format!("il y a {} h", minutes / 60),
  }
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::*;

  fn stream(id: &str) -> Stream {
    Stream {
      user_id: id.into(),
      user_login: format!("login{id}"),
      user_name: format!("Name{id}"),
      ..Stream::default()
    }
  }

  #[test]
  fn the_first_reading_announces_nothing() {
    let mut bell = Bell::default();
    bell.update(&[stream("1"), stream("2")], SystemTime::UNIX_EPOCH);
    assert_eq!(bell.unseen(), 0);
    assert!(bell.entries().is_empty());
  }

  #[test]
  fn channels_going_live_are_announced_newest_first() {
    let mut bell = Bell::default();
    let t0 = SystemTime::UNIX_EPOCH;
    bell.update(&[stream("1")], t0);
    bell.update(&[stream("1"), stream("2")], t0 + Duration::from_secs(120));
    bell.update(&[stream("2"), stream("3")], t0 + Duration::from_secs(240));
    let logins: Vec<String> = bell.entries().into_iter().map(|e| e.login).collect();
    assert_eq!(logins, ["login3", "login2"]);
    assert_eq!(bell.unseen(), 2);
    bell.see();
    assert_eq!(bell.unseen(), 0);
    // 1 is back on air: announced again.
    bell.update(
      &[stream("1"), stream("2"), stream("3")],
      t0 + Duration::from_secs(360),
    );
    assert_eq!(bell.entries()[0].login, "login1");
    assert_eq!(bell.unseen(), 1);
  }

  #[test]
  fn a_channel_back_on_air_keeps_one_entry() {
    let mut bell = Bell::default();
    let t0 = SystemTime::UNIX_EPOCH;
    bell.update(&[], t0);
    bell.update(&[stream("1")], t0);
    bell.update(&[], t0);
    bell.update(&[stream("1")], t0);
    assert_eq!(bell.entries().len(), 1);
    assert_eq!(bell.unseen(), 1);
  }

  #[test]
  fn ages_read_in_french() {
    let t0 = SystemTime::UNIX_EPOCH;
    assert_eq!(ago(t0, t0 + Duration::from_secs(30)), "à l'instant");
    assert_eq!(ago(t0, t0 + Duration::from_secs(300)), "il y a 5 min");
    assert_eq!(ago(t0, t0 + Duration::from_secs(7300)), "il y a 2 h");
  }
}
