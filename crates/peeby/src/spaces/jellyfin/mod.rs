//! The Jellyfin space, from the former peeby (ADR 0007, 0009): the login page while signed out;
//! then home, libraries, a title's page, search and playback in the content panel, as a history
//! of pages per tab (it renders the page shown in the tab shown, ADR 0015, ADR 0019); its part
//! of the settings page.

pub mod detail;
pub mod home;
pub mod items;
pub mod library;
pub mod login;
pub mod play;
pub mod search;
pub mod session;
pub mod settings_page;
pub mod tracks;

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AnyElement, AnyView, App, Context, Entity, EntityId, EventEmitter,
  SharedString, Subscription, Window, div, ease_out_quint, px, rgb,
};
use jellyfin_api::models::{BaseItem, CollectionType, User};
use jellyfin_api::{Client, Error, ItemId};
use ui::assets::Icon;
use ui::i18n::t;
use ui::images::Images;
use ui::picture::Picture;
use ui::player::PlayerView;
use ui::theme::Accent;
use ui::{runtime, theme};

use crate::settings::Settings;
use crate::shell::space::{Avatar, MenuEntry, Page, Space, SpaceEvent, SpaceInfo, TabId, Tabs};
use detail::DetailView;
use home::HomeView;
use library::LibraryView;
use login::{LoggedIn, LoginView};
use play::PlayView;
use search::SearchView;
use settings_page::SettingsView;
use tracks::TrackPrefs;

/// A page of the space.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
  Home,
  Search,
  Library(ItemId),
  /// A movie's or a series' page; for a series, the season to show first.
  Item {
    id: ItemId,
    season: Option<ItemId>,
  },
  /// Playback of a movie or an episode.
  Play(ItemId),
}

impl Route {
  /// Pages of the sidebar and the search: the sidebar lights the last one up to the page shown.
  fn is_top_level(&self) -> bool {
    !matches!(self, Self::Item { .. } | Self::Play(_))
  }
}

/// What a Jellyfin page holds to navigate.
pub type Nav = crate::shell::space::Nav<JellyfinSpace>;

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

  /// Names the tab while the page of `view` is on top (a title's name, once loaded).
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

/// The signed-in part: the client and what the sidebar shows.
struct Signed {
  client: Client,
  libraries: Vec<BaseItem>,
  user: Option<User>,
}

pub struct JellyfinSpace {
  signed: Option<Signed>,
  /// The login page while signed out; built at the next render (it needs the window).
  login: Option<Entity<LoginView>>,
  /// Why the login page shows again (the session expired).
  login_message: Option<SharedString>,
  tabs: Tabs<Route>,
  /// Its part of the settings page, made the first time it shows in a session.
  settings: Option<Entity<SettingsView>>,
  /// Audio and subtitle choices per series, for the rest of the session.
  track_prefs: HashMap<ItemId, TrackPrefs>,
  /// The player went fullscreen by itself: leave fullscreen once another page shows.
  leave_fullscreen: bool,
  _login_subscription: Option<Subscription>,
}

impl JellyfinSpace {
  /// Jellyfin's blue, the end of its icon's gradient.
  pub fn accent() -> Accent {
    Accent {
      base: rgb(0x00a4dc).into(),
      hover: rgb(0x33b6e3).into(),
      active: rgb(0x0090c2).into(),
    }
  }

  /// The space with one tab, on the saved session (`client`, restored before the UI started),
  /// else on the login page.
  pub fn new(client: Option<Client>, cx: &mut Context<Self>) -> Self {
    // Avatars and the pages' images arrive through the image store.
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |this, _, cx| {
        if this.signed.is_some() {
          cx.notify();
        }
      })
      .detach();
    }
    let mut space = Self {
      signed: None,
      login: None,
      login_message: None,
      tabs: Tabs::default(),
      settings: None,
      track_prefs: HashMap::new(),
      leave_fullscreen: false,
      _login_subscription: None,
    };
    let first = space.tabs.add(Vec::new(), None);
    space.tabs.show(first);
    if let Some(client) = client {
      space.sign_in(client, cx);
    }
    space
  }

  /// The signed-in client.
  pub fn client(&self) -> Option<&Client> {
    self.signed.as_ref().map(|s| &s.client)
  }

  /// Opens the session of `client`: its images go through it, then every tab on Home.
  fn sign_in(&mut self, client: Client, cx: &mut Context<Self>) {
    session::route_images(client.base_url().as_str(), Some(&client), cx);
    self.signed = Some(Signed {
      client,
      libraries: Vec::new(),
      user: None,
    });
    self.login = None;
    self.login_message = None;
    self._login_subscription = None;
    for tab in self.tabs.iter_mut() {
      tab.reset(vec![Route::Home]);
    }
    cx.notify();
    self.load_libraries(cx);
    self.load_user(cx);
  }

  fn on_logged_in(&mut self, client: Client, cx: &mut Context<Self>) {
    tracing::info!("signed in to Jellyfin at {}", client.base_url());
    session::save(&client, cx);
    self.sign_in(client, cx);
  }

  /// Leaves the session: the pages (and a playing video, which reports it stopped) go, and the
  /// login page shows at the next render. `message` says why (session expired).
  fn sign_out(&mut self, message: Option<SharedString>, cx: &mut Context<Self>) {
    let Some(signed) = self.signed.take() else {
      return;
    };
    session::route_images(signed.client.base_url().as_str(), None, cx);
    for tab in self.tabs.iter_mut() {
      tab.reset(Vec::new());
    }
    self.settings = None;
    self.track_prefs.clear();
    self.login_message = message;
    session::forget(cx);
    cx.notify();
  }

  /// "Se déconnecter": tells the server, then signs out here even if it cannot be reached.
  pub fn log_out(&mut self, cx: &mut Context<Self>) {
    if let Some(mut client) = self.client().cloned() {
      drop(runtime::spawn(cx, async move {
        if let Err(e) = client.logout().await {
          tracing::warn!("Jellyfin sign-out: {e}");
        }
      }));
    }
    self.sign_out(None, cx);
  }

  /// Reacts to an API error of any page: an expired session sends the space back to the login
  /// page, the rest is logged.
  pub fn handle_error(&mut self, error: &Error, cx: &mut Context<Self>) {
    if matches!(error, Error::Status { status: 401, .. }) {
      self.sign_out(
        Some(
          t(
            "Your session has expired. Sign in again.",
            "Votre session a expiré. Reconnectez-vous.",
          )
          .into(),
        ),
        cx,
      );
    } else {
      tracing::warn!("Jellyfin: {error}");
    }
  }

  fn load_libraries(&mut self, cx: &mut Context<Self>) {
    let Some(client) = self.client().cloned() else {
      return;
    };
    let fetch = runtime::spawn(cx, async move { client.user_views().await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| match result {
          Some(Ok(views)) => {
            if let Some(signed) = &mut this.signed {
              signed.libraries = views.items;
            }
            cx.notify();
          }
          Some(Err(e)) => this.handle_error(&e, cx),
          None => {}
        })
        .ok();
    })
    .detach();
  }

  fn load_user(&mut self, cx: &mut Context<Self>) {
    let Some(client) = self.client().cloned() else {
      return;
    };
    let fetch = runtime::spawn(cx, async move { client.current_user().await });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| match result {
          Some(Ok(user)) => {
            if let Some(signed) = &mut this.signed {
              signed.user = Some(user);
            }
            cx.notify();
          }
          Some(Err(e)) => this.handle_error(&e, cx),
          None => {}
        })
        .ok();
    })
    .detach();
  }

  /// The video libraries, in the user's order.
  pub fn video_libraries(&self) -> impl Iterator<Item = &BaseItem> {
    self
      .signed
      .iter()
      .flat_map(|s| &s.libraries)
      .filter(|l| l.collection_type.is_some_and(CollectionType::is_video))
  }

  /// The signed-in user's name, once known.
  pub fn user_name(&self) -> Option<String> {
    self.signed.as_ref()?.user.as_ref()?.name.clone()
  }

  /// Opens `route` in `tab`, after the page shown. A tab that has not shown yet builds it when
  /// it shows. A player left behind goes (it reports the playback stopped).
  pub fn open(&mut self, tab: TabId, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    if self
      .tabs
      .get(tab)
      .is_none_or(|t| t.current().is_some_and(|p| p.route == route))
    {
      return;
    }
    let Some(page) = self.page(tab, route, window, cx) else {
      return;
    };
    if let Some(t) = self.tabs.get_mut(tab) {
      t.push(page);
      t.unload_behind(|route| matches!(route, Route::Play(_)));
    }
    cx.notify();
  }

  /// The page of `route` in `tab`: built at once in a tab that has shown, else when it shows;
  /// `None` signed out.
  fn page(
    &self,
    tab: TabId,
    route: Route,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<Page<Route>> {
    self.signed.as_ref()?;
    if !self.tabs.get(tab)?.live() {
      return Some(Page::unbuilt(route));
    }
    let view = self.build(tab, &route, window, cx)?;
    Some(Page::new(route, view))
  }

  /// The view of `route` in `tab`; `None` signed out.
  fn build(
    &self,
    tab: TabId,
    route: &Route,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyView> {
    let client = self.client().cloned()?;
    let nav = Nav::new(cx.weak_entity(), tab);
    Some(match route {
      Route::Home => cx.new(|cx| HomeView::new(client, nav, cx)).into(),
      Route::Search => cx.new(|cx| SearchView::new(client, nav, window, cx)).into(),
      Route::Library(id) => {
        let library = self.video_libraries().find(|l| &l.id == id).cloned();
        cx.new(|cx| LibraryView::new(client, nav, id.clone(), library, cx))
          .into()
      }
      Route::Item { id, season } => cx
        .new(|cx| DetailView::new(client, nav, id.clone(), season.clone(), cx))
        .into(),
      Route::Play(id) => cx
        .new(|cx| PlayView::new(client, nav, id.clone(), window, cx))
        .into(),
    })
  }

  /// Opens `route` in a new tab after `from`, built when the tab first shows.
  pub fn open_in_new_tab(
    &mut self,
    from: TabId,
    route: Route,
    label: Option<SharedString>,
    cx: &mut Context<Self>,
  ) {
    if self.signed.is_none() {
      return;
    }
    let tab = self.tabs.add(vec![route], label);
    cx.emit(SpaceEvent::Opened {
      tab,
      after: Some(from),
      focus: false,
    });
    cx.notify();
  }

  /// Replaces the page shown in `tab` (or under the settings), e.g. the player of an episode by
  /// the next one's, built at once: it plays on in the mini-player.
  /// The page left goes first: its player reports the playback stopped before the next starts.
  pub fn replace(&mut self, tab: TabId, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    if self.signed.is_none() {
      return;
    }
    let Some(t) = self.tabs.get_mut(tab) else {
      return;
    };
    let live = t.live();
    t.replace(Page::unbuilt(route.clone()));
    if live
      && let Some(view) = self.build(tab, &route, window, cx)
      && let Some(t) = self.tabs.get_mut(tab)
    {
      t.set_view(view);
    }
    cx.notify();
  }

  /// Reloads the page under the player of `tab` (watched state, progress, "À suivre"), keeping
  /// what it shows until the new data arrives.
  pub fn refresh(&mut self, tab: TabId, cx: &mut Context<Self>) {
    let Some(view) = self.tabs.get(tab).and_then(|t| {
      t.behind()
        .find(|p| !matches!(p.route, Route::Play(_)))
        .and_then(|p| p.view.clone())
    }) else {
      return;
    };
    if let Ok(home) = view.clone().downcast::<HomeView>() {
      home.update(cx, |home, cx| home.reload(cx));
    } else if let Ok(detail) = view.clone().downcast::<DetailView>() {
      detail.update(cx, |detail, cx| detail.reload(cx));
    } else if let Ok(library) = view.clone().downcast::<LibraryView>() {
      library.update(cx, |library, cx| library.reload(cx));
    } else if let Ok(search) = view.downcast::<SearchView>() {
      search.update(cx, |search, cx| search.reload(cx));
    }
  }

  /// The tab's label on `route`, before a page sets its own (a title's name).
  fn route_name(&self, route: &Route) -> SharedString {
    match route {
      Route::Home => t("Home", "Accueil").into(),
      Route::Search => t("Search", "Recherche").into(),
      Route::Library(id) => self
        .video_libraries()
        .find(|l| &l.id == id)
        .and_then(|l| l.name.clone())
        .unwrap_or_else(|| t("Library", "Bibliothèque").into())
        .into(),
      Route::Item { .. } | Route::Play(_) => "Jellyfin".into(),
    }
  }

  /// The route of a sidebar entry.
  fn entry_route(id: &str) -> Option<Route> {
    match id {
      "home" => Some(Route::Home),
      _ => id
        .strip_prefix("library:")
        .map(|library| Route::Library(library.into())),
    }
  }

  /// The player of the page `tab` shows, or shows the settings over.
  fn top_play(&self, tab: TabId) -> Option<Entity<PlayView>> {
    self
      .tabs
      .get(tab)?
      .page()?
      .view
      .clone()?
      .downcast::<PlayView>()
      .ok()
  }

  /// Track choices last made in a series.
  pub fn track_prefs(&self, series: &ItemId) -> Option<&TrackPrefs> {
    self.track_prefs.get(series)
  }

  pub fn remember_tracks(&mut self, series: ItemId, prefs: TrackPrefs) {
    self.track_prefs.insert(series, prefs);
  }

  /// Asks to leave fullscreen when the next page that is not the player shows.
  pub fn leave_fullscreen(&mut self) {
    self.leave_fullscreen = true;
  }

  fn render_login(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<LoginView> {
    if let Some(login) = &self.login {
      return login.clone();
    }
    let settings = Settings::get(cx).jellyfin.clone();
    let info = session::client_info(&settings.device_id);
    let message = self.login_message.take();
    let login = cx.new(|cx| LoginView::new(info, settings.server_url, message, window, cx));
    self._login_subscription = Some(cx.subscribe_in(
      &login,
      window,
      |this, _, LoggedIn(client), _, cx| this.on_logged_in(client.clone(), cx),
    ));
    self.login = Some(login.clone());
    login
  }
}

/// Shorthand for pages: hands an API error to the space.
pub fn report(nav: &gpui::WeakEntity<JellyfinSpace>, error: &Error, cx: &mut App) {
  nav
    .update(cx, |space, cx| space.handle_error(error, cx))
    .ok();
}

impl EventEmitter<SpaceEvent> for JellyfinSpace {}

impl Space for JellyfinSpace {
  type Route = Route;

  fn info(&self) -> SpaceInfo {
    SpaceInfo {
      name: "Jellyfin".into(),
      icon: include_bytes!("icon.svg"),
      accent: Self::accent(),
    }
  }

  fn history(&self) -> &Tabs<Route> {
    &self.tabs
  }

  fn history_mut(&mut self) -> &mut Tabs<Route> {
    &mut self.tabs
  }

  fn tabs(&self) -> Vec<TabId> {
    self.tabs.ids()
  }

  fn new_tab(&mut self, _cx: &mut Context<Self>) -> TabId {
    let pages = match self.signed {
      Some(_) => vec![Route::Home],
      None => Vec::new(),
    };
    self.tabs.add(pages, None)
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
      .title(tab, |route| self.route_name(route))
      .unwrap_or_else(|| "Jellyfin".into())
  }

  /// Home, the video libraries, then the user (opening the settings) and sign-out at the bottom.
  fn menu(&self, tab: TabId, _cx: &App) -> Vec<MenuEntry> {
    let Some(signed) = &self.signed else {
      return Vec::new();
    };
    let root = self
      .tabs
      .get(tab)
      .and_then(|t| t.section(Route::is_top_level).cloned());
    let active = |id: &str| root.is_some() && root == Self::entry_route(id);
    let mut entries = vec![MenuEntry {
      active: active("home"),
      ..MenuEntry::new("home", t("Home", "Accueil"), Icon::Home)
    }];
    for (index, library) in self.video_libraries().enumerate() {
      let id = format!("library:{}", library.id);
      let icon = match library.collection_type {
        Some(CollectionType::Movies) => Icon::Film,
        Some(CollectionType::TvShows) => Icon::Tv,
        _ => Icon::Folder,
      };
      entries.push(MenuEntry {
        active: active(&id),
        heading: (index == 0).then(|| t("Libraries", "Bibliothèques").into()),
        ..MenuEntry::new(id, library.name.clone().unwrap_or_default(), icon)
      });
    }
    let name = self
      .user_name()
      .unwrap_or_else(|| t("Account", "Compte").into());
    let avatar = signed.user.as_ref().map(|user| Avatar {
      picture: user.primary_image_tag.as_deref().and_then(|tag| {
        let url = signed
          .client
          .user_image_url(&user.id, tag)
          .ok()?
          .to_string();
        Some(Picture::sized(move |_, _| url.clone()))
      }),
      initial: name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string()
        .into(),
    });
    let host = signed
      .client
      .base_url()
      .host_str()
      .map(|h| h.to_owned().into());
    entries.push(MenuEntry {
      bottom: true,
      avatar,
      detail: host,
      ..MenuEntry::new("account", name, Icon::Account)
    });
    entries.push(MenuEntry {
      bottom: true,
      ..MenuEntry::new("sign-out", t("Sign out", "Se déconnecter"), Icon::SignOut)
    });
    entries
  }

  fn open_entry(&mut self, tab: TabId, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    match id {
      "account" => cx.emit(SpaceEvent::OpenSettings),
      "sign-out" => self.log_out(cx),
      _ => {
        if let Some(route) = Self::entry_route(id) {
          self.open(tab, route, window, cx);
        }
      }
    }
  }

  fn open_entry_in_new_tab(&mut self, tab: TabId, id: &str, cx: &mut Context<Self>) {
    if let Some(route) = Self::entry_route(id) {
      self.open_in_new_tab(tab, route, None, cx);
    }
  }

  fn searchable(&self) -> bool {
    self.signed.is_some()
  }

  fn open_search(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
    self.open(tab, Route::Search, window, cx);
  }

  fn settings(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
    let Some(client) = self.client().cloned() else {
      return Some(
        div()
          .text_sm()
          .text_color(theme::muted())
          .child(t(
            "Sign in to your server from the Jellyfin tab.",
            "Connectez-vous à votre serveur depuis l'onglet Jellyfin.",
          ))
          .into_any_element(),
      );
    };
    let nav = cx.weak_entity();
    let view = self
      .settings
      .get_or_insert_with(|| cx.new(|cx| SettingsView::new(client, nav, cx)))
      .clone();
    Some(view.into_any_element())
  }

  fn player(&self, tab: TabId, cx: &App) -> Option<Entity<PlayerView>> {
    Some(self.top_play(tab)?.read(cx).player())
  }
}

impl Render for JellyfinSpace {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    if self.signed.is_none() {
      return div()
        .size_full()
        .child(self.render_login(window, cx))
        .into_any_element();
    }
    let Some(tab) = self.tabs.shown() else {
      return div().size_full().into_any_element();
    };
    // A tab opened in the background, a page reached with "back" or "forward", signing in: the
    // page shown is built now.
    if let Some(route) = self.tabs.get(tab).and_then(|t| t.unbuilt()).cloned()
      && let Some(view) = self.build(tab, &route, window, cx)
      && let Some(t) = self.tabs.get_mut(tab)
    {
      t.set_view(view);
    }
    let playing = self
      .tabs
      .get(tab)
      .and_then(|t| t.current())
      .is_some_and(|p| matches!(p.route, Route::Play(_)));
    if self.leave_fullscreen && !playing {
      self.leave_fullscreen = false;
      if window.is_fullscreen() {
        window.toggle_fullscreen();
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
    // A short fade and rise each time a page appears; not for the player (an external texture,
    // and leaving fullscreen would replay the fade).
    if playing || window.is_fullscreen() {
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
