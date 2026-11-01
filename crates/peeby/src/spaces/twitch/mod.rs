//! The Twitch space (ADR 0010, docs/twitch-plan.md): nothing without signing in (the user's
//! choice), so its only page while signed out is the sign-in card. Signed in, it keeps the Helix
//! session (refreshed one at a time, validated at startup and every hour, as Twitch requires) and
//! the optional web `auth-token` used for playback. Its pages come with the next steps.

pub mod login;
pub mod session;
pub mod settings_page;

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  AnyElement, App, Context, Entity, EventEmitter, SharedString, Subscription, Task, Window, div,
  rgb,
};
use gpui_component::v_flex;
use twitch::Error;
use twitch::auth::{Auth, Session, Validation};
use ui::assets::Icon;
use ui::theme::Accent;
use ui::{runtime, theme};

use crate::settings::Settings;
use crate::shell::space::{Avatar, MenuEntry, Space, SpaceEvent, SpaceInfo};
use login::{LoginView, SignedIn};
use session::Secrets;
use settings_page::SettingsView;

/// Twitch requires validating the token every hour.
const VALIDATE_EVERY: Duration = Duration::from_secs(3600);

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
  /// The web `auth-token`, for GQL playback tokens (step 6).
  auth_token: Option<String>,
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
  /// in the background.
  pub fn new(auth: Auth, secrets: Secrets, cx: &mut Context<Self>) -> Self {
    let mut space = Self {
      auth,
      state: State::SignedOut,
      auth_token: secrets.auth_token,
      login: None,
      login_message: None,
      settings: None,
      _login_subscription: None,
    };
    if let Some(refresh) = secrets.refresh {
      space.restore(refresh, cx);
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
    cx.notify();
  }

  /// Leaves the session here: the refresh token is deleted, the sign-in card shows with
  /// `message`.
  fn sign_out(&mut self, message: Option<SharedString>, cx: &mut Context<Self>) {
    self.state = State::SignedOut;
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

  /// A new (checked and saved) or removed web `auth-token`.
  pub fn set_auth_token(&mut self, token: Option<String>, cx: &mut Context<Self>) {
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
    vec![
      MenuEntry {
        active: true,
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
      "account" => cx.emit(SpaceEvent::OpenSettings),
      "sign-out" => self.log_out(cx),
      _ => {}
    }
  }

  fn can_go_back(&self) -> bool {
    false
  }

  fn back(&mut self, _cx: &mut Context<Self>) {}

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
    let centered = |text: String| {
      div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(div().text_color(theme::muted()).child(text))
    };
    match &self.state {
      State::Restoring => centered("Connexion à Twitch…".into()).into_any_element(),
      State::SignedOut => div()
        .size_full()
        .child(self.render_login(cx))
        .into_any_element(),
      State::SignedIn(signed) => div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(
          v_flex()
            .items_center()
            .gap_2()
            .child(format!("Connecté à Twitch en tant que {}.", signed.login))
            .child(
              div()
                .text_sm()
                .text_color(theme::muted())
                .child("Vos chaînes suivies apparaîtront ici."),
            ),
        )
        .into_any_element(),
    }
  }
}
