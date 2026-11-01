//! The sign-in card, the space's only page while signed out: Twitch's Device Code Grant. A click
//! asks for a code, shows it large and opens twitch.tv/activate in the browser; the card polls
//! until the user approves there (docs/twitch-plan.md step 5). No password goes through Peeby.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, Context, EventEmitter, FontWeight, SharedString, Task, Window, div,
  ease_out_quint, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{ActiveTheme, Disableable, h_flex, v_flex};
use twitch::Error;
use twitch::auth::{Auth, DeviceCode, Poll, Session, Validation};
use ui::{assets, runtime, theme};

use super::session;

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
    Error::DeviceExpired => "Le code a expiré. Recommencez la connexion.".into(),
    Error::DeviceDenied => "La connexion a été refusée sur Twitch.".into(),
    Error::SessionExpired => "Votre session Twitch a expiré. Reconnectez-vous.".into(),
    Error::Http(_) => "Twitch est injoignable. Vérifiez votre connexion.".into(),
    _ => "Twitch a répondu de façon inattendue. Réessayez dans un moment.".into(),
  }
}

impl Render for LoginView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let danger = cx.theme().danger;
    let body = match &self.waiting {
      Some(waiting) => {
        let url = waiting.url.clone();
        v_flex()
          .gap_3()
          .items_center()
          .child(
            div()
              .text_color(theme::muted())
              .text_center()
              .child("Sur twitch.tv/activate, connecté à votre compte, saisissez :"),
          )
          .child(
            div()
              .text_size(px(40.))
              .font_weight(FontWeight::BOLD)
              .child(waiting.code.clone()),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme::muted())
              .child("En attente de votre validation…"),
          )
          .child(
            h_flex()
              .gap_2()
              .child(
                Button::new("twitch-open-activate")
                  .ghost()
                  .label("Rouvrir la page")
                  .on_click(move |_, _, cx| cx.open_url(&url)),
              )
              .child(
                Button::new("twitch-cancel")
                  .ghost()
                  .label("Annuler")
                  .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
              ),
          )
      }
      None => v_flex()
        .gap_3()
        .child(div().text_sm().text_color(theme::muted()).child(
          "Peeby affiche un code à saisir sur twitch.tv : aucun mot de passe ne passe par \
               l'application. Il lit vos chaînes suivies, rien d'autre.",
        ))
        .child(
          Button::new("twitch-login")
            .primary()
            .label("Se connecter avec Twitch")
            .loading(self.busy)
            .disabled(self.busy)
            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
        ),
    };

    div()
      .id("twitch-login")
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
            "twitch-login-card",
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
              .child(
                div()
                  .text_color(theme::muted())
                  .child("Connectez-vous pour regarder vos chaînes"),
              ),
          )
          .child(body)
          .when_some(self.error.clone(), |el, error| {
            el.child(div().text_sm().text_color(danger).child(error))
          }),
      )
  }
}
