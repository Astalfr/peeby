//! The YouTube space: its routes, a stack of pages per tab (it renders the one on top of the tab
//! shown, ADR 0015) and the modules of its pages.

pub mod account;
pub mod bell;
pub mod cards;
pub mod channels;
pub mod chat;
pub mod comments;
pub mod convert;
pub mod feed;
pub mod history;
pub mod inbox;
pub mod playlists;
pub mod prefetch;
pub mod queue;
pub mod rating;
pub mod save;
pub mod search;
pub mod settings_page;
pub mod shorts;
pub mod tools;
pub mod watch;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, AnyView, App, Context, Entity, EntityId, EventEmitter,
  SharedString, WeakEntity, Window, div, ease_out_quint, px,
};
use gpui_component::popover::PopoverState;
use innertube::{Client, Cookies};
use ui::assets::Icon;
use ui::i18n::t;
use ui::images::Images;
use ui::player::PlayerView;
use ui::theme::Accent;
use ui::{runtime, theme};

use crate::settings;
use crate::settings::{Settings, StartPage};
use crate::shell::space::{MenuEntry, Page, Space, SpaceEvent, SpaceInfo, TabId, Tabs};
use account::AccountView;
use channels::ChannelsView;
use feed::{FeedView, Source};
use playlists::{PlaylistView, PlaylistsView};
use queue::Queue;
use search::SearchView;
use shorts::ShortsView;
use watch::WatchView;

/// A page of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
  Home,
  Subscriptions,
  /// YouTube's Shorts, one at a time.
  Shorts,
  /// The account's watch history.
  History,
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
      title: save::watch_later_title().into(),
    }
  }

  /// The first page: the one of `start_page` ("Start page" setting).
  fn start(cx: &App) -> Self {
    match Settings::get(cx).start_page {
      StartPage::Home => Self::Home,
      StartPage::Subscriptions => Self::Subscriptions,
    }
  }

  /// Whether `self` and `other` are the same page. A playlist is its id: the title of "Watch
  /// later" follows the interface's language.
  fn same_page(&self, other: &Self) -> bool {
    match (self, other) {
      (Self::Playlist { id: a, .. }, Self::Playlist { id: b, .. }) => a == b,
      _ => self == other,
    }
  }

  /// Top-level pages (the sidebar's) reset the stack; the others are pushed on it.
  fn is_top_level(&self) -> bool {
    match self {
      Self::Home
      | Self::Subscriptions
      | Self::Shorts
      | Self::History
      | Self::Playlists
      | Self::Search => true,
      Self::Playlist { id, .. } => id == save::WATCH_LATER,
      _ => false,
    }
  }

  /// The tab's label on this page, before a page sets its own (a video's title).
  fn name(&self) -> SharedString {
    match self {
      Self::Home => t("Home", "Accueil").into(),
      Self::Subscriptions => t("Subscriptions", "Abonnements").into(),
      Self::Shorts => t("Shorts", "Shorts").into(),
      Self::History => t("History", "Historique").into(),
      Self::Channels => t("Channels", "Chaînes").into(),
      Self::Channel { name, .. } => name.clone().into(),
      Self::Playlists => t("Playlists", "Playlists").into(),
      Self::Playlist { title, .. } => title.clone().into(),
      Self::Search => t("Search", "Recherche").into(),
      Self::Watch(_) => t("Video", "Vidéo").into(),
    }
  }
}

/// What a YouTube page holds to navigate.
pub type Nav = crate::shell::space::Nav<YouTubeSpace>;

impl Nav {
  /// Opens `route` in the page's tab.
  pub fn open(&self, route: Route, window: &mut Window, cx: &mut App) {
    let tab = self.tab;
    self
      .update(cx, |space, cx| space.open(tab, route, window, cx))
      .ok();
  }

  /// Opens `route` in a new tab after the page's, in the background (a middle click); `label`
  /// names the tab until its page shows.
  pub fn open_in_new_tab(&self, route: Route, label: Option<SharedString>, cx: &mut App) {
    let tab = self.tab;
    self
      .update(cx, |space, cx| space.open_in_new_tab(tab, route, label, cx))
      .ok();
  }

  /// Names the tab while the page of `view` is on top (a video's title, once known).
  pub fn set_title(&self, view: EntityId, title: SharedString, cx: &mut App) {
    self
      .update(cx, |space, cx| {
        if space.tabs.set_title(view, title) {
          cx.notify();
        }
      })
      .ok();
  }
}

pub struct YouTubeSpace {
  client: Client,
  tabs: Tabs<Route>,
  /// The account part of the settings page, made the first time they open.
  account: Option<Entity<AccountView>>,
}

impl YouTubeSpace {
  /// The space with one tab on its start page (`Route::start`), then on `initial_videos` in
  /// order (command-line URLs or ids).
  pub fn new(client: Client, initial_videos: Vec<String>, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    cx.set_global(save::Session(client.clone()));
    save::load_watch_later(cx);
    bell::reset(cx);
    let unseen = inbox::inbox(cx);
    cx.observe(&unseen, |_, _, cx| cx.notify()).detach();
    inbox::reset(cx);
    let mut tabs = Tabs::default();
    let pages = std::iter::once(Route::start(cx))
      .chain(initial_videos.into_iter().map(Route::Watch))
      .collect();
    let first = tabs.add(pages, None);
    tabs.show(first);
    Self {
      client,
      tabs,
      account: None,
    }
  }

  /// Switches to a new session (imported cookies) or signs out (`None`), then puts every tab
  /// back on the start page.
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
    // Every page holds the old client: each tab starts over, rebuilt when it shows.
    let start = Route::start(cx);
    for tab in self.tabs.iter_mut() {
      tab.reset(vec![start.clone()]);
    }
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
          // The tabs are rebuilt with the new client the next time each shows (the settings
          // page is the shell's, so none is on screen meanwhile).
          this.set_session(cookies, cx);
        })
        .ok();
    })
    .detach();
  }

  /// Shows `route` in `tab`: top-level pages reset its stack, the others are pushed. A tab that
  /// has not built its pages yet opens it after them, when it shows.
  pub fn open(&mut self, tab: TabId, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    let Some(route) = self.tabs.queue_if_pending(tab, route) else {
      return cx.notify();
    };
    let Some(stack) = self.tabs.get(tab).map(|t| &t.stack) else {
      return;
    };
    if stack
      .last()
      .is_some_and(|page| page.route.same_page(&route))
    {
      return;
    }
    if route.is_top_level() {
      if let Some(t) = self.tabs.get_mut(tab) {
        t.stack.clear();
      }
    } else {
      // A covered watch page keeps its player (to resume after "back"), paused.
      self.set_top_covered(tab, true, cx);
    }
    let nav = Nav::new(cx.weak_entity(), tab);
    let client = self.client.clone();
    let view: AnyView = match &route {
      Route::Home => cx
        .new(|cx| FeedView::new(client, Source::Home, nav, cx))
        .into(),
      Route::Subscriptions => cx
        .new(|cx| FeedView::new(client, Source::Subscriptions, nav, cx))
        .into(),
      Route::Shorts => cx.new(|cx| ShortsView::new(client, nav, cx)).into(),
      Route::History => cx
        .new(|cx| FeedView::new(client, Source::History, nav, cx))
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
    if let Some(t) = self.tabs.get_mut(tab) {
      t.stack.push(Page::new(route, view));
    }
    cx.notify();
  }

  /// Opens `route` in a new tab after `from`, built when the tab first shows.
  pub fn open_in_new_tab(
    &mut self,
    from: TabId,
    route: Route,
    label: Option<SharedString>,
    cx: &mut Context<Self>,
  ) {
    let tab = self.tabs.add(vec![route], label);
    cx.emit(SpaceEvent::Opened {
      tab,
      after: Some(from),
      focus: false,
    });
    cx.notify();
  }

  /// Opens `route` from a notification: in the tab of the space last shown, else in a new one,
  /// and shows it.
  pub fn open_from_notification(
    &mut self,
    route: Route,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match self.tabs.shown() {
      Some(tab) => {
        cx.emit(SpaceEvent::Activate(tab));
        self.open(tab, route, window, cx);
      }
      None => {
        let tab = self.tabs.add(vec![route], None);
        cx.emit(SpaceEvent::Opened {
          tab,
          after: None,
          focus: true,
        });
      }
    }
  }

  /// Plays the current video of `queue` on a watch page of `tab`: pushed on its stack, or
  /// replacing the watch page on top (`replace`, from one video of the queue to the next).
  /// Replacing drops the previous page and its player, so a long queue keeps a single player,
  /// and "back" still leads to the page the queue was started from.
  pub fn play_queue(
    &mut self,
    tab: TabId,
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
    let Some(stack) = self.tabs.get(tab).map(|t| &t.stack) else {
      return;
    };
    history::offer(&id, resume, cx);
    if replace
      && stack
        .last()
        .is_some_and(|page| matches!(page.route, Route::Watch(_)))
    {
      if let Some(t) = self.tabs.get_mut(tab) {
        t.stack.pop();
      }
    } else {
      self.set_top_covered(tab, true, cx);
    }
    let nav = Nav::new(cx.weak_entity(), tab);
    let client = self.client.clone();
    let route = Route::Watch(id.clone());
    let view = cx.new(|cx| WatchView::new(client, id, Some(queue), nav, window, cx));
    if let Some(t) = self.tabs.get_mut(tab) {
      t.stack.push(Page::new(route, view));
    }
    cx.notify();
  }

  pub fn back(&mut self, tab: TabId, cx: &mut Context<Self>) {
    if let Some(t) = self.tabs.get_mut(tab)
      && t.stack.len() > 1
    {
      t.stack.pop();
      self.set_top_covered(tab, false, cx);
      cx.notify();
    }
  }

  fn top_watch(&self, tab: TabId) -> Option<Entity<WatchView>> {
    self
      .tabs
      .get(tab)?
      .stack
      .last()?
      .view
      .clone()
      .downcast::<WatchView>()
      .ok()
  }

  fn top_shorts(&self, tab: TabId) -> Option<Entity<ShortsView>> {
    self
      .tabs
      .get(tab)?
      .stack
      .last()?
      .view
      .clone()
      .downcast::<ShortsView>()
      .ok()
  }

  /// Pauses the player of the page on top of `tab` while another page covers it.
  fn set_top_covered(&self, tab: TabId, covered: bool, cx: &mut App) {
    if let Some(watch) = self.top_watch(tab) {
      watch.update(cx, |watch, cx| watch.set_covered(covered, cx));
    } else if let Some(shorts) = self.top_shorts(tab) {
      shorts.update(cx, |shorts, cx| shorts.set_covered(covered, cx));
    }
  }

  /// Asks the shell for the settings page (where the account is).
  pub fn open_settings(&mut self, cx: &mut Context<Self>) {
    cx.emit(SpaceEvent::OpenSettings);
  }

  /// The route of a sidebar entry.
  fn entry_route(id: &str) -> Option<Route> {
    Some(match id {
      "home" => Route::Home,
      "subscriptions" => Route::Subscriptions,
      "shorts" => Route::Shorts,
      "history" => Route::History,
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

  fn tabs(&self) -> Vec<TabId> {
    self.tabs.ids()
  }

  fn new_tab(&mut self, cx: &mut Context<Self>) -> TabId {
    self.tabs.add(vec![Route::start(cx)], None)
  }

  fn close_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
    self.tabs.remove(tab);
    cx.notify();
  }

  fn show_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
    self.tabs.show(tab);
    cx.notify();
  }

  fn title(&self, tab: TabId, _cx: &App) -> SharedString {
    self
      .tabs
      .title(tab, Route::name)
      .unwrap_or_else(|| "YouTube".into())
  }

  /// The entry of the page at the bottom of the tab's stack is active, so Subscriptions stays lit
  /// on the channels and channel pages opened from it.
  fn menu(&self, tab: TabId, _cx: &App) -> Vec<MenuEntry> {
    let root = self.tabs.get(tab).and_then(|t| t.root().cloned());
    [
      ("home", t("Home", "Accueil"), Icon::Home, false),
      (
        "subscriptions",
        t("Subscriptions", "Abonnements"),
        Icon::Subscriptions,
        false,
      ),
      ("shorts", t("Shorts", "Shorts"), Icon::Shorts, false),
      ("history", t("History", "Historique"), Icon::History, false),
      (
        "watch-later",
        save::watch_later_title(),
        Icon::WatchLater,
        false,
      ),
      (
        "playlists",
        t("Playlists", "Playlists"),
        Icon::Playlists,
        false,
      ),
    ]
    .into_iter()
    .map(|(id, label, icon, bottom)| MenuEntry {
      active: root
        .as_ref()
        .zip(Self::entry_route(id))
        .is_some_and(|(root, entry)| root.same_page(&entry)),
      bottom,
      ..MenuEntry::new(id, label, icon)
    })
    .collect()
  }

  fn open_entry(&mut self, tab: TabId, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(route) = Self::entry_route(id) {
      self.open(tab, route, window, cx);
    }
  }

  fn open_entry_in_new_tab(&mut self, tab: TabId, id: &str, cx: &mut Context<Self>) {
    if let Some(route) = Self::entry_route(id) {
      self.open_in_new_tab(tab, route, None, cx);
    }
  }

  fn can_go_back(&self, tab: TabId) -> bool {
    self.tabs.get(tab).is_some_and(|t| t.stack.len() > 1)
  }

  fn back(&mut self, tab: TabId, cx: &mut Context<Self>) {
    YouTubeSpace::back(self, tab, cx);
  }

  fn searchable(&self) -> bool {
    true
  }

  fn open_search(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
    self.open(tab, Route::Search, window, cx);
  }

  fn player(&self, tab: TabId, cx: &App) -> Option<Entity<PlayerView>> {
    if let Some(shorts) = self.top_shorts(tab) {
      return shorts.read(cx).player(cx);
    }
    Some(self.top_watch(tab)?.read(cx).player())
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
    let Some(tab) = self.tabs.shown() else {
      return div().size_full().into_any_element();
    };
    // A tab opened in the background, or after a session change: its pages now.
    let pending = self.tabs.take_pending(tab);
    if !pending.is_empty() {
      for route in pending {
        self.open(tab, route, window, cx);
      }
      self.tabs.label_top(tab);
    }
    let Some(page) = self.tabs.get(tab).and_then(|t| t.stack.last()) else {
      return div().size_full().into_any_element();
    };
    let depth = self.tabs.get(tab).map_or(0, |t| t.stack.len());
    let view = div().size_full().relative().child(page.view.clone());
    // A short fade and rise each time a page appears. Not on the watch and Shorts pages: their
    // video surface is an external texture, and leaving fullscreen would replay the fade.
    if matches!(page.route, Route::Watch(_) | Route::Shorts) || window.is_fullscreen() {
      return view.into_any_element();
    }
    view
      .with_animation(
        SharedString::from(format!("page-{tab}-{depth}-{:?}", page.route)),
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
