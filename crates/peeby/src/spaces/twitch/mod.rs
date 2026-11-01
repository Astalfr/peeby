//! The Twitch space (ADR 0010, docs/twitch-plan.md): nothing without signing in (the user's
//! choice), so its only page while signed out is the sign-in card. Signed in, it keeps the Helix
//! session (refreshed one at a time, validated at startup and every hour, as Twitch requires), the
//! optional web `auth-token` its playback client sends to GQL, and a stack of pages: "Suivies",
//! then a channel's live on top.

pub mod following;
pub mod login;
pub mod play;
pub mod session;
pub mod settings_page;
pub mod watch;

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  AnyElement, AnyView, App, Context, Entity, EventEmitter, SharedString, Subscription, Task,
  Window, div, rgb,
};
use twitch::Error;
use twitch::auth::{Auth, Session, Validation};
use ui::assets::Icon;
use ui::player::PlayerView;
use ui::theme::Accent;
use ui::{runtime, theme};

use crate::settings::Settings;
use crate::shell::space::{Avatar, MenuEntry, Space, SpaceEvent, SpaceInfo};
use following::FollowingView;
use login::{LoginView, SignedIn};
use session::Secrets;
use settings_page::SettingsView;
use watch::WatchView;

/// Twitch requires validating the token every hour.
const VALIDATE_EVERY: Duration = Duration::from_secs(3600);

/// A page of the space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
  /// "Suivies" (a field to open a channel until step 7).
  Following,
  /// A channel's live, by login.
  Watch(String),
}

struct Page {
  route: Route,
  view: AnyView,
}

/// The signed-in part.
struct Signed {
  session: Session,
  login: String,
  _validate: Task<()>,
}

enum State {
  /// A saved session is being restored (its refresh token exchanged, then validated).
  Restoring,
  SignedOut,
  SignedIn(Signed),
}

pub struct TwitchSpace {
  auth: Auth,
  state: State,
  /// The web `auth-token`, sent to GQL by `client`.
  auth_token: Option<String>,
  /// GQL and usher, for playback and the channels' headers.
  client: Option<twitch::Client>,
  stack: Vec<Page>,
  /// A channel asked for on the command line, opened once signed in.
  pending_channel: Option<String>,
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
  /// in the background. `channel` (from the command line) opens once signed in, and shows the tab.
  pub fn new(
    auth: Auth,
    secrets: Secrets,
    channel: Option<String>,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut space = Self {
      auth,
      state: State::SignedOut,
      auth_token: None,
      client: None,
      stack: Vec::new(),
      pending_channel: channel,
      login: None,
      login_message: None,
      settings: None,
      _login_subscription: None,
    };
    space.set_auth_token(secrets.auth_token, cx);
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
    if space.pending_channel.is_some() {
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
    self.state = State::SignedIn(Signed {
      session,
      login: validation.login,
      _validate: validate,
    });
    self.login = None;
    self.login_message = None;
    self._login_subscription = None;
    self.stack.clear();
    self.open(Route::Following, cx);
    if let Some(channel) = self.pending_channel.take() {
      self.open(Route::Watch(channel), cx);
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

  /// Shows `route`: "Suivies" resets the stack, a live is pushed (on top of another one too).
  pub fn open(&mut self, route: Route, cx: &mut Context<Self>) {
    let State::SignedIn(signed) = &self.state else {
      return;
    };
    if self.stack.last().is_some_and(|p| p.route == route) {
      return;
    }
    let nav = cx.weak_entity();
    let view: AnyView = match &route {
      Route::Following => {
        let login = signed.login.clone();
        cx.new(|_| FollowingView::new(nav, login)).into()
      }
      Route::Watch(login) => {
        let Some(client) = self.client.clone() else {
          return;
        };
        let login = login.clone();
        cx.new(|cx| WatchView::new(login, client, nav, cx)).into()
      }
    };
    if route == Route::Following {
      self.stack.clear();
    }
    self.cover_top(true, cx);
    self.stack.push(Page { route, view });
    cx.notify();
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

impl EventEmitter<SpaceEvent> for TwitchSpace {}

impl Space for TwitchSpace {
  fn info(&self) -> SpaceInfo {
    SpaceInfo {
      name: "Twitch".into(),
      badge: "TW".into(),
      accent: Self::accent(),
    }
  }

  /// "Suivies", then the user (opening the settings) and sign-out at the bottom.
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
    let following = self
      .stack
      .first()
      .is_some_and(|p| p.route == Route::Following);
    vec![
      MenuEntry {
        active: following,
        ..MenuEntry::new("following", "Suivies", Icon::Subscriptions)
      },
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
    ]
  }

  fn open_entry(&mut self, id: &str, _window: &mut Window, cx: &mut Context<Self>) {
    match id {
      "following" => self.open(Route::Following, cx),
      "account" => cx.emit(SpaceEvent::OpenSettings),
      "sign-out" => self.log_out(cx),
      _ => {}
    }
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
