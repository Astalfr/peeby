//! The "Suivies" page's cards. Signed out, [`AccountCard`] offers the web sign-in (ADR 0017)
//! first. Signed in, while Helix is not connected, [`LoginView`] connects the followed channels:
//! Twitch's Device Code Grant (the rest of the space needs no Helix). A click asks for a code, shows it large and opens twitch.tv/activate in the browser; the card polls
//! until the user approves there (docs/twitch-plan.md step 5). No password goes through Peeby.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, Context, Entity, EventEmitter, FontWeight, SharedString, Task,
  WeakEntity, Window, div, ease_out_quint, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{ActiveTheme, Disableable, h_flex, v_flex};
use twitch::Error;
use twitch::auth::{Auth, DeviceCode, Poll, Session, Validation};
use ui::i18n::t;
use ui::{assets, runtime, theme};

use super::sign_in::SignIn;
use super::{TwitchSpace, session};

/// Emitted once the user approved: the Helix session and who they are.
pub struct SignedIn(pub Session, pub Validation);

/// A code waiting for the user.
struct Waiting {
  code: SharedString,
  url: String,
  _poll: Task<()>,
}

pub struct LoginView {
  auth: Auth,
  busy: bool,
  waiting: Option<Waiting>,
  error: Option<SharedString>,
}

impl EventEmitter<SignedIn> for LoginView {}

impl LoginView {
  /// The card; `error` says why it shows again (the session expired).
  pub fn new(auth: Auth, error: Option<SharedString>) -> Self {
    Self {
      auth,
      busy: false,
      waiting: None,
      error,
    }
  }

  fn start(&mut self, cx: &mut Context<Self>) {
    self.busy = true;
    self.error = None;
    cx.notify();
    let auth = self.auth.clone();
    let request = runtime::spawn(cx, async move { auth.start_device().await });
    cx.spawn(async move |this, cx| {
      let result = request.await;
      this
        .update(cx, |this, cx| match result {
          Some(Ok(device)) => this.wait(device, cx),
          Some(Err(e)) => this.fail(describe(&e), cx),
          None => {}
        })
        .ok();
    })
    .detach();
  }

  /// Shows the code, opens the activation page and polls.
  fn wait(&mut self, device: DeviceCode, cx: &mut Context<Self>) {
    tracing::info!("Twitch sign-in code issued");
    cx.open_url(&device.verification_uri);
    let auth = self.auth.clone();
    let code = device.user_code.clone().into();
    let url = device.verification_uri.clone();
    let poll = cx.spawn(async move |this, cx| poll(this, auth, device, cx).await);
    self.waiting = Some(Waiting {
      code,
      url,
      _poll: poll,
    });
    cx.notify();
  }

  fn cancel(&mut self, cx: &mut Context<Self>) {
    self.waiting = None;
    self.busy = false;
    cx.notify();
  }

  fn fail(&mut self, message: SharedString, cx: &mut Context<Self>) {
    self.waiting = None;
    self.busy = false;
    self.error = Some(message);
    cx.notify();
  }
}

async fn poll(
  this: gpui::WeakEntity<LoginView>,
  auth: Auth,
  device: DeviceCode,
  cx: &mut gpui::AsyncApp,
) {
  let mut interval = Duration::from_secs(device.interval);
  let tokens = loop {
    cx.background_executor().timer(interval).await;
    let request = {
      let (auth, device) = (auth.clone(), device.clone());
      let Ok(request) = this.update(cx, |_, cx| {
        runtime::spawn(cx, async move { auth.poll(&device).await })
      }) else {
        return;
      };
      request
    };
    match request.await {
      Some(Ok(Poll::Pending)) => {}
      Some(Ok(Poll::SlowDown)) => interval += Duration::from_secs(5),
      Some(Ok(Poll::Done(tokens))) => break tokens,
      Some(Err(e)) => {
        this.update(cx, |this, cx| this.fail(describe(&e), cx)).ok();
        return;
      }
      None => return,
    }
  };
  // The refresh token is stored before anything else, then the token says who signed in.
  let Ok(open) = this.update(cx, |_, cx| {
    runtime::spawn(cx, async move {
      let session = Session::begin(auth, tokens, session::store()).await;
      let validation = session.validate().await?;
      Ok::<_, Error>((session, validation))
    })
  }) else {
    return;
  };
  let result = open.await;
  this
    .update(cx, |this, cx| match result {
      Some(Ok((session, validation))) => {
        tracing::info!("signed in to Twitch");
        this.waiting = None;
        this.busy = false;
        cx.emit(SignedIn(session, validation));
      }
      Some(Err(e)) => this.fail(describe(&e), cx),
      None => {}
    })
    .ok();
}

/// What the user reads for a sign-in error.
pub fn describe(error: &Error) -> SharedString {
  match error {
    Error::DeviceExpired => t(
      "The code expired. Start signing in again.",
      "Le code a expiré. Recommencez la connexion.",
    )
    .into(),
    Error::DeviceDenied => t(
      "Signing in was declined on Twitch.",
      "La connexion a été refusée sur Twitch.",
    )
    .into(),
    Error::SessionExpired => t(
      "The connection of your followed channels expired. Connect again.",
      "La connexion de vos chaînes suivies a expiré. Reconnectez-vous.",
    )
    .into(),
    Error::Http(_) => t(
      "Twitch can't be reached. Check your connection.",
      "Twitch est injoignable. Vérifiez votre connexion.",
    )
    .into(),
    _ => t(
      "Twitch answered unexpectedly. Try again in a moment.",
      "Twitch a répondu de façon inattendue. Réessayez dans un moment.",
    )
    .into(),
  }
}

impl Render for LoginView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let body = match &self.waiting {
      Some(waiting) => {
        let url = waiting.url.clone();
        v_flex()
          .gap_3()
          .items_center()
          .child(div().text_color(theme::muted()).text_center().child(t(
            "On twitch.tv/activate, signed in to your account, enter:",
            "Sur twitch.tv/activate, connecté à votre compte, saisissez :",
          )))
          .child(
            div()
              .text_size(px(40.))
              .font_weight(FontWeight::BOLD)
              .child(waiting.code.clone()),
          )
          .child(div().text_sm().text_color(theme::muted()).child(t(
            "Waiting for your approval…",
            "En attente de votre validation…",
          )))
          .child(
            h_flex()
              .gap_2()
              .child(
                Button::new("twitch-open-activate")
                  .ghost()
                  .label(t("Reopen the page", "Rouvrir la page"))
                  .on_click(move |_, _, cx| cx.open_url(&url)),
              )
              .child(
                Button::new("twitch-cancel")
                  .ghost()
                  .label(t("Cancel", "Annuler"))
                  .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
              ),
          )
      }
      None => v_flex()
        .gap_3()
        .child(div().text_sm().text_color(theme::muted()).child(t(
          "Peeby shows a code to enter on twitch.tv: no password goes through the app. It \
           reads your followed channels, nothing else.",
          "Peeby affiche un code à saisir sur twitch.tv : aucun mot de passe ne passe par \
           l'application. Il lit vos chaînes suivies, rien d'autre.",
        )))
        .child(
          Button::new("twitch-login")
            .primary()
            .label(t("Connect with Twitch", "Se connecter avec Twitch"))
            .loading(self.busy)
            .disabled(self.busy)
            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
        ),
    };

    frame(
      "twitch-login",
      t(
        "Connect your followed channels",
        "Connectez vos chaînes suivies",
      ),
      body,
      self.error.clone(),
      window,
      cx,
    )
  }
}

/// The "Suivies" page's card, centered: Twitch's logo and name, `subtitle`, `body`, then `error`.
fn frame(
  id: &'static str,
  subtitle: &'static str,
  body: impl IntoElement,
  error: Option<SharedString>,
  window: &mut Window,
  cx: &mut gpui::App,
) -> impl IntoElement {
  let danger = cx.theme().danger;
  div()
    .id(id)
    .size_full()
    .overflow_y_scroll()
    .flex()
    .items_center()
    .justify_center()
    .p_8()
    .child(
      v_flex()
        .w(px(420.))
        .p_8()
        .gap_6()
        .rounded(theme::RADIUS_LG)
        .bg(theme::surface())
        .border_1()
        .border_color(theme::border())
        .shadow_lg()
        .with_animation(
          id,
          Animation::new(theme::NORMAL).with_easing(ease_out_quint()),
          |el, t| el.opacity(t).top(px(12. * (1. - t))),
        )
        .child(
          v_flex()
            .gap_1()
            .child(
              h_flex()
                .gap_3()
                .items_center()
                .child(assets::logo(40., window, cx))
                .child(
                  div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child("Twitch"),
                ),
            )
            .child(div().text_color(theme::muted()).child(subtitle)),
        )
        .child(body)
        .when_some(error, |el, error| {
          el.child(div().text_sm().text_color(danger).child(error))
        }),
    )
}

/// "Suivies" before the account is signed in (or while Twitch rejects its token): the web
/// sign-in first (ADR 0017), since the followed channels' connection is only offered after it.
pub struct AccountCard {
  nav: WeakEntity<TwitchSpace>,
  sign_in: Entity<SignIn>,
  /// The saved token was rejected by Twitch.
  refused: bool,
}

impl AccountCard {
  pub fn new(
    nav: WeakEntity<TwitchSpace>,
    sign_in: Entity<SignIn>,
    refused: bool,
    cx: &mut Context<Self>,
  ) -> Self {
    cx.observe(&sign_in, |_, _, cx| cx.notify()).detach();
    Self {
      nav,
      sign_in,
      refused,
    }
  }
}

impl Render for AccountCard {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let (busy, waiting, outcome) = {
      let sign_in = self.sign_in.read(cx);
      (sign_in.busy(), sign_in.waiting(), sign_in.outcome())
    };
    let error = match outcome {
      Some((message, false)) => Some(message),
      _ if self.refused => Some(
        t(
          "Twitch rejects the saved sign-in. Sign in again.",
          "Twitch refuse la connexion enregistrée. Reconnectez-vous.",
        )
        .into(),
      ),
      _ => None,
    };
    let body = v_flex()
      .gap_3()
      .child(div().text_sm().text_color(theme::muted()).child(if waiting {
        t(
          "Sign in in the window that opened. It closes by itself once you are signed in.",
          "Connectez-vous dans la fenêtre qui s'est ouverte. Elle se ferme d'elle-même une fois            connecté.",
        )
      } else {
        t(
          "Sign in on twitch.tv, in a window Peeby opens: your password goes to Twitch only.            Videos then play as your account (where you stopped in VODs, channel points, Turbo            and subscriptions), and your followed channels can be connected here.",
          "Connectez-vous sur twitch.tv, dans une fenêtre que Peeby ouvre : votre mot de passe            ne va qu'à Twitch. Les vidéos se lisent alors avec votre compte (reprise des VOD,            points de chaîne, Turbo et abonnements), et vos chaînes suivies peuvent être            connectées ici.",
        )
      }))
      .child(
        h_flex()
          .gap_2()
          .child(
            Button::new("twitch-web-sign-in")
              .primary()
              .label(if waiting {
                t("Waiting for the sign-in…", "En attente de la connexion…")
              } else {
                t("Sign in to Twitch…", "Se connecter à Twitch…")
              })
              .loading(busy)
              .disabled(busy)
              .on_click(cx.listener(|this, _, _, cx| {
                this.sign_in.update(cx, |sign_in, cx| sign_in.open(cx))
              })),
          )
          .child(
            Button::new("twitch-paste-token")
              .ghost()
              .label(t("Paste a token…", "Coller un jeton…"))
              .on_click(cx.listener(|this, _, _, cx| {
                this
                  .nav
                  .update(cx, |space, cx| space.reveal_account(cx))
                  .ok();
              })),
          ),
      );
    frame(
      "twitch-account",
      t("Sign in to your account", "Connectez-vous à votre compte"),
      body,
      error,
      window,
      cx,
    )
  }
}
