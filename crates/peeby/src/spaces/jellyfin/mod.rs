//! The Jellyfin space, from the former peeby (ADR 0007, 0009): the login page while signed out;
//! then home, libraries, a title's page, search and playback in the content panel, as a stack of
//! pages (it renders the one on top); its part of the settings page.

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
  Animation, AnimationExt, AnyElement, AnyView, App, Context, Entity, EventEmitter, SharedString,
  Subscription, Window, div, ease_out_quint, px, rgb,
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
use crate::shell::space::{Avatar, MenuEntry, Space, SpaceEvent, SpaceInfo};
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
  /// Pages of the sidebar and the search reset the stack; titles and playback are pushed.
  fn is_top_level(&self) -> bool {
    !matches!(self, Self::Item { .. } | Self::Play(_))
  }
}

struct Page {
  route: Route,
  view: AnyView,
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
  stack: Vec<Page>,
  /// Its part of the settings page, made the first time it shows in a session.
  settings: Option<Entity<SettingsView>>,
  /// Audio and subtitle choices per series, for the rest of the session.
  track_prefs: HashMap<ItemId, TrackPrefs>,
  /// The player went fullscreen by itself: leave fullscreen once another page shows.
  leave_fullscreen: bool,
  _login_subscription: Option<Subscription>,
}

impl JellyfinSpace {
  /// Peeby's amber.
  pub fn accent() -> Accent {
    Accent {
      base: rgb(0xff8a3d).into(),
      hover: rgb(0xff9f5e).into(),
      active: rgb(0xe8752a).into(),
    }
  }

  /// The space on the saved session (`client`, restored before the UI started), else on the
  /// login page.
  pub fn new(client: Option<Client>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
      stack: Vec::new(),
      settings: None,
      track_prefs: HashMap::new(),
      leave_fullscreen: false,
      _login_subscription: None,
    };
    if let Some(client) = client {
      space.sign_in(client, window, cx);
    }
    space
  }

  /// The signed-in client.
  pub fn client(&self) -> Option<&Client> {
    self.signed.as_ref().map(|s| &s.client)
  }

  /// Opens the session of `client`: its images go through it, then Home.
  fn sign_in(&mut self, client: Client, window: &mut Window, cx: &mut Context<Self>) {
    session::route_images(client.base_url().as_str(), Some(&client), cx);
    self.signed = Some(Signed {
      client,
      libraries: Vec::new(),
      user: None,
    });
    self.login = None;
    self.login_message = None;
    self._login_subscription = None;
    self.stack.clear();
    self.open(Route::Home, window, cx);
    self.load_libraries(cx);
    self.load_user(cx);
  }

  fn on_logged_in(&mut self, client: Client, window: &mut Window, cx: &mut Context<Self>) {
    tracing::info!("signed in to Jellyfin at {}", client.base_url());
    session::save(&client, cx);
    self.sign_in(client, window, cx);
  }

  /// Leaves the session: the pages (and a playing video, which reports it stopped) go, and the
  /// login page shows at the next render. `message` says why (session expired).
  fn sign_out(&mut self, message: Option<SharedString>, cx: &mut Context<Self>) {
    let Some(signed) = self.signed.take() else {
      return;
    };
    session::route_images(signed.client.base_url().as_str(), None, cx);
    self.stack.clear();
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

  /// Shows `route`: top-level pages reset the stack, the others are pushed.
  pub fn open(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    let Some(client) = self.client().cloned() else {
      return;
    };
    if self.stack.last().is_some_and(|p| p.route == route) {
      return;
    }
    let nav = cx.weak_entity();
    let view: AnyView = match &route {
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
    };
    if route.is_top_level() {
      self.stack.clear();
    }
    self.stack.push(Page { route, view });
    cx.notify();
  }

  /// Replaces the page on top, e.g. the player of an episode by the next one's.
  pub fn replace(&mut self, route: Route, window: &mut Window, cx: &mut Context<Self>) {
    if self.stack.len() > 1 {
      self.stack.pop();
    }
    self.open(route, window, cx);
  }

  pub fn back(&mut self, cx: &mut Context<Self>) {
    if self.stack.len() > 1 {
      self.stack.pop();
      cx.notify();
    }
  }

  /// Reloads the page under the player (watched state, progress, "À suivre"), keeping what it
  /// shows until the new data arrives.
  pub fn refresh(&mut self, cx: &mut Context<Self>) {
    let Some(page) = self
      .stack
      .iter()
      .rev()
      .find(|p| !matches!(p.route, Route::Play(_)))
    else {
      return;
    };
    let view = page.view.clone();
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

  /// The sidebar entry of `route`.
  fn entry_id(route: &Route) -> Option<String> {
    Some(match route {
      Route::Home => "home".into(),
      Route::Library(id) => format!("library:{id}"),
      _ => return None,
    })
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
      |this, _, LoggedIn(client), window, cx| this.on_logged_in(client.clone(), window, cx),
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
  fn info(&self) -> SpaceInfo {
    SpaceInfo {
      name: "Jellyfin".into(),
      icon: include_bytes!("icon.svg"),
      accent: Self::accent(),
    }
  }

  /// Home, the video libraries, then the user (opening the settings) and sign-out at the bottom.
  fn menu(&self, _cx: &App) -> Vec<MenuEntry> {
    let Some(signed) = &self.signed else {
      return Vec::new();
    };
    let root = self.stack.first().and_then(|p| Self::entry_id(&p.route));
    let active = |id: &str| root.as_deref() == Some(id);
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

  fn open_entry(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
    let route = match id {
      "home" => Route::Home,
      "account" => return cx.emit(SpaceEvent::OpenSettings),
      "sign-out" => return self.log_out(cx),
      _ => match id.strip_prefix("library:") {
        Some(library) => Route::Library(library.into()),
        None => return,
      },
    };
    self.open(route, window, cx);
  }

  fn can_go_back(&self) -> bool {
    self.stack.len() > 1
  }

  fn back(&mut self, cx: &mut Context<Self>) {
    JellyfinSpace::back(self, cx);
  }

  fn searchable(&self) -> bool {
    self.signed.is_some()
  }

  fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.open(Route::Search, window, cx);
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

  fn player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    let play = self
      .stack
      .last()?
      .view
      .clone()
      .downcast::<PlayView>()
      .ok()?;
    Some(play.read(cx).player())
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
    let playing = self
      .stack
      .last()
      .is_some_and(|p| matches!(p.route, Route::Play(_)));
    if self.leave_fullscreen && !playing {
      self.leave_fullscreen = false;
      if window.is_fullscreen() {
        window.toggle_fullscreen();
      }
    }
    let Some(page) = self.stack.last() else {
      return div().size_full().into_any_element();
    };
    let view = div().size_full().relative().child(page.view.clone());
    // A short fade and rise each time a page appears; not for the player (an external texture,
    // and leaving fullscreen would replay the fade).
    if playing || window.is_fullscreen() {
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
