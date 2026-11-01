//! Login page: the server's address, then a user name and password, or Quick Connect.

use std::time::Duration;

use gpui::prelude::*;
use gpui::{
  Animation, AnimationExt, AppContext, Context, Entity, EventEmitter, FontWeight, SharedString,
  Subscription, Task, WeakEntity, Window, div, ease_out_quint, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{ActiveTheme, Disableable, h_flex, v_flex};
use jellyfin_api::{Client, ClientInfo, Error};
use ui::{assets, runtime, theme};

/// Emitted once a session is open.
pub struct LoggedIn(pub Client);

/// Quick Connect in progress: the code to enter on a signed-in device.
struct QuickConnect {
  code: SharedString,
  _poll: Task<()>,
}

pub struct LoginView {
  info: ClientInfo,
  server: Entity<InputState>,
  username: Entity<InputState>,
  password: Entity<InputState>,
  busy: bool,
  error: Option<SharedString>,
  quick_connect: Option<QuickConnect>,
  _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LoggedIn> for LoginView {}

impl LoginView {
  /// The form, with the last server's address and why the user is here again (`error`: the
  /// session expired).
  pub fn new(
    info: ClientInfo,
    server_url: Option<String>,
    error: Option<SharedString>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let server = cx.new(|cx| {
      let state = InputState::new(window, cx).placeholder("https://jellyfin.exemple.fr");
      match &server_url {
        Some(url) => state.default_value(url.clone()),
        None => state,
      }
    });
    let username = cx.new(|cx| InputState::new(window, cx).placeholder("Nom d'utilisateur"));
    let password = cx.new(|cx| {
      InputState::new(window, cx)
        .placeholder("Mot de passe")
        .masked(true)
    });
    let submit_on_enter = |this: &mut Self,
                           _: &Entity<InputState>,
                           event: &InputEvent,
                           window: &mut Window,
                           cx: &mut Context<Self>| {
      if matches!(event, InputEvent::PressEnter { .. }) {
        this.submit(window, cx);
      }
    };
    let subscriptions = vec![
      cx.subscribe_in(&server, window, submit_on_enter),
      cx.subscribe_in(&username, window, submit_on_enter),
      cx.subscribe_in(&password, window, submit_on_enter),
    ];
    let focus = if server_url.is_some() {
      &username
    } else {
      &server
    };
    focus.update(cx, |state, cx| state.focus(window, cx));
    Self {
      info,
      server,
      username,
      password,
      busy: false,
      error,
      quick_connect: None,
      _subscriptions: subscriptions,
    }
  }

  /// A client for the typed address; `https://` is added when no scheme is given.
  fn client(&self, cx: &Context<Self>) -> Result<Client, SharedString> {
    let raw = self.server.read(cx).value().trim().to_string();
    if raw.is_empty() {
      return Err("Saisissez l'adresse du serveur.".into());
    }
    let url = if raw.contains("://") {
      raw
    } else {
      format!("https://{raw}")
    };
    Client::new(&url, self.info.clone()).map_err(|e| describe(&e))
  }

  fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    if self.busy {
      return;
    }
    let mut client = match self.client(cx) {
      Ok(client) => client,
      Err(message) => return self.fail(message, cx),
    };
    let username = self.username.read(cx).value().trim().to_string();
    let password = self.password.read(cx).value().to_string();
    if username.is_empty() {
      return self.fail("Saisissez votre nom d'utilisateur.".into(), cx);
    }
    self.busy = true;
    self.error = None;
    cx.notify();

    let login = runtime::spawn(cx, async move {
      client.check_server().await?;
      client.authenticate_by_name(&username, &password).await?;
      Ok::<_, Error>(client)
    });
    cx.spawn(async move |this, cx| {
      let result = login.await;
      if let Some(Err(e)) = &result {
        tracing::info!("Jellyfin sign-in failed: {e}");
      }
      this
        .update(cx, |this, cx| match result {
          Some(Ok(client)) => cx.emit(LoggedIn(client)),
          Some(Err(e)) => this.fail(describe(&e), cx),
          None => this.fail("Le réseau de l'application s'est arrêté.".into(), cx),
        })
        .ok();
    })
    .detach();
  }

  fn start_quick_connect(&mut self, cx: &mut Context<Self>) {
    if self.busy {
      return;
    }
    let client = match self.client(cx) {
      Ok(client) => client,
      Err(message) => return self.fail(message, cx),
    };
    self.busy = true;
    self.error = None;
    cx.notify();

    let start = runtime::spawn(cx, {
      let client = client.clone();
      async move {
        client.check_server().await?;
        if !client.quick_connect_enabled().await? {
          return Ok(None);
        }
        client.quick_connect_initiate().await.map(Some)
      }
    });
    cx.spawn(async move |this, cx| {
      let (code, secret) = match start.await {
        Some(Ok(Some(qc))) => (qc.code, qc.secret),
        Some(Ok(None)) => {
          this
            .update(cx, |this, cx| {
              this.fail("Quick Connect est désactivé sur ce serveur.".into(), cx)
            })
            .ok();
          return;
        }
        Some(Err(e)) => {
          this.update(cx, |this, cx| this.fail(describe(&e), cx)).ok();
          return;
        }
        None => return,
      };
      this
        .update(cx, |this, cx| {
          let poll =
            cx.spawn(async move |this, cx| poll_quick_connect(this, cx, client, secret).await);
          this.quick_connect = Some(QuickConnect {
            code: code.into(),
            _poll: poll,
          });
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn cancel_quick_connect(&mut self, cx: &mut Context<Self>) {
    self.quick_connect = None;
    self.busy = false;
    cx.notify();
  }

  fn fail(&mut self, message: SharedString, cx: &mut Context<Self>) {
    self.busy = false;
    self.quick_connect = None;
    self.error = Some(message);
    cx.notify();
  }
}

/// Asks every 3 s whether the Quick Connect request was approved, then signs in.
async fn poll_quick_connect(
  this: WeakEntity<LoginView>,
  cx: &mut gpui::AsyncApp,
  client: Client,
  secret: String,
) {
  loop {
    cx.background_executor().timer(Duration::from_secs(3)).await;
    let Ok(check) = this.update(cx, |_, cx| {
      let client = client.clone();
      let secret = secret.clone();
      runtime::spawn(cx, async move { client.quick_connect_state(&secret).await })
    }) else {
      return;
    };
    match check.await {
      Some(Ok(state)) if state.authenticated => break,
      Some(Ok(_)) => continue,
      Some(Err(e)) => {
        this.update(cx, |this, cx| this.fail(describe(&e), cx)).ok();
        return;
      }
      None => return,
    }
  }
  let Ok(login) = this.update(cx, |_, cx| {
    let mut client = client.clone();
    runtime::spawn(cx, async move {
      client.authenticate_with_quick_connect(&secret).await?;
      Ok::<_, Error>(client)
    })
  }) else {
    return;
  };
  let result = login.await;
  this
    .update(cx, |this, cx| match result {
      Some(Ok(client)) => cx.emit(LoggedIn(client)),
      Some(Err(e)) => this.fail(describe(&e), cx),
      None => {}
    })
    .ok();
}

/// What the user reads for an API error.
pub fn describe(error: &Error) -> SharedString {
  match error {
    Error::Status { status: 401, .. } => "Nom d'utilisateur ou mot de passe incorrect.".into(),
    Error::Status { status, .. } => {
      format!("Le serveur a répondu par une erreur (HTTP {status}).").into()
    }
    Error::Http(e) if e.is_timeout() => "Le serveur n'a pas répondu à temps.".into(),
    Error::Http(_) => "Serveur injoignable. Vérifiez l'adresse et votre connexion.".into(),
    Error::InvalidUrl(_) => "Cette adresse de serveur n'est pas valide.".into(),
    Error::UnsupportedServer { version } => format!(
      "Jellyfin {version} n'est pas pris en charge : il faut la version 12.0 ou plus récente."
    )
    .into(),
    Error::Decode { .. } => {
      "Le serveur a envoyé une réponse inattendue. Est-ce bien un serveur Jellyfin ?".into()
    }
    Error::NotAuthenticated => "Vous n'êtes pas connecté.".into(),
    Error::TranscodingRequired { reasons } => format!(
      "Le serveur voudrait transcoder ce fichier, ce que Peeby ne fait pas ({}).",
      if reasons.is_empty() {
        "sans raison donnée".to_owned()
      } else {
        reasons.join(", ")
      }
    )
    .into(),
    Error::PlaybackRefused { code } => format!("Le serveur refuse la lecture ({code}).").into(),
  }
}

impl Render for LoginView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let danger = cx.theme().danger;
    let form = v_flex()
      .gap_3()
      .child(field(
        "Serveur",
        Input::new(&self.server).disabled(self.busy),
      ))
      .child(field(
        "Nom d'utilisateur",
        Input::new(&self.username).disabled(self.busy),
      ))
      .child(field(
        "Mot de passe",
        Input::new(&self.password).disabled(self.busy),
      ))
      .child(
        Button::new("login")
          .primary()
          .label("Se connecter")
          .loading(self.busy && self.quick_connect.is_none())
          .disabled(self.busy)
          .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
      )
      .child(
        Button::new("quick-connect")
          .ghost()
          .label("Utiliser Quick Connect")
          .disabled(self.busy)
          .on_click(cx.listener(|this, _, _, cx| this.start_quick_connect(cx))),
      );

    let body = match &self.quick_connect {
      Some(qc) => v_flex()
        .gap_3()
        .items_center()
        .child(
          div()
            .text_color(theme::muted())
            .text_center()
            .child("Sur un appareil déjà connecté, ouvrez Quick Connect et saisissez :"),
        )
        .child(
          div()
            .text_size(px(40.))
            .font_weight(FontWeight::BOLD)
            .child(qc.code.clone()),
        )
        .child(
          div()
            .text_sm()
            .text_color(theme::muted())
            .child("En attente de validation…"),
        )
        .child(
          Button::new("cancel-qc")
            .ghost()
            .label("Annuler")
            .on_click(cx.listener(|this, _, _, cx| this.cancel_quick_connect(cx))),
        ),
      None => form,
    };

    div()
      .id("jellyfin-login")
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
            "login-card",
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
                      .child("Jellyfin"),
                  ),
              )
              .child(
                div()
                  .text_color(theme::muted())
                  .child("Connectez-vous à votre serveur Jellyfin"),
              ),
          )
          .child(body)
          .when_some(self.error.clone(), |el, error| {
            el.child(div().text_sm().text_color(danger).child(error))
          }),
      )
  }
}

fn field(label: &'static str, input: impl IntoElement) -> impl IntoElement {
  v_flex()
    .gap_1()
    .child(h_flex().text_sm().child(label))
    .child(input)
}
