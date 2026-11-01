//! A grid of videos, more on demand: Home (recommendations), subscriptions, or a channel's
//! "Videos" tab. Signed out, Home and subscriptions explain how to import the session cookies.

use gpui::prelude::*;
use gpui::{Context, FontWeight, SharedString, WeakEntity, Window, div, px};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{Disableable, h_flex, v_flex};
use innertube::{Client, Error, Feed, VideoItem};

use crate::app::AppView;
use crate::images::Images;
use crate::ui::{error_text, video_card};
use crate::{runtime, settings, theme};

/// Card width, logical pixels.
const CARD_WIDTH: f32 = 320.;

/// What the grid shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
  Home,
  Subscriptions,
  /// A channel's videos, newest first.
  Channel {
    id: String,
    name: String,
  },
}

enum State {
  Loading,
  Loaded,
  /// Signed out on a page that needs a session.
  SignedOut,
  Failed(SharedString),
}

pub struct FeedView {
  client: Client,
  source: Source,
  nav: WeakEntity<AppView>,
  state: State,
  videos: Vec<VideoItem>,
  continuation: Option<String>,
  loading_more: bool,
  logged_in: bool,
  message: Option<String>,
}

impl FeedView {
  pub fn new(
    client: Client,
    source: Source,
    nav: WeakEntity<AppView>,
    cx: &mut Context<Self>,
  ) -> Self {
    if let Some(images) = cx.try_global::<Images>().cloned() {
      cx.observe(&images.0, |_, _, cx| cx.notify()).detach();
    }
    let mut view = Self {
      client,
      source,
      nav,
      state: State::Loading,
      videos: Vec::new(),
      continuation: None,
      loading_more: false,
      logged_in: false,
      message: None,
    };
    view.load(None, cx);
    view
  }

  /// Loads the first page (`token` = `None`) or the next one.
  fn load(&mut self, token: Option<String>, cx: &mut Context<Self>) {
    let client = self.client.clone();
    let source = self.source.clone();
    let more = token.is_some();
    self.loading_more = more;
    let fetch = runtime::spawn(cx, async move {
      match (token, source) {
        (Some(token), _) => client.continuation(&token).await,
        (None, Source::Home) => client.home().await,
        (None, Source::Subscriptions) => client.subscriptions().await,
        (None, Source::Channel { id, .. }) => client.channel_videos(&id).await,
      }
    });
    cx.spawn(async move |this, cx| {
      let result = fetch.await;
      this
        .update(cx, |this, cx| {
          this.loading_more = false;
          match result {
            Some(Ok(feed)) => this.apply(feed, more),
            Some(Err(Error::SignInRequired)) => this.state = State::SignedOut,
            Some(Err(e)) => {
              tracing::warn!("{:?}: {e}", this.source);
              if !more {
                this.state = State::Failed(error_text(&e).into());
              }
            }
            None => {}
          }
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn apply(&mut self, feed: Feed, more: bool) {
    tracing::info!(
      "{:?}: logged_in={} · {} videos · more: {}",
      self.source,
      feed.logged_in,
      feed.items.len(),
      feed.continuation.is_some()
    );
    if !more {
      self.videos.clear();
      self.logged_in = feed.logged_in;
      self.message = feed.message;
    }
    self.videos.extend(feed.items);
    self.continuation = feed.continuation;
    self.state = State::Loaded;
  }

  fn title(&self) -> String {
    match &self.source {
      Source::Home => "Accueil".into(),
      Source::Subscriptions => "Abonnements".into(),
      Source::Channel { name, .. } => name.clone(),
    }
  }

  fn sign_in_help() -> String {
    let path = settings::cookies_path()
      .map(|p| p.display().to_string())
      .unwrap_or_else(|| "cookies.txt".into());
    format!(
      "Exportez les cookies de youtube.com au format Netscape (cookies.txt) depuis une fenêtre \
       de navigation privée connectée, fermez cette fenêtre, puis enregistrez le fichier ici :\n\
       {path}\n\nEn attendant, collez une URL de vidéo dans la barre du haut."
    )
  }

  fn empty_state(&self) -> impl IntoElement + use<> {
    let (title, body) = match (&self.state, &self.source, self.logged_in) {
      (State::SignedOut, _, _) | (_, Source::Home, false) => (
        "Connectez-vous pour voir cette page".to_owned(),
        Self::sign_in_help(),
      ),
      (_, Source::Home, true) => (
        self
          .message
          .clone()
          .unwrap_or_else(|| "Aucune recommandation".into()),
        "YouTube ne propose rien pour l'instant : l'historique des vidéos regardées est \
         peut-être désactivé sur ce compte."
          .to_owned(),
      ),
      _ => ("Aucune vidéo".to_owned(), String::new()),
    };
    v_flex()
      .max_w(px(640.))
      .gap_3()
      .child(
        div()
          .text_xl()
          .font_weight(FontWeight::SEMIBOLD)
          .child(title),
      )
      .child(div().text_color(theme::muted()).child(body))
  }
}

impl Render for FeedView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let content = match &self.state {
      State::Loading => div()
        .text_color(theme::muted())
        .child("Chargement…")
        .into_any_element(),
      State::Failed(e) => v_flex()
        .gap_2()
        .child(
          div()
            .text_xl()
            .font_weight(FontWeight::SEMIBOLD)
            .child("Impossible de charger la page"),
        )
        .child(div().text_color(theme::muted()).child(e.clone()))
        .into_any_element(),
      State::SignedOut => self.empty_state().into_any_element(),
      State::Loaded if self.videos.is_empty() => self.empty_state().into_any_element(),
      State::Loaded => {
        let cards: Vec<_> = self
          .videos
          .iter()
          .map(|v| video_card(v, CARD_WIDTH, &self.nav, window, cx))
          .collect();
        let more = self.continuation.clone().map(|token| {
          Button::new("more")
            .primary()
            .label(if self.loading_more {
              "Chargement…"
            } else {
              "Plus de vidéos"
            })
            .disabled(self.loading_more)
            .on_click(cx.listener(move |this, _, _, cx| this.load(Some(token.clone()), cx)))
        });
        v_flex()
          .gap_6()
          .child(h_flex().flex_wrap().gap_x_4().gap_y_8().children(cards))
          .children(more.map(|b| h_flex().justify_center().child(b)))
          .into_any_element()
      }
    };
    v_flex()
      .id("feed")
      .size_full()
      .overflow_y_scrollbar()
      .child(
        v_flex()
          .p_6()
          .gap_6()
          .when(!matches!(self.source, Source::Home), |el| {
            el.child(
              div()
                .text_2xl()
                .font_weight(FontWeight::SEMIBOLD)
                .child(self.title()),
            )
          })
          .child(content),
      )
  }
}
