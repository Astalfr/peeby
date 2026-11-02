//! The Twitch space (ADR 0010, docs/twitch-plan.md): nothing without signing in (the user's
//! choice), so its only page while signed out is the sign-in card. Signed in, it keeps the Helix
//! session (refreshed one at a time, validated at startup and every hour, as Twitch requires), the
//! optional web `auth-token` its playback client sends to GQL, the Helix client of its pages, and a
//! stack of pages: "Suivies", "Parcourir" or the search, then categories and lives on top. The
//! sidebar lists the followed channels on air, refreshed every two minutes; those that go live in
//! the meantime show in the bell ([`bell`]).

pub mod bell;
pub mod browse;
pub mod cards;
pub mod channel;
pub mod chat;
pub mod following;
pub mod login;
pub mod play;
pub mod points;
pub mod search;
pub mod session;
pub mod settings_page;
pub mod watch;

use std::time::{Duration, SystemTime};

use gpui::prelude::*;
use gpui::{
  AnyElement, AnyView, App, Context, Entity, EventEmitter, SharedString, Subscription, Task,
  WeakEntity, Window, div, rgb,
};
use gpui_component::popover::PopoverState;
use twitch::Error;
use twitch::auth::{Auth, Session, Validation};
use twitch::helix::{Helix, Stream};
use ui::assets::Icon;
use ui::images::Images;
use ui::player::PlayerView;
use ui::theme::Accent;
use ui::{runtime, theme};

use crate::settings::Settings;
use crate::shell::space::{Avatar, MenuEntry, Space, SpaceEvent, SpaceInfo};
use browse::{BrowseView, CategoryView};
use cards::Avatars;
use channel::ChannelView;
use following::FollowingView;
use login::{LoginView, SignedIn};
use search::SearchView;
use session::Secrets;
use settings_page::SettingsView;
use watch::{Kind, WatchView};

/// Twitch requires validating the token every hour.
const VALIDATE_EVERY: Duration = Duration::from_secs(3600);
/// How often the sidebar's live channels are read again.
const SIDEBAR_EVERY: Duration = Duration::from_secs(120);
/// Live channels listed in the sidebar.
const SIDEBAR_LIVE: usize = 12;

/// A page of the space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
  /// "Suivies": the followed channels.
  Following,
  /// "Parcourir": the top categories.
  Browse,
  /// A category's live streams.
  Category {
    /// Category id.
    id: String,
    /// Its name, for the title.
    name: String,
  },
  /// The search page.
  Search,
  /// A channel's page (its VODs and clips), by login.
  Channel(String),
  /// A channel's live, by login.
  Watch(String),
  /// A VOD, by id.
  Vod(String),
  /// A clip, by slug.
  Clip(String),
}

impl Route {
  /// The page of a Twitch link: `twitch.tv/videos/<id>`, `clips.twitch.tv/<slug>`,
  /// `twitch.tv/<login>/clip/<slug>`, else `twitch.tv/<login>`'s live.
  pub fn from_url(url: &str) -> Option<Self> {
    let (_, rest) = url.split_once("twitch.tv/")?;
    let mut parts = rest
      .split(['?', '#'])
      .next()?
      .split('/')
      .filter(|p| !p.is_empty());
    let first = parts.next()?;
    let second = parts.next();
    let third = parts.next();
    if url.contains("clips.twitch.tv/") {
      return Some(Self::Clip(first.to_owned()));
    }
    match (first, second, third) {
      ("videos", Some(id), _) if id.bytes().all(|b| b.is_ascii_digit()) => {
        Some(Self::Vod(id.to_owned()))
      }
      (_, Some("clip"), Some(slug)) => Some(Self::Clip(slug.to_owned())),
      _ => following::channel_login(first).map(Self::Watch),
    }
  }

  /// Sidebar pages and the search reset the stack; categories and lives are pushed.
  fn is_top_level(&self) -> bool {
    matches!(self, Self::Following | Self::Browse | Self::Search)
  }
}

struct Page {
  route: Route,
  view: AnyView,
}

/// The signed-in part.
struct Signed {
  session: Session,
  helix: Helix,
  login: String,
  user_id: String,
  /// The followed channels on air, for the sidebar, with their avatars.
  live: Vec<Stream>,
  avatars: Avatars,
  /// Who went live since Peeby opened.
  bell: bell::Bell,
  _validate: Task<()>,
  _sidebar: Task<()>,
}

enum State {
  /// A saved session is being restored (its refresh token exchanged, then validated).
  Restoring,
  SignedOut,
  SignedIn(Box<Signed>),
}

pub struct TwitchSpace {
  auth: Auth,
  state: State,
  /// The web `auth-token`, sent to GQL by `client`.
  auth_token: Option<String>,
  /// GQL and usher, for playback and the channels' headers.
  client: Option<twitch::Client>,
  stack: Vec<Page>,
  /// A page asked for on the command line (a live, a VOD, a clip), opened once signed in.
  pending: Option<Route>,
  /// The sign-in card while signed out, made at the next render.
  login: Option<Entity<LoginView>>,
  /// Why the card shows (the session expired, Twitch unreachable).
  login_message: Option<SharedString>,
  /// Its part of the settings page, made the first time it shows.
  settings: Option<Entity<SettingsView>>,
  _login_subscription: Option<Subscription>,
}

impl TwitchSpace {
  /// Twitch's violet.
  pub fn accent() -> Accent {
    Accent {
      base: rgb(0x9146ff).into(),
      hover: rgb(0xa970ff).into(),
      active: rgb(0x772ce8).into(),
    }
  }

  /// The space with the secrets read before the UI started: a saved refresh token is exchanged
  /// in the background. `pending` (from the command line) opens once signed in, and shows the tab.
  pub fn new(auth: Auth, secrets: Secrets, pending: Option<Route>, cx: &mut Context<Self>) -> Self {
    let mut space = Self {
      auth,
      state: State::SignedOut,
      auth_token: None,
      client: None,
      stack: Vec::new(),
      pending,
      login: None,
      login_message: None,
      settings: None,
      _login_subscription: None,
    };
    space.set_auth_token(secrets.auth_token, cx);
    // The sidebar's avatars arrive through the image store.
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |this, _, cx| {
        if matches!(this.state, State::SignedIn(_)) {
          cx.notify();
        }
      })
      .detach();
    }
    // Twitch may have changed its web Client-ID: read the current one from its page.
    if let Some(client) = space.client.clone() {
      drop(runtime::spawn(cx, async move {
        if let Err(e) = client.refresh_client_id().await {
          tracing::warn!("Twitch web Client-ID: {e}");
        }
      }));
    }
    if let Some(refresh) = secrets.refresh {
      space.restore(refresh, cx);
    }
    if space.pending.is_some() {
      // After the shell has subscribed to the space's events.
      cx.spawn(async move |this, cx| {
        this.update(cx, |_, cx| cx.emit(SpaceEvent::Activate)).ok();
      })
      .detach();
    }
    space
  }

  fn restore(&mut self, refresh: String, cx: &mut Context<Self>) {
    self.state = State::Restoring;
    let auth = self.auth.clone();
    let restore = runtime::spawn(cx, async move {
      let session = Session::restore(auth, &refresh, session::store()).await?;
      let validation = session.validate().await?;
      Ok::<_, Error>((session, validation))
    });
    cx.spawn(async move |this, cx| {
      let result = restore.await;
      this
        .update(cx, |this, cx| match result {
          Some(Ok((session, validation))) => this.signed_in(session, validation, cx),
          Some(Err(Error::SessionExpired)) => this.sign_out(
            Some("Votre session Twitch a expiré. Reconnectez-vous.".into()),
            cx,
          ),
          Some(Err(e)) => {
            // The refresh token was not spent (Twitch did not answer): it stays for next time.
            tracing::warn!("Twitch session restore: {e}");
            this.state = State::SignedOut;
            this.login_message = Some(login::describe(&e));
            cx.notify();
          }
          None => {}
        })
        .ok();
    })
    .detach();
  }

  fn signed_in(&mut self, session: Session, validation: Validation, cx: &mut Context<Self>) {
    Settings::update(cx, |s| s.twitch.login = Some(validation.login.clone()));
    let validate = cx.spawn({
      let session = session.clone();
      async move |this, cx| {
        loop {
          cx.background_executor().timer(VALIDATE_EVERY).await;
          let check = {
            let session = session.clone();
            let Ok(check) = this.update(cx, |_, cx| {
              runtime::spawn(cx, async move { session.validate().await })
            }) else {
              return;
            };
            check
          };
          match check.await {
            Some(Ok(_)) => {}
            Some(Err(Error::SessionExpired)) => {
              this
                .update(cx, |this, cx| {
                  this.sign_out(
                    Some("Votre session Twitch a expiré. Reconnectez-vous.".into()),
                    cx,
                  )
                })
                .ok();
              return;
            }
            // Offline for a moment: the next hour tries again.
            Some(Err(e)) => tracing::warn!("Twitch token validation: {e}"),
            None => return,
          }
        }
      }
    });
    let helix = match Helix::new(session.clone()) {
      Ok(helix) => helix,
      Err(e) => {
        tracing::error!("cannot create the Helix client: {e}");
        return self.sign_out(Some(login::describe(&e)), cx);
      }
    };
    let sidebar = cx.spawn({
      let (helix, user_id) = (helix.clone(), validation.user_id.clone());
      async move |this, cx| {
        loop {
          let (helix, user_id) = (helix.clone(), user_id.clone());
          let Ok(fetch) = this.update(cx, |_, cx| {
            runtime::spawn(cx, async move {
              // Every live of the first page, for the bell; the sidebar shows the first ones.
              let live = helix.followed_streams(&user_id, None).await?.items;
              let ids = live.iter().map(|s| s.user_id.clone()).collect();
              let avatars = cards::avatars(&helix, ids).await;
              Ok::<_, Error>((live, avatars))
            })
          }) else {
            return;
          };
          let result = fetch.await;
          let alive = this.update(cx, |this, cx| match result {
            Some(Ok((live, avatars))) => {
              if let State::SignedIn(signed) = &mut this.state {
                signed.bell.update(&live, SystemTime::now());
                signed.live = live.into_iter().take(SIDEBAR_LIVE).collect();
                signed.avatars = avatars;
              }
              cx.notify();
            }
            Some(Err(e)) => this.handle_error(&e, cx),
            None => {}
          });
          if alive.is_err() {
            return;
          }
          cx.background_executor().timer(SIDEBAR_EVERY).await;
        }
      }
    });
    self.state = State::SignedIn(Box::new(Signed {
      session,
      helix,
      login: validation.login,
      user_id: validation.user_id,
      live: Vec::new(),
      avatars: Avatars::new(),
      bell: bell::Bell::default(),
      _validate: validate,
      _sidebar: sidebar,
    }));
    self.login = None;
    self.login_message = None;
    self._login_subscription = None;
    self.stack.clear();
    self.open(Route::Following, cx);
    if let Some(route) = self.pending.take() {
      self.open(route, cx);
    }
    cx.notify();
  }

  /// Leaves the session here: the pages go, the refresh token is deleted, the sign-in card shows
  /// with `message`.
  fn sign_out(&mut self, message: Option<SharedString>, cx: &mut Context<Self>) {
    self.state = State::SignedOut;
    self.stack.clear();
    self.login = None;
    self.login_message = message;
    cx.background_executor()
      .spawn(async { session::forget_refresh() })
      .detach();
    Settings::update(cx, |s| s.twitch.login = None);
    cx.notify();
  }

  /// "Se déconnecter": revokes the token at Twitch, then signs out here even if it fails.
  pub fn log_out(&mut self, cx: &mut Context<Self>) {
    if let State::SignedIn(signed) = &self.state {
      let session = signed.session.clone();
      drop(runtime::spawn(cx, async move {
        if let Err(e) = session.revoke().await {
          tracing::warn!("Twitch sign-out: {e}");
        }
      }));
    }
    self.sign_out(None, cx);
  }

  /// Shows `route`: sidebar pages reset the stack, categories and lives are pushed. The search
  /// needs the window: [`Space::open_search`].
  pub fn open(&mut self, route: Route, cx: &mut Context<Self>) {
    let State::SignedIn(signed) = &self.state else {
      return;
    };
    if self.stack.last().is_some_and(|p| p.route == route) {
      return;
    }
    let nav = cx.weak_entity();
    let helix = signed.helix.clone();
    let view: AnyView = match &route {
      Route::Following => {
        let user_id = signed.user_id.clone();
        cx.new(|cx| FollowingView::new(nav, helix, user_id, cx))
          .into()
      }
      Route::Browse => cx.new(|cx| BrowseView::new(nav, helix, cx)).into(),
      Route::Category { id, name } => {
        let (id, name) = (id.clone(), name.clone());
        cx.new(|cx| CategoryView::new(nav, helix, id, name, cx))
          .into()
      }
      Route::Search => return,
      Route::Channel(login) => {
        let Some(client) = self.client.clone() else {
          return;
        };
        let login = login.clone();
        cx.new(|cx| ChannelView::new(nav, helix, client, login, cx))
          .into()
      }
      Route::Watch(_) | Route::Vod(_) | Route::Clip(_) => {
        let Some(client) = self.client.clone() else {
          return;
        };
        let kind = match &route {
          Route::Vod(id) => Kind::Vod(id.clone()),
          Route::Clip(slug) => Kind::Clip(slug.clone()),
          Route::Watch(login) => Kind::Live(login.clone()),
          _ => return,
        };
        cx.new(|cx| WatchView::new(kind, client, helix, nav, cx))
          .into()
      }
    };
    self.push(route, view, cx);
  }

  fn push(&mut self, route: Route, view: AnyView, cx: &mut Context<Self>) {
    if route.is_top_level() {
      self.stack.clear();
    }
    self.cover_top(true, cx);
    self.stack.push(Page { route, view });
    cx.notify();
  }

  /// Reacts to an error of any page: an expired session signs out, the rest is logged.
  pub fn handle_error(&mut self, error: &Error, cx: &mut Context<Self>) {
    match error {
      Error::SessionExpired => self.sign_out(
        Some("Votre session Twitch a expiré. Reconnectez-vous.".into()),
        cx,
      ),
      other => tracing::warn!("Twitch: {other}"),
    }
  }

  pub fn back(&mut self, cx: &mut Context<Self>) {
    if self.stack.len() > 1 {
      self.stack.pop();
      self.cover_top(false, cx);
      cx.notify();
    }
  }

  /// Pauses the player of the page on top when another page covers it.
  fn cover_top(&self, covered: bool, cx: &mut Context<Self>) {
    if let Some(player) = self.top_player(cx) {
      player.update(cx, |player, _| player.set_covered(covered));
    }
  }

  fn top_player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    let watch = self
      .stack
      .last()?
      .view
      .clone()
      .downcast::<WatchView>()
      .ok()?;
    Some(watch.read(cx).player())
  }

  /// The bell's lives are seen (its panel opened).
  pub fn see_bell(&mut self, cx: &mut Context<Self>) {
    if let State::SignedIn(signed) = &mut self.state
      && signed.bell.unseen() > 0
    {
      signed.bell.see();
      cx.notify();
    }
  }

  /// The signed-in user's login.
  pub fn login(&self) -> Option<String> {
    match &self.state {
      State::SignedIn(signed) => Some(signed.login.clone()),
      _ => None,
    }
  }

  /// Whether a web `auth-token` is saved.
  pub fn has_auth_token(&self) -> bool {
    self.auth_token.is_some()
  }

  /// A new (checked and saved) or removed web `auth-token`: the playback client is rebuilt with
  /// it, for the next live opened.
  pub fn set_auth_token(&mut self, token: Option<String>, cx: &mut Context<Self>) {
    let web_client_id = Settings::get(cx).twitch.web_client_id.clone();
    self.client = match twitch::Client::new(web_client_id, token.clone()) {
      Ok(client) => Some(client),
      Err(e) => {
        tracing::error!("cannot create the Twitch HTTP client: {e}");
        None
      }
    };
    self.auth_token = token;
    cx.notify();
  }

  fn render_login(&mut self, cx: &mut Context<Self>) -> Entity<LoginView> {
    if let Some(login) = &self.login {
      return login.clone();
    }
    let message = self.login_message.take();
    let login = cx.new(|_| LoginView::new(self.auth.clone(), message));
    self._login_subscription = Some(cx.subscribe(
      &login,
      |this, _, SignedIn(session, validation), cx| {
        this.signed_in(session.clone(), validation.clone(), cx)
      },
    ));
    self.login = Some(login.clone());
    login
  }
}

/// What the user reads for a Helix error of a page.
pub fn error_text(error: &Error) -> String {
  match error {
    Error::SessionExpired => "Votre session Twitch a expiré.".into(),
    Error::RateLimited => "Trop de requêtes vers Twitch. Réessayez dans un instant.".into(),
    Error::Http(_) => "Twitch est injoignable. Vérifiez votre connexion.".into(),
    other => format!("Twitch a répondu par une erreur : {other}"),
  }
}

/// Shorthand for pages: hands an error to the space.
pub fn report(nav: &gpui::WeakEntity<TwitchSpace>, error: &Error, cx: &mut App) {
  nav
    .update(cx, |space, cx| space.handle_error(error, cx))
    .ok();
}

impl EventEmitter<SpaceEvent> for TwitchSpace {}

impl Space for TwitchSpace {
  fn info(&self) -> SpaceInfo {
    SpaceInfo {
      name: "Twitch".into(),
      icon: include_bytes!("icon.svg"),
      accent: Self::accent(),
    }
  }

  /// "Suivies", "Parcourir", the followed channels on air, then the user (opening the settings)
  /// and sign-out at the bottom.
  fn menu(&self, _cx: &App) -> Vec<MenuEntry> {
    let State::SignedIn(signed) = &self.state else {
      return Vec::new();
    };
    let initial: SharedString = signed
      .login
      .chars()
      .next()
      .unwrap_or('?')
      .to_uppercase()
      .to_string()
      .into();
    let root = self.stack.first().map(|p| &p.route);
    let top = self.stack.last().map(|p| &p.route);
    let mut entries = vec![
      MenuEntry {
        active: root == Some(&Route::Following),
        ..MenuEntry::new("following", "Suivies", Icon::Subscriptions)
      },
      MenuEntry {
        active: root == Some(&Route::Browse),
        ..MenuEntry::new("browse", "Parcourir", Icon::Grid)
      },
    ];
    for (i, stream) in signed.live.iter().enumerate() {
      let initial: SharedString = stream
        .user_name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string()
        .into();
      let watching = top == Some(&Route::Watch(stream.user_login.clone()));
      entries.push(MenuEntry {
        active: watching,
        heading: (i == 0).then(|| "En direct".into()),
        avatar: Some(Avatar {
          picture: signed
            .avatars
            .get(&stream.user_id)
            .map(|url| cards::template(url)),
          initial,
        }),
        detail: Some(stream.game_name.clone().into()),
        ..MenuEntry::new(
          format!("live:{}", stream.user_login),
          stream.user_name.clone(),
          Icon::Tv,
        )
      });
    }
    entries.extend([
      MenuEntry {
        bottom: true,
        avatar: Some(Avatar {
          picture: None,
          initial,
        }),
        detail: Some("Twitch".into()),
        ..MenuEntry::new("account", signed.login.clone(), Icon::Account)
      },
      MenuEntry {
        bottom: true,
        ..MenuEntry::new("sign-out", "Se déconnecter", Icon::SignOut)
      },
    ]);
    entries
  }

  fn open_entry(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
    match id {
      "following" => self.open(Route::Following, cx),
      "browse" => self.open(Route::Browse, cx),
      "account" => cx.emit(SpaceEvent::OpenSettings),
      "sign-out" => self.log_out(cx),
      _ => {
        if let Some(login) = id.strip_prefix("live:") {
          self.open(Route::Watch(login.to_owned()), cx);
        }
      }
    }
  }

  fn searchable(&self) -> bool {
    matches!(self.state, State::SignedIn(_))
  }

  fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let State::SignedIn(signed) = &self.state else {
      return;
    };
    if self.stack.last().is_some_and(|p| p.route == Route::Search) {
      return;
    }
    let (nav, helix) = (cx.weak_entity(), signed.helix.clone());
    let view = cx.new(|cx| SearchView::new(nav, helix, window, cx));
    self.push(Route::Search, view.into(), cx);
  }

  fn can_go_back(&self) -> bool {
    self.stack.len() > 1
  }

  fn back(&mut self, cx: &mut Context<Self>) {
    TwitchSpace::back(self, cx);
  }

  fn player(&self, cx: &App) -> Option<Entity<PlayerView>> {
    self.top_player(cx)
  }

  fn unseen(&self, _cx: &App) -> Option<usize> {
    match &self.state {
      State::SignedIn(signed) => Some(signed.bell.unseen()),
      _ => None,
    }
  }

  fn notifications(
    &self,
    popover: WeakEntity<PopoverState>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let State::SignedIn(signed) = &self.state else {
      return None;
    };
    Some(bell::panel(
      cx.weak_entity(),
      popover,
      signed.bell.entries(),
      signed.bell.unseen(),
      signed.avatars.clone(),
      window,
      cx,
    ))
  }

  fn settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
    let nav = cx.weak_entity();
    let view = self
      .settings
      .get_or_insert_with(|| cx.new(|cx| SettingsView::new(nav, window, cx)))
      .clone();
    Some(view.into_any_element())
  }
}

impl Render for TwitchSpace {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    match &self.state {
      State::Restoring => div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
          div()
            .text_color(theme::muted())
            .child("Connexion à Twitch…"),
        )
        .into_any_element(),
      State::SignedOut => div()
        .size_full()
        .child(self.render_login(cx))
        .into_any_element(),
      State::SignedIn(_) => match self.stack.last() {
        Some(page) => div()
          .size_full()
          .child(page.view.clone())
          .into_any_element(),
        None => div().size_full().into_any_element(),
      },
    }
  }
}

#[cfg(test)]
mod tests {
  use super::Route;

  #[test]
  fn links() {
    assert_eq!(
      Route::from_url("https://www.twitch.tv/squeezie"),
      Some(Route::Watch("squeezie".into()))
    );
    assert_eq!(
      Route::from_url("https://www.twitch.tv/videos/2915878235?t=1h"),
      Some(Route::Vod("2915878235".into()))
    );
    assert_eq!(
      Route::from_url("https://clips.twitch.tv/OpenRenownedNewtPMSTwin"),
      Some(Route::Clip("OpenRenownedNewtPMSTwin".into()))
    );
    assert_eq!(
      Route::from_url("twitch.tv/ow_esports/clip/OpenRenownedNewtPMSTwin"),
      Some(Route::Clip("OpenRenownedNewtPMSTwin".into()))
    );
    assert_eq!(Route::from_url("https://youtube.com/watch?v=x"), None);
  }
}
