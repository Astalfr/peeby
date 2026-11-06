//! Twitch's part of the settings page (behind the gear): the account, signed in with the web
//! `auth-token` (read from twitch.tv's sign-in in a WebView, ADR 0017, or pasted by hand with the
//! tutorial of docs/twitch.md, checked before it is saved), the followed channels' Helix
//! connection, the notifications, then playback.

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputState};
use gpui_component::{ActiveTheme, Disableable, Sizable, h_flex, v_flex};
use ui::form::{chips, row, section, toggle};
use ui::i18n::t;
use ui::{tf, theme};

use super::sign_in::SignIn;
use super::{TwitchSpace, play};
use crate::settings::Settings;
use crate::shell::delivery_row;

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
  /// The space's sign-in, shared with the "Suivies" card.
  sign_in: Entity<SignIn>,
}

impl SettingsView {
  pub fn new(
    nav: WeakEntity<TwitchSpace>,
    sign_in: Entity<SignIn>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    cx.observe_global::<Settings>(|_, cx| cx.notify()).detach();
    cx.observe(&sign_in, |_, _, cx| cx.notify()).detach();
    let input = cx.new(|cx| {
      InputState::new(window, cx)
        .placeholder(t("auth-token", "Jeton auth-token"))
        .masked(true)
    });
    Self {
      nav,
      input,
      sign_in,
    }
  }

  /// The pasted token.
  fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let token = self.input.read(cx).value().trim().to_owned();
    self
      .input
      .update(cx, |input, cx| input.set_value("", window, cx));
    self
      .sign_in
      .update(cx, |sign_in, cx| sign_in.check(token, cx));
  }

  /// Signs out: the auth-token and, since they need it, the followed channels.
  fn forget(&mut self, cx: &mut Context<Self>) {
    self.nav.update(cx, |space, cx| space.log_out(cx)).ok();
    self.sign_in.update(cx, |sign_in, cx| {
      sign_in.set_outcome(Some((t("Signed out.", "Déconnecté.").into(), true)), cx)
    });
  }

  /// Helix: the followed channels, connected from the "Suivies" page.
  fn followed_section(&self, cx: &mut Context<Self>) -> AnyElement {
    let (login, signed_in) = self
      .nav
      .upgrade()
      .map(|nav| (nav.read(cx).login(), nav.read(cx).account_signed_in()))
      .unwrap_or_default();
    let body = match login {
      _ if !signed_in => h_flex().child(div().text_sm().text_color(theme::muted()).child(t(
        "Sign in to your Twitch account first (above): the Following page then offers to \
         connect the followed channels.",
        "Connectez-vous d'abord à votre compte Twitch (ci-dessus) : la page Suivies propose \
         ensuite de connecter les chaînes suivies.",
      ))),
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
        "The Following page, the sidebar's lives and the bell also need Twitch's sign-in by \
         code: connect from the Following page.",
        "La page Suivies, les lives de la barre latérale et la cloche demandent aussi la \
         connexion Twitch par code : connectez-vous depuis la page Suivies.",
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
      let refused = self
        .nav
        .upgrade()
        .is_some_and(|nav| !nav.read(cx).account_signed_in());
      let status = match account {
        Some(name) => tf!("Signed in as {name}", "Connecté en tant que {name}"),
        None if refused => t(
          "Twitch rejects the saved token: sign out, then sign in again.",
          "Twitch refuse le jeton enregistré : déconnectez-vous, puis reconnectez-vous.",
        )
        .into(),
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
        .when_some(self.sign_in.read(cx).outcome(), |card, (message, ok)| {
          card.child(
            div()
              .text_sm()
              .text_color(if ok { success } else { danger })
              .child(message),
          )
        });
      return section(t("Account", "Compte"), vec![card.into_any_element()]).into_any_element();
    }
    let busy = self.sign_in.read(cx).busy();
    let waiting = self.sign_in.read(cx).waiting();
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
        ),
      )
      .child(div().text_xs().text_color(theme::muted()).child(
        if waiting {
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
      .when_some(self.sign_in.read(cx).outcome(), |card, (message, ok)| {
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

/// Where the followed channels' lives and category changes show.
fn notifications_section(cx: &gpui::App) -> AnyElement {
  section(
    t("Notifications", "Notifications"),
    vec![
      delivery_row(
        "twitch-notify-live",
        t("A followed channel goes live", "Une chaîne suivie passe en direct"),
        t(
          "In Peeby's bell, and as a Windows notification with the channel's picture. Needs the followed channels' connection.",
          "Dans la cloche de Peeby, et en notification Windows avec l'image de la chaîne. Demande la connexion des chaînes suivies.",
        ),
        |n| &mut n.twitch_live,
        cx,
      ),
      delivery_row(
        "twitch-notify-category",
        t(
          "A followed live changes category",
          "Un live suivi change de catégorie",
        ),
        t(
          "Another game or category than at the last check (every two minutes).",
          "Un autre jeu ou une autre catégorie qu'au dernier relevé (toutes les deux minutes).",
        ),
        |n| &mut n.twitch_category,
        cx,
      ),
    ],
  )
  .into_any_element()
}

fn playback_section(max_height: u32, low_latency: bool, catch_up: bool) -> AnyElement {
  let qualities = play::QUALITIES
    .iter()
    .map(|&h| (h, format!("{h}p")))
    .collect();
  let mut rows = vec![
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
  ];
  // Catch-up only applies to low latency: shown with it.
  if low_latency {
    rows.push(
      row(
        t("Catch up", "Rattrapage"),
        t(
          "Keeps lives close to real time by playing a touch faster when they fall behind (and a \
           touch slower before the buffer runs out). From the next live opened.",
          "Garde les lives près du direct en accélérant un peu quand ils prennent du retard (et en \
           ralentissant un peu avant que la mémoire tampon s'épuise). Au prochain live ouvert.",
        ),
        toggle("twitch-catch-up", catch_up, |on, cx| {
          Settings::update(cx, |s| s.twitch.catch_up = Some(on))
        }),
      )
      .into_any_element(),
    );
  }
  section(t("Playback", "Lecture"), rows).into_any_element()
}

impl Render for SettingsView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let max_height = Settings::get(cx)
      .twitch
      .max_height
      .unwrap_or(play::DEFAULT_HEIGHT);
    let low_latency = Settings::get(cx).twitch.low_latency.unwrap_or(true);
    let catch_up = Settings::get(cx).twitch.catch_up.unwrap_or(true);
    let flash = self
      .nav
      .upgrade()
      .map(|nav| nav.read(cx).account_flash())
      .unwrap_or_default();
    let sections = vec![
      flash.wrap("twitch-account-flash", self.account_section(cx), cx),
      self.followed_section(cx),
      notifications_section(cx),
      playback_section(max_height, low_latency, catch_up),
    ];
    v_flex().gap_8().children(sections)
  }
}
