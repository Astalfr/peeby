//! The YouTube space: its routes, a history of pages per tab (it renders the page shown in the
//! tab shown, ADR 0015, ADR 0019) and the modules of its pages.

mod account;
mod bell;
mod cards;
mod channels;
mod chat;
mod comments;
pub mod convert;
mod feed;
mod history;
mod inbox;
mod lives;
mod playlists;
pub mod prefetch;
mod queue;
mod rating;
mod save;
mod search;
mod settings_page;
mod shorts;
mod subscribe;
pub mod tools;
mod watch;

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
use ui::widgets::Flash;
use ui::{runtime, theme};

use account::AccountView;
use channels::ChannelsView;
use feed::{FeedView, Source};
use peeby_core::settings;
use peeby_core::settings::{Settings, StartPage};
use peeby_core::shell::space::{Lives, MenuEntry, Page, Space, SpaceEvent, SpaceInfo, TabId, Tabs};
use playlists::{Kind, PlaylistView, PlaylistsView};
use queue::Queue;
use search::SearchView;
use shorts::ShortsView;
use watch::WatchView;

/// A page of the app.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
  /// The home feed.
  Home,
  /// The subscriptions feed.
  Subscriptions,
  /// YouTube's Shorts, one at a time.
  Shorts,
  /// The account's watch history.
  History,
  /// The subscribed channels.
  Channels,
  /// A channel's videos.
  Channel {
    /// The channel's id.
    id: String,
    /// Its name, for the tab until the page loads.
    name: String,
  },
  /// The user's playlists.
  Playlists,
  /// The user's mixes.
  Mixes,
  /// A playlist's videos.
  Playlist {
    /// The playlist's id.
    id: String,
    /// Its title, for the tab until the page loads.
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

  /// Pages with a player: a sidebar page opened after them drops their view.
  fn plays(&self) -> bool {
    matches!(self, Self::Watch(_) | Self::Shorts)
  }

  /// Top-level pages (the sidebar's and the search): the sidebar lights the last one up to the
  /// page shown.
  fn is_top_level(&self) -> bool {
    match self {
      Self::Home
      | Self::Subscriptions
      | Self::Shorts
      | Self::History
      | Self::Playlists
      | Self::Mixes
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
      Self::Playlists => Kind::Playlists.title().into(),
      Self::Mixes => Kind::Mixes.title().into(),
      Self::Playlist { title, .. } => title.clone().into(),
      Self::Search => t("Search", "Recherche").into(),
      Self::Watch(_) => t("Video", "Vidéo").into(),
    }
  }
}

/// What a YouTube page holds to navigate.
#[derive(Clone)]
pub struct Nav(peeby_core::shell::space::Nav<YouTubeSpace>);

/// The shell's navigation (the page's tab, the space): a newtype only so that the methods below
/// can be defined here, out of the crate of `peeby_core::shell::space::Nav`.
impl std::ops::Deref for Nav {
  type Target = peeby_core::shell::space::Nav<YouTubeSpace>;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl Nav {
  /// The navigation of a page in `tab`.
  fn new(space: WeakEntity<YouTubeSpace>, tab: TabId) -> Self {
    Self(peeby_core::shell::space::Nav::new(space, tab))
  }

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

/// The space's id in the settings (`SpaceInfo::id`).
pub const ID: &str = "youtube";

/// The YouTube space: the InnerTube client (the session) and the space's tabs.
pub struct YouTubeSpace {
  client: Client,
  tabs: Tabs<Route>,
  /// The account part of the settings page, made the first time they open.
  account: Option<Entity<AccountView>>,
  /// The account's flash in the settings, after a page's "Sign in".
  account_flash: Flash,
  /// The subscribed channels on air, for the notification area's list.
  live: lives::Live,
}

impl YouTubeSpace {
  /// The space with one tab on its start page (`Route::start`), then on `initial_videos` in
  /// order (command-line URLs or ids), shown; without videos, only if the settings open a tab of
  /// the space at startup.
  pub fn new(client: Client, initial_videos: Vec<String>, cx: &mut Context<Self>) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    cx.set_global(save::Session(client.clone()));
    save::load_watch_later(cx);
    bell::reset(cx);
    subscribe::reset(cx);
    let unseen = inbox::inbox(cx);
    cx.observe(&unseen, |_, _, cx| cx.notify()).detach();
    inbox::reset(cx.weak_entity(), cx);
    let mut tabs = Tabs::default();
    let videos = !initial_videos.is_empty();
    if videos || Settings::get(cx).opens_at_startup(ID) {
      let pages = std::iter::once(Route::start(cx))
        .chain(initial_videos.into_iter().map(Route::Watch))
        .collect();
      let first = tabs.add(pages, None);
      tabs.remember(first);
      if videos {
        // Shown even after the tabs of other spaces; after the shell has subscribed to the
        // space's events.
        cx.spawn(async move |this, cx| {
          this
            .update(cx, |_, cx| cx.emit(SpaceEvent::Activate(first)))
            .ok();
        })
        .detach();
      }
    }
    Self {
      client,
      tabs,
      account: None,
      account_flash: Flash::default(),
      live: lives::Live::default(),
    }
  }

  /// Switches to a new session (imported cookies) or signs out (`None`), then puts every tab
  /// back on the start page.
  pub fn set_session(&mut self, cookies: Option<Cookies>, cx: &mut Context<Self>) {
    let Settings {
      language,
      region,
      original_titles,
      ..
    } = Settings::get(cx).clone();
    let client = match Client::new(cookies, Some(settings::cookie_saver()), &language, &region) {
      Ok(client) => client.with_original_titles(original_titles),
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
    subscribe::reset(cx);
    inbox::reset(cx.weak_entity(), cx);
    self.client = client;
    self.live = lives::Live::default();
    // Every page holds the old client: each tab starts over, rebuilt when it shows.
    let start = Route::start(cx);
    for tab in self.tabs.iter_mut() {
      tab.reset(vec![start.clone()]);
    }
    cx.notify();
  }

  /// Rebuilds the clients after a language or title change: the session from the cookie file, and the
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

  /// Opens `route` in `tab`, after the page shown (which is covered: a watch page pauses). A tab
  /// that has not shown yet builds it when it shows, so a tab in the background plays nothing.
  /// A sidebar page drops the paused players behind it.
  pub fn open(&mut self, tab: TabId, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    let Some(t) = self.tabs.get(tab) else {
      return;
    };
    if t.current().is_some_and(|page| page.route.same_page(&route)) {
      return;
    }
    let live = t.live();
    self.cover(tab, true, cx);
    let top_level = route.is_top_level();
    let page = if live {
      let view = self.build(tab, &route, window, cx);
      Page::new(route, view)
    } else {
      Page::unbuilt(route)
    };
    if let Some(t) = self.tabs.get_mut(tab) {
      t.push(page);
      if top_level {
        t.unload_behind(Route::plays);
      }
    }
    cx.notify();
  }

  /// The view of `route` in `tab`.
  fn build(
    &self,
    tab: TabId,
    route: &Route,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> AnyView {
    let nav = Nav::new(cx.weak_entity(), tab);
    let client = self.client.clone();
    match route {
      Route::Home => cx
        .new(|cx| FeedView::new(client, Source::Home, nav, window, cx))
        .into(),
      Route::Subscriptions => cx
        .new(|cx| FeedView::new(client, Source::Subscriptions, nav, window, cx))
        .into(),
      Route::Shorts => cx.new(|cx| ShortsView::new(client, nav, cx)).into(),
      Route::History => cx
        .new(|cx| FeedView::new(client, Source::History, nav, window, cx))
        .into(),
      Route::Channels => cx
        .new(|cx| ChannelsView::new(client, nav, window, cx))
        .into(),
      Route::Channel { id, name } => {
        let source = Source::Channel {
          id: id.clone(),
          name: name.clone(),
        };
        cx.new(|cx| FeedView::new(client, source, nav, window, cx))
          .into()
      }
      Route::Playlists => cx
        .new(|cx| PlaylistsView::new(client, Kind::Playlists, nav, cx))
        .into(),
      Route::Mixes => cx
        .new(|cx| PlaylistsView::new(client, Kind::Mixes, nav, cx))
        .into(),
      Route::Playlist { id, title } => cx
        .new(|cx| PlaylistView::new(client, id.clone(), title.clone(), nav, cx))
        .into(),
      Route::Search => cx.new(|cx| SearchView::new(client, nav, window, cx)).into(),
      Route::Watch(id) => cx
        .new(|cx| WatchView::new(client, id.clone(), None, nav, window, cx))
        .into(),
    }
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

  /// Opens `route` from a notification: in the space's tab shown in `window` (the window used
  /// last), else in its tab last shown in any window, else in a new one, and shows it.
  pub fn open_from_notification(
    &mut self,
    route: Route,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    // Not over the settings' tab: a tab of its own then.
    let in_window = self.tabs.shown_in(window.window_handle().window_id());
    match in_window
      .or(self.tabs.shown())
      .filter(|&tab| !self.settings_shown(tab))
    {
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

  /// A notification's middle click: `route` in a new tab after the tab shown, in the
  /// background (built when it first shows).
  pub fn open_from_notification_in_new_tab(&mut self, route: Route, cx: &mut Context<Self>) {
    let tab = self.tabs.add(vec![route], None);
    cx.emit(SpaceEvent::Opened {
      tab,
      after: None,
      focus: false,
    });
    cx.notify();
  }

  /// Plays the current video of `queue` on a watch page of `tab`: opened after the page shown,
  /// or replacing the watch page shown (`replace`, from one video of the queue to the next).
  /// Replacing drops the previous page and its player, so a long queue keeps a single player,
  /// and "back" still leads to the page the queue was started from. Built at once: the queue
  /// goes on in the mini-player.
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
    let Some(t) = self.tabs.get(tab) else {
      return;
    };
    history::offer(&id, resume, cx);
    // The watch page shown, or under the settings (the queue plays on in the mini-player).
    let replace = replace
      && t
        .page()
        .is_some_and(|page| matches!(page.route, Route::Watch(_)));
    if !replace {
      self.cover(tab, true, cx);
    }
    // The page replaced goes first, with its player.
    let route = Route::Watch(id.clone());
    if let Some(t) = self.tabs.get_mut(tab) {
      if replace {
        t.replace(Page::unbuilt(route));
      } else {
        t.push(Page::unbuilt(route));
      }
    }
    let nav = Nav::new(cx.weak_entity(), tab);
    let client = self.client.clone();
    let view = cx.new(|cx| WatchView::new(client, id, Some(queue), nav, window, cx));
    if let Some(t) = self.tabs.get_mut(tab) {
      t.set_view(view.into());
    }
    cx.notify();
  }

  fn top_watch(&self, tab: TabId) -> Option<Entity<WatchView>> {
    self.page_view(tab)?.downcast::<WatchView>().ok()
  }

  fn top_shorts(&self, tab: TabId) -> Option<Entity<ShortsView>> {
    self.page_view(tab)?.downcast::<ShortsView>().ok()
  }

  /// The view of the page `tab` shows, or shows the settings over.
  fn page_view(&self, tab: TabId) -> Option<AnyView> {
    self.tabs.get(tab)?.page()?.view.clone()
  }

  /// A page's "Sign in": the settings page, scrolled to the account, which flashes.
  pub fn sign_in(&mut self, cx: &mut Context<Self>) {
    self.account_flash.start();
    cx.emit(SpaceEvent::OpenSettings { reveal: true });
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
      "mixes" => Route::Mixes,
      _ => return None,
    })
  }
}

impl EventEmitter<SpaceEvent> for YouTubeSpace {}

impl Space for YouTubeSpace {
  type Route = Route;

  fn info(&self) -> SpaceInfo {
    SpaceInfo {
      id: ID,
      name: "YouTube".into(),
      icon: include_bytes!("icon.svg"),
      accent: Accent::red(),
    }
  }

  fn history(&self) -> &Tabs<Route> {
    &self.tabs
  }

  fn history_mut(&mut self) -> &mut Tabs<Route> {
    &mut self.tabs
  }

  /// A covered watch page keeps its player (to resume after "back"), paused.
  fn cover(&self, tab: TabId, covered: bool, cx: &mut Context<Self>) {
    if let Some(watch) = self.top_watch(tab) {
      watch.update(cx, |watch, cx| watch.set_covered(covered, cx));
    } else if let Some(shorts) = self.top_shorts(tab) {
      shorts.update(cx, |shorts, cx| shorts.set_covered(covered, cx));
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

  fn title(&self, tab: TabId, _cx: &App) -> SharedString {
    self
      .tabs
      .title(tab, Route::name)
      .unwrap_or_else(|| "YouTube".into())
  }

  /// The entry of the last sidebar page up to the one shown is active, so Subscriptions stays
  /// lit on the channels and channel pages opened from it.
  fn menu(&self, tab: TabId, _cx: &App) -> Vec<MenuEntry> {
    let root = self
      .tabs
      .get(tab)
      .and_then(|t| t.section(Route::is_top_level).cloned());
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
      ("playlists", Kind::Playlists.title(), Icon::Playlists, false),
      ("mixes", Kind::Mixes.title(), Icon::Mix, false),
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

  fn search_route(&self) -> Option<Route> {
    Some(Route::Search)
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
    Some(settings_page::part(account, self.account_flash, window, cx))
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

  fn lives(&self, _cx: &App) -> Option<Lives> {
    Some(self.live.lives(self.client.signed_in()))
  }

  fn refresh_lives(&mut self, cx: &mut Context<Self>) {
    self.live.refresh(&self.client, cx);
  }

  fn open_live(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    self.open_from_notification(Route::Watch(id.to_owned()), window, cx);
  }
}

impl Render for YouTubeSpace {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // Each window renders its own tab of the space (ADR 0028).
    let Some(tab) = self.tabs.shown_in(window.window_handle().window_id()) else {
      return div().size_full().into_any_element();
    };
    // A tab opened in the background, a page reached with "back" or "forward", a session
    // change: the page shown is built now.
    if let Some(route) = self.tabs.get(tab).and_then(|t| t.unbuilt()).cloned() {
      let view = self.build(tab, &route, window, cx);
      if let Some(t) = self.tabs.get_mut(tab) {
        t.set_view(view);
      }
    }
    let Some(t) = self.tabs.get(tab) else {
      return div().size_full().into_any_element();
    };
    let Some((page, view)) = t
      .current()
      .and_then(|page| Some((page, page.view.clone()?)))
    else {
      return div().size_full().into_any_element();
    };
    let depth = t.position();
    let view = div().size_full().relative().child(view);
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
