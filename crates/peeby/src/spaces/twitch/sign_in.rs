//! The space's sign-in with the web `auth-token`: twitch.tv's sign-in in its own window (ADR 0017),
//! or a token pasted by hand, checked with `currentUser` before it is saved. One per space, so
//! the settings and the "Suivies" card share it: a single window at a time, kept open while
//! either page is rebuilt (dropping the window's handle closes it).

use gpui::{Context, SharedString, WeakEntity};
use ui::i18n::t;
use ui::{runtime, tf};

use super::{TwitchSpace, session};
use crate::settings::Settings;
use crate::web_sign_in::{self, Outcome, Pending, Service};

pub struct SignIn {
  nav: WeakEntity<TwitchSpace>,
  /// The sign-in window, while it is open (dropping it closes the window).
  window: Option<Pending>,
  checking: bool,
  /// The outcome of the last attempt: the message, and whether it went well.
  outcome: Option<(SharedString, bool)>,
}

impl SignIn {
  pub fn new(nav: WeakEntity<TwitchSpace>) -> Self {
    Self {
      nav,
      window: None,
      checking: false,
      outcome: None,
    }
  }

  /// The sign-in window is open.
  pub fn waiting(&self) -> bool {
    self.window.is_some()
  }

  /// A window is open or a token is being checked.
  pub fn busy(&self) -> bool {
    self.checking || self.window.is_some()
  }

  pub fn outcome(&self) -> Option<(SharedString, bool)> {
    self.outcome.clone()
  }

  pub fn set_outcome(&mut self, outcome: Option<(SharedString, bool)>, cx: &mut Context<Self>) {
    self.outcome = outcome;
    cx.notify();
  }

  /// Opens twitch.tv's sign-in in its own window; the token it gives goes through `check`.
  pub fn open(&mut self, cx: &mut Context<Self>) {
    if self.busy() {
      return;
    }
    let failed = || {
      t(
        "The sign-in window couldn't open (WebView2 missing?). Paste the token by hand in the \
         settings.",
        "La fenêtre de connexion n'a pas pu s'ouvrir (WebView2 absent ?). Collez le jeton à la \
         main dans les réglages.",
      )
    };
    let (pending, run) = match web_sign_in::open(Service::Twitch) {
      Ok(window) => window,
      Err(e) => {
        tracing::warn!("cannot open the Twitch sign-in window: {e}");
        return self.set_outcome(Some((failed().into(), false)), cx);
      }
    };
    self.window = Some(pending);
    self.outcome = None;
    cx.notify();
    // Blocks a background thread until the window closes.
    let outcome = cx.background_executor().spawn(async move { run() });
    cx.spawn(async move |this, cx| {
      let outcome = outcome.await;
      this
        .update(cx, |this, cx| {
          if this.window.take().is_none() {
            return;
          }
          match outcome {
            Outcome::SignedIn(token) => this.check(token, cx),
            Outcome::Closed => {}
            Outcome::Failed => this.outcome = Some((failed().into(), false)),
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// Checks a token (its shape, then a GQL call that answers as signed in), then saves it in the
  /// Credential Manager and gives it to the space.
  pub fn check(&mut self, token: String, cx: &mut Context<Self>) {
    if !twitch::gql::is_auth_token_shape(&token) {
      return self.set_outcome(
        Some((
          t(
            "This isn't an auth-token: it is 30 letters and digits.",
            "Ce n'est pas un jeton auth-token : il fait 30 lettres et chiffres.",
          )
          .into(),
          false,
        )),
        cx,
      );
    }
    self.checking = true;
    self.outcome = None;
    cx.notify();
    let web_client_id = Settings::get(cx).twitch.web_client_id.clone();
    let check = runtime::spawn(cx, {
      let token = token.clone();
      async move {
        twitch::Client::new(web_client_id, Some(token))?
          .current_user()
          .await
      }
    });
    cx.spawn(async move |this, cx| {
      let result = check.await;
      let saved = match result {
        Some(Ok(login)) => {
          let stored = cx
            .background_executor()
            .spawn({
              let token = token.clone();
              async move { session::store_auth_token(&token) }
            })
            .await;
          match stored {
            Ok(()) => Ok((login, token)),
            Err(e) => {
              tracing::warn!("cannot save the Twitch auth-token: {e}");
              Err(
                t(
                  "The token is valid, but the Credential Manager refused to keep it.",
                  "Le jeton est valide, mais le Gestionnaire d'identification a refusé de le garder.",
                )
                .into(),
              )
            }
          }
        }
        Some(Err(twitch::Error::NotLoggedIn)) => {
          Err(t("Twitch rejects this token.", "Twitch refuse ce jeton.").into())
        }
        Some(Err(e)) => {
          tracing::warn!("Twitch auth-token check: {e}");
          Err(
            t(
              "Couldn't check the token with Twitch. Try again.",
              "Impossible de vérifier le jeton auprès de Twitch. Réessayez.",
            )
            .into(),
          )
        }
        None => return,
      };
      this
        .update(cx, |this, cx| {
          this.checking = false;
          this.outcome = Some(match saved {
            Ok((login, token)) => {
              this
                .nav
                .update(cx, |space, cx| space.set_auth_token(Some(token), cx))
                .ok();
              (
                tf!("Signed in as {login}.", "Connecté en tant que {login}.").into(),
                true,
              )
            }
            Err(message) => (message, false),
          });
          cx.notify();
        })
        .ok();
    })
    .detach();
  }
}
