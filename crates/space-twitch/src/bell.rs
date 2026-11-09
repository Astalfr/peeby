//! Twitch's tab of the bell: the account's notification center ([`super::inbox`]) and the
//! followed channels that went live or changed category while Peeby runs, from the sidebar's reading of the followed lives (every two minutes). The first
//! reading only learns who is on air, so that opening Peeby does not announce every live at
//! once. Opening the panel sees them; a click opens the live. Each kind shows in the bell, as a
//! Windows notification with the channel's avatar, both or neither (`[notifications]`,
//! ADR 0020).

use std::collections::{HashMap, VecDeque};
use std::time::SystemTime;

use gpui::prelude::*;
use gpui::{
  AnyElement, App, ClickEvent, Context, Div, ElementId, FontWeight, SharedString, Stateful,
  StyledText, TextRun, WeakEntity, Window, div, px,
};
use gpui_component::popover::PopoverState;
use gpui_component::{h_flex, v_flex};
use twitch::helix::{self, Stream};
use ui::i18n::t;
use ui::images::{self, Images};
use ui::picture;
use ui::{tf, theme};

use super::cards::{self, Avatars};
use super::inbox;
use super::{Route, TwitchSpace};
use peeby_core::desktop::toast::{self, Toast};
use peeby_core::settings::{Delivery, Settings};
use twitch::notifications::Notification;

/// Events kept in the panel.
const KEPT: usize = 30;
const AVATAR: f32 = 40.;
/// Beyond this many Windows notifications from one reading (after a sleep, an outage), a
/// single one sums them up.
const MAX_TOASTS: usize = 3;
/// The avatar's size in a Windows notification, pixels.
const TOAST_AVATAR: u32 = 96;

/// What a followed channel did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
  /// It went live.
  Live,
  /// Its live changed category.
  Category,
}

impl Change {
  /// Where this kind shows.
  pub fn delivery(self, cx: &App) -> Delivery {
    let notifications = &Settings::get(cx).notifications;
    match self {
      Change::Live => notifications.twitch_live,
      Change::Category => notifications.twitch_category,
    }
  }
}

/// A followed channel went live or changed category.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
  pub user_id: String,
  pub login: String,
  pub name: String,
  pub title: String,
  pub category: String,
  pub change: Change,
  /// When Peeby saw it.
  pub at: SystemTime,
}

/// The events seen, newest first, and how many the user has not seen.
#[derive(Default)]
pub struct Bell {
  /// Who was on air at the last reading, with their category's id; `None` before the first.
  known: Option<HashMap<String, String>>,
  entries: VecDeque<Event>,
  unseen: usize,
}

impl Bell {
  /// A new reading of the followed lives (`live`, every page read) at `now`: what happened
  /// since the last one (nothing on the first). The bell keeps the events whose kind
  /// `in_bell` lets in.
  pub fn update(
    &mut self,
    live: &[Stream],
    now: SystemTime,
    in_bell: impl Fn(Change) -> bool,
  ) -> Vec<Event> {
    let current: HashMap<String, String> = live
      .iter()
      .map(|s| (s.user_id.clone(), s.game_id.clone()))
      .collect();
    let mut events = Vec::new();
    if let Some(known) = &self.known {
      for stream in live {
        let change = match known.get(&stream.user_id) {
          None => Change::Live,
          // A category set, other than the one of the last reading.
          Some(game) if !stream.game_id.is_empty() && *game != stream.game_id => Change::Category,
          Some(_) => continue,
        };
        let event = Event {
          user_id: stream.user_id.clone(),
          login: stream.user_login.clone(),
          name: stream.user_name.clone(),
          title: stream.title.clone(),
          category: stream.game_name.clone(),
          change,
          at: now,
        };
        if in_bell(change) {
          // A channel's newest event replaces its older one.
          self.entries.retain(|e| e.user_id != stream.user_id);
          self.entries.push_front(event.clone());
          self.unseen += 1;
        }
        events.push(event);
      }
      self.entries.truncate(KEPT);
      self.unseen = self.unseen.min(self.entries.len());
    }
    self.known = Some(current);
    events
  }

  /// Events not seen yet.
  pub fn unseen(&self) -> usize {
    self.unseen
  }

  /// Everything is seen.
  pub fn see(&mut self) {
    self.unseen = 0;
  }

  /// The events, newest first.
  pub fn entries(&self) -> Vec<Event> {
    self.entries.iter().cloned().collect()
  }
}

/// Shows `events` (those of a reading) as Windows notifications, for the kinds that ask for
/// them: each with the channel's avatar (from `avatars`), a click opening its live; beyond
/// [`MAX_TOASTS`], one for them all, a click opening the followed channels.
pub fn toasts(events: Vec<Event>, avatars: &Avatars, nav: WeakEntity<TwitchSpace>, cx: &mut App) {
  let events: Vec<Event> = events
    .into_iter()
    .filter(|e| e.change.delivery(cx).windows)
    .collect();
  if events.len() > MAX_TOASTS {
    let names = events
      .iter()
      .map(|e| e.name.as_str())
      .collect::<Vec<_>>()
      .join(", ");
    let toast = Toast {
      title: tf!(
        "{} followed channels are live or changed category",
        "{} chaînes suivies sont en direct ou ont changé de catégorie",
        events.len()
      ),
      body: names,
      image: None,
      preview: None,
    };
    toast::show(toast, move |_, cx| open(&nav, Route::Following, cx), cx);
    return;
  }
  for event in events {
    let avatar = avatars.get(&event.user_id).map(String::as_str);
    let nav = nav.clone();
    let login = event.login.clone();
    show_toast(
      &event,
      avatar,
      move |_, cx| open(&nav, Route::Watch(login), cx),
      cx,
    );
  }
}

/// Shows `event` as a Windows notification, with the channel's `avatar` (a Helix template URL)
/// once it is on disk; a click runs `on_click`.
fn show_toast(
  event: &Event,
  avatar: Option<&str>,
  on_click: impl FnOnce(&mut Window, &mut App) + 'static,
  cx: &mut App,
) {
  let image = avatar.map(|url| {
    images::local_png(
      &helix::sized(url, 150, 150),
      (TOAST_AVATAR, TOAST_AVATAR),
      cx,
    )
  });
  let (title, body) = toast_text(event);
  cx.spawn(async move |cx| {
    let image = match image {
      Some(image) => image.await,
      None => None,
    };
    cx.update(|cx| {
      toast::show(
        Toast {
          title,
          body,
          image,
          preview: None,
        },
        on_click,
        cx,
      )
    });
  })
  .detach();
}

/// The settings' test: `sample` shown as a real notification, with `avatar` (a Helix template
/// URL), whatever the settings say; a click only shows Peeby.
pub fn test_toast(sample: &Event, avatar: Option<&str>, cx: &mut App) {
  show_toast(sample, avatar, |_, _| {}, cx);
}

/// A Windows notification's title and lines.
fn toast_text(event: &Event) -> (String, String) {
  // The category is in the headline.
  (headline(event), event.title.clone())
}

/// "X is live in Y", "X switched to Y".
fn headline(event: &Event) -> String {
  match event.change {
    Change::Live if event.category.is_empty() => {
      tf!("{} is live", "{} est en direct", event.name)
    }
    Change::Live => tf!(
      "{} is live in {}",
      "{} est en direct dans {}",
      event.name,
      event.category
    ),
    Change::Category => tf!(
      "{} switched to {}",
      "{} est passé sur {}",
      event.name,
      event.category
    ),
  }
}

fn open(nav: &WeakEntity<TwitchSpace>, route: Route, cx: &mut App) {
  nav
    .update(cx, |space, cx| space.open_from_notification(route, cx))
    .ok();
}

/// What the Twitch tab of the bell shows.
pub struct Sections {
  /// The followed lives' events, newest first; `None` without the followed channels' connection.
  pub lives: Option<Vec<Event>>,
  /// How many of them are new.
  pub unseen: usize,
  /// The channels' avatars.
  pub avatars: Avatars,
  /// The notification center's notifications, newest first, each with whether it is new.
  pub notifications: Vec<(Notification, bool)>,
}

/// One row of the panel.
enum Item {
  Live {
    event: Event,
    new: bool,
  },
  Twitch {
    notification: Notification,
    new: bool,
  },
}

impl Item {
  fn at(&self) -> SystemTime {
    match self {
      Item::Live { event, .. } => event.at,
      Item::Twitch { notification, .. } => inbox::created(notification),
    }
  }
}

/// The section of the open panel. It lives in the popover's element state: made when the panel
/// opens, with the events as they were (the new ones keep their dot while it is open).
pub fn panel(
  nav: WeakEntity<TwitchSpace>,
  popover: WeakEntity<PopoverState>,
  sections: Sections,
  window: &mut Window,
  cx: &mut App,
) -> AnyElement {
  window
    .use_keyed_state(ElementId::Name("twitch-bell".into()), cx, |_, cx| {
      BellPanel::new(nav, popover, sections, cx)
    })
    .into_any_element()
}

struct BellPanel {
  nav: WeakEntity<TwitchSpace>,
  popover: WeakEntity<PopoverState>,
  /// Newest first, lives and notifications together.
  items: Vec<Item>,
  /// The followed channels are not connected: their lives cannot show.
  no_lives: bool,
  avatars: Avatars,
}

impl BellPanel {
  fn new(
    nav: WeakEntity<TwitchSpace>,
    popover: WeakEntity<PopoverState>,
    sections: Sections,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    // Opening the panel sees the events; the space is being drawn now, so later.
    let see = nav.clone();
    cx.spawn(async move |_, cx| {
      see.update(cx, |space, cx| space.see_bell(cx)).ok();
    })
    .detach();
    let no_lives = sections.lives.is_none();
    let mut items: Vec<Item> = sections
      .lives
      .unwrap_or_default()
      .into_iter()
      .enumerate()
      .map(|(i, event)| Item::Live {
        event,
        new: i < sections.unseen,
      })
      .chain(
        sections
          .notifications
          .into_iter()
          .map(|(notification, new)| Item::Twitch { notification, new }),
      )
      .collect();
    // Newest first; the sort is stable, so each list keeps its own order.
    items.sort_by_key(|item| std::cmp::Reverse(item.at()));
    Self {
      nav,
      popover,
      items,
      no_lives,
      avatars: sections.avatars,
    }
  }

  fn dismiss(&self, window: &mut Window, cx: &mut App) {
    self
      .popover
      .update(cx, |popover, cx| popover.dismiss(window, cx))
      .ok();
  }

  fn open(&mut self, login: String, window: &mut Window, cx: &mut Context<Self>) {
    self.dismiss(window, cx);
    self
      .nav
      .update(cx, |space, cx| {
        space.open_from_notification(Route::Watch(login), cx)
      })
      .ok();
  }

  fn open_notification(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
    let Some(Item::Twitch { notification, .. }) = self.items.get(index) else {
      return;
    };
    if notification.url.is_none() {
      return;
    }
    let notification = notification.clone();
    self.dismiss(window, cx);
    inbox::open(&self.nav, &notification, cx);
  }

  /// A middle click: the page in a new tab, in the background; the panel stays open for the
  /// next ones.
  fn open_in_new_tab(&mut self, route: Route, cx: &mut Context<Self>) {
    self
      .nav
      .update(cx, |space, cx| {
        space.open_from_notification_in_new_tab(route, cx)
      })
      .ok();
  }

  fn live_row(
    &self,
    event: &Event,
    new: bool,
    now: SystemTime,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let avatar = self
      .avatars
      .get(&event.user_id)
      .map(|url| picture::avatar(&cards::template(url), AVATAR, window, cx));
    let login = event.login.clone();
    let in_new_tab = Route::Watch(login.clone());
    // The category is in the headline of a change already.
    let category = match event.change {
      Change::Live => event.category.clone(),
      Change::Category => String::new(),
    };
    let line = [category, ago(event.at, now)]
      .into_iter()
      .filter(|s| !s.is_empty())
      .collect::<Vec<_>>()
      .join(" · ");
    row(
      ElementId::Name(format!("twitch-bell-{}", event.user_id).into()),
      new,
      div()
        .size(px(AVATAR))
        .flex_none()
        .rounded_full()
        .bg(theme::surface_hover())
        .children(avatar)
        .into_any_element(),
      v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(div().text_sm().child(SharedString::from(headline(event))))
        .child(
          div()
            .text_sm()
            .text_color(theme::muted())
            .line_clamp(2)
            .child(event.title.clone()),
        )
        .child(div().text_xs().text_color(theme::subtle()).child(line))
        .into_any_element(),
    )
    .on_click(cx.listener(move |this, _, window, cx| this.open(login.clone(), window, cx)))
    .on_aux_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
      if event.is_middle_click() {
        cx.stop_propagation();
        this.open_in_new_tab(in_new_tab.clone(), cx);
      }
    }))
    .into_any_element()
  }

  fn notification_row(
    &self,
    index: usize,
    notification: &Notification,
    new: bool,
    now: SystemTime,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let round = inbox::is_avatar(notification);
    let radius = if round {
      px(AVATAR / 2.)
    } else {
      theme::RADIUS
    };
    let image = inbox::picture(notification)
      .map(|picture| picture::image_fill(&picture, AVATAR, 1., radius, window, cx));
    let in_new_tab = notification.url.as_deref().and_then(inbox::route);
    // Bold runs as Twitch shows them (a drop's reward), the text wrapping across them.
    let runs: Vec<TextRun> = notification
      .spans
      .iter()
      .map(|span| {
        let mut style = window.text_style();
        if span.bold {
          style.font_weight = FontWeight::SEMIBOLD;
        }
        style.to_run(span.text.len())
      })
      .collect();
    let text = StyledText::new(notification.text()).with_runs(runs);
    let line = [
      t("Twitch", "Twitch").to_owned(),
      ago(inbox::created(notification), now),
    ]
    .join(" · ");
    let clickable = notification.url.is_some();
    row(
      ElementId::Name(format!("twitch-inbox-{}", notification.id).into()),
      new,
      div()
        .size(px(AVATAR))
        .flex_none()
        .rounded(radius)
        .bg(theme::surface_hover())
        .children(image)
        .into_any_element(),
      v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(div().text_sm().line_clamp(3).child(text))
        .child(div().text_xs().text_color(theme::subtle()).child(line))
        .into_any_element(),
    )
    .when(!clickable, |el| el.cursor_default())
    .on_click(cx.listener(move |this, _, window, cx| this.open_notification(index, window, cx)))
    .on_aux_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
      if event.is_middle_click()
        && let Some(route) = in_new_tab.clone()
      {
        cx.stop_propagation();
        this.open_in_new_tab(route, cx);
      }
    }))
    .into_any_element()
  }
}

/// A row of the panel: its dot while new, its picture and its lines.
fn row(id: ElementId, new: bool, picture: AnyElement, lines: AnyElement) -> Stateful<Div> {
  h_flex()
    .id(id)
    .w_full()
    .px_2()
    .py_2()
    .gap_3()
    .items_start()
    .rounded(theme::RADIUS)
    .cursor_pointer()
    .hover(|s| s.bg(theme::surface_hover()))
    .child(
      div()
        .mt(px(16.))
        .size(px(6.))
        .flex_none()
        .rounded_full()
        .when(new, |el| el.bg(theme::accent())),
    )
    .child(picture)
    .child(lines)
}

impl Render for BellPanel {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.items.is_empty() {
      let message = if self.no_lives {
        t(
          "No notification. The followed channels going live show here once they are connected \
           (Following page).",
          "Aucune notification. Les chaînes suivies qui passent en direct s'affichent ici une fois \
           connectées (page Suivies).",
        )
      } else {
        t(
          "No notification, and no followed channel has gone live or changed category since \
           Peeby opened.",
          "Aucune notification, et aucune chaîne suivie n'est passée en direct ni n'a changé de \
           catégorie depuis l'ouverture de Peeby.",
        )
      };
      return div()
        .px_2()
        .pb_2()
        .text_sm()
        .text_color(theme::muted())
        .child(message)
        .into_any_element();
    }
    let now = SystemTime::now();
    let mut list = v_flex()
      .id("twitch-bell-list")
      .max_h(px(560.))
      .overflow_y_scroll()
      .gap_0p5();
    for index in 0..self.items.len() {
      let row = match &self.items[index] {
        Item::Live { event, new } => {
          let (event, new) = (event.clone(), *new);
          self.live_row(&event, new, now, window, cx)
        }
        Item::Twitch { notification, new } => {
          let (notification, new) = (notification.clone(), *new);
          self.notification_row(index, &notification, new, now, window, cx)
        }
      };
      list = list.child(row);
    }
    list.into_any_element()
  }
}

/// "just now", "5 min ago", "2 h ago", "3 d ago".
fn ago(at: SystemTime, now: SystemTime) -> String {
  let minutes = now.duration_since(at).map_or(0, |d| d.as_secs() / 60);
  match minutes {
    0 => t("just now", "à l'instant").into(),
    1..=59 => tf!("{minutes} min ago", "il y a {minutes} min"),
    60..=1439 => {
      let hours = minutes / 60;
      tf!("{hours} h ago", "il y a {hours} h")
    }
    _ => {
      let days = minutes / 1440;
      tf!("{days} d ago", "il y a {days} j")
    }
  }
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::*;

  fn stream(id: &str) -> Stream {
    playing(id, "")
  }

  fn playing(id: &str, game: &str) -> Stream {
    Stream {
      user_id: id.into(),
      user_login: format!("login{id}"),
      user_name: format!("Name{id}"),
      game_id: game.into(),
      game_name: format!("Game{game}"),
      ..Stream::default()
    }
  }

  fn all(_: Change) -> bool {
    true
  }

  #[test]
  fn the_first_reading_announces_nothing() {
    let mut bell = Bell::default();
    let events = bell.update(&[stream("1"), stream("2")], SystemTime::UNIX_EPOCH, all);
    assert!(events.is_empty());
    assert_eq!(bell.unseen(), 0);
    assert!(bell.entries().is_empty());
  }

  #[test]
  fn channels_going_live_are_announced_newest_first() {
    let mut bell = Bell::default();
    let t0 = SystemTime::UNIX_EPOCH;
    bell.update(&[stream("1")], t0, all);
    bell.update(
      &[stream("1"), stream("2")],
      t0 + Duration::from_secs(120),
      all,
    );
    bell.update(
      &[stream("2"), stream("3")],
      t0 + Duration::from_secs(240),
      all,
    );
    let logins: Vec<String> = bell.entries().into_iter().map(|e| e.login).collect();
    assert_eq!(logins, ["login3", "login2"]);
    assert_eq!(bell.unseen(), 2);
    bell.see();
    assert_eq!(bell.unseen(), 0);
    // 1 is back on air: announced again.
    let events = bell.update(
      &[stream("1"), stream("2"), stream("3")],
      t0 + Duration::from_secs(360),
      all,
    );
    assert_eq!(events.len(), 1);
    assert_eq!(bell.entries()[0].login, "login1");
    assert_eq!(bell.unseen(), 1);
  }

  #[test]
  fn a_channel_back_on_air_keeps_one_entry() {
    let mut bell = Bell::default();
    let t0 = SystemTime::UNIX_EPOCH;
    bell.update(&[], t0, all);
    bell.update(&[stream("1")], t0, all);
    bell.update(&[], t0, all);
    bell.update(&[stream("1")], t0, all);
    assert_eq!(bell.entries().len(), 1);
    assert_eq!(bell.unseen(), 1);
  }

  #[test]
  fn a_new_category_is_announced_not_a_cleared_one() {
    let mut bell = Bell::default();
    let t0 = SystemTime::UNIX_EPOCH;
    bell.update(&[playing("1", "10")], t0, all);
    assert!(bell.update(&[playing("1", "10")], t0, all).is_empty());
    let events = bell.update(&[playing("1", "20")], t0, all);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].change, Change::Category);
    assert_eq!(events[0].category, "Game20");
    // No category any more: nothing to say; a category again, from none: a change.
    assert!(bell.update(&[playing("1", "")], t0, all).is_empty());
    let events = bell.update(&[playing("1", "20")], t0, all);
    assert_eq!(events[0].change, Change::Category);
    // The newest event of a channel replaces the older one.
    assert_eq!(bell.entries().len(), 1);
  }

  #[test]
  fn kinds_left_out_of_the_bell_are_still_returned() {
    let mut bell = Bell::default();
    let t0 = SystemTime::UNIX_EPOCH;
    let lives_only = |change| change == Change::Live;
    bell.update(&[playing("1", "10")], t0, lives_only);
    let events = bell.update(&[playing("1", "20"), playing("2", "10")], t0, lives_only);
    assert_eq!(events.len(), 2);
    let logins: Vec<String> = bell.entries().into_iter().map(|e| e.login).collect();
    assert_eq!(logins, ["login2"]);
    assert_eq!(bell.unseen(), 1);
  }

  #[test]
  fn ages() {
    let t0 = SystemTime::UNIX_EPOCH;
    assert_eq!(ago(t0, t0 + Duration::from_secs(30)), "just now");
    assert_eq!(ago(t0, t0 + Duration::from_secs(300)), "5 min ago");
    assert_eq!(ago(t0, t0 + Duration::from_secs(7300)), "2 h ago");
    assert_eq!(
      ago(t0, t0 + Duration::from_secs(3 * 86_400 + 60)),
      "3 d ago"
    );
  }

  #[test]
  fn toast_lines() {
    let mut event = Event {
      user_id: "1".into(),
      login: "a".into(),
      name: "A".into(),
      title: "Title".into(),
      category: "Chess".into(),
      change: Change::Live,
      at: SystemTime::UNIX_EPOCH,
    };
    assert_eq!(
      toast_text(&event),
      ("A is live in Chess".into(), "Title".into())
    );
    event.change = Change::Category;
    assert_eq!(
      toast_text(&event),
      ("A switched to Chess".into(), "Title".into())
    );
  }
}
