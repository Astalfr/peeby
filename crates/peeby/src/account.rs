//! Account page: session state, cookie import (checked against YouTube before it replaces the
//! current session) and sign-out.

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{Context, FontWeight, PathPromptOptions, SharedString, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Disableable, h_flex, v_flex};
use innertube::{Client, Cookies};

use crate::app::AppView;
use crate::assets::Icon;
use crate::settings::Settings;
use crate::ui::{error_text, page_title};
use crate::{runtime, settings, theme};

enum Status {
  Idle,
  Checking,
  Failed(SharedString),
  Done(SharedString),
}

pub struct AccountView {
  nav: WeakEntity<AppView>,
  signed_in: bool,
  status: Status,
}

impl AccountView {
  pub fn new(nav: WeakEntity<AppView>, signed_in: bool) -> Self {
    Self {
      nav,
      signed_in,
      status: Status::Idle,
    }
  }

  fn pick_file(&mut self, cx: &mut Context<Self>) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
      files: true,
      directories: false,
      multiple: false,
      prompt: Some("Importer".into()),
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

  /// Reads `path`, checks the cookies with a Home request, then saves them and switches the
  /// app to the new session. A file YouTube does not accept changes nothing.
  fn import(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    self.status = Status::Checking;
    cx.notify();
    let Settings {
      language: hl,
      region: gl,
      ..
    } = Settings::get(cx).clone();
    let check = runtime::spawn(cx, async move {
      let text = std::fs::read_to_string(&path).map_err(|e| format!("Lecture impossible : {e}"))?;
      let cookies = Cookies::from_netscape(&text).map_err(|e| error_text(&e))?;
      // No cookie file yet: a rotation during the check must not touch the current one.
      let client =
        Client::new(Some(cookies.clone()), None, &hl, &gl).map_err(|e| error_text(&e))?;
      let feed = client.home().await.map_err(|e| error_text(&e))?;
      tracing::info!(
        "imported cookies accepted: {} videos on Home",
        feed.items.len()
      );
      settings::save_cookies(&cookies).map_err(|e| format!("Enregistrement impossible : {e}"))?;
      Ok::<Cookies, String>(cookies)
    });
    cx.spawn(async move |this, cx| {
      let result = check.await;
      this
        .update(cx, |this, cx| {
          match result {
            Some(Ok(cookies)) => {
              this.signed_in = true;
              this.status = Status::Done("Connecté. L'accueil affiche vos recommandations.".into());
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
    if let Some(path) = settings::cookies_path()
      && let Err(e) = std::fs::remove_file(&path)
      && e.kind() != std::io::ErrorKind::NotFound
    {
      self.status = Status::Failed(format!("Suppression impossible : {e}").into());
      return cx.notify();
    }
    self.signed_in = false;
    self.status = Status::Done("Déconnecté. Les cookies ont été supprimés.".into());
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
    let message = match &self.status {
      Status::Idle => None,
      Status::Checking => Some(("Vérification auprès de YouTube…".into(), theme::muted())),
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
    v_flex()
      .id("account")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        v_flex()
          .px_8()
          .py_6()
          .gap_6()
          .max_w(px(760.))
          .child(page_title("Compte"))
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
                        "Connecté avec des cookies YouTube"
                      } else {
                        "Non connecté"
                      }),
                  )
                  .child(
                    div()
                      .text_xs()
                      .text_color(theme::muted())
                      .truncate()
                      .child(format!("Fichier : {path}")),
                  ),
              )
              .when(self.signed_in, |row| {
                row.child(
                  Button::new("sign-out")
                    .ghost()
                    .label("Se déconnecter")
                    .disabled(checking)
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
              .child(
                div()
                  .font_weight(FontWeight::SEMIBOLD)
                  .child("Exporter les cookies de youtube.com au format Netscape"),
              )
              .child(
                v_flex()
                  .gap_3()
                  .text_color(theme::muted())
                  .child(step(
                    1,
                    "Ouvrez une fenêtre de navigation privée et connectez-vous à YouTube.",
                  ))
                  .child(step(
                    2,
                    "Dans le même onglet, allez sur youtube.com/robots.txt, puis exportez les \
                     cookies avec l'extension « Get cookies.txt LOCALLY ».",
                  ))
                  .child(step(
                    3,
                    "Fermez la fenêtre privée sans vous déconnecter, puis importez le fichier \
                     ici.",
                  )),
              )
              .child(div().text_xs().text_color(theme::subtle()).child(
                "Peeby garde les cookies à jour quand YouTube les renouvelle. Ce fichier \
                   donne accès à votre compte : ne le partagez pas.",
              ))
              .child(
                h_flex().child(
                  Button::new("import")
                    .primary()
                    .label(if checking {
                      "Vérification…"
                    } else {
                      "Importer cookies.txt…"
                    })
                    .disabled(checking)
                    .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx))),
                ),
              ),
          )
          .children(message.map(|(text, color)| div().text_color(color).child(text))),
      )
  }
}
