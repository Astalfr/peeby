//! A grid of videos, loading the next page as the end comes into view: Home (recommendations),
//! subscriptions, or a channel's "Videos" tab. Pages that need a session link to the account page.
//! The subscriptions page can also show its videos as a list (`FeedLayout`).

use gpui::prelude::*;
use gpui::{Context, ScrollHandle, SharedString, WeakEntity, Window, div};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{IconName, Selectable, h_flex, v_flex};
use innertube::{Client, Error, Feed, VideoItem};

use crate::app::{AppView, Route};
use crate::assets::Icon;
use crate::images::Images;
use crate::settings::{FeedLayout, Settings};
use crate::ui::{
  error_text, loading_more, near_scroll_end, notice, page_title, skeleton_card, skeleton_row,
  video_card, video_row,
};
use crate::{motion, runtime, theme};

/// Card width, logical pixels.
const CARD_WIDTH: f32 = 416.;

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
  /// YouTube no longer accepts the session's cookies.
  Expired,
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
  /// The last next-page request failed: no automatic retry, the button retries.
  more_failed: bool,
  scroll: ScrollHandle,
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
      more_failed: false,
      scroll: ScrollHandle::new(),
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
    self.more_failed = false;
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
            Some(Err(Error::NotLoggedIn)) if !more => this.state = State::Expired,
            Some(Err(e)) => {
              tracing::warn!("{:?}: {e}", this.source);
              this.more_failed = more;
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

  /// Next page once the end of the grid is within about a screen.
  fn load_more_if_near_end(&mut self, cx: &mut Context<Self>) {
    if self.loading_more || self.more_failed || !near_scroll_end(&self.scroll) {
      return;
    }
    if let Some(token) = self.continuation.clone() {
      self.load(Some(token), cx);
    }
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
    // YouTube repeats some videos (twice on a page, or again in the next one). A card's id is
    // its video id, and two elements with one id share GPUI's hover state, which then flips on
    // every frame: keep the first.
    let (received, before) = (feed.items.len(), self.videos.len());
    for video in feed.items {
      if !self.videos.iter().any(|v| v.id == video.id) {
        self.videos.push(video);
      }
    }
    let repeated = received - (self.videos.len() - before);
    if repeated > 0 {
      tracing::debug!("{:?}: {repeated} repeated videos skipped", self.source);
    }
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

  fn empty_state(&self) -> impl IntoElement + use<> {
    let (title, body) = match (&self.state, &self.source, self.logged_in) {
      (State::Expired, _, _) => (
        "Session expirée".to_owned(),
        "YouTube ne reconnaît plus les cookies de la session. Importez-en de nouveaux depuis \
         la page Compte."
          .to_owned(),
      ),
      (State::SignedOut, _, _) | (_, Source::Home, false) => (
        "Connectez-vous pour voir cette page".to_owned(),
        "YouTube n'affiche plus de recommandations sans compte. Importez les cookies d'une \
         session YouTube depuis la page Compte."
          .to_owned(),
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
    let needs_session = matches!(self.state, State::SignedOut | State::Expired)
      || (matches!(self.source, Source::Home) && !self.logged_in);
    let nav = self.nav.clone();
    notice(
      if needs_session {
        Icon::Account
      } else {
        Icon::Home
      },
      title,
      body,
    )
    .when(needs_session, |el| {
      el.child(
        h_flex().pt_2().child(
          Button::new("to-account")
            .primary()
            .label("Importer des cookies")
            .on_click(move |_, window, cx| {
              nav
                .update(cx, |app, cx| app.open(Route::Account, window, cx))
                .ok();
            }),
        ),
      )
    })
  }
}

/// The cards, then the end of the list (spinner or retry button).
fn grid(cards: Vec<gpui::AnyElement>, more: Option<gpui::AnyElement>) -> gpui::AnyElement {
  v_flex()
    .gap_8()
    // `h_flex` centers its children vertically: cards whose title takes one line would sit
    // lower than their neighbors.
    .child(
      h_flex()
        .flex_wrap()
        .items_start()
        .gap_x_4()
        .gap_y_8()
        .children(cards),
    )
    .children(more.map(|b| h_flex().justify_center().child(b)))
    .into_any_element()
}

/// The rows, one under the other, then the end of the list.
fn rows(rows: Vec<gpui::AnyElement>, more: Option<gpui::AnyElement>) -> gpui::AnyElement {
  v_flex()
    .w_full()
    .gap_2()
    .children(rows)
    .children(more.map(|b| h_flex().pt_4().justify_center().child(b)))
    .into_any_element()
}

/// Grid or list, for the subscriptions page; the choice is saved in the settings.
fn layout_switch(list: bool, cx: &mut Context<FeedView>) -> impl IntoElement {
  let choice = |id: &'static str, icon: Icon, tooltip: &'static str, layout: FeedLayout| {
    Button::new(id)
      .ghost()
      .icon(icon)
      .tooltip(tooltip)
      .selected((layout == FeedLayout::List) == list)
      .on_click(cx.listener(move |_, _, _, cx| {
        Settings::update(cx, |s| s.subscriptions_layout = layout);
        cx.notify();
      }))
  };
  h_flex()
    .p_0p5()
    .gap_0p5()
    .rounded(theme::RADIUS)
    .border_1()
    .border_color(theme::border())
    .child(choice(
      "layout-grid",
      Icon::Grid,
      "Grille",
      FeedLayout::Grid,
    ))
    .child(choice("layout-list", Icon::List, "Liste", FeedLayout::List))
}

impl Render for FeedView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    // Offsets from the last layout: enough to start the next page before the end is reached.
    self.load_more_if_near_end(cx);
    let list = matches!(self.source, Source::Subscriptions)
      && Settings::get(cx).subscriptions_layout == FeedLayout::List;
    let content = match &self.state {
      // Replaced by the skeleton below.
      State::Loading => div().into_any_element(),
      State::Failed(e) => notice(
        IconName::TriangleAlert,
        "Impossible de charger la page",
        e.clone(),
      )
      .child(
        h_flex()
          .pt_2()
          .child(
            Button::new("retry")
              .primary()
              .label("Réessayer")
              .on_click(cx.listener(|this, _, _, cx| {
                this.state = State::Loading;
                this.load(None, cx);
                cx.notify();
              })),
          ),
      )
      .into_any_element(),
      State::SignedOut | State::Expired => self.empty_state().into_any_element(),
      State::Loaded if self.videos.is_empty() => self.empty_state().into_any_element(),
      State::Loaded => {
        let cards: Vec<_> = self
          .videos
          .iter()
          .map(|v| {
            if list {
              video_row(v, &self.nav, window, cx)
            } else {
              video_card(v, CARD_WIDTH, &self.nav, window, cx)
            }
          })
          .collect();
        // Pages load by themselves; the button only shows to retry after a failure.
        let more = self.continuation.clone().and_then(|token| {
          if self.more_failed {
            Some(
              Button::new("more")
                .primary()
                .label("Réessayer")
                .on_click(cx.listener(move |this, _, _, cx| this.load(Some(token.clone()), cx)))
                .into_any_element(),
            )
          } else {
            self.loading_more.then(|| loading_more().into_any_element())
          }
        });
        if list {
          rows(cards, more)
        } else {
          grid(cards, more)
        }
      }
    };
    let skeleton = if list {
      v_flex()
        .w_full()
        .gap_2()
        .children((0..8).map(|_| skeleton_row()))
        .into_any_element()
    } else {
      h_flex()
        .flex_wrap()
        .gap_x_4()
        .gap_y_8()
        .children((0..12).map(|_| skeleton_card(CARD_WIDTH)))
        .into_any_element()
    };
    let body = motion::crossfade(
      SharedString::from(format!("feed-load-{}", cx.entity_id())),
      matches!(self.state, State::Loading),
      skeleton,
      content,
      window,
      cx,
    );
    div()
      .size_full()
      .relative()
      .vertical_scrollbar(&self.scroll)
      .child(
        v_flex()
          .id("feed")
          .size_full()
          .overflow_y_scroll()
          .track_scroll(&self.scroll)
          // Wheel scrolling does not re-render by itself: check the end on every scroll.
          .on_scroll(cx.listener(|this, _, _, cx| {
            this.load_more_if_near_end(cx);
            cx.notify();
          }))
          .child(
            v_flex()
              .px_8()
              .py_6()
              .gap_6()
              .when(!matches!(self.source, Source::Home), |el| {
                el.child(
                  h_flex()
                    .gap_4()
                    .items_center()
                    .child(page_title(self.title()))
                    .child(div().flex_1())
                    .when(matches!(self.source, Source::Subscriptions), |el| {
                      let nav = self.nav.clone();
                      el.child(layout_switch(list, cx)).child(
                        Button::new("all-channels")
                          .icon(Icon::Subscriptions)
                          .label("Toutes les chaînes")
                          .on_click(move |_, window, cx| {
                            nav
                              .update(cx, |app, cx| app.open(Route::Channels, window, cx))
                              .ok();
                          }),
                      )
                    }),
                )
              })
              .child(body),
          ),
      )
  }
}
