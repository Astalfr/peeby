//! The YouTube space: its routes, its stack of pages (it renders the one on top) and the
//! modules of its pages.

pub mod account;
pub mod bell;
pub mod cards;
pub mod channels;
pub mod chat;
pub mod convert;
pub mod feed;
pub mod history;
pub mod inbox;
pub mod playlists;
pub mod prefetch;
pub mod queue;
pub mod save;
pub mod search;
pub mod settings_page;
pub mod tools;
pub mod watch;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, AnyView, App, Context, Entity, EventEmitter, SharedString,
  WeakEntity, Window, div, ease_out_quint, px,
};
use gpui_component::popover::PopoverState;
use innertube::{Client, Cookies};
use ui::assets::Icon;
use ui::images::Images;
use ui::player::PlayerView;
use ui::theme::Accent;
use ui::{runtime, theme};

use crate::settings;
use crate::settings::{Settings, StartPage};
use crate::shell::space::{MenuEntry, Space, SpaceEvent, SpaceInfo};
use account::AccountView;
use channels::ChannelsView;
use feed::{FeedView, Source};
use playlists::{PlaylistView, PlaylistsView};
use queue::Queue;
use search::SearchView;
use watch::WatchView;

/// A page of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
  Home,
  Subscriptions,
  /// The subscribed channels.
  Channels,
  /// A channel's videos.
  Channel {
    id: String,
    name: String,
  },
  /// The user's playlists.
  Playlists,
  /// A playlist's videos.
  Playlist {
    id: String,
    title: String,
  },
  /// Search, from the top bar.
  Search,
  /// Watch page of a video id.
  Watch(String),
}

impl Route {
  /// "Watch later", a sidebar page.
  fn watch_later() -> Self {
    Self::Playlist {
      id: save::WATCH_LATER.into(),
      title: save::WATCH_LATER_TITLE.into(),
    }
  }

  /// The first page: the one of `start_page` ("Page d'accueil" setting).
  fn start(cx: &App) -> Self {
    match Settings::get(cx).start_page {
      StartPage::Home => Self::Home,
      StartPage::Subscriptions => Self::Subscriptions,
    }
  }

  /// Top-level pages (the sidebar's) reset the stack; the others are pushed on it.
  fn is_top_level(&self) -> bool {
    match self {
      Self::Home | Self::Subscriptions | Self::Playlists | Self::Search => true,
      Self::Playlist { id, .. } => id == save::WATCH_LATER,
      _ => false,
    }
  }
}

struct Page {
  route: Route,
  view: AnyView,
}

pub struct YouTubeSpace {
  client: Client,
  stack: Vec<Page>,
  /// The account part of the settings page, made the first time they open.
  account: Option<Entity<AccountView>>,
  /// Page to open at the next render (it needs the window), after a session change.
  reset_to: Option<Route>,
}

impl YouTubeSpace {
  /// The app on its start page (`Route::start`), then on `initial_videos` in order (command-line
  /// URLs or ids).
  pub fn new(
    client: Client,
    initial_videos: Vec<String>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    cx.set_global(save::Session(client.clone()));
    save::load_watch_later(cx);
    bell::reset(cx);
    let unseen = inbox::inbox(cx);
    cx.observe(&unseen, |_, _, cx| cx.notify()).detach();
    inbox::reset(cx);
    let mut app = Self {
      client,
      stack: Vec::new(),
      account: None,
      reset_to: None,
    };
    app.open(Route::start(cx), window, cx);
    for id in initial_videos {
      app.open(Route::Watch(id), window, cx);
    }
    app
  }

  /// Switches to a new session (imported cookies) or signs out (`None`), then shows the start
  /// page.
  pub fn set_session(&mut self, cookies: Option<Cookies>, cx: &mut Context<Self>) {
    let Settings {
      language, region, ..
    } = Settings::get(cx).clone();
    let client = match Client::new(cookies, settings::cookies_path(), &language, &region) {
      Ok(client) => client,
      Err(e) => return tracing::error!("cannot create the HTTP client: {e}"),
    };
    tracing::info!(
      "session {}",
      if client.signed_in() {
        "imported"
      } else {
        "closed"
      }
    );
    let warmup = client.clone();
    drop(runtime::spawn(cx, async move {
      if let Err(e) = warmup.refresh_client_version().await {
        tracing::warn!("ytcfg: {e}");
      }
    }));
    cx.set_global(save::Session(client.clone()));
    save::load_watch_later(cx);
    bell::reset(cx);
    inbox::reset(cx);
    self.client = client;
    // Pages hold the old client: they are rebuilt on the next navigation; the start page now.
    self.reset_to = Some(Route::start(cx));
    cx.notify();
  }

  /// Rebuilds the clients after a language change: the session from the cookie file, and the
  /// anonymous one of the stream prefetch (caption names follow the language).
  pub fn reload_session(&mut self, cx: &mut Context<Self>) {
    let Settings {
      language, region, ..
    } = Settings::get(cx).clone();
    match Client::new(None, None, &language, &region) {
      Ok(anonymous) => prefetch::init(anonymous, cx),
      Err(e) => tracing::error!("cannot create the HTTP client: {e}"),
    }
    let load = cx
      .background_executor()
      .spawn(async { settings::load_cookies() });
    cx.spawn(async move |this, cx| {
      let cookies = load.await;
      this
        .update(cx, |this, cx| {
          // Home is rebuilt with the new client the next time the space shows (the settings
          // page is the shell's, so the space is not on screen meanwhile).
          this.set_session(cookies, cx);
        })
        .ok();
    })
    .detach();
  }

  /// Shows `route`: top-level pages reset the stack, the others are pushed.
  pub fn open(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    if self.stack.last().is_some_and(|page| page.route == route) {
      return;
    }
    if route.is_top_level() {
      self.stack.clear();
    } else {
      // A covered watch page keeps its player (to resume after "back"), paused.
      self.set_top_covered(true, cx);
    }
    let nav = cx.weak_entity();
    let client = self.client.clone();
    let view: AnyView = match &route {
      Route::Home => cx
        .new(|cx| FeedView::new(client, Source::Home, nav, cx))
        .into(),
      Route::Subscriptions => cx
        .new(|cx| FeedView::new(client, Source::Subscriptions, nav, cx))
        .into(),
      Route::Channels => cx
        .new(|cx| ChannelsView::new(client, nav, window, cx))
        .into(),
      Route::Channel { id, name } => {
        let source = Source::Channel {
          id: id.clone(),
          name: name.clone(),
        };
        cx.new(|cx| FeedView::new(client, source, nav, cx)).into()
      }
      Route::Playlists => cx.new(|cx| PlaylistsView::new(client, nav, cx)).into(),
      Route::Playlist { id, title } => cx
        .new(|cx| PlaylistView::new(client, id.clone(), title.clone(), nav, cx))
        .into(),
      Route::Search => cx.new(|cx| SearchView::new(client, nav, window, cx)).into(),
      Route::Watch(id) => cx
        .new(|cx| WatchView::new(client, id.clone(), None, nav, window, cx))
        .into(),
    };
    self.stack.push(Page { route, view });
    cx.notify();
  }

  /// Plays the current video of `queue` on a watch page: pushed on the stack, or replacing the
  /// watch page on top (`replace`, from one video of the queue to the next). Replacing drops
  /// the previous page and its player, so a long queue keeps a single player, and "back"
  /// still leads to the page the queue was started from.
  pub fn play_queue(
    &mut self,
    queue: Entity<Queue>,
    replace: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some((id, resume)) = queue
      .read(cx)
      .current_video()
      .map(|v| (v.id.clone(), history::card_resume(v)))
    else {
      return;
    };
    history::offer(&id, resume, cx);
    if replace
      && self
        .stack
        .last()
        .is_some_and(|page| matches!(page.route, Route::Watch(_)))
    {
      self.stack.pop();
    } else {
      self.set_top_covered(true, cx);
    }
    let nav = cx.weak_entity();
    let client = self.client.clone();
    let route = Route::Watch(id.clone());
    let view = cx
      .new(|cx| WatchView::new(client, id, Some(queue), nav, window, cx))
      .into();
    self.stack.push(Page { route, view });
    cx.notify();
  }

  pub fn back(&mut self, cx: &mut Context<Self>) {
    if self.stack.len() > 1 {
      self.stack.pop();
      self.set_top_covered(false, cx);
      cx.notify();
    }
  }

  fn set_top_covered(&self, covered: bool, cx: &mut App) {
    if let Some(watch) = self
      .stack
      .last()
      .and_then(|page| page.view.clone().downcast::<WatchView>().ok())
    {
      watch.update(cx, |watch, cx| watch.set_covered(covered, cx));
    }
  }

  /// Asks the shell for the settings page (where the account is).
  pub fn open_settings(&mut self, cx: &mut Context<Self>) {
    cx.emit(SpaceEvent::OpenSettings);
  }

  /// Asks the shell to show this space (a notification was clicked from another tab).
  pub fn request_focus(&mut self, cx: &mut Context<Self>) {
    cx.emit(SpaceEvent::Activate);
  }

  /// The route of a sidebar entry.
  fn entry_route(id: &str) -> Option<Route> {
    Some(match id {
      "home" => Route::Home,
      "subscriptions" => Route::Subscriptions,
      "watch-later" => Route::watch_later(),
      "playlists" => Route::Playlists,
      _ => return None,
    })
  }
}

impl EventEmitter<SpaceEvent> for YouTubeSpace {}

impl Space for YouTubeSpace {
  fn info(&self) -> SpaceInfo {
    SpaceInfo {
      name: "YouTube".into(),
      icon: include_bytes!("icon.svg"),
      accent: Accent::red(),
    }
  }

  /// The entry of the page at the bottom of the stack is active, so Subscriptions stays lit on
  /// the channels and channel pages opened from it.
  fn menu(&self, _cx: &App) -> Vec<MenuEntry> {
    let root = self.stack.first().map(|page| page.route.clone());
    [
      ("home", "Accueil", Icon::Home, false),
      ("subscriptions", "Abonnements", Icon::Subscriptions, false),
      (
        "watch-later",
        save::WATCH_LATER_TITLE,
        Icon::WatchLater,
        false,
      ),
      ("playlists", "Playlists", Icon::Playlists, false),
    ]
    .into_iter()
    .map(|(id, label, icon, bottom)| MenuEntry {
      active: root.is_some() && Self::entry_route(id) == root,
      bottom,
      ..MenuEntry::new(id, label, icon)
    })
    .collect()
  }

  fn open_entry(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(route) = Self::entry_route(id) {
      self.open(route, window, cx);
    }
  }

  fn can_go_back(&self) -> bool {
    self.stack.len() > 1
  }

  fn back(&mut self, cx: &mut Context<Self>) {
    YouTubeSpace::back(self, cx);
  }

  fn searchable(&self) -> bool {
    true
  }

  fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.open(Route::Search, window, cx);
  }

  fn player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    let watch = self
      .stack
      .last()?
      .view
      .clone()
      .downcast::<WatchView>()
      .ok()?;
    Some(watch.read(cx).player())
  }

  fn unseen(&self, cx: &App) -> Option<usize> {
    inbox::unseen(cx)
  }

  fn settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
    let nav = cx.weak_entity();
    let signed_in = self.client.signed_in();
    let account = self
      .account
      .get_or_insert_with(|| cx.new(|_| AccountView::new(nav, signed_in)))
      .clone();
    Some(settings_page::part(account, window, cx))
  }

  fn notifications(
    &self,
    popover: WeakEntity<PopoverState>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    inbox::unseen(cx)?;
    Some(inbox::panel(cx.weak_entity(), popover, window, cx))
  }
}

impl Render for YouTubeSpace {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if let Some(route) = self.reset_to.take() {
      // Every page holds the old client: start over, even if the start page is on top.
      self.stack.clear();
      self.open(route, window, cx);
    }
    let Some(page) = self.stack.last() else {
      return div().size_full().into_any_element();
    };
    let view = div().size_full().relative().child(page.view.clone());
    // A short fade and rise each time a page appears. Not on the watch page: its video surface
    // is an external texture, and leaving fullscreen would replay the fade.
    if matches!(page.route, Route::Watch(_)) || window.is_fullscreen() {
      return view.into_any_element();
    }
    view
      .with_animation(
        SharedString::from(format!("page-{}-{:?}", self.stack.len(), page.route)),
        Animation::new(theme::NORMAL).with_easing(ease_out_quint()),
        |el, t| el.opacity(t).top(px(8. * (1. - t))),
      )
      .into_any_element()
  }
}

/// Video id of a watch URL (`watch?v=`, `youtu.be/`, `shorts/`) or of a bare 11-character id.
pub fn video_id(text: &str) -> Option<&str> {
  let is_id = |s: &str| {
    s.len() == 11
      && s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
  };
  if is_id(text) {
    return Some(text);
  }
  let rest = text
    .split_once("v=")
    .map(|(_, rest)| rest)
    .or_else(|| text.split_once("youtu.be/").map(|(_, rest)| rest))
    .or_else(|| text.split_once("/shorts/").map(|(_, rest)| rest))?;
  let id = rest.split(['&', '?', '#', '/']).next()?;
  is_id(id).then_some(id)
}

#[cfg(test)]
mod tests {
  use super::video_id;

  #[test]
  fn video_ids() {
    for url in [
      "dQw4w9WgXcQ",
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      "https://www.youtube.com/watch?list=x&v=dQw4w9WgXcQ&t=10",
      "https://youtu.be/dQw4w9WgXcQ?si=abc",
      "https://www.youtube.com/shorts/dQw4w9WgXcQ",
    ] {
      assert_eq!(video_id(url), Some("dQw4w9WgXcQ"), "{url}");
    }
    assert_eq!(video_id("hello"), None);
    assert_eq!(video_id("https://www.youtube.com/"), None);
  }
}
