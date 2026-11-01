//! Twitch's part of the settings page (behind the gear): the Helix account and sign-out, then the
//! optional web `auth-token` with the tutorial of docs/twitch.md, checked before it is saved.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, SharedString, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use ui::form::{chips, row, section, toggle};
use ui::{runtime, theme};

use super::{TwitchSpace, play, session};
use crate::settings::Settings;

/// The tutorial's steps (docs/twitch.md, "The auth-token tutorial").
const TUTORIAL: &[&str] = &[
  "Dans votre navigateur, ouvrez twitch.tv et connectez-vous.",
  "Appuyez sur F12 pour ouvrir les outils de développement.",
  "Chrome ou Edge : onglet « Application », puis « Cookies ». Firefox : onglet « Stockage », puis \
   « Cookies ». Choisissez le site twitch.tv.",
  "Dans la liste, trouvez la ligne auth-token et copiez sa valeur (30 lettres et chiffres).",
  "Collez-la ici.",
];

pub struct SettingsView {
  nav: WeakEntity<TwitchSpace>,
  input: Entity<InputState>,
  checking: bool,
  /// The outcome of the last save: the message, and whether it went well.
  outcome: Option<(SharedString, bool)>,
}

impl SettingsView {
  pub fn new(nav: WeakEntity<TwitchSpace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
    cx.observe_global::<Settings>(|_, cx| cx.notify()).detach();
    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .placeholder("Jeton auth-token")
        .masked(true)
    });
    Self {
      nav,
      input,
      checking: false,
      outcome: None,
    }
  }

  /// Checks the pasted token (its shape, then a GQL call that answers as signed in), then saves
  /// it in the Credential Manager.
  fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let token = self.input.read(cx).value().trim().to_owned();
    if !twitch::gql::is_auth_token_shape(&token) {
      self.outcome = Some((
        "Ce n'est pas un jeton auth-token : il fait 30 lettres et chiffres.".into(),
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
    self
      .input
      .update(cx, |input, cx| input.set_value("", window, cx));
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
                "Le jeton est valide, mais le Gestionnaire d'identification a refusé de le garder."
                  .into(),
              )
            }
          }
        }
        Some(Err(twitch::Error::NotLoggedIn)) => Err("Twitch refuse ce jeton.".into()),
        Some(Err(e)) => {
          tracing::warn!("Twitch auth-token check: {e}");
          Err("Impossible de vérifier le jeton auprès de Twitch. Réessayez.".into())
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
              (format!("Jeton enregistré, compte {login}.").into(), true)
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
    self.outcome = Some(("Jeton supprimé.".into(), true));
    cx.notify();
  }

  fn account_section(&self, cx: &mut Context<Self>) -> AnyElement {
    let login = self.nav.upgrade().and_then(|nav| nav.read(cx).login());
    let body = match login {
      Some(login) => h_flex()
        .gap_4()
        .items_center()
        .child(
          div()
            .flex_1()
            .text_sm()
            .child(format!("Connecté en tant que {login}")),
        )
        .child(
          Button::new("twitch-sign-out")
            .small()
            .label("Se déconnecter")
            .on_click(cx.listener(|this, _, _, cx| {
              this.nav.update(cx, |space, cx| space.log_out(cx)).ok();
            })),
        ),
      None => h_flex().child(
        div()
          .text_sm()
          .text_color(theme::muted())
          .child("Connectez-vous depuis l'onglet Twitch."),
      ),
    };
    section(
      "Compte",
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

  fn auth_token_section(&self, cx: &mut Context<Self>) -> AnyElement {
    let (success, danger) = (cx.theme().success, cx.theme().danger);
    let saved = self
      .nav
      .upgrade()
      .is_some_and(|nav| nav.read(cx).has_auth_token());
    let steps = TUTORIAL.iter().enumerate().map(|(i, step)| {
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
        .child(div().flex_1().min_w_0().text_sm().child(*step))
    });
    let card = v_flex()
      .px_5()
      .py_4()
      .gap_3()
      .rounded(theme::RADIUS)
      .bg(theme::surface())
      .child(div().text_sm().child(
        "Avec lui, Peeby lit les vidéos comme votre compte sur twitch.tv : sans publicité si vous \
         avez Turbo ou un abonnement à la chaîne, et les VOD réservées aux abonnés. Sans lui, la \
         lecture marche aussi : Peeby retire lui-même les publicités.",
      ))
      .child(v_flex().gap_1().children(steps))
      .child(div().text_xs().text_color(theme::muted()).child(
        "Ce jeton donne un accès complet à votre compte : ne le partagez pas. Peeby le garde dans \
           le Gestionnaire d'identification de Windows. Il reste valable même après une \
           déconnexion du site ; pour l'annuler, changez votre mot de passe Twitch ou utilisez \
           « Se déconnecter partout ».",
      ))
      .child(
        h_flex()
          .gap_2()
          .child(
            div()
              .flex_1()
              .child(Input::new(&self.input).disabled(self.checking)),
          )
          .child(
            Button::new("twitch-auth-token-save")
              .primary()
              .small()
              .label("Enregistrer")
              .loading(self.checking)
              .disabled(self.checking)
              .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
          )
          .when(saved, |row| {
            row.child(
              Button::new("twitch-auth-token-forget")
                .ghost()
                .small()
                .label("Supprimer le jeton")
                .on_click(cx.listener(|this, _, _, cx| this.forget(cx))),
            )
          }),
      )
      .child(div().text_xs().text_color(theme::muted()).child(if saved {
        "Un jeton est enregistré."
      } else {
        "Aucun jeton enregistré."
      }))
      .when_some(self.outcome.clone(), |card, (message, ok)| {
        card.child(
          div()
            .text_sm()
            .text_color(if ok { success } else { danger })
            .child(message),
        )
      });
    section(
      "Jeton du site Twitch (facultatif)",
      vec![card.into_any_element()],
    )
    .into_any_element()
  }
}

fn playback_section(max_height: u32, low_latency: bool) -> AnyElement {
  let qualities = play::QUALITIES
    .iter()
    .map(|&h| (h, format!("{h}p")))
    .collect();
  section(
    "Lecture",
    vec![
      row(
        "Qualité par défaut",
        "La plus haute choisie au lancement d'un live. Sans le jeton du site, Twitch s'arrête à 1080p.",
        chips("twitch-quality", qualities, &max_height, |h, cx| {
          Settings::update(cx, |s| s.twitch.max_height = Some(h))
        }),
      )
      .into_any_element(),
      row(
        "Faible latence",
        "Les lives démarrent plus près du direct, quelques secondes de moins. Au prochain live ouvert.",
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
      playback_section(max_height, low_latency),
      self.auth_token_section(cx),
    ];
    v_flex().gap_8().children(sections)
  }
}
