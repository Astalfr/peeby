//! The YouTube account, in the settings page: session state, sign-in through Google's page in a
//! WebView (ADR 0017), cookie import as the manual way (both checked against YouTube before they
//! replace the current session) and sign-out.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{Context, FontWeight, PathPromptOptions, SharedString, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{Disableable, h_flex, v_flex};
use innertube::{Client, Cookies};

use crate::settings;
use crate::settings::Settings;
use crate::spaces::youtube::YouTubeSpace;
use crate::spaces::youtube::convert::error_text;
use crate::web_sign_in::{self, Outcome, Pending, Service};
use ui::assets::Icon;
use ui::i18n::t;
use ui::{runtime, tf, theme};

enum Status {
  Idle,
  /// The sign-in window is open (it closes when this is dropped).
  Waiting {
    _window: Pending,
  },
  Checking,
  Failed(SharedString),
  Done(SharedString),
}

pub struct AccountView {
  nav: WeakEntity<YouTubeSpace>,
  signed_in: bool,
  status: Status,
}

impl AccountView {
  pub fn new(nav: WeakEntity<YouTubeSpace>, signed_in: bool) -> Self {
    Self {
      nav,
      signed_in,
      status: Status::Idle,
    }
  }

  /// Opens Google's sign-in in its own window; the session it gives goes through `check`.
  fn sign_in(&mut self, cx: &mut Context<Self>) {
    let (pending, run) = match web_sign_in::open(Service::YouTube) {
      Ok(window) => window,
      Err(e) => {
        tracing::warn!("cannot open the YouTube sign-in window: {e}");
        self.status = Status::Failed(window_failed().into());
        return cx.notify();
      }
    };
    self.status = Status::Waiting { _window: pending };
    cx.notify();
    // Blocks a background thread until the window closes.
    let outcome = cx.background_executor().spawn(async move { run() });
    cx.spawn(async move |this, cx| {
      let outcome = outcome.await;
      this
        .update(cx, |this, cx| {
          if !matches!(this.status, Status::Waiting { .. }) {
            return;
          }
          match outcome {
            Outcome::SignedIn(jar) => this.check(Ok(jar), cx),
            Outcome::Closed => {
              this.status = Status::Idle;
              cx.notify();
            }
            Outcome::Failed => {
              this.status = Status::Failed(window_failed().into());
              cx.notify();
            }
          }
        })
        .ok();
    })
    .detach();
  }

  fn pick_file(&mut self, cx: &mut Context<Self>) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
      files: true,
      directories: false,
      multiple: false,
      prompt: Some(t("Import", "Importer").into()),
    });
    cx.spawn(async move |this, cx| {
      let Ok(Ok(Some(paths))) = paths.await else {
        return;
      };
      let Some(path) = paths.into_iter().next() else {
        return;
      };
      this.update(cx, |this, cx| this.import(path, cx)).ok();
    })
    .detach();
  }

  fn import(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    self.status = Status::Checking;
    cx.notify();
    let read = cx.background_executor().spawn(async move {
      std::fs::read_to_string(&path)
        .map_err(|e| tf!("Cannot read the file: {e}", "Lecture impossible : {e}"))
    });
    cx.spawn(async move |this, cx| {
      let text = read.await;
      this.update(cx, |this, cx| this.check(text, cx)).ok();
    })
    .detach();
  }

  /// Checks a cookie file's `text` (imported, or from the sign-in window) with a Home request,
  /// then saves it and switches the app to the new session. Cookies YouTube does not accept
  /// change nothing.
  fn check(&mut self, text: Result<String, String>, cx: &mut Context<Self>) {
    self.status = Status::Checking;
    cx.notify();
    let Settings {
      language: hl,
      region: gl,
      ..
    } = Settings::get(cx).clone();
    let check = runtime::spawn(cx, async move {
      let text = text?;
      let cookies = Cookies::from_netscape(&text).map_err(|e| error_text(&e))?;
      // No cookie file yet: a rotation during the check must not touch the current one.
      let client =
        Client::new(Some(cookies.clone()), None, &hl, &gl).map_err(|e| error_text(&e))?;
      let feed = client.home().await.map_err(|e| error_text(&e))?;
      tracing::info!("new cookies accepted: {} videos on Home", feed.items.len());
      settings::save_cookies(&cookies).map_err(|e| {
        tf!(
          "Cannot save the cookies: {e}",
          "Enregistrement impossible : {e}"
        )
      })?;
      Ok::<Cookies, String>(cookies)
    });
    cx.spawn(async move |this, cx| {
      let result = check.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(cookies)) => {
              this.signed_in = true;
              this.status = Status::Done(
                t(
                  "Signed in. Home shows your recommendations.",
                  "Connecté. L'accueil affiche vos recommandations.",
                )
                .into(),
              );
              this
                .nav
                .update(cx, |app, cx| app.set_session(Some(cookies), cx))
                .ok();
            }
            Some(Err(e)) => this.status = Status::Failed(e.into()),
            None => this.status = Status::Idle,
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn sign_out(&mut self, cx: &mut Context<Self>) {
    super::tools::remove_cookie_copy();
    if let Err(e) = settings::delete_cookies() {
      self.status = Status::Failed(
        tf!(
          "Cannot delete the cookies: {e}",
          "Suppression impossible : {e}"
        )
        .into(),
      );
      return cx.notify();
    }
    self.signed_in = false;
    self.status = Status::Done(
      t(
        "Signed out. The cookies were deleted.",
        "Déconnecté. Les cookies ont été supprimés.",
      )
      .into(),
    );
    self
      .nav
      .update(cx, |app, cx| app.set_session(None, cx))
      .ok();
    cx.notify();
  }
}

impl Render for AccountView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let path = settings::cookies_path()
      .map(|p| p.display().to_string())
      .unwrap_or_default();
    let checking = matches!(self.status, Status::Checking);
    let waiting = matches!(self.status, Status::Waiting { .. });
    let busy = checking || waiting;
    let message = match &self.status {
      Status::Idle => None,
      Status::Waiting { .. } => Some((
        t(
          "Sign in in the window that opened. It closes by itself once you are signed in.",
          "Connectez-vous dans la fenêtre qui s'est ouverte. Elle se ferme d'elle-même une fois \
           connecté.",
        )
        .into(),
        theme::muted(),
      )),
      Status::Checking => Some((
        t("Checking with YouTube…", "Vérification auprès de YouTube…").into(),
        theme::muted(),
      )),
      Status::Failed(e) => Some((e.clone(), theme::accent())),
      Status::Done(m) => Some((m.clone(), theme::foreground())),
    };
    let step = |n: u32, text: &'static str| {
      h_flex()
        .gap_3()
        .items_start()
        .child(
          div()
            .size(px(22.))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme::accent_soft())
            .text_color(theme::accent())
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .child(n.to_string()),
        )
        .child(div().pt(px(2.)).child(text))
    };
    v_flex().child(
      v_flex()
        .gap_4()
        .child(
          div()
            .pb_2()
            .text_base()
            .font_weight(FontWeight::SEMIBOLD)
            .child(t("Account", "Compte")),
        )
        .child(
          h_flex()
            .px_5()
            .py_4()
            .gap_4()
            .items_center()
            .rounded(theme::RADIUS_LG)
            .bg(theme::surface())
            .child(
              div()
                .size(px(44.))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(self.signed_in, |el| {
                  el.bg(theme::accent()).text_color(theme::on_accent())
                })
                .when(!self.signed_in, |el| {
                  el.bg(theme::surface_hover()).text_color(theme::muted())
                })
                .child(gpui_component::Icon::new(Icon::Account).size(px(22.))),
            )
            .child(
              v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                  div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if self.signed_in {
                      t(
                        "Signed in with YouTube cookies",
                        "Connecté avec des cookies YouTube",
                      )
                    } else {
                      t("Not signed in", "Non connecté")
                    }),
                )
                .child(
                  div()
                    .text_xs()
                    .text_color(theme::muted())
                    .truncate()
                    .child(tf!("File: {path}", "Fichier : {path}")),
                ),
            )
            .when(self.signed_in, |row| {
              row.child(
                Button::new("sign-out")
                  .ghost()
                  .label(t("Sign out", "Se déconnecter"))
                  .disabled(busy)
                  .on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))),
              )
            }),
        )
        .child(
          v_flex()
            .gap_4()
            .p_5()
            .rounded(theme::RADIUS_LG)
            .border_1()
            .border_color(theme::border())
            .text_sm()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(t(
              "Sign in with your Google account",
              "Se connecter avec son compte Google",
            )))
            .child(div().text_color(theme::muted()).child(t(
              "A window opens on Google's sign-in page. Your password goes to Google only: Peeby \
               keeps the session cookies once you are signed in, then closes the window.",
              "Une fenêtre s'ouvre sur la page de connexion de Google. Votre mot de passe ne va \
               qu'à Google : Peeby garde les cookies de la session une fois connecté, puis ferme \
               la fenêtre.",
            )))
            .child(
              h_flex().child(
                Button::new("sign-in")
                  .primary()
                  .label(if waiting {
                    t("Waiting for the sign-in…", "En attente de la connexion…")
                  } else if checking {
                    t("Checking…", "Vérification…")
                  } else {
                    t("Sign in to YouTube…", "Se connecter à YouTube…")
                  })
                  .loading(busy)
                  .disabled(busy)
                  .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
              ),
            ),
        )
        .child(
          v_flex()
            .gap_4()
            .p_5()
            .rounded(theme::RADIUS_LG)
            .border_1()
            .border_color(theme::border())
            .text_sm()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(t(
              "Or import the cookies of youtube.com by hand (Netscape format)",
              "Ou importer les cookies de youtube.com à la main (format Netscape)",
            )))
            .child(
              v_flex()
                .gap_3()
                .text_color(theme::muted())
                .child(step(
                  1,
                  t(
                    "Open a private browsing window and sign in to YouTube.",
                    "Ouvrez une fenêtre de navigation privée et connectez-vous à YouTube.",
                  ),
                ))
                .child(step(
                  2,
                  t(
                    "In the same tab, go to youtube.com/robots.txt, then export the cookies \
                     with the \"Get cookies.txt LOCALLY\" extension.",
                    "Dans le même onglet, allez sur youtube.com/robots.txt, puis exportez les \
                     cookies avec l'extension « Get cookies.txt LOCALLY ».",
                  ),
                ))
                .child(step(
                  3,
                  t(
                    "Close the private window without signing out, then import the file here.",
                    "Fermez la fenêtre privée sans vous déconnecter, puis importez le fichier \
                     ici.",
                  ),
                )),
            )
            .child(div().text_xs().text_color(theme::subtle()).child(t(
              "Peeby keeps the cookies up to date when YouTube renews them. This file \
                 gives access to your account: do not share it.",
              "Peeby garde les cookies à jour quand YouTube les renouvelle. Ce fichier \
                 donne accès à votre compte : ne le partagez pas.",
            )))
            .child(
              h_flex().child(
                Button::new("import")
                  .label(t("Import cookies.txt…", "Importer cookies.txt…"))
                  .disabled(busy)
                  .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx))),
              ),
            ),
        )
        .children(message.map(|(text, color)| div().text_color(color).child(text))),
    )
  }
}

fn window_failed() -> &'static str {
  t(
    "The sign-in window couldn't open (WebView2 missing?). Import the cookies by hand below.",
    "La fenêtre de connexion n'a pas pu s'ouvrir (WebView2 absent ?). Importez les cookies à la \
     main ci-dessous.",
  )
}
