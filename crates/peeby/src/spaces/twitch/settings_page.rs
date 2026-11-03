//! Twitch's part of the settings page (behind the gear): the account, signed in with the web
//! `auth-token` (read from twitch.tv's sign-in in a WebView, ADR 0017, or pasted by hand with the
//! tutorial of docs/twitch.md, checked before it is saved), the followed channels' Helix
//! connection, then playback.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, SharedString, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use ui::form::{chips, row, section, toggle};
use ui::i18n::t;
use ui::{runtime, tf, theme};

use super::{TwitchSpace, play, session};
use crate::settings::Settings;
use crate::web_sign_in::{self, Outcome, Pending, Service};

/// The tutorial's steps (docs/twitch.md, "The auth-token tutorial").
fn tutorial() -> [&'static str; 5] {
  [
    t(
      "In your browser, open twitch.tv and sign in.",
      "Dans votre navigateur, ouvrez twitch.tv et connectez-vous.",
    ),
    t(
      "Press F12 to open the developer tools.",
      "Appuyez sur F12 pour ouvrir les outils de développement.",
    ),
    t(
      "Chrome or Edge: the \"Application\" tab, then \"Cookies\". Firefox: the \"Storage\" tab, \
       then \"Cookies\". Pick the twitch.tv site.",
      "Chrome ou Edge : onglet « Application », puis « Cookies ». Firefox : onglet « Stockage », \
       puis « Cookies ». Choisissez le site twitch.tv.",
    ),
    t(
      "In the list, find the auth-token row and copy its value (30 letters and digits).",
      "Dans la liste, trouvez la ligne auth-token et copiez sa valeur (30 lettres et chiffres).",
    ),
    t("Paste it here.", "Collez-la ici."),
  ]
}

pub struct SettingsView {
  nav: WeakEntity<TwitchSpace>,
  input: Entity<InputState>,
  /// The sign-in window, while it is open (dropping it closes the window).
  window: Option<Pending>,
  checking: bool,
  /// The outcome of the last save: the message, and whether it went well.
  outcome: Option<(SharedString, bool)>,
}

impl SettingsView {
  pub fn new(nav: WeakEntity<TwitchSpace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
    cx.observe_global::<Settings>(|_, cx| cx.notify()).detach();
    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .placeholder(t("auth-token", "Jeton auth-token"))
        .masked(true)
    });
    Self {
      nav,
      input,
      window: None,
      checking: false,
      outcome: None,
    }
  }

  /// Opens twitch.tv's sign-in in its own window; the token it gives goes through `check`.
  fn sign_in(&mut self, cx: &mut Context<Self>) {
    let failed = || {
      t(
        "The sign-in window couldn't open (WebView2 missing?). Paste the token by hand below.",
        "La fenêtre de connexion n'a pas pu s'ouvrir (WebView2 absent ?). Collez le jeton à la \
         main ci-dessous.",
      )
    };
    let (pending, run) = match web_sign_in::open(Service::Twitch) {
      Ok(window) => window,
      Err(e) => {
        tracing::warn!("cannot open the Twitch sign-in window: {e}");
        self.outcome = Some((failed().into(), false));
        return cx.notify();
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

  /// The pasted token.
  fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let token = self.input.read(cx).value().trim().to_owned();
    self
      .input
      .update(cx, |input, cx| input.set_value("", window, cx));
    self.check(token, cx);
  }

  /// Checks a token (its shape, then a GQL call that answers as signed in), then saves it in the
  /// Credential Manager.
  fn check(&mut self, token: String, cx: &mut Context<Self>) {
    if !twitch::gql::is_auth_token_shape(&token) {
      self.outcome = Some((
        t(
          "This isn't an auth-token: it is 30 letters and digits.",
          "Ce n'est pas un jeton auth-token : il fait 30 lettres et chiffres.",
        )
        .into(),
        false,
      ));
      return cx.notify();
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
        Some(Err(twitch::Error::NotLoggedIn)) => Err(t("Twitch rejects this token.", "Twitch refuse ce jeton.").into()),
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
                tf!(
                  "Token saved, account {login}.",
                  "Jeton enregistré, compte {login}."
                )
                .into(),
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

  fn forget(&mut self, cx: &mut Context<Self>) {
    cx.background_executor()
      .spawn(async { session::forget_auth_token() })
      .detach();
    self
      .nav
      .update(cx, |space, cx| space.set_auth_token(None, cx))
      .ok();
    self.outcome = Some((t("Signed out.", "Déconnecté.").into(), true));
    cx.notify();
  }

  /// Helix: the followed channels, connected from the "Suivies" page.
  fn followed_section(&self, cx: &mut Context<Self>) -> AnyElement {
    let login = self.nav.upgrade().and_then(|nav| nav.read(cx).login());
    let body = match login {
      Some(login) => h_flex()
        .gap_4()
        .items_center()
        .child(
          div()
            .flex_1()
            .text_sm()
            .child(tf!("Connected as {login}", "Connecté en tant que {login}")),
        )
        .child(
          Button::new("twitch-sign-out")
            .small()
            .label(t("Disconnect", "Déconnecter"))
            .on_click(cx.listener(|this, _, _, cx| {
              this
                .nav
                .update(cx, |space, cx| space.disconnect_helix(cx))
                .ok();
            })),
        ),
      None => h_flex().child(div().text_sm().text_color(theme::muted()).child(t(
        "The Following page, the sidebar's lives and the bell need Twitch's sign-in by code: \
         connect from the Following page.",
        "La page Suivies, les lives de la barre latérale et la cloche demandent la connexion \
         Twitch par code : connectez-vous depuis la page Suivies.",
      ))),
    };
    section(
      t("Followed channels", "Chaînes suivies"),
      vec![
        div()
          .px_5()
          .py_4()
          .rounded(theme::RADIUS)
          .bg(theme::surface())
          .child(body)
          .into_any_element(),
      ],
    )
    .into_any_element()
  }

  /// The site's auth-token: the space's sign-in.
  fn account_section(&self, cx: &mut Context<Self>) -> AnyElement {
    let (success, danger) = (cx.theme().success, cx.theme().danger);
    let saved = self
      .nav
      .upgrade()
      .is_some_and(|nav| nav.read(cx).has_auth_token());
    let account = self
      .nav
      .upgrade()
      .and_then(|nav| nav.read(cx).account().map(|a| a.display_name.clone()));
    if saved {
      let status = match account {
        Some(name) => tf!("Signed in as {name}", "Connecté en tant que {name}"),
        None => t("A token is saved.", "Un jeton est enregistré.").into(),
      };
      let card = v_flex()
        .px_5()
        .py_4()
        .gap_3()
        .rounded(theme::RADIUS)
        .bg(theme::surface())
        .child(
          h_flex()
            .gap_4()
            .items_center()
            .child(div().flex_1().text_sm().child(status))
            .child(
              Button::new("twitch-auth-token-forget")
                .small()
                .label(t("Sign out", "Se déconnecter"))
                .on_click(cx.listener(|this, _, _, cx| this.forget(cx))),
            ),
        )
        .when_some(self.outcome.clone(), |card, (message, ok)| {
          card.child(
            div()
              .text_sm()
              .text_color(if ok { success } else { danger })
              .child(message),
          )
        });
      return section(t("Account", "Compte"), vec![card.into_any_element()]).into_any_element();
    }
    let busy = self.checking || self.window.is_some();
    let steps = tutorial().into_iter().enumerate().map(|(i, step)| {
      h_flex()
        .gap_2()
        .items_start()
        .child(
          div()
            .w(px(18.))
            .flex_none()
            .text_sm()
            .text_color(theme::muted())
            .child(format!("{}.", i + 1)),
        )
        // `min_w_0` lets the text wrap instead of widening the row.
        .child(div().flex_1().min_w_0().text_sm().child(step))
    });
    let card = v_flex()
      .px_5()
      .py_4()
      .gap_3()
      .rounded(theme::RADIUS)
      .bg(theme::surface())
      .child(div().text_sm().child(t(
        "Peeby signs in to your account with the twitch.tv site's token. With it, videos play as \
         on twitch.tv (without ads if you have Turbo or a subscription to the channel, and \
         subscriber-only VODs), with your channel points and where you stopped in VODs. Without \
         it, Twitch opens too: browsing, search and playback, with the ads removed by Peeby.",
        "Peeby se connecte à votre compte avec le jeton du site twitch.tv. Avec lui, les vidéos \
         se lisent comme sur twitch.tv (sans publicité si vous avez Turbo ou un abonnement à la \
         chaîne, et les VOD réservées aux abonnés), avec vos points de chaîne et la reprise des \
         VOD. Sans lui, Twitch s'ouvre aussi : catalogue, recherche et lecture, les publicités \
         retirées par Peeby.",
      )))
      .child(
        h_flex().child(
          Button::new("twitch-sign-in")
            .primary()
            .small()
            .label(if self.window.is_some() {
              t("Waiting for the sign-in…", "En attente de la connexion…")
            } else {
              t("Sign in to Twitch…", "Se connecter à Twitch…")
            })
            .loading(busy)
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
        ),
      )
      .child(div().text_xs().text_color(theme::muted()).child(
        if self.window.is_some() {
          t(
            "Sign in in the window that opened. It closes by itself once you are signed in.",
            "Connectez-vous dans la fenêtre qui s'est ouverte. Elle se ferme d'elle-même une fois \
             connecté.",
          )
        } else {
          t(
            "A window opens on twitch.tv's sign-in page. Your password goes to Twitch only: Peeby \
             keeps the site's token once you are signed in, then closes the window.",
            "Une fenêtre s'ouvre sur la page de connexion de twitch.tv. Votre mot de passe ne va \
             qu'à Twitch : Peeby garde le jeton du site une fois connecté, puis ferme la fenêtre.",
          )
        },
      ))
      .child(div().pt_2().text_sm().child(t(
        "Or paste the token by hand:",
        "Ou collez le jeton à la main :",
      )))
      .child(v_flex().gap_1().children(steps))
      .child(div().text_xs().text_color(theme::muted()).child(t(
        "This token gives full access to your account: don't share it. Peeby keeps it in the \
         Windows Credential Manager. It stays valid even after signing out of the site; to revoke \
         it, change your Twitch password or use \"Log out everywhere\".",
        "Ce jeton donne un accès complet à votre compte : ne le partagez pas. Peeby le garde dans \
         le Gestionnaire d'identification de Windows. Il reste valable même après une \
         déconnexion du site ; pour l'annuler, changez votre mot de passe Twitch ou utilisez \
         « Se déconnecter partout ».",
      )))
      .child(
        h_flex()
          .gap_2()
          .child(
            div()
              .flex_1()
              .child(Input::new(&self.input).disabled(busy)),
          )
          .child(
            Button::new("twitch-auth-token-save")
              .small()
              .label(t("Save", "Enregistrer"))
              .disabled(busy)
              .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
          ),
      )
      .when_some(self.outcome.clone(), |card, (message, ok)| {
        card.child(
          div()
            .text_sm()
            .text_color(if ok { success } else { danger })
            .child(message),
        )
      });
    section(t("Account", "Compte"), vec![card.into_any_element()]).into_any_element()
  }
}

fn playback_section(max_height: u32, low_latency: bool) -> AnyElement {
  let qualities = play::QUALITIES
    .iter()
    .map(|&h| (h, format!("{h}p")))
    .collect();
  section(
    t("Playback", "Lecture"),
    vec![
      row(
        t("Default quality", "Qualité par défaut"),
        t(
          "The highest one picked when a live starts. Without the site token, Twitch stops at \
           1080p.",
          "La plus haute choisie au lancement d'un live. Sans le jeton du site, Twitch s'arrête à \
           1080p.",
        ),
        chips("twitch-quality", qualities, &max_height, |h, cx| {
          Settings::update(cx, |s| s.twitch.max_height = Some(h))
        }),
      )
      .into_any_element(),
      row(
        t("Low latency", "Faible latence"),
        t(
          "Lives start closer to real time, a few seconds less. From the next live opened.",
          "Les lives démarrent plus près du direct, quelques secondes de moins. Au prochain live \
           ouvert.",
        ),
        toggle("twitch-low-latency", low_latency, |on, cx| {
          Settings::update(cx, |s| s.twitch.low_latency = Some(on))
        }),
      )
      .into_any_element(),
    ],
  )
  .into_any_element()
}

impl Render for SettingsView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let max_height = Settings::get(cx)
      .twitch
      .max_height
      .unwrap_or(play::DEFAULT_HEIGHT);
    let low_latency = Settings::get(cx).twitch.low_latency.unwrap_or(true);
    let sections = vec![
      self.account_section(cx),
      self.followed_section(cx),
      playback_section(max_height, low_latency),
    ];
    v_flex().gap_8().children(sections)
  }
}
