//! The Twitch space (ADR 0010, ADR 0016, docs/twitch-plan.md): it works without signing in, its
//! catalog through anonymous GQL ([`Catalog`]). The web `auth-token` is its sign-in: its client
//! plays as the user's account (points, VOD positions, subscriber-only VODs) and names the account
//! entry. Helix is optional and only for the followed channels (GQL refuses them to raw queries),
//! and the catalog's backup: its session is refreshed one at a time, validated at startup and
//! every hour, as Twitch requires; it is offered and used only once the account is signed in.
//! A history of pages per tab (ADR 0015, ADR 0019): "Suivies" (the account's sign-in card, then
//! the Helix card until connected), "Parcourir" or the search, then categories and lives. With Helix, the
//! sidebar lists the followed channels on air, refreshed every two minutes; those that go live in
//! the meantime show in the bell ([`bell`]). The sidebar's "Multistream" plays up to six lives at
//! once ([`multistream`], ADR 0033).
//! Debug builds add a fake chat to the sidebar ([`fake_chat`]): the live chat's features without
//! a channel nor the network.

mod bell;
mod browse;
mod cards;
mod channel;
mod chat;
#[cfg(debug_assertions)]
mod fake_chat;
mod follow;
mod following;
mod hype_train;
mod login;
mod multistream;
mod play;
mod points;
mod polls;
mod progress;
mod replay;
mod search;
pub mod session;
mod settings_page;
mod sign_in;
mod watch;

use std::time::{Duration, SystemTime};

use gpui::prelude::*;
use gpui::{
  AnyElement, AnyView, App, Context, Entity, EntityId, EventEmitter, SharedString, Subscription,
  Task, WeakEntity, Window, div, rgb,
};
use gpui_component::popover::PopoverState;
use twitch::auth::{Auth, Session, Validation};
use twitch::catalog::Catalog;
use twitch::helix::{Helix, Stream};
use twitch::{Error, gql};
use ui::assets::Icon;
use ui::i18n::t;
use ui::images::Images;
use ui::player::PlayerView;
use ui::theme::Accent;
use ui::widgets::Flash;
use ui::{runtime, theme};

use browse::{BrowseView, CategoryView};
use cards::Avatars;
use channel::ChannelView;
use following::FollowingView;
use login::{AccountCard, LoginView, SignedIn};
use multistream::MultistreamView;
use peeby_core::settings::Settings;
use peeby_core::shell::space::{
  Avatar, HoverCard, LiveEntry, Lives, MenuEntry, Page, Space, SpaceEvent, SpaceInfo, TabId, Tabs,
};
use search::SearchView;
use session::Secrets;
use settings_page::SettingsView;
use sign_in::SignIn;
use watch::{Kind, WatchView};

/// The space's id in the settings (`SpaceInfo::id`).
pub const ID: &str = "twitch";
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
  /// "Multistream": several lives at once.
  Multistream,
  /// A channel's page (its VODs and clips), by login.
  Channel(String),
  /// A channel's live, by login.
  Watch(String),
  /// A VOD, by id, from `start` seconds (a chapter, a link's `?t=`) rather than where the viewer
  /// stopped.
  Vod {
    /// The VOD's id.
    id: String,
    /// Where to start, in seconds.
    start: Option<u32>,
  },
  /// A clip, by slug.
  Clip(String),
  /// The fake chat (debug builds).
  #[cfg(debug_assertions)]
  FakeChat,
}

impl Route {
  /// The page of a Twitch link: `twitch.tv/videos/<id>` (from its `?t=1h2m3s`, if any),
  /// `clips.twitch.tv/<slug>`, `twitch.tv/<login>/clip/<slug>`, else `twitch.tv/<login>`'s live.
  pub fn from_url(url: &str) -> Option<Self> {
    let (_, rest) = url.split_once("twitch.tv/")?;
    let start = rest
      .split_once('?')
      .and_then(|(_, query)| query.split(['&', '#']).find_map(|p| p.strip_prefix("t=")))
      .map(|t| u32::try_from(twitch::helix::parse_duration(t)).unwrap_or(u32::MAX))
      .filter(|&s| s > 0);
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
      ("videos", Some(id), _) if id.bytes().all(|b| b.is_ascii_digit()) => Some(Self::Vod {
        id: id.to_owned(),
        start,
      }),
      (_, Some("clip"), Some(slug)) => Some(Self::Clip(slug.to_owned())),
      _ => following::channel_login(first).map(Self::Watch),
    }
  }

  /// Sidebar pages and the search: the sidebar lights the last one up to the page shown.
  fn is_top_level(&self) -> bool {
    #[cfg(debug_assertions)]
    if *self == Self::FakeChat {
      return true;
    }
    matches!(
      self,
      Self::Following | Self::Browse | Self::Search | Self::Multistream
    )
  }

  /// Pages with a player: a sidebar page opened after them drops their view (the multistream's
  /// lineup stays with the space).
  fn plays(&self) -> bool {
    matches!(
      self,
      Self::Watch(_) | Self::Vod { .. } | Self::Clip(_) | Self::Multistream
    )
  }

  /// The tab's label on this page, before a page sets its own (a VOD's title).
  fn name(&self) -> SharedString {
    match self {
      Self::Following => t("Following", "Suivies").into(),
      Self::Browse => t("Browse", "Parcourir").into(),
      Self::Category { name, .. } => name.clone().into(),
      Self::Search => t("Search", "Recherche").into(),
      Self::Multistream => "Multistream".into(),
      Self::Channel(login) | Self::Watch(login) => login.clone().into(),
      Self::Vod { .. } => t("Video", "Vidéo").into(),
      Self::Clip(_) => "Clip".into(),
      #[cfg(debug_assertions)]
      Self::FakeChat => t("Fake chat", "Faux chat").into(),
    }
  }
}

/// What a Twitch page holds to navigate.
#[derive(Clone)]
pub struct Nav(peeby_core::shell::space::Nav<TwitchSpace>);

/// The shell's navigation (the page's tab, the space): a newtype only so that the methods below
/// can be defined here, out of the crate of `peeby_core::shell::space::Nav`.
impl std::ops::Deref for Nav {
  type Target = peeby_core::shell::space::Nav<TwitchSpace>;

  fn deref(&self) -> &Self::Target {
    &self.0
  }
}

impl Nav {
  /// The navigation of a page in `tab`.
  fn new(space: WeakEntity<TwitchSpace>, tab: TabId) -> Self {
    Self(peeby_core::shell::space::Nav::new(space, tab))
  }

  /// Opens `route` in the page's tab.
  pub fn open(&self, route: Route, cx: &mut App) {
    let tab = self.tab;
    self.update(cx, |space, cx| space.open(tab, route, cx)).ok();
  }

  /// Opens `route` in a new tab after the page's, in the background (a middle click); `label`
  /// names the tab until its page shows.
  pub fn open_in_new_tab(&self, route: Route, label: Option<SharedString>, cx: &mut App) {
    let tab = self.tab;
    self
      .update(cx, |space, cx| space.open_in_new_tab(tab, route, label, cx))
      .ok();
  }

  /// Names the tab while the page of `view` is on top (a VOD's title, once known).
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

/// The Helix part, when connected: the followed channels.
struct Signed {
  session: Session,
  helix: Helix,
  login: String,
  user_id: String,
  /// The followed channels on air (the sidebar shows the first ones, the notification area's
  /// list every one), with their avatars (and the user's); `None` before the first reading.
  live: Option<Vec<Stream>>,
  avatars: Avatars,
  /// Who went live since Peeby opened.
  bell: bell::Bell,
  _validate: Task<()>,
  _sidebar: Task<()>,
}

/// Helix's connection.
enum State {
  /// A saved session is being restored (its refresh token exchanged, then validated).
  Restoring,
  SignedOut,
  SignedIn(Box<Signed>),
}

/// "Suivies" while the saved Helix session is being restored.
struct Connecting;

impl Render for Connecting {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .flex()
      .items_center()
      .justify_center()
      .child(
        div()
          .text_color(theme::muted())
          .child(t("Connecting to Twitch…", "Connexion à Twitch…")),
      )
  }
}

/// The Twitch space: the anonymous catalog, the optional sign-ins and the space's tabs.
pub struct TwitchSpace {
  auth: Auth,
  /// Helix, for the followed channels only.
  state: State,
  /// The web `auth-token`, sent to GQL by `client`: the space's sign-in.
  auth_token: Option<String>,
  /// Who `auth_token` belongs to, once Twitch answered.
  account: Option<gql::Account>,
  /// Twitch answered that `auth_token` is not signed in (revoked, expired): the space is signed
  /// out until a new one. Unreachable, the token counts as valid (it was checked when saved).
  token_refused: bool,
  /// The web sign-in, shared by the settings and the "Suivies" card; made when first needed.
  sign_in: Option<Entity<SignIn>>,
  /// A saved Helix refresh token left unspent while there is no auth-token: the followed channels
  /// need the account's sign-in first, so it is restored once a token arrives.
  parked_refresh: Option<String>,
  _account: Option<Task<()>>,
  /// GQL and usher, for playback, the account's data and the channels' headers.
  client: Option<twitch::Client>,
  /// The pages' catalog: anonymous GQL, Helix as its backup when connected.
  catalog: Option<Catalog>,
  tabs: Tabs<Route>,
  /// The Helix card on "Suivies" while not connected, made when first shown.
  login: Option<Entity<LoginView>>,
  /// Why the card shows (the connection expired, Twitch unreachable).
  login_message: Option<SharedString>,
  /// Its part of the settings page, made the first time it shows.
  settings: Option<Entity<SettingsView>>,
  /// The account's flash in the settings, after the sidebar's "Sign in".
  account_flash: Flash,
  /// The multistream's lives (the main one first), kept while Peeby runs: its page finds them
  /// again when it is built again.
  multistream: Vec<String>,
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

  /// The space and the secrets read before the UI started: a saved refresh token is exchanged in
  /// the background. One tab when the settings open one at startup, or for `pending` (from the
  /// command line), which opens in it at once and shows it.
  pub fn new(auth: Auth, secrets: Secrets, pending: Option<Route>, cx: &mut Context<Self>) -> Self {
    let mut space = Self {
      auth,
      state: State::SignedOut,
      auth_token: None,
      account: None,
      token_refused: false,
      sign_in: None,
      parked_refresh: None,
      _account: None,
      client: None,
      catalog: None,
      tabs: Tabs::default(),
      login: None,
      login_message: None,
      settings: None,
      account_flash: Flash::default(),
      multistream: Vec::new(),
      _login_subscription: None,
    };
    space.set_auth_token(secrets.auth_token, cx);
    // The sidebar's avatars arrive through the image store.
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    // Twitch may have changed its web Client-ID: read the current one from its page.
    if let Some(client) = space.client.clone() {
      drop(runtime::spawn(cx, async move {
        if let Err(e) = client.refresh_client_id().await {
          tracing::warn!("Twitch web Client-ID: {e}");
        }
      }));
    }
    // The followed channels come after the account's sign-in: without an auth-token, a saved
    // Helix session waits for one.
    if let Some(refresh) = secrets.refresh {
      if space.auth_token.is_some() {
        space.restore(refresh, cx);
      } else {
        space.parked_refresh = Some(refresh);
      }
    }
    let shown = pending.is_some();
    if !shown && !Settings::get(cx).opens_at_startup(ID) {
      return space;
    }
    let pages = std::iter::once(space.home()).chain(pending).collect();
    let first = space.tabs.add(pages, None);
    space.tabs.remember(first);
    if shown {
      // After the shell has subscribed to the space's events.
      cx.spawn(async move |this, cx| {
        this
          .update(cx, |_, cx| cx.emit(SpaceEvent::Activate(first)))
          .ok();
      })
      .detach();
    }
    space
  }

  /// A new tab's page: "Suivies" with Helix (or while it is being restored) and the account
  /// signed in, else "Parcourir".
  fn home(&self) -> Route {
    match self.state {
      State::Restoring | State::SignedIn(_) if self.account_signed_in() => Route::Following,
      _ => Route::Browse,
    }
  }

  /// The account is signed in: an auth-token is saved and Twitch did not reject it. The followed
  /// channels (Helix) are offered and shown only then.
  pub fn account_signed_in(&self) -> bool {
    self.auth_token.is_some() && !self.token_refused
  }

  /// The space's web sign-in.
  pub fn sign_in(&mut self, cx: &mut Context<Self>) -> Entity<SignIn> {
    let nav = cx.weak_entity();
    self
      .sign_in
      .get_or_insert_with(|| cx.new(|_| SignIn::new(nav)))
      .clone()
  }

  /// Opens the settings on the account, which flashes while signed out (the sidebar's "Sign
  /// in", the card's "Paste a token").
  pub fn reveal_account(&mut self, cx: &mut Context<Self>) {
    if !self.account_signed_in() {
      self.account_flash.start();
    }
    cx.emit(SpaceEvent::OpenSettings { reveal: true });
  }

  fn helix(&self) -> Option<Helix> {
    match &self.state {
      State::SignedIn(signed) => Some(signed.helix.clone()),
      _ => None,
    }
  }

  /// Helix connected or gone: the catalog gets (or loses) its backup, and the "Suivies" pages of
  /// the tabs are made again when they show; the other pages stay.
  fn helix_changed(&mut self, cx: &mut Context<Self>) {
    let helix = self.helix();
    self.catalog = self.catalog.as_ref().map(|c| c.with_helix(helix));
    self.following_changed(cx);
  }

  /// The "Suivies" pages of the tabs are made again when they show (Helix or the account's
  /// sign-in changed); the other pages stay.
  fn following_changed(&mut self, cx: &mut Context<Self>) {
    for tab in self.tabs.iter_mut() {
      for page in tab.pages_mut() {
        if page.route == Route::Following {
          page.view = None;
        }
      }
    }
    cx.notify();
  }

  /// "Suivies" in `tab`: the account's sign-in card while signed out, then the followed
  /// channels, the connecting note, or the Helix card.
  fn following_view(&mut self, tab: TabId, cx: &mut Context<Self>) -> Option<AnyView> {
    if !self.account_signed_in() {
      let (nav, sign_in, refused) = (cx.weak_entity(), self.sign_in(cx), self.token_refused);
      return Some(
        cx.new(|cx| AccountCard::new(nav, sign_in, refused, cx))
          .into(),
      );
    }
    Some(match &self.state {
      State::SignedIn(signed) => {
        let nav = Nav::new(cx.weak_entity(), tab);
        let (helix, user_id) = (signed.helix.clone(), signed.user_id.clone());
        let catalog = self.catalog.clone()?;
        cx.new(|cx| FollowingView::new(nav, helix, catalog, user_id, cx))
          .into()
      }
      State::Restoring => cx.new(|_| Connecting).into(),
      State::SignedOut => self.render_login(cx).into(),
    })
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
          Some(Err(Error::SessionExpired)) => this.sign_out(Some(expired()), cx),
          Some(Err(e)) => {
            // The refresh token was not spent (Twitch did not answer): it stays for next time.
            tracing::warn!("Twitch session restore: {e}");
            this.state = State::SignedOut;
            this.login_message = Some(login::describe(&e));
            this.helix_changed(cx);
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
                .update(cx, |this, cx| this.sign_out(Some(expired()), cx))
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
          let Ok(fetch) = this.update(cx, |this, cx| {
            let catalog = this.catalog.clone();
            runtime::spawn(cx, async move {
              // Every live of the first page, for the bell; the sidebar shows the first ones.
              let live = helix.followed_streams(&user_id, None).await?.items;
              // The user's own avatar first, for the account entry.
              let ids = std::iter::once(user_id.clone())
                .chain(live.iter().map(|s| s.user_id.clone()))
                .collect();
              let avatars = match catalog {
                Some(catalog) => cards::avatars(&catalog, ids).await,
                None => Avatars::new(),
              };
              Ok::<_, Error>((live, avatars))
            })
          }) else {
            return;
          };
          let result = fetch.await;
          let alive = this.update(cx, |this, cx| match result {
            Some(Ok((live, avatars))) => {
              let deliveries = [bell::Change::Live, bell::Change::Category]
                .map(|change| (change, change.delivery(cx)));
              if let State::SignedIn(signed) = &mut this.state {
                let in_bell = |change| deliveries.iter().any(|(c, d)| *c == change && d.in_app);
                let events = signed.bell.update(&live, SystemTime::now(), in_bell);
                bell::toasts(events, &avatars, cx.weak_entity(), cx);
                signed.live = Some(live);
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
      live: None,
      avatars: Avatars::new(),
      bell: bell::Bell::default(),
      _validate: validate,
      _sidebar: sidebar,
    }));
    self.login = None;
    self.login_message = None;
    self._login_subscription = None;
    self.helix_changed(cx);
  }

  /// Disconnects Helix here: the refresh token is deleted, "Suivies" shows the card again with
  /// `message`; the other pages stay.
  fn sign_out(&mut self, message: Option<SharedString>, cx: &mut Context<Self>) {
    self.state = State::SignedOut;
    self.login = None;
    self.login_message = message;
    cx.background_executor()
      .spawn(async { session::forget_refresh() })
      .detach();
    Settings::update(cx, |s| s.twitch.login = None);
    self.helix_changed(cx);
  }

  /// Disconnects the followed channels: revokes the Helix token at Twitch, then signs out here
  /// even if it fails.
  pub fn disconnect_helix(&mut self, cx: &mut Context<Self>) {
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

  /// "Se déconnecter": forgets the auth-token and disconnects Helix (which needs it).
  pub fn log_out(&mut self, cx: &mut Context<Self>) {
    if self.parked_refresh.take().is_some() {
      cx.background_executor()
        .spawn(async { session::forget_refresh() })
        .detach();
    }
    if self.auth_token.is_some() {
      cx.background_executor()
        .spawn(async { session::forget_auth_token() })
        .detach();
      self.set_auth_token(None, cx);
    }
    if !matches!(self.state, State::SignedOut) {
      self.disconnect_helix(cx);
    }
  }

  /// Opens `route` in `tab`, after the page shown (which is covered: a player pauses). It is
  /// built when it shows, so a tab in the background builds nothing. A sidebar page or the
  /// search drops the paused players behind it.
  pub fn open(&mut self, tab: TabId, route: Route, cx: &mut Context<Self>) {
    let Some(t) = self.tabs.get(tab) else {
      return;
    };
    if t.current().is_some_and(|p| p.route == route) {
      return;
    }
    if self.catalog.is_none() || self.client.is_none() {
      return;
    }
    self.cover(tab, true, cx);
    let top_level = route.is_top_level();
    if let Some(t) = self.tabs.get_mut(tab) {
      t.push(Page::unbuilt(route));
      if top_level {
        t.unload_behind(Route::plays);
      }
    }
    cx.notify();
  }

  /// The view of `route` in `tab`; `None` without a catalog.
  fn build(
    &mut self,
    tab: TabId,
    route: &Route,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Option<AnyView> {
    // Needs neither the catalog nor a client: nothing of it reaches Twitch.
    #[cfg(debug_assertions)]
    if *route == Route::FakeChat {
      return Some(cx.new(fake_chat::FakeChatView::new).into());
    }
    let (catalog, client) = (self.catalog.clone()?, self.client.clone()?);
    let nav = Nav::new(cx.weak_entity(), tab);
    Some(match route {
      Route::Following => self.following_view(tab, cx)?,
      Route::Browse => cx.new(|cx| BrowseView::new(nav, catalog, cx)).into(),
      Route::Category { id, name } => {
        let (id, name) = (id.clone(), name.clone());
        cx.new(|cx| CategoryView::new(nav, catalog, id, name, cx))
          .into()
      }
      Route::Search => cx
        .new(|cx| SearchView::new(nav, catalog, window, cx))
        .into(),
      Route::Multistream => {
        let lineup = self.multistream.clone();
        cx.new(|cx| MultistreamView::new(nav, client, catalog, lineup, cx))
          .into()
      }
      Route::Channel(login) => {
        let login = login.clone();
        cx.new(|cx| ChannelView::new(nav, catalog, client, login, cx))
          .into()
      }
      Route::Watch(_) | Route::Vod { .. } | Route::Clip(_) => {
        let kind = match route {
          Route::Vod { id, start } => Kind::Vod {
            id: id.clone(),
            start: *start,
          },
          Route::Clip(slug) => Kind::Clip(slug.clone()),
          Route::Watch(login) => Kind::Live(login.clone()),
          _ => return None,
        };
        cx.new(|cx| WatchView::new(kind, client, catalog, nav, cx))
          .into()
      }
      #[cfg(debug_assertions)]
      Route::FakeChat => return None,
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
  pub fn open_from_notification(&mut self, route: Route, cx: &mut Context<Self>) {
    // Not over the settings' tab: a tab of its own then.
    match self.tabs.shown().filter(|&tab| !self.settings_shown(tab)) {
      Some(tab) => {
        cx.emit(SpaceEvent::Activate(tab));
        self.open(tab, route, cx);
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

  /// Reacts to an error of any page: an expired Helix session disconnects it, the rest is
  /// logged.
  pub fn handle_error(&mut self, error: &Error, cx: &mut Context<Self>) {
    match error {
      Error::SessionExpired => self.sign_out(Some(expired()), cx),
      other => tracing::warn!("Twitch: {other}"),
    }
  }

  /// The player of the page `tab` shows, or shows the settings over: a multistream's main one.
  fn top_player(&self, tab: TabId, cx: &App) -> Option<Entity<PlayerView>> {
    let view = self.tabs.get(tab)?.page()?.view.clone()?;
    match view.downcast::<WatchView>() {
      Ok(watch) => Some(watch.read(cx).player()),
      Err(view) => view.downcast::<MultistreamView>().ok()?.read(cx).player(cx),
    }
  }

  /// The multistream the page `tab` shows, if it does.
  fn top_multistream(&self, tab: TabId) -> Option<Entity<MultistreamView>> {
    self
      .tabs
      .get(tab)?
      .page()?
      .view
      .clone()?
      .downcast::<MultistreamView>()
      .ok()
  }

  /// The route of a sidebar entry.
  fn entry_route(id: &str) -> Option<Route> {
    match id {
      "following" => Some(Route::Following),
      "browse" => Some(Route::Browse),
      "multistream" => Some(Route::Multistream),
      #[cfg(debug_assertions)]
      "fake-chat" => Some(Route::FakeChat),
      _ => id
        .strip_prefix("live:")
        .map(|login| Route::Watch(login.to_owned())),
    }
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

  /// The login Helix is connected with.
  pub fn login(&self) -> Option<String> {
    match &self.state {
      State::SignedIn(signed) => Some(signed.login.clone()),
      _ => None,
    }
  }

  /// What the settings' test notification of `change` announces, with its avatar (a Helix
  /// template URL): a followed channel on air, as a real one would be; else the user's own
  /// account; else neither.
  fn test_notification(&self, change: bell::Change) -> Option<(bell::Event, Option<String>)> {
    let signed = match &self.state {
      State::SignedIn(signed) => Some(signed),
      _ => None,
    };
    let event = |user_id: &str, login: &str, name: &str, title: &str, category: &str| bell::Event {
      user_id: user_id.to_owned(),
      login: login.to_owned(),
      name: name.to_owned(),
      title: title.to_owned(),
      category: category.to_owned(),
      change,
      at: SystemTime::now(),
    };
    if let Some(stream) = signed.and_then(|s| s.live.as_deref()?.first()) {
      let avatar = signed.and_then(|s| s.avatars.get(&stream.user_id).cloned());
      let sample = event(
        &stream.user_id,
        &stream.user_login,
        &stream.user_name,
        &stream.title,
        &stream.game_name,
      );
      return Some((sample, avatar));
    }
    let title = t(
      "This is a test notification",
      "Ceci est une notification de test",
    );
    if let Some(account) = &self.account {
      // A 70 px picture (`…-profile_image-70x70.png`): its template, for the toast's size.
      let avatar = account
        .avatar_url
        .as_deref()
        .map(|url| url.replace("-70x70.", "-{width}x{height}."));
      let sample = event(
        &account.id,
        &account.login,
        &account.display_name,
        title,
        "Just Chatting",
      );
      return Some((sample, avatar));
    }
    signed.map(|signed| {
      let avatar = signed.avatars.get(&signed.user_id).cloned();
      let sample = event(
        &signed.user_id,
        &signed.login,
        &signed.login,
        title,
        "Just Chatting",
      );
      (sample, avatar)
    })
  }

  /// The settings' test notification of `change` ([`Self::test_notification`], else Peeby
  /// itself), with its avatar.
  fn test_sample(&self, change: bell::Change) -> (bell::Event, Option<String>) {
    self.test_notification(change).unwrap_or_else(|| {
      let sample = bell::Event {
        user_id: String::new(),
        login: String::new(),
        name: "Peeby".into(),
        title: t(
          "This is a test notification",
          "Ceci est une notification de test",
        )
        .into(),
        category: "Just Chatting".into(),
        change,
        at: SystemTime::now(),
      };
      (sample, None)
    })
  }

  /// Shows the settings' test notification of `change`.
  pub fn test_toast(space: &Entity<Self>, change: bell::Change, cx: &mut App) {
    let (sample, avatar) = space.read(cx).test_sample(change);
    bell::test_toast(&sample, avatar.as_deref(), cx);
  }

  /// The auth-token's account, once Twitch answered.
  pub fn account(&self) -> Option<&gql::Account> {
    self.account.as_ref()
  }

  /// Whether a web `auth-token` is saved.
  /// The account's flash in the settings.
  pub fn account_flash(&self) -> Flash {
    self.account_flash
  }

  /// Signed in with the web `auth-token` (chat, points, polls, watch progress).
  pub fn has_auth_token(&self) -> bool {
    self.auth_token.is_some()
  }

  /// A new (checked and saved) or removed web `auth-token`: the client is rebuilt with it, for the
  /// next page opened, and its account read again.
  pub fn set_auth_token(&mut self, token: Option<String>, cx: &mut Context<Self>) {
    let web_client_id = Settings::get(cx).twitch.web_client_id.clone();
    self.client = match twitch::Client::new(web_client_id, token.clone()) {
      Ok(client) => Some(client),
      Err(e) => {
        tracing::error!("cannot create the Twitch HTTP client: {e}");
        None
      }
    };
    let helix = self.helix();
    self.catalog = self
      .client
      .as_ref()
      .map(|client| Catalog::new(client).with_helix(helix));
    self.account = None;
    self.token_refused = false;
    self._account = match (&self.client, &token) {
      (Some(client), Some(_)) => {
        let fetch = runtime::spawn(cx, {
          let client = client.clone();
          async move { client.account().await }
        });
        Some(cx.spawn(async move |this, cx| {
          let result = fetch.await;
          this
            .update(cx, |this, cx| match result {
              Some(Ok(account)) => {
                this.account = Some(account);
                cx.notify();
              }
              Some(Err(Error::NotLoggedIn)) => {
                tracing::warn!("Twitch rejects the saved auth-token");
                this.token_refused = true;
                this.following_changed(cx);
              }
              Some(Err(e)) => tracing::warn!("Twitch auth-token account: {e}"),
              None => {}
            })
            .ok();
        }))
      }
      _ => None,
    };
    self.auth_token = token;
    // A saved Helix session waited for the account's sign-in.
    if self.auth_token.is_some()
      && matches!(self.state, State::SignedOut)
      && let Some(refresh) = self.parked_refresh.take()
    {
      self.restore(refresh, cx);
    }
    self.following_changed(cx);
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

/// Why the Helix card shows again on "Suivies" after its session expired.
fn expired() -> SharedString {
  login::describe(&Error::SessionExpired)
}

/// What the user reads for a Helix error of a page.
pub fn error_text(error: &Error) -> String {
  match error {
    Error::SessionExpired => t(
      "Your Twitch session expired.",
      "Votre session Twitch a expiré.",
    )
    .into(),
    Error::RateLimited => t(
      "Too many requests to Twitch. Try again in a moment.",
      "Trop de requêtes vers Twitch. Réessayez dans un instant.",
    )
    .into(),
    Error::Http(_) => t(
      "Twitch can't be reached. Check your connection.",
      "Twitch est injoignable. Vérifiez votre connexion.",
    )
    .into(),
    other => ui::tf!(
      "Twitch answered with an error: {other}",
      "Twitch a répondu par une erreur : {other}"
    ),
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
  type Route = Route;

  fn info(&self) -> SpaceInfo {
    SpaceInfo {
      id: ID,
      name: "Twitch".into(),
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

  /// A covered player pauses; a multistream's every one, its row's playing on once shown again.
  fn cover(&self, tab: TabId, covered: bool, cx: &mut Context<Self>) {
    if let Some(multistream) = self.top_multistream(tab) {
      multistream.update(cx, |view, cx| view.set_covered(covered, cx));
    } else if let Some(player) = self.top_player(tab, cx) {
      player.update(cx, |player, _| player.set_covered(covered));
    }
  }

  fn tabs(&self) -> Vec<TabId> {
    self.tabs.ids()
  }

  fn new_tab(&mut self, _cx: &mut Context<Self>) -> TabId {
    let home = self.home();
    self.tabs.add(vec![home], None)
  }

  fn close_tab(&mut self, tab: TabId, cx: &mut Context<Self>) {
    self.tabs.remove(tab);
    cx.notify();
  }

  fn title(&self, tab: TabId, _cx: &App) -> SharedString {
    self
      .tabs
      .title(tab, Route::name)
      .unwrap_or_else(|| "Twitch".into())
  }

  /// "Suivies", "Parcourir", the followed channels on air (with Helix), then the account
  /// (opening the settings) and sign-out at the bottom, or "Se connecter" without any.
  fn menu(&self, tab: TabId, _cx: &App) -> Vec<MenuEntry> {
    let signed = match &self.state {
      State::SignedIn(signed) => Some(signed),
      _ => None,
    };
    let tab = self.tabs.get(tab);
    let root = tab.and_then(|t| t.section(Route::is_top_level));
    let top = tab.and_then(|t| t.current()).map(|p| &p.route);
    let mut entries = vec![
      MenuEntry {
        active: root == Some(&Route::Following),
        ..MenuEntry::new("following", t("Following", "Suivies"), Icon::Subscriptions)
      },
      MenuEntry {
        active: root == Some(&Route::Browse),
        ..MenuEntry::new("browse", t("Browse", "Parcourir"), Icon::Grid)
      },
      MenuEntry {
        active: root == Some(&Route::Multistream),
        ..MenuEntry::new("multistream", "Multistream", Icon::Multistream)
      },
    ];
    let live = signed.and_then(|s| s.live.as_deref()).unwrap_or_default();
    for (i, stream) in live.iter().take(SIDEBAR_LIVE).enumerate() {
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
        heading: (i == 0).then(|| t("Live", "En direct").into()),
        avatar: Some(Avatar {
          picture: signed
            .and_then(|s| s.avatars.get(&stream.user_id))
            .map(|url| cards::template(url)),
          initial,
        }),
        detail: Some(stream.game_name.clone().into()),
        live: true,
        hover: Some(HoverCard {
          // `line_clamp` only limits wrapped lines.
          title: stream.title.replace(['\n', '\r'], " ").into(),
          detail: (!stream.game_name.is_empty()).then(|| stream.game_name.clone().into()),
        }),
        ..MenuEntry::new(
          format!("live:{}", stream.user_login),
          stream.user_name.clone(),
          Icon::Tv,
        )
      });
    }
    #[cfg(debug_assertions)]
    entries.push(MenuEntry {
      active: root == Some(&Route::FakeChat),
      bottom: true,
      ..MenuEntry::new("fake-chat", t("Fake chat", "Faux chat"), Icon::Comments)
    });
    // The auth-token's account first, else the one Helix is connected with.
    let account = match (&self.account, signed) {
      (Some(account), _) => Some((account.display_name.clone(), account.avatar_url.clone())),
      (None, Some(signed)) => Some((
        signed.login.clone(),
        signed.avatars.get(&signed.user_id).cloned(),
      )),
      (None, None) => None,
    };
    match account {
      Some((name, avatar)) => {
        let initial: SharedString = name
          .chars()
          .next()
          .unwrap_or('?')
          .to_uppercase()
          .to_string()
          .into();
        entries.extend([
          MenuEntry {
            bottom: true,
            avatar: Some(Avatar {
              picture: avatar.map(|url| cards::template(&url)),
              initial,
            }),
            detail: Some("Twitch".into()),
            ..MenuEntry::new("account", name, Icon::Account)
          },
          MenuEntry {
            bottom: true,
            ..MenuEntry::new("sign-out", t("Sign out", "Se déconnecter"), Icon::SignOut)
          },
        ]);
      }
      None => entries.push(MenuEntry {
        bottom: true,
        ..MenuEntry::new("account", t("Sign in", "Se connecter"), Icon::Account)
      }),
    }
    entries
  }

  fn open_entry(&mut self, tab: TabId, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
    match id {
      // Signed out, the entry is "Sign in": the account flashes.
      "account" => self.reveal_account(cx),
      "sign-out" => self.log_out(cx),
      _ => {
        if let Some(route) = Self::entry_route(id) {
          self.open(tab, route, cx);
        }
      }
    }
  }

  fn open_entry_in_new_tab(&mut self, tab: TabId, id: &str, cx: &mut Context<Self>) {
    if let Some(route) = Self::entry_route(id) {
      self.open_in_new_tab(tab, route, None, cx);
    }
  }

  fn search_route(&self) -> Option<Route> {
    self.catalog.is_some().then_some(Route::Search)
  }

  fn player(&self, tab: TabId, cx: &App) -> Option<Entity<PlayerView>> {
    self.top_player(tab, cx)
  }

  fn unseen(&self, _cx: &App) -> Option<usize> {
    if !self.account_signed_in() {
      return None;
    }
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
    if !self.account_signed_in() {
      return None;
    }
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

  fn lives(&self, _cx: &App) -> Option<Lives> {
    if !self.account_signed_in() {
      return Some(Lives::Unavailable(
        t(
          "Sign in to Twitch (Following page) to see the followed channels on air.",
          "Connectez-vous à Twitch (page Suivies) pour voir les chaînes suivies en direct.",
        )
        .into(),
      ));
    }
    let signed = match &self.state {
      State::Restoring => return Some(Lives::Loading),
      State::SignedOut => {
        return Some(Lives::Unavailable(
          t(
            "Connect Twitch to Peeby (Following page) to see the followed channels on air.",
            "Connectez Twitch à Peeby (page Suivies) pour voir les chaînes suivies en direct.",
          )
          .into(),
        ));
      }
      State::SignedIn(signed) => signed,
    };
    let Some(live) = &signed.live else {
      return Some(Lives::Loading);
    };
    let entries = live
      .iter()
      .map(|stream| LiveEntry {
        id: stream.user_login.clone().into(),
        channel: stream.user_name.clone().into(),
        title: stream.title.replace(['\n', '\r'], " ").into(),
        detail: (!stream.game_name.is_empty()).then(|| stream.game_name.clone().into()),
        viewers: Some(cards::compact_viewers(stream.viewer_count).into()),
        since: stream
          .started_at
          .and_then(|ms| u64::try_from(ms).ok())
          .map(|ms| SystemTime::UNIX_EPOCH + Duration::from_millis(ms)),
        preview: (!stream.thumbnail_url.is_empty()).then(|| cards::preview(&stream.thumbnail_url)),
        avatar: Avatar::of(
          &stream.user_name,
          signed
            .avatars
            .get(&stream.user_id)
            .map(|url| cards::template(url)),
        ),
      })
      .collect();
    Some(Lives::On(entries))
  }

  fn open_live(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
    self.open_from_notification(Route::Watch(id.to_owned()), cx);
  }

  fn settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
    let nav = cx.weak_entity();
    let sign_in = self.sign_in(cx);
    let view = self
      .settings
      .get_or_insert_with(|| cx.new(|cx| SettingsView::new(nav, sign_in, window, cx)))
      .clone();
    Some(view.into_any_element())
  }
}

impl Render for TwitchSpace {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // Each window renders its own tab of the space (ADR 0028).
    let tab = self.tabs.shown_in(window.window_handle().window_id());
    // A tab opened in the background, a page reached with "back" or "forward", Helix connected
    // or gone: the page shown is built now.
    if let Some(tab) = tab
      && let Some(route) = self.tabs.get(tab).and_then(|t| t.unbuilt()).cloned()
      && let Some(view) = self.build(tab, &route, window, cx)
      && let Some(t) = self.tabs.get_mut(tab)
    {
      t.set_view(view);
    }
    match tab
      .and_then(|tab| self.tabs.get(tab))
      .and_then(|t| t.current())
      .and_then(|page| page.view.clone())
    {
      Some(view) => div().size_full().child(view).into_any_element(),
      None => div().size_full().into_any_element(),
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
      Some(Route::Vod {
        id: "2915878235".into(),
        start: Some(3600)
      })
    );
    assert_eq!(
      Route::from_url("https://www.twitch.tv/videos/2915878235?filter=archives&t=1h2m3s"),
      Some(Route::Vod {
        id: "2915878235".into(),
        start: Some(3723)
      })
    );
    assert_eq!(
      Route::from_url("https://www.twitch.tv/videos/2915878235"),
      Some(Route::Vod {
        id: "2915878235".into(),
        start: None
      })
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
